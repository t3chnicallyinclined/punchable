//! The differential: the Python gsync's recorded runs (`rust/tools/gen_gsync_tapes.py`) replayed against pk-gsync.
//!
//! A tape is a list of CLI calls. For each call it holds the files to change first, the arguments, the clock, the
//! free disk, every HTTP request the Python made in order (Google Drive, Keycloak, Google's token endpoint, the
//! hosted API) with the answer it was given, and what it left behind (exit code, printed text, every file under the
//! test's home and data folders). The replay server answers pk-gsync's requests from the tape and fails on the first
//! request that is not the next recorded one: method, path, query, bearer token, content type, range and body (a
//! `POST`ed idea, a Drive multipart upload, a token form) all have to be equal. Then the exit code, the printed text
//! and every file have to be equal too. The red arms at the bottom change one thing and must see the replay fail.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use pk_core::Env;
use pk_core::time::Fixed;
use pk_gsync::cli;
use pk_gsync::services::{GOOGLE_ROOT, Services, restrict};
use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

// ---- the replay server ----------------------------------------------------------------------------------------------
#[derive(Default)]
struct State {
    expected: Vec<Value>,
    cursor: usize,
    failures: Vec<String>,
    served: usize,
}

fn svc_of(path: &str) -> &'static str {
    if path.starts_with("/drive/") || path.starts_with("/upload/drive/") {
        "drive"
    } else if path.starts_with("/kc/") || path.starts_with("/google/") || path.starts_with("/wtoken") {
        "auth"
    } else if path == "/_spawn" {
        "spawn"
    } else {
        "pk"
    }
}

fn parse_multipart(body: &[u8], ctype: &str) -> Value {
    let boundary = ctype
        .split("boundary=")
        .nth(1)
        .map(|b| b.trim().trim_matches('"').to_string())
        .unwrap_or_default();
    let text = String::from_utf8_lossy(body).into_owned();
    let marker = format!("--{boundary}");
    let parts: Vec<&str> = text
        .split(&marker)
        .skip(1)
        .filter(|p| !p.trim_start().starts_with("--"))
        .collect();
    let content = |p: &str| -> String {
        let (_, rest) = p.split_once("\r\n\r\n").unwrap_or(("", ""));
        rest.strip_suffix("\r\n").unwrap_or(rest).to_string()
    };
    let meta: Value = parts.first().map_or(Value::Null, |p| {
        serde_json::from_str(&content(p)).unwrap_or(Value::Null)
    });
    let media = parts.get(1).map_or(Value::Null, |p| json!(content(p)));
    json!({"meta": meta, "media": media})
}

fn actual_request(method: &str, target: &str, headers: &BTreeMap<String, String>, body: &[u8]) -> Value {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut q: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (k, v) in form_urlencoded::parse(query.as_bytes()) {
        q.entry(k.into_owned()).or_default().push(v.into_owned());
    }
    let ctype = headers
        .get("content-type")
        .map(|c| c.split(';').next().unwrap_or_default().trim().to_string());
    let parsed = match ctype.as_deref() {
        Some("application/json") if !body.is_empty() => serde_json::from_slice(body).unwrap_or(Value::Null),
        Some("application/x-www-form-urlencoded") => {
            let mut m = serde_json::Map::new();
            for (k, v) in form_urlencoded::parse(body) {
                m.entry(k.into_owned()).or_insert(json!(v.into_owned()));
            }
            Value::Object(m)
        }
        Some("multipart/related") => parse_multipart(body, headers.get("content-type").map_or("", String::as_str)),
        _ => Value::Null,
    };
    json!({"svc": svc_of(path), "method": method, "path": path, "query": q,
           "auth": headers.get("authorization"), "ctype": ctype, "range": headers.get("range"), "body": parsed})
}

