//! Say it or ask it: the deterministic part of `POST /api/say` (`api-dev/say_api.py`), with no IO. One input on every
//! screen; the app does not ask him which it is.
//!
//! [`classify`] decides question or bit (or command) from the words alone: a question mark; a leading wh-word,
//! auxiliary or request ("build me", "give me"); "my material / my bits / my closer". "what if ..." without a question
//! mark is a premise, so a bit. Everything else is a bit: a lost punchline costs more than a misfiled question.
//! [`parse_command`] reads the two commands ("new 7 for friday" makes a set list of that length; "add the kale bit to
//! tonight" puts a bit on tonight's board) and [`match_bits`] finds the bit a words query means.
//!
//! The clock is an argument: a command without a target falls back to the weekday, which the caller names.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::corpus::{Corpus, round_to};
use crate::display::short_name_of;

/// How many candidates a command offers (`MAX_CANDIDATES`).
pub const MAX_CANDIDATES: usize = 5;

fn re(p: &str) -> Regex {
    Regex::new(p).unwrap_or_else(|e| unreachable!("constant regex {p}: {e}"))
}

const FILLERS: [&str; 15] = [
    "oh", "and", "so", "um", "uh", "hey", "ok", "okay", "also", "wait", "like", "yeah", "well", "hmm", "but",
];
const QUESTION_STARTS: &[&str] = &[
    "what", "whats", "what's", "when", "where", "which", "who", "whos", "who's", "why", "how", "hows", "how's", "do",
    "does", "did", "can", "could", "should", "would", "will", "is", "are", "was", "were", "have", "has", "had", "am",
    "any", "anything", "anyone", "got", "remind", "list", "show", "find", "pull", "give", "build", "make", "tell",
    "search", "look",
];

static REQUEST: LazyLock<Regex> = LazyLock::new(|| {
    re(r"\A(?:build|make|give|show|find|pull|put|get|tell|write|new|sort|run)\b.*\b(?:me|up|my|a|an|the)\b")
});
static NEW_SET: LazyLock<Regex> = LazyLock::new(|| re(r"\A(?:new|a new|build|make)\b.*\b\d+\b"));
static MY_MATERIAL: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\bmy (?:bits?|material|jokes?|sets?|closer|opener|notebook|stuff|best|strongest|weakest|last set|tight \d+|\d+)\b",
    )
});
static PREMISE: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"\A(?:what if|imagine|picture this|you know (?:how|when|what)|so my|my (?:dad|mom|wife|husband|",
        r"girlfriend|boyfriend|kid|kids|son|daughter|brother|sister|roommate|neighbou?r|landlord|",
        r"boss|doctor|dentist|uber|therapist|cat|dog))\b"
    ))
});
static WORD: LazyLock<Regex> = LazyLock::new(|| re(r"[a-z0-9']+"));

/// The number words a command may use, in the Python dict's order (the alternation below is sorted by length, longest
/// first, ties in this order).
const NUM_WORDS: [(&str, i64); 18] = [
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("twelve", 12),
    ("fifteen", 15),
    ("twenty", 20),
    ("thirty", 30),
    ("forty", 40),
    ("forty-five", 45),
    ("forty five", 45),
    ("sixty", 60),
];

