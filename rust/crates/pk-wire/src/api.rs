//! The shapes `app/src/lib/api.ts` exports, as the Python API writes them (`api-dev/server.py`): the explorer's
//! edge view and briefs, the laugh, the Desk's joke summary, the set card and the review step, the write receipt,
//! and the two client-side records (`ApiEvent`, `UploadMeta`) the app builds itself.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::nullable::Nullable;
use crate::worker::kb::Decision;

/// A verbatim quote that grounds an edge or a suggestion: `{node, quote}` plus whatever the pipeline added
/// (`start_ms` on a performance, `punchline` on a try note, `field`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Evidence {
    /// The node the quote is from.
    pub node: String,
    /// The words, verbatim.
    pub quote: String,
    /// `start_ms`, `field`, `punchline`, ...
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The explorer's brief of a node (`kb.py` `View.brief`): `id`, `type`, `title`, then the type's own fields
/// (`stage`, `summary` on a joke; `kind`, `definition` on a label; `kind`, `date`, `doc_title`, `joke`,
/// `joke_title` on a unit; `set`, `date`, `venue`, `start_ms`, `end_ms` on a performance; `date`, `venue` on a set).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Brief {
    /// The node id.
    pub id: String,
    /// `joke`, `unit`, `label`, `performance`, `set`, `list`, or `missing`.
    #[serde(rename = "type")]
    pub node_type: String,
    /// The title (a unit's or performance's first 140 characters of text).
    pub title: String,
    /// The type's own fields.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// An edge's AI state: `ai` = inferred and untouched (shown as fact), `suggested` = a proposal, the rest are the
/// comic's own; `dismissed` means the AI retired it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum EdgeState {
    /// Inferred by the AI and untouched.
    Ai,
    /// Proposed, waiting for him.
    Suggested,
    /// He confirmed it.
    Confirmed,
    /// He edited it.
    Edited,
    /// He added it.
    Yours,
    /// He rejected it.
    Rejected,
    /// The AI retired it (superseded).
    Dismissed,
}

/// The explorer's edge view (`kb.py` `View.edge_view`) plus its AI state (`server.py` `AppIndex.deco`): the
/// edge's own fields, then `bucket`, `display_kind`, `yours`, `edited`, the two briefs, and, when the view was
/// built around a focus node, `other` and `direction` (`via_unit` on the joke page when the edge is on a unit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct EdgeView {
    /// `e:` + 12 hex.
    pub id: String,
    /// Source node.
    pub src: String,
    /// Target node.
    pub dst: String,
    /// `part_of`, `performed_as`, `has_label`, `segue`, `callback_to`, `same_premise`, `custom`, ...
    pub kind: String,
    /// The raw status (`proposed`, `inferred`, `confirmed`, `rejected`, `superseded`, ...); absent on an edge with none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// The pipeline's confidence; absent on an edge with none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Why; absent on an edge with none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Verbatim quotes; absent on an edge with none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<Evidence>>,
    /// `proposed`, `confirmed` or `rejected`.
    pub bucket: String,
    /// The kind, or a custom edge's `custom_kind`.
    pub display_kind: String,
    /// He added it.
    pub yours: bool,
    /// He edited it.
    pub edited: bool,
    /// The source's brief.
    pub src_brief: Brief,
    /// The target's brief.
    pub dst_brief: Brief,
    /// The edges a merge folded into this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_from: Option<Vec<String>>,
    /// The other end's brief, when built around a focus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<Brief>,
    /// `out` or `in` from the focus, when built around one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// The joke's unit the edge sits on (joke page only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_unit: Option<Brief>,
    /// The AI state.
    pub state: EdgeState,
    /// `state == ai`.
    pub ai: bool,
    /// Any other edge field (`source`, `custom_kind`, ...).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A detected laugh (`corpus/sets/<set>/laughs_auto.json`, passed through): the fields the app reads, and the
