//! `rant` and `hook` (`creative/rant.py`, Film it's two sessions on one version of a bit): "Make it a rant" asks for
//! whole-line hunks (change / cut / add) that turn the version to face a phone camera; "Pick a hook" asks for three
//! one-line openers (`exclude`: the ones he already saw). The input is the version as the API holds it, split into
//! the app's lines, so neither kind reads the notebook. The punch-up's prompt pattern: the voice profile first, the
//! learned block second, the task line (with the public method) third, the numbered lines last.
//!
//! Every hunk that quotes his line is checked word for word against the version (whitespace aside) and dropped when
//! it is not that line; with a punch line given, a change that loses its reveal word or a cut of it is dropped. A
//! hook equal to a line of the version, or to one he already saw, is dropped. Nothing here writes.

use std::collections::HashSet;
use std::sync::LazyLock;

use pk_core::py;
use pk_creative::write::DRAFT_LABEL;
use pk_llm::ChatMessage;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Ctx, Done, Runner};
use crate::error::{JobError, provider_why};

/// The tasks in `engine/config.hosted.toml`.
pub const RANT_TASK: &str = "creative_rant";
/// The hook task.
pub const HOOK_TASK: &str = "creative_hook";
/// A hunk's new line, at most (code points).
pub const MAX_LINE_CHARS: usize = 400;
/// An opener, at most (code points).
pub const MAX_HOOK_CHARS: usize = 300;
/// An opener, at most (words).
pub const MAX_HOOK_WORDS: usize = 24;
/// Openers shown per round.
pub const HOOKS: usize = 3;

/// `RANT_SYSTEM`.
pub const RANT_SYSTEM: &str = "You rework one comedian's own bit so he can say it straight into a phone's front camera. You \
suggest changes line by line; he keeps or skips each one. Never invent biography.";
/// `RANT_METHOD`: public, invented examples only.
pub const RANT_METHOD: &str = "How a rant cut works. He is filming himself, looking into the lens, talking to one person. Rework the bit for \
that, one whole line at a time:\n\
- Second person and present tense: \"You scan forty things.\" not \"I scanned forty things.\" For this cut that \
overrides the first-person rule above; keep his words everywhere else.\n\
- Direct address: he talks to the viewer, never about \"the audience\" or \"you guys\".\n\
- Escalation: each line pushes a little further than the one before it; nothing goes flat in the middle.\n\
- The punch stays the last beat of the run: keep its reveal word, and keep it last before the tags.\n\
- Tags come after the punch, short, each a fresh hit on the same idea.\n\
- Leave a line alone when it already works for the camera: no hunk is better than a change for its own sake.\n\
- Spoken and short, one breath per line. No stage directions, no emoji, no hashtags.\n\
Example (invented): \"Self-checkout is the store hiring you for free.\" can become \"You know what self-checkout \
is? It's the store hiring you. For free.\"; \"I scanned forty things and nobody clapped.\" can become \"You \
scan forty things. Nobody claps.\"";
/// `HOOK_SYSTEM`.
pub const HOOK_SYSTEM: &str = "You write openers for one comedian's bit, the first line he says into a phone's front camera. You \
suggest; he picks one or none. Never invent biography.";
/// `HOOK_METHOD`: public, invented examples only.
pub const HOOK_METHOD: &str = "A hook is one spoken line that makes a stranger stay for the second line. It earns the bit's first line: it \
points at the premise without giving away the punch. Short (at most 24 words), in his voice, no question bait \
(\"Have you ever\u{2026}\"), no \"wait for it\", no emoji, no hashtags. Each of the three takes a different angle \
(a confession, a claim, a complaint, a scene). Invented examples for a bit about self-checkout: \"Stop scanning \
your own groceries.\" \u{b7} \"I want to talk to whoever invented self-checkout.\" \u{b7} \"I got fired today. From a \
store I don't work at.\"";

/// `RANT_SCHEMA`.
pub static RANT_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({"type": "object", "properties": {"hunks": {"type": "array", "items": {
        "type": "object", "properties": {"op": {"type": "string", "enum": ["change", "cut", "add"]},
                                         "line_no": {"type": "integer"}, "was": {"type": "string"},
                                         "now": {"type": "string"}, "why": {"type": "string"}},
        "required": ["op", "line_no", "was", "now", "why"], "additionalProperties": false}}},
        "required": ["hunks"], "additionalProperties": false})
});
/// `HOOK_SCHEMA`.
pub static HOOK_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({"type": "object", "properties": {"hooks": {"type": "array", "items": {
        "type": "object", "properties": {"text": {"type": "string"}, "why": {"type": "string"}},
        "required": ["text", "why"], "additionalProperties": false}}},
        "required": ["hooks"], "additionalProperties": false})
});

