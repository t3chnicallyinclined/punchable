//! Intents (`say_api.intent_apply`): what the one input bar decided a line was. The classifier's guess is the app's
//! (`by: ai:intent`); an override is his tap, never the app's; `acted` records what it became.

use serde_json::{Value, json};

use super::{item_mut, unknown_op};
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::pyish::{get_or_null, or_else};
use crate::store::{Items, Reducer};

/// What a line can be (`KINDS`).
pub const KINDS: [&str; 3] = ["question", "bit", "command"];
/// The classifier's actor id.
pub const AI_ACTOR: &str = "ai:intent";

/// The intent reducer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Intents;

fn kind_of(d: &crate::pyish::Obj) -> CreativeResult<&str> {
    d.get("kind")
        .and_then(Value::as_str)
        .filter(|k| KINDS.contains(k))
        .ok_or_else(|| CreativeError::refused("kind must be question, bit or command"))
}

impl Reducer for Intents {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, i, d) = (ev.op.as_str(), ev.id.as_str(), ev.data()?);
        let by_ai = ev.by_text().starts_with("ai:");
        if op == "classify" {
            if items.contains_key(i) {
                return Err(CreativeError::refused(format!("intent {i} exists")));
            }
            let kind = kind_of(d)?;
            if !by_ai {
                return Err(CreativeError::refused(
                    "a classification is the app's judgment: by must be ai:*",
                ));
            }
            items.insert(
                i.to_string(),
                json!({
                    "id": i, "text": or_else(d, "text", json!("")), "guess": kind, "rule": get_or_null(d, "rule"),
                    "kind": kind, "context": or_else(d, "context", Value::Null), "command": get_or_null(d, "command"),
                    "by": ev.by, "at": ev.ts, "overridden": false, "override_by": null, "ref": null,
                }),
            );
            return Ok(());
        }
        let Some(it) = item_mut(items, i) else {
            return Err(CreativeError::refused(format!("no intent {i}")));
        };
        match op {
            "override" => {
                let kind = kind_of(d)?;
                if by_ai {
                    return Err(CreativeError::refused("an override is his tap, never the app's"));
                }
                it.insert("kind".into(), json!(kind));
                it.insert("overridden".into(), json!(true));
                it.insert("override_by".into(), ev.by.clone());
            }
            "acted" => {
                it.insert("ref".into(), get_or_null(d, "ref"));
            }
            _ => return Err(unknown_op(op)),
        }
        Ok(())
    }
}