/// The first place two JSON values differ, as a path and both sides (strings: around the first differing byte).
#[allow(clippy::many_single_char_names)]
fn json_diff(a: &Value, b: &Value, path: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            // a differing `text` explains a differing `sha256`, so the hash is looked at last
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort_by_key(|k| k.as_str() == "sha256");
            for k in keys {
                let (l, r) = (x.get(k).unwrap_or(&Value::Null), y.get(k).unwrap_or(&Value::Null));
                if x.contains_key(k) != y.contains_key(k) {
                    return Some(format!(
                        "{path}.{k}: key only {}",
                        if x.contains_key(k) { "recorded" } else { "in pk-gsync" }
                    ));
                }
                if let Some(d) = json_diff(l, r, &format!("{path}.{k}")) {
                    return Some(d);
                }
            }
            None
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                let names = |v: &[Value]| {
                    v.iter()
                        .map(|i| i["path"].as_str().unwrap_or("?").to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                return Some(format!(
                    "{path}: {} items recorded ({}), {} in pk-gsync ({})",
                    x.len(),
                    names(x),
                    y.len(),
                    names(y)
                ));
            }
            x.iter()
                .zip(y)
                .enumerate()
                .find_map(|(i, (l, r))| json_diff(l, r, &format!("{path}[{i}]")))
        }
        (Value::String(x), Value::String(y)) if x != y => {
            let at = x
                .bytes()
                .zip(y.bytes())
                .position(|(p, q)| p != q)
                .unwrap_or(x.len().min(y.len()));
            let around = |s: &str| {
                let from = s
                    .char_indices()
                    .map(|(i, _)| i)
                    .rfind(|&i| i <= at.saturating_sub(40))
                    .unwrap_or(0);
                s[from..].chars().take(120).collect::<String>()
            };
            Some(format!(
                "{path}: recorded ...{:?} / pk-gsync ...{:?}",
                around(x),
                around(y)
            ))
        }
        _ if a != b => Some(format!("{path}: recorded {a} / pk-gsync {b}")),
        _ => None,
    }
}

fn diff_request(expected: &Value, actual: &Value) -> Option<String> {
    let out: Vec<String> = ["svc", "method", "path", "query", "auth", "ctype", "range", "body"]
        .into_iter()
        .filter_map(|k| json_diff(&expected[k], &actual[k], k))
        .map(|d| format!("  {d}"))
        .collect();
    (!out.is_empty()).then(|| {
        out.join(
            "
",
        )
    })
}

type Reply = (u16, String, Vec<u8>, Vec<(String, String)>);

fn answer(st: &mut State, method: &str, target: &str, headers: &BTreeMap<String, String>, body: &[u8]) -> Reply {
    let actual = actual_request(method, target, headers, body);
    let refuse = |why: &str| -> Reply {
        (
            599,
            "application/json".into(),
            json!({"error": why}).to_string().into_bytes(),
            vec![],
        )
    };
    if !st.failures.is_empty() {
        return refuse("an earlier request already differed");
    }
    let Some(exp) = st.expected.get(st.cursor).cloned() else {
        st.failures.push(format!(
            "request #{} was not recorded: {method} {target}",
            st.cursor + 1
        ));
        return refuse("more requests than the tape has");
    };
    if let Some(d) = diff_request(&exp, &actual) {
        st.failures.push(format!(
            "request #{} ({method} {}) differs:\n{d}",
            st.cursor + 1,
            actual["path"]
        ));
        return refuse("request differs from the tape");
    }
    st.cursor += 1;
    st.served += 1;
    let resp = &exp["resp"];
    let status = resp["status"].as_u64().unwrap_or(200) as u16;
    let ctype = resp["ctype"].as_str().unwrap_or("application/json").to_string();
    let bytes = if let Some(j) = resp.get("json") {
        serde_json::to_vec(j).unwrap_or_default()
    } else if let Some(t) = resp.get("text").and_then(Value::as_str) {
        t.as_bytes().to_vec()
    } else if let Some(b) = resp.get("b64").and_then(Value::as_str) {
        base64::engine::general_purpose::STANDARD.decode(b).unwrap_or_default()
    } else {
        Vec::new()
    };
    let extra = resp["extra"]
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                .collect()
        })
        .unwrap_or_default();
    (status, ctype, bytes, extra)
}

fn handle_conn(stream: TcpStream, st: &Arc<Mutex<State>>) {
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let mut parts = line.split_whitespace();
        let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
            return;
        };
        let (method, target) = (method.to_string(), target.to_string());
        let mut headers = BTreeMap::new();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 {
                return;
            }
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
            }
        }
        let n: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut body = vec![0u8; n];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let (status, ctype, payload, extra) = answer(&mut st.lock().expect("lock"), &method, &target, &headers, &body);
        let phrase = reqwest::StatusCode::from_u16(status)
            .ok()
            .and_then(|s| s.canonical_reason())
            .unwrap_or("Status");
        let mut head = format!(
            "HTTP/1.1 {status} {phrase}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: keep-alive\r\n",
            payload.len()
        );
        for (k, v) in extra {
            let _ = write!(head, "{k}: {v}\r\n");
        }
        head.push_str("\r\n");
        if writer
            .write_all(head.as_bytes())
            .and_then(|()| writer.write_all(&payload))
            .is_err()
        {
            return;
        }
    }
}

