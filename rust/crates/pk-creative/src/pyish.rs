//! The few Python semantics the reducers lean on, named so a reader sees where the Python shows through: `int()`
//! over a JSON value, `str()`, `list()`, `sorted()`, `str.isspace`, a `KeyError`, the type name in a `TypeError`.
//! Nothing here is clever; each function says which Python expression it stands for.

use serde_json::{Map, Value};

pub use pk_kb::model::{head, truthy};

use crate::error::{CreativeError, CreativeResult};

/// A JSON object, keys in insertion order (`serde_json` `preserve_order`).
pub type Obj = Map<String, Value>;

/// Python's `str.isspace` on one character: Unicode `White_Space` plus the information separators U+001C–U+001F,
/// which Rust's `char::is_whitespace` leaves out.
pub fn py_isspace(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// `s.strip()`.
pub fn py_strip(s: &str) -> &str {
    s.trim_matches(py_isspace)
}

/// Python's type name for a JSON value, as a `TypeError` message spells it.
pub fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

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
    d.get(k).filter(|v| truthy(Some(v)))
}

/// `d.get(k)` as the JSON to store: `null` for a missing key.
pub fn get_or_null(d: &Obj, k: &str) -> Value {
    d.get(k).cloned().unwrap_or(Value::Null)
}

/// `d.get(k) or default`.
pub fn or_else(d: &Obj, k: &str, default: Value) -> Value {
    or(d, k).cloned().unwrap_or(default)
}

/// Python's `repr()` of a JSON value, nested (`{'a': 1, 'b': None}`).
pub fn repr(v: &Value) -> String {
    match v {
        Value::Array(a) => format!("[{}]", a.iter().map(repr).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => format!(
            "{{{}}}",
            o.iter()
                .map(|(k, v)| format!("{}: {}", repr(&Value::String(k.clone())), repr(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => pk_kb::model::py_repr(Some(other)),
    }
}

/// `str(v)`: a string as itself, anything else as its repr. The reducers put this in messages and cut it.
pub fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => repr(other),
    }
}

/// `(d.get(k) or "").strip()`: the Python fails on a truthy non-string, so this does too.
pub fn strip_or_empty<'a>(d: &'a Obj, k: &str) -> CreativeResult<&'a str> {
    match or(d, k) {
        None => Ok(""),
        Some(Value::String(s)) => Ok(py_strip(s)),
        Some(v) => Err(CreativeError::faulted(format!(
            "'{}' object has no attribute 'strip'",
            type_name(v)
        ))),
    }
}

/// `int(v)`: a bool is 0 or 1, a float is truncated, a string is parsed (surrounding whitespace, a sign and
/// single underscores between digits allowed); anything else is the `TypeError`. `None` here is Python's `None`.
pub fn py_int(v: Option<&Value>) -> CreativeResult<i64> {
    let v = v.unwrap_or(&Value::Null);
    match v {
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f.trunc() as i64))
            .ok_or_else(|| CreativeError::faulted(format!("int() argument out of range: {n}"))),
        Value::String(s) => parse_int(py_strip(s)).ok_or_else(|| {
            CreativeError::refused(format!(
                "invalid literal for int() with base 10: {}",
                repr(&Value::String(s.clone()))
            ))
        }),
        other => Err(CreativeError::faulted(format!(
            "int() argument must be a string, a bytes-like object or a real number, not '{}'",
            type_name(other)
        ))),
    }
}

fn parse_int(s: &str) -> Option<i64> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }
    let clean: String = digits.chars().filter(|c| *c != '_').collect();
    if !clean.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: i64 = clean.parse().ok()?;
    Some(if neg { -n } else { n })
}

/// `list(v)`: an array as is, a string as its characters, an object as its keys; the rest is not iterable.
pub fn py_list(v: &Value) -> CreativeResult<Vec<Value>> {
    match v {
        Value::Array(a) => Ok(a.clone()),
        Value::String(s) => Ok(s.chars().map(|c| Value::String(c.to_string())).collect()),
        Value::Object(o) => Ok(o.keys().map(|k| Value::String(k.clone())).collect()),
        other => Err(CreativeError::faulted(format!(
            "'{}' object is not iterable",
            type_name(other)
        ))),
    }
}

/// `sorted(set(xs))` over strings or over numbers; a mix, or an unhashable item, is the Python's `TypeError`.
pub fn py_sorted_set(xs: &[Value]) -> CreativeResult<Vec<Value>> {
    let mut out: Vec<Value> = Vec::new();
    for x in xs {
        if matches!(x, Value::Array(_) | Value::Object(_)) {
            return Err(CreativeError::faulted(format!("unhashable type: '{}'", type_name(x))));
        }
        if !out.contains(x) {
            out.push(x.clone());
        }
    }
    if out.iter().all(Value::is_string) {
        out.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    } else if out.iter().all(Value::is_number) {
        out.sort_by(|a, b| {
            a.as_f64()
                .unwrap_or(f64::NAN)
                .total_cmp(&b.as_f64().unwrap_or(f64::NAN))
        });
    } else if out.len() > 1 {
        return Err(CreativeError::faulted(format!(
            "'<' not supported between instances of '{}' and '{}'",
            type_name(&out[1]),
            type_name(&out[0])
        )));
    }
    Ok(out)
}

/// `len(s)` in code points.
pub fn py_len(s: &str) -> usize {
    s.chars().count()
}

/// `len(set(xs)) == len(xs)`: no two equal items.
pub fn all_distinct(xs: &[Value]) -> bool {
    xs.iter().enumerate().all(|(i, x)| !xs[..i].contains(x))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ints_like_python() {
        assert_eq!(py_int(Some(&json!(" 12 "))).unwrap(), 12);
        assert_eq!(py_int(Some(&json!(3.9))).unwrap(), 3);
        assert_eq!(py_int(Some(&json!(-3.9))).unwrap(), -3);
        assert_eq!(py_int(Some(&json!(true))).unwrap(), 1);
        assert_eq!(py_int(Some(&json!("1_0"))).unwrap(), 10);
        assert_eq!(
            py_int(Some(&json!("3.0"))).unwrap_err().msg(),
            "invalid literal for int() with base 10: '3.0'"
        );
        assert_eq!(
            py_int(None).unwrap_err().msg(),
            "int() argument must be a string, a bytes-like object or a real number, not 'NoneType'"
        );
    }

    #[test]
    fn repr_like_python() {
        assert_eq!(
            repr(&json!({"a": 1, "b": null, "c": true, "d": [1.0, "x"]})),
            "{'a': 1, 'b': None, 'c': True, 'd': [1.0, 'x']}"
        );
        assert_eq!(py_str(&json!("plain")), "plain");
    }

    #[test]
    fn isspace_includes_the_separators() {
        assert!(py_isspace('\u{1c}'));
        assert!(py_isspace('\u{a0}'));
        assert!(!py_isspace('a'));
        assert_eq!(py_strip("\u{1c} ab \u{1f}"), "ab");
    }
}
