//! Write's "Ideas from the AI" (`creative/write.py` riff), the halves with no model call: the related bits the
//! riff is grounded in (`riff_grounds`, each with its whole lead text and his latest night), the user turn
//! (`riff_user`), and what is kept of an answer (`riff_ideas`). The worker puts the voice blocks in front and calls
//! the model between the two halves.

use std::fmt::Write as _;
use std::sync::LazyLock;

use pk_core::py;
use serde::Serialize;
use serde_json::{Value, json};

use crate::ask::words::month_year;
use crate::corpus::{Corpus, venue_short};
use crate::notebook::{Index, rating_word, related};

/// `DRAFT_LABEL`.
pub const DRAFT_LABEL: &str = "DRAFT · AI · not your words yet";
/// `RIFF_SYSTEM`: the task's own system line.
pub const RIFF_SYSTEM: &str = "You help one comedian brainstorm. You suggest; he decides. Never invent biography.";
/// `RIFF_BITS`: related bits shown in full.
pub const RIFF_BITS: usize = 4;
/// `RIFF_BIT_CHARS`: of each bit's lead text.
pub const RIFF_BIT_CHARS: usize = 600;
/// Lines drafted at most.
pub const RIFF_CAP: usize = 3;

/// `RIFF_SCHEMA`.
pub static RIFF_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({"type": "object", "properties": {"ideas": {"type": "array", "items": {
        "type": "object", "properties": {"line": {"type": "string"}, "built_on": {"type": "array", "items": {"type": "string"}},
                                         "move": {"type": "string"}},
        "required": ["line", "built_on", "move"], "additionalProperties": false}}},
        "required": ["ideas"], "additionalProperties": false})
});

/// One bit a riff is grounded in.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ground {
    /// `B1`…
    pub id: String,
    /// The bit.
    pub joke_id: String,
    /// Its short title.
    pub title: String,
    /// The line of it that matched.
    pub quote: String,
    /// Its lead text, cut at `RIFF_BIT_CHARS`.
    pub text: String,
    /// One line of stage history.
    pub stage_line: Option<String>,
    /// His latest night in words.
    pub last_night: Option<String>,
}

/// `last_night_words(c, jid)`: "last night at Mabel's, Mar 2024: worked, 6 s of laughs"; `None` when never on tape.
pub fn last_night_words(c: &Corpus, jid: &str) -> Option<String> {
    let p = c.perfs.get(jid)?.last()?;
    let room = venue_short(p.venue.as_str(), None);
    let room = py::strip(&room);
    let where_ = if !room.is_empty() && !room.starts_with("Unknown") {
        format!("at {room}, ")
    } else {
        String::new()
    };
    let word = rating_word(&p.ai_rating).unwrap_or("on stage").to_lowercase();
    let when = month_year(Some(&p.date));
    // `if secs:` on a number is an exact test for zero
    #[allow(clippy::float_cmp)]
    let silent = p.laugh_s == 0.0;
    Some(if silent {
        format!("last night {where_}{when}: {word}, no laugh")
    } else {
        format!("last night {where_}{when}: {word}, {:.0} s of laughs", p.laugh_s)
    })
}

/// `riff_grounds(c, idea, lines)`: up to four related bits that have a quote, each with its whole lead text.
pub fn riff_grounds(c: &Corpus, ix: &Index, idea: &str, lines: &[String]) -> Vec<Ground> {
    let text = format!("{idea}\n{}", lines.join("\n"));
    let mut out = Vec::new();
    for it in related(c, ix, &text, 6) {
        let Some(quote) = it.quote.filter(|q| !q.is_empty()) else {
            continue;
        };
        let lead = c.lead_unit(&it.joke_id);
        let text = lead.as_deref().map_or("", |u| c.node_text(u));
        let mut text = py::strip(text).to_string();
        let chars: Vec<char> = text.chars().collect();
        if chars.len() > RIFF_BIT_CHARS {
            let cut = chars[..RIFF_BIT_CHARS].iter().rposition(|ch| *ch == ' ');
            let at = cut.filter(|c| *c > RIFF_BIT_CHARS / 2).unwrap_or(RIFF_BIT_CHARS);
            let head: String = chars[..at].iter().collect();
            text = format!("{} [...]", head.trim_end_matches(py::isspace));
        }
        out.push(Ground {
            id: format!("B{}", out.len() + 1),
            last_night: last_night_words(c, &it.joke_id),
            joke_id: it.joke_id,
            title: it.title,
            quote,
            text,
            stage_line: it.stage_line,
        });
        if out.len() >= RIFF_BITS {
            break;
        }
    }
    out
}

/// The riff's user turn (`riff_messages`): his idea, his riff, the related bits verbatim with how they did.
/// `whose_idea` is "Pat's idea" when he is named, else "His idea".
pub fn riff_user(whose_idea: &str, idea: &str, lines: &[String], grounds: &[Ground]) -> String {
    let bit = |g: &Ground| {
        let mut stage = g.stage_line.clone().unwrap_or_else(|| "not on tape".to_string());
        if let Some(n) = &g.last_night {
            let _ = write!(stage, " · {n}");
        }
        let body = if !g.text.is_empty() && g.text.contains(&g.quote) {
            format!("\"{}\"", g.text)
        } else {
            let mut b = format!("His line that matches: \"{}\"", g.quote);
            if !g.text.is_empty() {
                let _ = write!(b, "\n\"{}\"", g.text);
            }
            b
        };
        format!("{} {} · {stage}:\n{body}", g.id, g.title)
    };
    let mut msg = format!("{whose_idea}: \"{idea}\"\n");
    if !lines.is_empty() {
        let ls: Vec<String> = lines.iter().map(|x| format!("- {x}")).collect();
        let _ = write!(msg, "His riff so far (his words):\n{}\n", ls.join("\n"));
    }
    if !grounds.is_empty() {
        let bs: Vec<String> = grounds.iter().map(bit).collect();
        let _ = write!(
            msg,
            "Related bits from his own notebook (verbatim), with how they did on stage:\n{}\n\n",
            bs.join("\n\n")
        );
    }
    let _ = write!(
        msg,
        "Draft up to {RIFF_CAP} new lines he could try next. Each line builds on his riff or on the bits above: cite \
         what it builds on in `built_on` (their ids; \"riff\" for his riff) and name the move in `move`. No new facts \
         about his life."
    );
    msg
}

/// The drafts kept from an answer: non-empty lines (400 characters), `built_on` filtered to known ids, the move
/// (80 characters) or null; three at most.
pub fn riff_ideas(data: Option<&Value>, grounds: &[Ground]) -> Vec<Value> {
    let known = |b: &str| b == "riff" || grounds.iter().any(|g| g.id == b);
    let mut out = Vec::new();
    for x in data
        .and_then(|d| d.get("ideas"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let line = py::strip(x.get("line").and_then(Value::as_str).unwrap_or(""));
        if line.is_empty() {
            continue;
        }
        let built_on: Vec<&str> = x
            .get("built_on")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|b| known(b))
            .collect();
        let mv: String = py::strip(x.get("move").and_then(Value::as_str).unwrap_or(""))
            .chars()
            .take(80)
            .collect();
        out.push(
            json!({"text": line.chars().take(400).collect::<String>(), "built_on": built_on,
                        "move": if mv.is_empty() { Value::Null } else { json!(mv) }, "draft": true,
                        "label": DRAFT_LABEL}),
        );
        if out.len() >= RIFF_CAP {
            break;
        }
    }
    out
}
