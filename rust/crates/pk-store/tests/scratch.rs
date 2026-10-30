//! The proof against a scratch `SurrealDB` 3.3.4 (`surreal start memory`, the binary and the root password from the
//! same places `db/sdb.py` reads them). Each test starts its own server and is skipped, with a message, when the
//! binary or the password is missing.
//!
//! The replay test is docs/RUST.md step 1's three-way differential: the `pk-kb` fixture graph goes through
//! `NodeRow`/`EdgeRow`, its decisions through `CREATE decision` (so `fn::apply_decision` applies them), and the
//! snapshot the database holds is compared, field by field, to `pk_kb::View::new`'s replay of the same decisions
//! over the raw graph. `pk-kb` is itself golden against `kb.py`, so this closes the triangle.

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures::StreamExt;
use pk_core::Env;
use pk_kb::{Decision, Rec, View};
use pk_store::{Config, CreativeEvent, NodeRow, Reconcile, Store, StoreError, store::vars};
use serde_json::{Value as Json, json};
use surrealdb::types::{Object, RecordId, Value};
use tokio::time::timeout;

// ---------------------------------------------------------------- the scratch server

struct Scratch {
    child: Child,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn surreal_exe() -> Option<PathBuf> {
    let p = std::env::var_os("PUNCH_SURREAL").map_or_else(
        || {
            std::env::home_dir()
                .unwrap_or_default()
                .join(".punchable")
                .join("bin")
                .join("surreal.exe")
        },
        PathBuf::from,
    );
    p.is_file().then_some(p)
}

fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    l.local_addr().expect("addr").port()
}

/// A skipped test is a failed test where the externals are promised: with `PK_REQUIRE_EXTERNAL` set (CI), a missing
/// binary or password panics instead of skipping (docs/ARCHITECTURE.md section 5).
fn skip(tag: &str, why: &str) {
    assert!(
        std::env::var_os("PK_REQUIRE_EXTERNAL").is_none_or(|v| v.is_empty()),
        "PK_REQUIRE_EXTERNAL is set and the scratch test {tag} cannot run: {why}"
    );
    eprintln!("SKIP {tag}: {why}");
}

/// A scratch server plus a store on `punchable_test/t_<tag>` with the schema applied; `None` (after a message) when
/// the binary or the password is missing and `PK_REQUIRE_EXTERNAL` is unset.
async fn scratch(tag: &str) -> Option<(Scratch, Store)> {
    let Some(exe) = surreal_exe() else {
        skip(tag, "no surreal binary (PUNCH_SURREAL or ~/.punchable/bin/surreal.exe)");
        return None;
    };
    let env = Env::from_process();
    let Some(pass) = env.opt("SURREAL_PASS") else {
        skip(tag, &format!("no SURREAL_PASS in {}", env.place()));
        return None;
    };
    let user = env.opt("SURREAL_USER").unwrap_or("root").to_string();
    let port = free_port();
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
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        if let Some(code) = srv.child.try_wait().expect("try_wait") {
            panic!("surreal exited {code} before listening");
        }
        assert!(Instant::now() < deadline, "surreal did not listen on {port} within 60s");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let cfg = Config::scratch(&env, port, "punchable_test", &format!("t_{tag}")).expect("config");
    let store = Store::connect(cfg).await.expect("connect");
    let v = store.check_version().await.expect("version");
    assert!(v >= (3, 3, 1), "server {v:?}");
    assert!(
        store.apply_schema(false).await.expect("schema"),
        "a fresh database takes the schema"
    );
    Some((srv, store))
}

// ---------------------------------------------------------------- helpers

fn fixture() -> Json {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../pk-kb/tests/goldens/fixture.json");
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
        .expect("fixture is JSON")
}

/// The fixture's graph as both replays get it. One adjustment, applied to both sides: the synthetic `list:tonight`
/// carries `title`, but `list` has no `title` column (`db/schema.surql` defines `name`, `items`, `status`) and the
/// app writes `name` (`api-dev/server.py` `TONIGHT_ID`), so the key is renamed.
fn fixture_graph(f: &Json) -> Json {
    let mut g = f["graph"].clone();
    if let Some(nodes) = g.get_mut("nodes").and_then(Json::as_array_mut) {
        for n in nodes.iter_mut().filter_map(Json::as_object_mut) {
            if n.get("type").and_then(Json::as_str) == Some("list")
                && let Some(t) = n.remove("title")
            {
                n.insert("name".into(), t);
            }
        }
    }
    g
}

