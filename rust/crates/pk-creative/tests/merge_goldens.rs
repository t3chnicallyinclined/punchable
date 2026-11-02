//! The same-idea and memo halves against `creative/merge.py` and `creative/memolink.py`:
//! `rust/tools/gen_merge_goldens.py` runs the real Python over a synthetic corpus built so every path fires; this
//! builds the same corpus from the same graph, decisions and vectors and compares as parsed JSON. Each unit's own
//! "usually this close" level comes from the similarity matrix (a BLAS product in the Python) and is compared to
//! float32 precision; everything decided from it is compared exactly.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use pk_creative::corpus::{Corpus, Parts, Vectors};
use pk_creative::memolink::{self, attach_event, clear_attaches, memo_jokes, memos, suggestions};
use pk_creative::merge::{
    Index, JUDGE_BATCH, MergeState, SECOND_BATCH, build_groups, judge_candidates, judge_finish, judge_user, match_text,
    pair_score, second_finish, second_todo, second_user, verdict,
};
use serde_json::{Map, Value, json};

fn golden() -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens/merge/merge.json");
    serde_json::from_str(&std::fs::read_to_string(p).expect("run rust/tools/gen_merge_goldens.py")).unwrap()
}

#[allow(clippy::float_cmp)]
fn diff(a: &Value, b: &Value, at: &str) -> Option<String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) if x.as_f64() == y.as_f64() => None,
        (Value::Object(x), Value::Object(y)) => {
            if x.len() != y.len() {
                return Some(format!("{at}: keys rust {:?} vs python {:?}", x.keys(), y.keys()));
            }
            x.iter().find_map(|(k, v)| {
                y.get(k).map_or_else(
                    || Some(format!("{at}.{k}: missing")),
                    |w| diff(v, w, &format!("{at}.{k}")),
                )
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
    let extras = vec![("extra_merge.json".to_string(), json!({"rows": g["vectors"]}))];
    Corpus::new(Parts {
        graph: g["graph"].clone(),
        decisions: g["decisions"].as_array().cloned().unwrap(),
        vectors: Arc::new(Vectors::from_parts(&[], &extras)),
        owner: "owner".into(),
        ..Parts::default()
    })
}

/// The canned judge the generator used: same for k % 3 != 2, confidence 0.95 / 0.7 / 0.9.
fn judge_answer(n: usize) -> Value {
    let pairs: Vec<Value> = (0..n)
        .map(|k| {
            let conf = [0.95, 0.7, 0.9][k % 3];
            json!({"id": format!("p{}", k + 1), "same": k % 3 != 2, "confidence": conf, "why": format!("verdict {k}")})
        })
        .collect();
    json!({"pairs": pairs})
}

#[test]
#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
fn the_same_idea_index_judge_groups_and_memos() {
    let g = golden();
    let c = Arc::new(corpus(&g));
    let ix = Index::new(Arc::clone(&c));
    same(&json!(ix.jokes), &g["index"]["jokes"], "jokes");
    let cands: Vec<Value> = ix.candidates().into_iter().map(|(a, b)| json!([a, b])).collect();
    same(&Value::Array(cands), &g["candidates"], "candidates");
    for (k, want) in g["features"].as_object().unwrap() {
        let (a, b) = k.split_once('|').unwrap();
        let f = ix.features(a, b);
        same(&f, want, &format!("features {k}"));
        same(&json!(pair_score(&f)), &g["pair_score"][k], &format!("pair_score {k}"));
        same(&json!(ix.judge_key(&f)), &g["judge_keys"][k], &format!("judge_key {k}"));
    }
    let jc = judge_candidates(&ix);
    same(&Value::Array(jc.clone()), &g["judge_candidates"], "judge_candidates");

    // the judge, batch by batch, then the second look
    let mut cache = Map::new();
    for (i, batch) in jc.chunks(JUDGE_BATCH).enumerate() {
        same(
            &json!(judge_user(&ix, batch)),
            &g["judge_asked"][i],
            &format!("judge prompt {i}"),
        );
        judge_finish(
            &ix,
            batch,
            Some(&judge_answer(batch.len())),
            "claude-haiku-5-5",
            &mut cache,
        );
    }
    let todo = second_todo(&c, &cache);
    same(&json!(todo), &g["second_todo"], "second_todo");
    for (i, batch) in todo.chunks(SECOND_BATCH).enumerate() {
        same(
            &json!(second_user(&ix, &cache, batch)),
            &g["second_asked"][i],
            &format!("second prompt {i}"),
        );
        let answer: Vec<Value> = (0..batch.len())
            .map(|k| json!({"id": format!("p{}", k + 1), "same": true, "confidence": 0.85, "why": "on a closer look"}))
            .collect();
        second_finish(batch, Some(&json!({"pairs": answer})), &mut cache);
    }
    same(&Value::Object(cache.clone()), &g["cache"], "cache");
    for (k, want) in g["verdicts"].as_object().unwrap() {
        same(&json!(verdict(cache.get(k))), want, &format!("verdict {k}"));
    }
    for (j, want) in g["full"].as_object().unwrap() {
        same(&json!(ix.full(j, 2500)), want, &format!("full {j}"));
    }

    // groups (his attach and dismiss applied), matches
    let items = g["items"].as_object().unwrap().clone();
    let st = MergeState::from_items(&items);
    let exclude: HashSet<String> = st.attached.keys().cloned().collect();
    same(
        &Value::Array(build_groups(&ix, &cache, &st, &exclude)),
        &g["groups"],
        "groups",
    );
    let empty = std::collections::HashMap::new();
    for m in g["matches"].as_array().unwrap() {
        let vec = m["vec"].as_str().and_then(|k| c.vectors.get(k));
        let r = match_text(&ix, m["text"].as_str().unwrap(), vec, &HashSet::new(), &empty);
        same(&r, &m["result"], &format!("match_text {} / {}", m["text"], m["vec"]));
    }

    // the memos, the triage, the suggestions, the clear attaches
    let files: BTreeMap<String, String> = serde_json::from_value(g["memo_files"].clone()).unwrap();
    let ms = memos(&files);
    same(&json!(ms), &g["memos"], "memos");
    let mj = memo_jokes(&c, &ms);
    same(&json!(mj), &g["memo_jokes"], "memo_jokes");
    let mut tcache = Map::new();
    let (batches, asked, skipped) = memolink::triage_batches(&ms, &tcache, |_| false);
    let mut sorted = 0;
    for (i, b) in batches.iter().enumerate() {
        same(
            &json!(memolink::triage_user(&ms, b)),
            &g["triage_asked"][i],
            &format!("triage prompt {i}"),
        );
        let answer: Vec<Value> = (0..b.len())
            .map(|k| {
                let kind = ["joke", "performed_set", "note", "bogus"][k % 4];
                json!({"id": format!("m{}", k + 1), "kind": kind, "audience": k % 2 == 1, "confidence": 0.8,
                       "why": format!("memo {k}")})
            })
            .collect();
        sorted += memolink::triage_finish(&ms, b, Some(&json!({"items": answer})), "claude-haiku-5-5", &mut tcache);
    }
    same(
        &json!({"asked": asked, "sorted": sorted, "legal_skipped": skipped, "cached": tcache.len()}),
        &g["triage"],
        "triage",
    );
    same(
        &Value::Object(tcache.clone()),
        &g["triage_doc"]["items"],
        "triage cache",
    );
    let recordings = memolink::set_recordings(Some(&g["triage_doc"]));
    let groups = memolink::memo_groups(&ix, &cache, &st, &recordings);
    let sugg: Vec<Value> = suggestions(&groups, &mj)
        .iter()
        .map(|x| x["id"].clone())
        .collect();
    same(&Value::Array(sugg), &g["suggestions"], "suggestions");
    let clear = clear_attaches(&ix, &c, &cache, &st, &recordings, &mj, &groups);
    same(&Value::Array(clear.clone()), &g["clear"], "clear attaches");
    let events: Vec<Value> = clear
        .iter()
        .map(|x| {
            let (aid, d) = attach_event(x, "<today>");
            json!([aid, d])
        })
        .collect();
    same(&Value::Array(events), &g["events"], "attach events");
}
