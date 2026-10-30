//! What Python prints and how it judges a JSON value, named after the builtin each function stands for. Used where
//! a text must equal the Python's: the refusal a reducer or the decision builder answers (`unknown kind 1e-05`), the
//! schema a retry instruction embeds (`json.dumps`), the prompt hash the runs log records. Not a Python serializer in
//! general: the subset a `serde_json::Value` can need, and the escapes `repr` writes for the characters that occur
//! in such texts (`\\`, `\n`, `\r`, `\t`, `\xNN` for the other ASCII controls; wider non-printables are left as they
//! are).
//!
//! The rule (`rust/CLAUDE.md`): anything named after a Python builtin lives here and nowhere else.

use std::fmt::Write as _;

use serde_json::{Map, Number, Value};

/// A Python exception these helpers raise, by class: the reducers answer a `ValueError` as a refusal (HTTP 400) and
/// a `TypeError` as a fault. The text is the message Python prints.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PyError {
    /// `ValueError`.
    #[error("{0}")]
    Value(String),
    /// `TypeError`.
    #[error("{0}")]
    Type(String),
}

/// Python truthiness of a JSON value: `None`, `False`, `0`, `""`, `[]` and `{}` are false. A missing key is `None`.
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
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

/// `repr(value)` of what `json.loads` gives back: `None`, `True`, `1`, `1.5`, `1e-05`, `'text'`, `[1, 2]`,
/// `{'k': 1}`, nested.
pub fn repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Number(n) => number(n),
        Value::String(s) => quote(s),
        Value::Array(a) => format!("[{}]", a.iter().map(repr).collect::<Vec<_>>().join(", ")),
        Value::Object(m) => {
            let items: Vec<String> = m.iter().map(|(k, v)| format!("{}: {}", quote(k), repr(v))).collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

/// `str(value)`: a string as itself, anything else as its [`repr`] (so `None` for null).
pub fn str(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        v => repr(v),
    }
}

/// `repr(str)`: single quotes unless the text has a single quote and no double quote; `\\`, `\n`, `\r`, `\t` and
/// the other ASCII and Latin-1 controls (U+007F to U+00A0, the soft hyphen) escaped the way Python does.
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
            c if (c as u32) < 0x20 || (0x7f..=0xa0).contains(&(c as u32)) || c as u32 == 0xad => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

/// `repr(float)`: the shortest round-trip digits, fixed notation for decimal exponents in `-4..16`, else
/// `d.de+XX` (`1e+20`, `1e-05`); `inf`, `-inf`, `nan`, `-0.0`.
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
            format!("0.{}{}", "0".repeat(point.unsigned_abs() as usize), digits)
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

/// A JSON number the way Python prints it: an integer as is, a float through [`float_repr`].
fn number(n: &Number) -> String {
    if let Some(i) = n.as_i64() {
        i.to_string()
    } else if let Some(u) = n.as_u64() {
        u.to_string()
    } else {
        float_repr(n.as_f64().unwrap_or(f64::NAN))
    }
}

/// `json.dumps(value, ensure_ascii=False)`: `", "` between items, `": "` after keys, keys in the value's order,
/// floats through [`float_repr`], strings with JSON's escapes and non-ASCII text left alone.
pub fn dumps(value: &Value) -> String {
    let mut out = String::new();
    write_json(&mut out, value);
    out
}

/// `json.dumps(value, sort_keys=True, ensure_ascii=False)`.
pub fn dumps_sorted(value: &Value) -> String {
    dumps(&sort_keys(value))
}

/// The value with every object's keys sorted, recursively (what `sort_keys=True` serialises).
pub fn sort_keys(value: &Value) -> Value {
    match value {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = Map::with_capacity(m.len());
            for k in keys {
                out.insert(k.clone(), sort_keys(&m[k]));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(sort_keys).collect()),
        v => v.clone(),
    }
}

fn write_json(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&number(n)),
        Value::String(s) => write_json_str(out, s),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_json(out, x);
            }
            out.push(']');
        }
        Value::Object(m) => {
            out.push('{');
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_json_str(out, k);
                out.push_str(": ");
                write_json(out, v);
            }
            out.push('}');
        }
    }
}

/// A JSON string with `json.dumps`'s escapes (`\"`, `\\`, `\n`, `\r`, `\t`, `\b`, `\f`, `\u00XX` for the other
/// controls), which are `serde_json`'s compact ones.
fn write_json_str(out: &mut String, s: &str) {
    out.push_str(&serde_json::to_string(s).expect("a string serialises"));
}

