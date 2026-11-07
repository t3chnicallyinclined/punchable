//! `json.dumps` as the Python wrote the state files: `ensure_ascii=False`, floats through `repr`, with an optional
//! indent and `sort_keys`. The state files are the contract between the Python and the Rust (either may read what the
//! other wrote), so they are byte-for-byte what `json.dumps` printed.

use pk_core::py;
use serde_json::Value;

/// How to print.
#[derive(Debug, Clone, Copy)]
pub struct Fmt {
    /// `indent=N` (items on their own lines), or none.
    pub indent: Option<usize>,
    /// `sort_keys=True`.
    pub sort_keys: bool,
    /// The separator between items (`","` with an indent or compact separators).
    pub item_sep: &'static str,
    /// The separator after a key.
    pub key_sep: &'static str,
}

/// `json.dumps(v, ensure_ascii=False, indent=1, sort_keys=True)`: `map.json`, `memos.json`.
pub const INDENT1_SORTED: Fmt = Fmt {
    indent: Some(1),
    sort_keys: true,
    item_sep: ",",
    key_sep: ": ",
};
/// `json.dumps(v, ensure_ascii=False, indent=1)`: `units.json`.
pub const INDENT1: Fmt = Fmt {
    indent: Some(1),
    sort_keys: false,
    item_sep: ",",
    key_sep: ": ",
};
/// `json.dumps(v, ensure_ascii=False, separators=(",", ":"))`: the `*.jsonl` lines.
pub const COMPACT: Fmt = Fmt {
    indent: None,
    sort_keys: false,
    item_sep: ",",
    key_sep: ":",
};

/// Print `v` the way `json.dumps` would with `f`.
pub fn dumps(v: &Value, f: Fmt) -> String {
    let mut out = String::new();
    write(&mut out, v, f, 0);
    out
}

fn newline(out: &mut String, f: Fmt, depth: usize) {
    if let Some(n) = f.indent {
        out.push('\n');
        out.push_str(&" ".repeat(n * depth));
    }
}

fn write(out: &mut String, v: &Value, f: Fmt, depth: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else {
                out.push_str(&py::float_repr(n.as_f64().unwrap_or(f64::NAN)));
            }
        }
        Value::String(s) => out.push_str(&serde_json::to_string(s).expect("a string serialises")),
        Value::Array(a) => {
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(f.item_sep);
                }
                newline(out, f, depth + 1);
                write(out, x, f, depth + 1);
            }
            newline(out, f, depth);
            out.push(']');
        }
        Value::Object(m) => {
            if m.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            let mut keys: Vec<&String> = m.keys().collect();
            if f.sort_keys {
                keys.sort();
            }
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push_str(f.item_sep);
                }
                newline(out, f, depth + 1);
                out.push_str(&serde_json::to_string(k).expect("a key serialises"));
                out.push_str(f.key_sep);
                write(out, &m[k], f, depth + 1);
            }
            newline(out, f, depth);
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn indent_one_sorted_matches_python() {
        // json.dumps({"b": [1, 2], "a": {}, "c": [], "d": "é\n", "e": 1.0}, ensure_ascii=False, indent=1, sort_keys=True)
        let v = json!({"b": [1, 2], "a": {}, "c": [], "d": "é\n", "e": 1.0});
        let want = "{\n \"a\": {},\n \"b\": [\n  1,\n  2\n ],\n \"c\": [],\n \"d\": \"é\\n\",\n \"e\": 1.0\n}";
        assert_eq!(dumps(&v, INDENT1_SORTED), want);
    }

    #[test]
    fn compact_has_no_spaces() {
        let v = json!({"a": [1, {"b": null}], "c": true});
        assert_eq!(dumps(&v, COMPACT), r#"{"a":[1,{"b":null}],"c":true}"#);
    }
}
