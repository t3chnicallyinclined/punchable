//! His voice profile (`profile_api.profile_apply`): one document, `profile:voice`. `put` saves his text and keeps
//! the previous version for Undo; `learn` folds the lines he kept or passed over into the `learned` section
//! ([`super::learned`]) and touches nothing else.

use serde_json::{Value, json};

use super::learned;
use super::unknown_op;
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::pyish::{get, get_or_null, key, or};
use crate::store::{Items, Reducer};
use pk_core::py;

/// The one document id (`VOICE_ID`).
pub const VOICE_ID: &str = "profile:voice";

/// The profile reducer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Profile;

impl Reducer for Profile {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, i, d) = (ev.op.as_str(), ev.id.as_str(), ev.data()?);
        if i != VOICE_ID {
            return Err(CreativeError::refused(format!("profiles live on {VOICE_ID}")));
        }
        let cur = super::item(items, i).cloned();
        match op {
            "put" => {
                let Some(text) = d.get("text").filter(|t| t.is_string()) else {
                    return Err(CreativeError::refused("text must be a string"));
                };
                let previous = match &cur {
                    Some(c) if !c.is_empty() && get(c, "text").is_some() => json!({
                        "text": key(c, "text")?, "updated_at": key(c, "updated_at")?, "by": get_or_null(c, "by"),
                    }),
                    _ => Value::Null,
                };
                let learned = cur
                    .as_ref()
                    .and_then(|c| or(c, "learned"))
                    .cloned()
                    .unwrap_or_else(learned::empty);
                items.insert(
                    i.to_string(),
                    json!({
                        "id": i, "kind": py::str(or(d, "kind").unwrap_or(&json!("voice"))), "text": text,
                        "updated_at": ev.ts, "by": ev.by, "previous": previous, "learned": learned,
                    }),
                );
            }
            "learn" => {
                let is_list = |k: &str| d.get(k).is_none_or(Value::is_array);
                if !is_list("kept") || !is_list("dropped") {
                    return Err(CreativeError::refused("kept and dropped must be lists"));
                }
                if or(d, "kept").is_none() && or(d, "dropped").is_none() {
                    return Err(CreativeError::refused("a learn event names at least one line"));
                }
                let mut base = cur.unwrap_or_else(|| {
                    json!({"id": i, "kind": "voice", "text": null, "updated_at": null, "by": null, "previous": null})
                        .as_object()
                        .cloned()
                        .expect("an object")
                });
                let merged = learned::merge(base.get("learned"), d.get("kept"), d.get("dropped"), Some(&ev.ts));
                base.insert("learned".into(), merged);
                items.insert(i.to_string(), Value::Object(base));
            }
            _ => return Err(unknown_op(op)),
        }
        Ok(())
    }
}
