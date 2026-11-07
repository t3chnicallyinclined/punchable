//! Uploads, the inbox and the transcriber's fetch (T10 group 6a; `server.py` `upload`, `inbox_list`,
//! `creative_api.Routes.w_file`): `POST /api/upload` (the phone's recording, streamed into the database a megabyte at
//! a time, then queued as a `process_set` job for the transcriber), `GET /api/inbox` (what waits) and `GET
//! /api/worker/jobs/<id>/file` (the bytes back). The bytes live in `upload_part` rows and the commit is the
//! `upload:<name>` row (`pk_store::upload`; `api-dev/upload_store.py` writes and reads the same rows), so either
//! server sees every upload and the pod's disk holds none.
//!
//! A write runs to its end once started ([`detached`]): a phone that loses signal mid-body ends the stream, the parts
//! already written are dropped and the answer is "upload was cut short", as the Python's.

use std::sync::LazyLock;

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::StreamExt as _;
use pk_core::py;
use pk_creative::Items;
use pk_store::Store;
use pk_store::upload::{PART, UploadRow};
use regex::Regex;
use serde_json::{Map, Value, json};

use super::jobs::{brief, enqueue, waiting_note};
use super::{Creative, detached, http, not_connected, store_err};
use crate::auth::Caller;
use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::state::AppState;

/// The largest upload when the database is its home (docs/adr/0014): 64 MiB. Audio of a set is tens of megabytes; a
/// Film-it video never leaves the phone. Over it is a 413, on both servers while both exist (`server.py` `upload`).
pub const MAX_UPLOAD: i64 = 64 * 1024 * 1024;
/// `AUDIO_EXT`.
const AUDIO_EXT: [&str; 12] = [
    ".webm", ".m4a", ".mp3", ".wav", ".ogg", ".opus", ".mp4", ".mov", ".aac", ".flac", ".caf", ".3gp",
];
/// `recent_duplicate`'s window.
const DUPLICATE_WINDOW_S: f64 = 120.0;

/// An upload's file name as the worker's routes accept it (`^[A-Za-z0-9._\-]{1,200}$`).
static NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A[A-Za-z0-9._\-]{1,200}\z").unwrap_or_else(|e| unreachable!("constant regex: {e}")));
/// `upload_extras`'s `X-Recorded-At`.
static RECORDED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A\d{4}-\d{2}-\d{2}(?:[T ][0-9:.]+)?(?:Z|[+-]\d{2}:?\d{2})?\z")
        .unwrap_or_else(|e| unreachable!("constant regex: {e}"))
});

/// A header as the Python reads it (latin-1: any byte is a character); empty when absent.
fn hdr(h: &HeaderMap, name: &str) -> String {
    h.get(name)
        .map(|v| v.as_bytes().iter().map(|b| char::from(*b)).collect())
        .unwrap_or_default()
}

/// `urllib.parse.unquote`: percent escapes decoded as UTF-8, a bad sequence replaced.
fn unquote(s: &str) -> String {
    percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// `s[:n]` over characters.
fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `float(s)` for a header: a finite number, else none (the Python also reads `inf` and `nan`, which no recording has).
fn float_of(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|f| f.is_finite())
}

/// `Path(name).suffix.lower()`: from the last dot when it is neither first nor last.
fn suffix_of(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if i > 0 && i + 1 < base.len() => base[i..].to_lowercase(),
        _ => String::new(),
    }
}

/// `Path(name).stem`.
pub(crate) fn stem_of(name: &str) -> &str {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if i > 0 && i + 1 < base.len() => &base[..i],
        _ => base,
    }
}

/// The extension a content type stands for when the file name has none of the audio ones.
fn ext_of_type(ct: &str) -> Option<&'static str> {
    Some(match ct {
        "audio/webm" | "video/webm" => ".webm",
        "audio/mp4" => ".m4a",
        "audio/mpeg" => ".mp3",
        "audio/wav" => ".wav",
        "audio/ogg" => ".ogg",
        "video/mp4" => ".mp4",
        "video/quicktime" => ".mov",
        _ => return None,
    })
}

/// `re.sub(r"[^a-z0-9]+", "-", stem.lower()).strip("-")[:40] or "recording"`.
fn slug(stem: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in stem.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c);
        } else {
            dash = true;
        }
    }
    let s = cut(&out, 40);
    if s.is_empty() { "recording".to_string() } else { s }
}

