//! The bucket: path-style requests to one bucket, `https://<endpoint>/<bucket>/<key>`, each signed with `SigV4` in the
//! `Authorization` header (not presigned URLs), as `backup.py` makes them. Blocking, one request after another.
//!
//! [`ObjectStore`] is the five calls the job makes, so the run logic can be tested against a fake as the Python's
//! `_FakeS3` does. [`S3`] is the real client; it retries network errors and 5xx three times, waiting
//! `retry_unit * (attempt + 1)` after each, and never retries a download that has started writing into its file.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use reqwest::blocking::{Body, Client};
use reqwest::{Method, Url};

use crate::error::{BackupError, Result, fail};
use crate::settings::Vars;
use crate::sigv4::{Creds, EMPTY_SHA256, query_string, quote, sigv4};

/// The calls the backup makes on the bucket.
pub trait ObjectStore {
    /// PUT a file with its SHA-256 as the payload hash.
    fn put_file(&self, key: &str, path: &Path, sha256: &str) -> Result<()>;
    /// The stored size, or `None` for a missing key.
    fn head(&self, key: &str) -> Result<Option<u64>>;
    /// Server-side copy inside the bucket.
    fn copy(&self, src_key: &str, dst_key: &str) -> Result<()>;
    /// Delete one key.
    fn delete(&self, key: &str) -> Result<()>;
    /// Every `(key, size)` under a prefix.
    fn list(&self, prefix: &str) -> Result<Vec<(String, u64)>>;
}

/// The real thing.
#[derive(Debug)]
pub struct S3 {
    host: String,
    port: u16,
    bucket: String,
    access_key: String,
    secret_key: String,
    region: String,
    client: Client,
    /// The base of the retry wait: 2 s in the job, shorter in tests.
    pub retry_unit: Duration,
}

/// What a request answered.
#[derive(Debug)]
pub struct Reply {
    /// The HTTP status.
    pub status: u16,
    /// The headers, names lower-cased.
    pub headers: HashMap<String, String>,
    /// The body (empty when it was streamed to a writer).
    pub body: Vec<u8>,
    /// The bytes streamed to the writer, when there was one and the answer was 200.
    pub streamed: Option<u64>,
}

enum Payload<'a> {
    Empty,
    File { path: &'a Path, len: u64 },
}

/// One request to make: the method and key, and what rides with them.
struct Call<'a> {
    method: &'a str,
    key: &'a str,
    query: Vec<(String, String)>,
    payload: Payload<'a>,
    payload_hash: &'a str,
    extra: Vec<(&'a str, String)>,
    timeout: Duration,
}

impl<'a> Call<'a> {
    /// A body-less call with the usual five-minute timeout.
    fn new(method: &'a str, key: &'a str) -> Self {
        Self {
            method,
            key,
            query: Vec::new(),
            payload: Payload::Empty,
            payload_hash: EMPTY_SHA256,
            extra: Vec::new(),
            timeout: Duration::from_mins(5),
        }
    }
}

impl S3 {
    /// From `R2_ENDPOINT` (an https URL), `R2_BUCKET`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` and `R2_REGION`;
    /// `extra_roots_pem` are certificates trusted in addition to the built-in roots.
    pub fn from_vars(v: &Vars, extra_roots_pem: &[Vec<u8>]) -> Result<Self> {
        let ep = v.env("R2_ENDPOINT", None)?;
        let url = Url::parse(&ep)
            .ok()
            .filter(|u| u.scheme() == "https" && u.host_str().is_some());
        let Some(url) = url else {
            return fail("R2_ENDPOINT must be an https:// URL");
        };
        Ok(Self {
            host: url.host_str().unwrap_or_default().to_string(),
            port: url.port().unwrap_or(443),
            bucket: v.env("R2_BUCKET", None)?,
            access_key: v.env("R2_ACCESS_KEY_ID", None)?,
            secret_key: v.env("R2_SECRET_ACCESS_KEY", None)?,
            region: v.get_or("R2_REGION", "auto").to_string(),
            client: http_client(extra_roots_pem)?,
            retry_unit: Duration::from_secs(2),
        })
    }