struct Server {
    base: String,
    state: Arc<Mutex<State>>,
}

fn serve() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://127.0.0.1:{}", listener.local_addr().expect("addr").port());
    let state = Arc::new(Mutex::new(State::default()));
    let st = state.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let st = st.clone();
            std::thread::spawn(move || handle_conn(stream, &st));
        }
    });
    Server { base, state }
}

// ---- the files ------------------------------------------------------------------------------------------------------
fn snapshot(root: &Path) -> BTreeMap<String, String> {
    fn walk(dir: &Path, root: &Path, expiry: &Regex, out: &mut BTreeMap<String, String>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, root, expiry, out);
                continue;
            }
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if [".lock", ".tmp", ".part"]
                .iter()
                .any(|x| name.to_ascii_lowercase().ends_with(x))
            {
                continue;
            }
            let rel = p
                .strip_prefix(root)
                .expect("under root")
                .to_string_lossy()
                .replace('\\', "/");
            let data = fs::read(&p).unwrap_or_default();
            let text = match String::from_utf8(data.clone()) {
                Ok(t) => {
                    let t = t.replace("\r\n", "\n");
                    expiry.replace_all(&t, r#""expiry": "<expiry>""#).into_owned()
                }
                Err(_) => format!("bin:{}:{}", data.len(), &hex::encode(Sha256::digest(&data))[..16]),
            };
            out.insert(rel, text);
        }
    }
    let mut out = BTreeMap::new();
    walk(
        root,
        root,
        &Regex::new(r#""expiry": "[^"]*""#).expect("regex"),
        &mut out,
    );
    out
}

fn first_difference(a: &str, b: &str) -> String {
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    for i in 0..la.len().max(lb.len()) {
        if la.get(i) != lb.get(i) {
            let (x, y) = (
                la.get(i).copied().unwrap_or("<end>"),
                lb.get(i).copied().unwrap_or("<end>"),
            );
            let at = x
                .bytes()
                .zip(y.bytes())
                .position(|(p, q)| p != q)
                .unwrap_or(x.len().min(y.len()));
            let around = |s: &str| {
                let from = s
                    .char_indices()
                    .map(|(i, _)| i)
                    .rfind(|&i| i <= at.saturating_sub(60))
                    .unwrap_or(0);
                s[from..].chars().take(160).collect::<String>()
            };
            return format!(
                "line {}: recorded ...{:?} / pk-gsync ...{:?}",
                i + 1,
                around(x),
                around(y)
            );
        }
    }
    "same lines, different ending".into()
}

// ---- a replay -------------------------------------------------------------------------------------------------------
/// What a replay counted.
#[derive(Debug, Default, Clone, Copy)]
struct Counts {
    steps: usize,
    exchanges: usize,
    files: usize,
}

fn load_tape(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("tapes")
        .join(format!("{name}.json"));
    serde_json::from_str(&fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
        .expect("a tape is JSON")
}

fn tape_names() -> Vec<String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("tapes");
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("tapes dir")
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_string_lossy()
                .strip_suffix(".json")
                .map(str::to_string)
        })
        .collect();
    names.sort();
    names
}

