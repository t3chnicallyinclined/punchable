//! Drive v3 access (`gsync/drive.py`): the write gate and the real client (backoff and pacing).
//!
//! Write gate: gsync writes only to docs it created (`appProperties.punchable == "1"`) or docs adopted by ID (the
//! board doc). Everything else, every one of his own docs included, is [`GsError::WriteBlocked`] before any request
//! is made.
//!
//! Move gate (merges): one of his docs may be MOVED between folders (its parents change; its text, name and trashed
//! state never do). `move_doc` re-reads the doc's metadata and refuses a legal doc, a doc that is not his, a trashed
//! doc and anything that is not a Google Doc, before any request is made.
//!
//! `read_only` is added in the port: a client made read-only refuses every write before a request is made. A dry run
//! is built that way, so it cannot write to a Doc even if a code path forgot to check.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use pk_core::py;
use serde_json::{Map, Value, json};

use crate::auth::Creds;
use crate::config::{DOC_MIME, FOLDER_MIME};
use crate::error::{GsError, HttpError, Result};
use crate::http::{Http, Response, quote_plus};
use crate::rules::Rules;
use crate::services::Services;

/// The metadata fields every call asks for.
pub const META_FIELDS: &str =
    "id,name,mimeType,trashed,parents,owners(emailAddress),createdTime,modifiedTime,version,appProperties";
/// Voice memos: audio files are only listed and downloaded, never written.
pub const MEDIA_FIELDS: &str = "id,name,mimeType,trashed,parents,owners(emailAddress),createdTime,modifiedTime,version,appProperties,size,md5Checksum,fileExtension";

const RETRY_STATUS: [u16; 5] = [429, 500, 502, 503, 504];
const RATE_REASONS: [&str; 2] = ["rateLimitExceeded", "userRateLimitExceeded"];
const CHUNK: u64 = 8 * 1024 * 1024;
const MULTIPART_BOUNDARY: &str = "==============pk-gsync==";

/// The counters a run record carries as `drive`.
#[derive(Debug, Clone, Default)]
pub struct Stats {
    /// Requests made (each attempt counts).
    pub calls: u64,
    /// Text exports.
    pub exports: u64,
    /// Writes through the gate.
    pub writes: u64,
    /// Retries after a rate limit or a server error.
    pub backoffs: u64,
    /// Media downloads.
    pub downloads: u64,
}

impl Stats {
    /// The record's `drive` object, in the Python's key order.
    pub fn to_value(&self) -> Value {
        json!({"calls": self.calls, "exports": self.exports, "writes": self.writes, "backoffs": self.backoffs,
               "downloads": self.downloads})
    }
}

/// The Google Drive client.
pub struct GoogleDrive<'a> {
    http: Http,
    creds: Creds,
    svc: &'a Services,
    rules: &'a Rules,
    root: String,
    writable: BTreeSet<String>,
    /// The counters.
    pub stats: Stats,
    read_only: bool,
    export_gap: Duration,
    last_export: Option<Instant>,
}

type Query<'q> = Vec<(&'q str, String)>;

impl<'a> GoogleDrive<'a> {
    /// A client for `creds`.
    pub fn new(creds: Creds, svc: &'a Services, rules: &'a Rules, read_only: bool) -> Self {
        Self {
            http: Http::new(Duration::from_secs(60)),
            creds,
            svc,
            rules,
            root: svc.drive_root.trim_end_matches('/').to_string(),
            writable: BTreeSet::new(),
            stats: Stats::default(),
            read_only,
            export_gap: Duration::from_millis(200),
            last_export: None,
        }
    }

    // ---- the gate -----------------------------------------------------------------------------------------------
    /// Mark a doc as one gsync may write.
    pub fn allow(&mut self, fid: &str) {
        self.writable.insert(fid.to_string());
    }

    fn assert_writable(&mut self, fid: &str) -> Result<()> {
        if self.read_only {
            return Err(GsError::WriteBlocked(format!("a dry run never writes to Drive: {fid}")));
        }
        if self.writable.contains(fid) {
            return Ok(());
        }
        let meta = self.get(fid)?;
        if meta.is_some_and(|m| m["appProperties"]["punchable"] == "1") {
            self.writable.insert(fid.to_string());
            return Ok(());
        }
        Err(GsError::WriteBlocked(format!(
            "gsync never writes to a doc it did not create: {fid}"
        )))
    }