fn rec(v: Json) -> Rec {
    match v {
        Json::Object(m) => m,
        other => panic!("not an object: {other}"),
    }
}

fn obj(v: Value) -> Object {
    match v {
        Value::Object(o) => o,
        other => panic!("not an object: {other:?}"),
    }
}

/// Every path where two JSON values differ (numbers within 1e-9).
fn diffs(db: &Json, kb: &Json, path: &str, out: &mut Vec<String>) {
    match (db, kb) {
        (Json::Object(dbo), Json::Object(kbo)) => {
            for key in dbo.keys().chain(kbo.keys().filter(|k| !dbo.contains_key(*k))) {
                match (dbo.get(key), kbo.get(key)) {
                    (Some(dv), Some(kv)) => diffs(dv, kv, &format!("{path}.{key}"), out),
                    (Some(dv), None) => out.push(format!("{path}.{key}: db has {dv}, kb.py replay has nothing")),
                    (None, Some(kv)) => out.push(format!("{path}.{key}: db has nothing, kb.py replay has {kv}")),
                    (None, None) => {}
                }
            }
        }
        (Json::Array(dba), Json::Array(kba)) => {
            if dba.len() != kba.len() {
                out.push(format!(
                    "{path}: db has {} items, kb.py replay has {}",
                    dba.len(),
                    kba.len()
                ));
            }
            for (i, (dv, kv)) in dba.iter().zip(kba).enumerate() {
                diffs(dv, kv, &format!("{path}[{i}]"), out);
            }
        }
        (Json::Number(dn), Json::Number(kn)) => {
            let (fd, fk) = (dn.as_f64().unwrap_or(f64::NAN), kn.as_f64().unwrap_or(f64::NAN));
            if (fd - fk).abs() > 1e-9 {
                out.push(format!("{path}: db {dn}, kb.py replay {kn}"));
            }
        }
        _ => {
            if db != kb {
                out.push(format!("{path}: db {db}, kb.py replay {kb}"));
            }
        }
    }
}

fn blob_decision(n: usize) -> Decision {
    Decision(rec(json!({
        "ts": format!("2026-10-29T12:00:0{n}+00:00"), "target": "blob", "id": "blob:x", "action": "add",
        "before": null, "after": {"n": n}, "reason": "ignored by both replays", "by": "owner"
    })))
}

// ---------------------------------------------------------------- tests

#[tokio::test]
async fn record_looking_strings_stay_strings() {
    let Some((_srv, store)) = scratch("strings").await else {
        return;
    };
    let n = rec(json!({
        "id": "perf:p1", "type": "performance", "set": "set:s1", "start_ms": 1,
        "text": "note: see doc", "name": "2015-02-18", "kind": "bit", "source": "joke:zzz"
    }));
    let row = NodeRow::from_node(&n, 0).unwrap();
    store
        .write(
            "CREATE performance CONTENT $row RETURN id;",
            vars([("row", row.content())]),
            Some(1),
        )
        .await
        .unwrap();
    let raw = obj(store
        .last("SELECT * FROM ONLY performance:p1;", Object::new())
        .await
        .unwrap());
    assert_eq!(
        raw.get("text"),
        Some(&Value::String("note: see doc".into())),
        "a colon does not make a record id"
    );
    assert_eq!(
        raw.get("name"),
        Some(&Value::String("2015-02-18".into())),
        "a date-looking string stays text"
    );
    assert_eq!(raw.get("source"), Some(&Value::String("joke:zzz".into())));
    assert_eq!(
        raw.get("set"),
        Some(&Value::RecordId(RecordId::new("set", "s1"))),
        "set is typed on purpose"
    );
    let back = NodeRow::from_record(Value::Object(raw)).unwrap().into_node();
    assert_eq!(Json::Object(back), Json::Object(n));
}

