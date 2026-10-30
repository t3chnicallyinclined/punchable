//! The shapes `app/src/lib/simple.ts` exports, as the Python writes them (`creative/notebook.py` through
//! `api-dev/creative_api.py`, `creative/write.py` through the worker): the bit card, the joke map, the set builder,
//! and the two Write drafts (facts, riff).

use serde::{Deserialize, Serialize};

use crate::nullable::Nullable;
use crate::worker::JobBrief;
use crate::worker::kb::{IdeaResponse, JokeText};
use crate::worker::voice::LearnedCounts;

/// A bit as the app shows it anywhere (`creative/notebook.py` `card`): short title, his line verbatim, its date,
/// one stage line in words. The map's "older versions" items carry `unit_id` and no `nights`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BitCard {
    /// The joke id.
    pub joke_id: String,
    /// The title.
    pub title: String,
    /// What he writes on a paper set list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    /// The stage; absent on a version item.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::nullable::deserialize"
    )]
    pub stage: Nullable<String>,
    /// His line, verbatim.
    pub quote: Option<String>,
    /// Where the line is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_node: Option<String>,
    /// The quote was cut at the start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_start: Option<bool>,
    /// The quote was cut at the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_end: Option<bool>,
    /// The line's date.
    pub date: Option<String>,
    /// One stage line in words.
    pub stage_line: Option<String>,
    /// It has done well on stage.
    pub proven: bool,
    /// Nights on tape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nights: Option<u64>,
    /// How long it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
    /// The duration is an estimate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated: Option<bool>,
    /// Why it is here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// He made the link himself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mine: Option<bool>,
    /// Done back to back on tape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proven_pair: Option<bool>,
    /// The unit (an older version).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_id: Option<String>,
    /// Its rank in a search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<u64>,
    /// The whole text (the map's own bit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// `GET /api/notebook/related?q=`: his own bits about an idea.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Related {
    /// The idea asked about.
    pub query: String,
    /// The bits, best first.
    pub items: Vec<BitCard>,
    /// Matches in all.
    pub total: u64,
}

/// The joke map's groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum MapGroupKey {
    /// Could come next.
    Next,
    /// Could tag it.
    Tags,
    /// Same world.
    SameWorld,
    /// Callbacks.
    Callbacks,
    /// Older versions.
    Versions,
}

/// One group of the joke map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct MapGroup {
    /// The group.
    pub key: MapGroupKey,
    /// Its heading.
    pub title: String,
    /// The bits.
    pub items: Vec<BitCard>,
}

/// `GET /api/jokes/<id>/map`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct JokeMap {
    /// The bit, with its whole `text`.
    pub joke: BitCard,
    /// The five groups, in order.
    pub groups: Vec<MapGroup>,
}

/// A transition into a step of the set builder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TransitionIn {
    /// In plain words.
    pub words: String,
    /// A jump.
    pub weak: bool,
    /// `proven`, `link` or `jump`.
    pub kind: String,
    /// The bit before.
    pub from: String,
    /// This bit.
    pub to: String,
}

/// A step of the set builder: the bit card plus its position and the transition into it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Step {
    /// The bit.
    #[serde(flatten)]
    pub card: BitCard,
    /// Its place, from 0.
    pub position: i64,
    /// The transition into it; `null` on the first.
    pub transition_in: Option<TransitionIn>,
    /// The bit's newest written version, split (the joke page's `text`; `from` is `null` without a unit).
    pub text: JokeText,
    /// His typed idea's words, when the bit is one; `null` otherwise.
    pub idea: Option<String>,
}

