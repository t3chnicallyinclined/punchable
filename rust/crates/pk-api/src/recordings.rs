//! The recordings bucket (T11 round 2, docs/adr/0015): where a set's recording waits for a transcriber.
//!
//! One bucket in R2, one token scoped to that bucket, path-style requests signed with `SigV4` (the backup job's shape,
//! `infra/backup/backup.py`), by `rusty-s3` over the `reqwest` client pk-api already has. Every request is a presigned
//! URL good for five minutes and used at once, so the signature sits in the query and a URL must never be logged or
//! returned: every error here is built from a status or a scrubbed transport error, never from the URL.
//!
//! * [`Recordings::put_stream`] streams a request body in: a recording of at most one part goes in with a single PUT,
//!   a bigger one as a multipart upload of [`PART`]-byte parts, so memory is one part per upload however long the
//!   recording is. A failed or abandoned upload aborts its multipart upload; nothing is ever listed before the caller
//!   writes the pointer row, which it does last.
//! * [`Recordings::get`] streams an object back.
//! * [`Recordings::delete`] and [`Recordings::list`] serve the release rules and the bucket sweep.
//! * [`Recordings::check`] is the start-up `HEAD` on the bucket: a wrong token or bucket keeps the pod unready.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures::stream::BoxStream;
use futures::{Stream, StreamExt as _, TryStreamExt as _};
use reqwest::StatusCode;
use rusty_s3::actions::{CompleteMultipartUploadResponse, CreateMultipartUpload, ListObjectsV2};
use rusty_s3::{Bucket, Credentials, S3Action as _, UrlStyle};

use crate::config::RecConfig;

/// The size of a part: R2 wants every part but the last to be the same size and at least 5 MiB.
pub const PART: usize = 8 * 1024 * 1024;
/// Every recording's key starts with this; the lifecycle rule and the bucket sweep look only here.
pub const KEY_PREFIX: &str = "rec/";
/// How long a presigned URL is good for: it is used the moment it is made.
const SIGN_FOR: Duration = Duration::from_secs(300);
/// A request is tried this many times on a transport error, a 5xx, a 408 or a 429.
const TRIES: u32 = 3;
/// The wait before each retry is this times the number of the failed try.
const BACKOFF: Duration = Duration::from_millis(250);
/// A request that moves one part (or one small body) gives up after this.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// What a recordings call can end in.
#[derive(Debug, thiserror::Error)]
pub enum RecError {
    /// The bucket did not do what was asked: unreachable, refused the token, or answered an error. The text names a
    /// status or a cause and never a URL, a key or a bucket.
    #[error("the recording store failed: {0}")]
    Store(String),
    /// The body being uploaded ended early, failed, or stalled: the phone lost signal. Nothing was kept.
    #[error("upload was cut short")]
    Cut,
}

fn store(what: &str, why: impl std::fmt::Display) -> RecError {
    RecError::Store(format!("{what}: {why}"))
}

/// A new recording's object key.
pub fn new_key() -> String {
    format!("{KEY_PREFIX}{}", uuid::Uuid::new_v4().simple())
}

/// What a finished upload left in the bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stored {
    /// Bytes stored.
    pub bytes: u64,
    /// Parts sent (1 for a single PUT).
    pub parts: i64,
}

/// An object coming back.
pub struct Download {
    /// Its size, from the bucket's own `Content-Length`.
    pub len: u64,
    /// Its bytes, in order; an error ends the stream (the receiver sees a truncated body, never a clean end).
    pub stream: BoxStream<'static, Result<Bytes, io::Error>>,
}

impl std::fmt::Debug for Download {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Download")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

/// One object in a listing.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    /// Its key.
    pub key: String,
    /// When it was last written, seconds since the epoch (`None` if the listing gave none).
    pub modified: Option<f64>,
}

/// The recordings bucket.
#[derive(Clone)]
pub struct Recordings {
    http: reqwest::Client,
    bucket: Arc<Bucket>,
    creds: Arc<Credentials>,
}

impl std::fmt::Debug for Recordings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recordings").finish_non_exhaustive()
    }
}

