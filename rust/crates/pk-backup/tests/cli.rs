//! The job end to end against an HTTPS S3 stand-in and a stub `SurrealDB`: what it uploads, what it keeps and deletes,
//! what it refuses, and how it exits. The "red arms" are here too: each check the Python makes before it trusts an
//! upload or deletes anything has a test that makes the check fire and shows nothing was deleted.

#[path = "support/standin.rs"]
mod standin;
#[path = "support/surreal_stub.rs"]
mod surreal_stub;

use std::sync::OnceLock;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use pk_backup::Vars;
use pk_backup::cli::{Ctx, run, run_database};
use pk_backup::dump::ExportConfig;
use pk_backup::s3::S3;
use standin::{Options, Pki, Standin};
use surreal_stub::{Mode, SurrealStub};

const PASS: &str = "pw-for-the-test-only";
const SECRET: &str = "test-secret-not-a-real-one";

fn pki() -> &'static Pki {
    static P: OnceLock<Pki> = OnceLock::new();
    P.get_or_init(Pki::new)
}

struct Rig {
    st: Standin,
    stub: SurrealStub,
    dir: tempfile::TempDir,
    vars: Vars,
}

fn rig(mode: Mode, extra: &[(&str, &str)]) -> Rig {
    let st = Standin::start(pki(), Options::default());
    let stub = SurrealStub::start(
        &["decision", "joke", "upload", "upload_part", "upload_stage"],
        mode,
        100,
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let mut pairs: Vec<(String, String)> = vec![
        ("SURREAL_URL".into(), stub.url.clone()),
        ("SURREAL_USER".into(), "api".into()),
        ("SURREAL_PASS".into(), PASS.into()),
        ("R2_ENDPOINT".into(), st.endpoint.clone()),
        ("R2_BUCKET".into(), st.bucket.clone()),
        ("R2_ACCESS_KEY_ID".into(), st.key_id.clone()),
        ("R2_SECRET_ACCESS_KEY".into(), st.secret.clone()),
        ("BACKUP_TMP".into(), dir.path().display().to_string()),
        ("BACKUP_WEEKLY".into(), "never".into()),
    ];
    for (k, v) in extra {
        pairs.retain(|(pk, _)| pk != k);
        pairs.push(((*k).to_string(), (*v).to_string()));
    }
    let vars = Vars::from_pairs(pairs);
    Rig { st, stub, dir, vars }
}

/// `(exit, stdout, stderr)` of one command line, in process.
fn cli(r: &Rig, args: &[&str]) -> (i32, String, String) {
    let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
    let roots = vec![r.st.ca_pem.clone().into_bytes()];
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = {
        let mut ctx = Ctx {
            vars: &r.vars,
            roots: &roots,
            out: &mut out,
            retry_unit: Some(Duration::from_millis(1)),
        };
        run(&args, &mut ctx, &mut err)
    };
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

fn old_daily(day: u32) -> String {
    format!("surreal/daily/punchable-main-202610{day:02}T041700Z.surql.gz")
}

fn old_weekly(day: u32) -> String {
    format!("surreal/weekly/punchable-main-202610{day:02}T041700Z.surql.gz")
}

fn no_secret_leaked(r: &Rig, text: &str) {
    for s in [PASS, SECRET, r.st.key_id.as_str()] {
        assert!(!text.contains(s), "a secret value reached the output: {text}");
    }
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::MultiGzDecoder::new(bytes)
        .read_to_end(&mut out)
        .expect("gunzip");
    out
}

// ------------------------------------------------------------------------------------------ the happy path

#[test]
fn a_night_uploads_verifies_copies_the_weekly_and_logs_what_it_did() {
    let r = rig(Mode::Normal, &[("BACKUP_WEEKLY", "always")]);
    let (code, out, err) = cli(&r, &["run"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    let keys = r.st.keys();
    assert_eq!(keys.len(), 2, "{keys:?}");
    let daily = keys
        .iter()
        .find(|k| k.starts_with("surreal/daily/punchable-main-"))
        .expect("daily");
    let weekly = keys
        .iter()
        .find(|k| k.starts_with("surreal/weekly/punchable-main-"))
        .expect("weekly");
    assert!(daily.ends_with("Z.surql.gz") && weekly.ends_with(&daily["surreal/daily/".len()..]));
    // the order of the calls: PUT, HEAD, COPY, HEAD, LIST (and no DELETE: nothing to prune)
    let ops: Vec<String> =
        r.st.ops()
            .iter()
            .map(|o| o.split(' ').next().unwrap_or("").to_string())
            .collect();
    assert_eq!(ops, ["PUT", "HEAD", "COPY", "HEAD", "LIST"]);
    assert_eq!(r.st.refused(), 0, "every signature and payload hash was accepted");
    // what was stored is a complete export: the kept tables with rows, the upload tables as definitions only
    let sql = String::from_utf8(gunzip(&r.st.bytes(daily).expect("bytes"))).expect("utf8");
    assert!(sql.contains("DEFINE TABLE joke") && sql.contains("INSERT [ { id: joke:r0"));
    assert!(sql.contains("DEFINE TABLE upload_part") && !sql.contains("upload_part:r0") && !sql.contains("upload:r0"));
    assert_eq!(
        sql.matches("OPTION IMPORT;").count(),
        1,
        "an import refuses a second OPTION IMPORT"
    );
    assert_eq!(r.st.bytes(daily), r.st.bytes(weekly));
    // the log lines
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[0].starts_with("export ok: punchable/main ") && lines[0].contains(" tables, "),
        "{out}"
    );
    assert!(
        lines[1].starts_with(&format!("uploaded {daily} (")) && lines[1].ends_with(" bytes, verified)"),
        "{out}"
    );
    assert_eq!(lines[2], format!("weekly copy {weekly} (verified)"));
    assert_eq!(lines[3], "retention: kept 2 (14 daily / 8 weekly max), deleted 0");
    no_secret_leaked(&r, &out);
}

#[test]
fn bugsink_follows_the_database_with_its_own_prefix_and_retention() {
    let r0 = rig(Mode::Normal, &[]);
    let db = r0.dir.path().join("db.sqlite3");
    {
        let c = rusqlite::Connection::open(&db).expect("open");
        c.pragma_update(None, "journal_mode", "WAL").expect("wal");
        c.execute("CREATE TABLE event (id INTEGER PRIMARY KEY, body TEXT)", [])
            .expect("create");
        for _ in 0..200 {
            c.execute("INSERT INTO event (body) VALUES (?1)", [&"x".repeat(100)])
                .expect("insert");
        }
    }
    let r = rig(
        Mode::Normal,
        &[
            ("BACKUP_WEEKLY", "always"),
            ("BUGSINK_DB_PATH", &db.display().to_string()),
            ("BACKUP_KEEP_DAILY", "3"),
        ],
    );
    for d in 1..=5 {
        r.st.plant(
            &format!("bugsink/daily/bugsink-db-202610{d:02}T041700Z.sqlite3.gz"),
            b"old",
        );
    }
    r.st.plant(
        "bugsink/daily/bugsink-db-20261010T041700Z.surql.gz",
        b"wrong extension: never a candidate",
    );
    let (code, out, err) = cli(&r, &["run"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    let keys = r.st.keys();
    let daily: Vec<&String> = keys
        .iter()
        .filter(|k| k.starts_with("bugsink/daily/") && k.ends_with(".sqlite3.gz"))
        .collect();
    assert_eq!(daily.len(), 3, "{keys:?}");
    assert!(keys.iter().any(|k| k.starts_with("bugsink/weekly/bugsink-db-")));
    assert!(keys.contains(&"bugsink/daily/bugsink-db-20261010T041700Z.surql.gz".to_string()));
    assert!(
        out.contains("bugsink ok: ")
            && out.contains("integrity ok")
            && out.contains("bugsink retention: kept 4, deleted 3"),
        "{out}"
    );
    // the stored copy is a SQLite database with the rows
    let new = daily.iter().find(|k| !k.contains("202610")).expect("the new one");
    let copy = r.dir.path().join("copy.sqlite3");
    std::fs::write(&copy, gunzip(&r.st.bytes(new).expect("bytes"))).expect("write");
    let n: i64 = rusqlite::Connection::open(&copy)
        .expect("open")
        .query_row("SELECT count(*) FROM event", [], |x| x.get(0))
        .expect("count");
    assert_eq!(n, 200);
}

// ------------------------------------------------------------------------------------------ retention

#[test]
fn retention_deletes_exactly_the_old_backups_of_this_database_and_nothing_else() {
    let r = rig(Mode::Normal, &[]);
    for d in 1..=30 {
        r.st.plant(&old_daily(d), b"d");
    }
    for d in [4, 11, 18, 25] {
        r.st.plant(&old_weekly(d), b"w");
    }
    for m in 1..=8 {
        r.st.plant(
            &format!("surreal/weekly/punchable-main-2026{m:02}01T041700Z.surql.gz"),
            b"w",
        );
    }
    let foreign = [
        "audio/set-1/tape.m4a",
        "surreal-selftest/daily/punchable-main-20260101T000000Z.surql.gz",
        "surreal/daily/notes.txt",
        "surreal/daily/punchable-main-20260101T000000Z.surql.gz.part",
        "surreal/monthly/punchable-main-20260101T000000Z.surql.gz",
        "surreal/daily/other-main-20200101T000000Z.surql.gz",
        "bugsink/daily/bugsink-db-20200101T000000Z.sqlite3.gz",
    ];
    for f in foreign {
        r.st.plant(f, b"not ours");
    }
    let (code, out, err) = cli(&r, &["run"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    let mut deleted: Vec<String> =
        r.st.ops()
            .iter()
            .filter_map(|o| o.strip_prefix("DELETE ").map(str::to_string))
            .collect();
    deleted.sort();
    // 31 dailies (30 old + the new one) keep the newest 14 -> the 17 oldest go; 12 weeklies keep the newest 8 -> 4 go
    let mut want: Vec<String> = (1..=17).map(old_daily).collect();
    want.extend((1..=4).map(|m| format!("surreal/weekly/punchable-main-2026{m:02}01T041700Z.surql.gz")));
    want.sort();
    assert_eq!(deleted, want);
    assert!(
        out.contains("retention: kept 22 (14 daily / 8 weekly max), deleted 21"),
        "{out}"
    );
    let keys = r.st.keys();
    assert!(
        foreign.iter().all(|f| keys.contains(&(*f).to_string())),
        "something that is not ours was touched"
    );
}

#[test]
fn a_new_backup_that_is_not_among_the_newest_stops_the_prune() {
    // a clock that is behind: fourteen dailies from the future make today's backup the oldest
    let r = rig(Mode::Normal, &[]);
    for d in 1..=14 {
        r.st.plant(
            &format!("surreal/daily/punchable-main-209901{d:02}T041700Z.surql.gz"),
            b"d",
        );
    }
    r.st.plant(&old_daily(1), b"old");
    let (code, _, err) = cli(&r, &["run"]);
    assert_eq!(code, 1);
    assert!(
        err.starts_with("backup: FAIL prune refused: the new backup surreal/daily/punchable-main-"),
        "{err}"
    );
    assert_eq!(
        r.st.count("DELETE"),
        0,
        "nothing is deleted when the new backup would be the one pruned"
    );
}

#[test]
fn weekly_copy_follows_the_day_of_the_week_in_utc() {
    let sunday = Utc.with_ymd_and_hms(2026, 11, 15, 4, 17, 0).unwrap();
    let monday = Utc.with_ymd_and_hms(2026, 11, 16, 4, 17, 0).unwrap();
    for (now, weekly_made) in [(sunday, true), (monday, false)] {
        let r = rig(Mode::Normal, &[("BACKUP_WEEKLY", "auto")]);
        let roots = vec![r.st.ca_pem.clone().into_bytes()];
        let store = S3::from_vars(&r.vars, &roots).expect("s3");
        let export = ExportConfig::from_vars(&r.vars, &roots).expect("export");
        let mut out = Vec::new();
        let mut ctx = Ctx {
            vars: &r.vars,
            roots: &roots,
            out: &mut out,
            retry_unit: None,
        };
        run_database(&mut ctx, &store, &export, now, "surreal/", (14, 8)).expect("run");
        let keys = r.st.keys();
        let stamp = now.format("%Y%m%dT%H%M%SZ");
        assert!(keys.contains(&format!("surreal/daily/punchable-main-{stamp}.surql.gz")));
        assert_eq!(
            keys.contains(&format!("surreal/weekly/punchable-main-{stamp}.surql.gz")),
            weekly_made,
            "{now}"
        );
    }
}

// ------------------------------------------------------------------------------------------ red arms: verification

#[test]
fn an_upload_that_arrives_short_fails_the_night_and_deletes_nothing() {
    let r = rig(Mode::Normal, &[("BACKUP_WEEKLY", "always")]);
    for d in 1..=20 {
        r.st.plant(&old_daily(d), b"d");
    }
    r.st.store_half(true);
    let (code, out, err) = cli(&r, &["run"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        err.starts_with("backup: FAIL HEAD surreal/daily/punchable-main-") && err.contains(": size "),
        "{err}"
    );
    assert_eq!(
        r.st.count("DELETE"),
        0,
        "a failed verification must leave the older backups alone"
    );
    assert_eq!(r.st.count("COPY"), 0, "no weekly copy of an unverified upload");
    assert_eq!(r.st.count("LIST"), 0, "the prune is never reached");
    assert_eq!(err.lines().count(), 1, "one line");
    no_secret_leaked(&r, &err);
}

#[test]
fn a_failed_weekly_copy_fails_the_night() {
    let r = rig(Mode::Normal, &[("BACKUP_WEEKLY", "always")]);
    r.st.plant(&old_daily(1), b"d");
    r.st.fail("copy", 500, -1);
    let (code, _, err) = cli(&r, &["run"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("backup: FAIL COPY -> surreal/weekly/") || err.contains("HTTP 500"),
        "{err}"
    );
    assert_eq!(r.st.count("DELETE"), 0);
}

#[test]
fn an_export_that_is_not_an_export_never_reaches_the_bucket() {
    let r = rig(Mode::NotAnExport, &[("BACKUP_MIN_BYTES", "1024")]);
    r.st.plant(&old_daily(1), b"d");
    let (code, _, err) = cli(&r, &["run"]);
    assert_eq!(code, 1);
    assert!(err.contains("does not start like a SurrealDB export"), "{err}");
    assert_eq!(r.st.count("PUT"), 0);
    assert_eq!(r.st.count("DELETE"), 0);
}

#[test]
fn a_tiny_export_is_refused() {
    let r = rig(Mode::Normal, &[]);
    let one_table = SurrealStub::start(&["joke"], Mode::Normal, 0);
    let vars = Vars::from_pairs([
        ("SURREAL_URL", one_table.url.as_str()),
        ("SURREAL_USER", "api"),
        ("SURREAL_PASS", PASS),
    ]);
    let out = r.dir.path().join("tiny.surql.gz");
    let roots: Vec<Vec<u8>> = Vec::new();
    let (mut o, mut e) = (Vec::new(), Vec::new());
    let mut ctx = Ctx {
        vars: &vars,
        roots: &roots,
        out: &mut o,
        retry_unit: None,
    };
    let code = run(&["export".into(), out.display().to_string()], &mut ctx, &mut e);
    assert_eq!(code, 1);
    assert!(
        String::from_utf8_lossy(&e).contains("export is only"),
        "{}",
        String::from_utf8_lossy(&e)
    );
    assert!(!out.exists(), "a refused export is not left behind");
}

#[test]
fn a_dropped_export_stream_fails_before_any_upload() {
    let r = rig(Mode::Cut, &[("BACKUP_MIN_BYTES", "1")]);
    let (code, _, err) = cli(&r, &["run"]);
    assert_eq!(code, 1);
    assert!(err.starts_with("backup: FAIL export:"), "{err}");
    assert_eq!(r.st.ops().len(), 0, "nothing was sent to the bucket");
}

#[test]
fn surrealdb_refusing_the_login_is_one_redacted_line() {
    let r = rig(Mode::Status(401), &[]);
    let (code, out, err) = cli(&r, &["run"]);
    assert_eq!(code, 1);
    assert_eq!(err.lines().count(), 1);
    assert!(
        err.starts_with("backup: FAIL export: HTTP 401") || err.contains("HTTP 401"),
        "{err}"
    );
    no_secret_leaked(&r, &format!("{out}{err}"));
    assert_eq!(r.st.ops().len(), 0);
}

#[test]
fn a_rejected_put_and_a_flaky_bucket_fail_the_night_without_deleting() {
    let r = rig(Mode::Normal, &[]);
    r.st.plant(&old_daily(1), b"d");
    r.st.fail("put", 403, 1);
    let (code, _, err) = cli(&r, &["run"]);
    assert_eq!(code, 1);
    assert!(err.contains("PUT surreal/daily/") && err.contains("HTTP 403"), "{err}");
    assert_eq!(r.st.count("DELETE"), 0);
    // 5xx is retried three times, then fails
    let r = rig(Mode::Normal, &[]);
    r.st.fail("put", 503, -1);
    let (code, _, err) = cli(&r, &["run"]);
    assert_eq!(code, 1);
    assert!(err.contains("HTTP 503"), "{err}");
    assert_eq!(r.st.count("PUT"), 3);
    // ... and a 5xx that clears is survived
    let r = rig(Mode::Normal, &[]);
    r.st.fail("put", 503, 2);
    let (code, out, err) = cli(&r, &["run"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert_eq!(r.st.count("PUT"), 3);
}

#[test]
fn a_failed_listing_fails_the_night_after_the_backup_is_safe_and_deletes_nothing() {
    let r = rig(Mode::Normal, &[]);
    for d in 1..=20 {
        r.st.plant(&old_daily(d), b"d");
    }
    r.st.fail("list", 403, -1);
    let (code, _, err) = cli(&r, &["run"]);
    assert_eq!(code, 1);
    assert!(err.contains("LIST surreal/: HTTP 403"), "{err}");
    assert_eq!(r.st.count("DELETE"), 0);
    assert!(
        r.st.keys().iter().any(|k| !k.contains("202610")),
        "the new backup is in the bucket"
    );
}

#[test]
fn a_bugsink_failure_fails_the_job_after_the_database_backup_was_kept() {
    let r = rig(Mode::Normal, &[("BUGSINK_DB_PATH", "no-such-dir/db.sqlite3")]);
    let (code, out, err) = cli(&r, &["run"]);
    assert_eq!(code, 1);
    assert!(err.contains("BUGSINK_DB_PATH") && err.contains("no such file"), "{err}");
    assert!(
        out.contains("retention: kept 1"),
        "the database half finished first: {out}"
    );
    assert_eq!(r.st.keys().len(), 1);
}

#[test]
fn settings_are_checked_before_anything_is_sent() {
    for (k, v, want) in [
        ("BACKUP_PREFIX", "audio/", "BACKUP_PREFIX must look like"),
        ("BACKUP_KEEP_DAILY", "0", "BACKUP_KEEP_DAILY must be >= 1"),
        ("BACKUP_KEEP_WEEKLY", "x", "BACKUP_KEEP_WEEKLY must be an integer"),
        ("SURREAL_DB", "Main", "SURREAL_NS / SURREAL_DB must be [a-z0-9_]+"),
        (
            "BACKUP_WEEKLY",
            "sometimes",
            "BACKUP_WEEKLY must be auto, always or never",
        ),
        (
            "SURREAL_AUTH_LEVEL",
            "namespace",
            "SURREAL_AUTH_LEVEL must be database or root",
        ),
        (
            "R2_ENDPOINT",
            "http://insecure.example",
            "R2_ENDPOINT must be an https:// URL",
        ),
        ("R2_BUCKET", "", "R2_BUCKET is not set"),
    ] {
        let r = rig(Mode::Normal, &[(k, v)]);
        let (code, out, err) = cli(&r, &["run"]);
        assert_eq!(code, 1, "{k}");
        assert!(err.starts_with(&format!("backup: FAIL {want}")), "{k}: {err} {out}");
        assert_eq!(r.st.ops().len(), 0, "{k}");
        assert!(
            r.stub.seen().is_empty(),
            "{k}: SurrealDB was asked before the settings were checked"
        );
    }
}

// ------------------------------------------------------------------------------------------ the other commands

#[test]
fn the_exports_ask_for_every_table_but_the_uploads_then_their_definitions_alone() {
    let r = rig(Mode::Normal, &[("BACKUP_MIN_BYTES", "1")]);
    let out = r.dir.path().join("e.surql.gz");
    let (code, log, err) = cli(&r, &["export", &out.display().to_string()]);
    assert_eq!((code, err.as_str()), (0, ""), "{log}");
    let seen = r.stub.seen();
    assert!(seen[0].path.ends_with("/sql") && seen[0].body.contains("INFO FOR DB"));
    let cfg1: serde_json::Value = serde_json::from_str(&seen[1].body).expect("json");
    assert_eq!(
        cfg1,
        serde_json::json!({"tables": ["decision", "joke"]}),
        "the rows of everything else"
    );
    let cfg2: serde_json::Value = serde_json::from_str(&seen[2].body).expect("json");
    assert_eq!(
        cfg2,
        serde_json::json!({"tables": ["upload", "upload_part", "upload_stage"], "records": false, "users": false,
            "accesses": false, "params": false, "functions": false, "analyzers": false}),
        "the recordings' table definitions, no rows"
    );
    // database-level login: Basic auth plus the two Surreal-Auth headers
    for s in &seen {
        assert_eq!(s.headers.get("surreal-ns").map(String::as_str), Some("punchable"));
        assert_eq!(s.headers.get("surreal-db").map(String::as_str), Some("main"));
        assert_eq!(s.headers.get("surreal-auth-ns").map(String::as_str), Some("punchable"));
        assert_eq!(s.headers.get("surreal-auth-db").map(String::as_str), Some("main"));
        assert!(s.headers.get("authorization").is_some_and(|a| a.starts_with("Basic ")));
    }
    assert!(log.starts_with("export ok: ") && log.contains("tables)"), "{log}");
    no_secret_leaked(&r, &log);
}

#[test]
fn root_level_sends_no_auth_scope_headers_and_a_database_without_uploads_is_one_request() {
    let stub = SurrealStub::start(&["decision", "joke"], Mode::Normal, 100);
    let dir = tempfile::tempdir().expect("tempdir");
    let vars = Vars::from_pairs([
        ("SURREAL_URL", stub.url.as_str()),
        ("SURREAL_USER", "root"),
        ("SURREAL_PASS", PASS),
        ("SURREAL_AUTH_LEVEL", "root"),
    ]);
    let roots: Vec<Vec<u8>> = Vec::new();
    let (mut o, mut e) = (Vec::new(), Vec::new());
    let out = dir.path().join("e.surql.gz");
    let mut ctx = Ctx {
        vars: &vars,
        roots: &roots,
        out: &mut o,
        retry_unit: None,
    };
    assert_eq!(
        run(&["export".into(), out.display().to_string()], &mut ctx, &mut e),
        0,
        "{}",
        String::from_utf8_lossy(&e)
    );
    let seen = stub.seen();
    assert_eq!(
        seen.iter()
            .map(|s| s.path.rsplit('/').next().unwrap_or(""))
            .collect::<Vec<_>>(),
        ["sql", "export"]
    );
    assert!(seen.iter().all(|s| !s.headers.contains_key("surreal-auth-ns")));
}

#[test]
fn list_prints_size_and_key_oldest_first() {
    let r = rig(Mode::Normal, &[]);
    r.st.plant("surreal/daily/b", b"12345");
    r.st.plant("surreal/daily/a", b"1");
    r.st.plant("audio/other", b"zzz");
    let (code, out, err) = cli(&r, &["list"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(out, format!("{:>12}  surreal/daily/a\n{:>12}  surreal/daily/b\n", 1, 5));
}

#[test]
fn list_follows_the_pages() {
    let r = rig(Mode::Normal, &[]);
    r.st.set_page(3);
    for d in 1..=10 {
        r.st.plant(&old_daily(d), b"d");
    }
    let (code, out, _) = cli(&r, &["list"]);
    assert_eq!(code, 0);
    assert_eq!(out.lines().count(), 10);
    assert_eq!(r.st.count("LIST"), 4);
}

#[test]
fn fetch_downloads_checks_and_refuses_what_is_cut_missing_or_outside_the_prefix() {
    let r = rig(Mode::Normal, &[]);
    let (code, _, err) = cli(&r, &["run"]);
    assert_eq!((code, err.as_str()), (0, ""));
    let key = r.st.keys().into_iter().next().expect("key");
    let out = r.dir.path().join("restore.surql.gz");
    let (code, log, err) = cli(&r, &["fetch", &key, &out.display().to_string()]);
    assert_eq!((code, err.as_str()), (0, ""), "{log}");
    assert_eq!(std::fs::read(&out).expect("file"), r.st.bytes(&key).expect("bytes"));
    assert!(
        log.starts_with(&format!("fetched {key} -> ")) && log.contains(" sha256 "),
        "{log}"
    );
    assert!(!r.dir.path().join("restore.surql.gz.part").exists());

    let (code, _, err) = cli(&r, &["fetch", "audio/x", &out.display().to_string()]);
    assert_eq!((code, err.trim()), (1, "backup: FAIL key must be under surreal/"));
    let (code, _, err) = cli(&r, &["fetch", "surreal/daily/missing", &out.display().to_string()]);
    assert_eq!((code, err.trim()), (1, "backup: FAIL surreal/daily/missing: not found"));

    r.st.cut_downloads(true);
    let cut = r.dir.path().join("cut.surql.gz");
    let (code, _, err) = cli(&r, &["fetch", &key, &cut.display().to_string()]);
    assert_eq!(code, 1, "{err}");
    assert!(err.starts_with("backup: FAIL GET "), "{err}");
    assert!(
        !cut.exists() && !r.dir.path().join("cut.surql.gz.part").exists(),
        "no half file is left"
    );
}

#[test]
fn fetch_refuses_a_download_that_is_not_an_export() {
    let r = rig(Mode::Normal, &[]);
    r.st.plant("surreal/daily/junk", &[b'x'; 6000]);
    let out = r.dir.path().join("junk.gz");
    let (code, _, err) = cli(&r, &["fetch", "surreal/daily/junk", &out.display().to_string()]);
    assert_eq!(code, 1);
    assert!(err.contains("not a complete gzip"), "{err}");
    assert!(!out.exists());
}

#[test]
fn restore_check_accepts_a_good_file_and_refuses_a_cut_one() {
    let r = rig(Mode::Normal, &[]);
    let good = r.dir.path().join("good.surql.gz");
    let (code, _, err) = cli(&r, &["export", &good.display().to_string()]);
    assert_eq!((code, err.as_str()), (0, ""));
    let (code, out, err) = cli(&r, &["restore-check", &good.display().to_string()]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(
        out.starts_with("ok: ") && out.contains(" gz, ") && out.contains(" sql bytes, 5 tables, sha256 "),
        "{out}"
    );
    let raw = std::fs::read(&good).expect("read");
    let cut = r.dir.path().join("cut.surql.gz");
    std::fs::write(&cut, &raw[..raw.len() / 2]).expect("write");
    let (code, out, err) = cli(&r, &["restore-check", &cut.display().to_string()]);
    assert_eq!(code, 1, "{out}");
    assert!(err.contains("not a complete gzip"), "{err}");
}

#[test]
fn a_command_line_that_is_none_of_these_exits_2() {
    let r = rig(Mode::Normal, &[]);
    for args in [
        &[][..],
        &["run", "now"],
        &["list", "x"],
        &["fetch", "k"],
        &["restore-check"],
        &["export"],
        &["backup"],
    ] {
        let (code, out, err) = cli(&r, args);
        assert_eq!(code, 2, "{args:?}");
        assert!(
            out.is_empty() && err.starts_with("usage: pk-backup run | list | fetch"),
            "{err}"
        );
    }
    assert_eq!(r.st.ops().len(), 0);
}

// ------------------------------------------------------------------------------------------ the real binary

fn binary(r: &Rig, args: &[&str]) -> std::process::Output {
    let ca = r.dir.path().join("ca.pem");
    std::fs::write(&ca, &r.st.ca_pem).expect("ca");
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_pk-backup"));
    cmd.args(args).env_clear();
    // the process environment is the only source: what the CronJob would hand it
    for k in [
        "SURREAL_URL",
        "SURREAL_USER",
        "SURREAL_PASS",
        "R2_ENDPOINT",
        "R2_BUCKET",
        "R2_ACCESS_KEY_ID",
        "R2_SECRET_ACCESS_KEY",
        "BACKUP_TMP",
        "BACKUP_WEEKLY",
    ] {
        if let Some(v) = r.vars.get(k) {
            cmd.env(k, v);
        }
    }
    if let Some(sys) = std::env::var_os("SystemRoot") {
        cmd.env("SystemRoot", sys); // Windows needs it for sockets
    }
    cmd.env("SSL_CERT_FILE", &ca);
    cmd.output().expect("run pk-backup")
}

#[test]
fn the_binary_exits_0_on_a_night_1_on_a_failure_and_2_on_a_bad_command() {
    let r = rig(Mode::Normal, &[]);
    let o = binary(&r, &["run"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("retention: kept 1"));
    assert_eq!(String::from_utf8_lossy(&o.stderr), "");
    let n = r.st.keys().len();
    r.st.fail("put", 403, 1);
    let o = binary(&r, &["run"]);
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(err.lines().count(), 1, "{err}");
    assert!(err.starts_with("backup: FAIL PUT "), "{err}");
    no_secret_leaked(&r, &err);
    assert_eq!(r.st.keys().len(), n);
    let o = binary(&r, &["frobnicate"]);
    assert_eq!(o.status.code(), Some(2));
    // no SSL_CERT_FILE, no trust: a stand-in with a throwaway CA is refused, the job fails, one line
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_pk-backup"));
    cmd.arg("list").env_clear();
    for k in ["R2_ENDPOINT", "R2_BUCKET", "R2_ACCESS_KEY_ID", "R2_SECRET_ACCESS_KEY"] {
        cmd.env(k, r.vars.get(k).unwrap_or(""));
    }
    if let Some(sys) = std::env::var_os("SystemRoot") {
        cmd.env("SystemRoot", sys);
    }
    let o = cmd.output().expect("run");
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&o.stderr).lines().count(), 1);
}
