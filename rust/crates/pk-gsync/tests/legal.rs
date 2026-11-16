//! The legal net fails closed (T11 round A0). Each test builds a small invented Drive and Punchable on 127.0.0.1, runs
//! `pk-gsync run` through the command line and looks at what was asked of Drive and what was left in the state. The
//! rules, keywords, folders, ids and titles here are all invented. A doc the rules close must not be exported or
//! downloaded, and nothing private (the keyword that matched) may reach the map or the state posted to the database.
//!
//! The tapes (`differential.rs`) prove the unchanged behaviour against the Python; these prove what the Python did
//! wrong and pk-gsync no longer does. Each has a red arm in the commit message: the old behaviour makes it fail.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pk_core::Env;
use pk_core::time::Fixed;
use pk_gsync::cli;
use pk_gsync::services::Services;
use serde_json::{Value, json};

const DOC: &str = "application/vnd.google-apps.document";
const OWNER: &str = "owner@example.com";
const LEGAL_FOLDER: &str = "legal-folder-0001";
const OK_FOLDER: &str = "sample-folder-punchable-0000000000";
const KEYWORD: &str = "zorblax";

/// What the fake Drive and Punchable know, and what they were asked.
#[derive(Default)]
struct World {
    /// Drive metadata by id.
    files: BTreeMap<String, Value>,
    /// The text of each doc.
    texts: BTreeMap<String, String>,
    /// Ids Drive answers with this status instead (404, 400, ...).
    refuse: BTreeMap<String, u16>,
    /// The ids in the Changes feed.
    changes: Vec<String>,
    /// What `POST /upload/drive/v3/files` returns (a doc gsync makes).
    created: Option<Value>,
    /// The app's jokes (id, title, text), as the decisions feed names them.
    jokes: Vec<(String, String, String)>,
    /// The app's storyboards.
    boards: Value,
    /// Every request, `METHOD /path?query`.
    seen: Vec<String>,
    /// The body of every `POST /api/worker/files` (the state sent to the database).
    posted: Vec<String>,
}

type Shared = Arc<Mutex<World>>;

fn meta(id: &str, name: &str, parents: &[&str]) -> Value {
    json!({"id": id, "name": name, "mimeType": DOC, "parents": parents, "owners": [{"emailAddress": OWNER}],
           "createdTime": "2026-10-20T10:00:00.000Z", "modifiedTime": "2026-10-20T10:00:00.000Z", "version": "2",
           "trashed": false, "appProperties": {}})
}

fn folder(id: &str, parents: &[&str]) -> Value {
    json!({"id": id, "name": "A folder", "mimeType": "application/vnd.google-apps.folder", "parents": parents})
}

fn reply(status: u16, ctype: &str, body: &[u8]) -> Vec<u8> {
    let mut raw = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    raw.extend_from_slice(body);
    raw
}

fn json_reply(v: &Value) -> Vec<u8> {
    reply(200, "application/json", v.to_string().as_bytes())
}

