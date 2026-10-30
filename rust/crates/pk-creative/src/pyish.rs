//! The dict idioms the reducers lean on, named so a reader sees where the Python shows through: `d[k]` and its
//! `KeyError`, `d.get(k) or ...`, `(d.get(k) or "").strip()`, `len(set(xs)) == len(xs)`. The builtins themselves
//! (`str`, `int`, `repr`, `list`, `sorted`, `str.isspace`, truthiness) are `pk_core::py`; nothing here is a copy of
//! one.

use pk_core::py;
use serde_json::{Map, Value};

use crate::error::{CreativeError, CreativeResult};

/// A JSON object, keys in insertion order (`serde_json` `preserve_order`).
pub type Obj = Map<String, Value>;

/// `d[k]`: the value, or the `KeyError`.
pub fn key<'a>(d: &'a Obj, k: &str) -> CreativeResult<&'a Value> {
    d.get(k).ok_or_else(|| CreativeError::missing(k))
}

/// `d.get(k)` where `None` and a missing key are the same thing.
pub fn get<'a>(d: &'a Obj, k: &str) -> Option<&'a Value> {
    d.get(k).filter(|v| !v.is_null())
}

/// `d.get(k) or ...`: the value only when it is truthy.
pub fn or<'a>(d: &'a Obj, k: &str) -> Option<&'a Value> {
    d.get(k).filter(|v| py::truthy(Some(v)))
}

/// `d.get(k)` as the JSON to store: `null` for a missing key.
pub fn get_or_null(d: &Obj, k: &str) -> Value {
    d.get(k).cloned().unwrap_or(Value::Null)
}

/// `d.get(k) or default`.
pub fn or_else(d: &Obj, k: &str, default: Value) -> Value {
    or(d, k).cloned().unwrap_or(default)
}

/// `(d.get(k) or "").strip()`: the Python fails on a truthy non-string, so this does too.
pub fn strip_or_empty<'a>(d: &'a Obj, k: &str) -> CreativeResult<&'a str> {
    match or(d, k) {
        None => Ok(""),
        Some(Value::String(s)) => Ok(py::strip(s)),
        Some(v) => Err(CreativeError::faulted(format!(
            "'{}' object has no attribute 'strip'",
            py::type_name(v)
        ))),
    }
}

/// `len(set(xs)) == len(xs)`: no two equal items.
pub fn all_distinct(xs: &[Value]) -> bool {
    xs.iter().enumerate().all(|(i, x)| !xs[..i].contains(x))
}