    fn assert_may_write(&self) -> Result<()> {
        if self.read_only {
            return Err(GsError::WriteBlocked("a dry run never writes to Drive".into()));
        }
        Ok(())
    }

    /// Make a Google Doc with `text` (a plain-text upload Drive converts), tagged as ours.
    pub fn create_doc(&mut self, name: &str, text: &str, parent: Option<&str>, props: &Value) -> Result<Value> {
        self.assert_may_write()?;
        let props = tagged(props);
        let mut body = Map::new();
        body.insert("name".into(), json!(name));
        body.insert("mimeType".into(), json!(DOC_MIME));
        body.insert("appProperties".into(), props);
        if let Some(p) = parent.filter(|p| !p.is_empty()) {
            body.insert("parents".into(), json!([p]));
        }
        let meta = self.multipart("POST", "/upload/drive/v3/files", Vec::new(), &Value::Object(body), text)?;
        self.allow(meta["id"].as_str().unwrap_or_default());
        self.stats.writes += 1;
        Ok(meta)
    }

    /// Replace a doc's text (and its name when given).
    pub fn update_doc(&mut self, fid: &str, text: &str, name: Option<&str>) -> Result<Value> {
        self.assert_writable(fid)?;
        self.stats.writes += 1;
        let body = match name.filter(|n| !n.is_empty()) {
            Some(n) => json!({"name": n}),
            None => json!({}),
        };
        self.multipart(
            "PATCH",
            &format!("/upload/drive/v3/files/{}", quote_plus(fid)),
            Vec::new(),
            &body,
            text,
        )
    }

    /// Trash a doc gsync made.
    pub fn trash(&mut self, fid: &str) -> Result<()> {
        self.assert_writable(fid)?;
        self.stats.writes += 1;
        self.json_request(
            "PATCH",
            &format!("/drive/v3/files/{}", quote_plus(fid)),
            vec![("fields", "id".to_string())],
            Some(&json!({"trashed": true})),
        )?;
        Ok(())
    }

    /// The text of a doc in the given export format. Reading is never gated.
    pub fn export(&mut self, fid: &str, mime: &str) -> Result<String> {
        self.stats.exports += 1;
        if let Some(t) = self.last_export {
            let gap = self.export_gap.saturating_sub(t.elapsed());
            if !gap.is_zero() {
                (self.svc.sleep)(gap);
            }
        }
        self.last_export = Some(Instant::now());
        let r = self.execute(
            "GET",
            &format!("/drive/v3/files/{}/export", quote_plus(fid)),
            vec![("mimeType", mime.to_string())],
            None,
            None,
        )?;
        String::from_utf8(r.body).map_err(|e| GsError::other("UnicodeDecodeError", e.to_string()))
    }

    /// Make a folder gsync owns.
    pub fn create_folder(&mut self, name: &str, parent: Option<&str>, props: &Value) -> Result<Value> {
        self.assert_may_write()?;
        let body = json!({"name": name, "mimeType": FOLDER_MIME, "appProperties": tagged(props), "parents": [parent]});
        let meta = self.json_request(
            "POST",
            "/drive/v3/files",
            vec![("fields", META_FIELDS.to_string())],
            Some(&body),
        )?;
        self.allow(meta["id"].as_str().unwrap_or_default());
        self.stats.writes += 1;
        Ok(meta)
    }