/// `X-Marks-Ms`: `json.loads` of a list of numbers or numeric text, `int()` of each, 500 at most; anything else is none.
fn marks_of(raw: &str) -> Vec<Value> {
    let text = if raw.is_empty() { "[]" } else { raw };
    let Ok(Value::Array(xs)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(xs.len().min(500));
    for x in &xs {
        let n = match x {
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)),
            Value::String(s) => py::int_saturating(s).ok(),
            Value::Bool(b) => Some(i64::from(*b)),
            _ => None,
        };
        match n {
            Some(n) => out.push(json!(n)),
            None => return Vec::new(),
        }
    }
    out.truncate(500);
    out
}

/// `upload_extras(h)`: the optional capture metadata, anything malformed dropped.
fn extras(h: &HeaderMap, out: &mut Map<String, Value>) {
    let ra = py::strip(&hdr(h, "X-Recorded-At")).to_string();
    if !ra.is_empty() && RECORDED_RE.is_match(&ra) {
        out.insert("recorded_at".into(), json!(ra));
    }
    for (name, key) in [
        ("X-Venue", "venue"),
        ("X-Show-Group", "show_group"),
        ("X-Align-Working-Set", "align_working_set"),
    ] {
        let v = cut(py::strip(&unquote(&hdr(h, name))), 200);
        if !v.is_empty() {
            out.insert(key.into(), json!(v));
        }
    }
    if let (Some(lat), Some(lng)) = (float_of(&hdr(h, "X-Lat")), float_of(&hdr(h, "X-Lng")))
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lng)
    {
        out.insert("gps".into(), json!({"lat": lat, "lon": lng}));
    }
}

/// The inbox meta.json (`Handler.upload_meta`), in the Python's key order.
fn upload_meta(h: &HeaderMap, kind: &str, orig: &str, n: i64) -> Map<String, Value> {
    let title = cut(&unquote(&hdr(h, "X-Title")), 200);
    let dur = float_of(&hdr(h, "X-Duration-S")).filter(|d| *d != 0.0);
    let mut m = Map::new();
    m.insert("kind".into(), json!(kind));
    m.insert(
        "original_name".into(),
        if orig.is_empty() { Value::Null } else { json!(orig) },
    );
    m.insert(
        "title".into(),
        if title.is_empty() { Value::Null } else { json!(title) },
    );
    m.insert("received_at".into(), json!(pk_core::time::now_iso()));
    m.insert("bytes".into(), json!(n));
    m.insert("duration_s".into(), dur.map_or(Value::Null, |d| json!(d)));
    m.insert("marks_ms".into(), Value::Array(marks_of(&hdr(h, "X-Marks-Ms"))));
    m.insert("status".into(), json!("queued"));
    m.insert("by".into(), json!("punchable-api-dev"));
    extras(h, &mut m);
    m
}

/// `recent_duplicate(jobs, meta, now)`: the queued `process_set` job of an identical upload (same bytes, duration,
/// recording time and title) made within two minutes: the phone posted one recording twice.
fn recent_duplicate<'a>(jobs: &'a Items, meta: &Map<String, Value>, now: f64) -> Option<&'a Map<String, Value>> {
    const KEYS: [&str; 4] = ["bytes", "duration_s", "recorded_at", "title"];
    if !KEYS.iter().any(|k| py::truthy(meta.get(*k))) {
        return None;
    }
    jobs.values().filter_map(Value::as_object).find(|j| {
        if j.get("kind").and_then(Value::as_str) != Some("process_set")
            || j.get("status").and_then(Value::as_str) != Some("queued")
        {
            return false;
        }
        let theirs = j.get("input").and_then(|i| i.get("meta")).and_then(Value::as_object);
        let same = KEYS
            .iter()
            .all(|k| theirs.and_then(|m| m.get(*k)).unwrap_or(&Value::Null) == meta.get(*k).unwrap_or(&Value::Null));
        let t = crate::worker::iso_ts(j.get("created_at").and_then(Value::as_str));
        same && t != 0.0 && (0.0..=DUPLICATE_WINDOW_S).contains(&(now - t))
    })
}

fn cut_short() -> ApiError {
    ApiError::bad_request("upload was cut short")
}

