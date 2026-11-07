//! What the tapes cannot say: the guards the port added (a dry run cannot write), the command line's refusals, the
//! chunked download, and the sign-in files being left alone by `status`. No Python is involved; every server here
//! is either absent (a closed port: a test that must make no request fails loudly if it makes one) or a few lines of
//! std.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};

use pk_core::Env;
use pk_core::time::Fixed;
use pk_gsync::auth::Creds;
use pk_gsync::cli;
use pk_gsync::config::Config;
use pk_gsync::drive::GoogleDrive;
use pk_gsync::error::GsError;
use pk_gsync::punchable::{Hosted, WorkerAuth};
use pk_gsync::rules::Rules;
use pk_gsync::services::Services;
use serde_json::json;

/// Nothing listens here: a request to it fails with a connection error, so a guard that fires first is visible.
const CLOSED: &str = "http://127.0.0.1:9";

fn services(base: &str) -> Services {
    Services {
        clock: Box::new(Fixed::parse("2026-10-26T12:00:00+00:00").expect("time")),
        sleep: Box::new(|_| {}),
        opener: Box::new(|_| {}),
        spawn: Box::new(|_| Ok(())),
        free_mb: Box::new(|_| 50_000),
        restrict: Box::new(|_| true),
        google_token_uri: format!("{base}/google/token"),
        drive_root: base.to_string(),
    }
}

fn env_in(root: &Path, extra: &[(&str, &str)]) -> Env {
    let mut vars: Vec<(String, String)> = vec![
        (
            "PUNCHABLE_HOME".into(),
            root.join("home").to_string_lossy().into_owned(),
        ),
        ("PK_WORKSPACE".into(), root.join("ws").to_string_lossy().into_owned()),
        ("PK_AUTH_URL".into(), format!("{CLOSED}/kc")),
        ("PUNCHABLE_API".into(), CLOSED.into()),
    ];
    vars.extend(extra.iter().map(|(k, v)| ((*k).into(), (*v).into())));
    Env::from_map(vars).with_home(root.join("home"))
}

fn creds() -> Creds {
    let mut c = Creds::new(
        "fake-refresh",
        "fake-client",
        "fake-secret",
        &format!("{CLOSED}/google/token"),
        Vec::new(),
    );
    c.token = Some("fake-access".into());
    c
}

fn cli_run(root: &Path, extra: &[(&str, &str)], args: &[&str]) -> (i32, String, String) {
    let args: Vec<String> = args.iter().map(ToString::to_string).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = cli::run(&args, &env_in(root, extra), &services(CLOSED), &mut out, &mut err);
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

// ---- a dry run cannot write ------------------------------------------------------------------------------------------
#[test]
fn a_read_only_drive_refuses_every_write_before_any_request() {
    let svc = services(CLOSED);
    let rules = Rules::new(&json!({}), None);
    let mut drive = GoogleDrive::new(creds(), &svc, &rules, true);
    let blocked = |r: Result<serde_json::Value, GsError>| matches!(r, Err(GsError::WriteBlocked(_)));
    assert!(blocked(drive.create_doc("n", "t", Some("p"), &json!({}))));
    assert!(blocked(drive.update_doc("d", "t", None)));
    assert!(blocked(drive.create_folder("f", Some("p"), &json!({}))));
    assert!(blocked(drive.set_props("d", &json!({"k": null}), None)));
    assert!(blocked(drive.move_doc("d", "to", &["from".to_string()], &[])));
    assert!(matches!(drive.trash("d"), Err(GsError::WriteBlocked(_))));
    // not one of them reached the network (a request to the closed port would be a ConnectionError)
    assert_eq!(drive.stats.calls, 0);
    assert_eq!(drive.stats.writes, 0);
}

#[test]
fn a_writable_drive_still_refuses_a_doc_it_did_not_make_without_a_write_request() {
    // the write gate itself: a doc without our tag is looked at (one GET) and refused; with no server the look fails
    // as a connection error, which is also "no write happened"
    let svc = services(CLOSED);
    let rules = Rules::new(&json!({}), None);
    let mut drive = GoogleDrive::new(creds(), &svc, &rules, false);
    let r = drive.update_doc("not-ours", "text", None);
    assert!(r.is_err(), "{r:?}");
    assert_eq!(drive.stats.writes, 0);
}

#[test]
fn a_read_only_punchable_refuses_every_post() {
    let svc = services(CLOSED);
    let cfg = Config::from_env(&env_in(Path::new("."), &[]));
    let env = Env::from_map([
        ("PK_WORKER_SECRET", "fake-secret"),
        ("PK_API", CLOSED),
        ("PK_TOKEN_URL", "http://127.0.0.1:9/token"),
    ]);
    let mut pk = Hosted::new(
        &cfg,
        &svc,
        WorkerAuth::from_env(&env).expect("auth"),
        "import:gsync",
        true,
    );
    let r = pk.add_idea("text", None, &json!({}), "reason");
    assert!(matches!(r, Err(GsError::WriteBlocked(_))), "{r:?}");
    let r = pk.edit_joke("joke:x", &json!({"summary": "s"}), "reason");
    assert!(matches!(r, Err(GsError::WriteBlocked(_))), "{r:?}");
    let r = pk.push_state(&[], true);
    assert!(r.is_ok() || matches!(r, Err(GsError::WriteBlocked(_))), "{r:?}");
    assert_eq!(pk.calls, 0, "no request was counted, none was made");
}

// ---- the command line ------------------------------------------------------------------------------------------------
/// Serve one connection: for each request, `answer(target, headers, body)` gives the whole raw response.
fn serve_connection(stream: TcpStream, mut answer: impl FnMut(&str, &BTreeMap<String, String>, &[u8]) -> Vec<u8>) {
    let Ok(mut writer) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let target = line.split_whitespace().nth(1).unwrap_or_default().to_string();
        let mut headers = BTreeMap::new();
        loop {
            let mut raw = String::new();
            if reader.read_line(&mut raw).unwrap_or(0) == 0 {
                return;
            }
            let raw = raw.trim_end();
            if raw.is_empty() {
                break;
            }
            if let Some((name, value)) = raw.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
            }
        }
        let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut body = vec![0u8; len];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        if writer.write_all(&answer(&target, &headers, &body)).is_err() {
            return;
        }
    }
}

