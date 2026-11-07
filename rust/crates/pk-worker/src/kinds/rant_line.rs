//! `rant_line` (the Rant tab's script, Claude on one line: `docs/design/SPEC-2026-11-06-rant-tab.md` §12.5; Rust only,
//! R7): three versions of one line, or tags to follow it, with the lines around it and his voice profile as context.
//! **Nothing changes until he picks one:** the kind writes nothing and the phone swaps the line only on his tap. Every
//! option is Claude's, shown with the AI badge; none claims to be his words, so the verbatim gate has nothing to pass
//! here (the words he edits stay his through the script's own marks).

use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::LazyLock;

use pk_core::py;
use pk_creative::write::DRAFT_LABEL;
use pk_llm::ChatMessage;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::rant::words;
use super::rant_draft::MAX_LINE_CHARS;
use super::{Ctx, Done, Runner};
use crate::error::{JobError, provider_why};

/// The task in `engine/config.hosted.toml`: the rant task's routing, shared (a new task would change the
/// Python-recorded routing goldens for a kind the Python never runs).
pub const TASK: &str = super::rant::RANT_TASK;
/// Options shown, at most.
pub const MAX_OPTIONS: usize = 3;

/// `SYSTEM`.
pub const SYSTEM: &str = "You help one comedian with one line of a rant he will say straight into a phone's front camera. You \
offer; he picks one or none. Never invent biography.";
/// `METHOD`: public, invented examples only.
pub const METHOD: &str = "Give three different options for the line he marks. Each is one spoken line in his voice, one breath, no \
stage directions, no emoji, no hashtags. Keep what the line is about and how it sits between the lines around it; do not \
repeat those lines. Three real choices, not three spellings of one idea.\n\
Example (invented), the line \"The machine asked me to wait for help.\", asked for sharper: \"The machine called a manager. On me.\" / \
\"It put me on hold in person.\" / \"I got reported by a scale.\"";

/// `SCHEMA`.
pub static SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({"type": "object", "properties": {"options": {"type": "array", "items": {"type": "string"}}},
           "required": ["options"], "additionalProperties": false})
});

/// What each ask means.
fn ask_words(ask: &str) -> Option<&'static str> {
    Some(match ask {
        "sharper" => "Make it sharper: fewer words around the point, the funny word last.",
        "shorter" => "Make it shorter: the same point in as few words as it can take.",
        "angrier" => "Make it angrier: more worked up, more certain, the same point.",
        "bigger" => "Make it bigger: push the idea further, past where a reasonable person stops.",
        "tag" => {
            "Write tags: three short lines that could follow it, each one getting another laugh off the same idea. \
They come after his line; they do not replace it."
        }
        _ => return None,
    })
}

/// The job.
#[derive(Debug, Default, Deserialize)]
pub struct LineJob {
    /// The rant bit, echoed back.
    #[serde(default)]
    pub joke: Value,
    /// The line he asked about.
    #[serde(default)]
    pub line: Value,
    /// Up to three lines before it.
    #[serde(default)]
    pub before: Vec<Value>,
    /// Up to three lines after it.
    #[serde(default)]
    pub after: Vec<Value>,
    /// `sharper`, `shorter`, `angrier`, `bigger`, `tag`, or nothing with `say`.
    #[serde(default)]
    pub ask: Value,
    /// What he typed.
    #[serde(default)]
    pub say: Value,
}

fn text_list(v: &[Value]) -> Vec<&str> {
    v.iter().filter_map(Value::as_str).collect()
}

/// The user turn.
pub fn user_turn(job: &LineJob) -> String {
    let mut msg = String::new();
    let before = text_list(&job.before);
    let after = text_list(&job.after);
    if !before.is_empty() {
        let _ = write!(msg, "The lines before it:\n{}\n\n", before.join("\n"));
    }
    let _ = write!(msg, "The line: \"{}\"\n\n", job.line.as_str().unwrap_or(""));
    if !after.is_empty() {
        let _ = write!(msg, "The lines after it:\n{}\n\n", after.join("\n"));
    }
    if let Some(a) = job.ask.as_str().and_then(ask_words) {
        let _ = writeln!(msg, "{a}");
    }
    if let Some(s) = job.say.as_str().filter(|s| !s.is_empty()) {
        let _ = writeln!(msg, "What he wants from it: {s}");
    }
    msg.push_str("Answer with three options.");
    msg
}