static WORD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\w+").unwrap_or_else(|e| unreachable!("constant regex: {e}")));

/// `same(a, b)`: equal once runs of whitespace are one space (Python's `str.split()`).
pub fn same(a: &str, b: &str) -> bool {
    py::split(a).eq(py::split(b))
}

/// `words(s)`: the lower-cased `\w+` runs.
pub fn words(s: &str) -> Vec<String> {
    let low = s.to_lowercase();
    WORD_RE.find_iter(&low).map(|m| m.as_str().to_string()).collect()
}

/// `reveal_word(line)`: the punch line's last word.
pub fn reveal_word(line: &str) -> Option<String> {
    words(line).pop()
}

/// `numbered(lines, punch)`.
pub fn numbered(lines: &[String], punch: Option<usize>) -> String {
    lines
        .iter()
        .enumerate()
        .map(|(i, ln)| {
            format!(
                "{}. {ln}{}",
                i + 1,
                if punch == Some(i) { "   <- the punch" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The job as the Python reads it (`inp.get(...)`, every field lenient; `pk_wire::worker::RantInput` /
/// `HookInput` are the API's stricter shapes of the same thing).
#[derive(Debug, Default, Deserialize)]
pub struct CutJob {
    /// Echoed back.
    #[serde(default)]
    pub joke_id: Value,
    /// Echoed back; "untitled" in the prompt when falsy.
    #[serde(default)]
    pub title: Value,
    /// Echoed back.
    #[serde(default)]
    pub unit: Value,
    /// `rant` / `stage`; "stage" when falsy.
    #[serde(default)]
    pub cut: Value,
    /// The version's lines (only the non-blank strings count).
    #[serde(default)]
    pub lines: Value,
    /// A line index, or anything else (ignored).
    #[serde(default)]
    pub punch: Value,
    /// The openers he already saw (hook only).
    #[serde(default)]
    pub exclude: Value,
}

impl CutJob {
    /// `_clean_input`: the non-blank string lines and a punch that is an int (not a bool) inside them.
    pub fn clean(&self) -> (Vec<String>, Option<usize>) {
        let lines: Vec<String> = self
            .lines
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|x| !py::strip(x).is_empty())
            .map(str::to_string)
            .collect();
        let punch = self
            .punch
            .as_i64()
            .filter(|p| *p >= 0)
            .and_then(|p| usize::try_from(p).ok())
            .filter(|p| *p < lines.len());
        (lines, punch)
    }

    /// `_exclude`: the stripped, non-blank strings.
    pub fn exclude(&self) -> Vec<String> {
        self.exclude
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|x| py::strip(x).to_string())
            .filter(|x| !x.is_empty())
            .collect()
    }

    fn title(&self) -> String {
        if py::truthy(Some(&self.title)) {
            py::str(&self.title)
        } else {
            "untitled".to_string()
        }
    }

    /// `_base`.
    fn base(&self, lines: &[String], punch: Option<usize>) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("joke_id".into(), self.joke_id.clone());
        m.insert("title".into(), self.title.clone());
        m.insert("unit".into(), self.unit.clone());
        let cut = if py::truthy(Some(&self.cut)) {
            self.cut.clone()
        } else {
            json!("stage")
        };
        m.insert("cut".into(), cut);
        m.insert("lines".into(), json!(lines));
        m.insert("punch".into(), json!(punch));
        m.insert("draft".into(), json!(true));
        m.insert("label".into(), json!(DRAFT_LABEL));
        m
    }
}

/// The user turn of a rant call.
pub fn rant_user(job: &CutJob) -> String {
    let (lines, punch) = job.clean();
    let head = format!("The bit \"{}\", line by line (his words, verbatim):", job.title());
    format!(
        "{head}\n{}\n\nMake it a rant cut. Return one hunk per line you change and nothing for a line you keep:\n\
- change: line_no = the line, was = that line exactly as written above, now = the new line\n\
- cut: line_no = the line, was = that line exactly as written above, now = \"\"\n\
- add: line_no = the line it goes after (0 = before the first), was = \"\", now = the new line\n\
`why` in three to six plain words. Copy `was` exactly; a hunk whose `was` is not the line is thrown away.",
        numbered(&lines, punch)
    )
}

/// The user turn of a hook call.
pub fn hook_user(job: &CutJob) -> String {
    let (lines, punch) = job.clean();
    let ex = job.exclude();
    let mut msg = format!(
        "The bit \"{}\", line by line (his words, verbatim):\n{}\n\nWrite {HOOKS} openers he could say before line 1, \
each one line, each a different angle; `why` in three to six plain words.",
        job.title(),
        numbered(&lines, punch)
    );
    if !ex.is_empty() {
        msg.push_str("\nHe has seen these and wants others (do not repeat them or their angle):\n");
        msg.push_str(&ex.iter().map(|x| format!("- {x}")).collect::<Vec<_>>().join("\n"));
    }
    msg
}

fn str_or_empty(v: Option<&Value>) -> &str {
    v.and_then(Value::as_str).unwrap_or("")
}

/// `(h.get("why") ...).strip()[:80] or None`.
fn why_of(h: &Map<String, Value>) -> Value {
    let w: String = py::strip(str_or_empty(h.get("why"))).chars().take(80).collect();
    if w.is_empty() { Value::Null } else { json!(w) }
}

/// `rant_hunks`: Claude's hunks, checked: (kept, dropped).
pub fn rant_hunks(data: Option<&Value>, lines: &[String], punch: Option<usize>) -> (Vec<Value>, usize) {
    let (mut out, mut dropped, mut touched) = (Vec::new(), 0usize, HashSet::new());
    let rw = punch.and_then(|p| reveal_word(&lines[p]));
    let n = lines.len();
    let hunks = data.and_then(|d| d.get("hunks")).and_then(Value::as_array);
    for h in hunks.into_iter().flatten() {
        let Some(h) = h.as_object() else {
            dropped += 1;
            continue;
        };
        let op = h.get("op").and_then(Value::as_str);
        let was = str_or_empty(h.get("was"));
        let now = py::strip(str_or_empty(h.get("now"))).to_string();
        let why = why_of(h);
        // an int that is not a bool (serde keeps bools apart); a float or a huge number is not a line
        let Some(k) = h.get("line_no").and_then(Value::as_i64) else {
            dropped += 1;
            continue;
        };
        if py::len(&now) > MAX_LINE_CHARS {
            dropped += 1;
            continue;
        }
        let in_range = |lo: i64, hi: usize| k >= lo && usize::try_from(k).is_ok_and(|x| x <= hi);
        if op == Some("add") {
            if !in_range(0, n) || now.is_empty() {
                dropped += 1;
                continue;
            }
            out.push(json!({"op": "add", "line_no": k, "was": "", "now": now, "why": why}));
            continue;
        }
        if !matches!(op, Some("change" | "cut")) || !in_range(1, n) || touched.contains(&k) {
            dropped += 1;
            continue;
        }
        let i = usize::try_from(k - 1).unwrap_or(usize::MAX);
        let line = &lines[i];
        if !same(was, line) {
            dropped += 1;
            continue;
        }
        if op == Some("cut") {
            if punch == Some(i) {
                dropped += 1;
                continue;
            }
            touched.insert(k);
            out.push(json!({"op": "cut", "line_no": k, "was": line, "now": "", "why": why}));
            continue;
        }
        let nw = words(&now);
        let loses_reveal = punch == Some(i) && rw.as_ref().is_some_and(|r| !nw.contains(r));
        if now.is_empty() || nw == words(line) || loses_reveal {
            dropped += 1;
            continue;
        }
        touched.insert(k);
        out.push(json!({"op": "change", "line_no": k, "was": line, "now": now, "why": why}));
    }
    (out, dropped)
}

/// `hook_rows`: Claude's openers, checked: (kept, dropped).
pub fn hook_rows(data: Option<&Value>, lines: &[String], exclude: &[String]) -> (Vec<Value>, usize) {
    let mut seen: HashSet<Vec<String>> = lines.iter().chain(exclude).map(|x| words(x)).collect();
    let (mut out, mut dropped) = (Vec::new(), 0usize);
    let hooks = data.and_then(|d| d.get("hooks")).and_then(Value::as_array);
    for h in hooks.into_iter().flatten() {
        let obj = h.as_object();
        let text = py::strip(str_or_empty(obj.and_then(|o| o.get("text")))).to_string();
        let w = words(&text);
        if w.is_empty()
            || text.contains('\n')
            || py::len(&text) > MAX_HOOK_CHARS
            || py::split(&text).count() > MAX_HOOK_WORDS
            || seen.contains(&w)
            || out.len() >= HOOKS
        {
            dropped += 1;
            continue;
        }
        seen.insert(w);
        let why = obj.map_or(Value::Null, why_of);
        out.push(json!({"text": text, "why": why, "draft": true, "label": DRAFT_LABEL}));
    }
    (out, dropped)
}

fn finish(mut m: Map<String, Value>, extra: Vec<(&str, Value)>) -> Done {
    for (k, v) in extra {
        m.insert(k.to_string(), v);
    }
    Done::result(Value::Object(m))
}

/// The `rant` kind.
#[derive(Debug)]
pub struct Rant;

impl Runner for Rant {
    type Input = CutJob;

    fn bad_input(e: &serde_json::Error) -> JobError {
        JobError::Value(format!("bad rant input: {e}"))
    }

    async fn run(ctx: Ctx<'_>, job: CutJob) -> Result<Done, JobError> {
        let (lines, punch) = job.clean();
        let mut base = job.base(&lines, punch);
        base.insert("hunks".into(), json!([]));
        base.insert("dropped".into(), json!(0));
        if lines.is_empty() {
            return Ok(finish(
                base,
                vec![("status", json!("empty")), ("why", json!("This version has no lines."))],
            ));
        }
        let task_system = format!("{RANT_SYSTEM}\n\n{RANT_METHOD}");
        let msgs = ctx
            .voice
            .messages(RANT_TASK, &task_system, [ChatMessage::user(rant_user(&job))]);
        let done = match ctx
            .router
            .complete(RANT_TASK, &msgs, Some(&RANT_SCHEMA), Some("creative"), None)
            .await
        {
            Ok(r) => {
                let (mut hunks, dropped) = rant_hunks(r.data.as_ref(), &lines, punch);
                for h in &mut hunks {
                    h["draft"] = json!(true);
                    h["label"] = json!(DRAFT_LABEL);
                }
                let status = if hunks.is_empty() { "none" } else { "ok" };
                finish(
                    base,
                    vec![
                        ("status", json!(status)),
                        ("hunks", Value::Array(hunks)),
                        ("dropped", json!(dropped)),
                        ("provider", json!(r.provider)),
                        ("model", json!(r.model)),
                        ("call_id", json!(r.call_id)),
                        ("task", json!(RANT_TASK)),
                    ],
                )
            }
            Err(e) => finish(
                base,
                vec![("status", json!("provider_error")), ("why", json!(provider_why(&e)))],
            ),
        };
        Ok(done)
    }
}

/// The `hook` kind.
#[derive(Debug)]
pub struct Hook;

impl Runner for Hook {
    type Input = CutJob;

    fn bad_input(e: &serde_json::Error) -> JobError {
        JobError::Value(format!("bad hook input: {e}"))
    }

    async fn run(ctx: Ctx<'_>, job: CutJob) -> Result<Done, JobError> {
        let (lines, punch) = job.clean();
        let ex = job.exclude();
        let mut base = job.base(&lines, punch);
        base.insert("hooks".into(), json!([]));
        base.insert("exclude".into(), json!(ex));
        base.insert("dropped".into(), json!(0));
        if lines.is_empty() {
            return Ok(finish(
                base,
                vec![("status", json!("empty")), ("why", json!("This version has no lines."))],
            ));
        }
        let task_system = format!("{HOOK_SYSTEM}\n\n{HOOK_METHOD}");
        let msgs = ctx
            .voice
            .messages(HOOK_TASK, &task_system, [ChatMessage::user(hook_user(&job))]);
        let done = match ctx
            .router
            .complete(HOOK_TASK, &msgs, Some(&HOOK_SCHEMA), Some("creative"), None)
            .await
        {
            Ok(r) => {
                let (rows, dropped) = hook_rows(r.data.as_ref(), &lines, &ex);
                let status = if rows.is_empty() { "none" } else { "ok" };
                finish(
                    base,
                    vec![
                        ("status", json!(status)),
                        ("hooks", Value::Array(rows)),
                        ("dropped", json!(dropped)),
                        ("provider", json!(r.provider)),
                        ("model", json!(r.model)),
                        ("call_id", json!(r.call_id)),
                        ("task", json!(HOOK_TASK)),
                    ],
                )
            }
            Err(e) => finish(
                base,
                vec![("status", json!("provider_error")), ("why", json!(provider_why(&e)))],
            ),
        };
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines() -> Vec<String> {
        [
            "Self-checkout is the store hiring you for free.",
            "I scanned forty things and nobody clapped.",
            "At my job I at least get a chair.",
            "Unexpected item in the bagging area: my dignity.",
        ]
        .map(String::from)
        .to_vec()
    }

    #[test]
    fn a_hunk_must_quote_his_line_word_for_word() {
        let l = lines();
        let (h, d) = rant_hunks(
            Some(&json!({"hunks": [
                {"op": "change", "line_no": 1, "was": l[0], "now": "You know what self-checkout is?", "why": "direct"},
                {"op": "change", "line_no": 2, "was": "I scanned 40 things and no one clapped.", "now": "x", "why": "p"},
                {"op": "cut", "line_no": 3, "was": "At my job  I at least get a chair. ", "now": "ignored", "why": "t"},
                {"op": "add", "line_no": 0, "was": "", "now": "Stop scanning your own groceries.", "why": "hook"}
            ]})),
            &l,
            Some(3),
        );
        let ops: Vec<&str> = h.iter().filter_map(|x| x["op"].as_str()).collect();
        assert_eq!((ops, d), (vec!["change", "cut", "add"], 1));
        assert_eq!(h[1]["was"], json!(l[2]));
    }

    #[test]
    fn the_punch_keeps_its_reveal_word_and_is_never_cut_and_bools_are_not_lines() {
        let l = lines();
        let lose = json!({"op": "change", "line_no": 4, "was": l[3], "now": "Bagging area: my pride.", "why": "x"});
        let cut = json!({"op": "cut", "line_no": 4, "was": l[3], "now": "", "why": "x"});
        let keep = json!({"op": "change", "line_no": 4, "was": l[3], "now": "Yeah. It's my dignity.", "why": "x"});
        assert_eq!(rant_hunks(Some(&json!({"hunks": [lose]})), &l, Some(3)).1, 1);
        assert_eq!(rant_hunks(Some(&json!({"hunks": [cut]})), &l, Some(3)).1, 1);
        assert_eq!(rant_hunks(Some(&json!({"hunks": [keep]})), &l, Some(3)).0.len(), 1);
        assert_eq!(rant_hunks(Some(&json!({"hunks": [lose]})), &l, None).0.len(), 1);
        let b = json!({"op": "change", "line_no": true, "was": l[0], "now": "x", "why": "x"});
        let f = json!({"op": "change", "line_no": 1.0, "was": l[0], "now": "x", "why": "x"});
        assert_eq!(rant_hunks(Some(&json!({"hunks": [b, f]})), &l, None), (vec![], 2));
    }

    #[test]
    fn hooks_are_new_lines_three_at_most() {
        let l = lines();
        let (rows, d) = hook_rows(
            Some(&json!({"hooks": [
                {"text": "Stop scanning your own groceries.", "why": "claim"},
                {"text": "self-checkout is the store hiring you for free", "why": "a line"},
                {"text": "I want to talk to whoever invented self-checkout.", "why": "seen"},
                {"text": "stop scanning your own groceries!", "why": "repeat"},
                {"text": "A.", "why": "1"}, {"text": "B.", "why": "2"}, {"text": "C.", "why": "past three"}
            ]})),
            &l,
            &["I want to talk to whoever invented self-checkout.".to_string()],
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(d, 4);
    }

    #[test]
    fn the_input_is_read_leniently() {
        let j: CutJob = serde_json::from_value(json!({"lines": ["a", " ", 3, "b"], "punch": true})).unwrap();
        assert_eq!(j.clean(), (vec!["a".to_string(), "b".to_string()], None));
        let j: CutJob = serde_json::from_value(json!({"lines": ["a", "b"], "punch": 1})).unwrap();
        assert_eq!(j.clean().1, Some(1));
        assert!(rant_user(&j).contains("2. b   <- the punch") && rant_user(&j).contains("\"untitled\""));
    }
}
