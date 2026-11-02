//! The one-shot Ask's halves with no model call against the Python's own (`rust/tools/gen_ask_goldens.py`, run
//! in-process over the synthetic notebook): the sources `engine/ask.py` `Corpus` builds (id, kind, text, date, title,
//! joke, in order), the ids `SQLite`'s FTS5 + bm25 returns for each search (through `pk_fts` and `oneshot::fts_expr`),
//! and `engine/verify.py`'s `strip_unverified_inline_quotes` / `normalize_ids`. Red arms: a bm25 order reversed, a
//! source dropped and a quote check skipped each fail the comparison.

use std::collections::BTreeMap;

use pk_creative::oneshot::{AskCorpus, Kb, fts_expr};
use pk_creative::verbatim::{normalize_ids, strip_unverified_inline_quotes};
use serde_json::{Value, json};

fn golden() -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
        .join("ask_oneshot.json");
    serde_json::from_str(&std::fs::read_to_string(p).expect("run rust/tools/gen_ask_goldens.py")).expect("JSON")
}

fn built(g: &Value) -> (Kb, AskCorpus) {
    let files: BTreeMap<String, String> = g["files"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
        .collect();
    let graph: Value = serde_json::from_str(&files["kb/v0/graph.json"]).unwrap();
    let kb = Kb::new(&graph, g["decisions"].as_array().unwrap(), g["owner"].as_str().unwrap());
    let c = AskCorpus::build(&kb, &files).unwrap();
    (kb, c)
}

fn docs_value(c: &AskCorpus) -> Value {
    Value::Array(
        c.docs
            .values()
            .map(|d| {
                json!({"id": d.id, "kind": d.kind.name(), "text": d.text, "date": d.date, "title": d.title,
                       "joke": d.joke})
            })
            .collect(),
    )
}

fn search(idx: &pk_fts::Index, q: &str) -> Vec<String> {
    fts_expr(q).map_or_else(Vec::new, |e| idx.search(&e, 50).unwrap_or_default())
}

#[test]
fn the_sources_are_the_pythons_in_its_order() {
    let g = golden();
    let (kb, c) = built(&g);
    assert_eq!(docs_value(&c), g["docs"]);
    let labels: Vec<Value> = kb
        .labels()
        .into_iter()
        .map(|(id, name)| json!([id, name.unwrap_or(json!(""))]))
        .collect();
    assert_eq!(Value::Array(labels), g["labels"]);
    let mut performed: Vec<&String> = kb.performed.iter().collect();
    performed.sort();
    assert_eq!(json!(performed), g["performed"]);
}

#[test]
fn a_dropped_source_is_caught() {
    let g = golden();
    let (_, mut c) = built(&g);
    c.docs.shift_remove_index(3);
    assert_ne!(docs_value(&c), g["docs"]);
}

#[test]
fn bm25_orders_as_the_pythons_sqlite() {
    let g = golden();
    let (_, c) = built(&g);
    let idx = pk_fts::Index::build(c.fts_rows()).unwrap();
    let mut ordered = 0;
    for s in g["searches"].as_array().unwrap() {
        let q = s["q"].as_str().unwrap();
        let got = search(&idx, q);
        assert_eq!(json!(got), s["hits"], "{q}");
        // the red arm: the same hits in another order are not the Python's
        if got.len() > 1 {
            let mut rev = got.clone();
            rev.reverse();
            assert_ne!(json!(rev), s["hits"], "{q}: the comparison must see the order");
            ordered += 1;
        }
    }
    assert!(ordered >= 5, "enough searches with more than one hit to pin the order");
    eprintln!(
        "pk-fts runs SQLite {} (the goldens: {})",
        pk_fts::sqlite_version(),
        g["sqlite"]
    );
}

/// Step 4 alone over the Python worker's own run (`scenario_ask_cited_good_and_bad`): the same sources and the same
/// model answer give the same citations, removals and answer. The red arm: the citations as the model wrote them
/// (the gate skipped) are not the Python's.
#[test]
fn verify_is_the_pythons_and_a_skipped_check_is_caught() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
        .join("scenario_ask_cited_good_and_bad.json");
    let sc: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let files: BTreeMap<String, String> = sc["notebook_files"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
        .collect();
    let graph: Value = serde_json::from_str(&files["kb/v0/graph.json"]).unwrap();
    let kb = Kb::new(&graph, sc["notebook_decisions"].as_array().unwrap(), "owner");
    let c = AskCorpus::build(&kb, &files).unwrap();
    let result = sc["expect"]["api"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|r| r["path"].as_str().is_some_and(|x| x.ends_with("/result")))
        .map(|r| r["body"]["result"].clone())
        .unwrap();
    let picked: Vec<String> = serde_json::from_value(result["sources"].clone()).unwrap();
    let data: Value = serde_json::from_str(sc["model"][1]["text"].as_str().unwrap()).unwrap();
    let v = pk_creative::oneshot::verify(&c, &kb, &picked, &data);
    assert_eq!(json!(v.citations), result["citations"]);
    assert_eq!(json!(v.removed), result["removed"]);
    assert_eq!(json!(v.answer), result["answer"]);
    assert_eq!(json!(v.found), result["found"]);
    assert!(
        v.removed.len() >= 4,
        "a bad quote, an unretrieved source, a prose quote, an unretrieved id"
    );
    let unchecked: Vec<Value> = data["citations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| json!({"source_id": x["source_id"], "quote": x["quote"], "note": x["note"]}))
        .collect();
    assert_ne!(json!(unchecked), result["citations"]);
}

#[test]
fn the_verify_helpers_are_the_pythons() {
    let g = golden();
    let (_, c) = built(&g);
    let texts: Vec<&str> = c.docs.values().map(|d| d.text.as_str()).collect();
    for case in g["inline"].as_array().unwrap() {
        let (out, flags) = strip_unverified_inline_quotes(
            case["text"].as_str().unwrap(),
            &texts,
            usize::try_from(case["min_words"].as_u64().unwrap()).unwrap(),
        );
        assert_eq!(json!([out, flags]), case["out"], "{}", case["text"]);
    }
    // the red arm: with no sources to check against, a true quote would be cut too
    let first = &g["inline"][0];
    let (out, _) = strip_unverified_inline_quotes(first["text"].as_str().unwrap(), &[] as &[&str], 4);
    assert_ne!(json!(out), first["out"][0]);
    for pair in g["ids"].as_array().unwrap() {
        assert_eq!(normalize_ids(pair[0].as_str().unwrap()), pair[1].as_str().unwrap());
    }
}
