//! Ask conversations (`ask_api.chat_apply`): a chat is a title and its turns; a turn is his question, then the
//! answer (`by: ai:ask`) or the failure. Ops: `new`, `ask`, `answer`, `fail`, `patch`.

use serde_json::{Value, json};

use super::{item_mut, list_mut, unknown_op};
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::pyish::{get, get_or_null, head, key, or, py_len, py_list, py_str, py_strip, truthy};
use crate::store::{Items, Reducer};

/// The chat reducer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Chats;

impl Reducer for Chats {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, i, d) = (ev.op.as_str(), ev.id.as_str(), ev.data()?);
        if op == "new" {
            if items.contains_key(i) {
                return Err(CreativeError::refused(format!("chat {i} exists")));
            }
            let title = head(&py_str(or(d, "title").unwrap_or(&json!(""))), 120);
            let joke = d.get("joke").filter(|j| j.is_string()).cloned().unwrap_or(Value::Null);
            items.insert(
                i.to_string(),
                json!({"id": i, "title": title, "created_at": ev.ts, "updated_at": ev.ts, "deleted": false,
                       "turns": [], "joke": joke}),
            );
            return Ok(());
        }
        let Some(ch) = item_mut(items, i) else {
            return Err(CreativeError::refused(format!("no chat {i}")));
        };
        match op {
            "ask" => {
                let Some(q) = get(d, "q").and_then(Value::as_str).filter(|q| !py_strip(q).is_empty()) else {
                    return Err(CreativeError::refused("q is required"));
                };
                let want = get_or_null(d, "turn");
                let turns = list_mut(ch, "turns")?;
                if turns.iter().any(|t| t.get("id") == Some(&want)) {
                    return Err(CreativeError::refused(format!("turn {} exists", py_str(&want))));
                }
                let turn = key(d, "turn")?.clone();
                let sources = match or(d, "sources") {
                    Some(s) => py_list(s)?,
                    None => Vec::new(),
                };
                turns.push(json!({"id": turn, "q": q, "status": "thinking", "asked_at": ev.ts,
                                  "job": get_or_null(d, "job"), "sources": sources, "result": null, "error": null}));
            }
            "answer" | "fail" => {
                let want = get_or_null(d, "turn");
                let turns = list_mut(ch, "turns")?;
                let Some(t) = turns
                    .iter_mut()
                    .find(|t| t.get("id") == Some(&want))
                    .and_then(Value::as_object_mut)
                else {
                    return Err(CreativeError::refused(format!("no turn {}", py_str(&want))));
                };
                let status = py_str(key(t, "status")?);
                if status != "thinking" {
                    return Err(CreativeError::refused(format!(
                        "turn {} is {status}",
                        py_str(key(t, "id")?)
                    )));
                }
                if op == "answer" {
                    t.insert("status".into(), json!("done"));
                    t.insert("result".into(), get_or_null(d, "result"));
                } else {
                    t.insert("status".into(), json!("failed"));
                    t.insert(
                        "error".into(),
                        json!(head(&py_str(or(d, "error").unwrap_or(&json!("failed"))), 500)),
                    );
                }
                t.insert("done_at".into(), json!(ev.ts));
            }
            "patch" => {
                if let Some(t) = d.get("title") {
                    let title = if truthy(Some(t)) { py_str(t) } else { String::new() };
                    let title = py_strip(&title);
                    if title.is_empty() || py_len(title) > 120 {
                        return Err(CreativeError::refused("title must be 1-120 characters"));
                    }
                    ch.insert("title".into(), json!(title));
                }
                if let Some(del) = d.get("deleted") {
                    ch.insert("deleted".into(), json!(truthy(Some(del))));
                }
            }
            _ => return Err(unknown_op(op)),
        }
        ch.insert("updated_at".into(), json!(ev.ts));
        Ok(())
    }
}