fn answer(w: &mut World, method: &str, target: &str, body: &[u8]) -> Vec<u8> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    w.seen.push(format!("{method} {target}"));
    let q: BTreeMap<String, String> = form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    match (method, path) {
        ("POST", "/google/token") => return json_reply(&json!({"access_token": "fake-g", "expires_in": 3599})),
        ("POST", "/wtoken") => return json_reply(&json!({"access_token": "fake-w", "expires_in": 300})),
        ("GET", "/api/health") => return json_reply(&json!({"ok": true})),
        ("GET", "/api/worker/files") => return json_reply(&json!({"files": []})),
        ("POST", "/api/worker/files") => {
            w.posted.push(String::from_utf8_lossy(body).into_owned());
            return json_reply(&json!({"written": []}));
        }
        ("GET", "/api/worker/decisions") => {
            let lines: Vec<Value> = w
                .jokes
                .iter()
                .map(|(id, title, text)| {
                    json!({"decision": {"ts": "t", "by": "owner", "target": "node", "id": id, "action": "add",
                        "after": {"type": "joke", "title": title, "summary": text}, "reason": "saved from review"}})
                })
                .collect();
            return json_reply(&json!({"total": lines.len(), "lines": lines}));
        }
        ("GET", jp) if jp.starts_with("/api/worker/jokes/") => {
            let id = jp.trim_start_matches("/api/worker/jokes/");
            return match w.jokes.iter().find(|(j, _, _)| j == id) {
                Some((_, title, text)) => {
                    json_reply(&json!({"joke": {"title": title, "summary": text, "stage": "idea",
                    "hidden": false, "lead_unit": null}, "versions": [], "yours": true}))
                }
                None => reply(404, "application/json", br#"{"error": "no such joke"}"#),
            };
        }
        ("GET", "/api/worker/creative") => {
            let items = if q.get("store").map(String::as_str) == Some("storyboards") {
                w.boards.clone()
            } else {
                json!({})
            };
            return json_reply(&json!({"items": items}));
        }
        ("GET", "/drive/v3/changes") => {
            let changes: Vec<Value> = w
                .changes
                .iter()
                .map(|id| json!({"fileId": id, "file": w.files.get(id).cloned().unwrap_or(Value::Null)}))
                .collect();
            return json_reply(&json!({"changes": changes, "newStartPageToken": "9"}));
        }
        ("POST", "/upload/drive/v3/files") => {
            return json_reply(w.created.as_ref().expect("a created doc is set"));
        }
        ("GET", "/drive/v3/files") => return json_reply(&json!({"files": []})),
        _ => {}
    }
    if let Some(rest) = path.strip_prefix("/drive/v3/files/") {
        let (id, export) = rest.strip_suffix("/export").map_or((rest, false), |i| (i, true));
        if let Some(&status) = w.refuse.get(id) {
            return reply(status, "application/json", br#"{"error": {"message": "refused"}}"#);
        }
        if export {
            return match w.texts.get(id) {
                Some(t) => reply(200, "text/plain", t.as_bytes()),
                None => reply(404, "application/json", br#"{"error": {"message": "no text"}}"#),
            };
        }
        if q.get("alt").map(String::as_str) == Some("media") {
            return reply(200, "audio/x-m4a", b"audio");
        }
        return match w.files.get(id) {
            Some(m) => json_reply(m),
            None => reply(404, "application/json", br#"{"error": {"message": "not found"}}"#),
        };
    }
    reply(404, "application/json", br#"{"error": {"message": "no such route"}}"#)
}

fn serve_connection(stream: TcpStream, world: &Shared) {
    let Ok(mut writer) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_string();
        let target = parts.next().unwrap_or_default().to_string();
        let mut len = 0usize;
        loop {
            let mut raw = String::new();
            if reader.read_line(&mut raw).unwrap_or(0) == 0 {
                return;
            }
            let raw = raw.trim_end();
            if raw.is_empty() {
                break;
            }
            if let Some((name, value)) = raw.split_once(':')
                && name.trim().eq_ignore_ascii_case("content-length")
            {
                len = value.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; len];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let out = answer(&mut world.lock().expect("lock"), &method, &target, &body);
        if writer.write_all(&out).is_err() {
            return;
        }
    }
}

fn start(world: &Shared) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://127.0.0.1:{}", listener.local_addr().expect("addr").port());
    let world = world.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let world = world.clone();
            std::thread::spawn(move || serve_connection(stream, &world));
        }
    });
    base
}

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    fs::write(p, text).expect("write");
}

/// The invented rules: a legal folder, a private keyword, content markers.
fn rules_text() -> String {
    json!({"owner": OWNER, "legal_parents": [LEGAL_FOLDER], "keywords": [KEYWORD],
           "content_markers": [], "legal_ids": []})
    .to_string()
}

/// A home with a sign-in, the rules and the state, in a fresh temp folder.
struct Case {
    dir: tempfile::TempDir,
    world: Shared,
    base: String,
    extra_env: Vec<(String, String)>,
}

