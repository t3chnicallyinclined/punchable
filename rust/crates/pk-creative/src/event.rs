//! One line of a creative event log: `{ts, op, id, by, data}`, the closed shape every store shares
//! (`creative/store.py`). `by` is the owner's id for his own actions and `ai:<what>` / `import:<what>` otherwise.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{CreativeError, CreativeResult};
use crate::pyish::Obj;
use pk_core::py;

static EMPTY: LazyLock<Obj> = LazyLock::new(Map::new);

/// One event. `by` and `data` are kept as the JSON they were (a line is never rewritten); `data` is read through
/// [`Event::data`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// When, ISO 8601 with offset (`now_iso()` in Python).
    pub ts: String,
    /// What: one of the store's ops.
    pub op: String,
    /// Which item.
    pub id: String,
    /// Who: the owner's id, `ai:<what>`, `import:<what>`; `null` when a line had none.
    #[serde(default)]
    pub by: Value,
    /// The op's payload, normally an object.
    #[serde(default)]
    pub data: Value,
}

impl Event {
    /// An event with every field given.
    pub fn new(op: &str, id: &str, data: Value, by: &str, ts: &str) -> Self {
        Self {
            ts: ts.to_string(),
            op: op.to_string(),
            id: id.to_string(),
            by: Value::String(by.to_string()),
            data,
        }
    }

    /// The payload as the reducers read it: `ev.get("data") or {}`. A falsy payload is the empty object; a truthy
    /// non-object is the fault the Python raises on its first `.get`.
    pub fn data(&self) -> CreativeResult<&Obj> {
        match &self.data {
            Value::Object(o) => Ok(o),
            v if !py::truthy(Some(v)) => Ok(&EMPTY),
            v => Err(CreativeError::faulted(format!(
                "'{}' object has no attribute 'get'",
                py::type_name(v)
            ))),
        }
    }

    /// `by` when it is a string.
    pub fn by_str(&self) -> Option<&str> {
        self.by.as_str()
    }

    /// `str(ev.get("by") or "")`: the text the actor checks run on.
    pub fn by_text(&self) -> String {
        if py::truthy(Some(&self.by)) {
            py::str(&self.by)
        } else {
            String::new()
        }
    }

    /// One parsed log line. A line that is not an object with `ts`, `op` and `id` is an error (the Python crashes
    /// on it during replay).
    pub fn from_value(v: Value) -> CreativeResult<Self> {
        serde_json::from_value(v).map_err(|e| CreativeError::faulted(e.to_string()))
    }

    /// The line as this side writes it: compact JSON. Equality with the Python's line is as parsed JSON, not bytes
    /// (docs/RUST.md, fact 3).
    pub fn line(&self) -> String {
        serde_json::to_string(self).expect("an event is always serialisable")
    }
}
