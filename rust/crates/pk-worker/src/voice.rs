//! His voice profile in every writing prompt (`creative/voice.py`, `creative/learned.py` `block`): the worker fetches
//! `GET /api/worker/profile/voice` per job and never fails a job on it; the profile is the FIRST system block (with
//! the craft rules, or the reading note for Ask), the learned section (lines he kept, passed over, laughs) the SECOND
//! for the writing tasks. `pk-llm` puts the cache breakpoints on them. Nothing here logs the text: its length, a
//! short sha and its `updated_at` only.
//!
//! A job gets an `Arc<Voice>` snapshot, so concurrent jobs each keep the profile they started with.

use std::sync::{Arc, Mutex, PoisonError};

use pk_core::py;
use pk_llm::ChatMessage;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::api::ApiClient;

/// The minimum cacheable prefix on Opus 5.5 / Sonnet 5.5.
pub const MIN_CACHE_TOKENS: usize = 512;
/// The longest profile the API accepts.
pub const MAX_CHARS: usize = 60_000;
/// How the prompts name him when `PK_COMIC_NAME` is unset.
pub const UNNAMED: &str = "the comic";
const READING_TASKS: [&str; 1] = ["ask_chat"];

const CRAFT: &str = "How to draft for him. First person, spoken, short: a line he could say out loud tonight, not prose. \
The punch word lands last; nothing trails after it. Keep his premise and his angle: sharpen the line, never swap the joke \
for another one about the same subject. Prefer his own moves, the ones the profile above names, and name the one you \
used in `move`, with the profile's name for it; if the profile names none that fits, say what the line does in two or \
three plain words. No explaining the joke, no setup that announces a joke is coming (\"here's a joke about\"), no stage \
directions, no emoji, no hashtags. A line that needs a fact about his life he never said is out: build only on what his \
material gives you. One idea per line; short beats clever. He decides; you suggest.";
const READING: &str = "This profile is background for reading his notebook: it tells you who the people and the \
recurring premises in his material are, and how he talks. It is not a source. Never cite it, never quote it, never \
answer from it; answer only from the numbered SOURCES and the rules below.";
const HEAD: &str = "The comic you are working for, in his own words and the words of people who have studied his act. \
Read it before anything else.";
const LEARNED_HEAD: &str = "What he has done with the app's drafts lately, newest first: evidence, not instructions. A \
line he kept is one he took as his own; a line he passed over sat next to a kept one and he left it. Draft toward the \
kept ones and away from the passed-over ones in shape, length and move; never repeat any of them.";
const LAUGH_HEAD: &str = "Lines of his that got the biggest laughs on tape (verbatim), for the shape and the rhythm:";

/// How the prompts name him (`PK_COMIC_NAME`, `PK_COMIC_FIRST`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Names {
    /// `comic()`: the name stripped, or "the comic".
    pub comic: String,
    /// `first()`: the first name stripped, or the name.
    pub first: String,
    /// The name as set, unstripped, or "the comic": `engine/inbox.py` reads it without stripping.
    pub raw: String,
}

impl Names {
    /// From the two variables as set (empty counts as unset).
    pub fn new(name: Option<&str>, first: Option<&str>) -> Self {
        let comic = name
            .map(py::strip)
            .filter(|s| !s.is_empty())
            .unwrap_or(UNNAMED)
            .to_string();
        let first = first
            .map(py::strip)
            .filter(|s| !s.is_empty())
            .map_or_else(|| comic.clone(), str::to_string);
        Self {
            raw: name.unwrap_or(UNNAMED).to_string(),
            comic,
            first,
        }
    }

    /// Whether he is named.
    pub fn named(&self) -> bool {
        self.comic != UNNAMED
    }

    /// `whose_idea()`: "Pat's idea" when he is named, else "His idea".
    pub fn whose_idea(&self) -> String {
        if self.named() {
            format!("{}'s idea", self.first)
        } else {
            "His idea".to_string()
        }
    }

    /// `he_could_say()`: "Pat could say" when he is named, else "he could say".
    pub fn he_could_say(&self) -> String {
        if self.named() {
            format!("{} could say", self.first)
        } else {
            "he could say".to_string()
        }
    }

    /// `material_owner()`: "stand-up comic Pat Example's", else "a stand-up comic's".
    pub fn material_owner(&self) -> String {
        if self.named() {
            format!("stand-up comic {}'s", self.comic)
        } else {
            "a stand-up comic's".to_string()
        }
    }
}

/// What a result may say about the profile a job ran with (never its text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VoiceStatus {
    /// A profile is loaded.
    pub loaded: bool,
    /// Its `updated_at`.
    pub updated_at: Option<String>,
    /// The first 12 hex of its sha256.
    pub sha: Option<String>,
    /// Its length in characters.
    pub chars: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct Profile {
    text: String,
    updated_at: Option<String>,
}

