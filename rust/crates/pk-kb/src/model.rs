//! Records and the small vocabulary over them: a node or an edge is a JSON object ([`Rec`]), its status falls in a
//! [`Bucket`], an edge's display kind is its custom kind if it has one.

use serde_json::{Map, Value};

/// A node or an edge: a JSON object, keys in insertion order. Private replay markers start with `_`
/// (`_by_tristan`, `_last`, `_edited`, `_merged_from`); [`public`] drops them.
pub type Rec = Map<String, Value>;

/// The edge kinds the pipeline and the comic may add.
pub const EDGE_KINDS: [&str; 10] = [
    "part_of",
    "variant_of",
    "performed_as",
    "tag_of",
    "same_shape",
    "callback_to",
    "same_premise",
    "has_label",
    "segue",
    "custom",
];
/// A joke's stages, in order.
pub const STAGES: [&str; 5] = ["idea", "to_try", "testing", "working", "retired"];
/// The label kinds, in display order.
pub const LABEL_KINDS: [&str; 8] = [
    "device",
    "structure",
    "form",
    "attitude",
    "theory",
    "topic",
    "persona",
    "custom",
];
/// What a stretch of tape is.
pub const PERF_KINDS: [&str; 3] = ["bit", "crowd_work", "banter"];

/// Where a status lands: confirmed (`confirmed`, `edited`), rejected (`rejected`, `ruled_out`, `superseded`), else
/// proposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Bucket {
    /// Not yet judged (the default).
    Proposed,
    /// Judged true, or edited.
    Confirmed,
    /// Judged false.
    Rejected,
}

impl Bucket {
    /// The word the API writes.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Confirmed => "confirmed",
            Self::Rejected => "rejected",
        }
    }

    /// The rank a merge uses to keep the better of two duplicate edges.
    pub fn rank(self) -> u8 {
        match self {
            Self::Rejected => 0,
            Self::Proposed => 1,
            Self::Confirmed => 2,
        }
    }
}

/// The bucket of a status value (a string, or anything else = proposed).
pub fn bucket(status: Option<&Value>) -> Bucket {
    match status.and_then(Value::as_str) {
        Some("confirmed" | "edited") => Bucket::Confirmed,
        Some("rejected" | "ruled_out" | "superseded") => Bucket::Rejected,
        _ => Bucket::Proposed,
    }
}

/// An edge's display kind: `custom_kind` if truthy, else `kind`, else `?`.
pub fn ekind(e: &Rec) -> String {
    truthy_str(e.get("custom_kind"))
        .or_else(|| truthy_str(e.get("kind")))
        .unwrap_or("?")
        .to_string()
}

/// The record without its private (`_`) fields.
pub fn public(r: &Rec) -> Rec {
    r.iter()
        .filter(|(k, _)| !k.starts_with('_'))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Python truthiness of a JSON value: null, false, 0, "", [] and {} are false.
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

/// A non-empty string, or None (the Python `x or` idiom over a string field).
pub fn truthy_str(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// The string in a field, or None.
pub fn str_of<'a>(r: &'a Rec, k: &str) -> Option<&'a str> {
    r.get(k).and_then(Value::as_str)
}

/// The first `n` characters (code points, like a Python slice) of a string.
pub fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Python's `repr()` of a JSON value, for the error messages the comic reads (`unknown node 'joke:x'`).
pub fn py_repr(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "None".to_string(),
        Some(Value::Bool(true)) => "True".to_string(),
        Some(Value::Bool(false)) => "False".to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => {
            let quote = if s.contains('\'') && !s.contains('"') {
                '"'
            } else {
                '\''
            };
            let mut out = String::with_capacity(s.len() + 2);
            out.push(quote);
            for c in s.chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    '\r' => out.push_str("\\r"),
                    c if c == quote => {
                        out.push('\\');
                        out.push(c);
                    }
                    c => out.push(c),
                }
            }
            out.push(quote);
            out
        }
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn buckets() {
        assert_eq!(bucket(Some(&json!("edited"))), Bucket::Confirmed);
        assert_eq!(bucket(Some(&json!("superseded"))), Bucket::Rejected);
        assert_eq!(bucket(None), Bucket::Proposed);
        assert_eq!(bucket(Some(&json!(3))), Bucket::Proposed);
    }

    #[test]
    fn repr_matches_python() {
        assert_eq!(py_repr(Some(&json!("joke:x"))), "'joke:x'");
        assert_eq!(py_repr(Some(&json!("it's"))), "\"it's\"");
        assert_eq!(py_repr(None), "None");
        assert_eq!(py_repr(Some(&json!(true))), "True");
    }

    #[test]
    fn truthiness() {
        assert!(!truthy(Some(&json!(""))) && !truthy(Some(&json!(0))) && !truthy(Some(&json!({}))));
        assert!(truthy(Some(&json!("a"))) && truthy(Some(&json!(0.5))) && truthy(Some(&json!([0]))));
    }
}