/// The tape with the server's address in the requests and the files to write; the printed text and the files
/// The tape with the server's address in the requests and the files to write; the printed text and the files
/// compared afterwards keep `{BASE}` (pk-gsync's output is turned back into it before the comparison).
///
/// A posted file whose text names the server (a run's error with its URL) was hashed with the recording's port,
/// which is not this replay's. Its hash is made again for the replay's text, in the post and in every later listing
/// of the stored files; every other hash stays as recorded.
fn replace_base(v: &Value, base: &str) -> Value {
    let mut t = v.clone();
    let swap = |x: &Value| -> Value {
        serde_json::from_str(
            &x.to_string()
                .replace("{BASE}", base)
                .replace("{GDOCS_HOST}", "docs.google.com"),
        )
        .expect("json")
    };
    let sha_of = |text: &str| hex::encode(Sha256::digest(text.as_bytes()));
    t["files"] = swap(&v["files"]);
    let mut named: BTreeMap<String, String> = BTreeMap::new();
    for step in t["steps"].as_array_mut().expect("steps") {
        let recorded = step["exchanges"].clone();
        step["exchanges"] = swap(&recorded);
        step["before"] = swap(&step["before"]);
        for ex in step["exchanges"].as_array_mut().expect("exchanges") {
            if ex["path"] != "/api/worker/files" {
                continue;
            }
            if ex["method"] == "POST" {
                for f in ex["body"]["files"].as_array_mut().into_iter().flatten() {
                    let path = f["path"].as_str().unwrap_or_default().to_string();
                    let text = f["text"].as_str().unwrap_or_default().to_string();
                    if text.contains(base) {
                        let sha = sha_of(&text);
                        f["sha256"] = json!(sha);
                        named.insert(path, sha);
                    } else {
                        named.remove(&path);
                    }
                }
            } else {
                for f in ex["resp"]["json"]["files"].as_array_mut().into_iter().flatten() {
                    if let Some(sha) = named.get(f["path"].as_str().unwrap_or_default()) {
                        f["sha256"] = json!(sha);
                    }
                }
            }
        }
    }
    t
}

fn write_file(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    fs::write(p, text).expect("write");
}