static NEW_SET_CMD: LazyLock<Regex> = LazyLock::new(|| {
    let mut ws: Vec<&str> = NUM_WORDS.iter().map(|(w, _)| *w).collect();
    // `sorted(NUM_WORDS, key=len, reverse=True)`: longest first, a stable sort keeps the dict's order among equals
    ws.sort_by_key(|w| std::cmp::Reverse(w.chars().count()));
    let num = format!(r"(\d{{1,3}}|{})", ws.join("|"));
    re(&format!(
        concat!(
            r"\A(?:(?:make|build|start|create|give|set up|setup)(?: me)?|new|a new)\s+(?:(?:a|an|another)\s+)?",
            r"(?:(?:new|tight|quick|fresh|clean|solid|killer)\s+)*{}",
            r"(?:\s*(?:min|mins|minute|minutes|minuter|m))?(?:\s+(?:set|set list|setlist|list|one|spot))?",
            r"(?:\s+(?:for|at|on)\s+(.+?))?\s*$"
        ),
        num
    ))
});
static ADD_CMD: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"\A(?:add|put|stick|throw|drop|slot)\s+(?:in\s+)?(?:the\s+)?(.+?)\s+(?:in|into|on|onto|to)\s+",
        r"(?:tonight(?:'s|s)?(?:\s+(?:set\s+list|setlist|set|list))?|",
        r"(?:the|my)\s+(?:set\s+list|setlist|set|list)(?:\s+(?:for\s+)?tonight)?)",
        r"\s*$"
    ))
});
static LEADING_ARTICLE: LazyLock<Regex> = LazyLock::new(|| re(r"\A(?:the|a|an)\s+"));
const QUERY_STOP: &[&str] = &[
    "the", "a", "an", "my", "that", "this", "bit", "one", "joke", "thing", "about", "on", "of", "to", "and", "with",
    "it",
];

/// `_words(text)`: the lower-cased words of a text (letters, digits and apostrophes).
pub fn words(text: &str) -> Vec<String> {
    let low = text.to_lowercase().replace('\u{2019}', "'");
    WORD.find_iter(&low).map(|m| m.as_str().to_string()).collect()
}

fn is_query_stop(w: &str) -> bool {
    QUERY_STOP.contains(&w)
}

/// `classify(text)`: `(kind, rule)`: `question`, `bit` or `command`, and the test that decided.
pub fn classify(text: &str) -> (&'static str, &'static str) {
    let t = pk_core::py::split(text).collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        return ("bit", "empty");
    }
    if pk_core::py::strip(&t).ends_with('?') {
        return ("question", "question_mark");
    }
    let mut ws = words(&t);
    let skip = ws.iter().take_while(|w| FILLERS.contains(&w.as_str())).count();
    ws.drain(..skip);
    if ws.is_empty() {
        return ("bit", "only_fillers");
    }
    let low = ws.join(" ");
    if PREMISE.is_match(&low) {
        return ("bit", "premise");
    }
    if let Some(cmd) = parse_command(&low, "") {
        return (
            "command",
            if cmd.get("op").and_then(Value::as_str) == Some("new_set") {
                "command_new_set"
            } else {
                "command_add_to_tonight"
            },
        );
    }
    let first = ws[0].as_str();
    if QUESTION_STARTS.contains(&first) && (!matches!(first, "got" | "find" | "look" | "any") || ws.len() >= 2) {
        return ("question", "leading_wh_or_aux");
    }
    if REQUEST.is_match(&low) || NEW_SET.is_match(&low) {
        return ("question", "request");
    }
    if MY_MATERIAL.is_match(&low) {
        return ("question", "my_material");
    }
    ("bit", "default")
}

