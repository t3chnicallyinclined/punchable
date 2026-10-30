//! The shapes `app/src/lib/creative.ts` exports, as the Python creative routes write them
//! (`api-dev/creative_api.py` over `creative/`): connections, storyboards, working sets, nights, bit history,
//! venues, the today feed, clips, jobs and the four drafts (bridge, punch-up, ask, and `simple`'s riff / facts).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::api::{BestPick, BriefJoke, Evidence, SetCard};
use crate::nullable::Nullable;
use crate::worker::JobBrief;

/// Who an item comes from: `his` = he confirmed or wrote it, `inferred` = AI-accepted, `proposed` = a suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Source {
    /// The comic's own.
    His,
    /// Accepted by the AI.
    Inferred,
    /// A suggestion.
    Proposed,
}

/// `creative.ts`'s name for a verbatim quote.
pub type Ev = Evidence;

/// A bit connected to the one on the page (`creative/connections.py`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ConnItem {
    /// The connected bit's id (the same as `joke_id`).
    pub id: String,
    /// The connected bit.
    pub joke_id: String,
    /// Its title.
    pub title: String,
    /// Its stage.
    pub stage: Option<String>,
    /// 0 to 1 within the group.
    pub score: f64,
    /// Why, in plain words.
    pub why: String,
    /// Who says so.
    pub source: Source,
    /// What the score rests on (`back_to_back`, `segue`, ...).
    pub basis: Vec<String>,
    /// Verbatim quotes.
    pub evidence: Vec<Evidence>,
    /// The edge behind it, when one is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<String>,
    /// Nights it followed this bit on tape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nights: Option<u64>,
    /// The unit a tag line comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_id: Option<String>,
    /// The line itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    /// Shared personas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personas: Option<Vec<String>>,
    /// Shared topics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topics: Option<Vec<String>>,
    /// Shared labels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared: Option<Vec<String>>,
    /// `in` or `out`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
}

/// The six connection groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum ConnGroupKey {
    /// What could come next.
    Next,
    /// Lines that could tag it.
    Tags,
    /// Same persona or premise.
    SameWorld,
    /// Callbacks.
    Callbacks,
    /// Same shape.
    SameShape,
    /// Same topic, other angle.
    Contrasts,
}

/// One group of connections.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ConnGroup {
    /// The group.
    pub key: ConnGroupKey,
    /// Its heading.
    pub title: String,
    /// The bits.
    pub items: Vec<ConnItem>,
}

/// One night of a bit's stage trend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TrendNight {
    /// The show date.
    pub date: String,
    /// The set key (`<date>_<id>`).
    pub set: String,
    /// The room.
    pub venue: String,
    /// "<room> '24".
    pub label: String,
    /// Laugh-seconds per minute.
    pub laugh_s_per_min: f64,
    /// Against the room's own rate.
    pub vs_room: Option<f64>,
    /// The AI's rating.
    pub ai_rating: Option<String>,
    /// Laugh-seconds.
    pub laugh_s: f64,
    /// The bit's length that night.
    pub duration_s: f64,
    /// Seconds to the first laugh.
    pub time_to_first_laugh_s: Option<f64>,
    /// The transcript segments.
    pub seg_ids: Vec<String>,
}

/// `GET /api/jokes/<id>/connections` (`creative/connections.py` `joke_connections`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Connections {
    /// The bit on the page.
    pub joke: ConnJoke,
    /// The six groups, in order.
    pub groups: Vec<ConnGroup>,
    /// How it has done on stage.
    pub stage_trend: StageTrend,
    /// What would land harder.
    pub land_harder: Vec<LandHarder>,
    /// Reading notes.
    pub notes: Vec<String>,
}

/// The bit a connections page is about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ConnJoke {
    /// The joke id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Its stage.
    pub stage: Option<String>,
    /// `his`, `inferred` or `proposed`.
    pub stage_source: String,
    /// Its summary.
    pub summary: Option<String>,
    /// Its device labels.
    pub devices: Vec<String>,
    /// How long it runs.
    pub duration_s: f64,
    /// Where the duration comes from.
    pub duration_basis: String,
    /// Distinct rooms.
    pub rooms: u64,
    /// Nights on tape.
    pub nights: u64,
}

/// A bit's stage trend over its nights. Beyond the TypeScript the wire carries `trend_rel_change`,
/// `vs_room_trend` and `vs_room_rel_change`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct StageTrend {
    /// The nights, oldest first.
    pub nights: Vec<TrendNight>,
    /// `up`, `flat`, `down`, or `null`.
    pub trend: Option<String>,
    /// The relative change behind `trend`.
    pub trend_rel_change: Option<f64>,
    /// The same for `vs_room`.
    pub vs_room_trend: Option<String>,
    /// The relative change behind it.
    pub vs_room_rel_change: Option<f64>,
    /// Mean laugh-seconds per minute.
    pub avg_laugh_s_per_min: Option<f64>,
    /// Mean `vs_room`.
    pub avg_vs_room: Option<f64>,
    /// The best night (a partial [`TrendNight`] plus `basis`), or `null`.
    pub best_night: Option<Map<String, Value>>,
    /// Fewer than three nights.
    pub low_n: bool,
    /// A reading note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What would land harder, with its basis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LandHarder {
    /// The advice, in plain words.
    pub text: String,
    /// What it rests on.
    pub basis: String,
    /// Who says so.
    pub source: Source,
    /// Verbatim quotes.
    pub evidence: Vec<Evidence>,
}

/// A transition's label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum TransitionLabel {
    /// Done back to back on tape.
    Proven,
    /// A link in the graph.
    Link,
    /// No link yet.
    Jump,
}

