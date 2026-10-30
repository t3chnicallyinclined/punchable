//! The worker's creative-store routes: the event lines of a store (`creative_api.py` `w_creative`) and the attach
//! events the memo linker posts (`ingest_api.py` `post_event`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// `GET /api/worker/creative?store=<name>[&snapshot=1]`: a store's event lines as written, and with `snapshot=1`
/// the replayed items (for readers without the creative package).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct CreativeLines {
    /// `storyboards`, `workingsets`, `venues`, `merges` or `suggestions`.
    pub store: String,
    /// Lines in the log.
    pub total: u64,
    /// Each event line's JSON text, exactly as written (`{ts, op, id, by, data}`).
    pub lines: Vec<String>,
    /// The replayed items by id (a `creative::Board` for storyboards, a [`MergeRecord`] for merges, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Map<String, Value>>,
}

/// `POST /api/worker/events`: a machine judgment on the `merges` store (`attach` or `detach`), by an `ai:` actor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct EventRequest {
    /// Always `"merges"`.
    pub store: String,
    /// `attach` or `detach`.
    pub op: String,
    /// `at:<6-40 alphanumerics>`.
    pub id: String,
    /// The attach's data.
    pub data: AttachData,
    /// `ai:<what>` (`ai:attach`, `ai:judge`).
    pub actor: String,
}

/// An attach event's data (`creative/memolink.py` `attach_event`): the note and the bit it belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AttachData {
    /// The note (a joke node) being filed.
    pub joke: String,
    /// The bit it is filed under.
    pub into: String,
    /// The note's words (legal-checked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// The matcher's score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// `memo`, `app`, `gdocs`, ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Why, in plain words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasons: Option<Vec<String>>,
}

/// The answer to an event post: the record written, or `already` when the store was in that state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct EventResponse {
    /// Always `true`.
    pub ok: bool,
    /// The event's id.
    pub id: String,
    /// The merge store's record after the event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<MergeRecord>,
    /// Nothing was written: an attach already attached, or a detach of nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub already: Option<bool>,
}

/// A merge-store record (`creative/merge.py` `store_apply`), without its `snapshot`: a merge, an attach or a
/// dismiss. Open: each kind carries its own fields (`main`, `others`, `joke`, `into`, `text`, `jokes`, ...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct MergeRecord {
    /// `mg:`, `at:` or the dismiss id.
    pub id: String,
    /// `merge`, `attach` or `dismiss`.
    pub kind: String,
    /// `merged` / `undone`, `attached` / `detached`, `dismissed`.
    pub status: String,
    /// Who wrote it.
    pub by: Option<String>,
    /// When it was written.
    pub created_at: String,
    /// When it last changed.
    pub updated_at: String,
    /// The kind's own fields.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
