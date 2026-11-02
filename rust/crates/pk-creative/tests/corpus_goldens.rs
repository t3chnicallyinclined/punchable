//! The `Corpus` port against `creative/data.py` + `creative/notebook.py` + the riff grounds of `creative/write.py`:
//! `rust/tools/gen_corpus_goldens.py` loads the synthetic notebook (`rust/tools/notebook_fixture.py`) with the real
//! `Corpus.load` and dumps every structure the writing kinds read; this builds the Rust corpus from the same files
//! and compares as parsed JSON values (numbers by value, as Python's `==` compares them). Floats agree to a
//! relative 1e-12: `math.log` and Rust's `ln` may differ in the last bit on a platform's libm (seen on Windows for
//! one idf); every comparison the code makes over those values is computed the same way on each side.

use std::collections::{BTreeMap, HashSet};

use pk_creative::corpus::{Corpus, Parts, VectorPart, Vectors, venue_short};
use pk_creative::notebook::{Index, best_line, related, stage_words, texts};
use pk_creative::write::{last_night_words, riff_grounds};
use serde_json::{Value, json};

fn golden() -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens/corpus/notebook.json");
    serde_json::from_str(&std::fs::read_to_string(p).expect("run rust/tools/gen_corpus_goldens.py")).unwrap()
}

#[allow(clippy::many_single_char_names, clippy::float_cmp)]
/// Equal as Python compares parsed JSON: numbers by value (1 == 1.0), objects regardless of key order.
fn diff(a: &Value, b: &Value, at: &str) -> Option<String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y))
            if x.as_f64()
                .zip(y.as_f64())
                .is_some_and(|(p, q)| p == q || (p - q).abs() <= 1e-12 * p.abs().max(q.abs())) =>
        {
            None
        }
        (Value::Object(x), Value::Object(y)) => {
            if x.len() != y.len() {
                return Some(format!("{at}: keys rust {:?} vs python {:?}", x.keys(), y.keys()));
            }
            x.iter().find_map(|(k, v)| match y.get(k) {
                Some(w) => diff(v, w, &format!("{at}.{k}")),
                None => Some(format!("{at}.{k}: missing in python")),
            })
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
            .iter()
            .zip(y)
            .enumerate()
            .find_map(|(i, (p, q))| diff(p, q, &format!("{at}[{i}]"))),
        _ if a == b => None,
        _ => Some(format!("{at}: rust {a} vs python {b}")),
    }
}

fn same(rust: &Value, python: &Value, what: &str) {
    if let Some(d) = diff(rust, python, what) {
        panic!("{d}");
    }
}

fn corpus(g: &Value) -> Corpus {
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
    assert!(vectors.get("unit:bins-a").is_some() && vectors.get("summary:joke:carousel-idea").is_some());
    Corpus::new(Parts::from_files(&files, decisions, vectors, "owner").unwrap())
}

#[test]
fn the_graph_and_the_sets() {
    let g = golden();
    let c = corpus(&g);
    same(&json!(c.jokes), &g["jokes"], "jokes");
    same(&json!(c.labels), &g["labels"], "labels");
    let edges: Vec<Value> = c.edges.iter().map(|e| Value::Object(e.clone())).collect();
    same(
        &json!(edges.iter().map(|e| e["id"].clone()).collect::<Vec<_>>()),
        &g["edges"],
        "edge ids",
    );
    same(&Value::Array(edges), &g["edge_bodies"], "edges");
    let mut his: Vec<&String> = c.his_nodes.iter().collect();
    his.sort();
    same(&json!(his), &g["his_nodes"], "his_nodes");
    let mut his_stage: Vec<&String> = c.his_stage.iter().collect();
    his_stage.sort();
    same(&json!(his_stage), &g["his_stage"], "his_stage");
    same(&json!(c.perf_overrides), &g["perf_overrides"], "perf_overrides");
    same(&json!(c.his_lists), &g["his_lists"], "his_lists");
    same(&json!(c.unit_joke), &g["unit_joke"], "unit_joke");
    same(&json!(c.joke_units), &g["joke_units"], "joke_units");
    let jl: BTreeMap<&String, BTreeMap<&String, Value>> = c
        .joke_labels
        .iter()
        .map(|(j, ls)| {
            (
                j,
                ls.iter()
                    .map(|(l, (w, e))| (l, json!([w, c.edges[*e]["id"]])))
                    .collect(),
            )
        })
        .collect();
    same(&json!(jl), &g["joke_labels"], "joke_labels");
    same(&json!(c.label_df), &g["label_df"], "label_df");
    same(&json!(c.idf), &g["idf"], "idf");
    let mut inf: Vec<&String> = c.inferred_jokes.keys().collect();
    inf.sort();
    same(&json!(inf), &g["inferred_jokes"], "inferred_jokes");
    let sets: BTreeMap<&String, Value> = c
        .sets
        .iter()
        .map(|(k, s)| {
            (
                k,
                json!({"set": s.set, "date": s.date, "venue": s.venue, "title": s.title, "summary": s.summary,
                       "set_node": s.set_node}),
            )
        })
        .collect();
    same(&json!(sets), &g["sets"], "sets");
    let perfs: BTreeMap<&String, Vec<Value>> = c
        .perfs
        .iter()
        .map(|(j, ps)| (j, ps.iter().map(pk_creative::corpus::Perf::to_value).collect()))
        .collect();
    same(&json!(perfs), &g["perfs"], "perfs");
    let order: BTreeMap<&String, Vec<Value>> = c
        .order
        .iter()
        .map(|(k, o)| {
            (
                k,
                o.iter().map(|(j, i)| json!([j, c.segments[k][*i]["seg_id"]])).collect(),
            )
        })
        .collect();
    same(&json!(order), &g["order"], "order");
    let b2b: BTreeMap<String, Vec<Value>> = c
        .back_to_back
        .iter()
        .map(|((a, b), rs)| {
            (
                format!("{a}|{b}"),
                rs.iter()
                    .map(|r| {
                        json!({"set": r.set, "date": r.date, "venue": r.venue,
                                                           "a_seg": r.a_seg, "b_seg": r.b_seg})
                    })
                    .collect(),
            )
        })
        .collect();
    same(&json!(b2b), &g["back_to_back"], "back_to_back");
    same(&json!(c.set_keys()), &g["set_keys"], "set_keys");
}

