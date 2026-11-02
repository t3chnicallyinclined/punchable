//! The engine's store against a scratch `SurrealDB` 3.3.4 (`surreal start memory`, the binary and password from the
//! same places `tests/scratch.rs` reads them; skipped with a message when missing, a failure under
//! `PK_REQUIRE_EXTERNAL`). Proves, on a real server: the read over the synthetic `pk-kb` fixture graph, a lesson
//! written through the evidence rule and read back as its record, and every guard of `pk-engine/schema/engine.surql`
//! (append-only rows, READONLY fields, the config hash, a rollback bound to a snapshot, the learning mode raised only
//! by a signed row, a CONFIRMED lesson without evidence refused).

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use pk_core::Env;
use pk_engine::fact::{FactIn, Grade, Kind, QuoteOf};
use pk_engine::rank_config::RankConfig;
use pk_engine::{EngineStore, ReadQuery};
use pk_store::{Config, Store, store::vars};
use serde_json::Value as Json;
use surrealdb::types::{Object, Value};

struct Scratch {
    child: Child,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn skip(why: &str) {
    assert!(
        std::env::var_os("PK_REQUIRE_EXTERNAL").is_none_or(|v| v.is_empty()),
        "PK_REQUIRE_EXTERNAL is set and the engine scratch test cannot run: {why}"
    );
    eprintln!("SKIP engine_scratch: {why}");
}

async fn scratch() -> Option<(Scratch, Store)> {
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
    let Some(pass) = env.opt("SURREAL_PASS") else {
        skip(&format!("no SURREAL_PASS in {}", env.place()));
        return None;
    };
    let user = env.opt("SURREAL_USER").unwrap_or("root").to_string();
    let port = TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
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
        .env("SURREAL_PASS", pass)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("start {}: {e}", exe.display()));
    let mut srv = Scratch { child };
    let deadline = Instant::now() + Duration::from_secs(60);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        if let Some(code) = srv.child.try_wait().expect("try_wait") {
            panic!("surreal exited {code} before listening");
        }
        assert!(Instant::now() < deadline, "surreal did not listen on {port} within 60s");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let cfg = Config::scratch(&env, port, "punchable_test", "t_engine").expect("config");
    let store = Store::connect(cfg).await.expect("connect");
    assert!(store.apply_schema(false).await.expect("schema"));
    store.apply_engine_schema().await.expect("the engine schema applies");
    Some((srv, store))
}

fn fixture() -> Json {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../pk-kb/tests/goldens/fixture.json");
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
        .expect("fixture json")
}

async fn refused(store: &Store, sql: &str) -> String {
    match store.query(sql, Object::new()).await {
        Ok(v) => panic!("{sql}\nwas accepted: {v:?}"),
        Err(e) => e.to_string(),
    }
}

