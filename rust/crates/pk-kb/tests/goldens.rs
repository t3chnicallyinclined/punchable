//! The differential against the Python it replaces: `rust/tools/gen_kb_goldens.py` replays a synthetic graph and
//! decision list through `tools/explorer/kb.py` and writes what it saw; these tests replay the same fixture through
//! pk-kb and compare as parsed JSON. A change on either side that moves a field shows up here.

use std::path::PathBuf;

use pk_kb::read::NoLaughs;
use pk_kb::{Decision, View, build, undo_line};
use serde_json::{Value, json};

fn golden(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(name);
    serde_json::from_str(
        &std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("{}: {e} (run python rust/tools/gen_kb_goldens.py)", p.display())),
    )
    .expect("golden is JSON")
}

fn fixture() -> (View, Value, String) {
    let f = golden("fixture.json");
    let decisions: Vec<Decision> = serde_json::from_value(f["decisions"].clone()).expect("decisions");
    let report: Vec<String> = serde_json::from_value(f["report"].clone()).expect("report");
    let now = f["now"].as_str().expect("now").to_string();
    (View::new(&f["graph"], decisions, report, "owner"), f, now)
}

/// A readable diff on failure: the first path where two JSON values differ.
fn first_diff(rust: &Value, python: &Value, path: &str) -> Option<String> {
    match (rust, python) {
        (Value::Object(ro), Value::Object(po)) => {
            for key in ro.keys().chain(po.keys().filter(|k| !ro.contains_key(*k))) {
                match (ro.get(key), po.get(key)) {
                    (Some(rv), Some(pv)) => {
                        if let Some(d) = first_diff(rv, pv, &format!("{path}.{key}")) {
                            return Some(d);
                        }
                    }
                    (Some(rv), None) => return Some(format!("{path}.{key}: rust has {rv}, python has nothing")),
                    (None, Some(pv)) => return Some(format!("{path}.{key}: rust has nothing, python has {pv}")),
                    (None, None) => {}
                }
            }
            None
        }
        (Value::Array(ra), Value::Array(pa)) => {
            if ra.len() != pa.len() {
                return Some(format!("{path}: rust has {} items, python has {}", ra.len(), pa.len()));
            }
            ra.iter()
                .zip(pa)
                .enumerate()
                .find_map(|(i, (rv, pv))| first_diff(rv, pv, &format!("{path}[{i}]")))
        }
        (Value::Number(rn), Value::Number(pn)) => {
            let (fr, fp) = (rn.as_f64().unwrap_or(f64::NAN), pn.as_f64().unwrap_or(f64::NAN));
            ((fr - fp).abs() > 1e-9).then(|| format!("{path}: rust {rn}, python {pn}"))
        }
        _ => (rust != python).then(|| format!("{path}: rust {rust}, python {python}")),
    }
}

fn same(what: &str, rust: &Value, python: &Value) {
    if let Some(d) = first_diff(rust, python, what) {
        panic!("{what} differs from the Python golden at {d}");
    }
}

#[test]
fn replay_matches_the_python_view() {
    let (v, _, _) = fixture();
    let g = golden("view.json");
    same("graph_meta", &json!(v.graph_meta), &g["graph_meta"]);
    same("nodes", &json!(v.nodes), &g["nodes"]);
    same("edges", &json!(v.edges), &g["edges"]);
    same("orphans", &json!(v.orphans), &g["orphans"]);
    same("undone", &json!(v.undone), &g["undone"]);
    same("alias", &json!(v.alias), &g["alias"]);
    same("adj", &json!(v.adj), &g["adj"]);
    same("jokes", &json!(v.jokes), &g["jokes"]);
    same("labels", &json!(v.labels), &g["labels"]);
    same("unit_joke", &json!(v.unit_joke), &g["unit_joke"]);
    same("joke_units", &json!(v.joke_units), &g["joke_units"]);
    same("degree", &json!(v.degree), &g["degree"]);
}

