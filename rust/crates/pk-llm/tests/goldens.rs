//! The differential against the Python it replaces: `rust/tools/gen_llm_goldens.py` ran the pinned Anthropic SDK
//! over `engine/providers.py` with a fake transport and recorded the request bodies per task, the SDK's
//! `transform_schema`, the budget ledger after a scripted sequence of charges and the plain-words errors; these
//! tests build the same here and compare as parsed JSON.

mod common;

use common::{backend, golden, messages_of, same};
use pk_llm::api::Usage;
use pk_llm::budget::{Budget, price_of};
use pk_llm::schema::{parse_json, schema_instructions, transform_schema};
use pk_llm::{Config, LlmError, Router};
use serde_json::{Value, json};

#[test]
fn request_bodies_equal_what_the_python_sdk_sent() {
    let g = golden("requests.json");
    let cfg = Config::hosted();
    let b = backend(None, None);
    let router = Router::new(Config::hosted(), vec![backend(None, None)], None);
    let cases = g["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 50);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let task = case["task"].as_str().unwrap();
        let tcfg = cfg.task(task).unwrap();
        let chain = router.resolve(task).unwrap();
        assert_eq!(
            chain[0].1,
            case["model"].as_str().unwrap(),
            "{name}: the model the router picks"
        );
        let schema = (!case["schema"].is_null()).then(|| case["schema"].clone());
        let req = b
            .build_request(&chain[0].1, &messages_of(&case["messages"]), schema.as_ref(), &tcfg)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        same(name, &json!(req), &case["request"]["body"]);
        assert_eq!(case["request"]["method"], "POST");
        assert_eq!(case["request"]["path"], "/v1/messages");
    }
}

#[test]
fn the_router_resolves_every_hosted_task_as_the_python() {
    let g = golden("requests.json");
    let hosted = Config::hosted();
    let router = Router::new(Config::hosted(), vec![backend(None, None)], None);
    let table = g["routes"].as_object().expect("routes");
    assert_eq!(hosted.task_names(), table.keys().cloned().collect::<Vec<_>>());
    for (task, r) in table {
        let tcfg = hosted.task(task).unwrap();
        same(
            &format!("{task}.chain"),
            &json!(router.resolve(task).unwrap()),
            &r["chain"],
        );
        let model = tcfg.model_on("anthropic").unwrap();
        same(
            &format!("{task}.models_for"),
            &json!(router.models_for(&tcfg, "anthropic", model)),
            &r["models_for"],
        );
        same(&format!("{task}.effort"), &json!(tcfg.effort), &r["effort"]);
        same(&format!("{task}.offline"), &json!(tcfg.offline), &r["offline"]);
        same(&format!("{task}.fallbacks"), &json!(tcfg.fallbacks), &r["fallbacks"]);
        assert_eq!(router.describe(task), format!("anthropic:{model}"));
    }
    // without a key there is no provider, and the error is the Python's words
    let empty = Router::new(Config::hosted(), vec![], None);
    assert!(empty.resolve("creative_riff").unwrap().is_empty());
    assert_eq!(empty.describe("creative_riff"), "none");
}

/// The synthetic schemas by short name, as the generator used them.
fn schemas(g: &Value) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    for case in g["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap().rsplit('/').next().unwrap().to_string();
        if !case["schema"].is_null() && !out.iter().any(|(n, _)| *n == name) {
            out.push((name, case["schema"].clone()));
        }
    }
    out
}

#[test]
fn transform_schema_equals_the_sdks() {
    let g = golden("requests.json");
    let s = golden("schemas.json");
    let found = schemas(&g);
    assert_eq!(found.len(), 3);
    for (name, schema) in &found {
        same(
            &format!("transform({name})"),
            &transform_schema(schema).unwrap(),
            &s[name],
        );
    }
    same(
        "ref_only",
        &transform_schema(&json!({"$ref": "#/$defs/X", "description": "dropped"})).unwrap(),
        &s["ref_only"],
    );
    same(
        "string_plain",
        &transform_schema(&json!({"type": "string"})).unwrap(),
        &s["string_plain"],
    );
    // the instruction the retry sends embeds the schema the way Python prints it
    for (name, schema) in &found {
        assert_eq!(
            schema_instructions(schema),
            g["schema_instructions"][name].as_str().unwrap(),
            "{name}"
        );
    }
}

