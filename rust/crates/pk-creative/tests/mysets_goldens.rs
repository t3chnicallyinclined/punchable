//! `create_event`, `change_note` and `match_recording` against `creative/mysets.py` (`rust/tools/gen_mysets_goldens.py`)
//! over the synthetic notebook: the `promote` event of a new set, the words a change is written down as, and the
//! recording each set lines up with (the score, his own pick at import, none).

use std::collections::BTreeMap;
use std::path::PathBuf;

use pk_creative::corpus::{Corpus, Parts, VectorPart, Vectors};
use pk_creative::mysets::{change_note, create_event, match_recording};
use serde_json::{Map, Value, json};

fn read(rel: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(rel);
    serde_json::from_str(
        &std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("{}: {e} (run python rust/tools/gen_mysets_goldens.py)", p.display())),
    )
    .expect("golden is JSON")
}

fn corpus() -> Corpus {
    let g = read("corpus/notebook.json");
    let files: BTreeMap<String, String> = serde_json::from_value(g["inputs"]["files"].clone()).unwrap();
    let decisions: Vec<Value> = serde_json::from_value(g["inputs"]["decisions"].clone()).unwrap();
    let parts: Vec<VectorPart> = files
        .iter()
        .filter(|(p, _)| p.starts_with("vectors/") && p.contains(".part"))
        .map(|(_, t)| serde_json::from_str(t).unwrap())
        .collect();
    let extras: Vec<(String, Value)> = files
        .iter()
        .filter(|(p, _)| p.starts_with("vectors/extra_"))
        .map(|(p, t)| (p.clone(), serde_json::from_str(t).unwrap()))
        .collect();
    let vectors = std::sync::Arc::new(Vectors::from_parts(&parts, &extras));
    Corpus::new(Parts::from_files(&files, decisions, vectors, "owner").unwrap())
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

fn sets_of(g: &Value) -> Vec<Map<String, Value>> {
    g["sets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_object().unwrap().clone())
        .collect()
}

#[test]
fn a_new_set_is_the_event_the_python_writes() {
    let g = read("mysets.json");
    let c = corpus();
    for x in g["creates"].as_array().unwrap() {
        let (id, data) = create_event(
            &c,
            usize::try_from(x["n_items"].as_u64().unwrap()).unwrap(),
            x["title"].as_str().unwrap(),
            x["target_s"].as_i64().unwrap(),
            &strings(&x["ids"]),
            x["note"].as_str(),
            x["storyboard_id"].as_str(),
        );
        assert_eq!(json!(id), x["id"], "{}", x["title"]);
        assert_eq!(data, x["data"], "{}", x["title"]);
    }
}

#[test]
fn a_change_is_written_down_as_the_python_writes_it() {
    let g = read("mysets.json");
    let c = corpus();
    for n in g["notes"].as_array().unwrap() {
        let got = change_note(
            &c,
            &strings(&n["old"]),
            &strings(&n["new"]),
            n["old_t"].as_i64().unwrap(),
            n["new_t"].as_i64().unwrap(),
        );
        assert_eq!(json!(got), n["note"], "{} -> {}", n["old"], n["new"]);
    }
}

#[test]
fn a_recording_lines_up_as_the_python_says() {
    let g = read("mysets.json");
    let mut c = corpus();
    let sets = sets_of(&g);
    let refs: Vec<&Map<String, Value>> = sets.iter().collect();
    for k in g["keys"].as_array().unwrap() {
        let key = k.as_str().unwrap();
        assert_eq!(match_recording(&c, &refs, key), g["matches"][key], "{key}");
    }
    assert_eq!(match_recording(&c, &refs, "no-such-set"), g["missing"]);
    // his own pick at import wins over the score
    let key = g["pick"]["key"].as_str().unwrap();
    let meta = c.metas.entry(key.to_string()).or_insert_with(|| json!({}));
    meta["align_working_set"] = g["pick"]["id"].clone();
    assert_eq!(match_recording(&c, &refs, key), g["pick"]["result"]);
}
