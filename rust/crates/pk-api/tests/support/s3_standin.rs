//! A small S3 server for the recordings tests and the differentials (T11 round 2, docs/adr/0015): path-style, in memory,
//! one bucket, the calls pk-api makes and nothing else (PUT object, multipart create / part / complete / abort, GET,
//! DELETE, ListObjectsV2; a bucket HEAD is refused like an Object Read & Write token's). It is a stand-in, not R2: it checks what R2 checks that matters to us (the SigV4
//! signature of every presigned URL against its secret, its expiry, the bucket name, R2's multipart rules: every part but
//! the last at least 5 MiB and all the same size) and can be told to fail a kind of call, to age an object, and to
//! forget the bytes (`sink`, for the memory measurement). Real R2 is not exercised by it: the SRE's scratch test is.
//!
//! Included by path (`#[path]`) from the integration tests and from `src/bin/pk_s3_standin.rs`; never part of the
//! library or the shipped image.

#![allow(
    dead_code,
    missing_docs,
    clippy::pedantic,
    clippy::format_push_string,
    clippy::format_collect
)]

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use aws_lc_rs::{digest, hmac};
use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;

/// R2's smallest part (all but the last).
const MIN_PART: usize = 5 * 1024 * 1024;

/// How to start one.
#[derive(Debug, Clone)]
pub struct Options {
    pub bucket: String,
    pub key_id: String,
    pub secret: String,
    /// Keep only the sizes of what is stored (the memory measurement streams hundreds of MiB through it).
    pub sink: bool,
    /// Keys per listing page.
    pub page: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            bucket: "test-bucket".into(),
            key_id: "TESTKEYID".into(),
            secret: "test-secret-not-a-real-one".into(),
            sink: false,
            page: 1000,
        }
    }
}

#[derive(Debug)]
struct Obj {
    data: Bytes,
    len: usize,
    modified: f64,
}

#[derive(Debug, Default)]
struct Multi {
    key: String,
    parts: BTreeMap<u16, (Bytes, usize, String)>,
}

#[derive(Debug)]
struct Fault {
    kind: String,
    status: u16,
    times: i64,
}

#[derive(Debug, Default)]
struct Inner {
    objects: BTreeMap<String, Obj>,
    uploads: HashMap<String, Multi>,
    next: u64,
    faults: Vec<Fault>,
    ops: Vec<String>,
    aborted: usize,
    completed: usize,
    refused: usize,
}

