//! `all_map` against `creative/notebook.py` (`rust/tools/gen_map_goldens.py`): the bird's-eye map over the synthetic
//! notebook at several caps, with and without the merge judge's same-idea groups, compared as parsed JSON.

use std::collections::BTreeMap;
use std::path::PathBuf;

use pk_creative::connections::all_map;
use pk_creative::corpus::{Corpus, Parts, VectorPart, Vectors};
use serde_json::Value;

fn read(rel: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(rel);
    serde_json::from_str(
        &std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("{}: {e} (run python rust/tools/gen_map_goldens.py)", p.display())),
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

#[test]
fn the_map_is_the_pythons_at_every_cap() {
    let g = read("map.json");
    let c = corpus();
    for case in g["cases"].as_array().unwrap() {
        let cap = usize::try_from(case["cap"].as_u64().unwrap()).unwrap();
        let groups: Vec<Value> = case["same_idea"].as_array().cloned().unwrap_or_default();
        assert_eq!(
            all_map(&c, cap, &groups),
            case["map"],
            "cap {cap}, groups {}",
            case["same_idea"].is_array()
        );
    }
}