/// `GET /api/boards/<id>/build` (`creative/notebook.py` `build`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Build {
    /// The board.
    pub board: BuildBoard,
    /// Runtime in seconds.
    pub runtime_s: f64,
    /// Runtime as m:ss.
    pub runtime: String,
    /// Bits whose length is estimated.
    pub estimated_n: u64,
    /// `short`, `on`, `over` or `none`.
    pub status: String,
    /// The status in words.
    pub status_words: String,
    /// Jumps.
    pub weak_n: u64,
    /// The bits, in order.
    pub steps: Vec<Step>,
    /// What could come next.
    pub next: Vec<BitCard>,
    /// The rules that fired.
    pub notes: Vec<BuildNote>,
    /// The working set promoted from the board.
    pub working_set: Option<String>,
}

/// The board of a build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BuildBoard {
    /// The board id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// The target in seconds.
    pub target_s: i64,
    /// The low end of "on target".
    pub target_lo_s: i64,
}

/// A rule that fired on a build: the closer rule carries `suggest` (a joke id or `null`), the dip rule has no key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BuildNote {
    /// `closer` or `dip`.
    pub rule: String,
    /// In plain words.
    pub text: String,
    /// A bit to close on.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::nullable::deserialize"
    )]
    pub suggest: Nullable<String>,
}

/// A checked fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Fact {
    /// One sentence.
    pub text: String,
    /// The page's title.
    pub source_title: String,
    /// The page.
    pub source_url: String,
}

/// Facts for an idea (`creative/write.py` `check_facts`; job kind `facts`), cached or fresh; a queued answer is
/// `creative::QueuedJob`. Beyond the TypeScript the wire always carries `draft`, `provider`, `model`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Facts {
    /// The idea.
    pub idea: String,
    /// Always `true`.
    pub draft: bool,
    /// The label to show.
    pub label: String,
    /// `ok`, `none`, `empty`, `provider_error`.
    pub status: String,
    /// The facts that checked out.
    pub facts: Vec<Fact>,
    /// Facts dropped (no source, or unsupported).
    pub dropped: u64,
    /// Why there are none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The provider that answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The model that answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The answer is a finished job's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
    /// That job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<JobBrief>,
}

/// One riff line. Beyond the TypeScript the wire carries `move`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RiffIdea {
    /// The line.
    pub text: String,
    /// What it builds on: `riff` and bit ids.
    pub built_on: Vec<String>,
    /// The move, by the profile's name.
    #[serde(rename = "move")]
    pub move_name: Option<String>,
    /// Always `true`.
    pub draft: bool,
    /// The label to show.
    pub label: String,
}

/// A bit a riff is grounded in (`creative/write.py` `riff_grounds`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RiffGround {
    /// `B1`, `B2`, ...
    pub id: String,
    /// The joke id.
    pub joke_id: String,
    /// Its title.
    pub title: String,
    /// Its best matching line.
    pub quote: String,
    /// The lead text, cut.
    pub text: String,
    /// How it did on stage.
    pub stage_line: Option<String>,
    /// The latest night, in words.
    pub last_night: Option<String>,
}

/// Riff lines for an idea (`creative/write.py` `riff_ideas`; job kind `riff`), cached or fresh; a queued answer
/// is `creative::QueuedJob`. Beyond the TypeScript the wire always carries `draft`, `label`, `grounded_in`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Riff {
    /// The idea.
    pub idea: String,
    /// Always `true`.
    pub draft: bool,
    /// "DRAFT · AI · not your words yet".
    pub label: String,
    /// `ok`, `none`, `no_grounding`, `provider_error`.
    pub status: String,
    /// The lines.
    pub ideas: Vec<RiffIdea>,
    /// The bits they build on.
    pub grounded_in: Vec<RiffGround>,
    /// Why there are none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The provider that answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The model that answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The answer is a finished job's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
    /// That job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<JobBrief>,
}

/// `POST /api/ideas`' answer (`simple.saveIdea`): the same as the worker's.
pub type SavedIdea = IdeaResponse;

/// `POST /api/profile/voice/learn`'s answer (`simple.dismissDraft`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct DismissResponse {
    /// Always `true`.
    pub ok: bool,
    /// The learned block's counts.
    pub learned: LearnedCounts,
}
