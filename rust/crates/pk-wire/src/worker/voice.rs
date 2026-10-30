//! His voice profile as the worker fetches it per job (`api-dev/profile_api.py`, `creative/voice.py` `fetch`):
//! the profile text, the learned block and the laugh lines. The text is never logged.

use serde::{Deserialize, Serialize};

/// `GET /api/worker/profile/voice`: the profile, the learned block and the laugh lines (no history).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct VoiceProfile {
    /// Always `"voice"`.
    pub kind: String,
    /// The profile text; `null` until written.
    pub text: Option<String>,
    /// When it was last written.
    pub updated_at: Option<String>,
    /// The owner id, or `import:profile` for the loader.
    pub by: Option<String>,
    /// `owner`, `import` or `ai` from `by`; `null` when `by` is (T16).
    pub by_kind: Option<String>,
    /// `text`'s length (0 when `null`).
    pub chars: u64,
    /// What he kept and dismissed of the app's drafts.
    pub learned: Learned,
    /// His punchlines with the longest detector laughs (cap 15); `[]` without a laugh map.
    pub laugh_lines: Vec<LaughLine>,
}

/// The app's `GET /api/profile/voice`, and what the worker's `PUT` answers (the Python answers a PUT with the
/// app's shape: `previous` instead of `laugh_lines`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct VoiceProfileApp {
    /// Always `"voice"`.
    pub kind: String,
    /// The profile text; `null` until written.
    pub text: Option<String>,
    /// When it was last written.
    pub updated_at: Option<String>,
    /// Who wrote it.
    pub by: Option<String>,
    /// `owner`, `import` or `ai` from `by`; `null` when `by` is (T16).
    pub by_kind: Option<String>,
    /// `text`'s length.
    pub chars: u64,
    /// What he kept and dismissed of the app's drafts.
    pub learned: Learned,
    /// The version before this one (Undo in the app = PUT it again); `null` on the first write.
    pub previous: Option<VoicePrevious>,
}

/// The previous version of the profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct VoicePrevious {
    /// Its text.
    pub text: Option<String>,
    /// When it was written.
    pub updated_at: Option<String>,
    /// Who wrote it.
    pub by: Option<String>,
    /// `owner`, `import` or `ai` from `by`; `null` when `by` is (T16).
    pub by_kind: Option<String>,
}

/// The learned block (`creative/learned.py`): the lines he kept and dismissed, newest first, 25 each.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Learned {
    /// Drafts he took as his own.
    pub kept: Vec<LearnedLine>,
    /// Drafts he dismissed or passed over.
    pub dropped: Vec<LearnedLine>,
    /// When the block last changed; `null` while empty.
    pub updated_at: Option<String>,
}

/// One learned line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LearnedLine {
    /// The words.
    pub text: String,
    /// The bit it was drafted for.
    pub bit: Option<String>,
    /// The day.
    pub date: Option<String>,
    /// He edited the draft before keeping it (no draft matched the kept text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited: Option<bool>,
}

/// The learned block's counts (`POST /api/profile/voice/learn`'s answer, `learned` on an idea post).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LearnedCounts {
    /// Lines kept.
    pub kept: u64,
    /// Lines dropped.
    pub dropped: u64,
    /// When the block last changed.
    pub updated_at: Option<String>,
}

/// A punchline with the laugh the detector tied to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LaughLine {
    /// The line.
    pub text: String,
    /// The bit.
    pub bit: String,
    /// The night.
    pub date: Option<String>,
    /// The laugh's length in seconds (from the laugh map's timing, else the bars).
    pub laugh_s: f64,
}

/// `PUT /api/worker/profile/voice`: the one-off loader's write (`tools/profile/put_voice.py`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct VoicePut {
    /// The profile text (up to 60,000 characters, legal-checked).
    pub text: String,
    /// `import:profile`.
    pub actor: String,
}