struct Shared {
    opts: Options,
    inner: Mutex<Inner>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A running stand-in.
pub struct Standin {
    pub endpoint: String,
    pub bucket: String,
    pub key_id: String,
    pub secret: String,
    shared: Arc<Shared>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Standin {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

impl Standin {
    /// Start one on a free loopback port.
    pub async fn start(opts: Options) -> Self {
        Self::start_on("127.0.0.1:0", opts).await
    }

    /// Start one on `addr`.
    pub async fn start_on(addr: &str, opts: Options) -> Self {
        let listener = tokio::net::TcpListener::bind(addr).await.expect("bind the stand-in");
        let local: SocketAddr = listener.local_addr().expect("addr");
        let shared = Arc::new(Shared {
            opts: opts.clone(),
            inner: Mutex::new(Inner::default()),
        });
        let app = Router::new().fallback(handle).with_state(shared.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            endpoint: format!("http://{local}"),
            bucket: opts.bucket,
            key_id: opts.key_id,
            secret: opts.secret,
            shared,
            task,
        }
    }

    /// `(key, size)` of every stored object.
    pub fn objects(&self) -> Vec<(String, usize)> {
        self.shared
            .lock()
            .objects
            .iter()
            .map(|(k, o)| (k.clone(), o.len))
            .collect()
    }

    /// The stored bytes of `key` (empty in sink mode).
    pub fn bytes(&self, key: &str) -> Option<Vec<u8>> {
        self.shared.lock().objects.get(key).map(|o| o.data.to_vec())
    }

    /// Multipart uploads begun and neither completed nor aborted.
    pub fn open_uploads(&self) -> usize {
        self.shared.lock().uploads.len()
    }

    /// Multipart uploads aborted so far.
    pub fn aborted(&self) -> usize {
        self.shared.lock().aborted
    }

    /// Multipart uploads completed so far.
    pub fn completed(&self) -> usize {
        self.shared.lock().completed
    }

    /// Requests refused for a bad signature, credential, bucket or expiry.
    pub fn refused(&self) -> usize {
        self.shared.lock().refused
    }

    /// Every operation seen, in order (`PUT rec/..`, `PART rec/.. 3`, `DELETE rec/..`, ...), faults included.
    pub fn ops(&self) -> Vec<String> {
        self.shared.lock().ops.clone()
    }

    /// How many operations of `kind` were seen (`put`, `part`, `start`, `complete`, `abort`, `get`, `delete`, `list`, `head`).
    pub fn count(&self, kind: &str) -> usize {
        let tag = kind.to_uppercase();
        self.shared
            .lock()
            .ops
            .iter()
            .filter(|o| o.split(' ').next() == Some(tag.as_str()))
            .count()
    }

    /// Answer `status` to the next `times` calls of `kind` (all of them when `times` is negative).
    pub fn fail(&self, kind: &str, status: u16, times: i64) {
        self.shared.lock().faults.push(Fault {
            kind: kind.into(),
            status,
            times,
        });
    }

    /// Forget every fault.
    pub fn heal(&self) {
        self.shared.lock().faults.clear();
    }

    /// Make `key` look `secs` older than it is.
    pub fn age(&self, key: &str, secs: f64) {
        if let Some(o) = self.shared.lock().objects.get_mut(key) {
            o.modified -= secs;
        }
    }

    /// Put an object in directly (an orphan that no row names).
    pub fn plant(&self, key: &str, data: &[u8]) {
        self.shared.lock().objects.insert(
            key.into(),
            Obj {
                data: Bytes::copy_from_slice(data),
                len: data.len(),
                modified: now(),
            },
        );
    }
}

// ---------------------------------------------------------------------------------------------------- signing

fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let k = hmac::Key::new(hmac::HMAC_SHA256, key);
    hmac::sign(&k, msg).as_ref().to_vec()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// RFC 3986 encoding as SigV4 wants it (unreserved kept; `/` kept when `slash`).
fn enc(s: &str, slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') || (slash && b == b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn dec(s: &str) -> String {
    percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// Whether the presigned URL carries a good signature for `secret`; `Err` names why not.
fn verify(opts: &Options, method: &str, path: &str, raw_query: &str, host: &str) -> Result<(), &'static str> {
    let mut pairs: Vec<(String, String)> = raw_query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (dec(k), dec(v))
        })
        .collect();
    let get = |name: &str| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
    let (Some(cred), Some(date), Some(expires), Some(signed), Some(sig)) = (
        get("X-Amz-Credential"),
        get("X-Amz-Date"),
        get("X-Amz-Expires"),
        get("X-Amz-SignedHeaders"),
        get("X-Amz-Signature"),
    ) else {
        return Err("not a signed request");
    };
    if get("X-Amz-Algorithm").as_deref() != Some("AWS4-HMAC-SHA256") {
        return Err("algorithm");
    }
    let scope: Vec<&str> = cred.split('/').collect();
    if scope.len() != 5 || scope[0] != opts.key_id || scope[3] != "s3" || scope[4] != "aws4_request" {
        return Err("credential");
    }
    let started = chrono::NaiveDateTime::parse_from_str(&date, "%Y%m%dT%H%M%SZ")
        .map_err(|_| "date")?
        .and_utc()
        .timestamp() as f64;
    let ttl: f64 = expires.parse().map_err(|_| "expires")?;
    if now() > started + ttl || started > now() + 300.0 {
        return Err("expired");
    }
    pairs.retain(|(k, _)| k != "X-Amz-Signature");
    let mut canon: Vec<(String, String)> = pairs.iter().map(|(k, v)| (enc(k, false), enc(v, false))).collect();
    canon.sort();
    let query = canon
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    if signed != "host" {
        return Err("signed headers");
    }
    let request = format!(
        "{method}\n{}\n{query}\nhost:{host}\n\nhost\nUNSIGNED-PAYLOAD",
        enc(&dec(path), true)
    );
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{date}\n{}/{}/s3/aws4_request\n{}",
        scope[1],
        scope[2],
        hex(digest::digest(&digest::SHA256, request.as_bytes()).as_ref())
    );
    let k = hmac_sha256(format!("AWS4{}", opts.secret).as_bytes(), scope[1].as_bytes());
    let k = hmac_sha256(&k, scope[2].as_bytes());
    let k = hmac_sha256(&k, b"s3");
    let k = hmac_sha256(&k, b"aws4_request");
    if hex(&hmac_sha256(&k, to_sign.as_bytes())) == sig {
        Ok(())
    } else {
        Err("signature")
    }
}