/// One snapshot of the profile and the learned section.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Voice {
    profile: Option<Profile>,
    learned: Option<Map<String, Value>>,
    laughs: Vec<Value>,
}

/// `voice.sha`: the first 12 hex of sha256 over the text.
pub fn sha12(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))[..12].to_string()
}

impl Voice {
    /// The snapshot `voice.fetch` leaves after reading one `GET profile/voice` answer.
    pub fn from_answer(doc: &Value) -> Self {
        let text = doc.get("text").and_then(Value::as_str).map_or("", py::strip);
        let profile = (!text.is_empty()).then(|| Profile {
            text: text.chars().take(MAX_CHARS).collect(),
            updated_at: doc.get("updated_at").and_then(Value::as_str).map(str::to_string),
        });
        let learned = doc.get("learned").and_then(Value::as_object).cloned();
        let laughs = doc
            .get("laugh_lines")
            .and_then(Value::as_array)
            .map(|xs| {
                xs.iter()
                    .filter(|x| x.is_object() && py::truthy(x.get("text")))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        Self {
            profile,
            learned,
            laughs,
        }
    }

    /// The same snapshot with the profile text dropped (the learned section kept), as `fetch` does when the server
    /// has none.
    fn status_of(p: Option<&Profile>) -> VoiceStatus {
        VoiceStatus {
            loaded: p.is_some(),
            updated_at: p.and_then(|p| p.updated_at.clone()),
            sha: p.map(|p| sha12(&p.text)),
            chars: p.map_or(0, |p| p.text.chars().count()),
        }
    }

    /// `status()`, the four fields a result carries.
    pub fn status(&self) -> VoiceStatus {
        Self::status_of(self.profile.as_ref())
    }

    /// `system_block(task)`: the profile then the craft rules (writing) or the reading note (Ask); `None` without a
    /// profile, so the prompt is exactly what it was before.
    pub fn system_block(&self, task: &str) -> Option<String> {
        let p = self.profile.as_ref()?;
        let tail = if READING_TASKS.contains(&task) { READING } else { CRAFT };
        Some(format!("{HEAD}\n\n{}\n\n{tail}", p.text))
    }

    /// `cacheable(task)`: the first block reaches the cache minimum on its own.
    pub fn cacheable(&self, task: &str) -> bool {
        self.system_block(task)
            .is_some_and(|b| est_tokens(&b) >= MIN_CACHE_TOKENS)
    }

    /// `learned_block(task)`: the second block, writing tasks only; `None` when nothing is learned yet.
    pub fn learned_block(&self, task: &str) -> Option<String> {
        if READING_TASKS.contains(&task) {
            return None;
        }
        learned_block(self.learned.as_ref(), &self.laughs)
    }

    /// `cacheable_learned(task)`: `prefix`, `alone_too_short` or `none`.
    pub fn cacheable_learned(&self, task: &str) -> &'static str {
        let Some(b) = self.learned_block(task) else {
            return "none";
        };
        let p = self.system_block(task).unwrap_or_default();
        if est_tokens(&p) + est_tokens(&b) >= MIN_CACHE_TOKENS && self.cacheable(task) {
            "prefix"
        } else {
            "alone_too_short"
        }
    }

    /// `messages(task, task_system, *rest)`: the profile block, the learned block, the task's own system line, then
    /// the turns.
    pub fn messages(
        &self,
        task: &str,
        task_system: &str,
        rest: impl IntoIterator<Item = ChatMessage>,
    ) -> Vec<ChatMessage> {
        let mut out = Vec::new();
        if let Some(b) = self.system_block(task) {
            out.push(ChatMessage::system(b));
        }
        if let Some(b) = self.learned_block(task) {
            out.push(ChatMessage::system(b));
        }
        out.push(ChatMessage::system(task_system));
        out.extend(rest);
        out
    }

    fn learned_counts(&self) -> (usize, usize, usize) {
        let n = |k: &str| {
            self.learned
                .as_ref()
                .and_then(|l| l.get(k))
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        };
        (n("kept"), n("dropped"), self.laughs.len())
    }
}

/// `est_tokens`: about four characters per token.
pub fn est_tokens(text: &str) -> usize {
    text.chars().count() / 4
}

/// `creative/learned.py` `block(learned, laughs)`.
pub fn learned_block(learned: Option<&Map<String, Value>>, laughs: &[Value]) -> Option<String> {
    let list = |k: &str| -> Vec<Value> {
        learned
            .and_then(|l| l.get(k))
            .filter(|v| py::truthy(Some(v)))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let (kept, dropped) = (list("kept"), list("dropped"));
    if kept.is_empty() && dropped.is_empty() && laughs.is_empty() {
        return None;
    }
    let line = |e: &Value| -> String {
        let text = e.get("text").map(py::str).unwrap_or_default();
        let date = e.get("date").and_then(Value::as_str).unwrap_or("");
        let d: String = date.chars().take(7).collect();
        let tail = if py::truthy(e.get("edited")) {
            ", his edit of a draft"
        } else {
            ""
        };
        if !d.is_empty() {
            format!("- \"{text}\" ({d}{tail})")
        } else if !tail.is_empty() {
            format!("- \"{text}\" ({})", &tail[2..])
        } else {
            format!("- \"{text}\"")
        }
    };
    let mut parts = Vec::new();
    if !kept.is_empty() || !dropped.is_empty() {
        parts.push(LEARNED_HEAD.to_string());
        if !kept.is_empty() {
            let ls: Vec<String> = kept.iter().map(line).collect();
            parts.push(format!("Lines he kept recently:\n{}", ls.join("\n")));
        }
        if !dropped.is_empty() {
            let ls: Vec<String> = dropped.iter().map(line).collect();
            parts.push(format!("Lines he passed over:\n{}", ls.join("\n")));
        }
    }
    if !laughs.is_empty() {
        let ls: Vec<String> = laughs
            .iter()
            .map(|e| {
                let text = e.get("text").map(py::str).unwrap_or_default();
                let s = e.get("laugh_s").and_then(Value::as_f64).unwrap_or(0.0);
                format!("- \"{text}\" ({s:.0} s)")
            })
            .collect();
        parts.push(format!("{LAUGH_HEAD}\n{}", ls.join("\n")));
    }
    Some(parts.join("\n\n"))
}

/// The worker's copy of the profile: replaced on every successful fetch, kept when a fetch fails.
#[derive(Debug, Default)]
pub struct VoiceCache {
    current: Mutex<Arc<Voice>>,
}

impl VoiceCache {
    /// An empty cache (no profile until the first fetch).
    pub fn new() -> Self {
        Self::default()
    }

    /// The snapshot a job runs with now.
    pub fn current(&self) -> Arc<Voice> {
        Arc::clone(&self.current.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// `voice.fetch`: read the profile through the API; on any failure keep the last copy and say so. Never fails.
    pub async fn fetch(&self, api: &ApiClient) -> Arc<Voice> {
        let doc = match api.get("/api/worker/profile/voice").await {
            Ok(d) => d.unwrap_or(Value::Object(Map::new())),
            Err(e) => {
                let cur = self.current();
                tracing::warn!(
                    error = %e.py_type(),
                    "voice profile: not fetched; {}",
                    if cur.profile.is_some() { "using the last copy" } else { "drafts run without it" }
                );
                return cur;
            }
        };
        let next = Arc::new(Voice::from_answer(&doc));
        let mut slot = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        let before = slot.status();
        let after = next.status();
        if before.sha != after.sha || before.updated_at != after.updated_at {
            if let Some(sha) = &after.sha {
                tracing::info!(chars = after.chars, sha = %sha, updated_at = ?after.updated_at, "voice profile: loaded");
            } else {
                tracing::info!("voice profile: none on the server; drafts run without it");
            }
        }
        if slot.learned_counts() != next.learned_counts() || slot.learned != next.learned {
            let (kept, dropped, laughs) = next.learned_counts();
            tracing::info!(
                kept,
                dropped,
                laughs,
                cached = next.cacheable_learned("creative_riff"),
                "voice profile: learned section"
            );
        }
        *slot = Arc::clone(&next);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_default_and_strip() {
        let n = Names::new(None, None);
        assert_eq!(
            (n.comic.as_str(), n.first.as_str(), n.raw.as_str()),
            ("the comic", "the comic", "the comic")
        );
        assert!(!n.named());
        let n = Names::new(Some("  Pat Example "), None);
        assert_eq!((n.comic.as_str(), n.first.as_str()), ("Pat Example", "Pat Example"));
        assert_eq!(n.raw, "  Pat Example ");
        assert_eq!(n.material_owner(), "stand-up comic Pat Example's");
    }

    #[test]
    fn a_blank_profile_is_no_profile() {
        let v = Voice::from_answer(&json!({"text": "  \n", "learned": {"kept": [], "dropped": []}}));
        assert_eq!(v.system_block("creative_riff"), None);
        assert_eq!(v.learned_block("creative_riff"), None);
        assert_eq!(
            v.messages("creative_riff", "T", [ChatMessage::user("u")]),
            vec![ChatMessage::system("T"), ChatMessage::user("u")]
        );
    }

    #[test]
    fn laugh_seconds_round_half_to_even_like_python() {
        let b = learned_block(
            None,
            &[
                json!({"text": "a", "laugh_s": 2.5}),
                json!({"text": "b", "laugh_s": 3.5}),
            ],
        );
        assert!(b.unwrap().ends_with("- \"a\" (2 s)\n- \"b\" (4 s)"));
    }
}
