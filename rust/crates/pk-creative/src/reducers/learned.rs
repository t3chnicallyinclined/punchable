//! The learned section of his voice profile (`creative/learned.py`: `empty`, `merge`): the lines he kept and the
//! lines he passed over, newest first, duplicates folded (same words, any case or spacing), the latest action
//! winning between the two lists, each capped.

use serde_json::{Value, json};

use crate::pyish::{Obj, or};
use pk_core::py;
use pk_core::text::head;

/// Lines kept per list (`CAP_KEPT`).
pub const CAP_KEPT: usize = 25;
/// Lines passed over per list (`CAP_DROPPED`).
pub const CAP_DROPPED: usize = 25;
/// A kept line longer than this is cut (`LINE_CHARS`).
pub const LINE_CHARS: usize = 280;

/// `empty()`: nothing learned yet.
pub fn empty() -> Value {
    json!({"kept": [], "dropped": [], "updated_at": null})
}

/// `re.sub(r"\s+", " ", text).strip()`: whitespace runs collapsed, ends trimmed (Python's whitespace).
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if py::isspace(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out.trim_matches(' ').to_string()
}

/// `_norm(text)`: the comparison key of a line.
fn norm(text: &str) -> String {
    collapse(text).to_lowercase()
}

/// `_entry(x, ts)`: one line as stored, `{text, bit, date}` (+ `edited`), or nothing when it has no words.
fn entry(x: &Value, ts: Option<&str>) -> Option<Obj> {
    let bare;
    let x: &Obj = match x {
        Value::String(s) => {
            bare = json!({"text": s});
            bare.as_object().expect("an object")
        }
        Value::Object(o) => o,
        _ => return None,
    };
    let raw = x.get("text").and_then(Value::as_str)?;
    let mut text = collapse(raw);
    if text.is_empty() {
        return None;
    }
    if py::len(&text) > LINE_CHARS {
        let chars: Vec<char> = text.chars().collect();
        let cut = chars[..LINE_CHARS].iter().rposition(|c| *c == ' ');
        let keep = match cut {
            Some(c) if c > LINE_CHARS / 2 => c,
            _ => LINE_CHARS,
        };
        let kept: String = chars[..keep].iter().collect();
        text = format!("{} [...]", kept.trim_end_matches(py::isspace));
    }
    let bit = x
        .get("bit")
        .and_then(Value::as_str)
        .filter(|b| !b.is_empty())
        .map_or(Value::Null, |b| json!(b));
    let date = or(x, "date").cloned().unwrap_or_else(|| {
        ts.map(|t| head(t, 10))
            .filter(|t| !t.is_empty())
            .map_or(Value::Null, Value::String)
    });
    let mut out = Obj::new();
    out.insert("text".into(), json!(text));
    out.insert("bit".into(), bit);
    out.insert("date".into(), date);
    if x.get("edited") == Some(&Value::Bool(true)) {
        out.insert("edited".into(), json!(true));
    }
    Some(out)
}

fn entries(xs: Option<&Value>, ts: Option<&str>) -> Vec<Obj> {
    xs.filter(|v| py::truthy(Some(v)))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| entry(x, ts)).collect())
        .unwrap_or_default()
}

fn text_of(e: &Obj) -> &str {
    e.get("text").and_then(Value::as_str).unwrap_or("")
}

/// `merge(cur, kept, dropped, ts)`: the learned section after one event. `cur` is never mutated.
pub fn merge(cur: Option<&Value>, kept: Option<&Value>, dropped: Option<&Value>, ts: Option<&str>) -> Value {
    let empty_cur = empty();
    let cur: &Obj = cur
        .and_then(Value::as_object)
        .unwrap_or_else(|| empty_cur.as_object().expect("an object"));
    let new_k = entries(kept, ts);
    let k_keys: Vec<String> = new_k.iter().map(|e| norm(text_of(e))).collect();
    let new_d: Vec<Obj> = entries(dropped, ts)
        .into_iter()
        .filter(|e| !k_keys.contains(&norm(text_of(e))))
        .collect();
    let d_keys: Vec<String> = new_d.iter().map(|e| norm(text_of(e))).collect();
    let fold = |fresh: &[Obj], old: Option<&Value>, drop_keys: &[String]| -> Vec<Obj> {
        let mut out: Vec<Obj> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for e in fresh.iter().cloned().chain(entries(old, None)) {
            let k = norm(text_of(&e));
            if seen.contains(&k) || drop_keys.contains(&k) {
                continue;
            }
            seen.push(k);
            out.push(e);
        }
        out
    };
    let touched = !new_k.is_empty() || !new_d.is_empty();
    let mut kept_out = fold(&new_k, cur.get("kept"), &d_keys);
    kept_out.truncate(CAP_KEPT);
    let mut dropped_out = fold(&new_d, cur.get("dropped"), &k_keys);
    dropped_out.truncate(CAP_DROPPED);
    json!({
        "kept": kept_out,
        "dropped": dropped_out,
        "updated_at": if touched { ts.map_or(Value::Null, |t| json!(t)) } else { cur.get("updated_at").cloned().unwrap_or(Value::Null) },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_line_is_cut_at_a_space() {
        let long = format!("{} {} {}", "w".repeat(100), "x".repeat(100), "y".repeat(100));
        let m = merge(None, Some(&json!([long])), None, Some("2026-10-29T00:00:00+00:00"));
        let t = m["kept"][0]["text"].as_str().unwrap();
        assert_eq!(t.chars().count(), 207);
        assert!(t.ends_with("xxxxxx [...]"));
        assert_eq!(m["kept"][0]["date"], json!("2026-10-29"));
    }

    #[test]
    fn kept_beats_dropped_and_duplicates_fold() {
        let m = merge(
            None,
            Some(&json!(["A  line"])),
            Some(&json!(["a line", "Other"])),
            Some("t"),
        );
        assert_eq!(m["kept"].as_array().unwrap().len(), 1);
        assert_eq!(m["dropped"], json!([{"text": "Other", "bit": null, "date": "t"}]));
        let m2 = merge(Some(&m), None, Some(&json!(["a LINE"])), Some("u"));
        assert_eq!(m2["kept"], json!([]));
        assert_eq!(m2["dropped"].as_array().unwrap().len(), 2);
    }
}
