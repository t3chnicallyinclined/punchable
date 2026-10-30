//! The differential for Ask against `creative/askchat.py`: `rust/tools/gen_creative_goldens.py` flattens a
//! synthetic notebook (ten jokes, five sets, invented words), then records `tokens` and friends over word lists,
//! the sources `retrieve` picks (and the scores it sorted) for each question, the messages `prompt` builds, and
//! what `finish` makes of canned model answers. These tests run the same inputs through `pk_creative::ask` and
//! compare as parsed JSON; a fault the Python raised must be a fault here with the same message.

mod common;

use common::{golden, same};
use indexmap::IndexMap;
use pk_creative::ask::{
    MAX_SOURCES, Notebook, SNIPPET, Sources, Turn, chip, header, material_owner, mmss, month_year, refusal,
    short_title, snippet, system_text, tokens,
};
use serde_json::{Value, json};

fn notebook() -> Notebook {
    let g = golden("ask_notebook.json");
    let sources: Sources = serde_json::from_value(g["sources"].clone()).expect("sources");
    let rooms: IndexMap<String, String> = serde_json::from_value(g["rooms"].clone()).expect("rooms");
    let titles: IndexMap<String, String> = serde_json::from_value(g["joke_titles"].clone()).expect("titles");
    assert!(sources.len() > 20, "the notebook shrank");
    Notebook::new(sources, rooms, titles)
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(|x| x.as_str().expect("a string").to_string()).collect())
        .unwrap_or_default()
}

fn history(v: &Value) -> Vec<Turn> {
    if v.is_null() {
        return Vec::new();
    }
    serde_json::from_value(v.clone()).expect("history")
}

#[test]
fn words_match_python() {
    let g = golden("ask_text.json");
    for row in g["tokens"].as_array().expect("tokens") {
        assert_eq!(
            json!(tokens(row[0].as_str().unwrap())),
            row[1],
            "tokens of {:?}",
            row[0]
        );
    }
    for row in g["month_year"].as_array().expect("month_year") {
        assert_eq!(json!(month_year(row[0].as_str())), row[1], "month_year of {:?}", row[0]);
    }
    for row in g["mmss"].as_array().expect("mmss") {
        assert_eq!(json!(mmss(row[0].as_f64().unwrap())), row[1], "mmss of {:?}", row[0]);
    }
    for row in g["short_title"].as_array().expect("short_title") {
        assert_eq!(
            json!(short_title(row[0].as_str())),
            row[1],
            "short_title of {:?}",
            row[0]
        );
    }
    for case in g["snippet"].as_array().expect("snippet") {
        let terms = strings(&case["terms"]);
        let n = usize::try_from(case["n"].as_u64().unwrap()).unwrap();
        assert_eq!(
            json!(snippet(case["text"].as_str().unwrap(), &terms, n)),
            case["out"],
            "snippet {case}"
        );
    }
    assert!(g["tokens"].as_array().unwrap().len() >= 15 && g["snippet"].as_array().unwrap().len() >= 10);
}

#[test]
fn owner_rules_headers_and_chips_match_python() {
    let g = golden("ask_text.json");
    for case in g["owners"].as_array().expect("owners") {
        let owner = material_owner(case["name"].as_str());
        assert_eq!(json!(owner), case["owner"], "material_owner of {:?}", case["name"]);
        assert_eq!(
            json!(system_text(&owner)),
            case["system_text"],
            "system_text for {:?}",
            case["name"]
        );
    }
    assert_eq!(json!(pk_creative::ask::RULES), g["rules"]);
    assert_eq!(json!(pk_creative::ask::REFUSAL), g["refusal"]);
    let nb = notebook();
    let headers = g["headers"].as_object().expect("headers");
    assert_eq!(headers.len(), nb.sources().len());
    for (sid, want) in headers {
        assert_eq!(json!(header(&nb.sources()[sid])), *want, "header of {sid}");
    }
    let chips = g["chips"].as_array().expect("chips");
    assert_eq!(chips.len(), nb.sources().len());
    for (i, (s, want)) in nb.sources().values().zip(chips).enumerate() {
        let quote = if i % 2 == 0 { Some("a quote") } else { None };
        same(&format!("chip {}", s.id), &json!(chip(s, i + 1, quote)), want);
    }
}

