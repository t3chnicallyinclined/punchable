//! The differential against `infra/backup/backup.py` (T11 round 7): the Python and `pk-backup` run against one scratch
//! `SurrealDB` 3.3.4 (the real schema, with rows in the upload tables) and, each, its own HTTPS S3 stand-in holding the same
//! objects. Compared, scenario by scenario: the exit status, the calls made on the bucket in order, the objects left
//! (the new ones by kind, the rest by name), the retention decisions, the exports byte for byte, the log lines, and
//! how both judge the same corpus of good and broken dump files. Then the restore drill: the Rust backup is imported into a
//! scratch database and every table's count and definition is compared with the live one.
//!
//! The Python is the oracle and is not modified; it needs a CA the stand-in's certificate chains to, which Python on
//! Windows only reads from the system store, so a six-line launcher installs the test CA into its default HTTPS
//! context before running `backup.py` as `__main__`.
//!
//! Needs the `surreal` binary (`PUNCH_SURREAL`, else `~/.punchable/bin/surreal`) and `python` (`PUNCH_PYTHON`). A missing
//! one skips the test with a message, unless `PK_REQUIRE_EXTERNAL` is set, where it fails.

#![allow(clippy::many_single_char_names, clippy::type_complexity, clippy::format_push_string)]

#[path = "support/standin.rs"]
mod standin;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use flate2::read::MultiGzDecoder;
use regex::Regex;
use serde_json::Value;
use standin::{Options, Pki, Standin};

const LAUNCHER: &str = "import ssl, sys, runpy
ca, script, *rest = sys.argv[1:]
ctx = ssl.create_default_context(cafile=ca)
ctx.verify_flags &= ~ssl.VERIFY_X509_STRICT
ssl._create_default_https_context = lambda: ctx
sys.argv = [script, *rest]
runpy.run_path(script, run_name='__main__')
";

fn pki() -> &'static Pki {
    static P: OnceLock<Pki> = OnceLock::new();
    P.get_or_init(Pki::new)
}

// ------------------------------------------------------------------------------------------ the externals

struct Externals {
    surreal: PathBuf,
    python: String,
    script: PathBuf,
}

fn require(why: &str) -> Option<Externals> {
    assert!(
        std::env::var_os("PK_REQUIRE_EXTERNAL").is_none_or(|v| v.is_empty()),
        "PK_REQUIRE_EXTERNAL is set and the differential cannot run: {why}"
    );
    eprintln!("SKIP differential: {why}");
    None
}

fn externals() -> Option<Externals> {
    let surreal = std::env::var_os("PUNCH_SURREAL").map_or_else(
        || {
            let home = std::env::home_dir().unwrap_or_default().join(".punchable").join("bin");
            let exe = home.join("surreal.exe");
            if exe.is_file() { exe } else { home.join("surreal") }
        },
        PathBuf::from,
    );
    if !surreal.is_file() {
        return require("no surreal binary (PUNCH_SURREAL or ~/.punchable/bin/surreal)");
    }
    let candidates: Vec<String> =
        std::env::var("PUNCH_PYTHON").map_or_else(|_| vec!["python".into(), "python3".into()], |p| vec![p]);
    let python = candidates.into_iter().find(|p| {
        Command::new(p)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    });
    let Some(python) = python else {
        return require("no python (PUNCH_PYTHON, python, python3)");
    };
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../infra/backup/backup.py");
    assert!(script.is_file(), "{} is missing", script.display());
    Some(Externals {
        surreal,
        python,
        script,
    })
}

// ------------------------------------------------------------------------------------------ a scratch SurrealDB