/// The body into `upload_part` rows, `PART` bytes each; how many parts.
async fn stage(store: &Store, token: &str, body: Body, n: usize) -> ApiResult<i64> {
    // best effort: concurrent uploads each sweep the same stale staging rows and can lose a write conflict (seen under
    // the same-rate probe as a 500 on the upload that happened to sweep); the next upload sweeps again
    if let Err(e) = store.upload_sweep_orphans().await {
        tracing::warn!(error = %e, "upload: the orphan sweep failed; the next upload tries again");
    }
    let mut stream = body.into_data_stream();
    let mut buf: Vec<u8> = Vec::with_capacity(PART.min(n));
    let (mut seq, mut got) = (0_i64, 0_usize);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| cut_short())?;
        got += chunk.len();
        let mut rest = &chunk[..];
        while !rest.is_empty() {
            let take = (PART - buf.len()).min(rest.len());
            buf.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if buf.len() == PART {
                let part = std::mem::replace(&mut buf, Vec::with_capacity(PART.min(n - got.min(n))));
                store.upload_put_part(token, seq, part).await.map_err(store_err)?;
                seq += 1;
            }
        }
    }
    if !buf.is_empty() {
        store.upload_put_part(token, seq, buf).await.map_err(store_err)?;
        seq += 1;
    }
    if got < n {
        return Err(cut_short());
    }
    Ok(seq)
}

/// `POST /api/upload`: the recording in the body, its kind and name in the headers; answers `queued` with the file
/// and the job that waits for a transcriber (or the first file's job when the same recording was posted twice).
pub async fn upload(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    if !st.config().upload_db {
        // a path moved here must never silently diverge from the Python, which keeps the upload in its inbox folder
        tracing::error!(
            "POST /api/upload refused: uploads in the database are off (PK_UPLOAD_DB is not 1) but this path is routed here"
        );
        return Err(ApiError::unavailable(
            "uploads in the database are off (PK_UPLOAD_DB); route /api/upload to the Python API",
        ));
    }
    if !st.schema_extra_current() {
        // the `upload` tables are another build's (a deploy in progress): never write rows the schema does not describe
        return Err(ApiError::unavailable(
            "the database schema is not the one this build expects (a deploy is in progress); try again shortly",
        ));
    }
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let n = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    if n <= 0 {
        return Err(ApiError::bad_request("upload missing"));
    }
    if n > MAX_UPLOAD {
        return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "upload over 64 MiB"));
    }
    let kind = {
        let k = py::strip(&hdr(&headers, "X-Kind")).to_string();
        if k.is_empty() { "upload".to_string() } else { k }
    };
    if !matches!(kind.as_str(), "set" | "note" | "upload") {
        return Err(ApiError::bad_request("X-Kind must be set|note|upload"));
    }
    let orig = unquote(&hdr(&headers, "X-Filename"));
    let mut ext = suffix_of(&orig);
    if !AUDIO_EXT.contains(&ext.as_str()) {
        let ct = hdr(&headers, "Content-Type");
        let ct = py::strip(ct.split(';').next().unwrap_or(""));
        ext = ext_of_type(ct)
            .ok_or_else(|| ApiError::bad_request("only audio or video files can be uploaded"))?
            .to_string();
    }
    let now = chrono::Local::now();
    let name = format!(
        "{}_punchable-{kind}_{}_{}{ext}",
        now.format("%Y-%m-%d"),
        now.format("%H%M%S"),
        slug(stem_of(&orig))
    );
    let meta_headers = headers.clone();
    let out = detached(async move { stage_and_queue(st, body, n, kind, orig, name, meta_headers).await }).await?;
    Ok(ok(&out))
}

