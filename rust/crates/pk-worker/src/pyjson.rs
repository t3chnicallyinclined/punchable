//! The text files the worker writes, byte for byte as Python's `json.dumps(doc, ensure_ascii=False, indent=1)`
//! writes them: serde's pretty printer with a one-space indent, and floats printed by Python's `repr`
//! (`pk_core::py::float_repr`: `1e-05`, `2.8000000000000003`, `3.0`). Same bytes mean the same sha256, so a file the
//! Python worker wrote and the one the Rust worker writes for the same set are one `srcfile` row, not a rewrite.

use std::fmt::Write as _;
use std::io;

use pk_core::py;
use serde::Serialize;
use serde_json::ser::{Formatter, PrettyFormatter};

struct PyPretty<'a>(PrettyFormatter<'a>);

impl Formatter for PyPretty<'_> {
    fn write_f64<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f64) -> io::Result<()> {
        writer.write_all(py::float_repr(value).as_bytes())
    }
    fn write_f32<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f32) -> io::Result<()> {
        writer.write_all(py::float_repr(f64::from(value)).as_bytes())
    }
    fn begin_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.begin_array(w)
    }
    fn end_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_array(w)
    }
    fn begin_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        self.0.begin_array_value(w, first)
    }
    fn end_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_array_value(w)
    }
    fn begin_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.begin_object(w)
    }
    fn end_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_object(w)
    }
    fn begin_object_key<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        self.0.begin_object_key(w, first)
    }
    fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.begin_object_value(w)
    }
    fn end_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_object_value(w)
    }
}

/// `json.dumps(value, ensure_ascii=False, indent=1)`.
pub fn dumps_indent1(value: &impl Serialize) -> String {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, PyPretty(PrettyFormatter::with_indent(b" ")));
    // a serde_json::Value or a derived struct cannot fail to serialise into memory
    if value.serialize(&mut ser).is_err() {
        return String::new();
    }
    String::from_utf8(out).unwrap_or_default()
}

/// `json.dumps(value, separators=(",", ":"))`: compact, every character outside printable ASCII escaped
/// (`ensure_ascii`, astral characters as surrogate pairs), floats by Python's `repr`.
pub fn dumps_compact_ascii(value: &serde_json::Value) -> String {
    use serde_json::Value;
    fn string(out: &mut String, s: &str) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{8}' => out.push_str("\\b"),
                '\u{c}' => out.push_str("\\f"),
                ' '..='~' => out.push(c),
                _ => {
                    let mut buf = [0u16; 2];
                    for unit in c.encode_utf16(&mut buf) {
                        let _ = write!(out, "\\u{unit:04x}");
                    }
                }
            }
        }
        out.push('"');
    }
    fn walk(out: &mut String, v: &Value) {
        match v {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) if n.is_f64() => out.push_str(&py::float_repr(n.as_f64().unwrap_or(0.0))),
            Value::Number(n) => out.push_str(&n.to_string()),
            Value::String(s) => string(out, s),
            Value::Array(a) => {
                out.push('[');
                for (i, x) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    walk(out, x);
                }
                out.push(']');
            }
            Value::Object(m) => {
                out.push('{');
                for (i, (k, x)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    string(out, k);
                    out.push(':');
                    walk(out, x);
                }
                out.push('}');
            }
        }
    }
    let mut out = String::new();
    walk(&mut out, value);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn compact_and_ascii_like_python() {
        let v = serde_json::json!({"a": [1, 2.5, 1e-05], "é": "x\u{1f600}\"", "n": null});
        assert_eq!(
            super::dumps_compact_ascii(&v),
            concat!(
                r#"{"a":[1,2.5,1e-05],""#,
                r"\u00e9",
                r#"":"x"#,
                r"\ud83d\ude00",
                r#"\"","n":null}"#
            )
        );
    }
}

#[cfg(test)]
mod indent_tests {
    use super::*;
    use serde_json::json;

    #[test]
    #[allow(clippy::unreadable_literal)]
    fn the_python_layout() {
        let v = json!({"a": 1, "b": [1.0, 2.8000000000000003, 1e-05], "c": {}, "d": [], "e": "é\n\u{1f}"});
        assert_eq!(
            dumps_indent1(&v),
            "{\n \"a\": 1,\n \"b\": [\n  1.0,\n  2.8000000000000003,\n  1e-05\n ],\n \"c\": {},\n \"d\": [],\n \"e\": \"é\\n\\u001f\"\n}"
        );
    }
}