/// A transition between two bits on a board (`creative/story.py` `transition`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Transition {
    /// The bit before.
    pub from: String,
    /// The bit after.
    pub to: String,
    /// Proven, link or jump.
    pub label: TransitionLabel,
    /// Why, in plain words.
    pub why: String,
    /// The links it rests on.
    pub via: Vec<Via>,
    /// A jump that needs a bridge line.
    pub needs_bridge: bool,
    /// Topics both bits carry.
    pub shared_topics: Vec<String>,
    /// Nights they were done back to back.
    pub nights: Vec<NightRef>,
    /// Nights they were done the other way round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse_order_nights: Option<u64>,
    /// Verbatim quotes.
    pub evidence: Vec<Evidence>,
}

/// One link a transition rests on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Via {
    /// `segue`, `callback_to`, `persona`, `topic`, ...
    pub kind: String,
    /// A label's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Who says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The edge id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<String>,
    /// The edge's reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The edge points the other way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse: Option<bool>,
}

/// A night, briefly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct NightRef {
    /// The set key.
    pub set: String,
    /// The date.
    pub date: String,
    /// The room.
    pub venue: String,
}

/// A bit on a board, with its energy and heat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BoardBit {
    /// The joke id.
    pub joke_id: String,
    /// Its title.
    pub title: String,
    /// Its stage.
    pub stage: Option<String>,
    /// How long it runs.
    pub duration_s: f64,
    /// Where the duration comes from.
    pub duration_basis: String,
    /// Mean `vs_room` over taped nights; `null` with no tape.
    pub energy: Option<f64>,
    /// 1 to 4 for bars; `null` with no tape.
    pub heat: Option<i64>,
    /// Nights on tape.
    pub nights: u64,
    /// Device labels.
    pub devices: Vec<String>,
}

/// A board's health (`creative/story.py` `health`). Beyond the TypeScript the wire carries `notes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Health {
    /// The board id.
    pub board: String,
    /// Its title.
    pub title: String,
    /// Runtime in seconds.
    pub runtime_s: f64,
    /// Runtime as m:ss.
    pub runtime: String,
    /// The target in seconds.
    pub target_s: i64,
    /// The bits, in order.
    pub bits: Vec<BoardBit>,
    /// The transitions between them.
    pub transitions: Vec<Transition>,
    /// Energy and heat per bit, in order.
    pub energy_curve: Vec<EnergyPoint>,
    /// The rules checked.
    pub checks: Vec<Check>,
    /// Titles of bits with no tape.
    pub untaped: Vec<String>,
    /// How energy and heat are computed.
    pub notes: String,
}

/// One point of the energy curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct EnergyPoint {
    /// The bit.
    pub joke_id: String,
    /// Mean `vs_room`, or `null`.
    pub energy: Option<f64>,
    /// 1 to 4, or `null`.
    pub heat: Option<i64>,
}

/// One health check: `ok`, `warn` or `info`, with the rule's own extras.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Check {
    /// `runtime`, `jumps`, `repeated_devices`, `closer`, `energy_dip`.
    pub rule: String,
    /// `ok`, `warn` or `info`.
    pub status: String,
    /// In plain words.
    pub text: String,
    /// A bit to try (the closer rule).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggest: Option<Value>,
    /// Offending pairs (the jumps rule).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairs: Option<Value>,
    /// Repeated devices by pair (the `repeated_devices` rule).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeats: Option<Value>,
}

/// A storyboard record (`creative/story.py`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Board {
    /// `sb:<slug>-<8 hex>`.
    pub id: String,
    /// The title.
    pub title: String,
    /// The bits, in order.
    pub joke_ids: Vec<String>,
    /// The target in seconds.
    pub target_s: i64,
    /// His notes.
    pub notes: String,
    /// Who made it.
    pub source: Source,
    /// The actor.
    pub by: String,
    /// What seeded it (an insight chunk), or `null`.
    pub seed: Option<Value>,
    /// When it was created.
    pub created_at: String,
    /// When it last changed.
    pub updated_at: String,
    /// It was deleted.
    pub deleted: bool,
}

/// A bit that could come next on a board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BestNext {
    /// The joke id.
    pub joke_id: String,
    /// Its title.
    pub title: String,
    /// Its stage.
    pub stage: Option<String>,
    /// 0 to 1.
    pub score: f64,
    /// Why.
    pub why: String,
    /// Who says so.
    pub source: Source,
    /// Proven, link or jump from the last bit.
    pub transition: String,
    /// Mean `vs_room`, or `null`.
    pub energy: Option<f64>,
    /// Nights on tape.
    pub nights: u64,
    /// Verbatim quotes.
    pub evidence: Vec<Evidence>,
}

/// `GET /api/boards/<id>` and the board writes' answer (`creative/story.py` `board_view`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BoardView {
    /// The record.
    pub board: Board,
    /// Its health.
    pub health: Health,
    /// What could come next.
    pub best_next: Vec<BestNext>,
    /// The printable set list.
    pub setlist: Setlist,
}

/// A board as a set list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Setlist {
    /// The board id.
    pub board: String,
    /// Its title.
    pub title: String,
    /// One line per bit.
    pub lines: Vec<String>,
    /// Runtime as m:ss.
    pub runtime: String,
    /// The whole list as text.
    pub text: String,
}

/// A board in the list (`creative_api.py` `board_summary`). Beyond the TypeScript the wire carries `runtime_s`
/// and `names`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BoardListItem {
    /// The board id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Bits on it.
    pub n: u64,
    /// Runtime in seconds.
    pub runtime_s: f64,
    /// Runtime as m:ss.
    pub runtime: String,
    /// The target in seconds.
    pub target_s: i64,
    /// When it last changed.
    pub updated: String,
    /// Who made it.
    pub source: Source,
    /// Each bit's short name, in order.
    pub names: Vec<String>,
    /// The working set promoted from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_set: Option<String>,
}

/// `GET /api/boards`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Boards {
    /// Newest first.
    pub boards: Vec<BoardListItem>,
}

