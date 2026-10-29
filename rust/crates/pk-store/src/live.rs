//! One `LIVE` subscription over the tables the app watches (`meta`, `joke`, `decision`, `creative_event`), each
//! notification reduced to the change the `/api/live` socket forwards (`surreal_kb.LiveHub._on`). Coalescing into
//! `{type: change, rev, changes}` events is the API's job; this yields the changes.

use std::pin::Pin;

use futures::{Stream, StreamExt};
use serde::Serialize;
use surrealdb::Notification;
use surrealdb::types::{Object, Value};

use crate::error::Result;
use crate::ids::{rid_str, table_of};
use crate::kbmap::working_of;
use crate::store::Store;
use crate::value::{as_i64, as_str};

/// The tables subscribed to, in order.
pub const LIVE_TABLES: [&str; 4] = ["meta", "joke", "decision", "creative_event"];

/// One change, as the app receives it. Table-specific fields are `None` for the other tables.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    /// The table.
    pub table: Option<String>,
    /// `create` | `update` | `delete` | `killed` | `error`.
    pub action: String,
    /// The record's KB id (`joke:x`, `decision:...`).
    pub id: Option<String>,
    /// `joke`: the working flag after the change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working: Option<bool>,
    /// `joke`: how many sets it hit in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hit_sets: Option<usize>,
    /// `creative_event`: the store.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    /// `creative_event`: the op.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    /// `creative_event`: the item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// `meta`: the generation after the change (`gen` on the wire).
    #[serde(rename = "gen", skip_serializing_if = "Option::is_none")]
    pub generation: Option<i64>,
    /// `meta`: a reload is running (one event when it ends).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loading: Option<bool>,
}

impl Change {
    /// The change a notification carries. The record is the notification's data when it is an object (with its
    /// `id`), or just an id for a delete.
    pub fn from_notification(n: &Notification<Value>) -> Self {
        let empty = Object::new();
        let (rec, rid) = match &n.data {
            Value::Object(o) => (
                o,
                match o.get("id") {
                    Some(Value::RecordId(r)) => Some(r.clone()),
                    _ => None,
                },
            ),
            Value::RecordId(r) => (&empty, Some(r.clone())),
            _ => (&empty, None),
        };
        let table = rid.as_ref().map(table_of);
        let mut ch = Self {
            table: table.clone(),
            action: n.action.to_string().to_lowercase(),
            id: rid.as_ref().map(rid_str),
            working: None,
            hit_sets: None,
            store: None,
            op: None,
            item: None,
            generation: None,
            loading: None,
        };
        let s = |k: &str| rec.get(k).and_then(as_str).map(str::to_string);
        match table.as_deref() {
            Some("joke") => {
                let (w, sets) = working_of(rec);
                ch.working = Some(w);
                ch.hit_sets = Some(sets.len());
            }
            Some("creative_event") => {
                ch.store = s("store");
                ch.op = s("op");
                ch.item = s("item");
            }
            Some("meta") => {
                ch.generation = rec.get("gen").and_then(as_i64);
                ch.loading = match rec.get("loading") {
                    Some(Value::Bool(b)) => Some(*b),
                    _ => None,
                };
            }
            _ => {}
        }
        ch
    }
}

/// The subscription: changes as they happen, or the error that ended it.
pub type Changes = Pin<Box<dyn Stream<Item = Result<Change>> + Send>>;

impl Store {
    /// `LIVE SELECT * FROM` each of [`LIVE_TABLES`], on this connection, as one stream of changes.
    pub async fn subscribe(&self) -> Result<Changes> {
        let sql = LIVE_TABLES
            .iter()
            .fold(String::new(), |acc, t| acc + "LIVE SELECT * FROM " + t + ";");
        let mut resp = self.db().query(sql).await?;
        let stream = resp.stream::<Notification<Value>>(())?;
        Ok(Box::pin(stream.map(|r| {
            r.map(|n| Change::from_notification(&n))
                .map_err(crate::error::StoreError::from)
        })))
    }
}