impl Recordings {
    /// A client for the configured bucket. Nothing is sent yet.
    pub fn new(cfg: &RecConfig) -> anyhow::Result<Self> {
        let endpoint = reqwest::Url::parse(&cfg.endpoint)?;
        // https only, but for the loopback stand-in of the tests (the config already refused any other http endpoint)
        let local = endpoint.scheme() == "http";
        let bucket = Bucket::new(endpoint, UrlStyle::Path, cfg.bucket.clone(), cfg.region.clone())?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            // a stalled download ends instead of hanging; the total time of a long one is not capped
            .read_timeout(Duration::from_secs(60))
            .pool_idle_timeout(Duration::from_secs(30))
            .https_only(!local)
            .build()?;
        Ok(Self {
            http,
            bucket: Arc::new(bucket),
            creds: Arc::new(Credentials::new(cfg.key_id.clone(), cfg.secret.clone())),
        })
    }

    /// Send the request `build` makes (signed afresh each try) until `accept` accepts the status; transport errors, 5xx,
    /// 408 and 429 are tried again, anything else is final. The accepted response comes back.
    async fn send(
        &self,
        what: &str,
        build: impl Fn() -> reqwest::RequestBuilder,
        accept: impl Fn(StatusCode) -> bool,
    ) -> Result<reqwest::Response, RecError> {
        let mut last = String::new();
        for attempt in 1..=TRIES {
            if attempt > 1 {
                tokio::time::sleep(BACKOFF * (attempt - 1)).await;
            }
            match build().send().await {
                Ok(r) if accept(r.status()) => return Ok(r),
                Ok(r) => {
                    let s = r.status();
                    last = format!("answered {}", s.as_u16());
                    if !(s.is_server_error() || s == StatusCode::REQUEST_TIMEOUT || s == StatusCode::TOO_MANY_REQUESTS)
                    {
                        // the cause (an S3 error code) is for the log; the caller's text carries only the status
                        let code = r.text().await.ok().and_then(|t| s3_code(&t));
                        tracing::warn!(
                            op = what,
                            status = s.as_u16(),
                            code = code.as_deref().unwrap_or("-"),
                            "recordings: refused"
                        );
                        return Err(store(what, last));
                    }
                }
                Err(e) => last = format!("no answer ({})", e.without_url()),
            }
            tracing::warn!(op = what, attempt, "recordings: {last}");
        }
        Err(store(what, last))
    }

    fn put_req(&self, url: reqwest::Url, body: Bytes) -> reqwest::RequestBuilder {
        self.http.put(url).timeout(REQUEST_TIMEOUT).body(body)
    }

    /// Stream `body` into the bucket under `key`. `idle` is how long the body may say nothing before the upload counts
    /// as cut short. A recording that fits one part is a single PUT; a longer one is a multipart upload, aborted if
    /// anything fails.
    pub async fn put_stream<S, E>(&self, key: &str, mut body: S, idle: Duration) -> Result<Stored, RecError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
    {
        let mut multipart: Option<Multipart> = None;
        match self.fill(key, &mut body, idle, &mut multipart).await {
            Ok(done) => Ok(done),
            Err(e) => {
                if let Some(m) = multipart {
                    self.abort(key, &m.id).await;
                }
                Err(e)
            }
        }
    }