#[test]
#[allow(clippy::many_single_char_names)]
fn per_joke_reads() {
    let g = golden();
    let c = corpus(&g);
    let terms: HashSet<String> = ["belt", "parrot", "rent"].iter().map(|s| (*s).to_string()).collect();
    for (j, want) in g["per_joke"].as_object().unwrap() {
        let at = |k: &str| format!("{j}.{k}");
        same(&json!(c.lead_unit(j)), &want["lead_unit"], &at("lead_unit"));
        same(&json!(c.title(j)), &want["title"], &at("title"));
        let (st, src) = c.stage(j);
        same(&json!([st, src]), &want["stage"], &at("stage"));
        let lo: BTreeMap<String, Value> = c
            .labels_of(j, None, 0.5)
            .into_iter()
            .map(|(l, (w, e))| (l, json!([w, c.edges[e]["id"]])))
            .collect();
        same(&json!(lo), &want["labels_of"], &at("labels_of"));
        same(&json!(c.devices(j, 3, 0.55)), &want["devices"], &at("devices"));
        let (d, basis) = c.duration(j);
        same(&json!([d, basis]), &want["duration"], &at("duration"));
        let (e, n) = c.energy(j);
        same(&json!([e, n]), &want["energy"], &at("energy"));
        same(&json!(texts(&c, j)), &want["texts"], &at("texts"));
        let (line, proven, nights) = stage_words(&c, j);
        same(&json!([line, proven, nights]), &want["stage_words"], &at("stage_words"));
        same(
            &json!(best_line(&c, j, &HashSet::new())),
            &want["best_line"],
            &at("best_line"),
        );
        same(
            &json!(best_line(&c, j, &terms)),
            &want["best_line_terms"],
            &at("best_line_terms"),
        );
        same(&json!(last_night_words(&c, j)), &want["last_night"], &at("last_night"));
    }
}

#[test]
fn text_evidence_and_words() {
    let g = golden();
    let c = corpus(&g);
    for (n, want) in g["text_of"].as_object().unwrap() {
        same(&json!(c.text_of(n, None)), want, &format!("text_of {n}"));
    }
    same(
        &json!(c.text_of("joke:airport-bins", Some("anatomy.reveal"))),
        &g["text_of_field"],
        "text_of field",
    );
    for e in &c.edges {
        let id = e["id"].as_str().unwrap();
        same(
            &json!(c.edge_evidence(e, 2)),
            &g["edge_evidence"][id],
            &format!("edge_evidence {id}"),
        );
    }
    same(
        &json!(c.seg_evidence("seg:2024-03-01_FakeSet0001#2")),
        &g["seg_evidence"],
        "seg_evidence",
    );
    let cases = [
        ("The Corner Room Comedy Club, Springfield", Some("2024-03-01")),
        ("maybe The Loft", None),
        ("Unknown (title 'x')", Some("2023")),
        ("", None),
        ("Comix at Riverside", Some("2019-01-01")),
        ("Mabel's (upstairs)", None),
        ("Ab Comedy", None),
        ("Joe's Poets Cafe", Some("2020")),
    ];
    for (v, d) in cases {
        same(
            &json!(venue_short(Some(v), d)),
            &g["venue_short"][v],
            &format!("venue_short {v:?}"),
        );
    }
}

#[test]
fn lexical_related_and_riff_grounds() {
    let g = golden();
    let c = corpus(&g);
    let ix = Index::new(&c);
    for q in g["inputs"]["queries"].as_array().unwrap() {
        let q = q.as_str().unwrap();
        let lex: Vec<Value> = ix.lexical(q).into_iter().map(|(j, s)| json!([j, s])).collect();
        same(&Value::Array(lex), &g["lexical"][q], &format!("lexical {q:?}"));
        same(
            &json!(related(&c, &ix, q, 6)),
            &g["related"][q],
            &format!("related {q:?}"),
        );
    }
    for (i, case) in g["inputs"]["ideas"].as_array().unwrap().iter().enumerate() {
        let lines: Vec<String> = case["lines"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .map(|x| pk_core::py::strip(x).to_string())
            .filter(|x| !x.is_empty())
            .collect();
        let got = riff_grounds(&c, &ix, case["idea"].as_str().unwrap(), &lines);
        same(
            &json!(got),
            &g["riff_grounds"][i]["grounds"],
            &format!("riff_grounds {i}"),
        );
    }
}
