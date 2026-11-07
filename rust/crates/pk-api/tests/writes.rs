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
            "send {ranted: <the rant's joke id>} or {delete: true}",
        ),
        (
            format!("/api/topics/{tid}"),
            json!({"ranted": "unit:x"}),
            400,
            "send {ranted: <the rant's joke id>} or {delete: true}",
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