/// detector's others (`segment`) in `extra`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Laugh {
    /// The laugh's id in the map.
    pub id: String,
    /// Start.
    pub start_ms: i64,
    /// End.
    pub end_ms: i64,
    /// The laugh meter, 0 to 4.
    pub bars: u64,
    /// The detector's strength.
    pub strength: f64,
    /// `laugh`, `applause`, ...
    pub kind: String,
    /// The detector's confidence.
    pub confidence: f64,
    /// `segment` and anything else the detector wrote.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A label on a joke summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LabelChip {
    /// The label id.
    pub id: String,
    /// Its name.
    pub name: Option<String>,
    /// The edge's bucket.
    pub bucket: String,
}

/// A joke as the Desk lists it (`server.py` `AppIndex.summaries` plus the creative duration). Beyond the
/// TypeScript's fields the wire always carries `touched`, `ai_hit_sets`, `yours`, `who`, `stage_ai`,
/// `stage_confidence`, `short_title`, `punch`, `short_name`, `nights`, `avg_vs_room`, `avg_laugh_s_per_min`,
/// `last_performed`; `duration_s` and `duration_estimated` when the creative layer is on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct JokeSummary {
    /// The joke id.
    pub id: String,
    /// The catalogue title.
    pub title: String,
    /// `idea`, `to_try`, `testing`, `working`, `retired`.
    pub stage: Option<String>,
    /// The summary (`""` when none).
    pub summary: String,
    /// First seen (date).
    pub first_seen: Option<String>,
    /// Last seen (date).
    pub last_seen: Option<String>,
    /// His last decision on it (timestamp).
    pub touched: Option<String>,
    /// Units filed under it.
    pub versions: u64,
    /// Distinct sets with a live match.
    pub sets: u64,
    /// Sets where he confirmed the match and it hit.
    pub hit_sets: u64,
    /// Sets where his rating, else the AI's, hit.
    pub ai_hit_sets: u64,
    /// Proposed edges waiting on him.
    pub to_review: u64,
    /// Up to six labels, confirmed first.
    pub labels: Vec<LabelChip>,
    /// He added it.
    pub yours: bool,
    /// The stage shown is the AI's.
    pub stage_ai: bool,
    /// The AI's confidence in the stage, while `stage_ai`.
    pub stage_confidence: Option<f64>,
    /// `ai`, `confirmed` or `edited`.
    pub who: String,
    /// The title cut at its first parenthesis, at most 28 characters.
    pub short_title: String,
    /// The last sentence of the newest version (an exact substring), or `null`.
    pub punch: Option<String>,
    /// What he writes on a paper set list.
    pub short_name: String,
    /// `= sets`.
    pub nights: u64,
    /// Mean `vs_room` over his analysed nights (clipped at 3.0).
    pub avg_vs_room: Option<f64>,
    /// Mean laugh-seconds per minute over his analysed nights.
    pub avg_laugh_s_per_min: Option<f64>,
    /// The latest show date on tape.
    pub last_performed: Option<String>,
    /// How long it runs (from his tapes, else estimated); only with the creative layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
    /// `duration_s` is an estimate from the written words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_estimated: Option<bool>,
}

/// `GET /api/jokes`: a page of summaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct JokesPage {
    /// The page.
    pub items: Vec<JokeSummary>,
    /// Matches in all.
    pub total: u64,
    /// The page's first index.
    pub offset: u64,
    /// The page size asked for.
    pub limit: u64,
}

/// A review step's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum StepKind {
    /// A performance matched to a joke.
    Joke,
    /// A rejected match, or a laugh no performance covers.
    NewLine,
    /// Crowd work or banter.
    CrowdWork,
}