struct Scratch {
    child: Child,
    port: u16,
    root_pass: String,
    http: reqwest::blocking::Client,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

impl Scratch {
    fn start(exe: &Path) -> Self {
        let port = free_port();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let root_pass = format!("scratch-{nanos:x}-{:x}", std::process::id());
        let child = Command::new(exe)
            .args([
                "start",
                "--log",
                "warn",
                "--bind",
                &format!("127.0.0.1:{port}"),
                "memory",
            ])
            .env("SURREAL_USER", "root")
            .env("SURREAL_PASS", &root_pass)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start surreal");
        let mut s = Self {
            child,
            port,
            root_pass,
            http: reqwest::blocking::Client::new(),
        };
        let deadline = Instant::now() + Duration::from_secs(60);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(
                s.child.try_wait().expect("try_wait").is_none(),
                "surreal exited before listening"
            );
            assert!(Instant::now() < deadline, "surreal did not listen within 60s");
            std::thread::sleep(Duration::from_millis(100));
        }
        std::thread::sleep(Duration::from_millis(300));
        s
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Run `SurrealQL` as root in a namespace / database; every statement must be OK. Returns the results.
    fn sql(&self, ns: &str, db: &str, text: &str) -> Vec<Value> {
        let mut rb = self
            .http
            .post(format!("{}/sql", self.url()))
            .basic_auth("root", Some(&self.root_pass))
            .header("Accept", "application/json");
        if !ns.is_empty() {
            rb = rb.header("Surreal-NS", ns);
        }
        if !db.is_empty() {
            rb = rb.header("Surreal-DB", db);
        }
        let r = rb.body(text.to_string()).send().expect("sql");
        let status = r.status();
        let body = r.text().expect("body");
        assert!(
            status.is_success(),
            "sql HTTP {status}: {}",
            &body[..body.len().min(400)]
        );
        let v: Value = serde_json::from_str(&body).expect("json");
        let arr = v.as_array().expect("array").clone();
        for (i, st) in arr.iter().enumerate() {
            assert_eq!(
                st["status"],
                "OK",
                "statement {i} of {}: {}",
                &text[..text.len().min(60)],
                st
            );
        }
        arr
    }

    fn import(&self, ns: &str, db: &str, text: &[u8]) {
        let r = self
            .http
            .post(format!("{}/import", self.url()))
            .basic_auth("root", Some(&self.root_pass))
            .header("Surreal-NS", ns)
            .header("Surreal-DB", db)
            .header("Accept", "application/json")
            .body(text.to_vec())
            .send()
            .expect("import");
        let status = r.status();
        let body = r.text().expect("body");
        assert!(
            status.is_success(),
            "import HTTP {status}: {}",
            &body[..body.len().min(600)]
        );
        for st in serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
        {
            assert_eq!(st["status"], "OK", "import statement: {st}");
        }
    }

