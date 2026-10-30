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

/// A non-empty string, or None (the Python `x or` idiom over a string field).
pub fn truthy_str(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// The string in a field, or None.
pub fn str_of<'a>(r: &'a Rec, k: &str) -> Option<&'a str> {
    r.get(k).and_then(Value::as_str)
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
}
