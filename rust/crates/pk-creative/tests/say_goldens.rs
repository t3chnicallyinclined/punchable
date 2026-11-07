//! `classify`, `parse_command` and `match_bits` against `api-dev/say_api.py` (`rust/tools/gen_say_goldens.py`): a corpus
//! of phrasings (questions with and without a mark, premises, fillers, requests, both commands in their spellings, number
//! words, apostrophes of both kinds, case) and a synthetic notebook. Every answer is compared as parsed JSON.

use std::collections::BTreeMap;
use std::path::PathBuf;

use pk_creative::corpus::{Corpus, Parts};
use pk_creative::corpus::{VectorPart, Vectors};
use pk_creative::say::{classify, match_bits, parse_command, words};
use serde_json::{Value, json};

fn read(rel: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(rel);
    serde_json::from_str(
        &std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("{}: {e} (run python rust/tools/gen_say_goldens.py)", p.display())),
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
fn every_phrasing_is_classified_and_parsed_as_the_python_does() {
    let g = read("say.json");
    let weekday = g["weekday"].as_str().unwrap();
    let phrases = g["phrases"].as_array().unwrap();
    assert!(phrases.len() > 150);
    let mut bad = Vec::new();
    for p in phrases {
        let text = p["text"].as_str().unwrap();
        let (kind, rule) = classify(text);
        let cmd = parse_command(text, weekday).unwrap_or(Value::Null);
        if kind != p["kind"] || rule != p["rule"] || cmd != p["command"] {
            bad.push(format!(
                "{text:?}: rust {kind}/{rule}/{cmd} python {}/{}/{}",
                p["kind"], p["rule"], p["command"]
            ));
        }
    }
    assert!(bad.is_empty(), "{} differ:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn bits_are_matched_as_the_python_matches_them() {
    let g = read("say.json");
    let c = corpus();
    for b in g["bits"].as_array().unwrap() {
        let q = b["query"].as_str().unwrap();
        let ws: Vec<String> = b["words"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w.as_str().unwrap().to_string())
            .collect();
        assert_eq!(json!(match_bits(&c, &ws, q, 5)), b["rows"], "{q:?}");
        assert_eq!(json!(match_bits(&c, &ws, q, 1)), b["rows_cap_1"], "{q:?} capped");
    }
}

#[test]
fn words_are_lower_cased_letters_digits_and_apostrophes() {
    assert_eq!(
        words("It\u{2019}s a 5-minute BIT, no?"),
        ["it's", "a", "5", "minute", "bit", "no"]
    );
}
