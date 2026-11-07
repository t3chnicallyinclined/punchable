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

async fn send(st: &AppState, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let bytes = serde_json::to_vec(&body).expect("json");
    let req = Request::builder()
        .method(method)
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

/// T10 round 2: the Set page's, the Tonight list's, the set lists', his sets', the settings', the profile's and the
/// "not a joke?" writes land through the router and are read back at once, with the Python's statuses and texts.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_round_two_writes_land_and_are_read_back() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (_writer, st) = setup(&env, port).await;
    let d0 = st.data().await.expect("data");
    let n0 = d0.decisions().len();
    let v0 = d0.view(false).await.expect("view");
    let joke = v0.jokes.first().cloned().expect("a joke");
    let other = v0.jokes.get(1).cloned().expect("another joke");
    let perf = v0
        .nodes
        .values()
        .find(|n| n.get("type").and_then(Value::as_str) == Some("performance"))
        .and_then(|n| n.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .expect("a performance");

    // a rating: the line lands, the answer carries the bit when the moment has one
    let (s, r) = send(
        &st,
        "POST",
        "/api/review/rate",
        json!({"perf": perf, "rating": "worked"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(!r["indexes"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(
        r["decisions"].as_array().unwrap().last().unwrap()["after"]["rating"],
        "worked"
    );
    let (s, r) = send(
        &st,
        "POST",
        "/api/review/rate",
        json!({"perf": perf, "rating": "worked"}),
    )
    .await;
    assert_eq!(
        (s, r["error"].as_str().unwrap()),
        (StatusCode::CONFLICT, "already rated that way")
    );
    let (s, r) = send(
        &st,
        "POST",
        "/api/review/rate",
        json!({"perf": perf, "rating": "great"}),
    )
    .await;
    assert_eq!(
        (s, r["error"].as_str().unwrap()),
        (
            StatusCode::BAD_REQUEST,
            "rating must be one of ['killed', 'worked', 'meh', 'died']"
        )
    );
    let (s, r) = send(&st, "POST", "/api/set-row", json!({"perf": perf, "op": "crowd"})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["decisions"][0]["after"], json!({"kind": "crowd_work"}));
    let (s, r) = send(&st, "POST", "/api/set-row", json!({"perf": perf, "op": "zap"})).await;
    assert_eq!(
        (s, r["error"].as_str().unwrap()),
        (
            StatusCode::BAD_REQUEST,
            "op must be rate|match|new|crowd|bit|banter|confirm"
        )
    );
    let d = st.data().await.expect("data");
    assert!(d.decisions().len() >= n0 + 2, "the very next read shows the lines");

    // the tonight list, then the same list again
    let (s, r) = send(
        &st,
        "POST",
        "/api/tonight",
        json!({"items": [{"joke": joke, "done": 1}]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["tonight"]["source"], "saved");
    assert_eq!(r["tonight"]["items"][0]["id"], joke);
    assert_eq!(r["tonight"]["items"][0]["done"], true);
    let (s, r) = send(
        &st,
        "POST",
        "/api/tonight",
        json!({"items": [{"joke": joke, "done": true}]}),
    )
    .await;
    assert_eq!(
        (s, r["error"].as_str().unwrap()),
        (StatusCode::CONFLICT, "nothing changed")
    );
    let (s, r) = send(&st, "POST", "/api/tonight", json!({"items": [{"joke": "joke:nope"}]})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{r}");
    assert!(
        r["error"]
            .as_str()
            .unwrap()
            .starts_with("unknown joke in list: {'joke': 'joke:nope'}"),
        "{r}"
    );

    // a set list made, changed, added to, archived and brought back; a set promoted from it, versioned, locked,
    // a try note added and ticked
    let (s, r) = send(
        &st,
        "POST",
        "/api/boards",
        json!({"title": " Seven ", "joke_ids": [joke], "target_s": 300}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    let bid = r["board"]["id"].as_str().unwrap().to_string();
    assert!(bid.starts_with("sb:seven-"), "{bid}");
    assert_eq!(r["board"]["title"], "Seven");
    let path = format!("/api/boards/{bid}");
    let (s, r) = send(
        &st,
        "PATCH",
        &path,
        json!({"add": other, "move": {"from": 1, "to": 0}, "notes": "n"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["board"]["joke_ids"], json!([other, joke]));
    let (s, r) = send(&st, "PATCH", &path, json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{r}");
    let (s, r) = send(&st, "POST", &format!("{path}/items"), json!({"joke_id": joke})).await;
    assert_eq!(
        (s, r["error"].as_str().unwrap()),
        (StatusCode::CONFLICT, format!("{joke} already on the board").as_str())
    );
    let (s, r) = send(&st, "PATCH", &path, json!({"archive": true})).await;
    assert_eq!((s, r["board"]["archived"].as_bool()), (StatusCode::OK, Some(true)));
    let (s, r) = send(&st, "PATCH", &path, json!({"add": joke})).await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "an archived list refuses every change but unarchive and delete: {r}"
    );
    let (s, r) = send(&st, "PATCH", &path, json!({"unarchive": true, "remove": other})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["board"]["joke_ids"], json!([joke]));
    let cr = Creative::new(st.clone());
    assert_eq!(cr.items("storyboards").await.unwrap()[&bid]["archived"], false);

    let (s, r) = send(&st, "POST", "/api/workingsets", json!({"board_id": bid})).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    let wid = r["working_set"]["id"].as_str().unwrap().to_string();
    assert!(wid.starts_with("ws:seven-"), "{wid}");
    assert_eq!(r["working_set"]["versions"][0]["bits"][0]["joke_id"], joke);
    let wpath = format!("/api/workingsets/{wid}");
    let (s, r) = send(
        &st,
        "POST",
        &format!("{wpath}/versions"),
        json!({"note": "v2", "joke_ids": [other, joke]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!(r["working_set"]["current"], 2);
    let (s, r) = send(&st, "POST", &format!("{wpath}/lock"), json!({"joke_id": joke})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["working_set"]["locks"][&joke]["bit"], true);
    let (s, r) = send(&st, "POST", &format!("{wpath}/bits/{joke}/punchup"), json!({})).await;
    assert_eq!((s, r["status"].as_str().unwrap()), (StatusCode::OK, "locked"));
    let (s, r) = send(&st, "POST", &format!("/api/jokes/{other}/punchup"), json!({})).await;
    assert_eq!((s, r["status"].as_str().unwrap()), (StatusCode::OK, "no_tape"), "{r}");
    let (s, r) = send(
        &st,
        "POST",
        &format!("{wpath}/lock"),
        json!({"joke_id": joke, "line": "nobody said this"}),
    )
    .await;
    assert_eq!(
        (s, r["error"].as_str().unwrap()),
        (
            StatusCode::BAD_REQUEST,
            "that line is not verbatim in any taped night or written version of the bit"
        )
    );
    let (s, r) = send(&st, "POST", &format!("{wpath}/try"), json!({"text": " slower "})).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    let nid = r["note"]["id"].as_str().unwrap().to_string();
    assert!(nid.starts_with("try:"), "{nid}");
    assert_eq!(r["note"]["text"], "slower");
    let (s, r) = send(&st, "PATCH", &format!("{wpath}/try/{nid}"), json!({"done": true})).await;
    assert_eq!((s, r["changed"].as_bool()), (StatusCode::OK, Some(true)), "{r}");
    let (s, r) = send(&st, "PATCH", &format!("{wpath}/try/{nid}"), json!({"done": "yes"})).await;
    assert_eq!(
        (s, r["error"].as_str().unwrap()),
        (StatusCode::BAD_REQUEST, "done must be true or false")
    );
    let ws = cr.items("workingsets").await.unwrap();
    assert_eq!(
        ws[&wid]["try_notes"][0]["done"], true,
        "the tick is read back from the rows"
    );

    // the settings, the profile, a dismissed draft, a keep and a hide
    let (s, r) = send(&st, "PATCH", "/api/settings", json!({"training": true})).await;
    assert_eq!(
        (s, r["training"].as_bool(), r["source"].as_str()),
        (StatusCode::OK, Some(true), Some("his")),
        "{r}"
    );
    let (s, r) = send(&st, "PATCH", "/api/settings", json!({"foo": 1})).await;
    assert_eq!(
        (s, r["error"].as_str().unwrap()),
        (StatusCode::BAD_REQUEST, "unknown setting(s): foo")
    );
    let (s, r) = send(
        &st,
        "PUT",
        "/api/profile/voice",
        json!({"text": "\r\nI talk fast.\r\n\n"}),
    )
    .await;
    assert_eq!(
        (s, r["text"].as_str().unwrap()),
        (StatusCode::OK, "I talk fast.\n"),
        "{r}"
    );
    assert_eq!(r["chars"], 13);
    let (s, r) = send(&st, "PUT", "/api/profile/voice", json!({"text": "Then I wait."})).await;
    assert_eq!(
        (s, r["previous"]["text"].as_str().unwrap()),
        (StatusCode::OK, "I talk fast.\n")
    );
    let (s, r) = send(
        &st,
        "PUT",
        "/api/profile/voice",
        json!({"text": "The petitioner and the respondent, index no 5."}),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{r}");
    let (s, r) = send(
        &st,
        "POST",
        "/api/profile/voice/learn",
        json!({"dropped": ["a draft", {"text": "b", "bit": joke}]}),
    )
    .await;
    assert_eq!((s, r["learned"]["dropped"].as_u64()), (StatusCode::OK, Some(2)), "{r}");
    let (s, r) = send(
        &st,
        "POST",
        "/api/suggestions/not-a-joke/keep",
        json!({"joke_id": joke}),
    )
    .await;
    assert_eq!(r, json!({"ok": true, "joke_id": joke, "kept": true}), "{s}");
    let (s, r) = send(
        &st,
        "POST",
        "/api/suggestions/not-a-joke/keep",
        json!({"joke_id": joke}),
    )
    .await;
    assert_eq!(
        (s, r["kept"].as_bool()),
        (StatusCode::OK, Some(true)),
        "kept again: no second row"
    );
    assert_eq!(cr.items("suggestions").await.unwrap().len(), 1);
    let (s, r) = send(
        &st,
        "POST",
        "/api/suggestions/not-a-joke/hide",
        json!({"joke_id": other}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["decisions"][0]["after"], json!({"hidden": true}));
    assert_eq!(r["undo"]["indexes"], r["indexes"]);
    let d = st.data().await.expect("data");
    assert!(
        !d.view(false).await.unwrap().nodes.contains_key(&other),
        "the hidden bit left the visible view at once"
    );
    assert!(d.view(true).await.unwrap().nodes.contains_key(&other));
}

/// T10 round 2: eight writers changing one set list at once, each through the router's probe-then-append, and
/// eight ratings of one moment at once (the first line rebuilt from a fresh view on a lost race): every write
/// lands once, the store's rows have no gap, and no answer is a 500.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eight_writers_on_one_list_and_one_moment_all_land() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (writer, st) = setup(&env, port).await;
    let v0 = st.data().await.unwrap().view(false).await.unwrap();
    let joke = v0.jokes.first().cloned().expect("a joke");
    let perf = v0
        .nodes
        .values()
        .find(|n| n.get("type").and_then(Value::as_str) == Some("performance"))
        .and_then(|n| n.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .expect("a performance");
    let (s, r) = send(
        &st,
        "POST",
        "/api/boards",
        json!({"title": "Raced", "joke_ids": [joke]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    let bid = r["board"]["id"].as_str().unwrap().to_string();
    let n0 = writer.creative_lines("storyboards").await.expect("rows").len();
    let mut tasks = Vec::new();
    for w in 0..8usize {
        let st = st.clone();
        let bid = bid.clone();
        tasks.push(tokio::spawn(async move {
            send(
                &st,
                "PATCH",
                &format!("/api/boards/{bid}"),
                json!({"title": format!("Raced {w}"), "notes": format!("note {w}")}),
            )
            .await
        }));
    }
    for t in tasks {
        let (s, r) = t.await.expect("task");
        assert_eq!(s, StatusCode::OK, "{r}");
    }
    let rows = writer.creative_lines("storyboards").await.expect("rows");
    assert_eq!(rows.len(), n0 + 8, "eight update rows, 0..n with no gap");
    let cr = Creative::new(st.clone());
    let title = cr.items("storyboards").await.unwrap()[&bid]["title"].clone();
    assert!(title.as_str().unwrap().starts_with("Raced "), "{title}");

    // the raw lines of a rating, eight at once on one moment: ratings alternate so none is "already rated"
    let d0 = st.data().await.unwrap().decisions().len();
    let mut tasks = Vec::new();
    for w in 0..8usize {
        let st = st.clone();
        let perf = perf.clone();
        tasks.push(tokio::spawn(async move {
            let rating = ["killed", "worked", "meh", "died"][w % 4];
            send(
                &st,
                "POST",
                "/api/set-row",
                json!({"perf": perf, "op": "rate", "rating": rating}),
            )
            .await
        }));
    }
    let mut landed = 0;
    for t in tasks {
        let (s, r) = t.await.expect("task");
        match s {
            StatusCode::OK => landed += 1,
            StatusCode::CONFLICT => assert_eq!(r["error"], "already rated that way"),
            other => panic!("a rating answered {other}: {r}"),
        }
    }
    let d = st.data().await.unwrap();
    assert_eq!(
        d.decisions().len(),
        d0 + landed,
        "every rating that was answered 200 is one line, none twice"
    );
    assert!(landed >= 4, "at least the four distinct ratings landed: {landed}");
}

/// Film it (build 24): `POST /api/jokes/<id>/rant` and `/hook` queue one job over the version the KB holds (its
/// lines, its cut read from the source), a repeat answers the same queued job, and the refusals carry the Python's
/// statuses and texts. A version of another bit must 404: a lenient membership check would queue it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_film_it_sessions_queue_over_the_version_the_kb_holds() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (_writer, st) = setup(&env, port).await;
    let v0 = st.data().await.expect("data").view(false).await.expect("view");
    let joke = v0.jokes.first().cloned().expect("a joke");
    let other = v0.jokes.get(1).cloned().expect("another joke");
    let (s, r) = send(
        &st,
        "POST",
        &format!("/api/jokes/{joke}/versions"),
        json!({"text": "You scan forty things.\r\n\n  Nobody claps.  ", "source": "rant_session"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let unit = r["unit"].as_str().unwrap().to_string();
    let (_, r) = send(
        &st,
        "POST",
        &format!("/api/jokes/{other}/versions"),
        json!({"text": "Not this bit."}),
    )
    .await;
    let other_unit = r["unit"].as_str().unwrap().to_string();

    let rant = format!("/api/jokes/{joke}/rant");
    let hook = format!("/api/jokes/{joke}/hook");
    let (s, r) = send(&st, "POST", &rant, json!({"unit": unit, "text": "ignored"})).await;
    assert_eq!((s, r["status"].as_str()), (StatusCode::ACCEPTED, Some("queued")), "{r}");
    let jid = r["job"]["id"].as_str().unwrap().to_string();
    let (s, r) = send(&st, "POST", &rant, json!({"unit": unit})).await;
    assert_eq!(
        (s, r["job"]["id"].as_str()),
        (StatusCode::ACCEPTED, Some(jid.as_str())),
        "a repeat is the same job"
    );
    let jobs = Creative::new(st.clone()).items("jobs").await.unwrap();
    let j = &jobs[&jid];
    assert_eq!(j["kind"], "rant");
    assert_eq!(j["key"], json!(format!("rant|{joke}|{unit}|-")));
    assert_eq!(j["input"]["cut"], "rant");
    assert_eq!(
        j["input"]["lines"],
        json!(["You scan forty things.", "  Nobody claps."])
    );
    assert_eq!(j["input"]["punch"], Value::Null);

    let body = json!({"unit": unit, "exclude": [" One. ", "Two."], "punch": 1});
    let (s, r) = send(&st, "POST", &hook, body).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{r}");
    let hid = r["job"]["id"].as_str().unwrap().to_string();
    let jobs = Creative::new(st.clone()).items("jobs").await.unwrap();
    assert_eq!(jobs[&hid]["input"]["exclude"], json!(["One.", "Two."]));
    assert!(
        jobs[&hid]["key"]
            .as_str()
            .unwrap()
            .starts_with(&format!("hook|{joke}|{unit}|1|"))
    );

    let punch_msg = "punch must be the index of a line of that version";
    let ex_msg = "exclude must be a list of openers (at most 6)";
    let cases = [
        (
            rant.clone(),
            json!({}),
            400,
            "unit is required (the version to work on)".to_string(),
        ),
        (
            rant.clone(),
            json!({"unit": other_unit}),
            404,
            "that version is not on this bit".into(),
        ),
        (
            "/api/jokes/joke:nope/rant".into(),
            json!({"unit": unit}),
            404,
            "no such joke joke:nope".into(),
        ),
        (rant.clone(), json!({"unit": unit, "punch": 2}), 400, punch_msg.into()),
        (
            rant.clone(),
            json!({"unit": unit, "punch": true}),
            400,
            punch_msg.into(),
        ),
        (
            hook.clone(),
            json!({"unit": unit, "exclude": vec!["x"; 7]}),
            400,
            ex_msg.into(),
        ),
        (hook.clone(), json!({"unit": unit, "exclude": [""]}), 400, ex_msg.into()),
    ];
    for (path, body, status, msg) in cases {
        let (s, r) = send(&st, "POST", &path, body.clone()).await;
        assert_eq!(
            (s.as_u16(), r["error"].as_str().unwrap_or("")),
            (status, msg.as_str()),
            "{path} {body}"
        );
    }
}

/// T10 round 5, group A: the Ask conversations. A question is retrieved from the notebook and queued for the worker;
/// the GET that finds the job done gates the answer (a quote that is word for word stays, the chip carries it); a
/// failed job reads as plain words; nothing retrieved is the refusal at once; a rename and a hide change the list.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ask_conversations_queue_settle_and_are_renamed() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (_writer, st) = setup(&env, port).await;
    let (s, r) = send(
        &st,
        "POST",
        "/api/ask/chats",
        json!({"question": "  what did I write about   the meter and pronouns? "}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    let cid = r["id"].as_str().unwrap().to_string();
    assert!(cid.starts_with("chat:") && cid.len() == 13, "{cid}");
    assert_eq!(r["title"], "what did I write about the meter and pronouns?");
    assert_eq!(r["turns"][0]["status"], "thinking", "{r}");
    assert_eq!(r["turns"][0]["q"], "what did I write about the meter and pronouns?");
    assert!(r["worker"].is_object());
    let (s, list) = get_path(&st, "/api/ask/chats").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list["chats"][0]["id"], cid.as_str());
    assert_eq!(list["chats"][0]["questions"], 1);
    assert_eq!(list["chats"][0]["thinking"], true);
    let (s, r2) = send(
        &st,
        "POST",
        &format!("/api/ask/chats/{cid}"),
        json!({"question": "and another"}),
    )
    .await;
    assert_eq!(
        (s, r2["error"].as_str()),
        (StatusCode::CONFLICT, Some("still thinking about the last question"))
    );

    // the worker takes the job and answers: a quote that is in the source, and one that is not
    let (s, c) = send(
        &st,
        "POST",
        "/api/worker/claim",
        json!({"kinds": ["ask_chat"], "worker": "w1", "hosted": true}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{c}");
    let jid = c["job"]["id"].as_str().unwrap().to_string();
    let msgs = c["job"]["input"]["messages"].as_array().expect("the messages");
    assert_eq!(msgs[0]["role"], "system");
    assert!(msgs.last().unwrap()["content"].as_str().unwrap().contains("[S1]"));
    let answer = json!({"found": true, "answer": "You wrote it down [S1].",
                        "citations": [{"source": "S1", "quote": "The meter asked for my pronouns"},
                                      {"source": "S9", "quote": "nobody"}]});
    let (s, _) = send(
        &st,
        "POST",
        &format!("/api/worker/jobs/{jid}/result"),
        json!({"ok": true, "result": {"status": "answered", "data": answer, "provider": "p", "model": "m"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, r) = get_path(&st, &format!("/api/ask/chats/{cid}")).await;
    assert_eq!(s, StatusCode::OK);
    let t = &r["turns"][0];
    assert_eq!(t["status"], "done", "{r}");
    assert_eq!(
        (t["found"].clone(), t["answer"].as_str()),
        (json!(true), Some("You wrote it down [1]."))
    );
    assert_eq!(t["chips"][0]["quote"], "The meter asked for my pronouns");
    assert_eq!(t["chips"][0]["source"], "unit:m1");
    assert_eq!((t["dropped_n"].clone(), t["model"].clone()), (json!(1), json!("m")));
    // read again: settled once, the same answer
    let (_, again) = get_path(&st, &format!("/api/ask/chats/{cid}")).await;
    assert_eq!(again["turns"], r["turns"]);
    let lines = Creative::new(st.clone()).lines("chats").await.unwrap();
    assert_eq!(lines.len(), 3, "new, ask, answer: {lines:?}");

    // a follow-up that fails at the worker reads as plain words
    let (s, r) = send(
        &st,
        "POST",
        &format!("/api/ask/chats/{cid}"),
        json!({"question": "tell me more about the meter"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["turns"][1]["id"], "t2");
    let (_, c) = send(
        &st,
        "POST",
        "/api/worker/claim",
        json!({"kinds": ["ask_chat"], "worker": "w1", "hosted": true}),
    )
    .await;
    let jid = c["job"]["id"].as_str().unwrap().to_string();
    send(
        &st,
        "POST",
        &format!("/api/worker/jobs/{jid}/result"),
        json!({"ok": false, "error": "429 rate limit hit", "retry": false}),
    )
    .await;
    let (_, r) = get_path(&st, &format!("/api/ask/chats/{cid}")).await;
    assert_eq!(r["turns"][1]["status"], "failed", "{r}");
    assert!(
        r["turns"][1]["error"]
            .as_str()
            .unwrap()
            .starts_with("Claude is at its usage limit")
    );

    // nothing retrieved: the refusal at once, no job
    let (s, r) = send(&st, "POST", "/api/ask/chats", json!({"question": "zzqx wvvk"})).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!(r["turns"][0]["status"], "done");
    assert_eq!(r["turns"][0]["answer"], "I couldn't find that in your notebook.");
    assert_eq!(r["turns"][0]["found"], false);

    // a conversation about one bit; the refusals
    let (s, r) = send(
        &st,
        "POST",
        "/api/ask/chats",
        json!({"question": "meter", "joke": "joke:meters"}),
    )
    .await;
    assert_eq!(
        (s, r["joke"].as_str()),
        (StatusCode::CREATED, Some("joke:meters")),
        "{r}"
    );
    let cases = [
        (json!({}), 400, "question is required (at most 1000 characters)"),
        (json!({"question": "x", "joke": 5}), 400, "joke must be a joke id"),
        (
            json!({"question": "x", "joke": "joke:nope"}),
            404,
            "no such joke joke:nope",
        ),
    ];
    for (body, status, msg) in cases {
        let (s, r) = send(&st, "POST", "/api/ask/chats", body.clone()).await;
        assert_eq!((s.as_u16(), r["error"].as_str().unwrap_or("")), (status, msg), "{body}");
    }
    for p in ["/api/ask/chats/nope", "/api/ask/chats/chat:00000000"] {
        let (s, r) = get_path(&st, p).await;
        assert_eq!(
            (s, r["error"].as_str()),
            (StatusCode::NOT_FOUND, Some("no such conversation"))
        );
    }

    // a rename and a hide
    let path = format!("/api/ask/chats/{cid}");
    let (s, r) = send(&st, "PATCH", &path, json!({"title": "  Meters  "})).await;
    assert_eq!((s, r["title"].as_str()), (StatusCode::OK, Some("Meters")), "{r}");
    for (body, msg) in [
        (json!({}), "nothing to change (title, deleted)"),
        (json!({"title": "  "}), "title must be 1-120 characters"),
    ] {
        let (s, r) = send(&st, "PATCH", &path, body).await;
        assert_eq!((s.as_u16(), r["error"].as_str()), (400, Some(msg)));
    }
    let (_, before) = get_path(&st, "/api/ask/chats").await;
    send(&st, "PATCH", &path, json!({"deleted": true})).await;
    let (_, after) = get_path(&st, "/api/ask/chats").await;
    assert_eq!(
        before["chats"].as_array().unwrap().len() - 1,
        after["chats"].as_array().unwrap().len()
    );
    let (s, _) = get_path(&st, &path).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "a hidden conversation is not found");
    let (s, r) = send(&st, "PATCH", &path, json!({"deleted": false})).await;
    assert_eq!(
        (s, r["id"].as_str()),
        (StatusCode::OK, Some(cid.as_str())),
        "a PATCH finds a hidden one"
    );
}

async fn get_path(st: &AppState, path: &str) -> (StatusCode, Value) {
    send(st, "GET", path, Value::Null).await
}

/// T10 round 5, group B: the rooms. A notebook with no analysed set has no rooms; a pin or a rename of a room that is
/// not there is the Python's 404 before anything else is looked at, and nothing is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_rooms_answer_and_an_unknown_room_is_not_found() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (_writer, st) = setup(&env, port).await;
    let (s, r) = get_path(&st, "/api/venues").await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["venues"], json!([]));
    assert_eq!(r["ambiguous"], json!([]));
    assert_eq!(r["geocoder"]["requests_this_run"], 0);
    assert!(r["generated_at"].is_string());
    let (s, r) = get_path(&st, "/api/venues.geojson").await;
    assert_eq!(
        (s, r["type"].as_str(), r["features"].clone()),
        (StatusCode::OK, Some("FeatureCollection"), json!([]))
    );
    for body in [json!({"name": "x"}), json!({}), json!({"lat": 1, "lon": 2})] {
        let (s, r) = send(&st, "PATCH", "/api/venues/venue:nope", body).await;
        assert_eq!((s, r["error"].as_str()), (StatusCode::NOT_FOUND, Some("no such venue")));
    }
    let lines = Creative::new(st.clone()).lines("venues").await.unwrap();
    assert!(lines.is_empty(), "a refused edit writes nothing: {lines:?}");
}

/// The Rant tab (slice 1, Rust only: no Python twin, so the statuses and words are this test's): his topics are
/// added, answered once for the same words, marked ranted and deleted; a draft queues one `rant_draft` job over his
/// words read from the KB (a bit's newest version, a taped moment), never the text the phone sends; `/api/rants` lists
/// a bit once it has a rant cut. The red arm: the job's input must not carry the phone's text.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_rant_tab_topics_draft_and_rants() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (_writer, st) = setup(&env, port).await;
    let v0 = st.data().await.expect("data").view(false).await.expect("view");
    let joke = v0.jokes.first().cloned().expect("a joke");

    // topics: add, the same words again, list newest first
    let (s, r) = send(
        &st,
        "POST",
        "/api/topics",
        json!({"text": "  self-checkout \n machines "}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let tid = r["topic"]["id"].as_str().unwrap().to_string();
    assert_eq!(r["topic"]["text"], "self-checkout machines");
    let (_, again) = send(&st, "POST", "/api/topics", json!({"text": "Self-Checkout Machines"})).await;
    assert_eq!(
        again["topic"]["id"],
        json!(tid),
        "the same words answer the topic he has"
    );
    let (_, r) = send(&st, "POST", "/api/topics", json!({"text": "airport bins"})).await;
    let tid2 = r["topic"]["id"].as_str().unwrap().to_string();
    let (s, list) = send(&st, "GET", "/api/topics", Value::Null).await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = list["topics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert_eq!(ids, [tid2.as_str(), tid.as_str()], "newest first");

    // a draft: his words from the KB, the job queued once
    let (s, r) = send(
        &st,
        "POST",
        &format!("/api/jokes/{joke}/versions"),
        json!({"text": "Self-checkout is the store hiring you for free."}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let body = json!({"topic": "self-checkout machines", "topic_id": tid, "sources": [joke, "perf:p1", joke],
                      "text": "PHONE TEXT, never read"});
    let (s, r) = send(&st, "POST", "/api/rant/draft", body.clone()).await;
    assert_eq!((s, r["status"].as_str()), (StatusCode::ACCEPTED, Some("queued")), "{r}");
    let jid = r["job"]["id"].as_str().unwrap().to_string();
    let (_, r2) = send(&st, "POST", "/api/rant/draft", body).await;
    assert_eq!(r2["job"]["id"], json!(jid), "a repeat is the same job");
    let jobs = Creative::new(st.clone()).items("jobs").await.unwrap();
    let input = &jobs[&jid]["input"];
    assert_eq!(jobs[&jid]["kind"], "rant_draft");
    assert_eq!(input["topic_id"], json!(tid));
    let srcs = input["sources"].as_array().unwrap();
    assert_eq!(srcs.len(), 2, "each source once: {input}");
    assert_eq!(srcs[0]["id"], json!(joke));
    assert_eq!(srcs[0]["kind"], "bit");
    assert_eq!(
        srcs[0]["text"], "Self-checkout is the store hiring you for free.",
        "the newest version, from the KB"
    );
    assert!(srcs[0]["label"].as_str().unwrap().contains(" v"), "{}", srcs[0]);
    assert_eq!(srcs[1]["kind"], "moment");
    assert_eq!(srcs[1]["text"], "so the parking meter wants my pronouns now");
    // the red arm: nothing the phone sent as text reached the worker's input
    assert!(
        !input.to_string().contains("PHONE TEXT"),
        "the phone's text must never be the source"
    );

    let cases = [
        (json!({}), 400, "topic is required (what the rant is about)".to_string()),
        (json!({"topic": "x", "topic_id": "joke:x"}), 400, "bad topic id".into()),
        (
            json!({"topic": "x", "sources": "joke:x"}),
            400,
            "sources must be a list of at most 12 bit or moment ids".into(),
        ),
        (
            json!({"topic": "x", "sources": vec![joke.clone(); 13]}),
            400,
            "sources must be a list of at most 12 bit or moment ids".into(),
        ),
        (
            json!({"topic": "x", "sources": ["joke:nope"]}),
            404,
            "joke:nope is not a bit or a taped moment with words".into(),
        ),
        // the red arm's twin: a source the phone sends as an object with its own words is refused, never read
        (
            json!({"topic": "x", "sources": [{"id": joke, "text": "PHONE TEXT"}]}),
            400,
            "sources must be a list of at most 12 bit or moment ids".into(),
        ),
    ];
    for (b, status, msg) in cases {
        let (s, r) = send(&st, "POST", "/api/rant/draft", b.clone()).await;
        assert_eq!(
            (s.as_u16(), r["error"].as_str().unwrap_or("")),
            (status, msg.as_str()),
            "{b}"
        );
    }

    // a rant cut: the bit is listed, the topic marked, then deleted
    let (_, rants) = send(&st, "GET", "/api/rants", Value::Null).await;
    assert!(
        rants["rants"].as_array().unwrap().is_empty(),
        "no rant cut yet: {rants}"
    );
    let (s, r) = send(
        &st,
        "POST",
        &format!("/api/jokes/{joke}/versions"),
        json!({"text": "Stop scanning your own groceries.\nSelf-checkout is the store hiring you for free.",
               "source": "rant_draft", "draft": true}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let (_, rants) = send(&st, "GET", "/api/rants", Value::Null).await;
    let row = &rants["rants"][0];
    assert_eq!(row["id"], json!(joke), "{rants}");
    assert_eq!(row["written"], true);
    assert_eq!(row["v"], row["versions"], "the newest version is the rant cut");
    let (s, r) = send(&st, "PATCH", &format!("/api/topics/{tid}"), json!({"ranted": joke})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["topic"]["rant"], json!(joke));
    assert!(r["topic"]["ranted_at"].is_string());
    let bad = [
        (
            format!("/api/topics/{tid}"),
            json!({}),
            400,
            "send one of {ranted}, {delete: true}, {undelete: true}, {rename}, {keep}, {drop}, {unmark}, {jot}, {script}",
        ),
        (
            format!("/api/topics/{tid}"),
            json!({"ranted": "unit:x"}),
            400,
            "send one of {ranted}, {delete: true}, {undelete: true}, {rename}, {keep}, {drop}, {unmark}, {jot}, {script}",
        ),
        (
            "/api/topics/topic:nope".to_string(),
            json!({"delete": true}),
            404,
            "no topic topic:nope",
        ),
        (
            "/api/topics/joke:x".to_string(),
            json!({"delete": true}),
            400,
            "bad topic id",
        ),
    ];
    for (path, b, status, msg) in bad {
        let (s, r) = send(&st, "PATCH", &path, b.clone()).await;
        assert_eq!(
            (s.as_u16(), r["error"].as_str().unwrap_or("")),
            (status, msg),
            "{path} {b}"
        );
    }
    let (s, _) = send(&st, "PATCH", &format!("/api/topics/{tid}"), json!({"delete": true})).await;
    assert_eq!(s, StatusCode::OK);
    let (_, list) = send(&st, "GET", "/api/topics", Value::Null).await;
    assert_eq!(list["topics"].as_array().unwrap().len(), 1);
    let (s, r) = send(&st, "POST", "/api/topics", json!({"text": " "})).await;
    assert_eq!(
        (s.as_u16(), r["error"].as_str()),
        (400, Some("text is required (the topic)"))
    );

    // a new version of a ticked bit changes its text: a fresh draft, not the cached one
    let (_, r) = send(
        &st,
        "POST",
        &format!("/api/jokes/{joke}/versions"),
        json!({"text": "A newer take."}),
    )
    .await;
    assert!(r["unit"].is_string(), "{r}");
    let (s, r3) = send(
        &st,
        "POST",
        "/api/rant/draft",
        json!({"topic": "self-checkout machines", "sources": ["perf:p1", joke]}),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{r3}");
    assert_ne!(r3["job"]["id"], json!(jid), "changed text, a fresh job");
    let jobs = Creative::new(st.clone()).items("jobs").await.unwrap();
    let srcs = jobs[r3["job"]["id"].as_str().unwrap()]["input"]["sources"].clone();
    assert_eq!(
        srcs[1]["text"], "A newer take.",
        "the newest version, saved the same day: {srcs}"
    );

    // the cap is the route's 400, never the reducer's 500
    for k in 0..199 {
        let (s, r) = send(&st, "POST", "/api/topics", json!({"text": format!("topic {k}")})).await;
        assert_eq!(s, StatusCode::OK, "{k}: {r}");
    }
    let (s, r) = send(&st, "POST", "/api/topics", json!({"text": "one too many"})).await;
    assert_eq!(
        (s.as_u16(), r["error"].as_str()),
        (400, Some("at most 200 topics: delete one first"))
    );
}

/// The creative space (spec §12.7, Rust only: no Python twin, so the statuses and words are this test's): a topic is a
/// page of his words. Keep, drop, unmark and jot are references on the topic, never a change to the bit; the tab's row
/// and the page's heading are one number; a found hit is a real word match (no model here); the script's marks come
/// back on the version they were written for and never on another; a deleted topic comes back with its material; a
/// draft with no sources named draws on Kept and carries his lines as fixed; Claude on a line queues one `rant_line`.
/// The red arm: a row whose count ignored Kept would not equal the heading after a keep (asserted below by value).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[expect(
    clippy::many_single_char_names,
    reason = "a and b are the two bits of the story; s, r, p are the file's own status / reply / page names"
)]
async fn the_topic_page_material_script_and_line() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (_writer, st) = setup(&env, port).await;
    let v0 = st.data().await.expect("data").view(false).await.expect("view");
    let (a, b) = (v0.jokes[0].clone(), v0.jokes[1].clone());
    for (j, text) in [
        (&a, "Self-checkout is the store hiring you for free."),
        (&b, "The self-checkout called a manager on me."),
    ] {
        let (s, r) = send(&st, "POST", &format!("/api/jokes/{j}/versions"), json!({"text": text})).await;
        assert_eq!(s, StatusCode::OK, "{r}");
    }
    let units_b = st
        .data()
        .await
        .expect("data")
        .view(false)
        .await
        .expect("view")
        .joke_units
        .get(&b)
        .map(Vec::len);
    let (_, r) = send(&st, "POST", "/api/topics", json!({"text": "self-checkout"})).await;
    let tid = r["topic"]["id"].as_str().unwrap().to_string();
    let page = format!("/api/topics/{tid}");

    // the page: both bits found by their words (no distance), whole words on screen
    let (s, p) = send(&st, "GET", &page, Value::Null).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    let found: Vec<&str> = p["found"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["ref"].as_str())
        .collect();
    assert!(found.contains(&a.as_str()) && found.contains(&b.as_str()), "{p}");
    let fa = p["found"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["ref"] == json!(a))
        .unwrap();
    assert_eq!(fa["kind"], "bit");
    assert_eq!(fa["words"], "Self-checkout is the store hiring you for free.");
    assert!(fa["label"].as_str().unwrap().contains(" v"), "{fa}");
    assert_eq!(fa["distance"], Value::Null, "a word match");
    assert_eq!(p["found_more"], false);

    // keep one: it leaves Found for Kept; the row and the heading are one number
    let (s, r) = send(&st, "PATCH", &page, json!({"keep": a})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let (_, p) = send(&st, "GET", &page, Value::Null).await;
    assert_eq!(p["kept"][0]["ref"], json!(a));
    let n_found = p["found"].as_array().unwrap().len();
    let (_, list) = send(&st, "GET", "/api/topics", Value::Null).await;
    let row = list["topics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == json!(tid))
        .unwrap()
        .clone();
    assert_eq!(row["found"], json!(n_found), "the row's number is the heading's: {row}");
    assert_eq!(row["kept"], 1);
    assert!(row.get("material").is_none(), "the row carries counts, not the refs");
    // the red arm: the row before the keep counted the kept bit as found; that number would now be wrong
    assert_ne!(row["found"], json!(found.len()), "Kept left Found: {row}");

    // drop the other, put it back; unmark the kept one
    let (_, _) = send(&st, "PATCH", &page, json!({"drop": b})).await;
    let (_, p) = send(&st, "GET", &page, Value::Null).await;
    assert_eq!(p["dropped"][0]["ref"], json!(b));
    assert!(
        !p["found"].as_array().unwrap().iter().any(|m| m["ref"] == json!(b)),
        "a dropped bit stays out of Found"
    );
    let (_, _) = send(&st, "PATCH", &page, json!({"unmark": b})).await;
    let (_, p) = send(&st, "GET", &page, Value::Null).await;
    assert!(
        p["found"].as_array().unwrap().iter().any(|m| m["ref"] == json!(b)),
        "Put back"
    );
    // the bit itself never changed
    let v1 = st.data().await.expect("data").view(false).await.expect("view");
    assert_eq!(
        v1.joke_units.get(&b).map(Vec::len),
        units_b,
        "keep, drop and unmark never touch the bit"
    );

    // a jot: a real idea, then a reference; listed first under Kept, as a jot
    let (s, idea) = send(
        &st,
        "POST",
        "/api/ideas",
        json!({"text": "The scale knows my secrets."}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{idea}");
    let jot = idea["id"].as_str().unwrap().to_string();
    let (s, r) = send(&st, "PATCH", &page, json!({"jot": jot})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let (_, p) = send(&st, "GET", &page, Value::Null).await;
    assert_eq!(
        (p["kept"][0]["ref"].clone(), p["kept"][0]["kind"].clone()),
        (json!(jot), json!("jot")),
        "{p}"
    );
    let (_, list) = send(&st, "GET", "/api/topics", Value::Null).await;
    assert_eq!(list["topics"][0]["jots"], 1);

    // the page's search: word hits with their state on this topic
    let (s, m) = send(&st, "GET", &format!("{page}/material?q=manager"), Value::Null).await;
    assert_eq!(s, StatusCode::OK, "{m}");
    assert_eq!(m["rows"][0]["ref"], json!(b));
    assert_eq!(m["rows"][0]["state"], Value::Null);
    let (_, m) = send(
        &st,
        "GET",
        &format!("{page}/material?q=nothing-of-his-says-this"),
        Value::Null,
    )
    .await;
    assert_eq!(m["rows"], json!([]));
    let (s, m) = send(&st, "GET", &format!("{page}/material"), Value::Null).await;
    assert_eq!(s, StatusCode::OK, "Find more answers: {m}");

    // the rant: a version with its marks; the marks come back on that version only
    let (_, r) = send(
        &st,
        "POST",
        &format!("/api/jokes/{a}/versions"),
        json!({"text": "Nobody asked me.\nSelf-checkout is the store hiring you for free.", "source": "rant_write"}),
    )
    .await;
    let unit = r["unit"].as_str().unwrap().to_string();
    let (s, r) = send(&st, "PATCH", &page, json!({"ranted": a})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    // the first line claims to come from the other bit, whose words do not hold it: the server drops that claim
    let marks =
        json!([{"by": "him", "from": b, "label": "Lie v1"}, {"by": "him", "from": a, "label": "Self-checkout v2"}]);
    let (s, r) = send(&st, "PATCH", &page, json!({"script": {"unit": unit, "lines": marks}})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let (_, p) = send(&st, "GET", &page, Value::Null).await;
    let rant = &p["rants"][0];
    assert_eq!(rant["unit"], json!(unit));
    assert_eq!(
        rant["lines"][1]["text"],
        "Self-checkout is the store hiring you for free."
    );
    assert_eq!(rant["lines"][1]["from"], json!(a));
    assert_eq!(rant["lines"][1]["label"], "Self-checkout v2");
    assert_eq!(
        (rant["lines"][0]["by"].clone(), rant["lines"][0].get("from").cloned()),
        (json!("him"), None),
        "a claim the verbatim gate cannot find in its source is dropped"
    );
    let (_, list) = send(&st, "GET", "/api/topics", Value::Null).await;
    let row = &list["topics"][0];
    assert_eq!(
        row["last_lines"],
        json!(["Nobody asked me.", "Self-checkout is the store hiring you for free."])
    );
    assert_eq!(row["rant_id"], json!(a));
    // a newer rant version without marks shows none, never the older version's
    let (_, _) = send(
        &st,
        "POST",
        &format!("/api/jokes/{a}/versions"),
        json!({"text": "Nobody asked me.\nNew line.\nSelf-checkout is the store hiring you for free.", "source": "rant_write"}),
    )
    .await;
    let (_, p) = send(&st, "GET", &page, Value::Null).await;
    assert!(
        p["rants"][0]["lines"]
            .as_array()
            .unwrap()
            .iter()
            .all(|l| l.get("by").is_none()),
        "{p}"
    );
    // a mark row that does not line up with its version (two marks, three lines) shows nothing either
    let unit3 = p["rants"][0]["unit"].as_str().unwrap().to_string();
    let (s, r) = send(
        &st,
        "PATCH",
        &page,
        json!({"script": {"unit": unit3, "lines": [{"by": "ai"}, {"by": "ai"}]}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let (_, p2) = send(&st, "GET", &page, Value::Null).await;
    let l2 = p2["rants"][0]["lines"].as_array().unwrap();
    assert_eq!(l2.len(), 3);
    assert!(
        l2.iter().all(|l| l.get("by").is_none()),
        "no marks rather than wrong ones: {p2}"
    );

    // a topic of two words: a bit with one of them is not a word match (no model here, so nothing is found)
    let (_, r) = send(&st, "POST", "/api/topics", json!({"text": "self-checkout dinosaurs"})).await;
    let t2 = r["topic"]["id"].as_str().unwrap().to_string();
    let (_, p3) = send(&st, "GET", &format!("/api/topics/{t2}"), Value::Null).await;
    assert_eq!(p3["found"], json!([]), "one word of two is not the topic: {p3}");

    // refusals, each with its words
    let bad = [
        (
            json!({}),
            400,
            "send one of {ranted}, {delete: true}, {undelete: true}, {rename}, {keep}, {drop}, {unmark}, {jot}, {script}",
        ),
        (
            json!({"keep": a, "drop": a}),
            400,
            "send one of {ranted}, {delete: true}, {undelete: true}, {rename}, {keep}, {drop}, {unmark}, {jot}, {script}",
        ),
        (json!({"keep": "unit:x"}), 400, "material is a bit or a taped moment id"),
        (json!({"jot": "perf:p1"}), 400, "material is a bit or a taped moment id"),
        (
            json!({"keep": "joke:nope"}),
            404,
            "joke:nope is not a bit or a taped moment with words",
        ),
        (
            json!({"script": {"unit": "unit:nope", "lines": []}}),
            404,
            "no version unit:nope",
        ),
        (
            json!({"script": {"unit": unit, "lines": [{"by": "bot"}]}}),
            400,
            "by must be him, ai or yours",
        ),
        (json!({"rename": "  "}), 400, "text is required (the topic)"),
        (json!({"undelete": true}), 404, &*format!("no topic {tid}")),
    ];
    for (b, status, msg) in bad {
        let (s, r) = send(&st, "PATCH", &page, b.clone()).await;
        assert_eq!((s.as_u16(), r["error"].as_str().unwrap_or("")), (status, msg), "{b}");
    }

    // rename, delete, undelete: the material comes back
    let (_, r) = send(&st, "PATCH", &page, json!({"rename": "Self-checkout machines"})).await;
    assert_eq!(r["topic"]["text"], "Self-checkout machines");
    let (s, _) = send(&st, "PATCH", &page, json!({"delete": true})).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(&st, "GET", &page, Value::Null).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, r) = send(&st, "PATCH", &page, json!({"undelete": true})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let (_, p) = send(&st, "GET", &page, Value::Null).await;
    assert_eq!(p["kept"][0]["ref"], json!(jot), "Undo keeps the material");

    // Draft the rest: no sources named = Kept; his lines go along fixed
    let (s, r) = send(
        &st,
        "POST",
        "/api/rant/draft",
        json!({"topic": "Self-checkout machines", "topic_id": tid, "keep": ["Nobody asked me."]}),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{r}");
    let jobs = Creative::new(st.clone()).items("jobs").await.unwrap();
    let input = &jobs[r["job"]["id"].as_str().unwrap()]["input"];
    assert_eq!(input["keep"], json!(["Nobody asked me."]));
    let ids: Vec<&str> = input["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["id"].as_str())
        .collect();
    assert_eq!(ids, [jot.as_str(), a.as_str()], "Kept, the jot first: {input}");
    for (b, msg) in [
        (
            json!({"topic": "x", "keep": ["two\nlines"]}),
            "keep must be a list of at most 500 lines of at most 2000 characters",
        ),
        (json!({"topic": "x", "topic_id": "topic:nope"}), "no topic topic:nope"),
    ] {
        let (_, r) = send(&st, "POST", "/api/rant/draft", b.clone()).await;
        assert_eq!(r["error"].as_str().unwrap_or(""), msg, "{b}");
    }

    // Claude on a line: one job with the line, its neighbours and the ask; nothing written to the bit
    let lines = json!(["A", "B", "C", "D", "E", "F", "G", "H"]);
    let (s, r) = send(
        &st,
        "POST",
        "/api/rant/line",
        json!({"joke": a, "lines": lines, "at": 4, "ask": "sharper"}),
    )
    .await;
    assert_eq!((s, r["status"].as_str()), (StatusCode::ACCEPTED, Some("queued")), "{r}");
    let jobs = Creative::new(st.clone()).items("jobs").await.unwrap();
    let j = &jobs[r["job"]["id"].as_str().unwrap()];
    assert_eq!(j["kind"], "rant_line");
    assert_eq!(j["input"]["line"], "E");
    assert_eq!(j["input"]["before"], json!(["B", "C", "D"]));
    assert_eq!(j["input"]["after"], json!(["F", "G", "H"]));
    let (_, again) = send(
        &st,
        "POST",
        "/api/rant/line",
        json!({"joke": a, "lines": lines, "at": 4, "ask": "sharper"}),
    )
    .await;
    assert_eq!(again["job"]["id"], r["job"]["id"], "the same ask is the same job");
    for (b, msg) in [
        (
            json!({"lines": lines, "at": 9, "ask": "sharper"}),
            "at must be the place of a line in lines",
        ),
        (
            json!({"lines": lines, "at": 0, "ask": "funnier"}),
            "ask must be one of sharper, shorter, angrier, bigger, tag",
        ),
        (
            json!({"lines": lines, "at": 0}),
            "ask or say is required (what he wants from the line)",
        ),
        (
            json!({"lines": [], "at": 0, "ask": "tag"}),
            "at must be the place of a line in lines",
        ),
        (
            json!({"lines": lines, "at": 0, "say": "x".repeat(501)}),
            "say at most 500 characters",
        ),
    ] {
        let (s, r) = send(&st, "POST", "/api/rant/line", b.clone()).await;
        assert_eq!((s.as_u16(), r["error"].as_str().unwrap_or("")), (400, msg), "{b}");
    }
}