#[test]
fn read_models_match_the_python_view() {
    let (v, _, _) = fixture();
    let g = golden("view.json");
    let briefs: serde_json::Map<String, Value> = g["briefs"]
        .as_object()
        .expect("briefs")
        .keys()
        .map(|k| (k.clone(), Value::Object(v.brief(Some(k)))))
        .collect();
    same("briefs", &Value::Object(briefs), &g["briefs"]);
    let evs: serde_json::Map<String, Value> = v
        .edges
        .iter()
        .map(|(k, e)| (k.clone(), Value::Object(v.edge_view(e, None))))
        .collect();
    same("edge_views", &Value::Object(evs), &g["edge_views"]);
    let evf: serde_json::Map<String, Value> = v
        .edges
        .iter()
        .map(|(k, e)| {
            (
                k.clone(),
                Value::Object(v.edge_view(e, e.get("src").and_then(Value::as_str))),
            )
        })
        .collect();
    same("edge_views_focus", &Value::Object(evf), &g["edge_views_focus"]);
    same("summaries", &json!(v.joke_summaries()), &g["summaries"]);
    let searches: serde_json::Map<String, Value> = g["search"]
        .as_object()
        .expect("search")
        .keys()
        .map(|q| (q.clone(), json!(v.search(q))))
        .collect();
    same("search", &Value::Object(searches), &g["search"]);
    let details: serde_json::Map<String, Value> = v
        .jokes
        .iter()
        .map(|j| {
            (
                j.clone(),
                v.joke_detail(j, &NoLaughs).map_or(Value::Null, Value::Object),
            )
        })
        .collect();
    same("details", &Value::Object(details), &g["details"]);
    assert!(
        v.joke_detail("unit:m1", &NoLaughs).is_none(),
        "a unit has no joke detail"
    );
    same("label_index", &json!(v.label_index()), &g["label_index"]);
    same("actors", &json!(v.actors()), &g["actors"]);
}

#[test]
fn the_builder_accepts_and_refuses_exactly_like_python() {
    let (v, _, now) = fixture();
    let g = golden("builds.json");
    for (i, case) in g.as_array().expect("cases").iter().enumerate() {
        let req = &case["request"];
        let got = build(&v, req, &now, "owner");
        match (got, case.get("built"), case.get("error")) {
            (Ok(b), Some(want), _) => same(&format!("build[{i}] {req}"), &json!(b), want),
            (Err(e), _, Some(msg)) => {
                assert_eq!(json!(e.msg), *msg, "build[{i}] {req}: message");
                assert_eq!(json!(e.status), case["status"], "build[{i}] {req}: status");
            }
            (Ok(b), None, Some(msg)) => panic!("build[{i}] {req}: rust built {}, python refused: {msg}", json!(b)),
            (Err(e), Some(want), None) => panic!("build[{i}] {req}: rust refused ({}), python built {want}", e.msg),
            _ => panic!("build[{i}]: bad golden"),
        }
    }
}

#[test]
fn undo_lines_match_python() {
    let (v, _, now) = fixture();
    let g = golden("undo.json");
    for case in g.as_array().expect("cases") {
        let index = usize::try_from(case["index"].as_u64().expect("index")).expect("fits");
        match (undo_line(&v, index, &now, "owner"), case.get("line"), case.get("error")) {
            (Ok(line), Some(want), _) => same(&format!("undo[{index}]"), &Value::Object(line), want),
            (Err(e), _, Some(msg)) => {
                assert_eq!(json!(e.msg), *msg, "undo[{index}]");
                assert_eq!(json!(e.status), case["status"], "undo[{index}]");
            }
            (r, _, _) => panic!("undo[{index}]: rust {r:?}, golden {case}"),
        }
    }
}

#[test]
fn helpers_match_python() {
    let g = golden("helpers.json");
    for row in g["edge_ids"].as_array().expect("rows") {
        let (s, k, d, want) = (
            row[0].as_str().unwrap(),
            row[1].as_str().unwrap(),
            row[2].as_str().unwrap(),
            row[3].as_str().unwrap(),
        );
        assert_eq!(pk_kb::edge_id(s, k, d), want);
    }
    for row in g["slugs"].as_array().expect("rows") {
        assert_eq!(
            pk_kb::slugify(row[0].as_str().unwrap()),
            row[1].as_str().unwrap(),
            "slug of {:?}",
            row[0]
        );
    }
    for row in g["buckets"].as_array().expect("rows") {
        assert_eq!(
            pk_kb::bucket(Some(&row[0])).as_str(),
            row[1].as_str().unwrap(),
            "bucket of {}",
            row[0]
        );
    }
}