fn serve(handler: impl Fn(&str, &BTreeMap<String, String>, &[u8]) -> Vec<u8> + Send + Sync + 'static) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://127.0.0.1:{}", listener.local_addr().expect("addr").port());
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let handler = handler.clone();
            std::thread::spawn(move || serve_connection(stream, |t, h, b| handler(t, h, b)));
        }
    });
    base
}

/// A server that answers every request with a Google token (and counts them).
fn token_server() -> (String, Arc<Mutex<usize>>) {
    let hits: Arc<Mutex<usize>> = Arc::default();
    let counted = hits.clone();
    let base = serve(move |_, _, _| {
        *counted.lock().expect("lock") += 1;
        let payload = br#"{"access_token": "fake-access", "expires_in": 3599}"#;
        let mut raw = format!(
            "HTTP/1.1 200 OK
Content-Type: application/json
Content-Length: {}

",
            payload.len()
        )
        .into_bytes();
        raw.extend_from_slice(payload);
        raw
    });
    (base, hits)
}

fn signed_in_home(root: &Path) {
    let home = root.join("home");
    fs::create_dir_all(&home).expect("mkdir");
    fs::write(home.join("keycloak_gsync_token.json"), r#"{"refresh_token": "fake"}"#).expect("write");
    fs::write(
        home.join("google-oauth.env"),
        "GOOGLE_CLIENT_ID=a
GOOGLE_CLIENT_SECRET=b
",
    )
    .expect("write");
    fs::write(
        home.join("google_token.json"),
        r#"{"refresh_token": "fake-cached", "kc_checked_at": 1793015900}"#,
    )
    .expect("write");
}

#[test]
fn run_without_online_first_stops_with_the_reason_after_the_token_and_makes_no_other_request() {
    let dir = tempfile::tempdir().expect("tmp");
    signed_in_home(dir.path());
    let (base, hits) = token_server();
    let args = vec!["run".to_string()];
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = cli::run(
        &args,
        &env_in(dir.path(), &[("PK_WORKER_SECRET", "fake")]),
        &services(&base),
        &mut out,
        &mut err,
    );
    let err = String::from_utf8_lossy(&err).into_owned();
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("runs online-first only: set PK_ONLINE_FIRST=1"), "{err}");
    assert_eq!(*hits.lock().expect("lock"), 1, "only the Google token was asked for");
}

#[test]
fn run_without_a_worker_secret_says_where_it_looked() {
    let dir = tempfile::tempdir().expect("tmp");
    signed_in_home(dir.path());
    let (base, hits) = token_server();
    let args = vec!["run".to_string()];
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = cli::run(
        &args,
        &env_in(dir.path(), &[("PK_ONLINE_FIRST", "1")]),
        &services(&base),
        &mut out,
        &mut err,
    );
    let err = String::from_utf8_lossy(&err).into_owned();
    assert_eq!(code, 1, "{err}");
    assert!(err.starts_with("no PK_WORKER_SECRET in "), "{err}");
    assert_eq!(*hits.lock().expect("lock"), 1);
}