/// Python's `str.isspace` on one character: Unicode `White_Space` plus the information separators U+001C–U+001F,
/// which Rust's `char::is_whitespace` leaves out.
pub fn isspace(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// `s.strip()`.
pub fn strip(s: &str) -> &str {
    s.trim_matches(isspace)
}

/// `len(s)` in code points.
pub fn len(s: &str) -> usize {
    s.chars().count()
}

/// `int(v)`: a bool is 0 or 1, a float is truncated, a string is parsed (surrounding Unicode whitespace, which for
/// `int()` excludes the U+001C–U+001F separators that `str.strip` drops; a sign; single underscores between ASCII
/// digits); anything else is the `TypeError`. A missing key is `None`. Not ported: the non-ASCII decimal digits
/// `int()` also accepts.
pub fn int(v: Option<&Value>) -> Result<i64, PyError> {
    let v = v.unwrap_or(&Value::Null);
    match v {
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f.trunc() as i64))
            .ok_or_else(|| PyError::Type(format!("int() argument out of range: {n}"))),
        Value::String(s) => parse_int(s.trim())
            .ok_or_else(|| PyError::Value(format!("invalid literal for int() with base 10: {}", quote(s)))),
        other => Err(PyError::Type(format!(
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
pub fn list(v: &Value) -> Result<Vec<Value>, PyError> {
    match v {
        Value::Array(a) => Ok(a.clone()),
        Value::String(s) => Ok(s.chars().map(|c| Value::String(c.to_string())).collect()),
        Value::Object(o) => Ok(o.keys().map(|k| Value::String(k.clone())).collect()),
        other => Err(PyError::Type(format!("'{}' object is not iterable", type_name(other)))),
    }
}

/// `sorted(set(xs))` over strings or over numbers; a mix, or an unhashable item, is the Python's `TypeError`.
pub fn sorted_set(xs: &[Value]) -> Result<Vec<Value>, PyError> {
    let mut out: Vec<Value> = Vec::new();
    for x in xs {
        if matches!(x, Value::Array(_) | Value::Object(_)) {
            return Err(PyError::Type(format!("unhashable type: '{}'", type_name(x))));
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
        return Err(PyError::Type(format!(
            "'<' not supported between instances of '{}' and '{}'",
            type_name(&out[1]),
            type_name(&out[0])
        )));
    }
    Ok(out)
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
        assert_eq!(dumps(&json!([])), "[]");
        assert_eq!(dumps(&json!({})), "{}");
        assert_eq!(
            dumps(&json!({"f": 1e-5, "g": 1e20, "q": "a\"b\u{1}"})),
            r#"{"f": 1e-05, "g": 1e+20, "q": "a\"b\u0001"}"#
        );
    }

    #[test]
    fn repr_and_str() {
        assert_eq!(str(&json!("^[a-z]+$")), "^[a-z]+$");
        assert_eq!(
            str(&json!({"k": 1, "list": ["a", "b"]})),
            "{'k': 1, 'list': ['a', 'b']}"
        );
        assert_eq!(str(&Value::Null), "None");
        assert_eq!(
            repr(&json!([true, false, null, 1.5, "it's"])),
            "[True, False, None, 1.5, \"it's\"]"
        );
        assert_eq!(repr(&json!("joke:x")), "'joke:x'");
        assert_eq!(quote("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(quote("tab\there"), "'tab\\there'");
        assert_eq!(quote("bell\u{7}"), "'bell\\x07'");
        assert_eq!(quote("nbsp\u{a0}"), "'nbsp\\xa0'");
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
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
            (f64::NAN, "nan"),
            (-0.0, "-0.0"),
        ] {
            assert_eq!(float_repr(f), s, "{f}");
        }
    }

    #[test]
    fn ints_like_python() {
        assert_eq!(int(Some(&json!(" 12 "))).unwrap(), 12);
        assert_eq!(int(Some(&json!(3.9))).unwrap(), 3);
        assert_eq!(int(Some(&json!(-3.9))).unwrap(), -3);
        assert_eq!(int(Some(&json!(true))).unwrap(), 1);
        assert_eq!(int(Some(&json!("1_0"))).unwrap(), 10);
        assert_eq!(
            int(Some(&json!("3.0"))).unwrap_err(),
            PyError::Value("invalid literal for int() with base 10: '3.0'".into())
        );
        assert_eq!(
            int(None).unwrap_err(),
            PyError::Type(
                "int() argument must be a string, a bytes-like object or a real number, not 'NoneType'".into()
            )
        );
    }

    #[test]
    fn truthiness_and_types() {
        assert!(!truthy(Some(&json!(""))) && !truthy(Some(&json!(0))) && !truthy(Some(&json!({}))) && !truthy(None));
        assert!(truthy(Some(&json!("a"))) && truthy(Some(&json!(0.5))) && truthy(Some(&json!([0]))));
        assert_eq!(type_name(&json!(1.0)), "float");
        assert_eq!(type_name(&json!(1)), "int");
        assert_eq!(type_name(&Value::Null), "NoneType");
    }

    #[test]
    fn isspace_includes_the_separators() {
        assert!(isspace('\u{1c}'));
        assert!(isspace('\u{a0}'));
        assert!(!isspace('a'));
        assert_eq!(strip("\u{1c} ab \u{1f}"), "ab");
        assert_eq!(len("héllo"), 5);
    }

    #[test]
    fn list_and_sorted_set() {
        assert_eq!(list(&json!("ab")).unwrap(), vec![json!("a"), json!("b")]);
        assert_eq!(list(&json!({"k": 1})).unwrap(), vec![json!("k")]);
        assert!(matches!(list(&json!(1)), Err(PyError::Type(_))));
        assert_eq!(
            sorted_set(&[json!("b"), json!("a"), json!("b")]).unwrap(),
            vec![json!("a"), json!("b")]
        );
        assert_eq!(sorted_set(&[json!(2), json!(1.5)]).unwrap(), vec![json!(1.5), json!(2)]);
        assert_eq!(
            sorted_set(&[json!(1), json!("a")]).unwrap_err(),
            PyError::Type("'<' not supported between instances of 'str' and 'int'".into())
        );
        assert_eq!(
            sorted_set(&[json!([1])]).unwrap_err(),
            PyError::Type("unhashable type: 'list'".into())
        );
    }
}