/// A bridge draft (`creative/story.py` `bridge_draft`), cached (`cached`, `job`) or fresh. A queued answer is
/// [`QueuedJob`] instead (see [`Queued`]). The TypeScript's `job?: Job | string` is always a job brief on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Bridge {
    /// The bit before.
    pub from: String,
    /// The bit after.
    pub to: String,
    /// Its title.
    pub from_title: String,
    /// Its title.
    pub to_title: String,
    /// Proven, link or jump.
    pub transition: String,
    /// Always `true`: not his words.
    pub draft: bool,
    /// "DRAFT · AI · not your words yet".
    pub label: String,
    /// `ok`, `no_grounding`, `provider_error`.
    pub status: String,
    /// The line, or `null`.
    pub text: Option<String>,
    /// The links it is grounded in.
    pub grounded_in: Vec<BridgeLink>,
    /// Every link offered to the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_links: Option<Vec<String>>,
    /// Why there is no line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The provider that answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The model that answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The model call's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    /// The router task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// The answer is a finished job's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
    /// That job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<JobBrief>,
}

/// A link a bridge may use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BridgeLink {
    /// `L1`, `L2`, ...
    pub id: String,
    /// `persona`, `topic`, or an edge kind.
    pub kind: String,
    /// The label (a persona or topic link).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_id: Option<String>,
    /// The edge (a direct link).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_id: Option<String>,
    /// The label's name, or the edge's reason.
    pub name: Option<String>,
    /// Verbatim quotes.
    pub evidence: Vec<Evidence>,
}

/// A bit in a working-set version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WSBit {
    /// The joke id.
    pub joke_id: String,
    /// Its title.
    pub title: String,
    /// Its target in seconds (to 5 s).
    pub target_s: i64,
    /// Where the target comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_basis: Option<String>,
}

/// A working-set record (`creative/workingset.py`). Beyond the TypeScript the wire carries `by`, `source`,
/// `deleted` and `try_notes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WorkingSet {
    /// `ws:<slug>-<8 hex>`.
    pub id: String,
    /// The title.
    pub title: String,
    /// The board it was promoted from.
    pub storyboard_id: Option<String>,
    /// The actor.
    pub by: String,
    /// Who made it.
    pub source: Source,
    /// When it was created.
    pub created_at: String,
    /// When it last changed.
    pub updated_at: String,
    /// The current version's number.
    pub current: i64,
    /// Locks by joke id.
    pub locks: Map<String, Value>,
    /// It was deleted.
    pub deleted: bool,
    /// Every version, oldest first.
    pub versions: Vec<WSVersion>,
    /// His try-tonight notes.
    pub try_notes: Vec<TryNoteRecord>,
}

/// One version of a working set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WSVersion {
    /// The version number, from 1.
    pub v: i64,
    /// When it was made.
    pub created_at: String,
    /// What changed.
    pub note: String,
    /// The target in seconds.
    pub target_s: i64,
    /// The bits, in order.
    pub bits: Vec<WSBit>,
    /// The actor.
    pub by: String,
}

/// A lock on a bit: the whole bit, or lines of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Lock {
    /// The whole bit is locked.
    pub bit: bool,
    /// Locked lines.
    pub lines: Vec<LockedLine>,
}

/// A locked line: verbatim in a taped night or written version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LockedLine {
    /// The words.
    pub text: String,
    /// The segment or unit they are in.
    pub node: String,
    /// When it was locked.
    pub ts: String,
}

/// His own try-tonight note as the store holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TryNoteRecord {
    /// `try:<8 hex>`.
    pub id: String,
    /// The bit, or `null` for the whole set.
    pub joke_id: Option<String>,
    /// The note.
    pub text: String,
    /// Ticked.
    pub done: bool,
    /// When it was written.
    pub created_at: String,
    /// When it was ticked.
    pub done_at: Option<String>,
    /// The actor.
    pub by: String,
}

/// A bit as it went on one night against the plan (`creative/workingset.py` `align`): the planning fields always,
/// the night's fields only when it was performed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AlignBit {
    /// The joke id.
    pub joke_id: String,
    /// Its title.
    pub title: String,
    /// Its place in the plan, from 1.
    pub planned_pos: i64,
    /// Its target in seconds.
    pub target_s: i64,
    /// It is locked.
    pub locked: bool,
    /// It was done that night.
    pub performed: bool,
    /// The night's fields, when performed.
    #[serde(flatten)]
    pub night: Option<AlignPerformed>,
}

/// How a planned bit went on the night.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AlignPerformed {
    /// Its place on the night, from 1.
    pub performed_pos: Option<i64>,
    /// The analysis rows it covered.
    pub segments: Vec<i64>,
    /// How long it ran.
    pub time_s: Option<f64>,
    /// Against the target.
    pub delta_s: Option<f64>,
    /// Laugh-seconds.
    pub laugh_s: Option<f64>,
    /// Laugh-seconds per minute.
    pub laugh_s_per_min: Option<f64>,
    /// Against the room.
    pub vs_room: Option<f64>,
    /// The AI's rating.
    pub ai_rating: Option<String>,
    /// The strongest laugh's bars.
    pub max_bars: Option<i64>,
    /// Seconds to the first laugh.
    pub time_to_first_laugh_s: Option<f64>,
    /// `{on_script_pct, dropped, added}` against the written version.
    pub script_diff: Option<Value>,
    /// Which version the diff is against.
    pub script_diff_basis: Option<String>,
    /// It came in the planned order.
    pub in_order: bool,
    /// Verbatim quotes.
    pub evidence: Vec<Evidence>,
}