/// One step of a set review (`server.py` `review_steps`), in time order. A `new_line` from an uncovered laugh
/// has no `perf`, `name`, `joke`, `match` and no `ai_rating` key at all; a performance step always carries
/// `ai_rating` (possibly `null`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Step {
    /// The performance id, or `line:<set>@<start_ms>`.
    pub key: String,
    /// What it is.
    pub kind: StepKind,
    /// The performance node's public fields (open); `null` for an uncovered laugh.
    pub perf: Option<Map<String, Value>>,
    /// The performance's name.
    pub name: Option<String>,
    /// The matched joke, briefly.
    pub joke: Option<BriefJoke>,
    /// The `performed_as` edge view (no focus: no `other` / `direction`).
    #[serde(rename = "match")]
    pub matched: Option<EdgeView>,
    /// The words.
    pub text: String,
    /// Where the punch is.
    pub punch: StepPunch,
    /// Laughs in the span.
    pub laughs: Vec<Laugh>,
    /// The strongest laugh's bars.
    pub max_bars: u64,
    /// Laughs counted.
    pub hits: u64,
    /// The laugh meter's suggestion.
    pub suggested: String,
    /// His rating.
    pub rating: Option<String>,
    /// The AI's rating; the key is absent on an uncovered laugh.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::nullable::deserialize"
    )]
    pub ai_rating: Nullable<String>,
    /// The play window.
    pub play: crate::worker::kb::Play,
}

/// A joke, briefly, with its rooms (`server.py` `brief_joke`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BriefJoke {
    /// The joke id.
    pub id: String,
    /// The title (the id when none).
    pub title: String,
    /// The stage.
    pub stage: Option<String>,
    /// The summary (`""` when none).
    pub summary: String,
    /// Distinct sets.
    pub sets: u64,
    /// Sets where he confirmed and it hit.
    pub hit_sets: u64,
    /// Sets where his rating, else the AI's, hit.
    pub ai_hit_sets: u64,
    /// The stage shown is the AI's.
    pub stage_ai: bool,
}

/// The punch of a review step: the segment(s) before the strongest laugh, that laugh, and every segment in the span.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct StepPunch {
    /// The chosen segments' text.
    pub text: String,
    /// Start of the chosen segments.
    pub start_ms: Option<i64>,
    /// End of the chosen segments.
    pub end_ms: Option<i64>,
    /// The strongest laugh, if any.
    pub laugh: Option<Laugh>,
    /// Every transcript segment in the span, the chosen ones marked.
    pub segments: Vec<PunchSegment>,
}

/// A transcript segment in a punch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PunchSegment {
    /// Start.
    pub start_ms: i64,
    /// End.
    pub end_ms: i64,
    /// The words.
    pub text: String,
    /// This segment is the punch.
    pub punch: bool,
}

/// `GET /api/sets/<id>/review`: the set card, its steps and the audio links when the tape is here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Review {
    /// The set.
    pub set: SetCard,
    /// The steps, in time order.
    pub steps: Vec<Step>,
    /// `{opus, wav}` routes, or `null`.
    pub audio: Option<AudioLinks>,
}

/// The two audio routes of a set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AudioLinks {
    /// `/api/audio/<set>`.
    pub opus: String,
    /// `/api/audio/<set>?fmt=wav`.
    pub wav: String,
}

/// A set as the app lists it (`server.py` `set_card` + `AppIndex.set_info`). Beyond the TypeScript the wire
/// always carries `rateable`, `rated`, `reviewed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SetCard {
    /// `set:<date>_<id>`.
    pub id: String,
    /// The title.
    pub title: Option<String>,
    /// The graph's date.
    pub date: Option<String>,
    /// The room, as he last set it.
    pub venue: Option<String>,
    /// His show date, else the meta's, else `null`.
    pub show_date: Option<String>,
    /// `yours`, or the meta's source text, or `null`.
    pub show_date_source: Option<String>,
    /// The `YouTube` description when the tape's info is here.
    pub description: Option<String>,
    /// The analysis summary's.
    pub laugh_s_per_min: Option<f64>,
    /// The analysis summary's.
    pub room_heat: Option<f64>,
    /// His mark only (`true` / `false`), else `null`.
    pub audio_unreliable: Option<bool>,
    /// The room id `/api/venues` uses.
    pub venue_id: String,
    /// `youtube`, `voice_memo`, `app_upload` or `tape`.
    pub source: String,
    /// Where the recording lives; `null` for a tape only on his PC.
    pub reference: Option<Reference>,
    /// The set's length.
    pub duration_s: Option<f64>,
    /// Performance nodes.
    pub performances: u64,
    /// Performances that are a matched bit.
    pub rateable: u64,
    /// Of which rated by him.
    pub rated: u64,
    /// Every rateable performance is rated.
    pub reviewed: bool,
    /// Laughs in the map.
    pub laughs: u64,
    /// The tape is here.
    pub has_audio: bool,
    /// The AI's reading, briefly; `null` when it has not read the set.
    pub analysis: Option<AnalysisBrief>,
    /// One row per performance.
    pub jokes: Vec<SetJoke>,
}