    /// `(table -> definition text)` and `(table -> row count)` of a database.
    fn tables(&self, ns: &str, db: &str) -> (BTreeMap<String, String>, BTreeMap<String, i64>) {
        let info = self.sql(ns, db, "INFO FOR DB;");
        let defs: BTreeMap<String, String> = info[0]["result"]["tables"]
            .as_object()
            .expect("tables")
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
            .collect();
        let mut counts = BTreeMap::new();
        for t in defs.keys() {
            let r = self.sql(ns, db, &format!("SELECT count() AS n FROM `{t}` GROUP ALL;"));
            counts.insert(
                t.clone(),
                r[0]["result"].get(0).and_then(|x| x["n"].as_i64()).unwrap_or(0),
            );
        }
        (defs, counts)
    }
}

const API_PASS: &str = "api-pass-for-the-scratch-only";

/// The real schema, an `api` EDITOR user, and rows in the tables a backup must carry and the three it must not.
fn seed(s: &Scratch) {
    s.sql(
        "",
        "",
        "DEFINE NAMESPACE punchable; USE NS punchable; DEFINE DATABASE main;",
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../db");
    for f in ["schema.surql", "schema_extra.surql"] {
        let text = std::fs::read_to_string(root.join(f)).expect("schema");
        s.sql("punchable", "main", &text);
    }
    s.sql(
        "punchable",
        "main",
        &format!("DEFINE USER api ON DATABASE PASSWORD '{API_PASS}' ROLES EDITOR;"),
    );
    let mut q = String::new();
    for i in 0..60 {
        q.push_str(&format!(
            "CREATE joke:j{i} CONTENT {{ ord: {i}, title: 'joke number {i}', stage: 'idea', summary: 'a summary of {i}' }};\n\
             CREATE unit:u{i} CONTENT {{ ord: {i}, text: 'the line {i} of the set, with an apostrophe\\'s worth of text' }};\n"
        ));
    }
    q.push_str("UPSERT meta:state MERGE { gen: 7, loading: false, ord_next: 61 };\n");
    // the recordings: rows that must be in the live database and not in a backup
    for i in 0..5 {
        q.push_str(&format!(
            "CREATE upload:`rec{i}.m4a` CONTENT {{ name: 'rec{i}.m4a', token: 'tok{i}', meta: {{ kind: 'set' }}, bytes: 2048, parts: 2, t: 1.0 }};\n\
             CREATE upload_part:`tok{i}_0` CONTENT {{ token: 'tok{i}', seq: 0, data: <bytes>\"{}\" }};\n\
             CREATE upload_part:`tok{i}_1` CONTENT {{ token: 'tok{i}', seq: 1, data: <bytes>\"{}\" }};\n\
             CREATE upload_stage:`tok{i}` CONTENT {{ t: 1.0, n: 2 }};\n",
            "x".repeat(1024),
            "y".repeat(1024)
        ));
    }
    s.sql("punchable", "main", &q);
}

// ------------------------------------------------------------------------------------------ running an implementation

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Who {
    Python,
    Rust,
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

struct Rig<'a> {
    ext: &'a Externals,
    db: &'a Scratch,
    dir: tempfile::TempDir,
    ca: PathBuf,
}

impl<'a> Rig<'a> {
    fn new(ext: &'a Externals, db: &'a Scratch) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let ca = dir.path().join("ca.pem");
        std::fs::write(&ca, &pki().ca_pem).expect("ca");
        Self { ext, db, dir, ca }
    }

    fn base_env(&self, st: Option<&Standin>) -> BTreeMap<String, String> {
        let mut e: BTreeMap<String, String> = BTreeMap::new();
        e.insert("SURREAL_URL".into(), self.db.url());
        e.insert("SURREAL_USER".into(), "api".into());
        e.insert("SURREAL_PASS".into(), API_PASS.into());
        e.insert("BACKUP_TMP".into(), self.dir.path().display().to_string());
        if let Some(s) = st {
            e.insert("R2_ENDPOINT".into(), s.endpoint.clone());
            e.insert("R2_BUCKET".into(), s.bucket.clone());
            e.insert("R2_ACCESS_KEY_ID".into(), s.key_id.clone());
            e.insert("R2_SECRET_ACCESS_KEY".into(), s.secret.clone());
        }
        e
    }

    fn run(&self, who: Who, env: &BTreeMap<String, String>, args: &[&str]) -> Run {
        let mut cmd;
        match who {
            Who::Python => {
                cmd = Command::new(&self.ext.python);
                cmd.args(["-I", "-c", LAUNCHER])
                    .arg(&self.ca)
                    .arg(&self.ext.script)
                    .args(args);
                cmd.env("PYTHONDONTWRITEBYTECODE", "1");
            }
            Who::Rust => {
                cmd = Command::new(env!("CARGO_BIN_EXE_pk-backup"));
                cmd.args(args).env("SSL_CERT_FILE", &self.ca);
            }
        }
        cmd.env_clear();
        for k in ["SystemRoot", "PATH", "TEMP", "TMP", "USERPROFILE"] {
            if let Some(v) = std::env::var_os(k) {
                cmd.env(k, v);
            }
        }
        if who == Who::Rust {
            cmd.env("SSL_CERT_FILE", &self.ca);
        }
        if who == Who::Python {
            cmd.env("PYTHONDONTWRITEBYTECODE", "1");
        }
        cmd.envs(env);
        let o = cmd.output().unwrap_or_else(|e| panic!("run {who:?}: {e}"));
        Run {
            code: o.status.code().unwrap_or(-1),
            // Python's text-mode stdout writes CRLF on Windows
            out: String::from_utf8_lossy(&o.stdout).replace('\r', ""),
            err: String::from_utf8_lossy(&o.stderr).replace('\r', ""),
        }
    }
}

fn stamp_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"[0-9]{8}T[0-9]{6}Z").expect("regex"))
}

/// A stand-in with the objects of an earlier life: 30 dailies, 12 weeklies, other databases' and foreign keys.
fn planted(prefix: &str, bugsink: &str) -> Standin {
    let st = Standin::start(
        pki(),
        Options {
            page: 7,
            ..Options::default()
        },
    );
    for d in 1..=30 {
        st.plant(
            &format!("{prefix}daily/punchable-main-202610{d:02}T041700Z.surql.gz"),
            format!("old daily {d}").as_bytes(),
        );
    }
    for d in [4, 11, 18, 25] {
        st.plant(
            &format!("{prefix}weekly/punchable-main-202610{d:02}T041700Z.surql.gz"),
            b"old weekly",
        );
    }
    for m in 1..=8 {
        st.plant(
            &format!("{prefix}weekly/punchable-main-2026{m:02}01T041700Z.surql.gz"),
            b"older weekly",
        );
    }
    for d in 1..=16 {
        st.plant(
            &format!("{bugsink}daily/bugsink-db-202610{d:02}T041700Z.sqlite3.gz"),
            b"old bugsink",
        );
    }
    for f in [
        "audio/set-1/tape.m4a",
        "surreal-selftest/daily/punchable-main-20260101T000000Z.surql.gz",
        "surreal/daily/notes.txt",
        "surreal/daily/punchable-main-20260101T000000Z.surql.gz.part",
        "surreal/monthly/punchable-main-20260101T000000Z.surql.gz",
        "surreal/daily/other-main-20200101T000000Z.surql.gz",
        "surreal/daily/punchable-main_old-20200101T000000Z.surql.gz",
        "bugsink/daily/bugsink-db-20200101T000000Z.surql.gz",
    ] {
        st.plant(f, b"not ours");
    }
    st
}

