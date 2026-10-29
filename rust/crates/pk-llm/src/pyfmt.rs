//! What Python prints: `json.dumps` with its default separators, `str()` and `repr()` of a JSON value. Used where
//! the text reaches the model and must equal the Python's (the retry instruction that embeds the schema, the
//! leftover-keys description `transform_schema` writes), and for the prompt hash the runs log records. Not a general
//! Python serializer: the subset a JSON value can need.

use std::fmt::Write as _;
use std::io;

use serde_json::ser::{CompactFormatter, Formatter};
use serde_json::{Map, Value};

/// `json.dumps(value, ensure_ascii=False)`: `", "` between items, `": "` after keys, keys in the value's order.
pub fn dumps(value: &Value) -> String {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, PyFormatter);
    serde::Serialize::serialize(value, &mut ser).expect("a JSON value serialises");
    String::from_utf8(out).expect("serde_json writes UTF-8")
}

/// `json.dumps(value, sort_keys=True, ensure_ascii=False)`.
pub fn dumps_sorted(value: &Value) -> String {
    dumps(&sorted(value))
}

/// The value with every object's keys sorted, recursively.
pub fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = Map::with_capacity(m.len());
            for k in keys {
                out.insert(k.clone(), sorted(&m[k]));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
        v => v.clone(),
    }
}

/// `str(value)`: a string as is, otherwise `repr(value)`.
pub fn py_str(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        v => py_repr(v),
    }
}

/// `repr(value)` for what `json.loads` gives back: `None`, `True`, `1`, `1.5`, `'text'`, `[1, 2]`, `{'k': 1}`.
pub fn py_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(u) = n.as_u64() {
                u.to_string()
            } else {
                float_repr(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        Value::String(s) => quote(s),
        Value::Array(a) => format!("[{}]", a.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        Value::Object(m) => {
            let items: Vec<String> = m.iter().map(|(k, v)| format!("{}: {}", quote(k), py_repr(v))).collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

/// `repr(str)`: single quotes unless the text has a single quote and no double quote; `\\`, `\n`, `\r`, `\t` and
/// the other controls escaped the way Python does.
pub fn quote(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(q);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

/// `repr(float)`: the shortest round-trip digits, fixed notation for exponents in `-4..16`, else `d.de+XX`.
pub fn float_repr(f: f64) -> String {
    if f.is_nan() {
        return "nan".to_string();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    // shortest digits and the decimal exponent, from Rust's own shortest `{:e}` rendering
    let sci = format!("{:e}", f.abs());
    let (mantissa, exp) = sci.split_once('e').expect("{:e} has an exponent");
    let exp: i32 = exp.parse().expect("an integer exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let sign = if f < 0.0 { "-" } else { "" };
    if (-4..16).contains(&exp) {
        let point = exp + 1; // digits before the decimal point
        let body = if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if (point as usize) >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(point as usize - digits.len()))
        } else {
            let (a, b) = digits.split_at(point as usize);
            format!("{a}.{b}")
        };
        format!("{sign}{body}")
    } else {
        let (first, rest) = digits.split_at(1);
        let mant = if rest.is_empty() {
            first.to_string()
        } else {
            format!("{first}.{rest}")
        };
        let esign = if exp < 0 { '-' } else { '+' };
        format!("{sign}{mant}e{esign}{:02}", exp.abs())
    }
}

/// `json.dumps`'s default separators on top of `serde_json`'s compact escaping (which, like `ensure_ascii=False`,
/// leaves non-ASCII text alone).
struct PyFormatter;

impl Formatter for PyFormatter {
    fn begin_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if first { Ok(()) } else { w.write_all(b", ") }
    }

    fn begin_object_key<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if first { Ok(()) } else { w.write_all(b", ") }
    }

    fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        w.write_all(b": ")
    }

    fn write_f64<W: ?Sized + io::Write>(&mut self, w: &mut W, value: f64) -> io::Result<()> {
        // Python writes floats with repr(); serde_json's ryu differs on exponents (1e-05 vs 1e-5)
        w.write_all(float_repr(value).as_bytes())
    }

    fn write_string_fragment<W: ?Sized + io::Write>(&mut self, w: &mut W, fragment: &str) -> io::Result<()> {
        CompactFormatter.write_string_fragment(w, fragment)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn dumps_uses_pythons_separators() {
        let v = json!({"type": "object", "properties": {"a": {"type": "array", "items": [1, 2.5, true, null]}}, "s": "é\n"});
        assert_eq!(
            dumps(&v),
            r#"{"type": "object", "properties": {"a": {"type": "array", "items": [1, 2.5, true, null]}}, "s": "é\n"}"#
        );
        assert_eq!(
            dumps_sorted(&json!({"b": 1, "a": {"d": 1, "c": 2}})),
            r#"{"a": {"c": 2, "d": 1}, "b": 1}"#
        );
    }

    #[test]
    fn repr_and_str() {
        assert_eq!(py_str(&json!("^[a-z]+$")), "^[a-z]+$");
        assert_eq!(
            py_str(&json!({"k": 1, "list": ["a", "b"]})),
            "{'k': 1, 'list': ['a', 'b']}"
        );
        assert_eq!(
            py_repr(&json!([true, false, null, 1.5, "it's"])),
            "[True, False, None, 1.5, \"it's\"]"
        );
        assert_eq!(quote("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(quote("tab\there"), "'tab\\there'");
    }

    #[test]
    fn float_repr_follows_python() {
        for (f, s) in [
            (0.5, "0.5"),
            (1.0, "1.0"),
            (1.5, "1.5"),
            (280.0, "280.0"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (-2.25, "-2.25"),
            (123_456.789, "123456.789"),
            (1.5e-7, "1.5e-07"),
        ] {
            assert_eq!(float_repr(f), s, "{f}");
        }
    }
}