/// One night against one working-set version (`creative/workingset.py` `align`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Alignment {
    /// The working set.
    pub working_set: String,
    /// The version.
    pub version: i64,
    /// The set key.
    pub set: String,
    /// The date.
    pub date: String,
    /// The room.
    pub venue: String,
    /// "<room> '24".
    pub venue_label: String,
    /// Planned vs performed order.
    pub order: AlignOrder,
    /// Planned bits not done.
    pub dropped: Vec<String>,
    /// Rows done that were not planned.
    pub added: Vec<AddedRow>,
    /// Every planned bit.
    pub bits: Vec<AlignBit>,
    /// The night's totals.
    pub totals: AlignTotals,
}

/// Planned vs performed order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AlignOrder {
    /// The plan.
    pub planned: Vec<String>,
    /// What he did, in order.
    pub performed: Vec<String>,
    /// Done in the planned order.
    pub in_order: Vec<String>,
    /// Done out of order.
    pub moved: Vec<String>,
    /// Nothing moved.
    pub same_order: bool,
}

/// A row done that was not planned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AddedRow {
    /// The analysis row.
    pub idx: i64,
    /// Its title.
    pub title: String,
    /// `known`, `new`, `crowd_work`, `banter`.
    pub status: String,
    /// The joke, if known.
    pub joke_id: Option<String>,
    /// How long it ran.
    pub time_s: f64,
    /// Laugh-seconds per minute.
    pub laugh_s_per_min: f64,
    /// Against the room.
    pub vs_room: Option<f64>,
    /// The AI's rating.
    pub ai_rating: Option<String>,
}

/// A night's totals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AlignTotals {
    /// The plan's target.
    pub planned_target_s: i64,
    /// Time on planned bits.
    pub performed_planned_s: f64,
    /// Time on added rows.
    pub added_s: f64,
    /// Planned bits done.
    pub bits_performed: u64,
    /// Bits planned.
    pub bits_planned: u64,
    /// The working set's laugh rate that night.
    pub ws_laugh_s_per_min: Option<f64>,
    /// The whole set's.
    pub set_laugh_s_per_min: f64,
    /// The room's heat.
    pub room_heat: f64,
    /// Material seconds.
    pub material_s: f64,
}

/// `GET /api/workingsets/<id>/nights`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Nights {
    /// The working set.
    pub working_set: String,
    /// Every taped night against the plan.
    pub nights: Vec<Alignment>,
}

/// A try-tonight note: his own (`kind: "his"`, with `id`, `done`, `created_at`) or the AI's (`put_back`,
/// `dropped`, `slow_start`, ...; `id` = `<joke_id>|<kind>` and `done` on the view's list, neither under a bit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TryNote {
    /// The note's id, where the app can tick it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The bit, or `null` for the whole set.
    pub joke_id: Option<String>,
    /// Its title.
    pub title: Option<String>,
    /// `his`, or the AI's rule.
    pub kind: String,
    /// 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// The note.
    pub text: String,
    /// Verbatim quotes.
    pub evidence: Vec<Evidence>,
    /// Who says so.
    pub source: Source,
    /// Fewer nights than the rule wants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_n: Option<bool>,
    /// Ticked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    /// When his note was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// The perfected meter (`creative/workingset.py` `perfected`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Perfected {
    /// Locked share by target time.
    pub locked_share: f64,
    /// 1 minus the CV of laugh-s/min, weighted.
    pub stability: f64,
    /// The meter, 0 to 1.
    pub perfected: f64,
    /// How it is computed.
    pub formula: String,
    /// Per bit.
    pub bits: Vec<PerfectedBit>,
}

/// The perfected meter's row for one bit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PerfectedBit {
    /// The joke id.
    pub joke_id: String,
    /// Its title.
    pub title: String,
    /// Locked share of its words.
    pub locked_share: f64,
    /// CV of laugh-s/min across nights.
    pub cv_laugh_s_per_min: Option<f64>,
    /// 1 minus the CV.
    pub stability: Option<f64>,
    /// Nights on tape.
    pub nights: u64,
    /// Fewer than two nights.
    pub low_n: bool,
}

/// A working set's status line (`creative/workingset.py` `status`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WSStatus {
    /// The working set.
    pub working_set: String,
    /// Its title.
    pub title: String,
    /// The current version.
    pub version: i64,
    /// The target in seconds.
    pub target_s: i64,
    /// The last taped night, or `null`.
    pub last_night: Option<LastNight>,
    /// `up`, `down`, `flat`, or `null`.
    pub vs_previous: Option<String>,
    /// The one line the app shows.
    pub line: String,
    /// The perfected meter, on the today feed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perfected: Option<f64>,
}

/// The last taped night of a working set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LastNight {
    /// The set key.
    pub set: String,
    /// The date.
    pub date: String,
    /// "<room> '24".
    pub venue: String,
    /// The working set's laugh rate.
    pub ws_laugh_s_per_min: Option<f64>,
    /// Planned bits done.
    pub bits_performed: u64,
    /// Bits planned.
    pub bits_planned: u64,
}

/// A bit's row on the working-set view: the version's bit plus locks, history and notes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WSBitRow {
    /// The version's bit.
    #[serde(flatten)]
    pub bit: WSBit,
    /// The whole bit is locked.
    pub locked: bool,
    /// Locked lines.
    pub locked_lines: Vec<LockedLine>,
    /// Nights on tape.
    pub nights: u64,
    /// The best night (`{date, venue, laugh_s_per_min, vs_room}`), or `null`.
    pub best_night: Option<Map<String, Value>>,
    /// `up`, `flat`, `down`, or `null`.
    pub trend: Option<String>,
    /// 1 minus the CV.
    pub stability: Option<f64>,
    /// Fewer than two nights.
    pub low_n: bool,
    /// How it went last night, or `null` with no night.
    pub last_night: Option<BitLastNight>,
    /// The AI's notes on it.
    pub notes: Vec<TryNote>,
}

