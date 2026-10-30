//! Shared by the golden tests: load a golden from `tests/goldens/`, and compare two JSON values with a readable
//! first-difference message (numbers within 1e-9 are equal: Python's float repr and serde's differ in the last
//! digits, docs/RUST.md "Facts that shape the port" 3).

use std::path::PathBuf;

use serde_json::Value;

/// A golden by file name, parsed.
pub fn golden(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(name);
    serde_json::from_str(
        &std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("{}: {e} (run python rust/tools/gen_creative_goldens.py)", p.display())),
    )
    .expect("golden is JSON")
}

/// The first path where two JSON values differ, or `None` when they are the same value.
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

/// Panic with the first difference, naming `what`.
pub fn same(what: &str, rust: &Value, python: &Value) {
    if let Some(d) = first_diff(rust, python, what) {
        panic!("{what} differs from the Python golden at {d}");
    }
}
