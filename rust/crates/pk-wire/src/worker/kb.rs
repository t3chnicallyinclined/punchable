//! The worker's knowledge-base routes (`api-dev/ingest_api.py`, `creative_api.py` `w_decisions`): the decision
//! lines, an imported idea, a decision or an undo by an import actor, and the joke page gsync reads.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::api::{EdgeView, Laugh, Reference, Written};

/// A decision line (`docs/KB_SCHEMA.md`; `tools/explorer/kb.py`): open, because the replay is a dict merge and
/// the app's history rows add `index` and `undone` to the same object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Decision {
    /// When it was made (ISO, seconds, UTC).
    pub ts: String,
    /// `node` or `edge`.
    pub target: String,
    /// The node or edge id; a line without one is ignored by the replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// `confirm`, `reject`, `edit`, `add`, `delete`.
    pub action: String,
    /// The fields before (an add's is `null`).
    pub before: Option<Value>,
    /// The fields after (a delete's is `null`).
    pub after: Option<Value>,
    /// Why, in his words or the import's tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The owner id, `import:<source>` or `ai:<what>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// The line this one undoes (its index).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_of: Option<u64>,
    /// Anything else on the line (`index` and `undone` on a history row, `auto`, ...).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `GET /api/worker/decisions?after=N`: a page of up to 2,000 lines from `after`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct DecisionsPage {
    /// Lines in the file.
    pub total: u64,
    /// The page's first index.
    pub after: u64,
    /// The lines, in file order.
    pub lines: Vec<DecisionLine>,
}

/// One line of the page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct DecisionLine {
    /// Its index in the file.
    pub index: u64,
    /// The line.
    pub decision: Decision,
    /// SHA-256 of the line's canonical JSON (sorted keys, no spaces), bare hex: its identity across backends.
    /// (The goldens carry it as `sha256:<hex>` so the public repo's commit guard reads it as a hash.)
    pub h: String,
}

/// `POST /api/worker/ideas`: a new idea by an import or an AI actor (`worker/online.py`, `gsync/punchable.py`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct IdeaRequest {
    /// The idea's words (up to 4,000 characters).
    pub text: String,
    /// A title; the first seven words when empty or absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `import:<source>` or `ai:<what>`; required on this route.
    pub actor: String,
    /// Where it came from: the API keeps `set`, `perf`, `start_ms`, `end_ms`, `note` and drops the rest
    /// (`set_text` sends `unit` too, which is dropped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Map<String, Value>>,
    /// The decision line's reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The answer to an idea post, the worker's and the app's (`server.py` `idea_result`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct IdeaResponse {
    /// Always `true`.
    pub ok: bool,
    /// The appended line's index.
    pub indexes: Vec<u64>,
    /// The new joke's id (`joke:<slug>`).
    pub id: String,
    /// The appended line.
    pub decisions: Vec<Decision>,
    /// The auto-attach's verdict (`merge_api.py` `after_idea`); `null` when the creative layer is off.
    pub attach: Option<Attach>,
    /// The voice profile's learned counts: the key is absent on a worker's post, present on the owner's own post
    /// (`null` when the line was no draft of the app's).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::nullable::deserialize"
    )]
    pub learned: crate::nullable::Nullable<crate::worker::voice::LearnedCounts>,
}

/// Where a new idea went: filed under an existing bit (`attach`), too close to call (`ambiguous`), or its own (`new`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Attach {
    /// `attach`, `ambiguous` or `new`.
    pub decision: String,
    /// Why, in plain words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasons: Option<Vec<String>>,
    /// The attach record's id (`at:<8 hex>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The bit it was filed under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
    /// That bit's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// How to undo it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo: Option<AttachUndo>,
    /// The merge store's record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<crate::worker::creative::MergeRecord>,
    /// `"<Type>: <message>"` when the attach failed (the idea stands).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The undo of an attach: `POST /api/merge/undo {merge_id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AttachUndo {
    /// The route.
    #[serde(rename = "POST")]
    pub post: String,
    /// The attach record's id.
    pub merge_id: String,
}

/// `POST /api/worker/decision`: the explorer's request plus the actor. PC-only (gsync), deleted at T11.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct DecisionRequest {
    /// `node` or `edge`.
    pub target: String,
    /// `confirm`, `reject`, `edit`, `add`, `delete`.
    pub action: String,
    /// The node or edge (an add has none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The fields to set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
    /// Why (an edge add needs one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// `import:<source>` or `ai:<what>`.
    pub actor: String,
}

/// `POST /api/worker/undo`: line indexes to undo, newest first. PC-only (gsync), deleted at T11.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct UndoRequest {
    /// A non-empty list of line numbers.
    pub indexes: Vec<u64>,
    /// `import:<source>` or `ai:<what>`.
    pub actor: String,
}

/// The answer to a worker decision or undo: the same as the app's.
pub type DecisionResponse = Written;