/// How a bit went on the working set's last night.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BitLastNight {
    /// It was done.
    pub performed: bool,
    /// How long it ran.
    pub time_s: Option<f64>,
    /// Against the target.
    pub delta_s: Option<f64>,
    /// Laugh-seconds per minute.
    pub laugh_s_per_min: Option<f64>,
    /// Against the room.
    pub vs_room: Option<f64>,
    /// The AI's rating.
    pub ai_rating: Option<String>,
}

/// `GET /api/workingsets/<id>` and the working-set writes' answer (`creative_api.py` `ws_detail`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WSView {
    /// The record.
    pub working_set: WorkingSet,
    /// The current version.
    pub version: WSVersion,
    /// The status line.
    pub status: WSStatus,
    /// The perfected meter.
    pub perfected: Perfected,
    /// His notes first, then the AI's.
    pub try_tonight: Vec<TryNote>,
    /// One row per bit.
    pub bits: Vec<WSBitRow>,
}

/// A working set in the list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WSListItem {
    /// The id.
    pub id: String,
    /// The title.
    pub title: String,
    /// The current version.
    pub version: i64,
    /// The target in seconds.
    pub target_s: i64,
    /// Bits in the current version.
    pub n: u64,
    /// The board it came from.
    pub storyboard_id: Option<String>,
    /// When it last changed.
    pub updated: String,
    /// Who made it.
    pub source: Option<Source>,
}

/// `GET /api/workingsets`: `workingsets` is the key, `working_sets` an alias of the same list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WorkingSets {
    /// Newest first.
    pub workingsets: Vec<WSListItem>,
    /// The same list.
    pub working_sets: Vec<WSListItem>,
}

/// `GET /api/jobs/<id>`: the brief plus the result (`null` until done). The worker's own `worker::Job` is the full
/// store record (`key`, `input`, `by`, `claimed_at`, `worker`, and `waiting_for` always present).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Job<T = Value> {
    /// The brief.
    #[serde(flatten)]
    pub brief: JobBrief,
    /// The kind's result, once done.
    pub result: Option<T>,
}

/// The 202 body of a slow draft: queued for the worker (`creative_api.py` `_queued`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct QueuedJob {
    /// Always `"queued"`.
    pub status: String,
    /// The job.
    pub job: JobBrief,
    /// `/api/jobs/<id>`.
    pub poll: String,
    /// `"transcriber"` for a kind nothing on the server runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_for: Option<String>,
    /// The sentence shown while it waits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A slow draft's answer: 200 with the result (`cached: true` and `job` when it is a finished job's), or 202
/// [`QueuedJob`]. Untagged: a body with `poll` is queued, anything else is the result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[allow(clippy::large_enum_variant)] // a queued brief is small, a result is whatever the kind returns
pub enum Queued<T> {
    /// 202: queued.
    Queued(QueuedJob),
    /// 200: the result.
    Ready(T),
}

/// The older one-shot Ask's result (`engine/ask.py` `ask`; job kind `ask`). The wire's `sources` are source ids
/// (strings), not the `{node, quote, ...}` objects the TypeScript declares; `citations` carry the quotes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AskAnswer {
    /// His question.
    pub question: String,
    /// The retrieval plan (`by_year`, `kinds`, ...).
    pub plan: Map<String, Value>,
    /// The run id.
    pub run_id: Option<String>,
    /// The retrieved source ids.
    pub sources: Vec<String>,
    /// The answer is grounded.
    pub found: bool,
    /// The answer, with `[id]` citations.
    pub answer: String,
    /// What the model said it could not find.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_found: Option<String>,
    /// Verified quotes.
    pub citations: Vec<Citation>,
    /// Quotes the gate removed.
    pub removed: Vec<Value>,
    /// Seconds per phase.
    pub timings: Map<String, Value>,
    /// Prompt tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_in: Option<i64>,
    /// Answer tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_out: Option<i64>,
    /// "<provider>:<model>".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The answer is a finished job's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
    /// That job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<JobBrief>,
}

/// A verified quote in an Ask answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Citation {
    /// The source id.
    pub source_id: Option<String>,
    /// The words.
    pub quote: String,
}

/// `GET /api/workingsets/<id>/bits/<joke>` (`creative/workingset.py` `bit_history` plus the route's lock fields).
/// Beyond the TypeScript the wire carries `trend_rel_change`, `vs_room_trend`, `working_set`, `locked`,
/// `locked_lines`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BitHistory {
    /// The joke id.
    pub joke_id: String,
    /// Its title.
    pub title: String,
    /// Nights on tape.
    pub n_nights: u64,
    /// The nights, oldest first.
    pub nights: Vec<HistoryNight>,
    /// The best night (`{date, venue, laugh_s_per_min, vs_room}`), or `null`.
    pub best_night: Option<Map<String, Value>>,
    /// `up`, `flat`, `down`, or `null`.
    pub trend: Option<String>,
    /// The relative change behind it.
    pub trend_rel_change: Option<f64>,
    /// The same for `vs_room`.
    pub vs_room_trend: Option<String>,
    /// CV of laugh-s/min.
    pub cv_laugh_s_per_min: Option<f64>,
    /// 1 minus the CV.
    pub stability: Option<f64>,
    /// Fewer than two nights.
    pub low_n: bool,
    /// Wording vs laughs across nights.
    pub wording: Wording,
    /// A punch-up draft, when the route adds one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub punch_up: Option<PunchUp>,
    /// The working set.
    pub working_set: String,
    /// The whole bit is locked.
    pub locked: bool,
    /// Locked lines.
    pub locked_lines: Vec<LockedLine>,
}

/// One night in a bit's history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct HistoryNight {
    /// The date.
    pub date: String,
    /// The set key.
    pub set: String,
    /// "<room> '24".
    pub venue: String,
    /// How long it ran.
    pub time_s: f64,
    /// Laugh-seconds.
    pub laugh_s: f64,
    /// Laugh-seconds per minute.
    pub laugh_s_per_min: f64,
    /// Against the room.
    pub vs_room: Option<f64>,
    /// The AI's rating.
    pub ai_rating: Option<String>,
}