#[test]
fn the_budget_ledger_equals_the_python_after_the_same_charges() {
    let g = golden("budget.json");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.json");
    let mut b = Budget::new(g["daily_usd"].as_f64().unwrap(), &path);
    for (i, step) in g["steps"].as_array().unwrap().iter().enumerate() {
        b.set_day(step["day"].as_str().unwrap());
        if let Some(before) = step.get("remaining_before") {
            let status = b.status();
            assert!(
                (status.daily_usd - status.spent_usd - before.as_f64().unwrap()).abs() < 1e-9,
                "step {i}"
            );
        }
        let usage: Usage = serde_json::from_value(step["usage"].clone()).unwrap();
        let usd = b
            .charge(step["model"].as_str().unwrap(), &usage, step["task"].as_str().unwrap())
            .unwrap();
        assert!(
            (usd - step["usd"].as_f64().unwrap()).abs() < 1e-12,
            "step {i}: {usd} vs {}",
            step["usd"]
        );
    }
    b.set_day(g["today"].as_str().unwrap());
    let ledger: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    same("ledger", &ledger, &g["ledger"]);
    same("status", &json!(b.status()), &g["status"]);
    assert!((b.remaining() - g["remaining"].as_f64().unwrap()).abs() < 1e-9);
    assert!((b.spent_today() - g["spent_today"].as_f64().unwrap()).abs() < 1e-9);
    assert_eq!(b.reason(), g["reason"].as_str().unwrap());
    match (b.check(), g["exceeded"].as_str()) {
        (Ok(()), None) => {}
        (Err(e), Some(words)) => assert_eq!(e.to_string(), words),
        (r, w) => panic!("check: rust {r:?}, python {w:?}"),
    }
    for (model, prices) in g["prices"].as_object().unwrap() {
        let p = price_of(model);
        same(
            &format!("price_of({model:?})"),
            &json!([p.input, p.output, p.cache_write, p.cache_read]),
            prices,
        );
    }
    let zero = Budget::new(0.0, dir.path().join("zero.json")).with_day(g["today"].as_str().unwrap());
    assert_eq!(
        zero.check().unwrap_err().to_string(),
        g["zero_cap_exceeded"].as_str().unwrap()
    );
    let fresh = Budget::new(2.0, dir.path().join("fresh.json")).with_day(g["today"].as_str().unwrap());
    same("fresh_status", &json!(fresh.status()), &g["fresh_status"]);
}

#[test]
fn the_plain_words_are_the_pythons() {
    let g = golden("errors.json");
    for r in g["refusal"].as_array().unwrap() {
        let e = LlmError::Refusal {
            category: r["category"].as_str().map(String::from),
            explanation: r["explanation"].as_str().map(String::from),
        };
        assert_eq!(e.to_string(), r["plain"].as_str().unwrap());
    }
    for c in g["cut_short"].as_array().unwrap() {
        let e = LlmError::CutShort {
            max_tokens: c["max_tokens"].as_u64().unwrap() as u32,
        };
        assert_eq!(e.to_string(), c["plain"].as_str().unwrap());
    }
    for n in g["no_provider"].as_array().unwrap() {
        let e = LlmError::NoProvider {
            task: n["task"].as_str().unwrap().to_string(),
        };
        assert_eq!(e.to_string(), n["message"].as_str().unwrap());
    }
    for p in g["parse_json"].as_array().unwrap() {
        let got = parse_json(p["text"].as_str().unwrap());
        if p["ok"].as_bool().unwrap() {
            same("parse_json", &got.expect("parses"), &p["data"]);
        } else {
            assert!(got.is_err(), "{:?} must not parse", p["text"]);
        }
    }
}