/// Claude's options, checked: one line each, not over the limit, not his own line back, no repeats, at most three.
pub fn checked_options(data: Option<&Value>, line: &str) -> (Vec<Value>, usize) {
    let (mut out, mut dropped) = (Vec::new(), 0usize);
    let mut seen: HashSet<Vec<String>> = HashSet::new();
    seen.insert(words(line));
    let opts = data.and_then(|d| d.get("options")).and_then(Value::as_array);
    for o in opts.into_iter().flatten() {
        let text = py::strip(o.as_str().unwrap_or("")).to_string();
        let w = words(&text);
        if w.is_empty()
            || text.contains('\n')
            || py::len(&text) > MAX_LINE_CHARS
            || out.len() >= MAX_OPTIONS
            || !seen.insert(w)
        {
            dropped += 1;
            continue;
        }
        out.push(json!({"text": text, "draft": true, "label": DRAFT_LABEL}));
    }
    (out, dropped)
}

/// The `rant_line` kind.
#[derive(Debug)]
pub struct RantLine;

impl Runner for RantLine {
    type Input = LineJob;

    fn bad_input(e: &serde_json::Error) -> JobError {
        JobError::Value(format!("bad rant_line input: {e}"))
    }

    async fn run(ctx: Ctx<'_>, job: LineJob) -> Result<Done, JobError> {
        let line = py::strip(job.line.as_str().unwrap_or("")).to_string();
        let mut base = Map::new();
        base.insert("joke".into(), job.joke.clone());
        base.insert("line".into(), json!(line));
        base.insert("ask".into(), job.ask.clone());
        base.insert("options".into(), json!([]));
        base.insert("dropped".into(), json!(0));
        base.insert("draft".into(), json!(true));
        base.insert("label".into(), json!(DRAFT_LABEL));
        if line.is_empty() {
            base.insert("status".into(), json!("empty"));
            base.insert("why".into(), json!("There is no line to work on."));
            return Ok(Done::result(Value::Object(base)));
        }
        let task_system = format!("{SYSTEM}\n\n{METHOD}");
        let msgs = ctx
            .voice
            .messages(TASK, &task_system, [ChatMessage::user(user_turn(&job))]);
        match ctx
            .router
            .complete(TASK, &msgs, Some(&SCHEMA), Some("creative"), None)
            .await
        {
            Ok(r) => {
                let (options, dropped) = checked_options(r.data.as_ref(), &line);
                let status = if options.is_empty() { "none" } else { "ok" };
                if options.is_empty() {
                    base.insert("why".into(), json!("Claude has nothing new for this line."));
                }
                base.insert("status".into(), json!(status));
                base.insert("options".into(), Value::Array(options));
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

    #[test]
    fn his_own_line_back_repeats_and_bad_rows_are_dropped_and_three_at_most() {
        let (out, dropped) = checked_options(
            Some(&json!({"options": [
                "The machine asked me to wait for help!",
                "It called a manager. On me.",
                "it called a manager, on me",
                "Two\nlines",
                3,
                "It put me on hold in person.",
                "I got reported by a scale.",
                "A fourth one."
            ]})),
            "The machine asked me to wait for help.",
        );
        let texts: Vec<&str> = out.iter().filter_map(|o| o["text"].as_str()).collect();
        assert_eq!(
            texts,
            [
                "It called a manager. On me.",
                "It put me on hold in person.",
                "I got reported by a scale."
            ]
        );
        assert_eq!(dropped, 5);
        assert!(
            out.iter()
                .all(|o| o["draft"] == json!(true) && o["label"] == DRAFT_LABEL)
        );
    }

    #[test]
    fn the_ask_reaches_the_prompt_and_an_unknown_one_is_only_his_words() {
        let j: LineJob = serde_json::from_value(json!({
            "line": "L", "before": ["B1"], "after": ["A1", 3], "ask": "tag", "say": "about the scale"
        }))
        .unwrap();
        let t = user_turn(&j);
        assert!(t.contains("B1") && t.contains("A1") && t.contains("Write tags") && t.contains("about the scale"));
        let j: LineJob = serde_json::from_value(json!({"line": "L", "ask": "explode"})).unwrap();
        assert!(!user_turn(&j).contains("explode"));
    }
}