#[test]
fn retrieve_matches_python() {
    let nb = notebook();
    let g = golden("ask_retrieve.json");
    let cases = g.as_array().expect("cases");
    let (mut empty, mut roomed, mut pinned_cases, mut capped) = (0, 0, 0, 0);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let q = case["question"].as_str().unwrap();
        let hist = history(&case["history"]);
        let cap = usize::try_from(case["cap"].as_u64().unwrap()).unwrap();
        let pin = nb.pinned(case["pin_joke"].as_str());
        let mut pin_sorted = pin.clone(); // Python's `pinned` is a set; the generator sorted it
        pin_sorted.sort();
        assert_eq!(json!(pin_sorted), case["pinned"], "{name}: pinned");
        assert_eq!(
            json!(nb.rooms_named(q).into_iter().collect::<Vec<_>>()),
            case["rooms"],
            "{name}: rooms_named"
        );
        let scores = nb.scores(q, &hist, &pin);
        let mut sorted: Vec<(&String, &f64)> = scores.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        let scores_json: serde_json::Map<String, Value> =
            sorted.into_iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        same(&format!("{name}: scores"), &Value::Object(scores_json), &case["scores"]);
        let ids = nb.retrieve(q, &hist, cap, &pin);
        assert_eq!(json!(ids), case["ids"], "{name}: ids");
        if ids.is_empty() {
            empty += 1;
        }
        if !case["rooms"].as_array().unwrap().is_empty() {
            roomed += 1;
        }
        if !pin.is_empty() {
            pinned_cases += 1;
        }
        if cap < MAX_SOURCES {
            capped += 1;
        }
    }
    assert!(cases.len() >= 12, "the retrieve goldens shrank: {}", cases.len());
    assert!(
        empty >= 2 && roomed >= 3 && pinned_cases >= 2 && capped >= 2,
        "the cases lost a branch"
    );
}

#[test]
fn prompt_matches_python() {
    let nb = notebook();
    let g = golden("ask_prompt.json");
    let cases = g.as_array().expect("cases");
    let mut long = 0;
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let q = case["question"].as_str().unwrap();
        let hist = history(&case["history"]);
        let ids = strings(&case["ids"]);
        let scope = nb.scope_line(case["scope_joke"].as_str());
        assert_eq!(json!(scope), case["scope"], "{name}: scope_line");
        let owner = case["owner"].as_str().unwrap();
        match nb.prompt(q, &hist, &ids, scope.as_deref(), owner) {
            Ok(msgs) => {
                assert!(case.get("error").is_none(), "{name}: python faulted: {}", case["error"]);
                same(&format!("{name}: messages"), &json!(msgs), &case["messages"]);
                if msgs.last().unwrap().content.contains("[...]") {
                    long += 1;
                }
            }
            Err(e) => {
                assert_eq!(json!(e.msg()), case["error"], "{name}: fault");
                assert_eq!(case["type"], json!("KeyError"));
            }
        }
    }
    assert!(cases.len() >= 6 && long >= 2, "the prompt goldens shrank");
    assert!(
        cases
            .iter()
            .any(|c| c["ids"].as_array().is_some_and(|ids| ids.len() > SNIPPET / 1000))
    );
}

#[test]
fn finish_matches_python() {
    let nb = notebook();
    let g = golden("ask_finish.json");
    let cases = g["finish"].as_array().expect("finish");
    let (mut refused, mut faulted, mut cut) = (0, 0, 0);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let ids = strings(&case["ids"]);
        match nb.finish(&ids, &case["data"], case["provider"].as_str(), case["model"].as_str()) {
            Ok(out) => {
                assert!(case.get("error").is_none(), "{name}: python faulted: {}", case["error"]);
                same(&format!("{name}: out"), &json!(out), &case["out"]);
                if !out.found {
                    refused += 1;
                }
                if !out.dropped.is_empty() {
                    cut += 1;
                }
            }
            Err(e) => {
                faulted += 1;
                assert_eq!(json!(e.msg()), case["error"], "{name}: fault");
                assert_eq!(case["type"], json!("TypeError"), "{name}: a TypeError in Python");
            }
        }
    }
    assert!(cases.len() >= 10, "the finish goldens shrank: {}", cases.len());
    assert!(refused >= 4 && faulted >= 1 && cut >= 5, "the cases lost a branch");
    for case in g["refusal"].as_array().expect("refusal") {
        same("refusal", &json!(refusal(&strings(&case["ids"]))), &case["out"]);
    }
}
