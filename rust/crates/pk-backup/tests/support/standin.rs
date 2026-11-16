//! A small HTTPS S3 server for the backup tests and the differential (T11 round 7): path-style, in memory, one bucket,
//! the calls the backup makes and nothing else (PUT object, server-side COPY, HEAD, GET, DELETE, ListObjectsV2 with
//! paging). It is a stand-in, not R2, but it checks what R2 checks that matters here: the SigV4 `Authorization` header
//! of every request against its secret (rebuilt here, not with the crate's own signer), the date's freshness, the
//! bucket, and that a PUT's `x-amz-content-sha256` is the SHA-256 of the bytes that arrived.
//!
//! It can be told to fail a kind of call, to store only half of an upload, and to cut a download short. Its
//! certificate is made at test time (a throwaway CA and a leaf for `localhost`); `ca_pem` is what a client trusts.
//!
//! The server lives in threads (rustls's blocking API), so a test can use it from a plain `#[test]`. Included by path
//! from the integration tests; never part of the library.

#![allow(dead_code, missing_docs, clippy::pedantic)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use pk_backup::sigv4::{hmac_sha256, sha256_hex};
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// How to start one.
#[derive(Debug, Clone)]
pub struct Options {
    pub bucket: String,
    pub key_id: String,
    pub secret: String,
    /// Keys per listing page.
    pub page: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            bucket: "test-bucket".into(),
            key_id: "TESTKEYID".into(),
            secret: "test-secret-not-a-real-one".into(),
            page: 1000,
        }
    }
}

/// A throwaway CA and a server certificate for `localhost` signed by it.
pub struct Pki {
    pub ca_pem: String,
    leaf_der: Vec<u8>,
    key_der: Vec<u8>,
}

impl Pki {
    pub fn new() -> Self {
        let ca_key = KeyPair::generate().expect("ca key");
        let mut ca = CertificateParams::new(Vec::<String>::new()).expect("ca params");
        ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        ca.distinguished_name.push(DnType::CommonName, "pk-backup test CA");
        let ca_cert = ca.self_signed(&ca_key).expect("ca cert");
        let leaf_key = KeyPair::generate().expect("leaf key");
        let mut leaf = CertificateParams::new(vec!["localhost".to_string()]).expect("leaf params");
        leaf.use_authority_key_identifier_extension = true;
        let issuer = Issuer::new(ca, ca_key);
        let leaf_cert = leaf.signed_by(&leaf_key, &issuer).expect("leaf cert");
        Self {
            ca_pem: ca_cert.pem(),
            leaf_der: leaf_cert.der().to_vec(),
            key_der: leaf_key.serialize_der(),
        }
    }

    pub fn server_config(&self) -> Arc<rustls::ServerConfig> {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_der.clone()));
        let cfg = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_no_client_auth()
            .with_single_cert(vec![CertificateDer::from(self.leaf_der.clone())], key)
            .expect("server cert");
        Arc::new(cfg)
    }
}

#[derive(Debug, Clone)]
struct Obj {
    data: Vec<u8>,
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
    faults: Vec<Fault>,
    ops: Vec<String>,
    refused: usize,
    short_store: bool,
    cut_get: bool,
    page: usize,
}

struct Shared {
    opts: Options,
    inner: Mutex<Inner>,
    stop: AtomicBool,
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
    pub ca_pem: String,
    shared: Arc<Shared>,
    addr: SocketAddr,
}

impl Drop for Standin {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr); // unblock the accept
    }
}