#[tokio::test]
async fn none_and_null_stay_distinct() {
    let Some((_srv, store)) = scratch("nonenull").await else {
        return;
    };
    // a node: `summary: null` is NULL, `first_seen` absent is NONE
    let n = rec(json!({"id": "joke:n", "type": "joke", "title": "t", "stage": "idea", "summary": null}));
    let row = NodeRow::from_node(&n, 0).unwrap();
    store
        .write(
            "CREATE joke CONTENT $row RETURN id;",
            vars([("row", row.content())]),
            Some(1),
        )
        .await
        .unwrap();
    let raw = obj(store.last("SELECT * FROM ONLY joke:n;", Object::new()).await.unwrap());
    assert_eq!(raw.get("summary"), Some(&Value::Null));
    assert!(
        raw.get("first_seen").is_none_or(Value::is_none),
        "absent stays absent: {:?}",
        raw.get("first_seen")
    );
    let back = NodeRow::from_record(Value::Object(raw)).unwrap().into_node();
    assert_eq!(back.get("summary"), Some(&Json::Null));
    assert!(!back.contains_key("first_seen"));
    assert_eq!(Json::Object(back), Json::Object(n));
    // two decision lines that differ only in `undo_of: null` vs no `undo_of`, through the row and the event
    let a = blob_decision(0);
    let mut b = blob_decision(1);
    b.0.insert("undo_of".into(), Json::Null);
    store.append_decision(&a, Some(0)).await.unwrap();
    store.append_decision(&b, Some(1)).await.unwrap();
    let snap = store.fetch_snapshot().await.unwrap();
    assert_eq!(json!(snap.decisions), json!([a, b]));
    assert!(!snap.decisions[0].has_undo_of());
    assert!(snap.decisions[1].has_undo_of());
}

#[tokio::test]
async fn db_replay_matches_pk_kb_replay() {
    let Some((_srv, store)) = scratch("replay").await else {
        return;
    };
    let f = fixture();
    let graph = &fixture_graph(&f);
    let all: Vec<Decision> = serde_json::from_value(f["decisions"].clone()).unwrap();
    let report: Vec<String> = serde_json::from_value(f["report"].clone()).unwrap();
    let (nn, ne) = store.load_graph(graph).await.unwrap();
    assert_eq!((nn, ne), (13, 11), "the fixture's valid nodes and edges");
    // The fixture is a decisions.jsonl as kb.py's FILE replay tolerates it; the hosted path only appends what
    // `kb._build` made, so two of its lines can never reach the database and the schema refuses them as errors, never
    // rows: 10 (an edge edit with `status: null`; `_build` always writes `status: "confirmed"` on an edit and an undo's
    // `after` is the full edge snapshot) and 12 (a joke with no title; `title TYPE string`). Both replays then run
    // over what the database took.
    let mut decisions = Vec::new();
    let mut refused = Vec::new();
    for (i, d) in all.iter().enumerate() {
        match store.append_decision(d, Some(decisions.len())).await {
            Ok(a) => {
                assert_eq!(a.seq, i64::try_from(decisions.len()).unwrap());
                if d.target() == Some("node") && d.action() != Some("delete") {
                    assert!(a.record.is_some(), "decision {i} should read back its node");
                }
                decisions.push(d.clone());
            }
            Err(e) => {
                assert!(matches!(e, StoreError::Statement { .. }), "decision {i}: {e}");
                refused.push(i);
            }
        }
    }
    assert_eq!(refused, vec![10, 12], "the two lines the hosted path cannot make");
    let snap = store.fetch_snapshot().await.unwrap();
    assert_eq!(
        json!(snap.decisions),
        json!(decisions),
        "decision rows round-trip as the lines"
    );
    assert_eq!(
        snap.generation,
        Some(i64::try_from(decisions.len()).unwrap()),
        "gen moves once per decision"
    );
    let db_view = snap.view(report.clone(), "owner");
    let kb_view = View::new(graph, decisions, report, "owner");
    let mut out = Vec::new();
    diffs(&json!(db_view.nodes), &json!(kb_view.nodes), "nodes", &mut out);
    diffs(&json!(db_view.edges), &json!(kb_view.edges), "edges", &mut out);
    diffs(&json!(db_view.alias), &json!(kb_view.alias), "alias", &mut out);
    diffs(&json!(snap.orphans), &json!(kb_view.orphans), "orphans", &mut out);
    assert!(
        out.is_empty(),
        "fn::apply_decision (SurrealDB) and kb.py's replay (pk-kb) disagree:\n  {}",
        out.join("\n  ")
    );
}

