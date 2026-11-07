//! T10 group 5 against a scratch `SurrealDB` 3.3.4 (`surreal start memory`; skipped with a message when the binary
//! or the root password is missing, a failure with `PK_REQUIRE_EXTERNAL` set, as `writes.rs`).
//!
//! What it proves: the worker's claim lands in the database (`worker:seen`) and the Tonight screen's status reads
//! it back; a claim hands out the oldest queued job and marks it claimed, the next claim gets nothing, a result ends
//! it and a second result is the 409; eight claimers racing for three jobs get exactly three jobs, none twice; the
//! decisions and creative pages carry the lines with their hashes; the worker's profile view; the live hub sends one
//! `change` with the generation after a write, and `/api/live` over a real socket says `hello` then that change;
//! the sweeper embeds every pending row once, under a lease a second process cannot take.

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use futures::{SinkExt as _, StreamExt};
use pk_api::worker::Lease;
use pk_api::{ApiConfig, AppState, app};
use pk_core::Env;
use pk_store::{Config, Store, to_db, vars};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

#[path = "support/s3_standin.rs"]
mod s3_standin;
use s3_standin::{Options, Standin};

struct Scratch(Child);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn skip(why: &str) {
    assert!(
        std::env::var_os("PK_REQUIRE_EXTERNAL").is_none_or(|v| v.is_empty()),
        "PK_REQUIRE_EXTERNAL is set and the worker test cannot run: {why}"
    );
    eprintln!("SKIP worker: {why}");
}

async fn scratch() -> Option<(Scratch, Env, u16)> {
    scratch_on("memory").await
}

/// [`scratch`] on `engine` (`memory`, or `surrealkv://<dir>?versioned=true`: the engine production runs on).
async fn scratch_on(engine: &str) -> Option<(Scratch, Env, u16)> {
    let exe = std::env::var_os("PUNCH_SURREAL").map_or_else(
        || {
            std::env::home_dir()
                .unwrap_or_default()
                .join(".punchable")
                .join("bin")
                .join("surreal.exe")
        },
        PathBuf::from,
    );
    if !exe.is_file() {
        skip("no surreal binary (PUNCH_SURREAL or ~/.punchable/bin/surreal.exe)");
        return None;
    }
    let env = Env::from_process();
    let Some(pass) = env.opt("SURREAL_PASS").map(str::to_string) else {
        skip(&format!("no SURREAL_PASS in {}", env.place()));
        return None;
    };
    let user = env.opt("SURREAL_USER").unwrap_or("root").to_string();
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("a free port")
        .port();
    let child = Command::new(&exe)
        .args(["start", "--log", "warn", "--bind", &format!("127.0.0.1:{port}"), engine])
        .env("SURREAL_USER", &user)
        .env("SURREAL_PASS", &pass)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("start {}: {e}", exe.display()));
    let mut srv = Scratch(child);
    let deadline = Instant::now() + Duration::from_secs(60);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        if let Some(code) = srv.0.try_wait().expect("try_wait") {
            panic!("surreal exited {code} before listening");
        }
        assert!(Instant::now() < deadline, "surreal did not listen on {port} within 60s");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    Some((srv, env, port))
}

fn graph() -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../pk-kb/tests/goldens/fixture.json");
    let f: Value = serde_json::from_str(&std::fs::read_to_string(&p).expect("fixture")).expect("fixture is JSON");
    let mut g = f["graph"].clone();
    for n in g["nodes"]
        .as_array_mut()
        .expect("nodes")
        .iter_mut()
        .filter_map(Value::as_object_mut)
    {
        if n.get("type").and_then(Value::as_str) == Some("list")
            && let Some(t) = n.remove("title")
        {
            n.insert("name".into(), t);
        }
    }
    g
}

/// A loaded scratch database and the state over it, auth off, the word-hash embedder.
async fn setup(env: &Env, port: u16, db: &str) -> (Store, AppState) {
    setup_with(env, port, db, false).await
}

/// [`setup`] with the token check on (`auth`): a verifier whose key set cannot be fetched, so every token is refused.
async fn setup_with(env: &Env, port: u16, db: &str, auth: bool) -> (Store, AppState) {
    setup_full(env, port, db, auth, &[]).await
}

/// [`setup`] with recordings in R2 (docs/adr/0015) on, against the stand-in `s`.
async fn setup_uploads(env: &Env, port: u16, db: &str, s: &Standin) -> (Store, AppState) {
    setup_uploads_with(env, port, db, s, &[]).await
}

async fn setup_uploads_with(env: &Env, port: u16, db: &str, s: &Standin, more: &[(&str, &str)]) -> (Store, AppState) {
    let (writer, st) = setup_uploads_raw(env, port, db, s, more).await;
    assert!(st.check_recordings().await, "the stand-in answers the start-up check");
    (writer, st)
}

/// [`setup_uploads_with`] without the start-up check.
async fn setup_uploads_raw(env: &Env, port: u16, db: &str, s: &Standin, more: &[(&str, &str)]) -> (Store, AppState) {
    let mut extra = vec![
        ("PK_UPLOAD_R2", "1"),
        ("PK_REC_ENDPOINT", s.endpoint.as_str()),
        ("PK_REC_BUCKET", s.bucket.as_str()),
        ("PK_REC_ACCESS_KEY_ID", s.key_id.as_str()),
        ("PK_REC_SECRET_ACCESS_KEY", s.secret.as_str()),
    ];
    extra.extend_from_slice(more);
    setup_full(env, port, db, false, &extra).await
}

/// [`setup_with`] and more settings for the API (`PK_LIVE_PING_S`, ...).
async fn setup_full(env: &Env, port: u16, db: &str, auth: bool, extra: &[(&str, &str)]) -> (Store, AppState) {
    let cfg = Config::scratch(env, port, "punchable_test", db).expect("config");
    let writer = Store::connect(cfg.clone()).await.expect("connect");
    assert!(writer.apply_schema(false).await.expect("schema"));
    let g = graph();
    writer.load_graph(&g).await.expect("load");
    let text = serde_json::to_string(&g).expect("graph text");
    let key = pk_api::tape::srcfile_key("kb/v0/graph.json");
    writer
        .write(
            "CREATE type::record('srcfile', $k) CONTENT $c RETURN id;",
            vars([
                ("k", to_db(&json!(key))),
                ("c", pk_store::SrcFile::new("kb/v0/graph.json", &text).content()),
            ]),
            Some(1),
        )
        .await
        .expect("srcfile");
    writer
        .write(
            "UPDATE meta:state MERGE { gen: 0, loading: false, files_rev: 1, load_info: { bundle: $b, graph: 'kb/v0/graph.json' } } RETURN id;",
            vars([("b", to_db(&json!({"graph": "kb/v0/graph.json", "sha": "test"})))]),
            None,
        )
        .await
        .expect("bundle");
    let mut pairs = vec![
        ("SURREAL_PASS", "x"),
        ("PUBLIC_URL", "https://punchable.io/"),
        ("HOST", "127.0.0.1"),
        ("PK_OWNER_ID", "owner"),
        ("PK_EMBEDDER", "hash"),
        ("PK_VEC_SWEEP_S", "0"),
    ];
    pairs.extend(extra.iter().copied());
    if auth {
        pairs.push(("OIDC_ISSUER", "https://auth.example/realms/punchable"));
    } else {
        pairs.extend([("PK_AUTH", "0"), ("PK_ALLOW_NO_AUTH", "1")]);
    }
    let api_env = Env::from_map(pairs);
    let verifier = auth.then(|| {
        use pk_api::auth::verifier::{Clock, Fetch, Jwks, Verifier};
        let fetch: Fetch = std::sync::Arc::new(|| Box::pin(async { Err("no Keycloak in this test".to_string()) }));
        let clock: Clock = std::sync::Arc::new(|| 0.0);
        Verifier::new(
            "https://auth.example/realms/punchable",
            "pk-api",
            Jwks::with_fetch(fetch, clock.clone()),
            clock,
        )
    });
    let st = AppState::new(
        ApiConfig::from_env(&api_env).expect("api config"),
        verifier,
        pk_obs::ApiMetrics::new("0.1.0", "").expect("metrics"),
        reqwest::Client::new(),
    );
    st.set_store(Some(Store::connect(cfg).await.expect("connect")));
    st.data().await.expect("first load");
    (writer, st)
}

/// Another process over the same database: its own state, its own `creative_lock`, its own connection.
async fn another(env: &Env, port: u16, db: &str) -> AppState {
    let api_env = Env::from_map(vec![
        ("SURREAL_PASS", "x"),
        ("PUBLIC_URL", "https://punchable.io/"),
        ("HOST", "127.0.0.1"),
        ("PK_OWNER_ID", "owner"),
        ("PK_EMBEDDER", "hash"),
        ("PK_VEC_SWEEP_S", "0"),
        ("PK_AUTH", "0"),
        ("PK_ALLOW_NO_AUTH", "1"),
    ]);
    let st = AppState::new(
        ApiConfig::from_env(&api_env).expect("api config"),
        None,
        pk_obs::ApiMetrics::new("0.1.0", "").expect("metrics"),
        reqwest::Client::new(),
    );
    st.set_store(Some(
        Store::connect(Config::scratch(env, port, "punchable_test", db).expect("config"))
            .await
            .expect("connect"),
    ));
    st.data().await.expect("first load");
    st
}

