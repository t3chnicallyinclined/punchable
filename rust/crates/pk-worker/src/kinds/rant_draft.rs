//! `rant_draft` (the Rant tab's "Write a rant", `docs/design/SPEC-2026-11-06-rant-tab.md` §3; Rust only, R7): a rant
//! drafted from a topic and the sources he ticked (his bits, typed ideas, taped moments; the API reads their words
//! from the database, never from the phone). The answer is lines in rant shape (the hook, the setup in one breath,
//! beats that escalate, the turn, the button), each either **his** (quoted word for word from one source) or **new**
//! (a line Claude wrote, shown with the AI badge).
//!
//! The verbatim gate: a line that claims to be his is kept only when the gate (`pk_creative::verbatim`) finds it in
//! the source it names, on word boundaries, and it is then shown as the source's own words (the slice the gate
//! returns); a line that fails is dropped, never shown as his and never quietly turned into a new line. The prompt
//! follows the rant / hook kinds: the voice profile first, the learned block second, the task line with the public
//! method (invented examples only) third, the topic and the numbered sources last. Nothing here writes.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::ops::Range;
use std::sync::LazyLock;

use pk_core::py;
use pk_creative::verbatim::verbatim;
use pk_creative::write::DRAFT_LABEL;
use pk_llm::ChatMessage;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::rant::words;
use super::{Ctx, Done, Runner};
use crate::error::{JobError, provider_why};

/// The task in `engine/config.hosted.toml`: the rant task's Opus routing, shared (one more task would change the
/// Python-recorded routing goldens for a kind the Python never runs).
pub const TASK: &str = super::rant::RANT_TASK;
/// Sources a draft may draw on, at most (the API refuses more).
pub const MAX_SOURCES: usize = 12;
/// One source's words in the prompt, at most (code points).
pub const MAX_SOURCE_CHARS: usize = 4000;
/// A line, at most (code points).
pub const MAX_LINE_CHARS: usize = 400;
/// Lines kept, at most.
pub const MAX_LINES: usize = 24;
/// The roles, in rant order.
pub const ROLES: [&str; 5] = ["hook", "setup", "beat", "turn", "button"];

/// `SYSTEM`.
pub const SYSTEM: &str = "You help one comedian write a rant he will say straight into a phone's front camera, from a topic and \
his own material. You draft; he keeps or skips each line you wrote. Never invent biography.";
/// `METHOD`: public, invented examples only.
pub const METHOD: &str = "How a rant is built. One person, looking into the lens, getting more worked up line by line:\n\
- The hook: one spoken line that makes a stranger stay for the second line.\n\
- The setup, in one breath: what the thing is and why it is wrong.\n\
- Beats that escalate: each one pushes further than the one before it; nothing goes flat in the middle.\n\
- The turn: the line where it flips, the angle nobody saw coming.\n\
- The button: the last line, short, the hardest hit.\n\
Use his lines wherever they fit: a line marked his = true must be copied exactly, word for word, from ONE numbered \
source and name that source (S1, S2, ...); a part of a source line is fine, a changed word is not. Everything else is \
a new line (his = false, source = \"\"): connecting or escalating lines in his voice, spoken and short, one breath \
each. No stage directions, no emoji, no hashtags. Prefer his lines to new ones; never pass a new line off as his.\n\
Example (invented), topic \"self-checkout\", source S1 \"Self-checkout is the store hiring you for free.\": \
hook (new) \"Stop scanning your own groceries.\"; setup (his, S1) \"Self-checkout is the store hiring you for free.\"; \
beat (new) \"You bag. You weigh. You apologise to a machine.\"; turn (new) \"And then it calls a manager. On you.\"; \
button (new) \"I quit. I never worked there.\"";

/// `SCHEMA`.
pub static SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({"type": "object", "properties": {"lines": {"type": "array", "items": {
        "type": "object", "properties": {"role": {"type": "string", "enum": ROLES},
                                         "text": {"type": "string"}, "his": {"type": "boolean"},
                                         "source": {"type": "string"}},
        "required": ["role", "text", "his", "source"], "additionalProperties": false}}},
        "required": ["lines"], "additionalProperties": false})
});

/// One source as the API sends it: his words, read from the database.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Source {
    /// The bit's or the moment's id.
    #[serde(default)]
    pub id: String,
    /// `bit` | `idea` | `moment`.
    #[serde(default)]
    pub kind: String,
    /// What the phone shows under a line from it ("Self-checkout v8").
    #[serde(default)]
    pub label: String,
    /// His words.
    #[serde(default)]
    pub text: String,
}

