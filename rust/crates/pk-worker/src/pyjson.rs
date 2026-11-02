//! The text files the worker writes, byte for byte as Python's `json.dumps(doc, ensure_ascii=False, indent=1)`
//! writes them: serde's pretty printer with a one-space indent, and floats printed by Python's `repr`
//! (`pk_core::py::float_repr`: `1e-05`, `2.8000000000000003`, `3.0`). Same bytes mean the same sha256, so a file the
//! Python worker wrote and the one the Rust worker writes for the same set are one `srcfile` row, not a rewrite.

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

#[cfg(test)]
mod tests {
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
