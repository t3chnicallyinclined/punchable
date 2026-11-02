//! Freshness under concurrency, against a scratch `SurrealDB` 3.3.4 (`surreal start memory`; skipped with a message
//! when the binary or the root password is missing, a failure with `PK_REQUIRE_EXTERNAL` set, as pk-store's).
//!
//! The rule (docs/ARCHITECTURE.md section 2, `AppState::data`): a read that starts after a write has returned sees
//! that write, while readers hammer the same state; generations never go backwards for a reader; rebuilds are
//! coalesced (bounded by the writes, not by the readers); the views are built lazily and once per generation.

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pk_api::{ApiConfig, AppState};
use pk_core::Env;
use pk_kb::Decision;
use pk_store::{Config, Store, to_db, vars};
use serde_json::{Value, json};

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
        "PK_REQUIRE_EXTERNAL is set and the freshness test cannot run: {why}"
    );
    eprintln!("SKIP freshness: {why}");
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

fn edit(id: &str, n: usize) -> Decision {
    let Value::Object(m) = json!({
        "ts": format!("2026-11-02T12:{:02}:{:02}+00:00", n / 60, n % 60), "target": "node", "id": id,
        "action": "edit", "before": {}, "after": {"title": format!("version {n}")}, "by": "owner",
    }) else {
        unreachable!()
    };
    Decision(m)
}

const WRITES: usize = 40;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_read_after_a_write_sees_it_while_readers_hammer() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let cfg = Config::scratch(&env, port, "punchable_test", "t_fresh").expect("config");
    let writer = Store::connect(cfg.clone()).await.expect("connect");
    assert!(writer.apply_schema(false).await.expect("schema"));
    let g = graph();
    writer.load_graph(&g).await.expect("load");
    writer
        .write(
            "UPDATE meta:state MERGE { gen: 0, loading: false, load_info: { bundle: $b } } RETURN id;",
            vars([("b", to_db(&json!({"graph": "graph.json", "sha": "test"})))]),
            None,
        )
        .await
        .expect("bundle");
    let joke = g["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["type"] == "joke")
        .and_then(|n| n["id"].as_str())
        .expect("a joke")
        .to_string();

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
    let first = st.data().await.expect("first load");
    assert_eq!(first.decisions().len(), 0);
    assert_eq!(st.builds(), 1);

    // readers: as fast as they can, each checks its generations never go backwards and builds the index sometimes
    let stop = Arc::new(AtomicBool::new(false));
    let mut readers = Vec::new();
    for r in 0..8 {
        let (st, stop) = (st.clone(), Arc::clone(&stop));
        readers.push(tokio::spawn(async move {
            let mut last = 0usize;
            let mut n = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let d = st.data().await.expect("a read during writes");
                let seen = d.decisions().len();
                assert!(seen >= last, "reader {r} went back from {last} to {seen} decisions");
                last = seen;
                if n.is_multiple_of(4) {
                    let idx = d.index(false).await.expect("index");
                    assert_eq!(
                        idx.view.decisions.len(),
                        seen,
                        "the index is built over its own generation"
                    );
                }
                n += 1;
            }
            n
        }));
    }

    // the writer: every write is read back at once, through the same state the readers use
    let mut worst = Duration::ZERO;
    for i in 0..WRITES {
        writer.append_decision(&edit(&joke, i), None).await.expect("write");
        let t = Instant::now();
        let d = st.data().await.expect("read after write");
        let v = d.view(false).await.expect("view");
        worst = worst.max(t.elapsed());
        assert_eq!(d.decisions().len(), i + 1, "the read right after write {i} sees it");
        assert_eq!(
            v.nodes.get(&joke).and_then(|n| n.get("title")),
            Some(&json!(format!("version {i}"))),
            "the view right after write {i} has the edit"
        );
    }
    stop.store(true, Ordering::Relaxed);
    let mut reads = 0;
    for r in readers {
        reads += r.await.expect("reader");
    }
    // one build per generation at most (plus the first load), however many readers waited
    let builds = st.builds();
    assert!(builds <= WRITES as u64 + 1, "{builds} builds for {WRITES} writes");
    assert!(reads > WRITES as u64, "the readers ran ({reads} reads)");
    // coalesced: the readers that queued behind a rebuild are answered by it or by the next one, they do not each
    // start one (at most the writer's and one reader's per write, plus the first load)
    let started = st.rebuild_reads();
    assert!(
        started <= 2 * WRITES as u64 + 2,
        "{started} rebuilds started for {WRITES} writes"
    );
    eprintln!(
        "freshness: {WRITES} writes, {reads} concurrent reads, {builds} builds, {started} rebuilds started, \
         worst read-after-write {worst:?}"
    );

    // the views are lazy and built once per generation: the same Arc twice
    let d = st.data().await.expect("data");
    let (a, b) = (d.view(false).await.expect("view"), d.view(false).await.expect("view"));
    assert!(Arc::ptr_eq(&a, &b));
    let (a, b) = (
        d.index(false).await.expect("index"),
        d.index(true).await.expect("index"),
    );
    assert!(Arc::ptr_eq(&a.view, &d.view(false).await.expect("view")));
    // nothing hidden in the fixture: the whole view is the visible one
    assert!(Arc::ptr_eq(&a, &b));
}