/// `GET /api/worker/jokes/<id>` and the app's `GET /api/jokes/<id>` (`server.py` `Handler.joke`): the joke page.
/// PC-only on the worker side (gsync reads it), deleted at T11.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct JokePage {
    /// The joke node, public fields (open: the pipeline's fields ride along).
    pub joke: Map<String, Value>,
    /// He added it himself.
    pub yours: bool,
    /// The newest version's text, split so the reveal can be highlighted.
    pub text: JokeText,
    /// Distinct sets it was performed in, and where it hit.
    pub rooms: Rooms,
    /// The AI's stage and whether it still stands.
    pub ai: StageAi,
    /// `ai`, `confirmed` or `edited`.
    pub who: String,
    /// One row per live performance: the comic's rating, else the AI's.
    pub ai_ratings: Vec<AiRating>,
    /// The written versions, grouped by date.
    pub versions: Vec<VersionGroup>,
    /// Every performance, with its laughs, punch and play window.
    pub performances: Vec<Performance>,
    /// The `has_label` edges, AI state first.
    pub labels: Vec<EdgeView>,
    /// Every other edge, AI state first.
    pub links: Vec<EdgeView>,
    /// The last 30 decision lines touching the joke (each with `index` and `undone` in `extra`).
    pub history: Vec<Decision>,
}

/// The newest version's text: `reveal` is its last sentence (a display heuristic).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct JokeText {
    /// Everything before the reveal.
    pub setup: String,
    /// The last sentence.
    pub reveal: String,
    /// Which unit it is; `null` when the text is the summary or title.
    pub from: Option<TextFrom>,
}

/// The unit a joke page's text came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TextFrom {
    /// The unit id.
    pub unit: Option<String>,
    /// The unit's date.
    pub date: Option<String>,
    /// The doc it was written in.
    pub doc_title: Option<String>,
}

/// Distinct sets a joke was performed in and where it hit (`AppIndex.joke_rooms`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Rooms {
    /// Sets with a non-rejected `performed_as`.
    pub sets: u64,
    /// Sets where he confirmed the match and rated it killed or worked.
    pub hit_sets: u64,
    /// `hit_sets >= 3`.
    pub ready_for_working: bool,
    /// Sets where his rating, else the AI's, is killed or worked.
    pub ai_hit_sets: u64,
}

/// The AI's stage on a joke (`AppIndex.stage_ai`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct StageAi {
    /// The stage shown is the AI's and he has not touched it.
    pub stage_ai: bool,
    /// The AI's confidence, while `stage_ai`.
    pub stage_confidence: Option<f64>,
    /// The AI's reason, while `stage_ai`.
    pub stage_reason_ai: Option<String>,
    /// The graph's stage before the AI moved it, while `stage_ai`.
    pub from_stage: Option<String>,
    /// He confirmed the AI's stage.
    pub stage_confirmed: bool,
}

/// One live performance's rating on the joke page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AiRating {
    /// The set id.
    pub set: Option<String>,
    /// The set's date.
    pub date: Option<String>,
    /// The set's title.
    pub title: Option<String>,
    /// The performance id.
    pub perf: String,
    /// The effective rating: his, else the AI's.
    pub rating: Option<String>,
    /// `"you"` or `"ai"`.
    pub rating_by: Option<String>,
    /// The AI's rating.
    pub ai_rating: Option<String>,
    /// The strongest laugh's bars.
    pub max_bars: u64,
}

/// The written versions of one date.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct VersionGroup {
    /// The date, or `"undated"`.
    pub date: String,
    /// The units of that date.
    pub units: Vec<UnitVersion>,
}

/// A unit node plus its `part_of` edge view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct UnitVersion {
    /// The `part_of` edge.
    pub edge: EdgeView,
    /// The unit node's public fields (open).
    #[serde(flatten)]
    pub node: Map<String, Value>,
}

/// One performance on the joke page (`API.md` "Perf").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Performance {
    /// The performance node's public fields (open).
    pub perf: Map<String, Value>,
    /// The set node's public fields (open), or `{id}` when the set is unknown.
    pub set: Map<String, Value>,
    /// The `performed_as` edge.
    pub edge: EdgeView,
    /// The laughs in its span.
    pub laughs: Vec<Laugh>,
    /// The strongest laugh's bars.
    pub max_bars: u64,
    /// Laughs counted.
    pub hits: u64,
    /// His rating.
    pub rating: Option<String>,
    /// The AI's rating.
    pub ai_rating: Option<String>,
    /// Why the AI rated it so (`""` when it did not).
    pub rating_basis: String,
    /// His rating, else the AI's.
    pub effective_rating: Option<String>,
    /// `"you"` or `"ai"`.
    pub rating_by: Option<String>,
    /// `bit`, `crowd_work` or `banter`.
    pub kind: Option<String>,
    /// The laugh meter's suggestion: killed / worked / meh / died.
    pub suggested: String,
    /// The line said just before the strongest laugh.
    pub punch: Punch,
    /// From 4 s before the punch to 1.2 s after the laugh.
    pub play: Play,
    /// `/api/audio/<set>` when the tape is here.
    pub audio: Option<String>,
    /// Where the recording lives.
    pub reference: Option<Reference>,
}

/// The punch of a performance: text and span.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Punch {
    /// The transcript segment(s).
    pub text: String,
    /// Start of the segment.
    pub start_ms: Option<i64>,
    /// End of the segment.
    pub end_ms: Option<i64>,
}

/// A play window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Play {
    /// Start.
    pub start_ms: i64,
    /// End.
    pub end_ms: i64,
}