/// `parse_command(text)`: the command in a line, or `None`. `{op: new_set, minutes, title}` or
/// `{op: add_to_tonight, query, words}`. `weekday` is the title a new set takes when it names no target (the
/// Python's `time.strftime("%A")`).
pub fn parse_command(text: &str, weekday: &str) -> Option<Value> {
    let lowered = text.to_lowercase().replace('\u{2019}', "'");
    let t = pk_core::py::split(&lowered).collect::<Vec<_>>().join(" ");
    let t = t.trim_end_matches(['.', '!', ' ']);
    if t.is_empty() || t.ends_with('?') {
        return None;
    }
    if let Some(m) = NEW_SET_CMD.captures(t) {
        let n = &m[1];
        // `int(n) if n.isdigit() else NUM_WORDS.get(n)`
        let mins = if n.bytes().all(|b| b.is_ascii_digit()) {
            n.parse::<i64>().ok()
        } else {
            NUM_WORDS.iter().find(|(w, _)| *w == n).map(|(_, v)| *v)
        };
        let mins = mins.filter(|m| (1..=240).contains(m))?;
        let target = m.get(2).map_or("", |g| g.as_str());
        let target = target.trim_matches(|c| matches!(c, ' ' | '.' | ',' | '!'));
        // `target.strip(" .,!")` strips both ends; `re.sub(r"^(the|a|an)\s+", "", target)`
        let target = LEADING_ARTICLE.replace(target, "");
        let title = if target.is_empty() {
            weekday.to_string()
        } else {
            let mut cs = target.chars();
            let first: String = cs.next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            format!("{first}{}", cs.as_str())
        };
        let title: String = title.chars().take(40).collect();
        return Some(json!({"op": "new_set", "minutes": mins, "title": format!("{title} \u{b7} {mins}")}));
    }
    if let Some(m) = ADD_CMD.captures(t) {
        let q = m[1].trim_matches(|c| matches!(c, ' ' | '.' | ',' | '!' | '\'' | '"'));
        let ws: Vec<String> = words(q).into_iter().filter(|w| !is_query_stop(w)).collect();
        return Some(json!({"op": "add_to_tonight", "query": q, "words": ws}));
    }
    None
}

/// `match_bits(c, words, query, cap)`: fuzzy match on title and short name (word-prefix overlap, a bonus for the
/// whole query inside the name). Best first; `sure` on the first one when it is the one clear hit.
pub fn match_bits(c: &Corpus, wanted: &[String], query: &str, cap: usize) -> Vec<Value> {
    if wanted.is_empty() && query.is_empty() {
        return Vec::new();
    }
    let q = words(query).join(" ");
    let mut rows: Vec<(f64, usize, String, Map<String, Value>)> = Vec::new();
    for jid in &c.jokes {
        let Some(n) = c.joke(jid) else { continue };
        let title = n
            .get("title")
            .filter(|v| pk_core::py::truthy(Some(v)))
            .map_or_else(String::new, pk_core::py::str);
        let short = short_name_of(&title, n.get("short_name").and_then(Value::as_str));
        let mut tw: Vec<String> = words(&title);
        tw.extend(words(&short));
        if tw.is_empty() {
            continue;
        }
        let hit = wanted
            .iter()
            .filter(|w| {
                tw.iter().any(|x| {
                    x == *w
                        || (w.chars().count() >= 4 && x.starts_with(w.as_str()))
                        || (x.chars().count() >= 4 && w.starts_with(x.as_str()))
                })
            })
            .count();
        let mut score = if wanted.is_empty() {
            0.0
        } else {
            hit as f64 / wanted.len() as f64
        };
        let low = format!("{} | {}", words(&title).join(" "), words(&short).join(" "));
        if !q.is_empty() && low.contains(&q) {
            score += 0.3;
        }
        if score <= 0.0 {
            continue;
        }
        let nights = c.perfs.get(jid).map_or(0, Vec::len);
        let mut row = Map::new();
        row.insert("joke_id".into(), json!(jid));
        row.insert("title".into(), json!(crate::ask::words::short_title(Some(&title))));
        row.insert("short_name".into(), json!(short));
        row.insert("score".into(), json!(round_to(score.min(1.0), 3)));
        row.insert("nights".into(), json!(nights));
        rows.push((round_to(score.min(1.0), 3), nights, jid.clone(), row));
    }
    // `rows.sort(key=lambda r: (-r["score"], -r["nights"], r["joke_id"]))`
    rows.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)).then_with(|| a.2.cmp(&b.2)));
    rows.truncate(cap);
    let mut out: Vec<Value> = rows.into_iter().map(|r| Value::Object(r.3)).collect();
    if let Some(top) = out.first().and_then(|r| r["score"].as_f64()) {
        let second = out.get(1).and_then(|r| r["score"].as_f64()).unwrap_or(0.0);
        let sure = top >= 0.75 && (top - second >= 0.25 || (top >= 1.0 && second < 1.0));
        if let Some(Value::Object(first)) = out.first_mut() {
            first.insert("sure".into(), json!(sure));
        }
    }
    out
}