/// What a run did, with the new objects' time stamps replaced by `<stamp>` so two runs a second apart compare.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    code: i32,
    err_class: String,
    ops: Vec<String>,
    keys: Vec<String>,
    log: Vec<String>,
}

fn normalise_stamp(text: &str) -> String {
    stamp_re().replace_all(text, "<stamp>").into_owned()
}

fn outcome(r: &Run, st: &Standin, before: &BTreeSet<String>) -> Outcome {
    let keys: Vec<String> = st
        .keys()
        .into_iter()
        .map(|k| {
            if before.contains(&k) {
                k
            } else {
                format!("NEW {}", normalise_stamp(&k))
            }
        })
        .collect();
    let digits = Regex::new(r"-> [0-9]+ gz").expect("regex");
    let secs = Regex::new(r", [0-9.]+s$").expect("regex");
    let sizes = Regex::new(r"\([0-9]+ bytes, verified\)").expect("regex");
    let log = r
        .out
        .lines()
        .map(|l| {
            let l = normalise_stamp(l);
            let l = digits.replace(&l, "-> N gz");
            let l = secs.replace(&l, ", Ts");
            sizes.replace(&l, "(N bytes, verified)").into_owned()
        })
        .collect();
    let ops = st.ops().iter().map(|o| normalise_stamp(o)).collect();
    // the class of a failure: its first words up to the first colon after "FAIL"
    let err_class = normalise_stamp(
        &r.err
            .lines()
            .next()
            .unwrap_or("")
            .split(':')
            .take(2)
            .collect::<Vec<_>>()
            .join(":"),
    );
    Outcome {
        code: r.code,
        err_class,
        ops,
        keys,
        log,
    }
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    MultiGzDecoder::new(bytes).read_to_end(&mut out).expect("gunzip");
    out
}

/// One night with each implementation, each against its own copy of the same bucket; compared.
fn night(
    rig: &Rig<'_>,
    name: &str,
    prefix: &str,
    bugsink: &str,
    extra: &[(&str, &str)],
    sqlite: Option<&Path>,
    tweak: impl Fn(&Standin),
) -> (Outcome, Outcome, [Standin; 2]) {
    let mut results = Vec::new();
    let mut stands = Vec::new();
    for who in [Who::Python, Who::Rust] {
        let st = planted(prefix, bugsink);
        tweak(&st);
        let before: BTreeSet<String> = st.keys().into_iter().collect();
        let mut env = rig.base_env(Some(&st));
        env.insert("BACKUP_PREFIX".into(), prefix.into());
        env.insert("BUGSINK_PREFIX".into(), bugsink.into());
        if let Some(p) = sqlite {
            env.insert("BUGSINK_DB_PATH".into(), p.display().to_string());
        }
        for (k, v) in extra {
            env.insert((*k).into(), (*v).into());
        }
        let r = rig.run(who, &env, &["run"]);
        eprintln!(
            "[{name}] {who:?}: exit {} | {}",
            r.code,
            r.out.lines().last().unwrap_or(&r.err)
        );
        results.push(outcome(&r, &st, &before));
        stands.push(st);
    }
    let rust = results.pop().expect("rust");
    let python = results.pop().expect("python");
    let rust_st = stands.pop().expect("st");
    let py_st = stands.pop().expect("st");
    (python, rust, [py_st, rust_st])
}

fn assert_same(name: &str, py: &Outcome, rs: &Outcome) {
    assert_eq!(py.code, rs.code, "[{name}] exit status");
    assert_eq!(py.err_class, rs.err_class, "[{name}] failure class");
    assert_eq!(py.ops, rs.ops, "[{name}] the calls made on the bucket, in order");
    assert_eq!(py.keys, rs.keys, "[{name}] the objects left in the bucket");
    assert_eq!(py.log, rs.log, "[{name}] the log lines");
}

