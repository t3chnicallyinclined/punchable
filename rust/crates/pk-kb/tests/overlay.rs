//! The AI overlay against the Python it replaces: `rust/tools/gen_overlay_goldens.py` runs `ai_overlay.build_overlay`
//! over synthetic graphs and AI documents; these tests run the same inputs through `pk_kb::overlay` and compare the
//! merged graph and the index as parsed JSON (numbers by value, as Python's `==` does).

use std::path::PathBuf;

use indexmap::IndexMap;
use pk_core::py;
use pk_kb::overlay::{Doc, OverlayFiles, build_overlay};
use serde_json::Value;

fn golden() -> Vec<Value> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/goldens/overlay.json");
    serde_json::from_str(
        &std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("{}: {e} (run python rust/tools/gen_overlay_goldens.py)", p.display())),
    )
    .expect("golden is JSON")
}

fn doc_of(v: &Value) -> Doc {
    Doc {
        doc: v["doc"].clone(),
        source: v["source"].as_str().unwrap_or("").to_string(),
        path: v["path"].as_str().unwrap_or("").to_string(),
    }
}

fn files_of(f: &Value) -> OverlayFiles {
    OverlayFiles {
        id_map: f.get("id_map").filter(|v| !v.is_null()).cloned(),
        inferred: f.get("inferred").filter(|v| !v.is_null()).map(doc_of),
        analyses: f["analyses"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| (p[0].as_str().unwrap_or("").to_string(), doc_of(&p[1])))
            .collect(),
        set_meta: f["set_meta"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<IndexMap<_, _>>(),
    }
}

/// The first path where two JSON values differ (by Python equality).
fn first_diff(rust: &Value, python: &Value, path: &str) -> Option<String> {
    if py::eq(rust, python) {
        return None;
    }
    match (rust, python) {
        (Value::Object(r), Value::Object(p)) => {
            for k in r.keys().chain(p.keys().filter(|k| !r.contains_key(*k))) {
                match (r.get(k), p.get(k)) {
                    (Some(a), Some(b)) => {
                        if let Some(d) = first_diff(a, b, &format!("{path}.{k}")) {
                            return Some(d);
                        }
                    }
                    (a, b) => return Some(format!("{path}.{k}: rust {a:?} python {b:?}")),
                }
            }
            None
        }
        (Value::Array(r), Value::Array(p)) if r.len() == p.len() => r
            .iter()
            .zip(p)
            .enumerate()
            .find_map(|(i, (a, b))| first_diff(a, b, &format!("{path}[{i}]"))),
        (a, b) => Some(format!("{path}: rust {a} python {b}")),
    }
}

#[test]
fn every_case_matches_the_python_overlay() {
    let cases = golden();
    assert!(cases.len() >= 7);
    for c in &cases {
        let name = c["name"].as_str().unwrap_or("?");
        let (merged, index) =
            build_overlay(&c["graph"], &files_of(&c["files"])).unwrap_or_else(|e| panic!("{name}: {e}"));
        if let Some(d) = first_diff(&merged, &c["merged"], "merged") {
            panic!("{name}: {d}");
        }
        let rows: serde_json::Map<String, Value> = index
            .rows
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    Value::Array(
                        v.iter()
                            .map(|(r, p)| Value::Array(vec![r.clone(), Value::String(p.clone())]))
                            .collect(),
                    ),
                )
            })
            .collect();
        let got = serde_json::json!({
            "rows": rows,
            "analysis": index.analysis,
            "inferred": index.inferred,
            "id_map": index.id_map,
        });
        if let Some(d) = first_diff(&got, &c["index"], "index") {
            panic!("{name}: {d}");
        }
    }
}

#[test]
fn the_node_and_edge_order_is_the_pythons() {
    // dict order reaches the `ord` column: a new record lands after the graph's own, in the order it was made
    let cases = golden();
    let c = cases.iter().find(|c| c["name"] == "both").expect("case");
    let (merged, _) = build_overlay(&c["graph"], &files_of(&c["files"])).expect("overlay");
    let ids = |v: &Value, k: &str| -> Vec<String> {
        v[k].as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ids(&merged, "nodes"), ids(&c["merged"], "nodes"));
    assert_eq!(ids(&merged, "edges"), ids(&c["merged"], "edges"));
}