    fn request(&self, call: &Call<'_>, mut out: Option<&mut dyn Write>) -> Result<Reply> {
        let Call {
            method,
            key,
            query,
            payload,
            payload_hash,
            extra,
            timeout,
        } = call;
        let (method, key, payload_hash, timeout) = (*method, *key, *payload_hash, *timeout);
        let path = if key.is_empty() {
            format!("/{}", self.bucket)
        } else {
            format!("/{}/{key}", self.bucket)
        };
        let shown = if key.is_empty() { "/" } else { key };
        let host = if self.port == 443 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        };
        let mut last: Option<BackupError> = None;
        for attempt in 0..3u32 {
            let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
            let mut headers: Vec<(String, String)> = vec![
                ("host".into(), host.clone()),
                ("x-amz-date".into(), amz_date.clone()),
                ("x-amz-content-sha256".into(), payload_hash.to_string()),
            ];
            headers.extend(extra.iter().map(|(k, v)| ((*k).to_string(), v.clone())));
            if let Payload::File { len, .. } = payload {
                headers.push(("content-length".into(), len.to_string()));
            }
            let creds = Creds {
                access_key: &self.access_key,
                secret_key: &self.secret_key,
                region: &self.region,
                service: "s3",
            };
            let auth = sigv4(method, &path, query, &headers, payload_hash, &creds, &amz_date);
            let mut url = quote(&path, "/");
            if !query.is_empty() {
                url.push('?');
                url.push_str(&query_string(query));
            }
            let full = format!("https://{}:{}{url}", self.host, self.port);
            let m = Method::from_bytes(method.as_bytes()).map_err(|e| BackupError(e.to_string()))?;
            let mut rb = self.client.request(m, &full).timeout(timeout);
            for (k, v) in &headers {
                if k != "host" && k != "content-length" {
                    rb = rb.header(k.as_str(), v.as_str());
                }
            }
            rb = rb.header("authorization", auth);
            match payload {
                Payload::Empty => {}
                Payload::File { path, len } => match File::open(path) {
                    Ok(f) => rb = rb.body(Body::sized(f, *len)),
                    Err(e) => return fail(format!("{method} {shown}: {}: {e}", io_kind(&e))),
                },
            }
            match rb.send() {
                Ok(mut r) => {
                    let status = r.status().as_u16();
                    if status >= 500 {
                        last = Some(BackupError(format!("{method} {shown}: HTTP {status}")));
                        let _ = r.bytes();
                        std::thread::sleep(self.retry_unit * (attempt + 1));
                        continue;
                    }
                    let headers = r
                        .headers()
                        .iter()
                        .map(|(k, v)| {
                            (
                                k.as_str().to_lowercase(),
                                String::from_utf8_lossy(v.as_bytes()).into_owned(),
                            )
                        })
                        .collect();
                    if let Some(w) = out.as_deref_mut()
                        && status == 200
                    {
                        let n = std::io::copy(&mut r, w)
                            .map_err(|e| BackupError(format!("{method} {shown}: {}: {e}", io_kind(&e))))?;
                        return Ok(Reply {
                            status,
                            headers,
                            body: Vec::new(),
                            streamed: Some(n),
                        });
                    }
                    let mut body = Vec::new();
                    r.read_to_end(&mut body)
                        .map_err(|e| BackupError(format!("{method} {shown}: {}: {e}", io_kind(&e))))?;
                    return Ok(Reply {
                        status,
                        headers,
                        body,
                        streamed: None,
                    });
                }
                Err(e) => {
                    last = Some(BackupError(format!("{method} {shown}: {}", describe(&e))));
                    if out.is_some() {
                        // a partial download is not retried into the same file
                        break;
                    }
                    std::thread::sleep(self.retry_unit * (attempt + 1));
                }
            }
        }
        Err(last.unwrap_or_else(|| BackupError(format!("{method} {shown}: no answer"))))
    }

    /// GET a key into `out`; the reply carries the byte count in `streamed` when the status is 200.
    pub fn get_into(&self, key: &str, out: &mut dyn Write, timeout: Duration) -> Result<Reply> {
        let mut call = Call::new("GET", key);
        call.timeout = timeout;
        self.request(&call, Some(out))
    }
}

