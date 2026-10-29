//! "Not a joke?" answers (`suggest_api.store_apply`): his `keep` ("it is a joke, never suggest it again") per joke
//! id, and its `unkeep`.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::json;

use super::unknown_op;
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::pyish::or_else;
use crate::store::{Items, Reducer};

/// `ID_RE`: a joke id. The optional final newline is Python's `$`, which matches before one.
static ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^joke:[A-Za-z0-9:_@.+\-]{1,290}\n?$").expect("a fixed pattern"));

/// The suggestions reducer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Suggestions;

impl Reducer for Suggestions {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, i) = (ev.op.as_str(), ev.id.as_str());
        if !ID_RE.is_match(i) {
            return Err(CreativeError::refused("id must be a joke id"));
        }
        match op {
            "keep" => {
                let kind = or_else(ev.data()?, "kind", json!("not_a_joke"));
                items.insert(
                    i.to_string(),
                    json!({"joke_id": i, "kept": true, "kind": kind, "by": ev.by, "at": ev.ts}),
                );
            }
            "unkeep" => {
                items.shift_remove(i);
            }
            _ => return Err(unknown_op(op)),
        }
        Ok(())
    }
}