/// `GET /api/sets`: every set, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Sets {
    /// The cards.
    pub sets: Vec<SetCard>,
}

/// Where a set's recording lives (`server.py` `set_reference`), by `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Reference {
    /// A `YouTube` video: play = the url with `&t=<s>`. No golden (the commit guard refuses any watch URL).
    Youtube {
        /// The 11-character id.
        video_id: String,
        /// The watch url.
        url: String,
        /// The video's title.
        title: Option<String>,
    },
    /// His Drive file (a voice memo).
    Drive {
        /// The file's view url.
        url: String,
        /// The file's name.
        name: Option<String>,
    },
    /// The file on his phone (an upload).
    File {
        /// The file's name.
        name: Option<String>,
        /// When it was recorded.
        recorded_at: Option<String>,
    },
}

/// The AI's reading of a set, briefly (`server.py` `analysis_brief`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AnalysisBrief {
    /// `real` or `fixture`.
    pub source: String,
    /// The analysis run.
    pub run_id: Option<String>,
    /// When it ran.
    pub generated_at: Option<String>,
    /// Known plus new rows.
    pub jokes_counted: u64,
    /// New rows.
    pub new_material: u64,
    /// Crowd-work rows.
    pub crowd_work: u64,
    /// The best known row.
    pub best: Option<BestPick>,
    /// Laughs per minute.
    pub laughs_per_min: f64,
}

/// A set's best or weakest bit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BestPick {
    /// The joke id.
    pub id: String,
    /// Its title.
    pub title: Option<String>,
    /// The row's index.
    pub idx: Option<i64>,
    /// The effective rating.
    pub rating: Option<String>,
}

/// One performance row on a set card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SetJoke {
    /// The performance id.
    pub perf: String,
    /// The joke's title, the performance's name, or "New material".
    pub title: Option<String>,
    /// The matched joke, if the match is live.
    pub joke: Option<String>,
    /// `bit`, `crowd_work` or `banter`.
    pub kind: Option<String>,
    /// His rating, else the AI's.
    pub rating: Option<String>,
    /// `you` or `ai`.
    pub rating_by: Option<String>,
    /// Start.
    pub start_ms: Option<i64>,
}

/// Every write's receipt: the line numbers appended and the lines (`POST /api/decision`, `/api/undo`,
/// `/api/review/rate`, `/api/set-row`, and the worker's `decision` / `undo`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Written {
    /// Always `true`.
    pub ok: bool,
    /// The appended lines' indexes (pass them to undo).
    pub indexes: Vec<u64>,
    /// The appended lines.
    pub decisions: Vec<Decision>,
}

/// A failed request as the app's crash reporter records it: method, path pattern and status, never a body.
/// Client-side only; no route sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ApiEvent {
    /// The HTTP method.
    pub method: String,
    /// The path without its query.
    pub path: String,
    /// The HTTP status, 0 for a network failure.
    pub status: u16,
    /// The error's name on a network failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// What the app sends with an upload, as the `X-*` headers of `POST /api/upload` (camelCase, the app's own
/// record). Client-side only; the API stores it as `worker::UploadRecord`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct UploadMeta {
    /// `set`, `note` or `upload`.
    pub kind: String,
    /// The file name.
    pub filename: String,
    /// `X-Title`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `X-Duration-S` (rounded by the app).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
    /// `X-Marks-Ms`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marks_ms: Option<Vec<i64>>,
    /// `X-Recorded-At`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<String>,
    /// `X-Venue`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<String>,
    /// `X-Lat`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
    /// `X-Lng`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lng: Option<f64>,
    /// `X-Show-Group`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_group: Option<String>,
    /// `X-Align-Working-Set`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align_working_set: Option<String>,
}