    /// `appProperties` (a null value removes the key) and optionally the name of a doc gsync made. Never the text.
    pub fn set_props(&mut self, fid: &str, props: &Value, name: Option<&str>) -> Result<Value> {
        self.assert_writable(fid)?;
        self.stats.writes += 1;
        let mut body = Map::new();
        body.insert("appProperties".into(), props.clone());
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            body.insert("name".into(), json!(n));
        }
        self.json_request(
            "PATCH",
            &format!("/drive/v3/files/{}", quote_plus(fid)),
            vec![("fields", META_FIELDS.to_string())],
            Some(&Value::Object(body)),
        )
    }

    /// Move one of his docs into another folder: only its parents change. `ancestors` is every folder above it (for
    /// the legal folder rule). `WriteBlocked` for a legal, foreign, trashed or non-Doc file.
    pub fn move_doc(
        &mut self,
        fid: &str,
        add_parent: &str,
        remove_parents: &[String],
        ancestors: &[String],
    ) -> Result<Value> {
        self.assert_may_write()?;
        let meta = self.get(fid)?;
        let Some(meta) = meta.filter(|m| !py::truthy(m.get("trashed"))) else {
            return Err(GsError::WriteBlocked(format!(
                "not moving a missing or trashed doc: {fid}"
            )));
        };
        if meta["mimeType"] != DOC_MIME {
            return Err(GsError::WriteBlocked(format!("gsync only moves Google Docs: {fid}")));
        }
        if let Some(why) = self.rules.legal_reason(&meta, ancestors, &Map::new()) {
            let head = why.split(':').next().unwrap_or_default();
            return Err(GsError::WriteBlocked(format!("never touched (legal: {head}): {fid}")));
        }
        let rm: Vec<&str> = remove_parents
            .iter()
            .map(String::as_str)
            .filter(|p| *p != add_parent)
            .collect();
        self.stats.writes += 1;
        // parents only: no body, no media, so the doc's text and name are untouched
        self.json_request(
            "PATCH",
            &format!("/drive/v3/files/{}", quote_plus(fid)),
            vec![
                ("addParents", add_parent.to_string()),
                ("removeParents", rm.join(",")),
                ("fields", META_FIELDS.to_string()),
            ],
            None,
        )
    }

    /// Read-only: copy a file's bytes (an audio file he saved to Drive) to `dest`. Returns the byte count.
    pub fn download(&mut self, fid: &str, dest: &Path) -> Result<u64> {
        self.stats.downloads += 1;
        let mut f = fs::File::create(dest)?;
        let mut progress: u64 = 0;
        let url_path = format!("/drive/v3/files/{}", quote_plus(fid));
        loop {
            self.stats.calls += 1;
            let range = format!("bytes={}-{}", progress, progress + CHUNK - 1);
            let uri = self.url(&url_path, &[("alt", "media".to_string())]);
            let mut resp = None;
            for retry in 0..=5u32 {
                if retry > 0 {
                    (self.svc.sleep)(Duration::from_secs_f64(rand01() * f64::from(1u32 << retry)));
                }
                match self.send_authed("GET", &uri, &[("Range", range.clone())], None) {
                    Ok(r) if r.status >= 500 && retry < 5 => resp = Some(r),
                    Ok(r) => {
                        resp = Some(r);
                        break;
                    }
                    Err(e) if retry < 5 => {
                        let _ = e;
                    }
                    Err(e) => return Err(e),
                }
            }
            let r = resp.ok_or_else(|| GsError::other("OSError", "download: no answer"))?;
            let mut total: Option<u64> = None;
            match r.status {
                200 | 206 => {
                    progress += r.body.len() as u64;
                    f.write_all(&r.body)?;
                    if let Some(cr) = r.headers.get("content-range") {
                        total = cr.rsplit('/').next().and_then(|n| n.parse().ok());
                    } else if let Some(cl) = r.headers.get("content-length") {
                        total = cl.parse().ok();
                    }
                    if total.is_none_or(|t| progress == t) {
                        return Ok(progress);
                    }
                }
                416 => {
                    let total = r
                        .headers
                        .get("content-range")
                        .and_then(|cr| cr.rsplit('/').next())
                        .and_then(|n| n.parse::<u64>().ok());
                    if total == Some(0) {
                        return Ok(progress);
                    }
                    return Err(Self::http_error(&r, &uri));
                }
                _ => return Err(Self::http_error(&r, &uri)),
            }
        }
    }

    // ---- reads --------------------------------------------------------------------------------------------------
    /// The page token to start a Changes feed from.
    pub fn start_token(&mut self) -> Result<String> {
        let v = self.json_request("GET", "/drive/v3/changes/startPageToken", Vec::new(), None)?;
        v.get("startPageToken")
            .map(py::str)
            .ok_or_else(|| GsError::other("KeyError", "'startPageToken'"))
    }

    /// Every change since `token` and the token to resume from.
    pub fn changes(&mut self, token: &str) -> Result<(Vec<Value>, String)> {
        let mut out = Vec::new();
        let mut tok = token.to_string();
        loop {
            let q = vec![
                ("pageToken", tok.clone()),
                ("pageSize", "1000".to_string()),
                ("includeRemoved", "true".to_string()),
                ("restrictToMyDrive", "true".to_string()),
                ("spaces", "drive".to_string()),
                (
                    "fields",
                    format!("nextPageToken,newStartPageToken,changes(fileId,removed,time,file({META_FIELDS}))"),
                ),
            ];
            let r = self.json_request("GET", "/drive/v3/changes", q, None)?;
            if let Some(a) = r.get("changes").and_then(Value::as_array) {
                out.extend(a.iter().cloned());
            }
            if let Some(next) = r.get("nextPageToken").filter(|v| py::truthy(Some(v))) {
                tok = py::str(next);
                continue;
            }
            let new = r
                .get("newStartPageToken")
                .map(py::str)
                .ok_or_else(|| GsError::other("KeyError", "'newStartPageToken'"))?;
            return Ok((out, new));
        }
    }

    /// Every Google Doc he owns that is not trashed.
    pub fn list_docs(&mut self) -> Result<Vec<Value>> {
        self.list_all(
            &format!("'me' in owners and mimeType='{DOC_MIME}' and trashed=false"),
            META_FIELDS,
        )
    }

    /// Everything directly inside one folder (not trashed), with size and md5Checksum. Read-only.
    pub fn list_folder(&mut self, folder_id: &str) -> Result<Vec<Value>> {
        let fid = folder_id.replace('\\', "\\\\").replace('\'', "\\'");
        self.list_all(&format!("'{fid}' in parents and trashed=false"), MEDIA_FIELDS)
    }

    fn list_all(&mut self, q: &str, fields: &str) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut tok: Option<String> = None;
        loop {
            let mut query = vec![("q", q.to_string()), ("pageSize", "1000".to_string())];
            if let Some(t) = &tok {
                query.push(("pageToken", t.clone()));
            }
            query.push(("spaces", "drive".to_string()));
            query.push(("fields", format!("nextPageToken,files({fields})")));
            let r = self.json_request("GET", "/drive/v3/files", query, None)?;
            if let Some(a) = r.get("files").and_then(Value::as_array) {
                out.extend(a.iter().cloned());
            }
            match r.get("nextPageToken").filter(|v| py::truthy(Some(v))) {
                Some(t) => tok = Some(py::str(t)),
                None => return Ok(out),
            }
        }
    }

    /// One file's metadata; `None` when Drive says 404.
    pub fn get(&mut self, fid: &str) -> Result<Option<Value>> {
        match self.json_request(
            "GET",
            &format!("/drive/v3/files/{}", quote_plus(fid)),
            vec![("fields", META_FIELDS.to_string())],
            None,
        ) {
            Ok(v) => Ok(Some(v)),
            Err(GsError::Http(e)) if e.status == 404 => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The first file whose `appProperties[key] == value` (crash guard: a doc made but not recorded).
    pub fn find_by_prop(&mut self, key: &str, value: &str) -> Result<Option<Value>> {
        let v = value.replace('\\', "\\\\").replace('\'', "\\'");
        let r = self.json_request(
            "GET",
            "/drive/v3/files",
            vec![
                (
                    "q",
                    format!("appProperties has {{ key='{key}' and value='{v}' }} and trashed=false"),
                ),
                ("pageSize", "1".to_string()),
                ("spaces", "drive".to_string()),
                ("fields", format!("files({META_FIELDS})")),
            ],
            None,
        )?;
        Ok(r.get("files")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .cloned())
    }

    // ---- requests -----------------------------------------------------------------------------------------------
    fn url(&self, path: &str, query: &[(&str, String)]) -> String {
        let mut url = format!("{}{path}", self.root);
        if !query.is_empty() {
            let q: Vec<String> = query
                .iter()
                .map(|(k, v)| format!("{}={}", quote_plus(k), quote_plus(v)))
                .collect();
            url.push('?');
            url.push_str(&q.join("&"));
        }
        url
    }

    /// An authorised request; a 401 refreshes the token and tries again, up to twice (google-auth-httplib2).
    fn send_authed(
        &mut self,
        method: &str,
        uri: &str,
        extra: &[(&str, String)],
        body: Option<&[u8]>,
    ) -> Result<Response> {
        let mut attempt = 0;
        loop {
            if !self.creds.valid(self.svc.now_secs()) {
                self.creds.refresh(self.svc)?;
            }
            let mut headers: Vec<(&str, String)> = vec![(
                "Authorization",
                format!("Bearer {}", self.creds.token.clone().unwrap_or_default()),
            )];
            headers.extend(extra.iter().cloned());
            let r = self
                .http
                .send(method, uri, &headers, body.map(<[u8]>::to_vec))
                .map_err(|e| GsError::other(if e.timeout { "TimeoutError" } else { "ConnectionError" }, e.message))?;
            if r.status == 401 && attempt < 2 {
                attempt += 1;
                self.creds.refresh(self.svc)?;
                continue;
            }
            return Ok(r);
        }
    }

    fn http_error(r: &Response, uri: &str) -> GsError {
        GsError::Http(Box::new(HttpError::new(r.status, &r.phrase, &r.body, uri)))
    }

    /// `req.execute()` with `_x`'s backoff: up to 8 tries, a pause of `min(64, 2**n) + random` after a rate limit or
    /// a server error.
    fn execute(
        &mut self,
        method: &str,
        path: &str,
        mut query: Query<'_>,
        body: Option<&[u8]>,
        content_type: Option<&str>,
    ) -> Result<Response> {
        const TRIES: u32 = 8;
        if path.starts_with("/upload") {
            query.push(("uploadType", "multipart".to_string()));
        }
        let uri = self.url(path, &query);
        let mut extra: Vec<(&str, String)> = Vec::new();
        if let Some(ct) = content_type {
            extra.push(("Content-Type", ct.to_string()));
        }
        for n in 0..TRIES {
            self.stats.calls += 1;
            let r = self.send_authed(method, &uri, &extra, body)?;
            if r.status < 300 {
                return Ok(r);
            }
            let e = HttpError::new(r.status, &r.phrase, &r.body, &uri);
            let rate = r.status == 403 && RATE_REASONS.contains(&e.reason_code().as_str());
            if (RETRY_STATUS.contains(&r.status) || rate) && n < TRIES - 1 {
                self.stats.backoffs += 1;
                (self.svc.sleep)(Duration::from_secs_f64(f64::from((1u32 << n).min(64)) + rand01()));
                continue;
            }
            return Err(GsError::Http(Box::new(e)));
        }
        unreachable!("the last try returns")
    }

    fn json_request(&mut self, method: &str, path: &str, mut query: Query<'_>, body: Option<&Value>) -> Result<Value> {
        query.push(("alt", "json".to_string()));
        let bytes = body.map(|b| serde_json::to_vec(b).unwrap_or_default());
        let r = self.execute(method, path, query, bytes.as_deref(), body.map(|_| "application/json"))?;
        Ok(serde_json::from_slice(&r.body)?)
    }

    fn multipart(
        &mut self,
        method: &str,
        path: &str,
        mut query: Query<'_>,
        meta: &Value,
        media: &str,
    ) -> Result<Value> {
        query.push(("fields", META_FIELDS.to_string()));
        query.push(("alt", "json".to_string()));
        let mut body = Vec::new();
        let b = MULTIPART_BOUNDARY;
        body.extend_from_slice(
            format!("--{b}\r\nContent-Type: application/json\r\nMIME-Version: 1.0\r\n\r\n{}\r\n--{b}\r\nContent-Type: text/plain\r\nMIME-Version: 1.0\r\n\r\n", serde_json::to_string(meta)?)
                .as_bytes(),
        );
        body.extend_from_slice(media.as_bytes());
        body.extend_from_slice(format!("\r\n--{b}--").as_bytes());
        let ct = format!("multipart/related; boundary=\"{b}\"");
        let r = self.execute(method, path, query, Some(&body), Some(&ct))?;
        Ok(serde_json::from_slice(&r.body)?)
    }
}

fn tagged(props: &Value) -> Value {
    let mut m = props.as_object().cloned().unwrap_or_default();
    m.insert("punchable".into(), json!("1"));
    Value::Object(m)
}

/// A number in `[0, 1)` for the backoff jitter (the clock's nanoseconds; no cryptography needed).
fn rand01() -> f64 {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    f64::from(n % 1_000_000) / 1_000_000.0
}