/// The keys a night added: everything that was not planted.
fn new_keys(st: &Standin, planted_keys: &BTreeSet<String>) -> Vec<String> {
    st.keys().into_iter().filter(|k| !planted_keys.contains(k)).collect()
}

fn make_sqlite(path: &Path) {
    let c = rusqlite::Connection::open(path).expect("open");
    c.pragma_update(None, "journal_mode", "WAL").expect("wal");
    c.execute("CREATE TABLE event (id INTEGER PRIMARY KEY, body TEXT)", [])
        .expect("create");
    c.execute("CREATE TABLE issue (id INTEGER PRIMARY KEY, title TEXT)", [])
        .expect("create");
    for i in 0..300 {
        c.execute(
            "INSERT INTO event (body) VALUES (?1)",
            [&format!("{i}-{}", "x".repeat(100))],
        )
        .expect("insert");
    }
    c.execute("INSERT INTO issue (title) VALUES ('boom')", [])
        .expect("insert");
}

fn dump_sqlite(bytes: &[u8], dir: &Path) -> Vec<String> {
    let p = dir.join(format!("copy-{}.sqlite3", bytes.len()));
    std::fs::write(&p, bytes).expect("write");
    let c = rusqlite::Connection::open(&p).expect("open");
    let mut rows = Vec::new();
    for t in ["event", "issue"] {
        let mut st = c.prepare(&format!("SELECT * FROM {t} ORDER BY id")).expect("prepare");
        let n = st.column_count();
        let it = st
            .query_map([], |r| {
                Ok((0..n)
                    .map(|i| format!("{:?}", r.get_ref(i).map(|v| format!("{v:?}"))))
                    .collect::<Vec<_>>()
                    .join("|"))
            })
            .expect("query");
        rows.extend(it.map(|x| format!("{t}:{}", x.expect("row"))));
    }
    rows
}

// ------------------------------------------------------------------------------------------ the differential