async fn stage_and_queue(
    st: AppState,
    body: Body,
    n: i64,
    kind: String,
    orig: String,
    name: String,
    headers: HeaderMap,
) -> ApiResult<Value> {
    let store = st.store_handle().ok_or_else(not_connected)?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let committed = async {
        let parts = stage(&store, &token, body, usize::try_from(n).unwrap_or(usize::MAX)).await?;
        let meta = upload_meta(&headers, &kind, &orig, n);
        let used = store
            .upload_commit(&name, &token, &Value::Object(meta.clone()), n, parts)
            .await
            .map_err(store_err)?;
        Ok::<_, ApiError>((used, meta))
    }
    .await;
    let (used, meta) = match committed {
        Ok(x) => x,
        Err(e) => {
            // never a part that belongs to nothing
            if let Err(d) = store.upload_drop_parts(&token).await {
                tracing::warn!(error = %d, "upload: the parts of a failed upload could not be dropped");
            }
            return Err(e);
        }
    };
    let queued = async {
        let mut out = Map::new();
        out.insert("status".into(), json!("queued"));
        out.insert("file".into(), json!(used));
        out.insert("kind".into(), json!(kind));
        out.insert("bytes".into(), json!(n));
        // queued as process_set: no worker transcribes any more (the phone does), so the job says so, never a silent drop
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let (_, items) = cr.lines_and_items("jobs").await?;
        if let Some(dup) = recent_duplicate(&items, &meta, crate::worker::unix_now()) {
            store.upload_delete(&used).await.map_err(store_err)?;
            let file = dup
                .get("input")
                .and_then(|i| i.get("file"))
                .cloned()
                .unwrap_or(Value::Null);
            out.insert("file".into(), file);
            out.insert("job".into(), brief(dup));
            out.insert("duplicate".into(), json!(true));
            if let Some(w) = dup.get("waiting_for").filter(|w| py::truthy(Some(w))) {
                out.insert("waiting_for".into(), w.clone());
                out.insert("note".into(), w.as_str().map_or(Value::Null, waiting_note));
            }
            return Ok(Value::Object(out));
        }
        let owner = st.config().owner_id.clone();
        let (_, q) = enqueue(
            &cr,
            "process_set",
            &format!("process|{used}"),
            json!({"file": used, "meta": meta}),
            false,
            &owner,
        )
        .await?;
        out.insert("job".into(), q.get("job").cloned().unwrap_or(Value::Null));
        if let Some(w) = q.get("waiting_for").filter(|w| py::truthy(Some(w))) {
            out.insert("waiting_for".into(), w.clone());
            out.insert("note".into(), q.get("note").cloned().unwrap_or(Value::Null));
        }
        Ok::<Value, ApiError>(Value::Object(out))
    }
    .await;
    if queued.is_err()
        && let Err(d) = store.upload_delete(&used).await
    {
        // the job was not queued and the phone will send the recording again: this copy must not stay (the sweep
        // finds what is left by its staging row)
        tracing::warn!(error = %d, file = %used, "upload: not deleted after a failed enqueue");
    }
    queued
}

/// `inbox_row(name, bytes, meta)`: one waiting upload as the app lists it (hosted: waiting for the transcriber).
pub fn inbox_row(u: &UploadRow) -> Value {
    let get = |k: &str| u.meta.get(k).filter(|v| py::truthy(Some(v))).cloned();
    let mut row = Map::new();
    row.insert("file".into(), json!(u.name));
    row.insert("bytes".into(), json!(u.bytes));
    row.insert("kind".into(), get("kind").unwrap_or_else(|| json!("upload")));
    row.insert("title".into(), get("title").unwrap_or_else(|| json!(u.name)));
    row.insert(
        "received_at".into(),
        u.meta.get("received_at").cloned().unwrap_or(Value::Null),
    );
    row.insert(
        "duration_s".into(),
        u.meta.get("duration_s").cloned().unwrap_or(Value::Null),
    );
    row.insert("marks_ms".into(), get("marks_ms").unwrap_or_else(|| json!([])));
    row.insert("status".into(), json!("queued"));
    row.insert("waiting_for".into(), json!("transcriber"));
    row.insert("note".into(), waiting_note("transcriber"));
    Value::Object(row)
}

/// `GET /api/inbox`: the uploads waiting for a transcriber, newest first.
pub async fn inbox(State(st): State<AppState>, req: axum::extract::Request) -> ApiResult<Response> {
    if !st.config().upload_db {
        return Ok(crate::proxy::forward(State(st), req).await);
    }
    let q = Query::parse(req.uri().query());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let store = st.store_handle().ok_or_else(not_connected)?;
    let rows = store.upload_rows().await.map_err(store_err)?;
    Ok(ok(&json!({"items": rows.iter().map(inbox_row).collect::<Vec<_>>()})))
}

/// A job that will not run again lets go of its recording (ADR 0014): the bytes of a `process_set` job that was
/// dropped, or that failed for good, are deleted. A failed delete is logged and left to the expiry sweep: the job's
/// own end has already been written and the caller must answer it.
pub async fn release_for_job(st: &AppState, job: &Map<String, Value>) {
    if !st.config().upload_db || job.get("kind").and_then(Value::as_str) != Some("process_set") {
        return;
    }
    let (Some(name), Some(store)) = (upload_name(job), st.store_handle()) else {
        return;
    };
    if let Err(e) = store.upload_delete(&name).await {
        tracing::warn!(error = %e, file = %name, "upload: not deleted after its job ended; the expiry sweep will find it");
    }
}

/// The upload a `process_set` job names, when its name is a plain file name.
pub fn upload_name(job: &Map<String, Value>) -> Option<String> {
    let name = job.get("input").and_then(|i| i.get("file")).and_then(Value::as_str)?;
    NAME_RE.is_match(name).then(|| name.to_string())
}