/// Replay one tape from its recorded files. `tweak` may change the tape or the files first (the red arms).
fn replay(tape: &Value, tweak: impl FnOnce(&mut Value, &Path)) -> Result<Counts, String> {
    let server = serve();
    let base = server.base.clone();
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().to_path_buf();
    let mut tape = replace_base(tape, &base);
    for (rel, text) in tape["files"].as_object().expect("files") {
        write_file(&root, rel, text.as_str().unwrap_or_default());
    }
    tweak(&mut tape, &root);
    let user = std::env::var("USERNAME").ok().or_else(|| std::env::var("USER").ok());
    let secrets = fs::read_to_string(root.join("home").join("secrets.env")).unwrap_or_default();
    let mut vars: Vec<(String, String)> = pk_core::config::parse_secrets(&secrets).into_iter().collect();
    let root_s = root.to_string_lossy().into_owned();
    vars.extend([
        (
            "PUNCHABLE_HOME".into(),
            root.join("home").to_string_lossy().into_owned(),
        ),
        ("PK_ONLINE_FIRST".into(), "1".into()),
        ("PK_WORKSPACE".into(), root.join("ws").to_string_lossy().into_owned()),
        ("PK_API".into(), base.clone()),
        ("PK_TOKEN_URL".into(), format!("{base}/wtoken")),
        ("PK_AUTH_URL".into(), format!("{base}/kc/realms/punchable")),
        ("PK_GSYNC_MEMOS".into(), "1".into()),
        ("PUNCHABLE_API".into(), base.clone()),
    ]);
    if let Some(extra) = tape["env"].as_object() {
        for (k, v) in extra {
            vars.push((k.clone(), v.as_str().unwrap_or_default().replace("{ROOT}", &root_s)));
        }
    }
    let env = Env::from_map(vars).with_home(root.join("home"));
    let mut counts = Counts::default();
    for (n, step) in tape["steps"].as_array().expect("steps").iter().enumerate() {
        let tag = format!(
            "{} step {} ({})",
            tape["name"].as_str().unwrap_or("?"),
            n + 1,
            step["note"].as_str().unwrap_or("")
        );
        for (rel, text) in step["before"]["write"].as_object().into_iter().flatten() {
            write_file(&root, rel, text.as_str().unwrap_or_default());
        }
        for rel in step["before"]["delete"].as_array().into_iter().flatten() {
            let _ = fs::remove_file(root.join(rel.as_str().unwrap_or_default()));
        }
        {
            let mut st = server.state.lock().expect("lock");
            st.expected = step["exchanges"].as_array().cloned().unwrap_or_default();
            st.cursor = 0;
            st.failures.clear();
        }
        let free = step["free_mb"].as_u64().unwrap_or(50_000);
        let spawn_base = base.clone();
        let user2 = user.clone();
        let svc = Services {
            clock: Box::new(Fixed::parse(step["now"].as_str().expect("now")).expect("a time")),
            sleep: Box::new(|_| {}),
            opener: Box::new(|_| {}),
            spawn: Box::new(move |_cfg| {
                // the processor is Python and not started in a test: the start is an exchange of the tape
                let c = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .build()
                    .expect("client");
                let _ = c
                    .post(format!("{spawn_base}/_spawn"))
                    .header("Content-Type", "application/json")
                    .body("{}")
                    .send();
                Ok(())
            }),
            free_mb: Box::new(move |_| free),
            restrict: Box::new(move |p| restrict(p, user2.as_deref())),
            google_token_uri: format!("{base}/google/token"),
            drive_root: base.clone(),
        };
        let _ = GOOGLE_ROOT;
        let args: Vec<String> = step["argv"]
            .as_array()
            .expect("argv")
            .iter()
            .map(|a| a.as_str().unwrap_or_default().to_string())
            .collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = cli::run(&args, &env, &svc, &mut out, &mut err);
        let (failures, cursor, total) = {
            let st = server.state.lock().expect("lock");
            (st.failures.clone(), st.cursor, st.expected.len())
        };
        if let Some(f) = failures.first() {
            // the files are no evidence yet, but a file that already differs usually explains the request that did
            let hint: Vec<String> = step["after"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(rel, text)| {
                    let g = snapshot(&root)
                        .get(rel)?
                        .replace(&base, "{BASE}")
                        .replace("docs.google.com", "{GDOCS_HOST}");
                    (g != text.as_str().unwrap_or_default()).then(|| {
                        format!(
                            "  (the file {rel} already differs: {})",
                            first_difference(text.as_str().unwrap_or_default(), &g)
                        )
                    })
                })
                .take(2)
                .collect();
            return Err(format!(
                "{tag}: {f}
{}",
                hint.join(
                    "
"
                )
            ));
        }
        if cursor != total {
            let next = &step["exchanges"][cursor];
            return Err(format!(
                "{tag}: pk-gsync made {cursor} requests, the tape has {total}; next recorded: {} {}",
                next["method"], next["path"]
            ));
        }
        counts.exchanges += total;
        let norm = |s: &str| -> String {
            s.replace("\r\n", "\n")
                .replace(&base, "{BASE}")
                .replace(&root_s.replace('\\', "/"), "{ROOT}")
                .replace(&root_s, "{ROOT}")
                .replace('\\', "/")
                .replace("docs.google.com", "{GDOCS_HOST}")
        };
        let stdout = norm(&String::from_utf8_lossy(&out));
        if stdout != step["stdout"].as_str().unwrap_or_default() {
            return Err(format!(
                "{tag}: printed text differs: {}\n  stderr: {}",
                first_difference(step["stdout"].as_str().unwrap_or_default(), &stdout),
                String::from_utf8_lossy(&err)
            ));
        }
        if i64::from(code) != step["exit"].as_i64().unwrap_or(-1) {
            return Err(format!(
                "{tag}: exit code {code}, recorded {}; stderr: {}",
                step["exit"],
                String::from_utf8_lossy(&err)
            ));
        }
        let want = step["after"].as_object().expect("after");
        let got = snapshot(&root);
        for (rel, text) in want {
            match got.get(rel) {
                None => return Err(format!("{tag}: pk-gsync did not leave {rel}")),
                Some(g) => {
                    let g = g.replace(&base, "{BASE}").replace("docs.google.com", "{GDOCS_HOST}");
                    if g != text.as_str().unwrap_or_default() {
                        return Err(format!(
                            "{tag}: {rel} differs: {}",
                            first_difference(text.as_str().unwrap_or_default(), &g)
                        ));
                    }
                }
            }
        }
        if let Some(extra) = got.keys().find(|k| !want.contains_key(*k)) {
            return Err(format!("{tag}: pk-gsync left a file the Python did not: {extra}"));
        }
        counts.files += want.len();
        counts.steps += 1;
    }
    Ok(counts)
}