#[tokio::test]
async fn the_engine_reads_his_material_writes_lessons_and_its_guards_hold() {
    let Some((_srv, store)) = scratch().await else { return };
    // The pk-kb fixture graph, as tests/scratch.rs loads it (a list's `title` is the database's `name`).
    let mut graph = fixture()["graph"].clone();
    for n in graph
        .get_mut("nodes")
        .and_then(Json::as_array_mut)
        .into_iter()
        .flatten()
        .filter_map(Json::as_object_mut)
    {
        if n.get("type").and_then(Json::as_str) == Some("list")
            && let Some(t) = n.remove("title")
        {
            n.insert("name".into(), t);
        }
    }
    store.load_graph(&graph).await.expect("fixture loads");

    // A unit of the fixture with at least four words: its words read it back.
    let units = pk_store::from_db(
        store
            .last(
                "SELECT id, text FROM unit WHERE hidden != true AND string::len(text) > 30 LIMIT 1;",
                Object::new(),
            )
            .await
            .expect("a unit"),
    );
    let unit = &units[0];
    let (uid, text) = (
        unit["id"].as_str().expect("id").to_owned(),
        unit["text"].as_str().expect("text").to_owned(),
    );
    let words: Vec<&str> = text.split_whitespace().take(4).collect();
    let question = words.join(" ");
    let read = store
        .read(ReadQuery {
            question: &question,
            vector: None,
            depth: 60,
        })
        .await
        .expect("read");
    assert!(
        read.hits.iter().any(|h| h.key == uid && h.text_rank.is_some()),
        "{uid} not read for {question:?}"
    );
    let corpus = store.corpus_size().await.expect("corpus size");
    assert!(corpus >= read.hits.len() && corpus > 0, "{corpus}");

    // A lesson through the evidence rule: the record's own words; read back as that record.
    let lesson = FactIn {
        kind: Kind::Fact,
        text: format!("He opens this bit with {question}"),
        confidence: Grade::Confirmed,
        source: "engine_scratch".into(),
        by: "test".into(),
        owner_statement: None,
        quote: Some(QuoteOf {
            record: uid.clone(),
            text: question.clone(),
        }),
    };
    let id = store.add_lesson(&lesson, false).await.expect("a quoted lesson is kept");
    assert!(id.starts_with("lesson:"), "{id}");
    let got = store.lessons(&question, 5).await.expect("lessons");
    assert!(got.iter().any(|h| h.key == uid && h.fact.is_some()), "{got:?}");
    // A paraphrase of his words is refused before the database sees it.
    let mut para = lesson.clone();
    para.quote = Some(QuoteOf {
        record: uid.clone(),
        text: "words he never said at all".into(),
    });
    assert!(store.add_lesson(&para, true).await.is_err());

    // The guards, on the real server.
    let e = refused(&store, &format!("DELETE {id};")).await;
    assert!(e.contains("retired, never deleted"), "{e}");
    let e = refused(&store, &format!("UPDATE {id} SET text = 'edited';")).await;
    assert!(e.to_lowercase().contains("readonly") || e.contains("read only"), "{e}");
    refused(
        &store,
        "CREATE lesson SET text = 'x', kind = 'fact', grade = 'CONFIRMED', source = 's', by = 'b', owner_statement = 'said so';",
    )
    .await;
    let cfg = RankConfig::default();
    let canon = cfg.canonical_json();
    let ok = format!(
        "CREATE type::record('rank_config', '{}') SET canonical = $c, origin = 'default';",
        cfg.sha256()
    );
    store
        .query(&ok, vars([("c", Value::String(canon.clone()))]))
        .await
        .expect("a config named by its hash");
    let bad = "CREATE type::record('rank_config', 'notitshash') SET canonical = $c, origin = 'default';";
    assert!(
        store.query(bad, vars([("c", Value::String(canon))])).await.is_err(),
        "a config not named by its hash"
    );
    let to = format!("type::record('rank_config', '{}')", cfg.sha256());
    refused(
        &store,
        &format!("CREATE rank_switch SET to_cfg = {to}, reason = 'rollback', by = 'x';"),
    )
    .await;
    refused(
        &store,
        &format!("CREATE rank_switch SET to_cfg = {to}, reason = 'promote', by = 'x';"),
    )
    .await;
    store
        .query(
            &format!("CREATE rank_switch SET to_cfg = {to}, reason = 'promote', by = 'owner', owner_statement = 'go';"),
            Object::new(),
        )
        .await
        .expect("a promote with his statement");
    let e = refused(&store, "DELETE rank_switch;").await;
    assert!(e.contains("append-only"), "{e}");
    // Learning: from shadow, raising needs a signed row with his statement; lowering is open.
    refused(
        &store,
        "CREATE learning_mode SET mode = 'on', by = 'worker', owner_statement = 'go';",
    )
    .await;
    store
        .query("CREATE learning_mode SET mode = 'off', by = 'worker';", Object::new())
        .await
        .expect("lowering is open");
    store
        .query(
            "CREATE learning_mode SET mode = 'on', by = 'app', signed = true, owner_statement = 'go';",
            Object::new(),
        )
        .await
        .expect("he raises it");
}