/// Wording vs laughs across nights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Wording {
    /// Phrases said on some nights and not others.
    pub effects: Vec<Effect>,
    /// Night-to-night changes.
    pub changes: Vec<Change>,
    /// A reading note.
    pub note: String,
}

/// A phrase's effect. The wire's `examples` are `{node, quote, punchline}` objects, not the strings the
/// TypeScript declares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Effect {
    /// Nights with the phrase.
    pub nights_with: Vec<String>,
    /// Nights without.
    pub nights_without: Vec<String>,
    /// Count with.
    pub n_with: u64,
    /// Count without.
    pub n_without: u64,
    /// Phrases folded into this effect.
    pub n_phrases: u64,
    /// Laugh-seconds with minus without.
    pub delta_laugh_s: f64,
    /// `vs_room` with minus without.
    pub delta_vs_room: Option<f64>,
    /// In plain words.
    pub statement: String,
    /// A hunch, not a finding.
    pub low_n: bool,
    /// Where the phrase was said.
    pub examples: Vec<Evidence>,
    /// It was said on the last night.
    pub said_in_last: bool,
    /// The effect's weight.
    pub weight: f64,
}

/// What changed between two consecutive nights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Change {
    /// The earlier night's label.
    pub from: String,
    /// The later night's label.
    pub to: String,
    /// The earlier date.
    pub from_date: String,
    /// The later date.
    pub to_date: String,
    /// Lines added.
    pub added: Vec<Evidence>,
    /// Lines dropped.
    pub dropped: Vec<Evidence>,
    /// Count added.
    pub n_added: u64,
    /// Count dropped.
    pub n_dropped: u64,
    /// Laugh-seconds later minus earlier.
    pub delta_laugh_s: f64,
    /// `vs_room` later minus earlier.
    pub delta_vs_room: Option<f64>,
}

/// Punch-up drafts for a bit (`creative/workingset.py` `punch_up`), cached or fresh; a queued answer is
/// [`QueuedJob`] (see [`Queued`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PunchUp {
    /// The joke id.
    pub joke_id: String,
    /// Its title.
    pub title: String,
    /// Always `true`: not his words.
    pub draft: bool,
    /// "DRAFT · AI · not your words yet".
    pub label: String,
    /// `ok`, `locked`, `no_tape`, `all_locked`, `provider_error`.
    pub status: String,
    /// The drafts (empty unless `ok`).
    pub drafts: Vec<PunchDraft>,
    /// Why there are none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// Lines the locks kept from the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_locked: Option<u64>,
    /// The night the lines came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on: Option<String>,
    /// The provider that answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The model that answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The model call's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    /// The router task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// The answer is a finished job's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
    /// That job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<JobBrief>,
}

/// One punch-up draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PunchDraft {
    /// His line.
    pub target_line: String,
    /// Where it was said.
    pub target_evidence: Evidence,
    /// The sharper alternative.
    pub alt: String,
    /// The move used.
    #[serde(rename = "move")]
    pub move_name: Option<String>,
    /// Always `true`.
    pub draft: bool,
    /// The draft label.
    pub label: String,
}

/// A room (`creative/venues.py` `venue_stats`, overlaid with his pins).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[allow(clippy::struct_field_names)]
pub struct Venue {
    /// `venue:<slug>`.
    pub venue_id: String,
    /// The name as the sets carry it.
    pub raw: String,
    /// The parsed name (his rename wins).
    pub name: String,
    /// Its sets.
    pub sets: Vec<VenueSet>,
    /// The dates.
    pub dates: Vec<String>,
    /// Sets here.
    pub n_sets: u64,
    /// Mean laugh-seconds per minute.
    pub avg_laugh_s_per_min: Option<f64>,
    /// Mean room heat.
    pub avg_room_heat: Option<f64>,
    /// The best bits here.
    pub best_jokes: Vec<VenueJoke>,
    /// The worst bits here.
    pub worst_jokes: Vec<VenueJoke>,
    /// Bits that do better here than elsewhere.
    pub works_here: Vec<WorksRow>,
    /// Bits that do better elsewhere.
    pub works_elsewhere: Vec<WorksRow>,
    /// A reading note.
    pub note: String,
    /// The geocoder's answer (open: the cache's own fields), or `null`.
    pub geo: Option<Map<String, Value>>,
    /// His pin move or rename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited: Option<Map<String, Value>>,
}

/// One set at a venue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct VenueSet {
    /// The set key.
    pub set: String,
    /// The date.
    pub date: String,
    /// The title.
    pub title: Option<String>,
    /// Laugh-seconds per minute.
    pub laugh_s_per_min: Option<f64>,
    /// Room heat.
    pub room_heat: Option<f64>,
    /// Share of time laughing.
    pub pct_laughing: Option<f64>,
    /// Material seconds.
    pub material_s: Option<f64>,
    /// The best bit, from the analysis.
    pub best: Option<Value>,
    /// The weakest.
    pub weakest: Option<Value>,
}

/// A bit's best or worst showing at a venue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct VenueJoke {
    /// The joke id.
    pub joke_id: String,
    /// Its title.
    pub title: String,
    /// The night.
    pub date: String,
    /// Against the room.
    pub vs_room: Option<f64>,
    /// Laugh-seconds per minute.
    pub laugh_s_per_min: f64,
    /// The AI's rating.
    pub ai_rating: Option<String>,
    /// Verbatim quotes.
    pub evidence: Vec<Evidence>,
}