    async fn fill<S, E>(
        &self,
        key: &str,
        body: &mut S,
        idle: Duration,
        multipart: &mut Option<Multipart>,
    ) -> Result<Stored, RecError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
    {
        let mut buf = BytesMut::with_capacity(PART + (PART >> 3));
        let mut total = 0_u64;
        loop {
            let next = tokio::time::timeout(idle, body.next())
                .await
                .map_err(|_| RecError::Cut)?;
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|_| RecError::Cut)?;
            total += chunk.len() as u64;
            buf.extend_from_slice(&chunk);
            // a part goes only once more has arrived behind it, so the last part is never empty
            while buf.len() > PART {
                let part = buf.split_to(PART).freeze();
                self.send_part(key, multipart, part).await?;
            }
        }
        if multipart.is_none() {
            let url = self.bucket.put_object(Some(&self.creds), key).sign(SIGN_FOR);
            let body = buf.freeze();
            self.send("put", || self.put_req(url.clone(), body.clone()), |s| s.is_success())
                .await?;
            return Ok(Stored { bytes: total, parts: 1 });
        }
        if !buf.is_empty() {
            let part = buf.freeze();
            self.send_part(key, multipart, part).await?;
        }
        let Some(m) = multipart.as_ref() else {
            return Err(store("complete", "no upload open"));
        };
        let (etags, id) = (m.etags.clone(), m.id.clone());
        let action =
            self.bucket
                .complete_multipart_upload(Some(&self.creds), key, &id, etags.iter().map(String::as_str));
        let url = action.sign(SIGN_FOR);
        let xml = action.body();
        let r = self
            .send(
                "complete",
                || self.http.post(url.clone()).timeout(REQUEST_TIMEOUT).body(xml.clone()),
                |s| s.is_success(),
            )
            .await?;
        // S3 can answer 200 and still have failed to complete the upload
        let text = r.text().await.map_err(|e| store("complete", e.without_url()))?;
        CompleteMultipartUploadResponse::parse(&text)
            .map_err(|_| store("complete", s3_code(&text).unwrap_or_else(|| "unreadable answer".into())))?;
        Ok(Stored {
            bytes: total,
            parts: i64::try_from(etags.len()).unwrap_or(i64::MAX),
        })
    }

    /// One part of a multipart upload (the first one opens it).
    async fn send_part(&self, key: &str, multipart: &mut Option<Multipart>, part: Bytes) -> Result<(), RecError> {
        if multipart.is_none() {
            let url = self
                .bucket
                .create_multipart_upload(Some(&self.creds), key)
                .sign(SIGN_FOR);
            let r = self
                .send(
                    "start",
                    || self.http.post(url.clone()).timeout(REQUEST_TIMEOUT),
                    |s| s.is_success(),
                )
                .await?;
            let text = r.text().await.map_err(|e| store("start", e.without_url()))?;
            let id = CreateMultipartUpload::parse_response(&text)
                .map_err(|_| store("start", "unreadable answer"))?
                .upload_id()
                .to_string();
            *multipart = Some(Multipart { id, etags: Vec::new() });
        }
        let m = multipart.as_mut().ok_or_else(|| store("part", "no upload open"))?;
        let number = u16::try_from(m.etags.len() + 1).map_err(|_| store("part", "too many parts"))?;
        let url = self
            .bucket
            .upload_part(Some(&self.creds), key, number, &m.id)
            .sign(SIGN_FOR);
        let r = self
            .send("part", || self.put_req(url.clone(), part.clone()), |s| s.is_success())
            .await?;
        let etag = r
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| store("part", "no ETag in the answer"))?
            .to_string();
        m.etags.push(etag);
        Ok(())
    }

    /// Abandon a multipart upload; best effort (the bucket's lifecycle rule aborts what this misses).
    async fn abort(&self, key: &str, id: &str) {
        let url = self
            .bucket
            .abort_multipart_upload(Some(&self.creds), key, id)
            .sign(SIGN_FOR);
        if let Err(e) = self
            .send(
                "abort",
                || self.http.delete(url.clone()).timeout(REQUEST_TIMEOUT),
                |s| s.is_success() || s == StatusCode::NOT_FOUND,
            )
            .await
        {
            tracing::warn!("recordings: a failed upload could not be aborted ({e}); the lifecycle rule will");
        }
    }

    /// The object `key`, streamed; `None` when the bucket has no such object.
    pub async fn get(&self, key: &str) -> Result<Option<Download>, RecError> {
        if !ours(key) {
            return Ok(None);
        }
        let url = self.bucket.get_object(Some(&self.creds), key).sign(SIGN_FOR);
        let r = self
            .send(
                "get",
                || self.http.get(url.clone()),
                |s| s.is_success() || s == StatusCode::NOT_FOUND,
            )
            .await?;
        if r.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let len = r.content_length().ok_or_else(|| store("get", "no Content-Length"))?;
        let stream = r
            .bytes_stream()
            .map_err(|e| io::Error::other(e.without_url().to_string()))
            .boxed();
        Ok(Some(Download { len, stream }))
    }

    /// Delete the object `key`; one that is not there counts as deleted.
    pub async fn delete(&self, key: &str) -> Result<(), RecError> {
        if !ours(key) {
            return Err(store("delete", "not a recording key"));
        }
        let url = self.bucket.delete_object(Some(&self.creds), key).sign(SIGN_FOR);
        self.send(
            "delete",
            || self.http.delete(url.clone()).timeout(REQUEST_TIMEOUT),
            |s| s.is_success() || s == StatusCode::NOT_FOUND,
        )
        .await?;
        Ok(())
    }

    /// Every object under [`KEY_PREFIX`], page by page.
    pub async fn list(&self) -> Result<Vec<Listed>, RecError> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut action = self.bucket.list_objects_v2(Some(&self.creds));
            action.with_prefix(KEY_PREFIX);
            if let Some(t) = &token {
                action.with_continuation_token(t.clone());
            }
            let url = action.sign(SIGN_FOR);
            let r = self
                .send(
                    "list",
                    || self.http.get(url.clone()).timeout(REQUEST_TIMEOUT),
                    |s| s.is_success(),
                )
                .await?;
            let text = r.text().await.map_err(|e| store("list", e.without_url()))?;
            let page = ListObjectsV2::parse_response(&text).map_err(|_| store("list", "unreadable answer"))?;
            out.extend(page.contents.into_iter().map(|c| {
                Listed {
                    key: c.key,
                    modified: c
                        .last_modified
                        .as_deref()
                        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                        .map(|t| t.timestamp() as f64 + f64::from(t.timestamp_subsec_millis()) / 1000.0),
                }
            }));
            token = page.next_continuation_token;
            if token.is_none() {
                return Ok(out);
            }
        }
    }

    /// `HEAD` on the bucket: it exists and this token may reach it.
    pub async fn check(&self) -> Result<(), RecError> {
        let url = self.bucket.head_bucket(Some(&self.creds)).sign(SIGN_FOR);
        self.send(
            "head",
            || self.http.head(url.clone()).timeout(Duration::from_secs(10)),
            |s| s.is_success(),
        )
        .await?;
        Ok(())
    }
}