#[test]
fn the_rust_job_matches_the_python_on_the_same_database_and_bucket() {
    let Some(ext) = externals() else { return };
    let db = Scratch::start(&ext.surreal);
    seed(&db);
    let rig = Rig::new(&ext, &db);
    let (live_defs, live_counts) = db.tables("punchable", "main");
    assert!(
        live_counts["upload_part"] == 10 && live_counts["joke"] == 60,
        "{live_counts:?}"
    );

    // --- the exports, byte for byte
    let (py_exp, rs_exp) = (rig.dir.path().join("py.surql.gz"), rig.dir.path().join("rs.surql.gz"));
    let env = rig.base_env(None);
    let a = rig.run(Who::Python, &env, &["export", &py_exp.display().to_string()]);
    let b = rig.run(Who::Rust, &env, &["export", &rs_exp.display().to_string()]);
    eprintln!("[export] python: {} | rust: {}", a.out.trim(), b.out.trim());
    assert_eq!((a.code, b.code), (0, 0), "{} {}", a.err, b.err);
    let (py_sql, rs_sql) = (
        gunzip(&std::fs::read(&py_exp).expect("py")),
        gunzip(&std::fs::read(&rs_exp).expect("rs")),
    );
    assert_eq!(py_sql.len(), rs_sql.len(), "export length");
    assert!(
        py_sql == rs_sql,
        "the Rust export differs from the Python's for the same database"
    );
    let sql_text = String::from_utf8(rs_sql.clone()).expect("utf8");
    assert!(
        sql_text.contains("DEFINE TABLE upload_part") && !sql_text.contains("tok0_0") && !sql_text.contains("rec0.m4a")
    );
    assert_eq!(sql_text.matches("OPTION IMPORT;").count(), 1);
    let line = |r: &Run| {
        let l = Regex::new(r"-> .*? \(")
            .expect("regex")
            .replace(&r.out, "-> F (")
            .into_owned();
        Regex::new(r"\([0-9]+ gz,")
            .expect("regex")
            .replace(&l, "(N gz,")
            .into_owned()
    };
    assert_eq!(line(&a), line(&b), "the export log line");

    // --- nights: the same planted bucket, each implementation, compared call by call
    let bugsink_db = rig.dir.path().join("bugsink.sqlite3");
    make_sqlite(&bugsink_db);
    let mut kept_for_fetch: Option<[Standin; 2]> = None;
    let scenarios: Vec<(&str, &str, &str, Vec<(&str, &str)>, bool)> = vec![
        (
            "weekly always + bugsink",
            "surreal/",
            "bugsink/",
            vec![("BACKUP_WEEKLY", "always")],
            true,
        ),
        (
            "keep 3 and 2, no weekly",
            "surreal/",
            "bugsink/",
            vec![
                ("BACKUP_WEEKLY", "never"),
                ("BACKUP_KEEP_DAILY", "3"),
                ("BACKUP_KEEP_WEEKLY", "2"),
            ],
            true,
        ),
        (
            "weekly auto (today)",
            "surreal/",
            "bugsink/",
            vec![("BACKUP_WEEKLY", "auto")],
            false,
        ),
        (
            "the parallel-night prefixes",
            "surreal-rs/",
            "bugsink-rs/",
            vec![("BACKUP_WEEKLY", "always")],
            true,
        ),
        (
            "keep 1 and 1",
            "surreal/",
            "bugsink/",
            vec![
                ("BACKUP_WEEKLY", "always"),
                ("BACKUP_KEEP_DAILY", "1"),
                ("BACKUP_KEEP_WEEKLY", "1"),
            ],
            false,
        ),
    ];
    for (i, (name, pfx, bpfx, extra, with_bugsink)) in scenarios.iter().enumerate() {
        let sq = with_bugsink.then_some(bugsink_db.as_path());
        let (py, rs, stands) = night(&rig, name, pfx, bpfx, extra, sq, |_| {});
        assert_eq!(py.code, 0, "[{name}] python night failed: {:?}", py.err_class);
        assert_same(name, &py, &rs);
        let deleted = |o: &Outcome| o.ops.iter().filter(|x| x.starts_with("DELETE")).count();
        assert!(deleted(&rs) > 0, "[{name}] the scenario should prune something");
        // the new objects hold the same bytes: the export identical, Bugsink's copy the same rows
        let was: BTreeSet<String> = planted(pfx, bpfx).keys().into_iter().collect();
        let (pn, rn) = (new_keys(&stands[0], &was), new_keys(&stands[1], &was));
        assert_eq!(pn.len(), rn.len(), "[{name}] {pn:?} {rn:?}");
        for (pk, rk) in pn.iter().zip(&rn) {
            let (pb, rb) = (
                stands[0].bytes(pk).expect("py bytes"),
                stands[1].bytes(rk).expect("rs bytes"),
            );
            if pk.ends_with(".surql.gz") {
                assert!(gunzip(&pb) == gunzip(&rb), "[{name}] {pk}: the stored export differs");
                assert!(
                    gunzip(&rb) == rs_sql,
                    "[{name}] the stored export is not today's export"
                );
            } else {
                assert_eq!(
                    dump_sqlite(&gunzip(&pb), rig.dir.path()),
                    dump_sqlite(&gunzip(&rb), rig.dir.path()),
                    "[{name}] {pk}: Bugsink rows"
                );
            }
        }
        eprintln!(
            "[{name}] same: {} calls, {} objects left, {} deleted, exports equal ({} bytes)",
            rs.ops.len(),
            rs.keys.len(),
            deleted(&rs),
            rs_sql.len()
        );
        if i == 0 {
            kept_for_fetch = Some(stands);
        }
    }

    // --- failures: both fail the same way and neither deletes
    for (name, mutate_env, tweak) in [
        (
            "a wrong database password",
            Some(("SURREAL_PASS", "wrong-password-entirely")),
            0,
        ),
        (
            "a wrong bucket secret",
            Some(("R2_SECRET_ACCESS_KEY", "not-the-secret-at-all")),
            0,
        ),
        ("an upload that arrives short", None, 1),
        ("the bucket refuses the upload", None, 2),
        ("an unknown namespace", Some(("SURREAL_NS", "nothere")), 0),
        ("a bad auth level", Some(("SURREAL_AUTH_LEVEL", "namespace")), 0),
        ("a bad weekly mode", Some(("BACKUP_WEEKLY", "sometimes")), 0),
        (
            "a prefix outside the allowed ones",
            Some(("BACKUP_PREFIX", "audio/")),
            0,
        ),
    ] {
        let mut results = Vec::new();
        for who in [Who::Python, Who::Rust] {
            let st = planted("surreal/", "bugsink/");
            match tweak {
                1 => st.store_half(true),
                2 => st.fail("put", 403, -1),
                _ => {}
            }
            let before: BTreeSet<String> = st.keys().into_iter().collect();
            let mut env = rig.base_env(Some(&st));
            env.insert("BACKUP_WEEKLY".into(), "always".into());
            if let Some((k, v)) = mutate_env {
                env.insert(k.into(), v.into());
            }
            let r = rig.run(who, &env, &["run"]);
            assert_eq!(r.code, 1, "[{name}] {who:?}: {}", r.err);
            assert_eq!(r.err.lines().count(), 1, "[{name}] {who:?}: one line: {}", r.err);
            assert!(r.err.starts_with("backup: FAIL "), "[{name}] {who:?}: {}", r.err);
            for secret in [API_PASS, st.secret.as_str()] {
                assert!(
                    !r.err.contains(secret) && !r.out.contains(secret),
                    "[{name}] {who:?} printed a secret"
                );
            }
            assert_eq!(st.count("DELETE"), 0, "[{name}] {who:?} deleted after a failed night");
            assert!(
                st.keys().iter().all(|k| before.contains(k) || tweak == 1),
                "[{name}] {who:?}"
            );
            results.push(outcome(&r, &st, &before));
        }
        let (rs, py) = (results.pop().expect("r"), results.pop().expect("p"));
        assert_eq!(py.ops, rs.ops, "[{name}] calls");
        assert_eq!(py.keys, rs.keys, "[{name}] objects");
        eprintln!(
            "[fail: {name}] both exit 1 | python: {} | rust: {}",
            py.err_class, rs.err_class
        );
    }

    // --- list: the same listing in the same words, across pages
    {
        let (mut outs, mut keep) = (Vec::new(), Vec::new());
        for who in [Who::Python, Who::Rust] {
            let st = planted("surreal/", "bugsink/");
            let r = rig.run(who, &rig.base_env(Some(&st)), &["list"]);
            assert_eq!(r.code, 0, "{}", r.err);
            outs.push(r.out);
            keep.push(st);
        }
        assert_eq!(outs[0], outs[1], "list output");
        assert!(
            outs[0].lines().count() > 14 && outs[0].lines().next().is_some_and(|l| l.contains("surreal/daily/")),
            "{}",
            outs[0]
        );
        assert!(
            keep[0].count("LIST") >= 4 && keep[0].count("LIST") == keep[1].count("LIST"),
            "paging"
        );
    }

    // --- fetch and restore-check, crossed: each reads the other's backup, and judges every file the same
    let stands = kept_for_fetch.expect("first scenario ran");
    let was: BTreeSet<String> = planted("surreal/", "bugsink/").keys().into_iter().collect();
    let new_in = |s: &Standin| {
        new_keys(s, &was)
            .into_iter()
            .find(|k| k.starts_with("surreal/daily/"))
            .expect("new daily")
    };
    let (py_key, rs_key) = (new_in(&stands[0]), new_in(&stands[1]));
    let crossed = [
        (Who::Rust, &stands[0], &py_key, "rust-fetches-python"),
        (Who::Python, &stands[1], &rs_key, "python-fetches-rust"),
    ];
    let mut fetched = Vec::new();
    for (who, st, key, tag) in crossed {
        let out = rig.dir.path().join(format!("{tag}.surql.gz"));
        let r = rig.run(
            who,
            &rig.base_env(Some(st)),
            &["fetch", key, &out.display().to_string()],
        );
        assert_eq!(r.code, 0, "[{tag}] {}", r.err);
        assert_eq!(
            std::fs::read(&out).expect("fetched"),
            st.bytes(key).expect("bytes"),
            "[{tag}] the bytes"
        );
        fetched.push(out);
    }
    for (who, st, key) in [(Who::Python, &stands[0], &py_key), (Who::Rust, &stands[1], &rs_key)] {
        let r = rig.run(who, &rig.base_env(Some(st)), &["fetch", "audio/set-1/tape.m4a", "x.gz"]);
        assert_eq!(
            (r.code, r.err.trim()),
            (1, "backup: FAIL key must be under surreal/"),
            "{who:?}"
        );
        let r = rig.run(
            who,
            &rig.base_env(Some(st)),
            &["fetch", &format!("{key}.nothere"), "x.gz"],
        );
        assert_eq!(r.code, 1, "{who:?}");
        assert!(r.err.contains(": not found"), "{who:?}: {}", r.err);
    }

    // restore-check over a corpus: the same file, both implementations, the same verdict
    let good = std::fs::read(&fetched[0]).expect("good");
    let raw = gunzip(&good);
    let gz = |d: &[u8]| {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
        e.write_all(d).expect("gz");
        e.finish().expect("gz")
    };
    let mut flipped = good.clone();
    let n = flipped.len();
    flipped[n - 8] ^= 0xff;
    let text = String::from_utf8(raw.clone()).expect("utf8");
    let mut two_members = good.clone();
    two_members.extend_from_slice(&gz(b"-- more;\n"));
    let corpus: Vec<(&str, Vec<u8>)> = vec![
        ("good", good.clone()),
        ("truncated", good[..good.len() / 2].to_vec()),
        ("flipped crc", flipped),
        ("tiny", gz(&raw[..300])),
        ("no tables", gz(text.replace("DEFINE TABLE", "DEFINE FIELD").as_bytes())),
        ("cut statement", gz(&raw[..raw.len() - 20])),
        ("not an export", gz(&b"hello;\n".repeat(2000))),
        ("empty file", Vec::new()),
        ("not gzip", vec![b'x'; 9000]),
        ("trailing zeros", [good.clone(), vec![0u8; 16]].concat()),
        (
            "trailing garbage",
            [good.clone(), b"garbage after the member".to_vec()].concat(),
        ),
        ("two members", two_members),
        (
            "trailing newline in the export",
            gz(&[raw.clone(), b"\n\n  \n".to_vec()].concat()),
        ),
    ];
    let class = |r: &Run| -> String {
        let e = r.err.lines().next().unwrap_or("");
        for c in [
            "not a complete gzip",
            "export is only",
            "OPTION IMPORT",
            "DEFINE TABLE",
            "complete statement",
            "no such file",
        ] {
            if e.contains(c) {
                return c.to_string();
            }
        }
        e.to_string()
    };
    let mut verdicts = Vec::new();
    for (name, bytes) in &corpus {
        let p = rig
            .dir
            .path()
            .join(format!("corpus-{}.surql.gz", name.replace(' ', "-")));
        std::fs::write(&p, bytes).expect("write");
        let arg = p.display().to_string();
        let env = rig.base_env(None);
        let (a, b) = (
            rig.run(Who::Python, &env, &["restore-check", &arg]),
            rig.run(Who::Rust, &env, &["restore-check", &arg]),
        );
        if name == &"trailing garbage" || name == &"trailing zeros" || name == &"two members" {
            verdicts.push(format!("{name}: python exit {} / rust exit {}", a.code, b.code));
        }
        assert_eq!(
            a.code,
            b.code,
            "[corpus {name}] exit status: python `{}` / rust `{}`",
            a.err.trim(),
            b.err.trim()
        );
        assert_eq!(
            class(&a),
            class(&b),
            "[corpus {name}] which check fired: python `{}` / rust `{}`",
            a.err.trim(),
            b.err.trim()
        );
        if a.code == 0 {
            assert_eq!(
                a.out.replace(&arg, "F"),
                b.out.replace(&arg, "F"),
                "[corpus {name}] the ok line"
            );
        }
    }
    let r = rig.run(Who::Python, &rig.base_env(None), &["restore-check", "no-such-file.gz"]);
    let s = rig.run(Who::Rust, &rig.base_env(None), &["restore-check", "no-such-file.gz"]);
    assert_eq!((r.code, r.err.as_str()), (s.code, s.err.as_str()), "a missing file");
    eprintln!("[corpus] {} files judged alike; {}", corpus.len(), verdicts.join("; "));

    // --- the restore drill: the Rust export, imported into a scratch database, against the live one
    db.import("punchable", "restore_check", &rs_sql);
    let (rd, rc) = db.tables("punchable", "restore_check");
    assert_eq!(
        rd.keys().collect::<Vec<_>>(),
        live_defs.keys().collect::<Vec<_>>(),
        "the same tables"
    );
    for (t, n) in &live_counts {
        let want = if ["upload", "upload_part", "upload_stage"].contains(&t.as_str()) {
            0
        } else {
            *n
        };
        assert_eq!(rc[t], want, "rows of {t}");
    }
    assert_eq!(
        rd, live_defs,
        "every table definition, field and index survives the round trip"
    );
    eprintln!(
        "[drill] {} tables restored; counts equal ({} rows); upload tables defined and empty",
        rc.len(),
        rc.values().sum::<i64>()
    );
}