/// A bit here vs elsewhere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WorksRow {
    /// The joke id.
    pub joke_id: String,
    /// Its title.
    pub title: String,
    /// Mean `vs_room` here.
    pub here_vs_room: f64,
    /// Mean `vs_room` elsewhere.
    pub elsewhere_vs_room: f64,
    /// Here minus elsewhere.
    pub delta: f64,
    /// Nights here.
    pub n_here: u64,
    /// Nights elsewhere.
    pub n_elsewhere: u64,
    /// A hunch, not a finding.
    pub low_n: bool,
}

/// `GET /api/venues`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct VenuesView {
    /// When it was built.
    pub generated_at: String,
    /// The rooms.
    pub venues: Vec<Venue>,
    /// Rooms the geocoder could not place.
    pub ambiguous: Vec<Ambiguous>,
    /// The geocoder.
    pub geocoder: Geocoder,
}

/// A room the geocoder could not place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Ambiguous {
    /// `venue:<slug>`.
    pub venue_id: String,
    /// The name as the sets carry it.
    pub raw: String,
    /// Why.
    pub reasons: Vec<String>,
    /// The candidate picked, if any.
    pub picked: Option<Value>,
}

/// The geocoder used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Geocoder {
    /// The service.
    pub service: String,
    /// The user agent sent.
    pub user_agent: String,
    /// Requests made this run.
    pub requests_this_run: u64,
    /// The cache file.
    pub cache: String,
    /// The attribution to show.
    pub attribution: String,
}

/// One room on the map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct GeoFeature {
    /// Always `"Feature"`.
    #[serde(rename = "type")]
    pub feature_type: String,
    /// The point, or `null` while not placed.
    pub geometry: Option<Geometry>,
    /// The room.
    pub properties: GeoProps,
}

/// A `GeoJSON` point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Geometry {
    /// Always `"Point"`.
    #[serde(rename = "type")]
    pub geometry_type: String,
    /// `[lon, lat]`.
    pub coordinates: [f64; 2],
}

/// A map pin's properties.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct GeoProps {
    /// `venue:<slug>`.
    pub venue_id: String,
    /// The name.
    pub name: String,
    /// The name as the sets carry it.
    pub raw: String,
    /// `inferred` or `his`.
    pub status: String,
    /// Not placed with confidence.
    pub ambiguous: bool,
    /// Sets here.
    pub n_sets: u64,
    /// The dates.
    pub dates: Vec<String>,
    /// Mean laugh-seconds per minute.
    pub avg_laugh_s_per_min: Option<f64>,
    /// Mean room heat.
    pub avg_room_heat: Option<f64>,
    /// The pin's colour.
    pub color: String,
    /// The short label.
    pub label: String,
    /// When he moved or renamed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<String>,
}

/// `GET /api/venues.geojson`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct GeoJSON {
    /// Always `"FeatureCollection"`.
    #[serde(rename = "type")]
    pub collection_type: String,
    /// One per room.
    pub features: Vec<GeoFeature>,
}

/// `GET /api/today`: `today.build`'s keys (`date`, `latest_set`, `working_set`, `try_tonight`, `serendipity`,
/// `new_material`, `minutes`, `his_tonight_list`) merged into the older feed (`unreviewed_sets`,
/// `latest_analysis`, `glance`, `tonight`, `pipeline`, `next_show`, `inbox`), which the wire still carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TodayFeed {
    /// The newest set: the card plus today's fields, or `null`.
    pub latest_set: Option<TodayLatestSet>,
    /// Sets not fully reviewed.
    pub unreviewed_sets: u64,
    /// The newest analysed set, briefly.
    pub latest_analysis: Option<LatestAnalysis>,
    /// Stage counts and the newest set's new material.
    pub glance: Glance,
    /// Tonight's list.
    pub tonight: Tonight,
    /// Jokes per stage.
    pub pipeline: BTreeMap<String, u64>,
    /// Always `null` (no bookings yet).
    pub next_show: Option<Value>,
    /// Files in the inbox.
    pub inbox: u64,
    /// Today's date.
    pub date: String,
    /// The most recently changed working set's status, with `perfected`.
    pub working_set: Option<WSStatus>,
    /// Up to three try-tonight notes.
    pub try_tonight: Vec<TryNote>,
    /// Gems and orphans (`{kind, joke_id, title, stage, why, score, source, evidence}`).
    pub serendipity: Vec<Value>,
    /// New material on the latest set (`{idx, title, joke_id, ai_rating, laugh_s_per_min, vs_room, similar, evidence}`).
    pub new_material: Vec<Value>,
    /// Working and testing minutes.
    pub minutes: Option<Minutes>,
    /// His tonight list, when he saved one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub his_tonight_list: Option<Vec<TonightListItem>>,
    /// The creative half failed: the older feed stands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creative_error: Option<String>,
}

/// The newest set on the today feed: the set card plus `today.build`'s set fields and the review count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TodayLatestSet {
    /// The card (its `title`, `date`, `venue`, `laugh_s_per_min`, `room_heat` win).
    #[serde(flatten)]
    pub card: SetCard,
    /// The set key.
    pub set: String,
    /// "<room> '24".
    pub venue_label: String,
    /// Material as m:ss.
    pub material: String,
    /// Share of time laughing.
    pub pct_laughing: Option<f64>,
    /// Known plus new rows.
    pub jokes_counted: u64,
    /// New rows.
    pub new_material: u64,
    /// The best bit (`{joke_id, title}`), or `null`.
    pub best: Option<Value>,
    /// The weakest.
    pub weakest: Option<Value>,
    /// Review steps.
    pub moments: u64,
    /// Minutes to review them.
    pub review_minutes: u64,
}

