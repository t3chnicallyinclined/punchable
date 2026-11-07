//! Where each line of a rant came from (`docs/design/SPEC-2026-11-06-rant-tab.md` §12.7): one `set` per version unit,
//! `{unit, lines: [{by?: him | ai | yours, from?, label?, edited?, punch?}]}` (no `by`: not known), aligned with the version's lines. Written
//! after the version: when it fails the lines show no marks, never a wrong one. Rust only (R7): no Python twin, no
//! golden; `tests` below are the proof. 3c's `line_meta` is not built; when it lands this moves there.

use serde_json::{Map, Value, json};

use super::unknown_op;
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::store::{Items, Reducer};
use pk_core::py;

/// Lines one version may carry marks for.
pub const MAX_SCRIPT_LINES: usize = 500;
/// A source's label, at most (code points).
pub const MAX_LABEL_CHARS: usize = 200;

/// One line's mark, cleaned: only the fields the phone reads, or the refusal.
pub fn clean_mark(m: &Value) -> CreativeResult<Value> {
    let m = m
        .as_object()
        .ok_or_else(|| CreativeError::refused("a line's mark must be an object"))?;
    // no `by` = not known (a line from an older rant): it shows nothing, never a guess
    let mut out = Map::new();
    if let Some(by) = m.get("by").filter(|b| !b.is_null()) {
        let by = by.as_str().unwrap_or("");
        if !matches!(by, "him" | "ai" | "yours") {
            return Err(CreativeError::refused("by must be him, ai or yours"));
        }
        out.insert("by".into(), json!(by));
    }
    if let Some(from) = m.get("from").filter(|f| !f.is_null()) {
        let f = from.as_str().unwrap_or("");
        if !super::topics::is_material_ref(f) {
            return Err(CreativeError::refused("from must be a bit or a taped moment"));
        }
        out.insert("from".into(), json!(f));
    }
    if let Some(label) = m.get("label").filter(|l| !l.is_null()) {
        let l = label.as_str().unwrap_or("");
        if py::len(l) > MAX_LABEL_CHARS {
            return Err(CreativeError::refused(format!(
                "a label is at most {MAX_LABEL_CHARS} characters"
            )));
        }
        out.insert("label".into(), json!(l));
    }
    for k in ["edited", "punch"] {
        if m.get(k) == Some(&Value::Bool(true)) {
            out.insert(k.into(), json!(true));
        }
    }
    Ok(Value::Object(out))
}

/// The script reducer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Script;

impl Reducer for Script {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, id) = (ev.op.as_str(), ev.id.as_str());
        if !id.starts_with("unit:") {
            return Err(CreativeError::refused("id must be a version's unit"));
        }
        match op {
            "set" => {
                let d = ev.data()?;
                let lines = d
                    .get("lines")
                    .and_then(Value::as_array)
                    .ok_or_else(|| CreativeError::refused("lines must be a list"))?;
                if lines.len() > MAX_SCRIPT_LINES {
                    return Err(CreativeError::refused(format!("at most {MAX_SCRIPT_LINES} lines")));
                }
                let marks = lines.iter().map(clean_mark).collect::<CreativeResult<Vec<_>>>()?;
                items.insert(id.to_string(), json!({"unit": id, "lines": marks, "at": ev.ts}));
            }
            _ => return Err(unknown_op(op)),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(op: &str, id: &str, data: Value) -> Event {
        Event::new(op, id, data, "owner", "2026-11-07T12:00:00+00:00")
    }

    #[test]
    fn a_set_keeps_only_the_fields_the_phone_reads_and_the_newest_wins() {
        let mut items = Items::new();
        Script
            .apply(
                &mut items,
                &ev(
                    "set",
                    "unit:u1",
                    json!({"lines": [
                        {"by": "him", "from": "joke:a", "label": "Free employee v3", "edited": true, "x": 1},
                        {"by": "ai", "edited": false},
                        {"by": "yours", "punch": true, "from": null},
                        {"by": null}
                    ]}),
                ),
            )
            .unwrap();
        assert_eq!(
            items["unit:u1"]["lines"],
            json!([{"by": "him", "from": "joke:a", "label": "Free employee v3", "edited": true},
                   {"by": "ai"}, {"by": "yours", "punch": true}, {}])
        );
        Script
            .apply(&mut items, &ev("set", "unit:u1", json!({"lines": []})))
            .unwrap();
        assert_eq!(items["unit:u1"]["lines"], json!([]));
    }

    #[test]
    fn every_refusal_says_why() {
        let long = vec![json!({"by": "him"}); MAX_SCRIPT_LINES + 1];
        let cases = [
            (ev("set", "joke:a", json!({"lines": []})), "id must be a version's unit"),
            (ev("set", "unit:u", json!({})), "lines must be a list"),
            (
                ev("set", "unit:u", json!({"lines": [3]})),
                "a line's mark must be an object",
            ),
            (
                ev("set", "unit:u", json!({"lines": [{"by": "bot"}]})),
                "by must be him, ai or yours",
            ),
            (
                ev("set", "unit:u", json!({"lines": [{"by": "him", "from": "unit:x"}]})),
                "from must be a bit or a taped moment",
            ),
            (ev("set", "unit:u", json!({"lines": long})), "at most 500 lines"),
            (ev("del", "unit:u", json!({})), "unknown op del"),
        ];
        for (e, want) in cases {
            let mut items = Items::new();
            match Script.apply(&mut items, &e) {
                Err(CreativeError::Refused(m)) => assert_eq!(m, want),
                other => panic!("expected a refusal, got {other:?}"),
            }
            assert_eq!(items.len(), 0);
        }
    }
}