#[tokio::test]
async fn denied_writes_are_errors_never_silence() {
    let Some((_srv, store)) = scratch("denied").await else {
        return;
    };
    // an ASSERT on a field
    let e = store
        .write(
            "CREATE joke:bad CONTENT { ord: 0, title: 'x', stage: 'flying' } RETURN id;",
            Object::new(),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(e, StoreError::Statement { index: 0, .. }), "{e}");
    // a field the SCHEMAFULL table does not have
    let e = store
        .write(
            "CREATE joke:bad CONTENT { ord: 0, title: 'x', stage: 'idea', nope: 1 } RETURN id;",
            Object::new(),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(e, StoreError::Statement { .. }), "{e}");
    // a table the STRICT database does not have
    let e = store
        .write("CREATE nothing:x CONTENT { a: 1 } RETURN id;", Object::new(), None)
        .await
        .unwrap_err();
    assert!(matches!(e, StoreError::Statement { .. }), "{e}");
    // a target that does not exist: OK [] is a write that wrote nothing
    let e = store
        .write("UPDATE joke:absent SET title = 'x' RETURN AFTER;", Object::new(), None)
        .await
        .unwrap_err();
    assert!(matches!(e, StoreError::EmptyWrite { .. }), "{e}");
    assert_eq!(
        store.last("SELECT * FROM joke;", Object::new()).await.unwrap(),
        Value::Array(Vec::<Value>::new().into())
    );
    // a decision the event cannot apply rolls the whole transaction back: no row, no change
    store
        .write(
            "CREATE joke:j CONTENT { ord: 0, title: 't', stage: 'idea' } RETURN id;",
            Object::new(),
            Some(1),
        )
        .await
        .unwrap();
    let d = Decision(rec(
        json!({"target": "node", "id": "joke:j", "action": "edit", "before": {"stage": "idea"},
        "after": {"stage": "flying"}, "reason": "", "by": "owner"}),
    ));
    let e = store.append_decision(&d, Some(0)).await.unwrap_err();
    assert!(matches!(e, StoreError::Statement { .. }), "{e}");
    assert_eq!(
        store.decision_count().await.unwrap(),
        0,
        "the decision row was rolled back with its effect"
    );
    let j = obj(store.last("SELECT * FROM ONLY joke:j;", Object::new()).await.unwrap());
    assert_eq!(j.get("stage"), Some(&Value::String("idea".into())));
    // an id with an unknown prefix: fn::rid throws
    let d = Decision(rec(
        json!({"target": "node", "id": "blob:x", "action": "edit", "after": {"title": "y"}, "by": "owner"}),
    ));
    assert!(store.append_decision(&d, Some(0)).await.is_err());
    // out of step with the file: refused before the database sees it
    let e = store.append_decision(&blob_decision(0), Some(3)).await.unwrap_err();
    assert!(matches!(e, StoreError::Refused { status: 409, .. }), "{e}");
    assert_eq!(store.decision_count().await.unwrap(), 0);
}

#[tokio::test]
async fn missing_changefeed_is_detected_and_repaired() {
    let Some((_srv, store)) = scratch("feeds").await else {
        return;
    };
    assert!(
        store
            .check_changefeeds(&pk_store::CHANGEFEED_TABLES)
            .await
            .unwrap()
            .is_empty()
    );
    // trap T6: a redefinition without the clause drops the feed silently
    store
        .query("DEFINE TABLE OVERWRITE list SCHEMAFULL;", Object::new())
        .await
        .unwrap();
    let missing = store.check_changefeeds(&pk_store::CHANGEFEED_TABLES).await.unwrap();
    assert_eq!(missing.keys().cloned().collect::<Vec<_>>(), vec!["list".to_string()]);
    let missing = store.check_changefeeds(&["list", "ghost"]).await.unwrap();
    assert_eq!(missing.get("ghost").map(String::as_str), Some("(table missing)"));
    // an unchanged schema is not re-applied, unless a feed is missing: then it is, and the feed comes back
    assert!(store.apply_schema(false).await.unwrap());
    assert!(
        store
            .check_changefeeds(&pk_store::CHANGEFEED_TABLES)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        !store.apply_schema(false).await.unwrap(),
        "same sha, feeds present: skipped"
    );
    assert!(!store.apply_extra_schema(false).await.unwrap());
    assert!(store.apply_schema(true).await.unwrap(), "force applies");
    let st = store.meta_state().await.unwrap().unwrap();
    assert_eq!((st.schema_sha, st.extra_sha), {
        let (a, b) = pk_store::schema_shas();
        (Some(a), Some(b))
    });
}