impl Case {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tmp");
        let world: Shared = Arc::default();
        {
            let mut w = world.lock().expect("lock");
            w.files.insert(OK_FOLDER.into(), folder(OK_FOLDER, &["root0"]));
            w.files.insert("root0".into(), folder("root0", &[]));
            w.files.insert(LEGAL_FOLDER.into(), folder(LEGAL_FOLDER, &["root0"]));
            w.boards = json!({});
        }
        let base = start(&world);
        let r = dir.path();
        write(r, "home/legal_rules.json", &rules_text());
        write(r, "home/secrets.env", "PK_WORKER_SECRET=fake-worker-secret\n");
        write(
            r,
            "home/google-oauth.env",
            "GOOGLE_CLIENT_ID=a\nGOOGLE_CLIENT_SECRET=b\n",
        );
        write(
            r,
            "home/keycloak_gsync_token.json",
            r#"{"refresh_token": "fake-kc", "saved_at": 1}"#,
        );
        write(
            r,
            "home/google_token.json",
            r#"{"refresh_token": "fake-g-refresh", "source": "keycloak-broker", "kc_checked_at": 1793015900}"#,
        );
        // not a first run: the Changes feed is where we left off
        write(
            r,
            "ws/comedy/kb/gsync/map.json",
            &json!({"changes_token": "5"}).to_string(),
        );
        Self {
            dir,
            world,
            base,
            extra_env: vec![("PK_GSYNC_MEMOS".into(), "0".into())],
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    fn map(&self) -> Value {
        serde_json::from_str(&fs::read_to_string(self.root().join("ws/comedy/kb/gsync/map.json")).expect("map"))
            .expect("map json")
    }

    fn set_map(&self, m: &Value) {
        write(&self.root(), "ws/comedy/kb/gsync/map.json", &m.to_string());
    }

    /// A doc in the Changes feed with this text.
    fn doc(&self, m: Value, text: &str) {
        let mut w = self.world.lock().expect("lock");
        let id = m["id"].as_str().expect("id").to_string();
        w.changes.push(id.clone());
        w.texts.insert(id.clone(), text.to_string());
        w.files.insert(id, m);
    }

    fn run(&self, args: &[&str]) -> Outcome {
        let root = self.root();
        let mut vars: Vec<(String, String)> = vec![
            (
                "PUNCHABLE_HOME".into(),
                root.join("home").to_string_lossy().into_owned(),
            ),
            ("PK_ONLINE_FIRST".into(), "1".into()),
            ("PK_WORKSPACE".into(), root.join("ws").to_string_lossy().into_owned()),
            ("PK_API".into(), self.base.clone()),
            ("PK_TOKEN_URL".into(), format!("{}/wtoken", self.base)),
            ("PK_AUTH_URL".into(), format!("{}/kc/realms/punchable", self.base)),
            ("PUNCHABLE_API".into(), self.base.clone()),
            ("PK_WORKER_SECRET".into(), "fake-worker-secret".into()),
        ];
        vars.extend(self.extra_env.clone());
        let env = Env::from_map(vars).with_home(root.join("home"));
        let svc = Services {
            clock: Box::new(Fixed::parse("2026-10-26T12:00:00+00:00").expect("time")),
            sleep: Box::new(|_| {}),
            opener: Box::new(|_| {}),
            spawn: Box::new(|_| Ok(())),
            free_mb: Box::new(|_| 50_000),
            restrict: Box::new(|_| true),
            google_token_uri: format!("{}/google/token", self.base),
            drive_root: self.base.clone(),
        };
        let args: Vec<String> = args.iter().map(ToString::to_string).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = cli::run(&args, &env, &svc, &mut out, &mut err);
        let w = self.world.lock().expect("lock");
        Outcome {
            code,
            out: String::from_utf8_lossy(&out).into_owned(),
            err: String::from_utf8_lossy(&err).into_owned(),
            seen: w.seen.clone(),
            posted: w.posted.clone(),
        }
    }
}

struct Outcome {
    code: i32,
    out: String,
    err: String,
    seen: Vec<String>,
    posted: Vec<String>,
}

impl Outcome {
    fn asked(&self, needle: &str) -> bool {
        self.seen.iter().any(|s| s.contains(needle))
    }
}

// ---- the rules file is required ---------------------------------------------------------------------------------------
#[test]
fn a_missing_rules_file_stops_run_before_any_request() {
    let c = Case::new();
    fs::remove_file(c.root().join("home/legal_rules.json")).expect("rm");
    let o = c.run(&["run", "--dry-run"]);
    assert_eq!(o.code, 1, "{} {}", o.out, o.err);
    assert!(
        o.err.contains("cannot be read") && o.err.contains("will not run"),
        "{}",
        o.err
    );
    assert!(o.seen.is_empty(), "a request was made: {:?}", o.seen);
}

#[test]
fn an_unusable_rules_file_stops_run_with_the_reason_and_repeats_nothing_from_the_file() {
    let marker = "private-marker-xyzzy";
    let cases: [(&str, String, &str); 4] = [
        ("not json", format!("{{ broken {marker}"), "not a JSON object"),
        ("an array", format!("[\"{marker}\"]"), "not a JSON object"),
        (
            "no owner",
            json!({"legal_parents": [marker], "keywords": [marker]}).to_string(),
            "no owner address",
        ),
        (
            "no entries of his own",
            json!({"owner": OWNER}).to_string(),
            "names none of his own",
        ),
    ];
    for (what, text, reason) in cases {
        let c = Case::new();
        write(&c.root(), "home/legal_rules.json", &text);
        let o = c.run(&["run"]);
        assert_eq!(o.code, 1, "{what}: {} {}", o.out, o.err);
        assert!(o.err.contains(reason), "{what}: {}", o.err);
        assert!(
            !o.err.contains(marker) && !o.out.contains(marker),
            "{what}: the file's words were printed"
        );
        assert!(o.seen.is_empty(), "{what}: a request was made: {:?}", o.seen);
    }
}

#[test]
fn the_owner_address_may_come_from_the_environment() {
    let mut c = Case::new();
    write(
        &c.root(),
        "home/legal_rules.json",
        &json!({"legal_parents": [LEGAL_FOLDER]}).to_string(),
    );
    c.extra_env.push(("PUNCHABLE_OWNER_EMAIL".into(), OWNER.into()));
    let o = c.run(&["run", "--dry-run"]);
    assert_eq!(o.code, 0, "{} {}", o.out, o.err);
}

#[test]
fn a_rules_file_that_goes_missing_between_passes_stops_the_loop() {
    let c = Case::new();
    let rules = c.root().join("home/legal_rules.json");
    let (base, root) = (c.base.clone(), c.root());
    let mut vars: Vec<(String, String)> = vec![
        (
            "PUNCHABLE_HOME".into(),
            root.join("home").to_string_lossy().into_owned(),
        ),
        ("PK_ONLINE_FIRST".into(), "1".into()),
        ("PK_WORKSPACE".into(), root.join("ws").to_string_lossy().into_owned()),
        ("PK_API".into(), base.clone()),
        ("PK_TOKEN_URL".into(), format!("{base}/wtoken")),
        ("PK_AUTH_URL".into(), format!("{base}/kc/realms/punchable")),
        ("PUNCHABLE_API".into(), base.clone()),
        ("PK_WORKER_SECRET".into(), "fake-worker-secret".into()),
    ];
    vars.extend(c.extra_env.clone());
    let env = Env::from_map(vars).with_home(root.join("home"));
    let sleeps = Arc::new(Mutex::new(0usize));
    let counted = sleeps.clone();
    let svc = Services {
        clock: Box::new(Fixed::parse("2026-10-26T12:00:00+00:00").expect("time")),
        // the pause between two passes: the rules file is removed during it
        sleep: Box::new(move |_| {
            let n = {
                let mut n = counted.lock().expect("lock");
                *n += 1;
                *n
            };
            assert!(n < 3, "the loop went on after its rules file was removed");
            let _ = fs::remove_file(&rules);
        }),
        opener: Box::new(|_| {}),
        spawn: Box::new(|_| Ok(())),
        free_mb: Box::new(|_| 50_000),
        restrict: Box::new(|_| true),
        google_token_uri: format!("{base}/google/token"),
        drive_root: base.clone(),
    };
    let args: Vec<String> = ["run", "--watch", "--dry-run"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = cli::run(&args, &env, &svc, &mut out, &mut err);
    let err = String::from_utf8_lossy(&err).into_owned();
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("cannot be read"), "{err}");
    assert_eq!(
        *sleeps.lock().expect("lock"),
        1,
        "one pass ran, then the pause, then the refusal"
    );
    assert!(
        String::from_utf8_lossy(&out).contains("[pull DRY RUN]"),
        "the first pass ran"
    );
}

// ---- a folder Drive does not return ------------------------------------------------------------------------------------
#[test]
fn a_parent_folder_drive_does_not_return_closes_the_doc_and_is_not_cached() {
    let c = Case::new();
    c.doc(
        meta("doc-orphan-0001", "A harmless title", &["folder-gone-0001"]),
        "words\nmore words\n",
    );
    let o = c.run(&["run"]);
    assert_eq!(o.code, 0, "{} {}", o.out, o.err);
    assert!(!o.asked("doc-orphan-0001/export"), "the doc was opened: {:?}", o.seen);
    let m = c.map();
    assert_eq!(m["excluded"]["doc-orphan-0001"], "parent", "{m}");
    assert!(
        m["folders"].get("folder-gone-0001").is_none(),
        "the missing folder was cached: {m}"
    );
    assert!(
        o.out.contains(r#""why": "parent""#) || o.out.contains("skip_legal 1"),
        "{}",
        o.out
    );
}

#[test]
fn an_error_inside_the_legal_check_opens_nothing_and_records_only_its_kind() {
    let c = Case::new();
    c.doc(
        meta("doc-check-0001", "A harmless title", &["folder-flaky-0001"]),
        "words\nmore words\n",
    );
    c.world
        .lock()
        .expect("lock")
        .refuse
        .insert("folder-flaky-0001".into(), 400);
    let o = c.run(&["run"]);
    assert!(!o.asked("doc-check-0001/export"), "the doc was opened: {:?}", o.seen);
    let m = c.map();
    assert_eq!(m["pending"]["doc-check-0001"], "legal_check", "{m}");
    assert!(
        m["excluded"].get("doc-check-0001").is_none(),
        "a transient error is retried, not final: {m}"
    );
    let runs = fs::read_to_string(c.root().join("ws/comedy/kb/gsync/runs.jsonl")).expect("runs");
    let rec: Value = serde_json::from_str(runs.lines().last().expect("a run")).expect("json");
    assert_eq!(rec["errors"][0]["error"], "legal check failed: HttpError", "{rec}");
    // the next pass looks at the doc again, and when the folder answers the doc goes through
    c.world.lock().expect("lock").refuse.clear();
    c.world
        .lock()
        .expect("lock")
        .files
        .insert("folder-flaky-0001".into(), folder("folder-flaky-0001", &["root0"]));
    let o = c.run(&["run"]);
    assert!(o.code == 0 || o.code == 2, "{} {}", o.out, o.err);
}

// ---- the matched keyword stays out of the state ------------------------------------------------------------------------
#[test]
fn the_keyword_that_matched_never_reaches_the_map_or_the_state_sent_to_the_database() {
    let c = Case::new();
    c.doc(
        meta("doc-kw-0000001", &format!("Notes about {KEYWORD} again"), &[OK_FOLDER]),
        "words\nmore words\n",
    );
    // a map an older pk-gsync (or the Python) wrote, with the words in it
    let mut old = c.map();
    old["excluded"] = json!({"old-doc-0000001": format!("kw:{KEYWORD}")});
    c.set_map(&old);
    let o = c.run(&["run"]);
    assert!(!o.asked("doc-kw-0000001/export"), "{:?}", o.seen);
    let m = c.map();
    assert_eq!(m["excluded"]["doc-kw-0000001"], "kw", "{m}");
    assert_eq!(
        m["excluded"]["old-doc-0000001"], "kw",
        "an old reason is cut to its kind: {m}"
    );
    assert!(!m.to_string().contains(KEYWORD), "the keyword is in the map: {m}");
    let state = o.posted.join("\n");
    assert!(state.contains("map.json"), "the state was not posted: {state}");
    assert!(!state.contains(KEYWORD), "the keyword was sent to the database");
    assert!(!o.out.contains(KEYWORD), "{}", o.out);
}

// ---- the content net, before the text is stored -------------------------------------------------------------------------
const LEGAL_TEXT: &str = "The plaintiff and the defendant\nsecond line\n";

#[test]
fn an_ai_draft_doc_whose_text_trips_the_content_net_is_dropped_before_it_is_stored() {
    let c = Case::new();
    c.doc(
        meta("sample-ai-draft-doc-0000000000000000000000", "A draft", &[OK_FOLDER]),
        LEGAL_TEXT,
    );
    let mut m = c.map();
    m["docs"] = json!({"sample-ai-draft-doc-0000000000000000000000": {"origin": "ai_draft", "source": "ai_draft",
        "contains_drafts": true, "title": "A draft", "joke_id": null, "version": "1", "modified": "2026-10-01T10:00:00.000Z",
        "doc_hash": "old"}});
    c.set_map(&m);
    let o = c.run(&["run"]);
    assert!(
        o.asked("-0000000000000000000000/export"),
        "the doc is looked at once, in memory: {:?}",
        o.seen
    );
    let edits = c.root().join("ws/comedy/kb/gsync/edits.jsonl");
    assert!(
        !edits.exists() || !fs::read_to_string(&edits).expect("edits").contains("plaintiff"),
        "the text was stored"
    );
    let m = c.map();
    assert_eq!(
        m["excluded"]["sample-ai-draft-doc-0000000000000000000000"], "content",
        "{m}"
    );
    assert_eq!(
        m["docs"]["sample-ai-draft-doc-0000000000000000000000"]["excluded"], true,
        "{m}"
    );
    assert_ne!(
        m["docs"]["sample-ai-draft-doc-0000000000000000000000"]["doc_hash"],
        json!(sha_of_norm(LEGAL_TEXT))
    );
}

#[test]
fn a_merged_doc_whose_text_trips_the_content_net_is_dropped_before_it_is_stored() {
    let c = Case::new();
    c.doc(meta("merged-doc-000000001", "A merge", &[OK_FOLDER]), LEGAL_TEXT);
    let mut m = c.map();
    m["docs"] = json!({"merged-doc-000000001": {"origin": "merged", "merge_id": "m1", "merged_joke": "joke:x",
        "title": "A merge", "version": "1", "modified": "2026-10-01T10:00:00.000Z", "doc_hash": "old"}});
    c.set_map(&m);
    let o = c.run(&["run"]);
    assert!(o.asked("merged-doc-000000001/export"), "{:?}", o.seen);
    let edits = c.root().join("ws/comedy/kb/gsync/edits.jsonl");
    assert!(
        !edits.exists() || !fs::read_to_string(&edits).expect("edits").contains("plaintiff"),
        "the text was stored"
    );
    assert_eq!(c.map()["excluded"]["merged-doc-000000001"], "content");
}

#[test]
fn an_adopted_doc_whose_text_trips_the_content_net_is_dropped_before_it_is_hashed() {
    let c = Case::new();
    c.doc(meta("adopted-doc-00000001", "An old doc", &[OK_FOLDER]), LEGAL_TEXT);
    let mut m = c.map();
    m["docs"] = json!({"adopted-doc-00000001": {"origin": "app", "adopted": true, "joke_id": null, "version": null,
        "doc_hash": null, "app_hash": null}});
    c.set_map(&m);
    c.run(&["run"]);
    let m = c.map();
    assert_eq!(m["excluded"]["adopted-doc-00000001"], "content", "{m}");
    assert!(
        m["docs"]["adopted-doc-00000001"]["doc_hash"].is_null(),
        "the text was hashed: {m}"
    );
    assert!(m["docs"]["adopted-doc-00000001"]["version"].is_null(), "{m}");
}

fn sha_of_norm(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(pk_gsync::units::norm(text).as_bytes()))
}

// ---- gsync's own docs are not read back (or written) when the rules close them -----------------------------------------
#[test]
fn a_doc_gsync_just_made_is_not_read_back_when_it_landed_in_a_legal_folder() {
    let c = Case::new();
    {
        let mut w = c.world.lock().expect("lock");
        w.jokes
            .push(("joke:an-app-idea".into(), "An app idea".into(), "app words".into()));
        let made = meta("made-doc-0000000001", "An app idea", &[LEGAL_FOLDER]);
        w.files.insert("made-doc-0000000001".into(), made.clone());
        w.texts.insert("made-doc-0000000001".into(), "app words".into());
        w.created = Some(made);
    }
    let o = c.run(&["run"]);
    assert!(o.asked("POST /upload/drive/v3/files"), "the doc was made: {:?}", o.seen);
    assert!(
        !o.asked("made-doc-0000000001/export"),
        "the doc was read back: {:?}",
        o.seen
    );
    let m = c.map();
    assert_eq!(m["excluded"]["made-doc-0000000001"], "folder", "{m}");
    assert!(m["docs"]["made-doc-0000000001"]["doc_hash"].is_null(), "{m}");
}

#[test]
fn an_app_edit_is_not_written_to_a_doc_the_rules_have_since_closed() {
    let c = Case::new();
    {
        let mut w = c.world.lock().expect("lock");
        w.jokes
            .push(("joke:an-app-idea".into(), "An app idea".into(), "new app words".into()));
        w.files.insert(
            "made-doc-0000000002".into(),
            meta("made-doc-0000000002", "An app idea", &[LEGAL_FOLDER]),
        );
        w.texts.insert("made-doc-0000000002".into(), "old app words".into());
    }
    let mut m = c.map();
    m["docs"] = json!({"made-doc-0000000002": {"origin": "app", "joke_id": "joke:an-app-idea", "title": "An app idea",
        "version": "2", "modified": "2026-10-20T10:00:00.000Z", "doc_hash": "d", "app_hash": "old"}});
    c.set_map(&m);
    let o = c.run(&["run"]);
    assert!(!o.asked("PATCH /upload"), "the doc was written: {:?}", o.seen);
    assert!(!o.asked("made-doc-0000000002/export"), "the doc was read: {:?}", o.seen);
    assert_eq!(c.map()["excluded"]["made-doc-0000000002"], "folder");
}

#[test]
fn a_board_doc_is_neither_read_nor_rewritten_when_the_rules_close_it() {
    let c = Case::new();
    {
        let mut w = c.world.lock().expect("lock");
        w.boards = json!({"sb:one": {"id": "sb:one", "title": "Set one", "by": "owner", "joke_ids": []}});
        w.files.insert(
            "board-doc-0000000001".into(),
            meta("board-doc-0000000001", "Set one (board)", &[LEGAL_FOLDER]),
        );
        w.texts.insert("board-doc-0000000001".into(), "board words".into());
    }
    let mut m = c.map();
    m["boards"] =
        json!({"sb:one": {"doc_id": "board-doc-0000000001", "hash": "old", "version": null, "doc_hash": "x"}});
    m["docs"] = json!({"board-doc-0000000001": {"origin": "board", "board": "sb:one"}});
    c.set_map(&m);
    let o = c.run(&["run"]);
    assert!(!o.asked("PATCH /upload"), "the board doc was written: {:?}", o.seen);
    assert!(
        !o.asked("board-doc-0000000001/export"),
        "the board doc was read: {:?}",
        o.seen
    );
    assert_eq!(c.map()["boards"]["sb:one"]["excluded"], true);
}

// ---- a voice memo is cleared again, fresh, before its bytes are downloaded ---------------------------------------------
#[test]
fn a_listed_voice_memo_in_a_legal_folder_is_not_downloaded() {
    let mut c = Case::new();
    c.extra_env.retain(|(k, _)| k != "PK_GSYNC_MEMOS");
    {
        let mut w = c.world.lock().expect("lock");
        let mut memo = meta("memo-file-00000001", "a memo.m4a", &[LEGAL_FOLDER]);
        memo["mimeType"] = json!("audio/x-m4a");
        w.files.insert("memo-file-00000001".into(), memo);
        w.files
            .insert("memo-folder-0001".into(), folder("memo-folder-0001", &[OK_FOLDER]));
    }
    write(
        &c.root(),
        "ws/comedy/kb/gsync/memos.json",
        &json!({"version": 1, "folder_id": "memo-folder-0001", "paused": null, "processor": {}, "items": {
            "memo-file-00000001": {"id": "memo-file-00000001", "source": "drive", "drive_id": "memo-file-00000001",
                                   "name": "a memo.m4a", "md5": "m1", "size": 5, "status": "listed", "tries": 0}}})
        .to_string(),
    );
    let mut m = c.map();
    m["folder_id"] = json!(OK_FOLDER);
    c.set_map(&m);
    let o = c.run(&["run"]);
    assert!(!o.asked("alt=media"), "the memo was downloaded: {:?}", o.seen);
    let memos: Value =
        serde_json::from_str(&fs::read_to_string(c.root().join("ws/comedy/kb/gsync/memos.json")).expect("memos"))
            .expect("json");
    assert_eq!(
        memos["items"]["memo-file-00000001"]["status"], "skipped_legal",
        "{memos}"
    );
    assert_eq!(memos["items"]["memo-file-00000001"]["why"], "folder", "{memos}");
}

// ---- with no owner address nothing is his ------------------------------------------------------------------------------
#[test]
fn a_doc_by_anyone_is_closed_when_no_owner_is_known() {
    use pk_gsync::rules::Rules;
    let r = Rules::new(&json!({"legal_parents": [LEGAL_FOLDER]}), None);
    let m = meta("a-doc-00000001", "Airport", &[OK_FOLDER]);
    assert_eq!(r.legal_reason(&m, &[], &serde_json::Map::new()), Some("owner"));
    assert!(r.clear(&m, &[], &serde_json::Map::new()).is_err());
    let r = Rules::new(&json!({"owner": OWNER}), None);
    assert!(r.clear(&m, &[], &serde_json::Map::new()).is_ok());
}
