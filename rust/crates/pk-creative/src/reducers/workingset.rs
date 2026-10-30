//! Working sets (`creative/workingset.py::_apply`): a storyboard promoted to the set he performs, versioned, with
//! locks (a bit or a line the model must leave alone), his try-tonight notes and the ticks on them.

use serde_json::{Value, json};

use super::{item_mut, list_mut, list_of, obj_mut, unknown_op};
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::pyish::{Obj, get, get_or_null, key, or, or_else, strip_or_empty};
use crate::store::{Items, Reducer};
use pk_core::py;

/// The working-set reducer; `owner` is the actor id of his own edits.
#[derive(Debug, Clone)]
pub struct Workingsets {
    owner: String,
}

impl Workingsets {
    /// A reducer that calls `owner_id`'s edits his.
    pub fn new(owner_id: &str) -> Self {
        Self {
            owner: owner_id.to_string(),
        }
    }
}

/// `ws["locks"].setdefault(joke_id, {"bit": False, "lines": []})`.
fn lock_entry<'a>(ws: &'a mut Obj, joke_id: &str) -> CreativeResult<&'a mut Obj> {
    let locks = obj_mut(ws, "locks")?;
    let slot = locks
        .entry(joke_id.to_string())
        .or_insert_with(|| json!({"bit": false, "lines": []}));
    slot.as_object_mut()
        .ok_or_else(|| CreativeError::faulted("'NoneType' object does not support item assignment"))
}

impl Reducer for Workingsets {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, i, d) = (ev.op.as_str(), ev.id.as_str(), ev.data()?);
        if op == "promote" {
            if items.contains_key(i) {
                return Err(CreativeError::refused(format!("working set {i} exists")));
            }
            let title = key(d, "title")?.clone();
            let source = if ev.by_str() == Some(self.owner.as_str()) {
                "his"
            } else {
                "inferred"
            };
            let note = or_else(d, "note", json!("v1: promoted from the storyboard"));
            let target_s = py::int(Some(key(d, "target_s")?))?;
            let bits = py::list(key(d, "bits")?)?;
            items.insert(
                i.to_string(),
                json!({
                    "id": i, "title": title, "storyboard_id": get_or_null(d, "storyboard_id"), "by": ev.by,
                    "source": source, "created_at": ev.ts, "updated_at": ev.ts, "current": 1, "locks": {},
                    "deleted": false,
                    "versions": [{"v": 1, "created_at": ev.ts, "note": note, "target_s": target_s, "bits": bits,
                                  "by": ev.by}],
                }),
            );
            return Ok(());
        }
        let ws = match item_mut(items, i) {
            Some(ws) if !py::truthy(ws.get("deleted")) => ws,
            _ => return Err(CreativeError::refused(format!("no working set {i}"))),
        };
        let cur = list_of(ws, "versions")?
            .last()
            .and_then(Value::as_object)
            .cloned()
            .ok_or_else(|| CreativeError::faulted("list index out of range"))?;
        match op {
            "new_version" => {
                if !py::truthy(d.get("note")) {
                    return Err(CreativeError::refused("a new version needs a note saying what changed"));
                }
                let vn = key(&cur, "v")?.as_i64().unwrap_or(0) + 1;
                let target_s = match or(d, "target_s") {
                    Some(t) => py::int(Some(t))?,
                    None => py::int(Some(key(&cur, "target_s")?))?,
                };
                let bits = match get(d, "bits") {
                    None => py::list(key(&cur, "bits")?)?,
                    Some(b) => py::list(b)?,
                };
                let version = json!({"v": vn, "created_at": ev.ts, "note": d["note"], "target_s": target_s,
                                     "bits": bits, "by": ev.by});
                list_mut(ws, "versions")?.push(version);
                ws.insert("current".into(), json!(vn));
            }
            "lock_bit" | "unlock_bit" => {
                let jid = py::str(key(d, "joke_id")?);
                lock_entry(ws, &jid)?.insert("bit".into(), json!(op == "lock_bit"));
            }
            "lock_line" => {
                let jid = py::str(key(d, "joke_id")?);
                let text = key(d, "text")?;
                let lk = lock_entry(ws, &jid)?;
                let lines = list_mut(lk, "lines")?;
                if !lines.iter().any(|x| x.get("text") == Some(text)) {
                    lines.push(json!({"text": text, "node": get_or_null(d, "node"), "ts": ev.ts}));
                }
            }
            "unlock_line" => {
                let jid = py::str(key(d, "joke_id")?);
                let locks = obj_mut(ws, "locks")?;
                if let Some(lk) = locks
                    .get_mut(&jid)
                    .and_then(Value::as_object_mut)
                    .filter(|o| !o.is_empty())
                {
                    let lines = list_mut(lk, "lines")?;
                    if !lines.is_empty() {
                        let text = key(d, "text")?;
                        lines.retain(|x| x.get("text") != Some(text));
                    }
                }
            }
            "try_add" => {
                let text = strip_or_empty(d, "text")?.to_string();
                if text.is_empty() || py::len(&text) > 500 {
                    return Err(CreativeError::refused(
                        "a try note needs its text (up to 500 characters)",
                    ));
                }
                let want = get_or_null(d, "note_id");
                let notes = ws
                    .entry("try_notes".to_string())
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                    .ok_or_else(|| CreativeError::faulted("'NoneType' object is not iterable"))?;
                if notes.iter().any(|n| n.get("id") == Some(&want)) {
                    return Err(CreativeError::refused(format!("try note {} exists", py::str(&want))));
                }
                let note_id = key(d, "note_id")?.clone();
                notes.push(
                    json!({"id": note_id, "joke_id": get_or_null(d, "joke_id"), "text": text,
                                  "done": false, "created_at": ev.ts, "done_at": null, "by": ev.by}),
                );
            }
            "try_done" => {
                let done = py::truthy(d.get("done"));
                let Some(nid) = get(d, "note_id").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
                    return Err(CreativeError::refused("note_id is required"));
                };
                let pos = ws
                    .get("try_notes")
                    .and_then(Value::as_array)
                    .and_then(|ns| ns.iter().position(|n| n.get("id").and_then(Value::as_str) == Some(nid)));
                if let Some(p) = pos {
                    let n = list_mut(ws, "try_notes")?[p]
                        .as_object_mut()
                        .ok_or_else(|| CreativeError::faulted("'NoneType' object does not support item assignment"))?;
                    n.insert("done".into(), json!(done));
                    n.insert("done_at".into(), if done { json!(ev.ts) } else { Value::Null });
                } else {
                    ws.entry("try_done".to_string())
                        .or_insert_with(|| json!({}))
                        .as_object_mut()
                        .ok_or_else(|| CreativeError::faulted("'NoneType' object does not support item assignment"))?
                        .insert(nid.to_string(), json!(done));
                }
            }
            "update" => {
                if let Some(t) = d.get("title") {
                    ws.insert("title".into(), t.clone());
                }
            }
            "delete" => {
                ws.insert("deleted".into(), json!(true));
            }
            _ => return Err(unknown_op(op)),
        }
        ws.insert("updated_at".into(), json!(ev.ts));
        Ok(())
    }
}
