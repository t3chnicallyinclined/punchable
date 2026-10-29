//! A decision: one line of the append-only log. Kept as a JSON object so the replay sees every key the Python wrote
//! (and `"undo_of" in d` stays a presence test, which an `Option` field cannot express).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::model::{Rec, truthy_str};

/// `{ts, target, id, action, before, after, reason, by, undo_of?}`, plus whatever else the line carried.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Decision(pub Map<String, Value>);

impl Decision {
    /// `edge` | `node` (or anything else, which the replay ignores).
    pub fn target(&self) -> Option<&str> {
        self.str("target")
    }

    /// The node or edge id.
    pub fn id(&self) -> Option<&str> {
        truthy_str(self.0.get("id"))
    }

    /// `add` | `delete` | `confirm` | `reject` | `edit`.
    pub fn action(&self) -> Option<&str> {
        self.str("action")
    }

    /// The after image when it is an object, else an empty one (the Python `after if isinstance(dict) else {}`).
    pub fn after(&self) -> Rec {
        self.obj("after")
    }

    /// The before image, same rule.
    pub fn before(&self) -> Rec {
        self.obj("before")
    }

    /// The line's reason, if a string.
    pub fn reason(&self) -> Option<&str> {
        self.str("reason")
    }

    /// Who wrote it, if a string.
    pub fn by(&self) -> Option<&str> {
        self.str("by")
    }

    /// The timestamp, if a string.
    pub fn ts(&self) -> Option<&str> {
        self.str("ts")
    }

    /// Whether the line carries an `undo_of` key at all (even null): the Python `"undo_of" in d`.
    pub fn has_undo_of(&self) -> bool {
        self.0.contains_key("undo_of")
    }

    /// `undo_of` when it is an integer (not a bool, not a float).
    pub fn undo_of(&self) -> Option<usize> {
        match self.0.get("undo_of") {
            Some(Value::Number(n)) => n.as_u64().and_then(|u| usize::try_from(u).ok()),
            _ => None,
        }
    }

    fn str(&self, k: &str) -> Option<&str> {
        self.0.get(k).and_then(Value::as_str)
    }

    fn obj(&self, k: &str) -> Rec {
        match self.0.get(k) {
            Some(Value::Object(o)) => o.clone(),
            _ => Rec::new(),
        }
    }
}

/// The legacy importer lines: written with the owner's `by` before imports named their own actor (2026-10-27).
/// `(reason prefix, the real actor)`.
pub const LEGACY_IMPORT_REASONS: [(&str, &str); 2] =
    [("[gdocs] voice memo", "import:memo"), ("[gdocs]", "import:gsync")];

/// Who made a decision: `by`, except a legacy importer line stamped with the owner, whose reason says who it was.
/// His undo of an import is his own action.
pub fn actor_of<'a>(d: &'a Decision, owner_id: &str) -> Option<&'a str> {
    let by = d.by()?;
    if by == owner_id {
        let r = d.reason().unwrap_or("");
        for (prefix, actor) in LEGACY_IMPORT_REASONS {
            if r.starts_with(prefix) {
                return Some(actor);
            }
            if r.starts_with(&format!("undo: {prefix}")) {
                return Some(by);
            }
        }
    }
    Some(by)
}
