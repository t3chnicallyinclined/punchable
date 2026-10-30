//! Storyboards (`creative/story.py::_apply`): a titled, ordered list of joke ids with a target runtime. `source`
//! is `his` once he has touched the board, `inferred` while only the AI has.

use serde_json::{Value, json};

use super::{item_mut, list_mut, unknown_op};
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::pyish::{get, get_or_null, key, or, or_else};
use crate::store::{Items, Reducer};
use pk_core::py;

/// The storyboard reducer; `owner` is the actor id of his own edits.
#[derive(Debug, Clone)]
pub struct Storyboards {
    owner: String,
}

impl Storyboards {
    /// A reducer that calls `owner_id`'s edits his.
    pub fn new(owner_id: &str) -> Self {
        Self {
            owner: owner_id.to_string(),
        }
    }
}

impl Reducer for Storyboards {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, i, d) = (ev.op.as_str(), ev.id.as_str(), ev.data()?);
        let his = ev.by_str() == Some(self.owner.as_str());
        if op == "create" {
            if items.contains_key(i) {
                return Err(CreativeError::refused(format!("storyboard {i} exists")));
            }
            let title = or_else(d, "title", json!("Untitled"));
            let joke_ids = match or(d, "joke_ids") {
                Some(v) => py::list(v)?,
                None => Vec::new(),
            };
            let target_s = py::int(Some(or(d, "target_s").unwrap_or(&json!(900))))?;
            items.insert(
                i.to_string(),
                json!({
                    "id": i, "title": title, "joke_ids": joke_ids, "target_s": target_s,
                    "notes": or_else(d, "notes", json!("")), "source": if his { "his" } else { "inferred" },
                    "by": ev.by, "seed": get_or_null(d, "seed"), "created_at": ev.ts, "updated_at": ev.ts,
                    "deleted": false,
                }),
            );
            return Ok(());
        }
        let board = match item_mut(items, i) {
            Some(board) if !py::truthy(board.get("deleted")) => board,
            _ => return Err(CreativeError::refused(format!("no storyboard {i}"))),
        };
        if !board.contains_key("joke_ids") {
            return Err(CreativeError::missing("joke_ids"));
        }
        match op {
            "update" => {
                for k in ["title", "target_s", "notes"] {
                    if let Some(v) = d.get(k) {
                        let v = if k == "target_s" {
                            json!(py::int(Some(v))?)
                        } else {
                            v.clone()
                        };
                        board.insert(k.into(), v);
                    }
                }
            }
            "set_jokes" => {
                let ids = py::list(key(d, "joke_ids")?)?;
                board.insert("joke_ids".into(), Value::Array(ids));
            }
            "add" => {
                let joke = key(d, "joke_id")?;
                let ids = list_mut(board, "joke_ids")?;
                if ids.contains(joke) {
                    return Err(CreativeError::refused(format!(
                        "{} already on the board",
                        py::str(joke)
                    )));
                }
                let at = match get(d, "at") {
                    None => ids.len(),
                    Some(at) => usize::try_from(py::int(Some(at))?.max(0))
                        .unwrap_or(usize::MAX)
                        .min(ids.len()),
                };
                ids.insert(at, joke.clone());
            }
            "remove" => {
                let joke = key(d, "joke_id")?;
                let ids = list_mut(board, "joke_ids")?;
                let Some(pos) = ids.iter().position(|x| x == joke) else {
                    return Err(CreativeError::refused(format!("{} not on the board", py::str(joke))));
                };
                ids.remove(pos);
            }
            "move" => {
                let from = py::int(Some(key(d, "from")?))?;
                let to = py::int(Some(key(d, "to")?))?;
                let ids = list_mut(board, "joke_ids")?;
                let n = i64::try_from(ids.len()).unwrap_or(i64::MAX);
                if !(0 <= from && from < n && 0 <= to && to < n) {
                    return Err(CreativeError::refused("move out of range"));
                }
                let x = ids.remove(from as usize);
                ids.insert(to as usize, x);
            }
            "delete" => {
                board.insert("deleted".into(), json!(true));
            }
            _ => return Err(unknown_op(op)),
        }
        if his {
            board.insert("source".into(), json!("his"));
        }
        board.insert("updated_at".into(), json!(ev.ts));
        Ok(())
    }
}