async fn call(st: &AppState, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path);
    let body = match body {
        Some(b) => {
            let bytes = serde_json::to_vec(&b).expect("json");
            req = req
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CONTENT_LENGTH, bytes.len());
            Body::from(bytes)
        }
        None => Body::empty(),
    };
    let r = app(st.clone())
        .oneshot(req.body(body).unwrap())
        .await
        .expect("infallible");
    let (parts, body) = r.into_parts();
    let b = axum::body::to_bytes(body, 1 << 20).await.expect("body");
    let v: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
    (parts.status, v)
}

fn transcript(text: &str) -> Value {
    json!({"segments": [{"text": text, "start": 0, "end": 5}], "title": "A phone set"})
}

/// The first bit of the fixture's view.
async fn a_joke(st: &AppState) -> String {
    st.data()
        .await
        .expect("data")
        .view(false)
        .await
        .expect("view")
        .jokes
        .first()
        .cloned()
        .expect("a joke")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_worker_routes_hand_out_and_end_jobs_and_keep_the_claim_in_the_database() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (writer, st) = setup(&env, port, "t_worker").await;

    // no claim yet: the status says so; the first claim lands in the database for both servers to read
    assert!(pk_api::worker::load_seen(&writer).await.unwrap().is_none());
    let (s, _) = call(
        &st,
        "POST",
        "/api/worker/claim",
        Some(json!({"kinds": ["set_text"], "worker": "server-1", "hosted": true, "model": "m"})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "nothing queued");
    let seen = pk_api::worker::load_seen(&writer)
        .await
        .unwrap()
        .expect("the claim was kept");
    assert_eq!(
        (seen.name.as_str(), seen.hosted, seen.model.as_deref()),
        ("server-1", true, Some("m"))
    );
    let status = pk_api::worker::status(Some(&seen), false, pk_api::worker::unix_now());
    assert_eq!(
        (status["online"].as_bool(), status["name"].as_str()),
        (Some(true), Some("server-1"))
    );
    // a paused worker: the heartbeat lands, nothing is handed out even with a job queued
    let (s, r) = call(&st, "POST", "/api/sets/transcript", Some(transcript("The first set."))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{r}");
    let job_a = r["job"]["id"].as_str().unwrap().to_string();
    let (s, _) = call(
        &st,
        "POST",
        "/api/worker/claim",
        Some(json!({"kinds": ["set_text"], "worker": "server-1", "hosted": true, "paused": "over budget"})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let seen = pk_api::worker::load_seen(&writer).await.unwrap().unwrap();
    assert_eq!(seen.paused.as_deref(), Some("over budget"));
    let (_, r) = call(&st, "GET", &format!("/api/jobs/{job_a}"), None).await;
    assert_eq!(r["status"], "queued", "a paused claim takes nothing");

    // the claim: the queued job, marked claimed by the worker; the next claim gets nothing
    let (s, r) = call(
        &st,
        "POST",
        "/api/worker/claim",
        Some(json!({"kinds": ["set_text", "riff"], "worker": "server-1", "hosted": true})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["job"]["id"], job_a);
    assert_eq!(
        (
            r["job"]["status"].as_str(),
            r["job"]["worker"].as_str(),
            r["job"]["attempts"].as_i64()
        ),
        (Some("claimed"), Some("server-1"), Some(1))
    );
    assert!(r["job"]["input"].is_object(), "the whole job, input included");
    let (s, _) = call(
        &st,
        "POST",
        "/api/worker/claim",
        Some(json!({"kinds": ["set_text"], "worker": "server-1", "hosted": true})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    // a kind the worker does not take is not handed out
    let (s, r2) = call(&st, "POST", "/api/sets/transcript", Some(transcript("The second set."))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{r2}");
    let job_b = r2["job"]["id"].as_str().unwrap().to_string();
    let (s, _) = call(
        &st,
        "POST",
        "/api/worker/claim",
        Some(json!({"kinds": ["riff"], "worker": "server-1", "hosted": true})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // the result: done, with the result kept; again is the 409; the refusals
    let jq = |j: &str| j.replace(':', "%3A");
    let (s, r) = call(
        &st,
        "POST",
        &format!("/api/worker/jobs/{}/result", jq(&job_a)),
        Some(json!({"ok": true, "result": {"set": "set:x"}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        (r["ok"].as_bool(), r["job"]["status"].as_str(), r["job"]["id"].as_str()),
        (Some(true), Some("done"), Some(job_a.as_str()))
    );
    assert!(r["job"].get("result").is_none(), "the brief, not the result");
    let (_, r) = call(&st, "GET", &format!("/api/jobs/{job_a}"), None).await;
    assert_eq!(r["result"], json!({"set": "set:x"}));
    let (s, r) = call(
        &st,
        "POST",
        &format!("/api/worker/jobs/{}/result", jq(&job_a)),
        Some(json!({"ok": true, "result": {}})),
    )
    .await;
    assert_eq!((s, r["error"].as_str()), (StatusCode::CONFLICT, Some("job is done")));
    let (s, r) = call(
        &st,
        "POST",
        "/api/worker/jobs/job:nope/result",
        Some(json!({"ok": true, "result": {}})),
    )
    .await;
    assert_eq!((s, r["error"].as_str()), (StatusCode::NOT_FOUND, Some("no such job")));
    let (s, r) = call(
        &st,
        "POST",
        &format!("/api/worker/jobs/{}/result", jq(&job_b)),
        Some(json!({"ok": true, "result": {}})),
    )
    .await;
    assert_eq!((s, r["error"].as_str()), (StatusCode::CONFLICT, Some("job is queued")));
    // claimed, then failed with retry: back to the queue with an attempt spent
    let (s, _) = call(
        &st,
        "POST",
        "/api/worker/claim",
        Some(json!({"kinds": ["set_text"], "worker": "server-1", "hosted": true})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, r) = call(
        &st,
        "POST",
        &format!("/api/worker/jobs/{}/result", jq(&job_b)),
        Some(json!({"ok": true, "result": [1]})),
    )
    .await;
    assert_eq!(
        (s, r["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("result must be an object"))
    );
    let (s, r) = call(
        &st,
        "POST",
        &format!("/api/worker/jobs/{}/result", jq(&job_b)),
        Some(json!({"ok": false, "error": "the model timed out", "retry": true})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        (
            r["job"]["status"].as_str(),
            r["job"]["error"].as_str(),
            r["job"]["attempts"].as_i64()
        ),
        (Some("queued"), Some("the model timed out"), Some(1))
    );
    let (s, r) = call(&st, "POST", "/api/worker/claim", Some(json!({"kinds": ["zap"]}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(
        r["error"]
            .as_str()
            .unwrap()
            .starts_with("kinds must be a subset of ('bridge'")
    );
    let (s, _) = call(&st, "POST", "/api/worker/claim", Some(json!({"hosted": true}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // the decisions page: every line with the hash of its compact sorted dump; the creative pages
    let joke = a_joke(&st).await;
    let (s, r) = call(
        &st,
        "POST",
        "/api/decision",
        Some(json!({"target": "node", "id": joke, "action": "edit", "after": {"title": "Alpha, renamed"}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let (s, r) = call(&st, "GET", "/api/worker/decisions?after=0", None).await;
    assert_eq!(s, StatusCode::OK);
    let total = r["total"].as_u64().unwrap();
    assert!(total >= 1 && r["lines"].as_array().unwrap().len() as u64 == total && r["after"] == 0);
    let last = r["lines"].as_array().unwrap().last().unwrap();
    let want = pk_store::line_hash(&pk_core::py::dumps_compact_sorted(&last["decision"]));
    assert_eq!(
        (last["h"].as_str(), last["index"].as_u64()),
        (Some(want.as_str()), Some(total - 1))
    );
    assert_eq!(last["decision"]["after"]["title"], "Alpha, renamed");
    let (_, r) = call(&st, "GET", &format!("/api/worker/decisions?after={total}"), None).await;
    assert_eq!(
        (r["lines"].as_array().map(Vec::len), r["after"].as_u64()),
        (Some(0), Some(total))
    );
    let (_, r) = call(&st, "GET", "/api/worker/decisions?after=x", None).await;
    assert_eq!(r["after"], 0);
    let (s, r) = call(&st, "GET", "/api/worker/creative?store=jobs", None).await;
    assert_eq!(
        (s, r["error"].as_str()),
        (
            StatusCode::BAD_REQUEST,
            Some("store must be storyboards | workingsets | venues | merges | suggestions")
        )
    );
    let (s, r) = call(&st, "GET", "/api/worker/creative?store=storyboards&snapshot=1", None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        (r["store"].as_str(), r["total"].as_u64()),
        (Some("storyboards"), Some(0))
    );
    assert!(r["lines"].as_array().unwrap().is_empty() && r["items"].is_object());
    let (s, r) = call(
        &st,
        "POST",
        "/api/boards",
        Some(json!({"title": "Seven", "joke_ids": [joke]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    let (_, r) = call(&st, "GET", "/api/worker/creative?store=storyboards&snapshot=1", None).await;
    assert_eq!(r["total"], 1);
    let line: Value = serde_json::from_str(r["lines"][0].as_str().unwrap()).unwrap();
    assert_eq!(line["op"], "create");
    assert_eq!(r["items"].as_object().unwrap().len(), 1);
    let (_, r) = call(&st, "GET", "/api/worker/creative?store=venues", None).await;
    assert_eq!((r["total"].as_u64(), r.get("items").is_none()), (Some(0), true));

    // the worker's profile: no previous, laugh lines (none without punchlines); the loader's PUT names its actor
    let (s, r) = call(&st, "GET", "/api/worker/profile/voice", None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(r.get("previous").is_none() && r["laugh_lines"] == json!([]) && r["text"].is_null());
    let (s, r) = call(
        &st,
        "PUT",
        "/api/worker/profile/voice",
        Some(json!({"text": "I talk fast."})),
    )
    .await;
    assert_eq!(
        (s, r["error"].as_str()),
        (
            StatusCode::BAD_REQUEST,
            Some("a worker write must name its actor: import:<source> or ai:<what>")
        )
    );
    let (s, r) = call(
        &st,
        "PUT",
        "/api/worker/profile/voice",
        Some(json!({"text": "I talk fast.", "actor": "import:profile"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        (r["text"].as_str(), r["by"].as_str(), r["by_kind"].as_str()),
        (Some("I talk fast.\n"), Some("import:profile"), Some("import"))
    );
    let (_, r) = call(&st, "GET", "/api/worker/profile/voice", None).await;
    assert_eq!(r["chars"], 13);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn eight_claimers_racing_for_three_jobs_get_exactly_three() {
    claim_race("memory", "t_worker_race").await;
}

/// The same race on `SurrealKV` with versioning, the engine production runs on: its write-conflict behaviour is the
/// one that matters for two processes claiming.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn eight_claimers_racing_for_three_jobs_get_exactly_three_on_surrealkv() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("db").to_string_lossy().replace('\\', "/");
    claim_race(&format!("surrealkv://{path}?versioned=true"), "t_worker_race_kv").await;
}

async fn claim_race(engine: &str, db: &str) {
    let Some((_srv, env, port)) = scratch_on(engine).await else {
        return;
    };
    let (_writer, st) = setup(&env, port, db).await;
    // two processes (two states, two connections): one `creative_lock` serialises the claimers of one process, so
    // only the database's unique (store, seq) index arbitrates between these two
    let states = [st.clone(), another(&env, port, db).await];
    let mut ids = Vec::new();
    for i in 0..3 {
        let (s, r) = call(
            &st,
            "POST",
            "/api/sets/transcript",
            Some(transcript(&format!("Set number {i}."))),
        )
        .await;
        assert_eq!(s, StatusCode::ACCEPTED, "{r}");
        ids.push(r["job"]["id"].as_str().unwrap().to_string());
    }
    let mut tasks = Vec::new();
    for w in 0..8 {
        let st = states[w % 2].clone();
        tasks.push(tokio::spawn(async move {
            call(
                &st,
                "POST",
                "/api/worker/claim",
                Some(json!({"kinds": ["set_text"], "worker": format!("w{w}"), "hosted": true})),
            )
            .await
        }));
    }
    let mut got = Vec::new();
    let mut nothing = 0;
    for t in tasks {
        let (s, r) = t.await.unwrap();
        match s {
            StatusCode::OK => got.push(r["job"]["id"].as_str().unwrap().to_string()),
            StatusCode::NO_CONTENT => nothing += 1,
            other => panic!("a claim answered {other}: {r}"),
        }
    }
    got.sort();
    ids.sort();
    assert_eq!(
        (got, nothing),
        (ids, 5),
        "every job handed out once, the rest got nothing"
    );
}

/// The pointer row (docs/adr/0015): made last, naming the object; a taken name gets the next suffix; the rows list
/// newest first; the keys the bucket sweep must not delete; a delete gives the row back so its object can go too.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pointer_row_names_the_object_and_a_taken_name_gets_the_next_suffix() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (writer, _st) = setup(&env, port, "t_upload").await;
    let meta = json!({"kind": "set", "title": "Late show", "marks_ms": [1, 2]});
    let name = "2026-11-06_punchable-set_101010_take.m4a";
    assert_eq!(
        writer
            .upload_commit(name, "rec/aaa", &meta, 2_500_000, 1)
            .await
            .unwrap(),
        name
    );
    let second = writer.upload_commit(name, "rec/bbb", &meta, 5, 1).await.unwrap();
    assert_eq!(
        second, "2026-11-06_punchable-set_101010_take-2.m4a",
        "the name is reserved by the unique record id"
    );
    let rows = writer.upload_rows().await.unwrap();
    assert_eq!(
        rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        [second.as_str(), name],
        "newest first"
    );
    assert_eq!(writer.upload_count().await.unwrap(), 2);
    let row = writer.upload_get(name).await.unwrap().unwrap();
    assert_eq!(
        (row.bytes, row.parts, row.key.as_str(), row.meta["title"].clone()),
        (2_500_000, 1, "rec/aaa", json!("Late show"))
    );
    let mut keys = writer.upload_keys().await.unwrap();
    keys.sort();
    assert_eq!(keys, ["rec/aaa", "rec/bbb"], "what the bucket sweep must not delete");
    assert_eq!(writer.upload_names_before(1.0).await.unwrap(), Vec::<String>::new());
    assert_eq!(writer.upload_names_before(f64::MAX).await.unwrap().len(), 2);
    let gone = writer.upload_delete(name).await.unwrap().expect("the row comes back");
    assert_eq!(gone.key, "rec/aaa", "so its object can be deleted");
    assert!(writer.upload_get(name).await.unwrap().is_none());
    assert!(
        writer.upload_get(&second).await.unwrap().is_some(),
        "another upload stays"
    );
    assert!(
        writer.upload_delete("never-there.m4a").await.unwrap().is_none(),
        "deleting what is not there is not an error"
    );
    // nothing writes the database's old homes any more
    let left = writer
        .last(
            "RETURN count(SELECT id FROM upload_part) + count(SELECT id FROM upload_stage);",
            vars([]),
        )
        .await
        .unwrap();
    assert_eq!(pk_store::from_db(left), json!(0));
}

/// The next text message within `secs`, parsed.
async fn next_json<S>(ws: &mut S, secs: u64) -> Value
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(next) = tokio::time::timeout(left, ws.next()).await else {
            panic!("no message within {secs}s");
        };
        match next {
            Some(Ok(Message::Text(t))) => return serde_json::from_str(t.as_str()).expect("JSON"),
            Some(Ok(_)) => {}
            Some(Err(e)) => panic!("the socket failed: {e}"),
            None => panic!("the socket closed"),
        }
    }
}

/// Every `change` event within `secs` (one write's notifications may land in two 150 ms windows), as the tables
/// they named (`meta` excluded by the hub) and the newest `rev` among them.
async fn changes_within<S>(ws: &mut S, secs: f64) -> (Vec<Value>, Vec<String>, Option<String>)
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let deadline = Instant::now() + Duration::from_secs_f64(secs);
    let mut events = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        // the window closed: what was read is the answer
        let Ok(next) = tokio::time::timeout(left, ws.next()).await else {
            break;
        };
        match next {
            Some(Ok(Message::Text(t))) => {
                let v: Value = serde_json::from_str(t.as_str()).expect("JSON");
                if v["type"] == "change" {
                    events.push(v);
                }
            }
            Some(Ok(_)) => {}
            Some(Err(e)) => panic!("the socket failed: {e}"),
            None => panic!("the socket closed"),
        }
    }
    summarise(events)
}

fn summarise(events: Vec<Value>) -> (Vec<Value>, Vec<String>, Option<String>) {
    let mut tables = Vec::new();
    let mut rev = None;
    for ev in &events {
        for c in ev["changes"].as_array().into_iter().flatten() {
            if let Some(t) = c["table"].as_str() {
                tables.push(t.to_string());
            }
        }
        if let Some(r) = ev["rev"].as_str() {
            rev = Some(r.to_string());
        }
    }
    (events, tables, rev)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_live_hub_and_the_socket_report_a_write_with_its_generation() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (_writer, st) = setup(&env, port, "t_worker_live").await;
    tokio::spawn(st.live().clone().run(st.clone()));
    tokio::spawn(st.live().clone().flush_loop());
    let deadline = Instant::now() + Duration::from_secs(15);
    while !st.live().connected() {
        assert!(Instant::now() < deadline, "the hub did not subscribe");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // the hub alone: a decision is reported with the generation after it (the notifications of one write may
    // land in two windows: every event within a second and a half is read)
    let mut rx = st.live().subscribe();
    let joke = a_joke(&st).await;
    let (s, r) = call(
        &st,
        "POST",
        "/api/decision",
        Some(json!({"target": "node", "id": joke, "action": "edit", "after": {"stage": "testing"}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(1500);
    while let Ok(Ok(t)) = tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), rx.recv()).await {
        events.push(serde_json::from_str::<Value>(&t).unwrap());
    }
    let generation = st.data().await.unwrap().state.generation.unwrap();
    let (events, tables, rev) = summarise(events);
    assert!(!events.is_empty(), "an event in time");
    assert!(
        events
            .iter()
            .all(|e| e["type"] == "change" && e["at"].is_number() && e["count"].as_u64().is_some())
    );
    assert_eq!(
        rev.as_deref(),
        Some(format!("db:{generation}").as_str()),
        "the generation the write made: {events:?}"
    );
    assert!(
        tables.iter().any(|t| t == "decision") && !tables.iter().any(|t| t == "meta"),
        "{events:?}"
    );

    // over a real socket: hello, then a change after a creative write (no auth here: no first message needed)
    let listener = pk_api::bind("127.0.0.1:0").await.unwrap();
    let addr = {
        use axum::serve::Listener as _;
        listener.local_addr().unwrap()
    };
    tokio::spawn(axum::serve(listener, app(st.clone())).into_future());
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/live"))
        .await
        .expect("upgrade");
    let hello = next_json(&mut ws, 10).await;
    assert_eq!(
        (hello["type"].as_str(), hello["live"].as_bool()),
        (Some("hello"), Some(true))
    );
    assert_eq!(hello["rev"], format!("db:{generation}"));
    assert_eq!(st.metrics().live_clients.get(), 1);
    let (s, r) = call(
        &st,
        "POST",
        "/api/boards",
        Some(json!({"title": "Tonight", "joke_ids": [joke]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    let (events, tables, rev) = changes_within(&mut ws, 1.5).await;
    assert!(!events.is_empty(), "the socket reports the write");
    assert!(rev.is_none(), "a creative write moves no generation: {events:?}");
    assert_eq!(tables, ["creative_event"]);
    let c = &events[0]["changes"][0];
    assert_eq!(
        (c["store"].as_str(), c["op"].as_str()),
        (Some("storyboards"), Some("create"))
    );
    ws.send(Message::Close(None)).await.unwrap();
    drop(ws);
    let deadline = Instant::now() + Duration::from_secs(5);
    while st.metrics().live_clients.get() != 0 {
        assert!(Instant::now() < deadline, "the client gauge did not drop");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // a plain GET is the Python's 400
    let (s, r) = call(&st, "GET", "/api/live", None).await;
    assert_eq!(
        (s, r["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("this is a WebSocket endpoint"))
    );
    let r = app(st.clone())
        .oneshot(
            Request::builder()
                .uri("/api/live")
                .header(header::ORIGIN, "https://evil.example")
                .header(header::UPGRADE, "websocket")
                .header(header::CONNECTION, "upgrade")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                .header("sec-websocket-version", "13")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN, "an origin not allowed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_token_on_the_socket_is_one_error_then_close_4401() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (_writer, st) = setup_with(&env, port, "t_worker_auth", true).await;
    tokio::spawn(st.live().clone().run(st.clone()));
    tokio::spawn(st.live().clone().flush_loop());
    let listener = pk_api::bind("127.0.0.1:0").await.unwrap();
    let addr = {
        use axum::serve::Listener as _;
        listener.local_addr().unwrap()
    };
    tokio::spawn(axum::serve(listener, app(st.clone())).into_future());
    // a token the key set cannot verify, sent as the first message: the error, then the close code
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/live"))
        .await
        .expect("upgrade");
    ws.send(Message::Text(
        json!({"type": "auth", "token": "nope"}).to_string().into(),
    ))
    .await
    .unwrap();
    let err = next_json(&mut ws, 10).await;
    assert_eq!(
        (err["type"].as_str(), err["error"].as_str()),
        (Some("error"), Some("unauthorized")),
        "{err}"
    );
    assert!(
        err["detail"]
            .as_str()
            .is_some_and(|d| !d.is_empty() && d.chars().count() <= 80)
    );
    let close = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .expect("a close in time");
    match close {
        Some(Ok(Message::Close(Some(f)))) => assert_eq!(u16::from(f.code), 4401),
        other => panic!("expected close 4401, got {other:?}"),
    }
    assert_eq!(st.metrics().live_clients.get(), 0, "a refused socket is never a client");
    // not an auth message at all: the empty token is refused the same way
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/live"))
        .await
        .expect("upgrade");
    ws.send(Message::Text(json!({"type": "hello"}).to_string().into()))
        .await
        .unwrap();
    let err = next_json(&mut ws, 10).await;
    assert_eq!(err["error"], "unauthorized");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sequence_of_events_lands_whole_or_not_at_all() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (writer, _st) = setup(&env, port, "t_worker_tx").await;
    let ev = |seq: i64, op: &str| {
        let line =
            json!({"ts": "2026-11-06T00:00:00+00:00", "op": op, "id": "sb:x", "data": {}, "by": "owner"}).to_string();
        pk_store::CreativeEvent::new("storyboards", seq, &line).unwrap()
    };
    writer
        .create_creative_events(&[ev(0, "a"), ev(1, "b"), ev(2, "c")])
        .await
        .unwrap();
    assert_eq!(writer.creative_lines("storyboards").await.unwrap().len(), 3);
    // a batch whose second row takes a place already taken: the first row of it is not left behind
    let err = writer
        .create_creative_events(&[ev(3, "d"), ev(1, "dup"), ev(4, "e")])
        .await
        .unwrap_err();
    assert!(pk_api::writes::is_place_taken(&err), "{err}");
    let lines = writer.creative_lines("storyboards").await.unwrap();
    assert_eq!(lines.len(), 3, "nothing of the failed batch landed: {lines:?}");
    writer.create_creative_events(&[ev(3, "d"), ev(4, "e")]).await.unwrap();
    assert_eq!(writer.creative_lines("storyboards").await.unwrap().len(), 5);
    writer.create_creative_events(&[]).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn two_line_writes_from_two_processes_lose_no_line_and_write_none_twice() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (writer, st) = setup(&env, port, "t_worker_lines").await;
    let other = another(&env, port, "t_worker_lines").await;
    let joke = a_joke(&st).await;
    let before = writer.decision_count().await.unwrap();
    // eight writers over two processes, each writing two lines per round: the second line of a pair that finds its
    // place taken by the other process is sent again (`Kb::append_lines`), the first starts the pair over
    let (rounds, writers) = (40_usize, 16_usize);
    let mut tasks = Vec::new();
    for w in 0..writers {
        let state = if w % 2 == 0 { st.clone() } else { other.clone() };
        let joke = joke.clone();
        tasks.push(tokio::spawn(async move {
            let kb = pk_api::writes::Kb::new(state.clone(), false);
            let mut landed = 0;
            for r in 0..rounds {
                let _lock = state.kb_lock().lock().await;
                let joke = joke.clone();
                let out = kb
                    .append_lines("owner", move |_, _| {
                        let line = |stage: &str| pk_api::writes::Raw {
                            target: "node",
                            id: joke.clone(),
                            action: "edit",
                            before: Value::Null,
                            after: json!({ "stage": stage }),
                            reason: format!("round {r}"),
                        };
                        Ok(vec![line("testing"), line("idea")])
                    })
                    .await
                    .unwrap_or_else(|e| panic!("writer {w} round {r}: {} {}", e.status, e.msg));
                landed += out.len();
            }
            landed
        }));
    }
    let mut total = 0;
    for t in tasks {
        total += t.await.unwrap();
    }
    assert_eq!(total, 2 * rounds * writers);
    let after = writer.decision_count().await.unwrap();
    assert_eq!(
        after - before,
        i64::try_from(total).unwrap(),
        "every line landed once: none lost, none written twice"
    );
}

/// A listener over `st` for the socket tests, with the hub running.
async fn serve_live(st: &AppState) -> std::net::SocketAddr {
    tokio::spawn(st.live().clone().run(st.clone()));
    tokio::spawn(st.live().clone().flush_loop());
    let listener = pk_api::bind("127.0.0.1:0").await.unwrap();
    let addr = {
        use axum::serve::Listener as _;
        listener.local_addr().unwrap()
    };
    tokio::spawn(axum::serve(listener, app(st.clone())).into_future());
    addr
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_draining_pod_closes_its_live_sockets_with_1012() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (_writer, st) = setup(&env, port, "t_worker_drain").await;
    let addr = serve_live(&st).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/live"))
        .await
        .expect("upgrade");
    assert_eq!(next_json(&mut ws, 10).await["type"], "hello");
    st.start_draining();
    let close = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .expect("a close in time");
    match close {
        Some(Ok(Message::Close(Some(f)))) => assert_eq!(u16::from(f.code), 1012, "service restart"),
        other => panic!("expected close 1012, got {other:?}"),
    }
    // a socket that arrives after the drain began is told at once, too
    let (mut late, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/live"))
        .await
        .expect("upgrade");
    let mut got_close = false;
    while let Ok(Some(Ok(m))) = tokio::time::timeout(Duration::from_secs(10), late.next()).await {
        if let Message::Close(Some(f)) = m {
            assert_eq!(u16::from(f.code), 1012);
            got_close = true;
            break;
        }
    }
    assert!(got_close, "the late socket was closed with 1012");
}

/// A socket with events flowing and no frame from the client but its pongs must stay open past the idle limit: the
/// ping runs on its own deadline, so an event never postpones it (the app sends only its auth frame and pong replies).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_busy_live_socket_stays_open_past_the_idle_limit() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (_writer, st) = setup_full(
        &env,
        port,
        "t_worker_busy",
        false,
        &[("PK_LIVE_PING_S", "0.15"), ("PK_LIVE_IDLE_S", "0.4")],
    )
    .await;
    let addr = serve_live(&st).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/live"))
        .await
        .expect("upgrade");
    assert_eq!(next_json(&mut ws, 10).await["type"], "hello");
    let joke = a_joke(&st).await;
    let started = Instant::now();
    let mut events = 0;
    let mut i = 0;
    // events every ~100 ms (faster than the ping), for 1.6 s: four times the idle limit
    while started.elapsed() < Duration::from_millis(1600) {
        i += 1;
        let (s, r) = call(
            &st,
            "POST",
            "/api/decision",
            Some(json!({"target": "node", "id": joke, "action": "edit", "after": {"stage": if i % 2 == 0 { "testing" } else { "idea" }}})),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{r}");
        // reading answers the server's pings (tungstenite queues the pong), which is all a healthy app does
        while let Ok(Some(m)) = tokio::time::timeout(Duration::from_millis(100), ws.next()).await {
            match m.expect("the socket stays up") {
                Message::Text(t) if t.contains("\"change\"") => events += 1,
                Message::Close(f) => panic!("a busy socket was closed: {f:?} after {:?}", started.elapsed()),
                _ => {}
            }
        }
    }
    assert!(events > 0, "events flowed");
    assert_eq!(
        st.metrics().live_clients.get(),
        1,
        "still a client after {:?}",
        started.elapsed()
    );
}

/// A client that stops answering (no frame at all) is closed after the idle limit, not when TCP gives up.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_silent_live_socket_is_closed_after_the_idle_limit() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (_writer, st) = setup_full(
        &env,
        port,
        "t_worker_silent",
        false,
        &[("PK_LIVE_PING_S", "0.15"), ("PK_LIVE_IDLE_S", "0.4")],
    )
    .await;
    let addr = serve_live(&st).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/live"))
        .await
        .expect("upgrade");
    assert_eq!(next_json(&mut ws, 10).await["type"], "hello");
    assert_eq!(st.metrics().live_clients.get(), 1);
    // the client reads nothing and so answers nothing for longer than the idle limit
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        st.metrics().live_clients.get(),
        0,
        "the half-open socket was closed by the server"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_first_frame_over_32_kib_is_refused_before_it_is_read_whole() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (_writer, st) = setup_with(&env, port, "t_worker_bigfirst", true).await;
    let addr = serve_live(&st).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/live"))
        .await
        .expect("upgrade");
    let big = json!({"type": "auth", "token": "x".repeat(1 << 20)}).to_string();
    let _ = ws.send(Message::Text(big.into())).await;
    // the connection ends (a close for the size, or the stream dropped); it is never a verified token or a hello
    let mut ended = false;
    while let Ok(m) = tokio::time::timeout(Duration::from_secs(10), ws.next()).await {
        match m {
            None | Some(Err(_) | Ok(Message::Close(_))) => {
                ended = true;
                break;
            }
            Some(Ok(Message::Text(t))) => assert!(!t.contains("hello"), "{t}"),
            Some(Ok(_)) => {}
        }
    }
    assert!(ended, "the socket was ended");
    assert_eq!(st.metrics().live_clients.get(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn the_lease_is_granted_to_exactly_one_of_eight_askers_at_once() {
    lease_race("memory", "t_worker_lease").await;
}

/// The same race on `SurrealKV` with versioning (`SURREAL_PATH` of `infra/k8s/20-surrealdb.yaml`): the engine whose
/// write-conflict behaviour production has.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn the_lease_is_granted_to_exactly_one_of_eight_askers_at_once_on_surrealkv() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("db").to_string_lossy().replace('\\', "/");
    lease_race(&format!("surrealkv://{path}?versioned=true"), "t_worker_lease_kv").await;
}

async fn lease_race(engine: &str, db: &str) {
    let Some((_srv, env, port)) = scratch_on(engine).await else {
        return;
    };
    let (writer, _st) = setup(&env, port, db).await;
    let cfg = Config::scratch(&env, port, "punchable_test", db).expect("config");
    // eight processes are eight connections (a test: the service itself never opens a second one)
    let mut stores = Vec::new();
    for _ in 0..8 {
        stores.push(std::sync::Arc::new(Store::connect(cfg.clone()).await.expect("connect")));
    }
    let mut grants = Vec::new();
    for round in 0..30 {
        // even rounds: a row that exists and is free; odd rounds: no row at all, so all eight run the first-ever CREATE
        // (the loser's "already exists" is a refusal, not an error)
        let reset = if round % 2 == 0 {
            "UPDATE lease:embedder SET owner = NONE, until = NONE; IF !record::exists(lease:embedder) { CREATE lease:embedder CONTENT { kind: 'embedder' } };"
        } else {
            "DELETE lease:embedder;"
        };
        writer.query(reset, vars([])).await.unwrap();
        let mut tasks = Vec::new();
        for (i, s) in stores.iter().enumerate() {
            let s = std::sync::Arc::clone(s);
            tasks.push(tokio::spawn(async move {
                s.lease_acquire("embedder", &format!("p{i}"), 60).await.unwrap()
            }));
        }
        let mut won = 0;
        for t in tasks {
            won += usize::from(t.await.unwrap());
        }
        grants.push(won);
        assert_eq!(won, 1, "round {round}: exactly one of eight holds the lease, got {won}");
    }
    assert_eq!(grants.iter().sum::<usize>(), 30);
    // the holder renews, the others are refused, and a released lease is free for the next asker
    let (owner, _) = writer.lease_state("embedder").await.unwrap();
    let owner = owner.expect("an owner");
    assert!(writer.lease_acquire("embedder", &owner, 60).await.unwrap());
    assert!(!writer.lease_acquire("embedder", "someone else", 60).await.unwrap());
    writer.lease_release("embedder", &owner).await.unwrap();
    assert!(writer.lease_acquire("embedder", "someone else", 60).await.unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_sweeper_embeds_every_pending_row_once_under_one_lease() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (writer, st) = setup(&env, port, "t_worker_sweep").await;
    let count = |sql: &'static str| {
        let w = writer.clone();
        async move { pk_store::value::as_i64(&w.last(sql, vars([])).await.unwrap()).unwrap() }
    };
    let units_pending =
        count("RETURN count(SELECT id FROM unit WHERE embedding = NONE AND text != NONE AND string::len(text) > 0);")
            .await;
    assert!(units_pending > 0, "the fixture's units carry no vector");
    let mine = Lease::embedder("pk-api:test:1", Duration::from_secs(30));
    let done = pk_api::sweep::once(&st, &mine)
        .await
        .unwrap()
        .expect("the lease was free");
    assert_eq!(done["unit"], usize::try_from(units_pending).unwrap(), "{done:?}");
    assert_eq!(
        count("RETURN count(SELECT id FROM unit WHERE embedding = NONE AND text != NONE AND string::len(text) > 0);")
            .await,
        0
    );
    let embedded_jokes = count("RETURN count(SELECT id FROM joke WHERE embedded_sha != NONE);").await;
    assert_eq!(
        usize::try_from(embedded_jokes).unwrap(),
        done["joke"],
        "typed ideas carry their sha"
    );
    let again = pk_api::sweep::once(&st, &mine).await.unwrap().unwrap();
    assert_eq!(again.values().sum::<usize>(), 0, "nothing pending twice: {again:?}");
    // another process asks while this one holds the lease: it embeds nothing; a lapsed lease is free again
    let other = Lease::embedder("python:test:2", Duration::from_secs(30));
    assert!(pk_api::sweep::once(&st, &other).await.unwrap().is_none());
    let short = Lease {
        ttl_s: 1,
        ..Lease::embedder("pk-api:test:3", Duration::from_secs(30))
    };
    assert!(!short.acquire(&writer).await.unwrap(), "held by the first");
    writer
        .query("UPDATE lease:embedder SET until = time::now() - 1s;", vars([]))
        .await
        .unwrap();
    assert!(short.acquire(&writer).await.unwrap(), "lapsed: taken");
    assert!(!mine.acquire(&writer).await.unwrap(), "the first lost it");
    // a process with no model to run gives the lease up and reports it, so the other process embeds
    writer
        .query(
            "UPDATE joke SET embedded_sha = NONE WHERE embedded_sha != NONE;",
            vars([]),
        )
        .await
        .unwrap();
    let no_model = Env::from_map(vec![
        ("SURREAL_PASS", "x"),
        ("PUBLIC_URL", "https://punchable.io/"),
        ("HOST", "127.0.0.1"),
        ("PK_AUTH", "0"),
        ("PK_ALLOW_NO_AUTH", "1"),
        ("PK_OWNER_ID", "owner"),
        ("PK_VEC_SWEEP_S", "0"),
    ]);
    let bare = AppState::new(
        ApiConfig::from_env(&no_model).expect("api config"),
        None,
        pk_obs::ApiMetrics::new("0.1.0", "").expect("metrics"),
        reqwest::Client::new(),
    );
    bare.set_store(Some(
        Store::connect(cfg_of(&env, port, "t_worker_sweep"))
            .await
            .expect("connect"),
    ));
    bare.data().await.expect("first load");
    writer
        .query("UPDATE lease:embedder SET owner = NONE, until = NONE;", vars([]))
        .await
        .unwrap();
    assert_eq!(
        pk_api::sweep::once(&bare, &short).await,
        Err(pk_api::sweep::SweepError::NoModel)
    );
    assert!(
        mine.acquire(&writer).await.unwrap(),
        "released: the process with a model takes it at once"
    );
}

fn cfg_of(env: &Env, port: u16, db: &str) -> Config {
    Config::scratch(env, port, "punchable_test", db).expect("config")
}

/// `POST /api/upload` with `n` bytes of body and `claimed` as its Content-Length.
async fn upload_with(st: &AppState, n: usize, claimed: usize) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/api/upload")
        .header(header::CONTENT_TYPE, "audio/mp4")
        .header(header::CONTENT_LENGTH, claimed)
        .header("X-Kind", "set")
        .header("X-Filename", format!("late-show-{claimed}.m4a"))
        .body(Body::from(vec![7_u8; n]))
        .unwrap();
    let r = app(st.clone()).oneshot(req).await.expect("infallible");
    let (parts, body) = r.into_parts();
    let b = axum::body::to_bytes(body, 1 << 20).await.expect("body");
    (parts.status, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

/// `GET /api/worker/jobs/<id>/file`: status and bytes.
async fn fetch_file(st: &AppState, job: &str) -> (StatusCode, Vec<u8>, header::HeaderMap) {
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/worker/jobs/{job}/file"))
        .body(Body::empty())
        .unwrap();
    let r = app(st.clone()).oneshot(req).await.expect("infallible");
    let (parts, body) = r.into_parts();
    let b = axum::body::to_bytes(body, 64 << 20).await;
    (parts.status, b.map(|b| b.to_vec()).unwrap_or_default(), parts.headers)
}

/// A pattern of `n` bytes that a part boundary cannot hide a shift in.
fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| ((i * 31 + 7) % 251) as u8).collect()
}

/// docs/adr/0015: the streamed upload. The bytes are one object in the bucket and the database holds a pointer row; a
/// recording of a part and a bit is a multipart upload; the answer is the Python's; the transcriber's fetch streams the
/// bytes back whole with the Python's headers; the same recording twice is one job and one object; a body cut short
/// leaves no object, no row and no open multipart upload; and the caps and refusals are the Python's file mode's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::many_single_char_names)] // a status and a body per call, as every test here
async fn an_upload_streams_into_the_bucket_and_the_fetch_streams_it_back() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let standin = Standin::start(Options::default()).await;
    let (writer, st) = setup_uploads(&env, port, "t_upload_stream", &standin).await;
    // the Python's refusals, from Content-Length before a byte is read
    let (s, r) = upload_with(&st, 10, 1024 * 1024 * 1024 + 1).await;
    assert_eq!(
        (s, r["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("upload missing or over 1 GB"))
    );
    let (s, r) = upload_with(&st, 0, 0).await;
    assert_eq!(
        (s, r["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("upload missing or over 1 GB"))
    );
    assert_eq!(
        standin.ops().len(),
        1,
        "only the start-up listing: a refused upload sends nothing"
    );
    // 17 MiB and a bit: three parts
    let data = pattern(2 * pk_api::recordings::PART + 4321);
    let (s, r) = upload_with_bytes(&st, &data, data.len()).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        (r["status"].as_str(), r["bytes"].as_u64(), r["kind"].as_str()),
        (Some("queued"), Some(data.len() as u64), Some("set"))
    );
    assert_eq!(r["job"]["kind"], "process_set");
    assert_eq!(r["waiting_for"], "transcriber");
    let (file, job) = (
        r["file"].as_str().unwrap().to_string(),
        r["job"]["id"].as_str().unwrap().to_string(),
    );
    assert_eq!(
        (standin.count("start"), standin.count("part"), standin.completed()),
        (1, 3, 1)
    );
    let row = writer.upload_get(&file).await.unwrap().expect("the pointer row");
    assert_eq!((row.bytes, row.parts), (i64::try_from(data.len()).unwrap(), 3));
    assert!(row.key.starts_with("rec/"), "the pointer names the object");
    assert_eq!(standin.objects(), vec![(row.key.clone(), data.len())]);
    assert_eq!(standin.bytes(&row.key).unwrap(), data, "the object is the recording");
    assert_eq!(standin.refused(), 0, "every signature the stand-in checked was good");
    let tables = writer
        .last(
            "RETURN count(SELECT id FROM upload_part) + count(SELECT id FROM upload_stage);",
            vars([]),
        )
        .await
        .unwrap();
    assert_eq!(
        pk_store::from_db(tables),
        json!(0),
        "no byte and no staging row in the database"
    );
    // the inbox lists it, the transcriber gets it whole
    let (s, inbox) = call(&st, "GET", "/api/inbox", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(inbox["items"][0]["file"], file.as_str());
    assert_eq!(inbox["items"][0]["bytes"], data.len());
    let (s, got, h) = fetch_file(&st, &job).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(got, data, "the fetch streams the bytes back whole");
    assert_eq!(h[header::CONTENT_TYPE], "application/octet-stream");
    assert_eq!(h[header::CONTENT_LENGTH], data.len().to_string().as_str());
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
    // a job that is no upload, and one that is gone from the bucket
    let (s, _, _) = fetch_file(&st, "job:nope").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    standin.fail("get", 404, 1);
    let (s, _, _) = fetch_file(&st, &job).await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "the object is gone: the fetch is a 404, not a hang"
    );
    // the same recording twice within two minutes: one job, one object
    let same = pattern(5000);
    let (_, a) = upload_with_bytes(&st, &same, same.len()).await;
    let objects = standin.objects().len();
    let (s, b) = upload_with_bytes(&st, &same, same.len()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!((b["duplicate"].as_bool(), &b["file"]), (Some(true), &a["file"]));
    assert_eq!(b["job"]["id"], a["job"]["id"]);
    assert_eq!(standin.objects().len(), objects, "a duplicate leaves no object behind");
    assert_eq!(writer.upload_count().await.unwrap(), 2);
    // a body that ends before its Content-Length: no row, no object, no open multipart upload
    let before = (standin.objects().len(), standin.aborted());
    let (s, r) = upload_with(&st, 3 * 1024 * 1024, 20 * 1024 * 1024).await;
    assert_eq!(
        (s, r["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("upload was cut short"))
    );
    assert_eq!(writer.upload_count().await.unwrap(), 2);
    assert_eq!(standin.objects().len(), before.0, "nothing stored");
    assert_eq!(standin.open_uploads(), 0);
    // ... and one that fails after a part has gone is aborted
    let mut pieces: Vec<Result<axum::body::Bytes, std::io::Error>> = pattern(pk_api::recordings::PART + 100)
        .chunks(1 << 20)
        .map(|c| Ok(axum::body::Bytes::copy_from_slice(c)))
        .collect();
    pieces.push(Err(std::io::Error::other("the phone lost its signal")));
    let req = Request::builder()
        .method("POST")
        .uri("/api/upload")
        .header(header::CONTENT_TYPE, "audio/mp4")
        .header(header::CONTENT_LENGTH, 40 * 1024 * 1024)
        .header("X-Kind", "set")
        .body(Body::from_stream(futures::stream::iter(pieces)))
        .unwrap();
    let r = app(st.clone()).oneshot(req).await.expect("infallible");
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        (standin.open_uploads(), standin.aborted()),
        (0, before.1 + 1),
        "the multipart upload was aborted"
    );
    assert_eq!(standin.objects().len(), before.0);
    assert_eq!(writer.upload_count().await.unwrap(), 2, "nothing was listed");
}

/// `upload_with` for given bytes.
async fn upload_with_bytes(st: &AppState, data: &[u8], claimed: usize) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/api/upload")
        .header(header::CONTENT_TYPE, "audio/mp4")
        .header(header::CONTENT_LENGTH, claimed)
        .header("X-Kind", "set")
        .header("X-Filename", format!("late-show-{claimed}.m4a"))
        .header("X-Duration-S", "61.5")
        .body(Body::from(data.to_vec()))
        .unwrap();
    let r = app(st.clone()).oneshot(req).await.expect("infallible");
    let (parts, body) = r.into_parts();
    let b = axum::body::to_bytes(body, 1 << 20).await.expect("body");
    (parts.status, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

/// A bucket that fails answers 503 with a reason and keeps nothing; the phone still has the recording. A start-up check
/// that fails keeps readiness down; once the bucket answers it comes up.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_bucket_is_a_503_with_a_reason_and_keeps_readiness_down() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let standin = Standin::start(Options::default()).await;
    let (writer, st) = setup_uploads(&env, port, "t_upload_down", &standin).await;
    standin.fail("put", 500, -1);
    let (s, r) = upload_with(&st, 2000, 2000).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{r}");
    let why = r["error"].as_str().unwrap();
    assert!(why.contains("the phone keeps it") && why.contains("500"), "{why}");
    for private in [
        standin.endpoint.as_str(),
        standin.bucket.as_str(),
        standin.key_id.as_str(),
        "X-Amz",
    ] {
        assert!(!why.contains(private), "the reason leaks {private:?}: {why}");
    }
    assert_eq!(writer.upload_count().await.unwrap(), 0);
    assert_eq!(standin.objects().len(), 0);
    standin.heal();
    let (s, _) = upload_with(&st, 2001, 2001).await;
    assert_eq!(
        s,
        StatusCode::OK,
        "the bucket is back: the phone's next try goes through"
    );
    // readiness: a wrong token keeps the pod unready
    let health = |st: &AppState| {
        let st = st.clone();
        async move {
            let req = Request::builder()
                .uri("/api/health?ready=1")
                .body(Body::empty())
                .unwrap();
            let r = app(st).oneshot(req).await.expect("infallible");
            r.status()
        }
    };
    assert_eq!(health(&st).await, StatusCode::OK);
    let (_w, bad) = setup_uploads_raw(
        &env,
        port,
        "t_upload_down_bad",
        &standin,
        &[("PK_REC_SECRET_ACCESS_KEY", "wrong")],
    )
    .await;
    assert!(!bad.recordings_ready(), "not ready before the check has passed");
    assert!(!bad.check_recordings().await, "a wrong token: the start-up check fails");
    assert_eq!(
        health(&bad).await,
        StatusCode::SERVICE_UNAVAILABLE,
        "so the rollout stops"
    );
}

/// docs/adr/0014's lead defaults, kept by 0015: the recording goes when the job is dropped and when it fails for good
/// (not when it goes back to the queue); and after a week the expiry pass deletes the recording nobody claimed (pointer
/// row and object), failing its job with the reason, leaves one that was claimed and takes a stray. The bucket sweep
/// then deletes an object no row names, but only once it is an hour old.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::many_single_char_names)] // a status and a body per call, as every test here
async fn uploads_let_go_when_their_job_ends_and_after_a_week() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let standin = Standin::start(Options::default()).await;
    let (writer, st) = setup_uploads(&env, port, "t_upload_end", &standin).await;
    let (s, r) = upload_with(&st, 1000, 1000).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(writer.upload_count().await.unwrap(), 1);
    assert_eq!(standin.objects().len(), 1);

    // his drop of the queued job takes its recording with it: the row and the object
    let (file_a, job_a) = (
        r["file"].as_str().unwrap().to_string(),
        r["job"]["id"].as_str().unwrap().to_string(),
    );
    let (s, d) = call(&st, "POST", &format!("/api/jobs/{job_a}/drop"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert!(
        writer.upload_get(&file_a).await.unwrap().is_none(),
        "the dropped job's recording is gone"
    );
    assert!(standin.objects().is_empty(), "and so is its object");

    // a job that fails for good takes it; one sent back to the queue keeps it
    let claim = json!({"kinds": ["process_set"], "worker": "w1"});
    let (_, r) = upload_with(&st, 2000, 2000).await;
    let file_b = r["file"].as_str().unwrap().to_string();
    let (s, c) = call(&st, "POST", "/api/worker/claim", Some(claim.clone())).await;
    assert_eq!(s, StatusCode::OK, "{c}");
    let job_b = c["job"]["id"].as_str().unwrap().to_string();
    let (s, _) = call(
        &st,
        "POST",
        &format!("/api/worker/jobs/{job_b}/result"),
        Some(json!({"ok": false, "error": "no speech", "retry": true})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        writer.upload_get(&file_b).await.unwrap().is_some() && standin.objects().len() == 1,
        "a retry keeps the recording"
    );
    let (_, c) = call(&st, "POST", "/api/worker/claim", Some(claim.clone())).await;
    assert_eq!(c["job"]["id"], job_b.as_str(), "the retried job is handed out again");
    let (s, r) = call(
        &st,
        "POST",
        &format!("/api/worker/jobs/{job_b}/result"),
        Some(json!({"ok": false, "error": "no speech", "retry": false})),
    )
    .await;
    assert_eq!(
        (s, r["job"]["status"].as_str()),
        (StatusCode::OK, Some("failed")),
        "{r}"
    );
    assert!(
        writer.upload_get(&file_b).await.unwrap().is_none() && standin.objects().is_empty(),
        "a job that failed for good lets go"
    );

    // a job that is done lets go as well
    let (_, r) = upload_with(&st, 2500, 2500).await;
    let file_d = r["file"].as_str().unwrap().to_string();
    let (_, c) = call(&st, "POST", "/api/worker/claim", Some(claim.clone())).await;
    let job_d = c["job"]["id"].as_str().unwrap().to_string();
    let (s, r) = call(
        &st,
        "POST",
        &format!("/api/worker/jobs/{job_d}/result"),
        Some(json!({"ok": true, "result": {"sets": 1}})),
    )
    .await;
    assert_eq!((s, r["job"]["status"].as_str()), (StatusCode::OK, Some("done")), "{r}");
    assert!(writer.upload_get(&file_d).await.unwrap().is_none() && standin.objects().is_empty());

    // a delete the bucket refuses does not undo the job's end: the row is gone, the object is left for the sweep
    let (_, r) = upload_with(&st, 2600, 2600).await;
    let job_e = r["job"]["id"].as_str().unwrap().to_string();
    standin.fail("delete", 500, -1);
    let (s, d) = call(&st, "POST", &format!("/api/jobs/{job_e}/drop"), Some(json!({}))).await;
    assert_eq!(
        s,
        StatusCode::OK,
        "the drop is answered though the object could not be deleted: {d} (upload said {r})"
    );
    assert_eq!((writer.upload_count().await.unwrap(), standin.objects().len()), (0, 1));
    standin.heal();
    let left = standin.objects()[0].0.clone();
    assert_eq!(
        pk_api::expiry::sweep_bucket(&st, pk_api::worker::unix_now())
            .await
            .unwrap(),
        0,
        "an object an hour from old is not an orphan yet"
    );
    assert_eq!(
        pk_api::expiry::sweep_bucket(&st, pk_api::worker::unix_now() + 2.0 * 3600.0)
            .await
            .unwrap(),
        1,
        "the sweep deletes the object no row names"
    );
    assert!(standin.objects().is_empty(), "{left} went");

    // after a week: one waiting, one claimed, one with no job at all, and one too young
    let (_, r) = upload_with(&st, 3000, 3000).await;
    let (_, q) = upload_with(&st, 3001, 3001).await;
    let names = [
        r["file"].as_str().unwrap().to_string(),
        q["file"].as_str().unwrap().to_string(),
    ];
    let (_, c) = call(&st, "POST", "/api/worker/claim", Some(claim)).await;
    let claimed_job = c["job"]["id"].as_str().unwrap().to_string();
    let (claimed_file, waiting_file) = if c["job"]["input"]["file"] == names[0].as_str() {
        (names[0].clone(), names[1].clone())
    } else {
        (names[1].clone(), names[0].clone())
    };
    standin.plant("rec/stray-object", b"12345");
    writer
        .upload_commit("stray.m4a", "rec/stray-object", &json!({"kind": "set"}), 5, 1)
        .await
        .unwrap();
    assert_eq!(
        pk_api::expiry::expire_once(&st, pk_api::worker::unix_now())
            .await
            .unwrap(),
        0,
        "nothing is old yet"
    );
    let week_on = pk_api::worker::unix_now() + 8.0 * 86_400.0;
    assert_eq!(
        pk_api::expiry::expire_once(&st, week_on).await.unwrap(),
        2,
        "the waiting one and the stray"
    );
    assert!(writer.upload_get(&waiting_file).await.unwrap().is_none());
    assert!(writer.upload_get("stray.m4a").await.unwrap().is_none());
    assert!(
        writer.upload_get(&claimed_file).await.unwrap().is_some(),
        "a claimed recording is left alone"
    );
    assert_eq!(
        standin.objects().len(),
        1,
        "only the claimed one's object is left: the others' objects went with their rows"
    );
    assert!(standin.bytes("rec/stray-object").is_none());
    let jobs = pk_api::writes::Creative::new(st.clone()).items("jobs").await.unwrap();
    // (an earlier job of this test may carry the same file name, made in the same second: look at all of them)
    assert!(
        jobs.values().any(|j| j["input"]["file"] == waiting_file.as_str()
            && j["status"] == "failed"
            && j["error"].as_str().is_some_and(|e| e.contains("waited a week"))),
        "the waiting job ended with the reason"
    );
    assert_eq!(jobs[&claimed_job]["status"], "claimed");
    assert_eq!(
        pk_api::expiry::expire_once(&st, week_on).await.unwrap(),
        0,
        "a second pass finds nothing new"
    );
    // the sweep never deletes an object a row names, however old
    standin.age(&standin.objects()[0].0, 30.0 * 86_400.0);
    assert_eq!(pk_api::expiry::sweep_bucket(&st, week_on).await.unwrap(), 0);
    assert_eq!(standin.objects().len(), 1);
}

async fn set_extra_sha(writer: &Store, sha: &str) {
    writer
        .write(
            "UPDATE meta:state SET extra_sha = $x RETURN id;",
            vars([("x", to_db(&json!(sha)))]),
            None,
        )
        .await
        .expect("sha");
}

/// While the database holds another build's `extra_sha`, an upload answers 503 (never a row the schema does not
/// describe); once it is this build's again the next check lets uploads through.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upload_waits_while_the_stored_schema_is_another_builds() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let standin = Standin::start(Options::default()).await;
    let (writer, st) = setup_uploads_with(&env, port, "t_upload_schema", &standin, &[("PK_SCHEMA_OWNER", "0")]).await;
    st.check_schema().await;
    assert!(st.schema_extra_current(), "setup applied this build's schema");
    let (s, _) = upload_with(&st, 500, 500).await;
    assert_eq!(s, StatusCode::OK);
    set_extra_sha(&writer, "another-build").await;
    st.check_schema().await;
    assert!(!st.schema_extra_current());
    let objects = standin.objects().len();
    let (s, r) = upload_with(&st, 501, 501).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{r}");
    assert!(
        r["error"].as_str().unwrap().contains("not the one this build expects"),
        "{r}"
    );
    assert_eq!(writer.upload_count().await.unwrap(), 1, "nothing was written");
    assert_eq!(standin.objects().len(), objects, "and no byte was sent to the bucket");
    set_extra_sha(&writer, &pk_store::schema_shas().1).await;
    st.check_schema().await;
    let (s, r) = upload_with(&st, 502, 502).await;
    assert_eq!(s, StatusCode::OK, "{r}");
}

/// T10 round 5, group F: the schema is pk-api's to keep. A database that holds another sha of `schema_extra.surql`
/// (and has lost a table of it) gets the file applied again by the next check, the sha stored, and uploads work.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_schema_owner_applies_the_tables_again_when_the_sha_differs() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let standin = Standin::start(Options::default()).await;
    let (writer, st) = setup_uploads(&env, port, "t_schema_owner", &standin).await;
    writer
        .write(
            "REMOVE TABLE upload; UPDATE meta:state SET extra_sha = 'an-older-build' RETURN id;",
            vars([]),
            None,
        )
        .await
        .expect("the older database");
    let (s, _) = upload_with(&st, 700, 700).await;
    assert_eq!(
        s,
        StatusCode::INTERNAL_SERVER_ERROR,
        "the table is gone: the write has nowhere to go"
    );
    assert_eq!(
        standin.objects().len(),
        0,
        "and the object that was stored is deleted again"
    );
    st.check_schema().await;
    assert!(
        st.schema_extra_current(),
        "the owner applied the file and stored its sha"
    );
    let (s, r) = upload_with(&st, 701, 701).await;
    assert_eq!(s, StatusCode::OK, "{r}");
}

/// A sync killed in the middle (an OOM kill) leaves `loading` true with `loading_by = 'sync'` and its short lease held.
/// The supervise round clears the flag once the lease has lapsed, with no reconnect (the restarted process is already
/// connected when it first looks) and still never a flag without `loading_by` (a dbload's). Red arm: a round that does not
/// call `recover_sync` leaves the flag up for good (the second assertion).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dead_syncs_flag_is_cleared_by_a_supervise_round_after_its_lease_lapses() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (writer, st) = setup(&env, port, "t_worker_syncdead").await;
    let flag = || async {
        let v = writer
            .last("SELECT VALUE loading FROM ONLY meta:state;", pk_store::store::vars([]))
            .await
            .expect("loading");
        pk_store::from_db(v) == json!(true)
    };
    writer
        .query(
            "UPDATE meta:state SET loading = true, loading_by = 'sync' RETURN NONE;",
            pk_store::store::vars([]),
        )
        .await
        .expect("flag");
    assert!(writer.lease_acquire("sync", "the-dead-sync", 1).await.expect("lease"));
    st.supervise_round().await;
    assert!(flag().await, "the lease is live: that sync may still be running");
    tokio::time::sleep(Duration::from_millis(1600)).await;
    st.supervise_round().await;
    assert!(
        !flag().await,
        "the lease lapsed: the round cleared the flag, no reconnect"
    );
    // a dbload's flag (no loading_by) stays
    writer
        .query(
            "UPDATE meta:state SET loading = true, loading_by = NONE RETURN NONE;",
            pk_store::store::vars([]),
        )
        .await
        .expect("dbload flag");
    st.supervise_round().await;
    assert!(flag().await, "a dbload's flag is never cleared");
}

/// A sync held open past its lease (a slow step) renews the lease: a supervise round does not clear `loading` under it,
/// and the sync ends cleanly. When the lease is taken from it meanwhile the sync stops before its next write, answers an
/// error, owes the sync and brings the flag down. Red arm: without the renewal task the lease lapses under the first
/// sync and the round clears its flag (the first assertion).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sync_held_open_past_its_lease_keeps_it_and_one_that_loses_it_stops() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (writer, st) = setup(&env, port, "t_worker_synclease").await;
    let state = || async {
        let v = writer
            .last(
                "SELECT loading, sync_owed FROM ONLY meta:state;",
                pk_store::store::vars([]),
            )
            .await
            .expect("state");
        let j = pk_store::from_db(v);
        (j["loading"] == json!(true), j["sync_owed"] == json!(true))
    };
    let input = |text: &str| {
        let mut g = graph();
        for n in g["nodes"].as_array_mut().unwrap().iter_mut() {
            if n["type"] == "unit" {
                n["text"] = json!(text);
                break;
            }
        }
        pk_store::SyncInput {
            graph: g,
            files: pk_kb::overlay::OverlayFiles::default(),
            laughs: vec![],
            sources: vec![],
            paths: std::collections::BTreeMap::new(),
            owner: "owner",
        }
    };
    let slow = async || {
        tokio::time::sleep(Duration::from_millis(3000)).await;
        Ok(std::collections::HashMap::new())
    };
    // 1. held open for three lease lengths: renewed, so a round leaves its flag alone
    let w = writer.clone();
    let first = tokio::spawn(async move {
        w.sync_graph_with(
            input("a unit text that changed, once"),
            slow,
            1,
            Duration::from_millis(300),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(1700)).await;
    assert!(state().await.0, "the sync is running and raised its flag");
    st.supervise_round().await;
    assert!(state().await.0, "a live (renewed) lease: the round leaves the flag");
    first.await.expect("task").expect("the sync ends cleanly");
    assert_eq!(state().await, (false, false), "down, and not owed");
    // 2. the lease taken from a running sync: it stops at its next write and is owed
    let w = writer.clone();
    let second = tokio::spawn(async move {
        w.sync_graph_with(
            input("a unit text that changed, twice"),
            slow,
            1,
            Duration::from_millis(300),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(900)).await;
    writer
        .query(
            "UPDATE lease:sync SET owner = 'someone-else', until = time::now() + 60s RETURN NONE;",
            pk_store::store::vars([]),
        )
        .await
        .expect("steal");
    let r = second.await.expect("task");
    assert!(r.is_err(), "a sync that lost its lease stops: {r:?}");
    assert_eq!(state().await, (false, true), "the flag is down and the sync is owed");
}

/// A sync whose future is dropped half-way (the process shutting down, an aborted task) stops renewing its lease: the
/// lease lapses and the next supervise round clears the flag it left. Red arm: a renewal task that outlives the future (the
/// guard without its `Drop`) keeps the lease for ever and the flag stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancelled_sync_stops_renewing_and_its_flag_is_recovered() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (writer, st) = setup(&env, port, "t_worker_synccancel").await;
    let loading = || async {
        let v = writer
            .last("SELECT VALUE loading FROM ONLY meta:state;", pk_store::store::vars([]))
            .await
            .expect("loading");
        pk_store::from_db(v) == json!(true)
    };
    let mut g = graph();
    for n in g["nodes"].as_array_mut().unwrap().iter_mut() {
        if n["type"] == "unit" {
            n["text"] = json!("a unit text that changed for a sync nobody finishes");
            break;
        }
    }
    let input = pk_store::SyncInput {
        graph: g,
        files: pk_kb::overlay::OverlayFiles::default(),
        laughs: vec![],
        sources: vec![],
        paths: std::collections::BTreeMap::new(),
        owner: "owner",
    };
    let w = writer.clone();
    let task = tokio::spawn(async move {
        w.sync_graph_with(
            input,
            async || {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(std::collections::HashMap::new())
            },
            1,
            Duration::from_millis(300),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(loading().await, "the sync raised its flag");
    task.abort();
    let _ = task.await;
    tokio::time::sleep(Duration::from_millis(2200)).await;
    st.supervise_round().await;
    assert!(
        !loading().await,
        "the renewal stopped with the sync: the lease lapsed and the round cleared the flag"
    );
}

/// At most `UPLOAD_SLOTS` recordings stream at once; the next one is a 503 with a reason (the phone keeps the recording) and
/// nothing is sent to the bucket. Red arm: without the permit check the upload goes through while all slots are held.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_a_few_recordings_stream_at_once_and_the_next_is_told_to_try_again() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let standin = Standin::start(Options::default()).await;
    let (writer, st) = setup_uploads(&env, port, "t_upload_slots", &standin).await;
    let held: Vec<_> = (0..pk_api::writes::uploads::UPLOAD_SLOTS)
        .map(|_| st.upload_slots().try_acquire_owned().expect("a free slot"))
        .collect();
    let before = standin.ops().len();
    let (s, r) = upload_with(&st, 900, 900).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{r}");
    assert!(r["error"].as_str().unwrap().contains("too many recordings"), "{r}");
    assert_eq!((standin.ops().len(), writer.upload_count().await.unwrap()), (before, 0));
    drop(held);
    let (s, r) = upload_with(&st, 901, 901).await;
    assert_eq!(s, StatusCode::OK, "a slot is free again: {r}");
    // the slot is given back when the upload ends
    assert_eq!(
        st.upload_slots().available_permits(),
        pk_api::writes::uploads::UPLOAD_SLOTS
    );
}