/// Ask the bucket until it answers, so a wrong token or bucket keeps the pod unready (the rollout stops before traffic
/// moves) and a bucket that is only slow to come up does not need a restart. Stops at the first yes.
pub async fn probe(st: crate::state::AppState) {
    while !st.check_recordings().await {
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    tracing::info!("recordings: the bucket answered");
}

/// Only keys this service made are ever read or deleted: a pointer row restored from elsewhere cannot aim a request
/// at another object.
fn ours(key: &str) -> bool {
    key.starts_with(KEY_PREFIX) && key.len() > KEY_PREFIX.len() && !key.contains("..")
}

/// A multipart upload in progress.
struct Multipart {
    id: String,
    etags: Vec<String>,
}

/// The `<Code>` of an S3 error body, for the log (never the message, which may name the bucket).
fn s3_code(body: &str) -> Option<String> {
    let start = body.find("<Code>")? + "<Code>".len();
    let end = body[start..].find("</Code>")? + start;
    let code = &body[start..end];
    (code.len() <= 64 && code.chars().all(|c| c.is_ascii_alphanumeric())).then(|| code.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_short_plain_code_is_taken_from_an_error_body() {
        assert_eq!(
            s3_code("<Error><Code>NoSuchKey</Code><Message>m</Message></Error>").as_deref(),
            Some("NoSuchKey")
        );
        assert_eq!(s3_code("<Error><Code>a b</Code></Error>"), None);
        assert_eq!(s3_code("no xml"), None);
    }

    #[test]
    fn a_new_key_is_under_the_prefix_and_never_repeats() {
        let (a, b) = (new_key(), new_key());
        assert!(a.starts_with(KEY_PREFIX) && a != b);
        assert!(a.len() < 64 && a.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'/'));
    }
}