impl Standin {
    /// Start one on a free loopback port with this PKI.
    pub fn start(pki: &Pki, opts: Options) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the stand-in");
        let addr = listener.local_addr().expect("addr");
        let shared = Arc::new(Shared {
            opts: opts.clone(),
            inner: Mutex::new(Inner {
                page: opts.page,
                ..Inner::default()
            }),
            stop: AtomicBool::new(false),
        });
        let cfg = pki.server_config();
        let sh = shared.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if sh.stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(tcp) = conn else { continue };
                let (sh, cfg) = (sh.clone(), cfg.clone());
                std::thread::spawn(move || serve(&sh, &cfg, tcp));
            }
        });
        Self {
            endpoint: format!("https://localhost:{}", addr.port()),
            bucket: opts.bucket,
            key_id: opts.key_id,
            secret: opts.secret,
            ca_pem: pki.ca_pem.clone(),
            shared,
            addr,
        }
    }

    /// `(key, size)` of every stored object.
    pub fn objects(&self) -> Vec<(String, usize)> {
        self.shared
            .lock()
            .objects
            .iter()
            .map(|(k, o)| (k.clone(), o.data.len()))
            .collect()
    }

    pub fn keys(&self) -> Vec<String> {
        self.shared.lock().objects.keys().cloned().collect()
    }

    pub fn bytes(&self, key: &str) -> Option<Vec<u8>> {
        self.shared.lock().objects.get(key).map(|o| o.data.clone())
    }

    /// Put an object in directly (what an earlier night left, or something that is not ours).
    pub fn plant(&self, key: &str, data: &[u8]) {
        self.shared
            .lock()
            .objects
            .insert(key.into(), Obj { data: data.to_vec() });
    }

    /// Every operation seen, in order (`PUT key`, `HEAD key`, `COPY src dst`, `GET key`, `DELETE key`, `LIST prefix`).
    pub fn ops(&self) -> Vec<String> {
        self.shared.lock().ops.clone()
    }

    pub fn count(&self, kind: &str) -> usize {
        let tag = kind.to_uppercase();
        self.shared
            .lock()
            .ops
            .iter()
            .filter(|o| o.split(' ').next() == Some(tag.as_str()))
            .count()
    }

    pub fn clear_ops(&self) {
        self.shared.lock().ops.clear();
    }

    /// Requests refused for a bad signature, credential, bucket, date or payload hash.
    pub fn refused(&self) -> usize {
        self.shared.lock().refused
    }

    /// Answer `status` to the next `times` calls of `kind` (all of them when `times` is negative).
    pub fn fail(&self, kind: &str, status: u16, times: i64) {
        self.shared.lock().faults.push(Fault {
            kind: kind.into(),
            status,
            times,
        });
    }

    /// Keep only the first half of every uploaded object (an upload that arrived short).
    pub fn store_half(&self, on: bool) {
        self.shared.lock().short_store = on;
    }

    /// Send half the bytes of a download and drop the connection.
    pub fn cut_downloads(&self, on: bool) {
        self.shared.lock().cut_get = on;
    }

    pub fn set_page(&self, n: usize) {
        self.shared.lock().page = n;
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

struct Req {
    method: String,
    path: String,
    query: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Req {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

fn serve(sh: &Arc<Shared>, cfg: &Arc<rustls::ServerConfig>, tcp: TcpStream) {
    let Ok(conn) = rustls::ServerConnection::new(cfg.clone()) else {
        return;
    };
    let tls = rustls::StreamOwned::new(conn, tcp);
    let mut reader = BufReader::new(tls);
    loop {
        let Some(req) = read_request(&mut reader) else { return };
        let keep = req
            .header("connection")
            .is_none_or(|c| !c.eq_ignore_ascii_case("close"));
        let reply = handle(sh, &req);
        let tls = reader.get_mut();
        if write_reply(tls, &req.method, &reply).is_err() || reply.cut || !keep {
            return;
        }
    }
}

fn read_request<R: BufRead>(r: &mut R) -> Option<Req> {
    let mut line = String::new();
    if r.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut it = line.split_whitespace();
    let (method, target) = (it.next()?.to_string(), it.next()?.to_string());
    let (path, query) = target
        .split_once('?')
        .map_or((target.clone(), String::new()), |(p, q)| (p.to_string(), q.to_string()));
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).ok()? == 0 {
            return None;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        let (k, v) = h.split_once(':')?;
        headers.push((k.trim().to_lowercase(), v.trim().to_string()));
    }
    let len: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).ok()?;
    Some(Req {
        method,
        path,
        query,
        headers,
        body,
    })
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// Send only half the body, then drop the connection.
    cut: bool,
}

fn xml(status: u16, body: String) -> Reply {
    Reply {
        status,
        headers: vec![("content-type".into(), "application/xml".into())],
        body: format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{body}").into_bytes(),
        cut: false,
    }
}

fn error(status: u16, code: &str) -> Reply {
    xml(
        status,
        format!("<Error><Code>{code}</Code><Message>stand-in</Message></Error>"),
    )
}

fn plain(status: u16) -> Reply {
    Reply {
        status,
        headers: Vec::new(),
        body: Vec::new(),
        cut: false,
    }
}

fn write_reply<W: Write>(w: &mut W, method: &str, r: &Reply) -> std::io::Result<()> {
    let text = match r.status {
        200 => "OK",
        204 => "No Content",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Status",
    };
    let mut head = format!("HTTP/1.1 {} {text}\r\ncontent-length: {}\r\n", r.status, r.body.len());
    for (k, v) in &r.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes())?;
    if method != "HEAD" {
        let n = if r.cut { r.body.len() / 2 } else { r.body.len() };
        w.write_all(&r.body[..n])?;
    }
    w.flush()
}