/// The job (every field lenient, as the other kinds read theirs).
#[derive(Debug, Default, Deserialize)]
pub struct DraftJob {
    /// What the rant is about.
    #[serde(default)]
    pub topic: Value,
    /// The topic row it came from, echoed back.
    #[serde(default)]
    pub topic_id: Value,
    /// The sources he ticked.
    #[serde(default)]
    pub sources: Vec<Value>,
}

impl DraftJob {
    /// The topic, one line.
    pub fn topic(&self) -> String {
        self.topic
            .as_str()
            .map(|t| py::split(t).collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
    }

    /// The sources with words, at most [`MAX_SOURCES`], each cut to [`MAX_SOURCE_CHARS`].
    pub fn sources(&self) -> Vec<Source> {
        self.sources
            .iter()
            .filter_map(|v| serde_json::from_value::<Source>(v.clone()).ok())
            .filter(|s| !py::strip(&s.text).is_empty() && !s.id.is_empty())
            .take(MAX_SOURCES)
            .map(|mut s| {
                if py::len(&s.text) > MAX_SOURCE_CHARS {
                    s.text = s.text.chars().take(MAX_SOURCE_CHARS).collect();
                }
                s
            })
            .collect()
    }
}

/// The user turn.
pub fn user_turn(topic: &str, sources: &[Source]) -> String {
    let mut msg = format!("The topic: \"{topic}\".\n\n");
    if sources.is_empty() {
        msg.push_str("He ticked none of his material for this one: every line is new (his = false).\n");
    } else {
        msg.push_str("His material on it (his words, verbatim), numbered:\n");
        for (i, s) in sources.iter().enumerate() {
            let _ = write!(msg, "\n[S{}] {} ({})\n{}\n", i + 1, s.label, s.kind, s.text.trim_end());
        }
    }
    msg.push_str(
        "\nWrite the rant: 6 to 14 lines in order, the hook first and the button last. For each line: role (hook, \
setup, beat, turn, button), text, his (true only when the text is copied exactly from one source), source (S1, S2, \
... for his lines, \"\" for new ones). A line marked his that is not in its source word for word is thrown away.",
    );
    msg
}

/// Where the slice `found` sits in `source` (a byte range), checked: `None` when it is not a slice of `source` on
/// character boundaries (the gate always returns one; this refuses rather than trusts it).
fn range_in(source: &str, found: &str) -> Option<Range<usize>> {
    let start = (found.as_ptr() as usize).checked_sub(source.as_ptr() as usize)?;
    let end = start.checked_add(found.len())?;
    (end <= source.len() && source.is_char_boundary(start) && source.is_char_boundary(end)).then_some(start..end)
}

/// Whether `source[r]` starts and ends on a word boundary (a quote never starts or stops inside a word: "can" is not
/// in "scan").
fn on_word_boundaries(source: &str, r: &Range<usize>) -> bool {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let found = &source[r.clone()];
    let cut_in = |a: Option<char>, b: Option<char>| word(a) && word(b);
    !(cut_in(source[..r.start].chars().next_back(), found.chars().next())
        || cut_in(source[r.end..].chars().next(), found.chars().next_back()))
}

/// His words for a line that claims to be his: the source's own slice, the first place the gate finds it on word
/// boundaries and within one of his lines, else `None` (a model line that joins two of his lines is not one of his).
pub fn his_words<'a>(text: &str, source: &'a str) -> Option<&'a str> {
    let mut from = 0;
    while from < source.len() {
        let found = verbatim(text, &source[from..])?;
        let r = range_in(source, found)?;
        if on_word_boundaries(source, &r) && !found.contains('\n') {
            return Some(found);
        }
        // look again after this match's first character
        from = r.start + found.chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// Claude's lines, checked: (kept, dropped). `is_his` is the check a line claiming to be his must pass (the gate;
/// the tests pass a lenient one to prove the gate is what drops a paraphrase).
pub fn checked_lines_with(
    data: Option<&Value>,
    sources: &[Source],
    is_his: impl Fn(&str, &str) -> Option<String>,
) -> (Vec<Value>, usize) {
    let (mut out, mut dropped) = (Vec::new(), 0usize);
    let mut seen: HashSet<Vec<String>> = HashSet::new();
    let lines = data.and_then(|d| d.get("lines")).and_then(Value::as_array);
    for l in lines.into_iter().flatten() {
        let Some(l) = l.as_object() else {
            dropped += 1;
            continue;
        };
        let text = py::strip(l.get("text").and_then(Value::as_str).unwrap_or("")).to_string();
        let role = l
            .get("role")
            .and_then(Value::as_str)
            .filter(|r| ROLES.contains(r))
            .unwrap_or("beat");
        let w = words(&text);
        if w.is_empty() || text.contains('\n') || py::len(&text) > MAX_LINE_CHARS || out.len() >= MAX_LINES {
            dropped += 1;
            continue;
        }
        if l.get("his") == Some(&Value::Bool(true)) {
            let src = l.get("source").and_then(Value::as_str).unwrap_or("");
            let k = src
                .strip_prefix('S')
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|n| (1..=sources.len()).contains(n));
            let Some(s) = k.map(|n| &sources[n - 1]) else {
                dropped += 1;
                continue;
            };
            let Some(found) = is_his(&text, &s.text) else {
                dropped += 1;
                continue;
            };
            if !seen.insert(words(&found)) {
                dropped += 1;
                continue;
            }
            out.push(
                json!({"role": role, "text": found, "his": true, "source": s.id, "source_label": s.label,
                            "source_kind": s.kind}),
            );
        } else {
            if !seen.insert(w) {
                dropped += 1;
                continue;
            }
            out.push(
                json!({"role": role, "text": text, "his": false, "source": null, "draft": true,
                            "label": DRAFT_LABEL}),
            );
        }
    }
    (out, dropped)
}

