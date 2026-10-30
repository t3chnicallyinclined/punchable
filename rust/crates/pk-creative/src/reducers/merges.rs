//! Merges (`creative/merge.py::store_apply`): his merges of jokes and their undo, the AI's attaches of a new note
//! to the bit it belongs to and his detaches, and the pairs he dismissed as not the same bit.

use serde_json::{Value, json};

use super::{item_mut, unknown_op};
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::pyish::{Obj, all_distinct, get_or_null, key, or, or_else};
use crate::store::{Items, Reducer};
use pk_core::py;

/// The ops (`MERGE_OPS`).
pub const MERGE_OPS: [&str; 5] = ["merge", "undo", "attach", "detach", "dismiss"];

/// The merge-store reducer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Merges;

/// `d.get("x") or []` as a list.
fn list_or_empty(d: &Obj, k: &str) -> CreativeResult<Vec<Value>> {
    match or(d, k) {
        Some(v) => Ok(py::list(v)?),
        None => Ok(Vec::new()),
    }
}

fn is_live(m: Option<&Obj>, kind: &str, status: &str) -> CreativeResult<bool> {
    let Some(m) = m.filter(|m| !m.is_empty()) else {
        return Ok(false);
    };
    if m.get("kind").and_then(Value::as_str) != Some(kind) {
        return Ok(false);
    }
    Ok(py::str(key(m, "status")?) == status)
}

impl Reducer for Merges {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, i, d) = (ev.op.as_str(), ev.id.as_str(), ev.data()?);
        match op {
            "merge" => {
                if items.contains_key(i) {
                    return Err(CreativeError::refused(format!("merge {i} exists")));
                }
                let others = list_or_empty(d, "others")?;
                let main = or(d, "main");
                if main.is_none_or(|m| others.is_empty() || others.contains(m)) || !all_distinct(&others) {
                    return Err(CreativeError::refused(
                        "a merge needs a main and other jokes (not the main)",
                    ));
                }
                items.insert(
                    i.to_string(),
                    json!({
                        "id": i, "kind": "merge", "status": "merged", "main": main, "others": others,
                        "indexes": list_or_empty(d, "indexes")?, "snapshot": or_else(d, "snapshot", json!({})),
                        "group": get_or_null(d, "group"), "reasons": or_else(d, "reasons", json!([])), "by": ev.by,
                        "created_at": ev.ts, "updated_at": ev.ts,
                    }),
                );
            }
            "undo" => {
                if !is_live(super::item(items, i), "merge", "merged")? {
                    return Err(CreativeError::refused(format!("no merge {i} to undo")));
                }
                let indexes = list_or_empty(d, "indexes")?;
                let m = item_mut(items, i).expect("checked above");
                m.insert("status".into(), json!("undone"));
                m.insert("undo_indexes".into(), Value::Array(indexes));
                m.insert("undone_at".into(), json!(ev.ts));
                m.insert("updated_at".into(), json!(ev.ts));
            }
            "attach" => {
                if items.contains_key(i) {
                    return Err(CreativeError::refused(format!("attach {i} exists")));
                }
                let (joke, into) = (or(d, "joke"), or(d, "into"));
                let (Some(joke), Some(into)) = (joke, into) else {
                    return Err(CreativeError::refused(
                        "an attach needs the new joke and the joke it belongs to",
                    ));
                };
                if joke == into {
                    return Err(CreativeError::refused(
                        "an attach needs the new joke and the joke it belongs to",
                    ));
                }
                items.insert(
                    i.to_string(),
                    json!({
                        "id": i, "kind": "attach", "status": "attached", "joke": joke, "into": into,
                        "text": get_or_null(d, "text"), "date": get_or_null(d, "date"), "score": get_or_null(d, "score"),
                        "reasons": or_else(d, "reasons", json!([])), "source": get_or_null(d, "source"), "by": ev.by,
                        "created_at": ev.ts, "updated_at": ev.ts,
                    }),
                );
            }
            "detach" => {
                if !is_live(super::item(items, i), "attach", "attached")? {
                    return Err(CreativeError::refused(format!("no attached note {i}")));
                }
                let m = item_mut(items, i).expect("checked above");
                m.insert("status".into(), json!("detached"));
                m.insert("detached_at".into(), json!(ev.ts));
                m.insert("updated_at".into(), json!(ev.ts));
            }
            "dismiss" => {
                let js = py::sorted_set(&list_or_empty(d, "jokes")?)?;
                if js.len() < 2 {
                    return Err(CreativeError::refused("dismiss needs two or more jokes"));
                }
                items.insert(
                    i.to_string(),
                    json!({"id": i, "kind": "dismiss", "status": "dismissed", "jokes": js, "by": ev.by,
                           "created_at": ev.ts, "updated_at": ev.ts}),
                );
            }
            _ => return Err(unknown_op(op)),
        }
        Ok(())
    }
}