/// `GET /api/worker/jobs/<id>/file`: the upload's bytes, part by part (`send_file`'s headers).
pub async fn file(
    State(st): State<AppState>,
    Path(jid): Path<String>,
    req: axum::extract::Request,
) -> ApiResult<Response> {
    if !st.config().upload_db {
        return Ok(crate::proxy::forward(State(st), req).await);
    }
    let q = Query::parse(req.uri().query());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let items = Creative::new(st.clone()).items("jobs").await?;
    let job = items
        .get(&jid)
        .and_then(Value::as_object)
        .filter(|j| j.get("kind").and_then(Value::as_str) == Some("process_set"))
        .ok_or_else(|| http(404, "no such upload job"))?;
    let gone = || http(404, "the upload is gone (the pod restarted?)");
    let name = upload_name(job).ok_or_else(gone)?;
    let store = st.store_handle().ok_or_else(not_connected)?;
    let row = store.upload_get(&name).await.map_err(store_err)?.ok_or_else(gone)?;
    let (len, parts, token) = (row.bytes, row.parts, row.token);
    let stream = futures::stream::try_unfold(0_i64, move |seq| {
        let (store, token) = (store.clone(), token.clone());
        async move {
            if seq >= parts {
                return Ok::<_, std::io::Error>(None);
            }
            match store.upload_part(&token, seq).await {
                Ok(Some(data)) => Ok(Some((axum::body::Bytes::from(data), seq + 1))),
                Ok(None) => Err(std::io::Error::other(format!("upload part {seq} is missing"))),
                Err(e) => Err(std::io::Error::other(e.to_string())),
            }
        }
    });
    let mut resp = Response::new(Body::from_stream(stream));
    *resp.status_mut() = StatusCode::OK;
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len.max(0) as u64));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(resp.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_is_the_python_s() {
        assert_eq!(suffix_of("Take 1.M4A"), ".m4a");
        assert_eq!(suffix_of("noext"), "");
        assert_eq!(suffix_of(".hidden"), "");
        assert_eq!(suffix_of("a."), "");
        assert_eq!(stem_of("dir/Take 1.m4a"), "Take 1");
        assert_eq!(slug("Take 1 (Final)!"), "take-1-final");
        assert_eq!(slug("!!!"), "recording");
        assert_eq!(slug(&"a".repeat(60)).len(), 40);
    }

    #[test]
    fn marks_are_ints_up_to_500_or_none() {
        assert_eq!(marks_of(""), Vec::<Value>::new());
        assert_eq!(
            marks_of("[1, 2.9, \"3\", true]"),
            vec![json!(1), json!(2), json!(3), json!(1)]
        );
        assert_eq!(marks_of("[1, null]"), Vec::<Value>::new());
        assert_eq!(marks_of("{\"a\":1}"), Vec::<Value>::new());
        assert_eq!(marks_of("nope"), Vec::<Value>::new());
        let many = format!("[{}]", vec!["1"; 600].join(","));
        assert_eq!(marks_of(&many).len(), 500);
    }

    #[test]
    fn the_same_recording_twice_is_one_job() {
        let meta = json!({"bytes": 5, "duration_s": 2.5, "recorded_at": null, "title": "set"});
        let meta = meta.as_object().cloned().unwrap_or_default();
        let now = 1_000_000.0;
        let at = |s: f64| chrono::DateTime::from_timestamp(s as i64, 0).map(|t| t.to_rfc3339());
        let job = |id: &str, status: &str, age: f64| {
            json!({"id": id, "kind": "process_set", "status": status, "created_at": at(now - age),
                   "input": {"file": "a.m4a", "meta": meta}})
        };
        let items: Items = json!({"job:1": job("job:1", "queued", 30.0)})
            .as_object()
            .cloned()
            .unwrap_or_default();
        assert!(recent_duplicate(&items, &meta, now).is_some());
        let old: Items = json!({"job:1": job("job:1", "queued", 300.0)})
            .as_object()
            .cloned()
            .unwrap_or_default();
        assert!(recent_duplicate(&old, &meta, now).is_none());
        let claimed: Items = json!({"job:1": job("job:1", "claimed", 30.0)})
            .as_object()
            .cloned()
            .unwrap_or_default();
        assert!(recent_duplicate(&claimed, &meta, now).is_none());
        let empty = json!({"bytes": null}).as_object().cloned().unwrap_or_default();
        assert!(recent_duplicate(&items, &empty, now).is_none());
    }
}