/// The newest analysed set, briefly (the older feed's `latest_analysis`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LatestAnalysis {
    /// The set id.
    pub id: String,
    /// Its title.
    pub title: Option<String>,
    /// Its date.
    pub date: Option<String>,
    /// `real` or `fixture`.
    pub source: String,
    /// Known plus new rows.
    pub jokes_counted: u64,
    /// Known rows.
    pub known_jokes: u64,
    /// New rows.
    pub new_material: u64,
    /// Crowd-work rows.
    pub crowd_work: u64,
    /// Laughs per minute.
    pub laughs_per_min: f64,
    /// The best known row.
    pub best: Option<BestPick>,
    /// The weakest.
    pub weakest: Option<BestPick>,
}

/// Stage counts and the newest set's new material (the older feed's `glance`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Glance {
    /// Jokes per stage.
    pub stages: BTreeMap<String, u64>,
    /// Working jokes whose stage is the AI's.
    pub ai_working: u64,
    /// The newest analysed set.
    pub set: Option<GlanceSet>,
    /// New rows (`{set, idx, perf, start_ms, end_ms, title, punch, rating, saved_as}`).
    pub new_material: Vec<Value>,
    /// Up to three hints (`{kind, idx, joke, target, why}`).
    pub hints: Vec<Value>,
}

/// A set, by id and title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct GlanceSet {
    /// The set id.
    pub id: String,
    /// Its title.
    pub title: Option<String>,
}

/// Tonight's list (the older feed's `tonight`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Tonight {
    /// The jokes, with their rooms and `done`.
    pub items: Vec<TonightItem>,
    /// `default` or `saved`.
    pub source: String,
    /// Jokes he could add.
    pub candidates: Vec<Candidate>,
}

/// A joke on tonight's list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TonightItem {
    /// The joke, briefly.
    #[serde(flatten)]
    pub joke: BriefJoke,
    /// Ticked.
    pub done: bool,
}

/// A joke he could add to tonight's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Candidate {
    /// The joke id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Its stage.
    pub stage: Option<String>,
}

/// His tonight list on the today feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TonightListItem {
    /// The joke id.
    pub joke_id: String,
    /// Its title.
    pub title: String,
    /// Ticked.
    pub done: bool,
}

/// Working and testing minutes (`creative/today.py` `minutes`). Beyond the TypeScript the wire carries `toward`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Minutes {
    /// Minutes of working material.
    pub working_min: f64,
    /// Minutes of testing material.
    pub testing_min: f64,
    /// Both.
    pub working_plus_testing_min: f64,
    /// Share measured on tape (percent).
    pub measured_share: f64,
    /// Progress toward 5, 15, 30, 60 minutes.
    pub toward: Vec<Toward>,
    /// Working jokes.
    pub n_working: u64,
    /// Testing jokes.
    pub n_testing: u64,
    /// How it is computed.
    pub basis: String,
}

/// Progress toward one goal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Toward {
    /// The goal in minutes.
    pub goal_min: u64,
    /// Working material, percent of the goal.
    pub working_pct: f64,
    /// With testing material.
    pub with_testing_pct: f64,
}

/// A clip (`infer/clips.py`, the set's `clips.json` passed through): the fields it writes, and anything else in `extra`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Clip {
    /// 1 = best.
    pub rank: u64,
    /// Start.
    pub start_ms: i64,
    /// End.
    pub end_ms: i64,
    /// Length in seconds.
    pub duration_s: f64,
    /// The joke, if known.
    pub joke_id: Option<String>,
    /// Its title, or the row's.
    pub title: Option<String>,
    /// The row's status.
    pub status: Option<String>,
    /// The AI's rating.
    pub ai_rating: Option<String>,
    /// The clip score.
    pub score: f64,
    /// Per signal: `{raw, norm, weight, contribution}`.
    pub score_breakdown: Map<String, Value>,
    /// Why, in plain words.
    pub why: String,
    /// The punchline.
    pub punchline: Option<String>,
    /// Where the punch is.
    pub punch_at_ms: Option<i64>,
    /// The laugh after it.
    pub laugh_id: Option<String>,
    /// The words.
    pub transcript: Option<String>,
    /// Anything else the clipper wrote (`llm`, ...).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `GET /api/sets/<id>/clips`: the set's `clips.json` as it is (open: `run_id`, `generated_at`, `params`, `llm`,
/// ... ride in `extra`), or `{set, clips: [], source: null}` when there is none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ClipsView {
    /// The set folder (a real file) or the set id (none).
    pub set: String,
    /// The clips, best first.
    pub clips: Vec<Clip>,
    /// `null` when there is no clips file; absent when there is.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::nullable::deserialize"
    )]
    pub source: Nullable<String>,
    /// The file's other fields.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The AI worker as the app shows it (`creative_api.py` `worker_status`): the last claim, echoed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WorkerStatus {
    /// A claim within the window.
    pub online: bool,
    /// The last claim's time.
    pub last_seen: Option<String>,
    /// The worker's name.
    pub name: Option<String>,
    /// `the server`, `a development PC`, `this computer`.
    #[serde(rename = "where")]
    pub where_: String,
    /// The last claim said hosted.
    pub hosted: bool,
    /// The models it writes with.
    pub model: Option<String>,
    /// The budget reason while online and paused.
    pub paused: Option<String>,
    /// The window in seconds (120).
    pub online_window_s: u64,
}

/// `GET /api/connections`: one plain status per outside source (Settings > Connections).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ConnectionsStatus {
    /// gsync's state, counts only.
    pub google_docs: Map<String, Value>,
    /// `{sets, import: "link"}`.
    pub youtube: Map<String, Value>,
    /// The provider and the worker.
    pub ai: AiStatus,
    /// `{where: "your phone", waiting, note}`.
    pub transcriber: Map<String, Value>,
    /// `{status: "not_set_up", provider}`.
    pub audio_storage: Map<String, Value>,
}

/// The AI line on Settings > Connections.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AiStatus {
    /// `hosted`, `chatgpt`, `key`, `custom`.
    pub provider: String,
    /// The worker.
    pub worker: WorkerStatus,
}