impl ObjectStore for S3 {
    fn put_file(&self, key: &str, path: &Path, sha256: &str) -> Result<()> {
        let len = std::fs::metadata(path)
            .map_err(|e| BackupError(format!("{}: {e}", path.display())))?
            .len();
        let mut call = Call::new("PUT", key);
        call.payload = Payload::File { path, len };
        call.payload_hash = sha256;
        call.extra = vec![("content-type", "application/gzip".to_string())];
        call.timeout = Duration::from_mins(15);
        let r = self.request(&call, None)?;
        if r.status != 200 {
            return fail(format!("PUT {key}: HTTP {} {}", r.status, s3_error(&r.body)));
        }
        Ok(())
    }

    fn head(&self, key: &str) -> Result<Option<u64>> {
        let r = self.request(&Call::new("HEAD", key), None)?;
        match r.status {
            404 => Ok(None),
            200 => Ok(Some(
                r.headers
                    .get("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(u64::MAX),
            )),
            st => fail(format!("HEAD {key}: HTTP {st}")),
        }
    }

    fn copy(&self, src_key: &str, dst_key: &str) -> Result<()> {
        let source = quote(&format!("/{}/{src_key}", self.bucket), "/");
        let mut call = Call::new("PUT", dst_key);
        call.extra = vec![("x-amz-copy-source", source)];
        let r = self.request(&call, None)?;
        if r.status != 200 || contains(&r.body, b"<Error>") {
            return fail(format!("COPY -> {dst_key}: HTTP {} {}", r.status, s3_error(&r.body)));
        }
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let r = self.request(&Call::new("DELETE", key), None)?;
        if r.status != 200 && r.status != 204 {
            return fail(format!("DELETE {key}: HTTP {} {}", r.status, s3_error(&r.body)));
        }
        Ok(())
    }

    fn list(&self, prefix: &str) -> Result<Vec<(String, u64)>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut q = vec![
                ("list-type".to_string(), "2".to_string()),
                ("prefix".to_string(), prefix.to_string()),
                ("max-keys".to_string(), "1000".to_string()),
            ];
            if let Some(t) = &token {
                q.push(("continuation-token".to_string(), t.clone()));
            }
            let mut call = Call::new("GET", "");
            call.query = q;
            let r = self.request(&call, None)?;
            if r.status != 200 {
                return fail(format!("LIST {prefix}: HTTP {} {}", r.status, s3_error(&r.body)));
            }
            let (items, next) = parse_listing(&r.body)?;
            out.extend(items);
            match next {
                Listing::Done => return Ok(out),
                Listing::More(t) => token = Some(t),
                Listing::MissingToken => return fail("LIST: truncated listing without a continuation token"),
            }
        }
    }
}

/// Whether a listing has another page.
#[derive(Debug, PartialEq, Eq)]
pub enum Listing {
    /// `IsTruncated` is not `true`.
    Done,
    /// Truncated, with this `NextContinuationToken`.
    More(String),
    /// Truncated and no token: refused.
    MissingToken,
}

/// The `(key, size)` pairs of a `ListObjectsV2` answer and whether there is a next page.
pub fn parse_listing(body: &[u8]) -> Result<(Vec<(String, u64)>, Listing)> {
    let text = std::str::from_utf8(body).map_err(|_| BackupError("ParseError: the listing is not UTF-8".into()))?;
    if !text.trim_start().starts_with('<') || !text.contains("<ListBucketResult") {
        return fail("ParseError: the listing is not a ListBucketResult");
    }
    let mut items = Vec::new();
    for chunk in text.split("<Contents>").skip(1) {
        let block = chunk.split("</Contents>").next().unwrap_or(chunk);
        let key = element(block, "Key").map(|k| unescape(&k)).unwrap_or_default();
        let size = element(block, "Size").and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        items.push((key, size));
    }
    // the elements after the last </Contents> carry the paging
    let tail = text.rsplit("</Contents>").next().unwrap_or(text);
    let head = text.split("<Contents>").next().unwrap_or(text);
    let paging = |name: &str| element(tail, name).or_else(|| element(head, name));
    let truncated = paging("IsTruncated").is_some_and(|t| t.trim().eq_ignore_ascii_case("true"));
    if !truncated {
        return Ok((items, Listing::Done));
    }
    match paging("NextContinuationToken")
        .map(|t| unescape(&t))
        .filter(|t| !t.is_empty())
    {
        Some(t) => Ok((items, Listing::More(t))),
        None => Ok((items, Listing::MissingToken)),
    }
}

