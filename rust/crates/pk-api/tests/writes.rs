//! The write side against a scratch `SurrealDB` 3.3.4 (`surreal start memory`; skipped with a message when the
//! binary or the root password is missing, a failure with `PK_REQUIRE_EXTERNAL` set, as `fresh.rs`).
//!
//! What it proves (T10): a jot, a version, a decision, an undo, a transcript and the merge writes land through the
//! router and are read back through the same `AppState` at once; two writers racing for the same `decision.seq` or
//! the same `creative_event (store, seq)` never both land (the unique index refuses the second, `is_place_taken`
//! names it, the loser's retry lands next); the same transcript twice is one job; the router answers the writes
//! with the Python's statuses and refusal texts.

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use pk_api::writes::{Creative, Kb, is_place_taken};
use pk_api::{ApiConfig, AppState, app};
use pk_core::Env;
use pk_kb::Decision;
use pk_store::{Config, Store, to_db, vars};
use serde_json::{Value, json};
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
        "PK_REQUIRE_EXTERNAL is set and the writes test cannot run: {why}"
    );
    eprintln!("SKIP writes: {why}");
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

/// The pk-kb golden fixture's graph (the list's `title` renamed to the `name` column, as pk-store's test does).
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

/// A loaded scratch database (the graph, `meta:state` with a bundle, the graph as a `srcfile` row so the notebook
/// builds) and the state over it, auth off.
async fn setup(env: &Env, port: u16) -> (Store, AppState) {
    let cfg = Config::scratch(env, port, "punchable_test", "t_writes").expect("config");
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
    let api_env = Env::from_map(vec![
        ("SURREAL_PASS", "x"),
        ("PUBLIC_URL", "https://punchable.io/"),
        ("HOST", "127.0.0.1"),
        ("PK_AUTH", "0"),
        ("PK_ALLOW_NO_AUTH", "1"),
        ("PK_OWNER_ID", "owner"),
    ]);
    let st = AppState::new(
        ApiConfig::from_env(&api_env).expect("api config"),
        None,
        pk_obs::ApiMetrics::new("0.1.0", "").expect("metrics"),
        reqwest::Client::new(),
    );
    st.set_store(Some(Store::connect(cfg).await.expect("connect")));
    st.data().await.expect("first load");
    (writer, st)
}

async fn post(st: &AppState, path: &str, body: Value) -> (StatusCode, Value) {
    let bytes = serde_json::to_vec(&body).expect("json");
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, bytes.len())
        .body(Body::from(bytes))
        .unwrap();
    let r = app(st.clone()).oneshot(req).await.expect("infallible");
    let (parts, body) = r.into_parts();
    let b = axum::body::to_bytes(body, 1 << 20).await.expect("body");
    let v: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
    (parts.status, v)
}

