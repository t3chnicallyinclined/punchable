//! Shared by the integration tests: load a golden, diff two JSON values readably, the synthetic fixtures.

#![allow(dead_code)]

use std::path::PathBuf;

use pk_llm::{AnthropicBackend, Budget, ChatMessage, Client, Config};
use serde_json::Value;

/// The key the generator used; never a real one.
pub const KEY: &str = "sk-golden-not-a-key";

pub fn golden(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(name);
    serde_json::from_str(
        &std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("{}: {e} (run python rust/tools/gen_llm_goldens.py)", p.display())),
    )
    .expect("golden is JSON")
}

/// A readable diff on failure: the first path where two JSON values differ.
pub fn first_diff(rust: &Value, python: &Value, path: &str) -> Option<String> {
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

pub fn same(what: &str, rust: &Value, python: &Value) {
    if let Some(d) = first_diff(rust, python, what) {
        panic!("{what} differs from the Python golden at {d}");
    }
}

pub fn messages_of(v: &Value) -> Vec<ChatMessage> {
    serde_json::from_value(v.clone()).expect("messages")
}

/// The hosted backend on a client that never reaches the network (or the mock server when given).
pub fn backend(base_url: Option<&str>, budget: Option<Budget>) -> AnthropicBackend {
    let cfg = Config::hosted();
    let pcfg = cfg.providers()["anthropic"].clone();
    let mut client = Client::new(KEY).expect("client").with_max_retries(0);
    client = client.with_base_url(base_url.unwrap_or("http://127.0.0.1:9"));
    AnthropicBackend::new("anthropic", pcfg, client, budget)
}