#[test]
fn a_cached_google_token_that_cannot_be_refreshed_is_a_no_auth_record_with_exit_2() {
    let dir = tempfile::tempdir().expect("tmp");
    signed_in_home(dir.path());
    // the cached token is tried first (as in the Python), the closed port does not answer: the run is `no_auth`
    let (code, out, _) = cli_run(dir.path(), &[], &["run"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("[no_auth]"), "{out}");
}

#[test]
fn run_without_a_sign_in_says_so_and_exits_2() {
    let dir = tempfile::tempdir().expect("tmp");
    fs::create_dir_all(dir.path().join("home")).expect("mkdir");
    let (code, out, _) = cli_run(dir.path(), &[], &["run", "--dry-run"]);
    assert_eq!(code, 2);
    assert!(out.contains("no token: run `python -m gsync auth`"), "{out}");
}

#[test]
fn the_desktop_sign_in_is_named_as_not_ported() {
    let dir = tempfile::tempdir().expect("tmp");
    let (code, _, err) = cli_run(dir.path(), &[], &["auth", "--desktop"]);
    assert_eq!(code, 1);
    assert!(err.contains("auth --desktop is not in pk-gsync"), "{err}");
}

#[test]
fn a_bad_command_line_is_clap_s_exit_2() {
    let dir = tempfile::tempdir().expect("tmp");
    let (code, _, err) = cli_run(dir.path(), &[], &["sing"]);
    assert_eq!(code, 2);
    assert!(err.contains("unrecognized subcommand"), "{err}");
    let (code, out, _) = cli_run(dir.path(), &[], &["--help"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("run") && out.contains("undo") && out.contains("status") && out.contains("auth"),
        "{out}"
    );
    let (code, out, _) = cli_run(dir.path(), &[], &["run", "--help"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("--dry-run") && out.contains("--watch") && out.contains("--interval"),
        "{out}"
    );
}

#[test]
fn status_reads_the_sign_in_files_by_reference_and_prints_no_token() {
    let dir = tempfile::tempdir().expect("tmp");
    let home = dir.path().join("home");
    fs::create_dir_all(&home).expect("mkdir");
    fs::write(
        home.join("keycloak_gsync_token.json"),
        r#"{"refresh_token": "fake-kc-marker-1"}"#,
    )
    .expect("write");
    fs::write(
        home.join("google-oauth.env"),
        "GOOGLE_CLIENT_ID=fake-id-marker-4\nGOOGLE_CLIENT_SECRET=fake-g-marker-2\n",
    )
    .expect("write");
    fs::write(
        home.join("google_token.json"),
        r#"{"refresh_token": "fake-cached-marker-3", "kc_checked_at": 1793015900}"#,
    )
    .expect("write");
    let before = fs::read_to_string(home.join("google_token.json")).expect("read");
    let (code, out, err) = cli_run(dir.path(), &[], &["status"]);
    for text in [&out, &err] {
        for secret in [
            "fake-kc-marker-1",
            "fake-g-marker-2",
            "fake-cached-marker-3",
            "fake-id-marker-4",
        ] {
            assert!(!text.contains(secret), "{secret} was printed: {text}");
        }
    }
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("Google token       FAILED"), "{out}");
    assert_eq!(
        fs::read_to_string(home.join("google_token.json")).expect("read"),
        before,
        "status left the token file alone"
    );
}

// ---- a download in chunks ---------------------------------------------------------------------------------------------
#[test]
fn a_download_asks_for_8_mib_ranges_until_the_file_is_whole() {
    const CHUNK: usize = 8 * 1024 * 1024;
    let total = CHUNK + 5;
    let data: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let (served, log) = (data.clone(), seen.clone());
    let base = serve(move |target, headers, _| {
        let range = headers.get("range").cloned().unwrap_or_default();
        log.lock().expect("lock").push(format!("{target} {range}"));
        let nums: Vec<usize> = range
            .trim_start_matches("bytes=")
            .split('-')
            .filter_map(|n| n.parse().ok())
            .collect();
        let (first, last) = (nums[0], nums[1].min(served.len() - 1));
        let part = &served[first..=last];
        let mut raw = format!(
            "HTTP/1.1 206 Partial Content
Content-Type: audio/x-m4a
Content-Range: bytes {first}-{last}/{}
Content-Length: {}

",
            served.len(),
            part.len()
        )
        .into_bytes();
        raw.extend_from_slice(part);
        raw
    });
    let svc = services(&base);
    let rules = Rules::new(&json!({}), None);
    let mut drive = GoogleDrive::new(creds(), &svc, &rules, true);
    let folder = tempfile::tempdir().expect("tmp");
    let dest = folder.path().join("memo.part");
    let got = drive.download("aud-1", &dest).expect("downloads");
    assert_eq!(got as usize, total);
    assert_eq!(fs::read(&dest).expect("read"), data);
    let asks = seen.lock().expect("lock").clone();
    assert_eq!(
        asks,
        vec![
            format!("/drive/v3/files/aud-1?alt=media bytes=0-{}", CHUNK - 1),
            format!("/drive/v3/files/aud-1?alt=media bytes={CHUNK}-{}", 2 * CHUNK - 1),
        ]
    );
    assert_eq!((drive.stats.downloads, drive.stats.calls), (1, 2));
}