fn line(target: &str, id: &str, n: usize) -> Decision {
    let Value::Object(m) = json!({
        "ts": "2026-11-03T12:00:00+00:00", "target": target, "id": id, "action": "edit", "before": {},
        "after": {"title": format!("race {n}")}, "by": "owner",
    }) else {
        unreachable!()
    };
    Decision(m)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_writes_land_and_are_read_back_at_once() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (_writer, st) = setup(&env, port).await;
    let d0 = st.data().await.expect("data");
    let n0 = d0.decisions().len();
    let joke = d0
        .view(false)
        .await
        .expect("view")
        .jokes
        .first()
        .cloned()
        .expect("a joke");
    let unit_text = {
        let v = d0.view(false).await.expect("view");
        v.nodes
            .values()
            .find(|n| n.get("type").and_then(Value::as_str) == Some("unit"))
            .and_then(|n| n.get("text"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .expect("a unit with text")
    };

    // a jot: one line, read back at once; the attach ran on the notebook built from the srcfile row
    let (s, r) = post(
        &st,
        "/api/ideas",
        json!({"text": "The parking meter that talks back, a new jot here."}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["indexes"], json!([n0]));
    assert!(r["id"].as_str().unwrap().starts_with("joke:"), "{r}");
    assert!(r["attach"].is_object(), "{r}");
    assert!(
        r["attach"].get("error").is_none(),
        "the notebook built and the placing ran: {r}"
    );
    assert_eq!(
        r["learned"],
        Value::Null,
        "his own typed line writes nothing to the learned section"
    );
    let d = st.data().await.expect("data");
    assert_eq!(d.decisions().len(), n0 + 1, "the very next read shows the jot");
    assert!(
        d.view(false)
            .await
            .unwrap()
            .nodes
            .contains_key(r["id"].as_str().unwrap())
    );

    // a jot in a unit's exact words: attached under its bit (words only: no model here)
    let (s, r) = post(&st, "/api/ideas", json!({"text": unit_text})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["attach"]["decision"], "attach", "{r}");
    assert!(r["attach"]["id"].as_str().unwrap().starts_with("at:"), "{r}");
    let cr = Creative::new(st.clone());
    let merges = cr.items("merges").await.expect("merges");
    assert_eq!(merges.len(), 1, "one attach row");

    // a version: a unit and its edge, both read back; a kept draft teaches the profile
    let (s, r) = post(
        &st,
        &format!("/api/jokes/{joke}/versions"),
        json!({"text": "A second draft of the line.", "reason": "tightened", "draft": true}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["indexes"].as_array().unwrap().len(), 2);
    assert_eq!(r["learned"]["kept"], 1, "{r}");
    let d = st.data().await.expect("data");
    let v = d.view(false).await.unwrap();
    assert!(
        v.joke_units[&joke]
            .iter()
            .any(|(u, _)| u == r["unit"].as_str().unwrap()),
        "the new unit is one of the bit's"
    );
    let profile = cr.items("profile").await.expect("profile");
    assert_eq!(profile["profile:voice"]["learned"]["kept"][0]["edited"], true);

    // a decision, then the undo of the newest line
    let (s, r) = post(
        &st,
        "/api/decision",
        json!({"target": "node", "id": joke, "action": "edit", "after": {"stage": "idea"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let idx = r["indexes"][0].as_u64().unwrap();
    let (s, r) = post(&st, "/api/undo", json!({"index": null})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["decisions"][0]["undo_of"], idx);
    let (s, r) = post(&st, "/api/undo", json!({"indexes": [idx]})).await;
    assert_eq!(
        (s, r["error"].as_str().unwrap()),
        (StatusCode::CONFLICT, "already undone")
    );

    // the refusals the Python answers with, to the letter
    let (s, r) = post(&st, "/api/ideas", json!({"text": "   "})).await;
    assert_eq!(
        (s, r["error"].as_str().unwrap()),
        (
            StatusCode::BAD_REQUEST,
            "an idea needs its text (up to 4000 characters)"
        )
    );
    let (s, r) = post(&st, "/api/jokes/joke:nope/versions", json!({"text": "x"})).await;
    assert_eq!(
        (s, r["error"].as_str().unwrap()),
        (StatusCode::NOT_FOUND, "no such joke")
    );
    let (s, r) = post(
        &st,
        &format!("/api/jokes/{joke}/versions"),
        json!({"text": "The petitioner and the respondent, index no 5."}),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{r}");
    let (s, r) = post(&st, "/api/merge/dismiss", json!({"jokes": ["joke:a"]})).await;
    assert_eq!(
        (s, r["error"].as_str().unwrap()),
        (StatusCode::BAD_REQUEST, "jokes must be a list of 2 or more joke ids")
    );

    // a transcript: one job, the same transcript again is the same job
    let tx = json!({"segments": [{"text": "Hello room.", "start": 0, "end": 2}], "title": "Phone set"});
    let (s, r) = post(&st, "/api/sets/transcript", tx.clone()).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{r}");
    let jid = r["job"]["id"].as_str().unwrap().to_string();
    assert!(jid.starts_with("job:set_text-"), "{r}");
    assert_eq!(r["poll"], format!("/api/jobs/{jid}"));
    let (s, r) = post(&st, "/api/sets/transcript", tx).await;
    assert_eq!(
        (s, r["job"]["id"].as_str().unwrap()),
        (StatusCode::ACCEPTED, jid.as_str())
    );
    let jobs = cr.items("jobs").await.expect("jobs");
    assert_eq!(jobs.values().filter(|j| j["kind"] == "set_text").count(), 1);
    assert!(
        jobs.values().any(|j| j["kind"] == "embed"),
        "the jots queued an embed job"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_writers_racing_for_one_place_never_both_land() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (writer, st) = setup(&env, port).await;
    let joke = st
        .data()
        .await
        .unwrap()
        .view(false)
        .await
        .unwrap()
        .jokes
        .first()
        .cloned()
        .expect("a joke");
    // the raw race: two appends of the same seq, straight at the store (what two processes do): one is refused by
    // the unique index with the error `is_place_taken` names, never a silent duplicate
    let (l1, l2) = (line("node", &joke, 1), line("node", &joke, 2));
    let (first, second) = tokio::join!(writer.append_decision(&l1, None), writer.append_decision(&l2, None));
    let landed = st.data().await.unwrap().decisions().len();
    match (first, second) {
        (Ok(_), Ok(_)) => assert_eq!(
            landed, 2,
            "both landed on distinct places (the second read the count after the first)"
        ),
        (Ok(_), Err(err)) | (Err(err), Ok(_)) => {
            assert!(is_place_taken(&err), "the refusal names the taken place: {err}");
            assert_eq!(landed, 1);
        }
        (Err(err), Err(other)) => panic!("both refused: {err} / {other}"),
    }
    // through the API's primitive, without the process lock (as two processes are): the loser builds its line
    // again from a fresh view and lands next
    let n0 = st.data().await.unwrap().decisions().len();
    let mut tasks = Vec::new();
    for w in 0..8usize {
        let st = st.clone();
        let joke = joke.clone();
        tasks.push(tokio::spawn(async move {
            let req =
                json!({"target": "node", "id": joke, "action": "edit", "after": {"title": format!("race {}", 10 + w)}});
            Kb::new(st, false)
                .decide_at(&req, "2026-11-03T12:00:00+00:00", "", "owner")
                .await
                .expect("the retry lands")
        }));
    }
    let mut seqs = Vec::new();
    for t in tasks {
        seqs.push(t.await.expect("task").0);
    }
    seqs.sort_unstable();
    assert_eq!(
        seqs,
        (n0..n0 + 8).collect::<Vec<_>>(),
        "every writer got its own place, no gap, no duplicate"
    );
    // the red arm of a retry that re-sends a line built before the race: four undos of the same decision at once;
    // exactly one lands, the others are the Python's 409 "already undone" (a retry that appended its stale inverse
    // again would land two or more inverses of one line)
    let target = i64::try_from(seqs[0]).unwrap();
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let st = st.clone();
        tasks.push(tokio::spawn(async move {
            Kb::new(st, false).undo(Some(target), "owner").await
        }));
    }
    let mut undone = 0;
    for t in tasks {
        match t.await.expect("task") {
            Ok(_) => undone += 1,
            Err(err) => assert_eq!((err.status, err.msg.as_str()), (StatusCode::CONFLICT, "already undone")),
        }
    }
    assert_eq!(undone, 1, "one inverse of one line");
    let after = st.data().await.unwrap();
    let inverses = after
        .decisions()
        .iter()
        .filter(|x| x.undo_of() == Some(seqs[0]))
        .count();
    assert_eq!(inverses, 1);
    // the same for the creative rows: eight transcripts at once, the jobs store's sequence whole
    let mut tasks = Vec::new();
    for w in 0..8usize {
        let st = st.clone();
        tasks.push(tokio::spawn(async move {
            post(
                &st,
                "/api/sets/transcript",
                json!({"segments": [{"text": format!("Set {w} about the bus.")}]}),
            )
            .await
            .0
        }));
    }
    for t in tasks {
        assert_eq!(t.await.expect("task"), StatusCode::ACCEPTED);
    }
    let rows = Arc::new(writer.creative_lines("jobs").await.expect("jobs rows"));
    assert_eq!(
        rows.len(),
        8,
        "eight rows, 0..8 with no gap (creative_lines checks the sequence)"
    );
}
