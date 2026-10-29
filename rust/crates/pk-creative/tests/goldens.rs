//! The differential against the Python it replaces: `rust/tools/gen_creative_goldens.py` runs synthetic event logs
//! through every `*_apply`, the verbatim corpus through both Python gates and `ntok` over a word list, and writes
//! what it saw; these tests replay the same events through pk-creative and compare as parsed JSON. Every refusal's
//! message must match, and a refusal (`ValueError`) must be a `Refused`, a malformed event a `Faulted`.

use std::path::PathBuf;

use pk_creative::{CreativeError, Event, Store, for_store, ntok, parse_lines, verbatim};
use serde_json::{Value, json};

fn golden(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(name);
    serde_json::from_str(
        &std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("{}: {e} (run python rust/tools/gen_creative_goldens.py)", p.display())),
    )
    .expect("golden is JSON")
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

const STORES: [&str; 10] = [
    "jobs",
    "storyboards",
    "workingsets",
    "venues",
    "account",
    "merges",
    "chats",
    "intents",
    "suggestions",
    "profile",
];

/// One store's golden replayed step by step through `append_event`: the same items after every event, the same
/// refusal text, the same kind of error.
fn check_store(name: &str) -> (usize, usize) {
    let g = golden(&format!("store_{name}.json"));
    let owner = g["owner"].as_str().expect("owner");
    let reducer = for_store(name, owner).unwrap_or_else(|| panic!("no reducer for {name}"));
    let mut store = Store::new(reducer);
    let steps = g["steps"].as_array().expect("steps");
    let mut refused = 0;
    for (i, step) in steps.iter().enumerate() {
        let ev: Event = serde_json::from_value(step["event"].clone()).expect("event");
        let what = format!("{name}[{i}] {} {}", ev.op, ev.id);
        match store.append_event(ev) {
            Ok(_) => assert!(
                step.get("error").is_none(),
                "{what}: rust accepted, python refused: {}",
                step["error"]
            ),
            Err(e) => {
                refused += 1;
                let want = step
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| panic!("{what}: rust refused ({e}), python accepted"));
                assert_eq!(e.msg(), want, "{what}: message");
                let is_value_error = step["type"] == json!("ValueError");
                match &e {
                    CreativeError::Refused(_) => {
                        assert!(is_value_error, "{what}: rust Refused, python {}", step["type"]);
                    }
                    CreativeError::Faulted(_) => assert!(!is_value_error, "{what}: rust Faulted, python ValueError"),
                    CreativeError::Replay { .. } => panic!("{what}: an append is never a replay error"),
                }
            }
        }
        same(&what, &Value::Object(store.items().clone()), &step["items"]);
    }
    (steps.len(), refused)
}

#[test]
fn every_store_replays_like_python() {
    let mut total = (0, 0);
    for name in STORES {
        let (n, bad) = check_store(name);
        assert!(
            bad > 0,
            "{name}: the golden has no refusal, so it proves nothing about the texts"
        );
        total.0 += n;
        total.1 += bad;
    }
    assert!(total.0 > 150, "the goldens shrank: {} events", total.0);
}

#[test]
fn a_log_of_the_accepted_lines_replays_to_the_same_items() {
    for name in STORES {
        let g = golden(&format!("store_{name}.json"));
        let owner = g["owner"].as_str().expect("owner");
        let steps = g["steps"].as_array().expect("steps");
        let lines: Vec<String> = steps
            .iter()
            .filter(|s| s.get("error").is_none())
            .map(|s| serde_json::to_string(&s["event"]).expect("json"))
            .collect();
        let padded: Vec<String> = lines
            .iter()
            .flat_map(|l| [l.clone(), "  ".to_string()])
            .chain(["{\"ts\": \"torn".to_string()])
            .collect();
        let store =
            Store::replay(for_store(name, owner).expect("reducer"), &padded).expect("the accepted lines replay");
        assert_eq!(store.n_events(), lines.len(), "{name}: n_events");
        same(
            &format!("{name} final"),
            &Value::Object(store.items().clone()),
            &steps.last().expect("steps")["items"],
        );
        let snap = store.snapshot("2026-10-29T13:00:00+00:00");
        assert_eq!(snap["version"], json!(1));
        assert_eq!(snap["events"], json!(lines.len()));
        same(
            &format!("{name} snapshot"),
            &snap["items"],
            &steps.last().expect("steps")["items"],
        );
        assert_eq!(
            parse_lines(&padded).len(),
            lines.len(),
            "{name}: torn and blank lines skipped"
        );

        // the full log, refusals included, stops at the first refused line with its message
        let all: Vec<Value> = steps.iter().map(|s| s["event"].clone()).collect();
        let first_bad = steps.iter().position(|s| s.get("error").is_some()).expect("a refusal");
        match Store::from_values(for_store(name, owner).expect("reducer"), all) {
            Err(CreativeError::Replay { index, msg }) => {
                assert_eq!(index, first_bad, "{name}: the replay stops at the first refusal");
                assert_eq!(msg, steps[first_bad]["error"], "{name}: with its message");
            }
            other => panic!("{name}: expected a replay error, got {other:?}"),
        }
    }
}

#[test]
fn the_verbatim_gate_matches_askchat() {
    let g = golden("verbatim.json");
    let mut differ = Vec::new();
    for (i, case) in g.as_array().expect("cases").iter().enumerate() {
        let (q, s) = (case["quote"].as_str().unwrap(), case["source"].as_str().unwrap());
        let got = verbatim(q, s);
        assert_eq!(json!(got), case["askchat"], "verbatim[{i}] {q:?} in {s:?}");
        if let Some(found) = got {
            assert!(
                s.contains(found),
                "verbatim[{i}]: the result is always a slice of the source"
            );
        }
        if case["same"] == json!(false) {
            differ.push(i);
        }
    }
    // the two Python copies differ only on the ellipsis length-gate cases the generator documents
    for i in &differ {
        let case = &g[*i];
        assert_eq!(
            case["askchat"],
            Value::Null,
            "verbatim[{i}]: askchat is the stricter copy"
        );
        assert!(
            case["quote"].as_str().unwrap().contains('\u{2026}'),
            "verbatim[{i}]: the only known difference involves an ellipsis"
        );
    }
    assert!(
        !differ.is_empty(),
        "the corpus no longer shows the documented difference"
    );
}

#[test]
fn ntok_matches_python() {
    for row in golden("ntok.json").as_array().expect("rows") {
        assert_eq!(
            ntok(row[0].as_str().unwrap()),
            row[1].as_str().unwrap(),
            "ntok of {:?}",
            row[0]
        );
    }
}