// ---- the tests ------------------------------------------------------------------------------------------------------
#[test]
fn every_tape_replays_with_identical_requests_and_files() {
    let mut total = Counts::default();
    let mut failures = Vec::new();
    for name in tape_names() {
        match replay(&load_tape(&name), |_, _| {}) {
            Ok(c) => {
                println!(
                    "{name}: {} steps, {} requests, {} files equal",
                    c.steps, c.exchanges, c.files
                );
                total.steps += c.steps;
                total.exchanges += c.exchanges;
                total.files += c.files;
            }
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    println!(
        "differential: {} steps, {} requests, {} file comparisons",
        total.steps, total.exchanges, total.files
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    assert!(total.exchanges > 900, "the tapes shrank: {total:?}");
}

// ---- red arms: each changes one thing and the replay must see it -----------------------------------------------------
fn must_fail(r: Result<Counts, String>, needle: &str) {
    match r {
        Ok(c) => panic!("the replay passed ({c:?}) but should have failed on {needle:?}"),
        Err(e) => assert!(e.contains(needle), "failed, but not on {needle:?}: {e}"),
    }
}

#[test]
fn red_arm_a_changed_recorded_post_fails() {
    // the first idea the Python posted, with one word changed
    let r = replay(&load_tape("bootstrap_and_pull"), |t, _| {
        let steps = t["steps"].as_array_mut().expect("steps");
        let ex = steps[0]["exchanges"]
            .as_array_mut()
            .expect("exchanges")
            .iter_mut()
            .find(|e| e["path"] == "/api/worker/ideas")
            .expect("an idea was posted");
        ex["body"]["text"] = json!("a different text");
    });
    must_fail(r, "/api/worker/ideas");
}

#[test]
fn red_arm_a_weaker_legal_rule_lets_a_blocked_doc_through() {
    // drop the private keyword rule: the doc whose title trips it is no longer skipped, so pk-gsync exports it,
    // a request the Python never made
    let r = replay(&load_tape("bootstrap_and_pull"), |_, root| {
        let p = root.join("home").join("legal_rules.json");
        let mut v: Value = serde_json::from_str(&fs::read_to_string(&p).expect("rules")).expect("json");
        v["keywords"] = json!([]);
        fs::write(&p, v.to_string()).expect("write");
    });
    must_fail(r, "differs");
}

#[test]
fn red_arm_a_changed_recorded_file_fails() {
    let r = replay(&load_tape("push_and_boards"), |t, _| {
        let after = t["steps"][1]["after"].as_object_mut().expect("after");
        let key = after
            .keys()
            .find(|k| k.ends_with("kb/gsync/map.json"))
            .expect("a map")
            .clone();
        let text = after[&key]
            .as_str()
            .expect("text")
            .replace("\"version\"", "\"verzion\"");
        after.insert(key, json!(text));
    });
    must_fail(r, "map.json differs");
}

#[test]
fn red_arm_a_request_the_tape_lacks_fails() {
    // the tape loses its last recorded request of a step: pk-gsync makes one more than the tape has
    let r = replay(&load_tape("merges"), |t, _| {
        t["steps"][0]["exchanges"].as_array_mut().expect("exchanges").pop();
    });
    must_fail(r, "was not recorded");
}

#[test]
fn red_arm_a_request_pk_gsync_does_not_make_fails() {
    let r = replay(&load_tape("dry_run"), |t, _| {
        let ex = t["steps"][2]["exchanges"].as_array_mut().expect("exchanges");
        let last = ex.last().cloned().expect("an exchange");
        ex.push(last);
    });
    must_fail(r, "the tape has");
}

#[test]
fn a_dry_run_makes_no_write_request_to_google_or_the_hosted_api() {
    // not a comparison with the Python: a property of the tapes the dry runs are replayed from. In every dry-run
    // step, no request is a POST/PATCH/PUT/DELETE to Drive, and the only POSTs are the token forms.
    let mut dry_steps = 0;
    for name in tape_names() {
        let tape = load_tape(&name);
        for step in tape["steps"].as_array().expect("steps") {
            if step["argv"] != json!(["run", "--dry-run"]) {
                continue;
            }
            dry_steps += 1;
            for ex in step["exchanges"].as_array().expect("exchanges") {
                let write = ex["method"] != "GET";
                let token = ex["svc"] == "auth";
                assert!(
                    !write || token,
                    "a dry run wrote: {} {} in {name}",
                    ex["method"],
                    ex["path"]
                );
            }
        }
    }
    assert!(dry_steps >= 3, "{dry_steps} dry-run steps in the tapes");
}