#[tokio::test]
async fn creative_event_mirror_and_reconcile() {
    let Some((_srv, store)) = scratch("creative").await else {
        return;
    };
    let line = |i: usize| {
        format!(
            r#"{{"ts": "2026-10-29T00:00:0{i}+00:00", "op": "put", "id": "sb{i}", "by": "owner", "data": {{"n": {i}}}}}"#
        )
    };
    let ev0 = CreativeEvent::new("storyboards", 0, &line(0)).unwrap();
    let ev1 = CreativeEvent::new("storyboards", 1, &line(1)).unwrap();
    store.create_creative_event(&ev0).await.unwrap();
    store.create_creative_event(&ev1).await.unwrap();
    // the unique (store, seq) guard
    let dup = CreativeEvent::new("storyboards", 1, &line(9)).unwrap();
    let e = store.create_creative_event(&dup).await.unwrap_err();
    assert!(matches!(e, StoreError::Statement { .. }), "{e}");
    assert_eq!(
        store.creative_lines("storyboards").await.unwrap(),
        vec![line(0), line(1)]
    );
    assert!(store.creative_lines("workingsets").await.unwrap().is_empty());
    // the row carries what the LIVE consumer reads
    let raw = store
        .last(
            "SELECT * FROM ONLY creative_event WHERE store = 'storyboards' AND seq = 0 LIMIT 1;",
            Object::new(),
        )
        .await
        .unwrap();
    assert_eq!(CreativeEvent::from_record(raw).unwrap(), ev0);
    // reconcile: equal (as parsed JSON, not bytes), file ahead, db ahead, diverged
    let compact = |i: usize| serde_json::to_string(&serde_json::from_str::<Json>(&line(i)).unwrap()).unwrap();
    let r = store
        .reconcile_creative("storyboards", &[compact(0), compact(1)])
        .await
        .unwrap();
    assert_eq!(r.outcome, Reconcile::Equal);
    let r = store
        .reconcile_creative("storyboards", &[line(0), line(1), line(2)])
        .await
        .unwrap();
    assert_eq!(r.outcome, Reconcile::FileAhead { copied: 1 });
    assert_eq!(store.creative_lines("storyboards").await.unwrap().len(), 3);
    let r = store.reconcile_creative("storyboards", &[line(0)]).await.unwrap();
    assert_eq!(
        r.outcome,
        Reconcile::DbAhead {
            tail: vec![line(1), line(2)]
        }
    );
    assert_eq!(r.lines.len(), 3);
    let r = store
        .reconcile_creative("storyboards", &[line(0), line(7)])
        .await
        .unwrap();
    match r.outcome {
        Reconcile::Diverged { at, file, db, msg } => {
            assert_eq!((at, file, db), (1, 2, 3));
            assert_eq!(
                msg,
                "storyboards: storyboards.jsonl and the database differ from event 1 (2 lines in the file, 3 in the database)"
            );
        }
        other => panic!("{other:?}"),
    }
    // a gap is an error, not a shorter list
    store
        .create_creative_event(&CreativeEvent::new("venues", 1, &line(1)).unwrap())
        .await
        .unwrap();
    assert!(matches!(
        store.creative_lines("venues").await.unwrap_err(),
        StoreError::Gap { .. }
    ));
}

#[tokio::test]
async fn live_changes_arrive_for_decision_meta_and_creative_event() {
    let Some((_srv, store)) = scratch("live").await else {
        return;
    };
    let mut changes = store.subscribe().await.unwrap();
    store.append_decision(&blob_decision(0), Some(0)).await.unwrap();
    let ev = CreativeEvent::new(
        "storyboards",
        0,
        r#"{"ts":"t","op":"put","id":"sb1","by":"owner","data":{}}"#,
    )
    .unwrap();
    store.create_creative_event(&ev).await.unwrap();
    let (mut decision, mut meta, mut event) = (None, None, None);
    let deadline = Instant::now() + Duration::from_secs(15);
    while (decision.is_none() || meta.is_none() || event.is_none()) && Instant::now() < deadline {
        match timeout(deadline - Instant::now(), changes.next()).await {
            Ok(Some(Ok(c))) => match c.table.as_deref() {
                Some("decision") => decision = Some(c),
                Some("meta") => meta = Some(c),
                Some("creative_event") => event = Some(c),
                _ => {}
            },
            Ok(Some(Err(e))) => panic!("live: {e}"),
            Ok(None) | Err(_) => break,
        }
    }
    let d = decision.expect("a decision change");
    assert_eq!(d.action, "create");
    assert!(d.id.as_deref().is_some_and(|s| s.starts_with("decision:")));
    let m = meta.expect("a meta change");
    assert_eq!(
        (m.action.as_str(), m.id.as_deref(), m.generation),
        ("update", Some("meta:state"), Some(1))
    );
    let e = event.expect("a creative_event change");
    assert_eq!(
        (e.store.as_deref(), e.op.as_deref(), e.item.as_deref()),
        (Some("storyboards"), Some("put"), Some("sb1"))
    );
}