fn dec(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let pair = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = pair {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn enc(s: &str, keep_slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') || (keep_slash && b == b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn param(query: &str, name: &str) -> Option<String> {
    query.split('&').filter(|p| !p.is_empty()).find_map(|p| {
        let (k, v) = p.split_once('=').unwrap_or((p, ""));
        (dec(k) == name).then(|| dec(v))
    })
}

/// Rebuild the signature from what arrived. Written separately from the client's signer on purpose.
fn verify(opts: &Options, req: &Req, body_sha: &str) -> Result<(), &'static str> {
    let auth = req.header("authorization").ok_or("no authorization")?;
    let rest = auth.strip_prefix("AWS4-HMAC-SHA256 ").ok_or("algorithm")?;
    let field = |name: &str| -> Option<&str> { rest.split(", ").find_map(|f| f.strip_prefix(name)).map(str::trim) };
    let cred = field("Credential=").ok_or("credential")?;
    let signed = field("SignedHeaders=").ok_or("signed headers")?;
    let sig = field("Signature=").ok_or("signature")?;
    let scope: Vec<&str> = cred.split('/').collect();
    if scope.len() != 5 || scope[0] != opts.key_id || scope[3] != "s3" || scope[4] != "aws4_request" {
        return Err("credential");
    }
    let amz_date = req.header("x-amz-date").ok_or("no date")?;
    if &amz_date[..8.min(amz_date.len())] != scope[1] {
        return Err("date scope");
    }
    let t = chrono::NaiveDateTime::parse_from_str(amz_date, "%Y%m%dT%H%M%SZ")
        .map_err(|_| "date")?
        .and_utc()
        .timestamp();
    if (now_secs() as i64 - t).abs() > 900 {
        return Err("date skew");
    }
    let names: Vec<&str> = signed.split(';').collect();
    for must in ["host", "x-amz-date", "x-amz-content-sha256"] {
        if !names.contains(&must) {
            return Err("unsigned required header");
        }
    }
    let mut canon_headers = String::new();
    for n in &names {
        let v = req.header(n).ok_or("signed header missing")?;
        canon_headers.push_str(&format!("{n}:{}\n", v.split_whitespace().collect::<Vec<_>>().join(" ")));
    }
    let mut pairs: Vec<(String, String)> = req
        .query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (enc(&dec(k), false), enc(&dec(v), false))
        })
        .collect();
    pairs.sort();
    let canon_query = pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let declared = req.header("x-amz-content-sha256").ok_or("no payload hash")?;
    if declared != body_sha {
        return Err("payload hash");
    }
    let canon = format!(
        "{}\n{}\n{canon_query}\n{canon_headers}\n{signed}\n{declared}",
        req.method,
        enc(&dec(&req.path), true)
    );
    let sts = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{}/{}/s3/aws4_request\n{}",
        scope[1],
        scope[2],
        sha256_hex(canon.as_bytes())
    );
    let k = hmac_sha256(format!("AWS4{}", opts.secret).as_bytes(), scope[1].as_bytes());
    let k = hmac_sha256(&k, scope[2].as_bytes());
    let k = hmac_sha256(&k, b"s3");
    let k = hmac_sha256(&k, b"aws4_request");
    if hex(&hmac_sha256(&k, sts.as_bytes())) == sig {
        Ok(())
    } else {
        Err("signature")
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn due(inner: &mut Inner, kind: &str) -> Option<u16> {
    let f = inner.faults.iter_mut().find(|f| f.kind == kind && f.times != 0)?;
    if f.times > 0 {
        f.times -= 1;
    }
    Some(f.status)
}

fn handle(sh: &Arc<Shared>, req: &Req) -> Reply {
    let body_sha = sha256_hex(&req.body);
    if let Err(why) = verify(&sh.opts, req, &body_sha) {
        sh.lock().refused += 1;
        eprintln!("stand-in: refused {} {} ({why})", req.method, req.path);
        return error(403, "SignatureDoesNotMatch");
    }
    let Some(rest) = req.path.strip_prefix('/') else {
        return error(400, "BadRequest");
    };
    let (bucket, key) = match rest.split_once('/') {
        Some((b, k)) => (dec(b), dec(k)),
        None => (dec(rest), String::new()),
    };
    if bucket != sh.opts.bucket {
        sh.lock().refused += 1;
        return error(404, "NoSuchBucket");
    }
    let mut g = sh.lock();
    let copy_source = req.header("x-amz-copy-source").map(dec);
    let (kind, shown) = match (req.method.as_str(), key.is_empty()) {
        ("GET", true) => (
            "list",
            format!("LIST {}", param(&req.query, "prefix").unwrap_or_default()),
        ),
        ("HEAD", false) => ("head", format!("HEAD {key}")),
        ("GET", false) => ("get", format!("GET {key}")),
        ("DELETE", false) => ("delete", format!("DELETE {key}")),
        ("PUT", false) if copy_source.is_some() => (
            "copy",
            format!(
                "COPY {} {key}",
                copy_source
                    .clone()
                    .unwrap_or_default()
                    .trim_start_matches('/')
                    .trim_start_matches(&format!("{}/", sh.opts.bucket))
            ),
        ),
        ("PUT", false) => ("put", format!("PUT {key}")),
        _ => return error(405, "MethodNotAllowed"),
    };
    g.ops.push(shown);
    if let Some(status) = due(&mut g, kind) {
        return error(status, "InternalError");
    }
    match kind {
        "list" => {
            let prefix = param(&req.query, "prefix").unwrap_or_default();
            let after = param(&req.query, "continuation-token").unwrap_or_default();
            let all: Vec<(&String, &Obj)> = g
                .objects
                .iter()
                .filter(|(k, _)| k.starts_with(&prefix) && (after.is_empty() || k.as_str() > after.as_str()))
                .collect();
            let page = g.page.max(1);
            let truncated = all.len() > page;
            let shown = &all[..all.len().min(page)];
            let mut out = format!(
                "<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>{}</Name><Prefix>{prefix}</Prefix>\
                 <KeyCount>{}</KeyCount><MaxKeys>{page}</MaxKeys><IsTruncated>{truncated}</IsTruncated>",
                sh.opts.bucket,
                shown.len()
            );
            if truncated && let Some((last, _)) = shown.last() {
                out.push_str(&format!("<NextContinuationToken>{last}</NextContinuationToken>"));
            }
            for (k, o) in shown {
                out.push_str(&format!(
                    "<Contents><Key>{k}</Key><LastModified>2026-11-16T00:00:00.000Z</LastModified><ETag>\"x\"</ETag><Size>{}</Size>\
                     <StorageClass>STANDARD</StorageClass></Contents>",
                    o.data.len()
                ));
            }
            out.push_str("</ListBucketResult>");
            xml(200, out)
        }
        "put" => {
            let mut data = req.body.clone();
            if g.short_store {
                data.truncate(data.len() / 2);
            }
            g.objects.insert(key, Obj { data });
            Reply {
                status: 200,
                headers: vec![("etag".into(), format!("\"{}\"", &body_sha[..32]))],
                body: Vec::new(),
                cut: false,
            }
        }
        "copy" => {
            let src = copy_source.unwrap_or_default();
            let src = src.trim_start_matches('/');
            let src = src
                .strip_prefix(&format!("{}/", sh.opts.bucket))
                .unwrap_or(src)
                .to_string();
            match g.objects.get(&src).cloned() {
                Some(o) => {
                    g.objects.insert(key, o);
                    xml(200, "<CopyObjectResult><ETag>\"x\"</ETag></CopyObjectResult>".into())
                }
                None => error(404, "NoSuchKey"),
            }
        }
        "head" => match g.objects.get(&key) {
            Some(o) => Reply {
                status: 200,
                headers: Vec::new(),
                body: vec![0; o.data.len()],
                cut: false,
            },
            None => plain(404),
        },
        "get" => match g.objects.get(&key) {
            Some(o) => Reply {
                status: 200,
                headers: Vec::new(),
                body: o.data.clone(),
                cut: g.cut_get,
            },
            None => error(404, "NoSuchKey"),
        },
        "delete" => {
            g.objects.remove(&key);
            plain(204)
        }
        _ => error(405, "MethodNotAllowed"),
    }
}
