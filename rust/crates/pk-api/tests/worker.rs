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
        .args([
            "start",
            "--log",
            "warn",
            "--bind",
            &format!("127.0.0.1:{port}"),
            "memory",
        ])
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
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (_writer, st) = setup(&env, port, "t_worker_race").await;
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
        let st = st.clone();
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upload_is_rows_both_servers_read_and_a_taken_name_gets_the_next_suffix() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (writer, _st) = setup(&env, port, "t_upload").await;
    let meta = json!({"kind": "set", "title": "Late show", "marks_ms": [1, 2]});
    let a: Vec<u8> = (0..2_500_000_u32).map(|i| (i % 251) as u8).collect();
    for (seq, part) in a.chunks(pk_store::upload::PART).enumerate() {
        writer
            .upload_put_part("tok-a", i64::try_from(seq).unwrap(), part.to_vec())
            .await
            .unwrap();
    }
    let name = "2026-11-06_punchable-set_101010_take.m4a";
    assert_eq!(
        writer
            .upload_commit(name, "tok-a", &meta, i64::try_from(a.len()).unwrap(), 3)
            .await
            .unwrap(),
        name
    );
    writer.upload_put_part("tok-b", 0, vec![7; 5]).await.unwrap();
    let second = writer.upload_commit(name, "tok-b", &meta, 5, 1).await.unwrap();
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
        (row.bytes, row.parts, row.meta["title"].clone()),
        (2_500_000, 3, json!("Late show"))
    );
    let mut back = Vec::new();
    for seq in 0..row.parts {
        back.extend(writer.upload_part(&row.token, seq).await.unwrap().unwrap());
    }
    assert_eq!(back, a, "the bytes come back whole, in order");
    assert!(writer.upload_part(&row.token, 9).await.unwrap().is_none());
    // a part with no upload row, an hour old, is an orphan; a young one is not
    writer.upload_put_part("tok-orphan", 0, vec![1]).await.unwrap();
    writer.upload_sweep_orphans().await.unwrap();
    assert!(
        writer.upload_part("tok-orphan", 0).await.unwrap().is_some(),
        "a young part stays"
    );
    writer.upload_delete(name).await.unwrap();
    assert!(writer.upload_get(name).await.unwrap().is_none());
    assert!(
        writer.upload_part(&row.token, 0).await.unwrap().is_none(),
        "its parts went with it"
    );
    assert!(
        writer.upload_part("tok-b", 0).await.unwrap().is_some(),
        "another upload's parts stay"
    );
    writer.upload_delete("never-there.m4a").await.unwrap();
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

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn the_lease_is_granted_to_exactly_one_of_eight_askers_at_once() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (writer, _st) = setup(&env, port, "t_worker_lease").await;
    let cfg = Config::scratch(&env, port, "punchable_test", "t_worker_lease").expect("config");
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
    let mine = Lease::embedder("pk-api:test:1");
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
    let other = Lease::embedder("python:test:2");
    assert!(pk_api::sweep::once(&st, &other).await.unwrap().is_none());
    let short = Lease {
        ttl_s: 1,
        ..Lease::embedder("pk-api:test:3")
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