fn element(block: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let start = block.find(&open)? + open.len();
    let end = block[start..].find(&format!("</{name}>"))? + start;
    Some(block[start..end].to_string())
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let ent = &tail[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// `<Code>...</Code>` of an S3 error body, up to 80 characters.
pub fn s3_error(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    element(&text, "Code")
        .filter(|c| !c.is_empty() && c.len() <= 80 && !c.contains('<'))
        .unwrap_or_default()
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn io_kind(e: &std::io::Error) -> String {
    format!("{:?}", e.kind())
}

fn describe(e: &reqwest::Error) -> String {
    let mut s = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(inner) = src {
        s.push_str(": ");
        s.push_str(&inner.to_string());
        src = inner.source();
    }
    s
}

/// A blocking client with the built-in roots plus any extra PEM certificates.
pub fn http_client(extra_roots_pem: &[Vec<u8>]) -> Result<Client> {
    let mut b = Client::builder().connect_timeout(Duration::from_mins(5));
    for pem in extra_roots_pem {
        for cert in split_pem(pem) {
            let c = reqwest::Certificate::from_pem(cert.as_bytes())
                .map_err(|e| BackupError(format!("certificate: {e}")))?;
            b = b.add_root_certificate(c);
        }
    }
    b.build()
        .map_err(|e| BackupError(format!("http client: {}", describe(&e))))
}

/// The PEM blocks of a bundle, one string each.
fn split_pem(pem: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(pem);
    text.split("-----END CERTIFICATE-----")
        .filter_map(|part| {
            let start = part.find("-----BEGIN CERTIFICATE-----")?;
            Some(format!("{}-----END CERTIFICATE-----\n", &part[start..]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_with_a_next_page() {
        let xml = br#"<?xml version="1.0"?><ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>b</Name><IsTruncated>true</IsTruncated><NextContinuationToken>tok&amp;1</NextContinuationToken><Contents><Key>surreal/daily/a&amp;b</Key><Size>12</Size></Contents><Contents><Key>surreal/daily/c</Key><Size>7</Size></Contents></ListBucketResult>"#;
        let (items, next) = parse_listing(xml).unwrap();
        assert_eq!(
            items,
            vec![
                ("surreal/daily/a&b".to_string(), 12),
                ("surreal/daily/c".to_string(), 7)
            ]
        );
        assert_eq!(next, Listing::More("tok&1".to_string()));
    }

    #[test]
    fn a_truncated_listing_without_a_token_is_refused() {
        let xml = b"<ListBucketResult><IsTruncated>true</IsTruncated></ListBucketResult>";
        assert_eq!(parse_listing(xml).unwrap().1, Listing::MissingToken);
    }

    #[test]
    fn a_listing_that_is_not_xml_is_an_error() {
        assert!(parse_listing(b"<html>nope</html>").is_err());
        assert!(parse_listing(b"").is_err());
    }

    #[test]
    fn an_empty_listing() {
        let xml = b"<ListBucketResult><IsTruncated>false</IsTruncated><KeyCount>0</KeyCount></ListBucketResult>";
        let (items, next) = parse_listing(xml).unwrap();
        assert!(items.is_empty() && next == Listing::Done);
    }

    #[test]
    fn error_codes() {
        assert_eq!(s3_error(b"<Error><Code>AccessDenied</Code></Error>"), "AccessDenied");
        assert_eq!(s3_error(b"not xml"), "");
    }

    #[test]
    fn endpoint_must_be_https() {
        let v = Vars::from_pairs([
            ("R2_ENDPOINT", "http://x.example"),
            ("R2_BUCKET", "b"),
            ("R2_ACCESS_KEY_ID", "k"),
            ("R2_SECRET_ACCESS_KEY", "s"),
        ]);
        assert_eq!(
            S3::from_vars(&v, &[]).unwrap_err().0,
            "R2_ENDPOINT must be an https:// URL"
        );
    }
}