// ---------------------------------------------------------------------------------------------------- the server

fn xml(status: u16, body: String) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/xml")
        .body(Body::from(format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{body}")))
        .expect("response")
}

fn error(status: u16, code: &str) -> Response {
    xml(
        status,
        format!("<Error><Code>{code}</Code><Message>stand-in</Message></Error>"),
    )
}

fn etag_of(data: &[u8]) -> String {
    format!("\"{}\"", &hex(digest::digest(&digest::SHA256, data).as_ref())[..32])
}

fn param(raw_query: &str, name: &str) -> Option<String> {
    raw_query.split('&').find_map(|p| {
        let (k, v) = p.split_once('=').unwrap_or((p, ""));
        (dec(k) == name).then(|| dec(v))
    })
}

/// A fault set for `kind`, if one is due (and counted down).
fn due(inner: &mut Inner, kind: &str) -> Option<u16> {
    let f = inner.faults.iter_mut().find(|f| f.kind == kind && f.times != 0)?;
    if f.times > 0 {
        f.times -= 1;
    }
    Some(f.status)
}

async fn handle(State(sh): State<Arc<Shared>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let method = parts.method.as_str().to_string();
    let path = parts.uri.path().to_string();
    let raw_query = parts.uri.query().unwrap_or("").to_string();
    let host = parts
        .headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    if let Some(admin) = path.strip_prefix("/_standin/") {
        return admin_call(&sh, &method, admin, &raw_query);
    }
    let reject = |why: &'static str, status: u16, code: &str| {
        sh.lock().refused += 1;
        eprintln!("stand-in: refused {method} ({why})");
        error(status, code)
    };
    if let Err(why) = verify(&sh.opts, &method, &path, &raw_query, &host) {
        return reject(why, 403, "SignatureDoesNotMatch");
    }
    let Some(rest) = path.strip_prefix('/') else {
        return error(400, "BadRequest");
    };
    let (bucket, key) = match rest.split_once('/') {
        Some((b, k)) => (dec(b), dec(k)),
        None => (dec(rest), String::new()),
    };
    if bucket != sh.opts.bucket {
        return reject("bucket", 404, "NoSuchBucket");
    }
    let data = match axum::body::to_bytes(body, 64 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return error(400, "IncompleteBody"),
    };
    let upload_id = param(&raw_query, "uploadId");
    let part_number = param(&raw_query, "partNumber").and_then(|p| p.parse::<u16>().ok());
    let creating = raw_query
        .split('&')
        .any(|p| dec(p.split('=').next().unwrap_or("")) == "uploads");
    let mut g = sh.lock();
    // which call is this
    let kind = match (method.as_str(), key.is_empty(), upload_id.is_some(), creating) {
        ("HEAD", true, ..) => "head",
        ("GET", true, ..) => "list",
        ("PUT", false, true, _) => "part",
        ("PUT", false, false, _) => "put",
        ("POST", false, false, true) => "start",
        ("POST", false, true, _) => "complete",
        ("DELETE", false, true, _) => "abort",
        ("DELETE", false, false, _) => "delete",
        ("GET", false, ..) => "get",
        _ => return error(405, "MethodNotAllowed"),
    };
    g.ops.push(format!("{} {key}", kind.to_uppercase()));
    if let Some(status) = due(&mut g, kind) {
        return error(status, "InternalError");
    }
    match kind {
        // Cloudflare documents read, write and list for an Object Read & Write token, not a bucket HEAD: so is this one
        "head" => error(403, "AccessDenied"),
        "list" => {
            let prefix = param(&raw_query, "prefix").unwrap_or_default();
            let after = param(&raw_query, "continuation-token").unwrap_or_default();
            let all: Vec<(&String, &Obj)> = g
                .objects
                .iter()
                .filter(|(k, _)| k.starts_with(&prefix) && (after.is_empty() || k.as_str() > after.as_str()))
                .collect();
            let page = sh.opts.page.max(1);
            let truncated = all.len() > page;
            let shown = &all[..all.len().min(page)];
            let mut out = format!(
                "<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>{}</Name><Prefix>{prefix}</Prefix>\
                 <KeyCount>{}</KeyCount><MaxKeys>{page}</MaxKeys><EncodingType>url</EncodingType><IsTruncated>{truncated}</IsTruncated>",
                sh.opts.bucket,
                shown.len()
            );
            if truncated && let Some((last, _)) = shown.last() {
                out.push_str(&format!("<NextContinuationToken>{last}</NextContinuationToken>"));
            }
            for (k, o) in shown {
                let when = chrono::DateTime::from_timestamp_millis((o.modified * 1000.0) as i64)
                    .map(|t| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
                    .unwrap_or_default();
                out.push_str(&format!(
                    "<Contents><Key>{}</Key><LastModified>{when}</LastModified><ETag>\"x\"</ETag><Size>{}</Size>\
                     <StorageClass>STANDARD</StorageClass></Contents>",
                    enc(k, false),
                    o.len
                ));
            }
            out.push_str("</ListBucketResult>");
            xml(200, out)
        }
        "put" => {
            let etag = etag_of(&data);
            let len = data.len();
            let data = if sh.opts.sink { Bytes::new() } else { data };
            g.objects.insert(
                key,
                Obj {
                    data,
                    len,
                    modified: now(),
                },
            );
            Response::builder()
                .status(200)
                .header(header::ETAG, etag)
                .body(Body::empty())
                .expect("response")
        }
        "start" => {
            g.next += 1;
            let id = format!("upload-{}", g.next);
            g.uploads.insert(
                id.clone(),
                Multi {
                    key: key.clone(),
                    parts: BTreeMap::new(),
                },
            );
            xml(
                200,
                format!(
                    "<InitiateMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Bucket>{}</Bucket>\
                     <Key>{key}</Key><UploadId>{id}</UploadId></InitiateMultipartUploadResult>",
                    sh.opts.bucket
                ),
            )
        }
        "part" => {
            let (Some(id), Some(n)) = (upload_id, part_number) else {
                return error(400, "InvalidArgument");
            };
            let etag = etag_of(&data);
            let len = data.len();
            let data = if sh.opts.sink { Bytes::new() } else { data };
            match g.uploads.get_mut(&id) {
                Some(m) if m.key == key => {
                    m.parts.insert(n, (data, len, etag.clone()));
                    Response::builder()
                        .status(200)
                        .header(header::ETAG, etag)
                        .body(Body::empty())
                        .expect("response")
                }
                _ => error(404, "NoSuchUpload"),
            }
        }
        "complete" => {
            let Some(id) = upload_id else {
                return error(400, "InvalidArgument");
            };
            let Some(m) = g.uploads.get(&id) else {
                return error(404, "NoSuchUpload");
            };
            let text = String::from_utf8_lossy(&data).replace("&quot;", "\"");
            let mut asked: Vec<(u16, String)> = Vec::new();
            for chunk in text.split("<Part>").skip(1) {
                let num = chunk
                    .split("<PartNumber>")
                    .nth(1)
                    .and_then(|s| s.split('<').next())
                    .and_then(|s| s.parse().ok());
                let tag = chunk
                    .split("<ETag>")
                    .nth(1)
                    .and_then(|s| s.split('<').next())
                    .map(str::to_string);
                if let (Some(num), Some(tag)) = (num, tag) {
                    asked.push((num, tag));
                }
            }
            if asked.is_empty() || asked.iter().enumerate().any(|(i, (n, _))| usize::from(*n) != i + 1) {
                return error(400, "InvalidPartOrder");
            }
            let mut sizes = Vec::new();
            for (n, tag) in &asked {
                match m.parts.get(n) {
                    Some((_, len, etag)) if etag == tag => sizes.push(*len),
                    _ => return error(400, "InvalidPart"),
                }
            }
            // R2: every part but the last the same size, and at least 5 MiB
            let body_parts = &sizes[..sizes.len() - 1];
            if body_parts.iter().any(|s| *s < MIN_PART || *s != body_parts[0])
                || sizes.last().is_some_and(|l| *l > sizes[0])
            {
                return error(400, "EntityTooSmall");
            }
            let mut whole = Vec::new();
            let mut len = 0;
            for (n, _) in &asked {
                if let Some((d, l, _)) = m.parts.get(n) {
                    whole.extend_from_slice(d);
                    len += l;
                }
            }
            let key = m.key.clone();
            g.uploads.remove(&id);
            g.completed += 1;
            let etag = etag_of(&whole);
            g.objects.insert(
                key.clone(),
                Obj {
                    data: Bytes::from(whole),
                    len,
                    modified: now(),
                },
            );
            xml(
                200,
                format!(
                    "<CompleteMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Location>l</Location>\
                     <Bucket>{}</Bucket><Key>{key}</Key><ETag>{}</ETag></CompleteMultipartUploadResult>",
                    sh.opts.bucket,
                    etag.replace('"', "&quot;")
                ),
            )
        }
        "abort" => {
            let Some(id) = upload_id else {
                return error(400, "InvalidArgument");
            };
            if g.uploads.remove(&id).is_some() {
                g.aborted += 1;
                Response::builder().status(204).body(Body::empty()).expect("response")
            } else {
                error(404, "NoSuchUpload")
            }
        }
        "get" => match g.objects.get(&key) {
            Some(o) => Response::builder()
                .status(200)
                .header(header::CONTENT_LENGTH, o.len)
                .header(header::ETAG, etag_of(&o.data))
                .body(Body::from(o.data.clone()))
                .expect("response"),
            None => error(404, "NoSuchKey"),
        },
        "delete" => {
            g.objects.remove(&key);
            Response::builder().status(204).body(Body::empty()).expect("response")
        }
        _ => error(405, "MethodNotAllowed"),
    }
}

/// The harness's controls, over plain HTTP (the in-process tests call the methods instead).
fn admin_call(sh: &Arc<Shared>, method: &str, what: &str, raw_query: &str) -> Response {
    let p = |n: &str| param(raw_query, n);
    let mut g = sh.lock();
    match (method, what) {
        ("GET", "state") => {
            let t = now();
            let objects: Vec<serde_json::Value> = g
                .objects
                .iter()
                .map(|(k, o)| serde_json::json!({"key": k, "size": o.len, "age_s": t - o.modified}))
                .collect();
            let body = serde_json::json!({
                "objects": objects, "open_uploads": g.uploads.len(), "aborted": g.aborted, "completed": g.completed,
                "refused": g.refused, "ops": g.ops,
            });
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                body.to_string(),
            )
                .into_response()
        }
        ("POST", "fail") => {
            let (kind, status, times) = (
                p("kind").unwrap_or_default(),
                p("status").and_then(|s| s.parse().ok()).unwrap_or(500),
                p("times").and_then(|s| s.parse().ok()).unwrap_or(1),
            );
            g.faults.push(Fault { kind, status, times });
            StatusCode::NO_CONTENT.into_response()
        }
        ("POST", "reset") => {
            g.ops.clear();
            g.aborted = 0;
            g.completed = 0;
            g.refused = 0;
            StatusCode::NO_CONTENT.into_response()
        }
        ("POST", "delete") => {
            g.objects.remove(&p("key").unwrap_or_default());
            StatusCode::NO_CONTENT.into_response()
        }
        ("POST", "heal") => {
            g.faults.clear();
            StatusCode::NO_CONTENT.into_response()
        }
        ("POST", "age") => {
            let (key, secs) = (
                p("key").unwrap_or_default(),
                p("secs").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0),
            );
            if let Some(o) = g.objects.get_mut(&key) {
                o.modified -= secs;
            }
            StatusCode::NO_CONTENT.into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