/// [`checked_lines_with`] behind the verbatim gate.
pub fn checked_lines(data: Option<&Value>, sources: &[Source]) -> (Vec<Value>, usize) {
    checked_lines_with(data, sources, |t, s| his_words(t, s).map(str::to_string))
}

/// The `rant_draft` kind.
#[derive(Debug)]
pub struct RantDraft;

impl Runner for RantDraft {
    type Input = DraftJob;

    fn bad_input(e: &serde_json::Error) -> JobError {
        JobError::Value(format!("bad rant_draft input: {e}"))
    }

    async fn run(ctx: Ctx<'_>, job: DraftJob) -> Result<Done, JobError> {
        let topic = job.topic();
        let sources = job.sources();
        let mut base = Map::new();
        base.insert("topic".into(), json!(topic));
        base.insert("topic_id".into(), job.topic_id.clone());
        base.insert(
            "sources".into(),
            json!(
                sources
                    .iter()
                    .map(|s| json!({"id": s.id, "kind": s.kind, "label": s.label}))
                    .collect::<Vec<_>>()
            ),
        );
        base.insert("lines".into(), json!([]));
        base.insert("dropped".into(), json!(0));
        base.insert("draft".into(), json!(true));
        base.insert("label".into(), json!(DRAFT_LABEL));
        if topic.is_empty() {
            base.insert("status".into(), json!("empty"));
            base.insert("why".into(), json!("A rant needs a topic."));
            return Ok(Done::result(Value::Object(base)));
        }
        let task_system = format!("{SYSTEM}\n\n{METHOD}");
        let msgs = ctx
            .voice
            .messages(TASK, &task_system, [ChatMessage::user(user_turn(&topic, &sources))]);
        match ctx
            .router
            .complete(TASK, &msgs, Some(&SCHEMA), Some("creative"), None)
            .await
        {
            Ok(r) => {
                let (lines, dropped) = checked_lines(r.data.as_ref(), &sources);
                let status = if lines.is_empty() { "none" } else { "ok" };
                base.insert("status".into(), json!(status));
                base.insert("lines".into(), Value::Array(lines));
                base.insert("dropped".into(), json!(dropped));
                base.insert("provider".into(), json!(r.provider));
                base.insert("model".into(), json!(r.model));
                base.insert("call_id".into(), json!(r.call_id));
                base.insert("task".into(), json!(TASK));
            }
            Err(e) => {
                base.insert("status".into(), json!("provider_error"));
                base.insert("why".into(), json!(provider_why(&e)));
            }
        }
        Ok(Done::result(Value::Object(base)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources() -> Vec<Source> {
        vec![
            Source {
                id: "joke:self-checkout".into(),
                kind: "bit".into(),
                label: "Self-checkout v8".into(),
                text: "Self-checkout is the store hiring you for free.\nI scanned forty things and nobody clapped."
                    .into(),
            },
            Source {
                id: "performance:p1".into(),
                kind: "moment".into(),
                label: "On tape".into(),
                text: "unexpected item in the bagging area, my dignity".into(),
            },
        ]
    }

    #[test]
    fn his_lines_must_be_in_the_source_they_name() {
        let s = sources();
        let (lines, dropped) = checked_lines(
            Some(&json!({"lines": [
                {"role": "hook", "text": "Stop scanning your own groceries.", "his": false, "source": ""},
                {"role": "setup", "text": "Self-checkout is the store hiring you for free.", "his": true, "source": "S1"},
                {"role": "beat", "text": "I scanned 40 things and nobody clapped.", "his": true, "source": "S1"},
                {"role": "beat", "text": "nobody clapped", "his": true, "source": "S2"},
                {"role": "beat", "text": "nobody clapped", "his": true, "source": "S9"},
                {"role": "turn", "text": "can", "his": true, "source": "S1"},
                {"role": "button", "text": "my dignity", "his": true, "source": "S2"}
            ]})),
            &s,
        );
        let texts: Vec<&str> = lines.iter().filter_map(|l| l["text"].as_str()).collect();
        assert_eq!(
            texts,
            [
                "Stop scanning your own groceries.",
                "Self-checkout is the store hiring you for free.",
                "my dignity"
            ]
        );
        assert_eq!(
            dropped, 4,
            "a paraphrase, the wrong source, no such source, half a word"
        );
        assert_eq!(lines[1]["source"], "joke:self-checkout");
        assert_eq!(lines[1]["source_label"], "Self-checkout v8");
        assert_eq!(lines[0]["his"], false);
        assert_eq!(lines[0]["label"], DRAFT_LABEL);
    }

    #[test]
    fn his_words_are_the_sources_own() {
        let src = "Self-checkout is the store \u{201c}hiring\u{201d} you for free.";
        assert_eq!(
            his_words("the store \"hiring\" you", src),
            Some("the store \u{201c}hiring\u{201d} you")
        );
        assert_eq!(his_words("elf-checkout is", src), None, "inside a word");
        assert_eq!(his_words("the shop hiring you", src), None);
        // the first place inside a word, a later one on boundaries: the later one is his
        let two = "I bought the cart. Then the car.";
        let got = his_words("the car", two).unwrap();
        assert_eq!((got, got.as_ptr() as usize - two.as_ptr() as usize), ("the car", 24));
        // two of his lines joined by the model are not one line of his
        assert_eq!(
            his_words("for free. I scanned", "hiring you for free.\nI scanned forty things"),
            None
        );
    }

    #[test]
    fn the_red_arm_a_lenient_check_lets_the_paraphrase_through() {
        // the same answer through a check that believes every claim: the paraphrase survives, so the assertion in
        // `his_lines_must_be_in_the_source_they_name` is what the gate earns
        let s = sources();
        let answer = json!({"lines": [
            {"role": "beat", "text": "I scanned 40 things and nobody clapped.", "his": true, "source": "S1"}
        ]});
        let (lenient, _) = checked_lines_with(Some(&answer), &s, |t, _| Some(t.to_string()));
        let (strict, d) = checked_lines(Some(&answer), &s);
        assert_eq!((lenient.len(), strict.len(), d), (1, 0, 1));
    }

    #[test]
    fn repeats_and_bad_rows_are_dropped_and_the_input_is_lenient() {
        let s = sources();
        let (lines, dropped) = checked_lines(
            Some(&json!({"lines": [
                {"role": "hook", "text": "Stop.", "his": false, "source": ""},
                {"role": "hook", "text": "stop!", "his": false, "source": ""},
                {"role": "nonsense", "text": "Two\nlines", "his": false, "source": ""},
                {"role": "nonsense", "text": "A beat by default.", "his": false, "source": ""},
                3
            ]})),
            &s,
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1]["role"], "beat");
        assert_eq!(dropped, 3);
        let j: DraftJob = serde_json::from_value(json!({"topic": "  self-checkout \n machines ",
            "sources": [{"id": "joke:a", "kind": "bit", "label": "A", "text": "x"}, {"id": "", "text": "y"}, 3]}))
        .unwrap();
        assert_eq!(j.topic(), "self-checkout machines");
        assert_eq!(j.sources().len(), 1);
        assert!(user_turn(&j.topic(), &j.sources()).contains("[S1] A (bit)\nx"));
    }
}
