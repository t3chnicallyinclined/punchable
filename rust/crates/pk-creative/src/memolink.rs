//! Voice memos and app ideas linked to his bits (`creative/memolink.py`), the halves with no model and no IO:
//! which ideas have no units yet (`summary_only`, the text the matcher embeds), the text hash the vector file keys
//! on, and the vector file's update (`update_summary_vectors`): new or edited ideas embedded, gone ones dropped.

use std::collections::HashSet;

use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::corpus::{Corpus, s};

/// The vector file's path in the workspace (`vectors/extra_summary.json`).
pub const SUMMARY_FILE: &str = "vectors/extra_summary.json";

/// `sha(text)`: the first 16 hex of sha256.
pub fn sha16(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))[..16].to_string()
}

/// `" ".join(text.split())`: words joined by one space (Python's whitespace).
pub fn squash(text: &str) -> String {
    text.split(pk_core::py::isspace)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A joke's units as the same-idea index holds them (`merge.Index.units`): its non-hidden units, sorted.
pub fn units_of(c: &Corpus, j: &str) -> Vec<String> {
    let mut us: Vec<String> = c
        .joke_units
        .get(j)
        .map(|us| {
            us.iter()
                .filter(|u| !c.nodes.get(*u).is_some_and(|n| pk_core::py::truthy(n.get("hidden"))))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    us.sort();
    us
}

/// `summary_only(c)`: jokes with no units yet (an idea from a memo, the app, a Google Doc) → the text the matcher
/// sees, in joke-id order.
pub fn summary_only(c: &Corpus) -> IndexMap<String, String> {
    let mut jokes: Vec<&String> = c.jokes.iter().collect();
    jokes.sort();
    let mut out = IndexMap::new();
    for j in jokes {
        if !units_of(c, j).is_empty() {
            continue;
        }
        let summary = c.nodes.get(j).and_then(|n| s(n, "summary")).unwrap_or("");
        if !pk_core::py::strip(summary).is_empty() {
            out.insert(j.clone(), squash(summary));
        }
    }
    out
}

/// The vector file as `embed_summaries` keeps it: `{dim, model, rows, text_sha}` (an unreadable file starts over).
pub fn summary_doc(text: Option<&str>) -> Map<String, Value> {
    let mut doc = text
        .and_then(|t| serde_json::from_str::<Value>(t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_else(|| {
            json!({"dim": 384, "model": "bge-small-en-v1.5", "rows": {}, "text_sha": {}})
                .as_object()
                .cloned()
                .unwrap_or_default()
        });
    for k in ["rows", "text_sha"] {
        if !doc.get(k).is_some_and(Value::is_object) {
            doc.insert(k.to_string(), json!({}));
        }
    }
    doc
}

/// What to embed: the summary-only ideas whose text hash moved and that are not court text, by joke id.
pub fn to_embed(
    doc: &Map<String, Value>,
    want: &IndexMap<String, String>,
    is_legal: impl Fn(&str) -> bool,
) -> Vec<(String, String)> {
    let tsha = doc.get("text_sha").and_then(Value::as_object);
    let mut todo: Vec<(String, String)> = want
        .iter()
        .filter(|(j, t)| {
            tsha.and_then(|m| m.get(&format!("summary:{j}")))
                .and_then(Value::as_str)
                != Some(sha16(t).as_str())
                && !is_legal(t)
        })
        .map(|(j, t)| (j.clone(), t.clone()))
        .collect();
    todo.sort_by(|a, b| a.0.cmp(&b.0));
    todo
}

/// `float(f"{x:.7g}")`: seven significant digits, as the file stores a vector.
pub fn g7(x: f32) -> f64 {
    format!("{:.6e}", f64::from(x)).parse().unwrap_or(f64::from(x))
}

/// The file after embedding: new rows and hashes in, rows for ideas no longer summary-only out. Returns how many
/// were removed.
pub fn update_summary_vectors(
    doc: &mut Map<String, Value>,
    want: &IndexMap<String, String>,
    embedded: &[(String, String, Vec<f32>)],
) -> usize {
    let wanted: HashSet<&str> = want.keys().map(String::as_str).collect();
    // in place, as the Python's dicts: the file keeps its key order
    let mut gone: Vec<String> = Vec::new();
    if let Some(rows) = doc.get_mut("rows").and_then(Value::as_object_mut) {
        for (j, _, v) in embedded {
            rows.insert(
                format!("summary:{j}"),
                Value::Array(v.iter().map(|x| json!(g7(*x))).collect()),
            );
        }
        gone = rows
            .keys()
            .filter(|k| !wanted.contains(k.strip_prefix("summary:").unwrap_or(k)))
            .cloned()
            .collect();
        for k in &gone {
            rows.shift_remove(k);
        }
    }
    if let Some(tsha) = doc.get_mut("text_sha").and_then(Value::as_object_mut) {
        for (j, text, _) in embedded {
            tsha.insert(format!("summary:{j}"), json!(sha16(text)));
        }
        for k in &gone {
            tsha.shift_remove(k);
        }
    }
    gone.len()
}
