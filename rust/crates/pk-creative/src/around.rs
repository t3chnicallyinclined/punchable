//! Around this bit (`docs/design/SPEC-2026-11-16-bits-sets-tapes.md` §3.1): everything tied to one bit, grouped by
//! why, each card carrying his own words, plus the sets the bit is in. A join of what already exists: the merge
//! judge's group and what sounds like the bit, the connection groups ([`crate::connections`]), the taped moments that
//! sound like it, the newest night's different wording, the notes filed under it, and the set lists. Nothing here
//! decides anything new except where a card opens ([`anchor`], a guess that never moves his words) and that a pair he
//! dismissed stays gone ([`crate::reducers::links`]).
//!
//! Rust only: no Python twin, so no differential; `tests` below hold the synthetic cases, written by hand.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use crate::corpus::{Corpus, venue_short};
use crate::merge::MergeState;
use crate::reducers::links::{dismissed_pairs, sorted_pair};
use crate::store::Items;
use pk_core::py;

/// The groups, in the order the page shows them: (key, title).
pub const GROUPS: [(&str, &str); 5] = [
    ("same_idea", "Same idea"),
    ("callbacks", "Callbacks"),
    ("on_stage", "On stage"),
    ("loose", "Loose scraps"),
    ("same_shape", "Same shape"),
];
/// Lines of his one card carries, at most (the half sheet lets him pick one).
pub const MAX_WORDS: usize = 12;
/// One line on a card, at most (code points).
pub const MAX_LINE_CHARS: usize = 280;
/// The anchor's bar: at least this many content words shared with the line.
pub const ANCHOR_MIN_SHARED: usize = 2;
/// The anchor's bar: at least this share of the smaller side's content words.
pub const ANCHOR_MIN_SHARE: f64 = 0.5;

/// Another bit as a card shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bit {
    /// What he calls it.
    pub title: String,
    /// What it says now: its newest version's lines, else his typed idea.
    pub lines: Vec<String>,
    /// It has a written version (a typed idea with none is a loose scrap).
    pub written: bool,
    /// Its facts in a few words ("killed twice", "3 nights", "never on stage").
    pub meta: String,
}

/// The newest night on which he said the bit differently from the written version.
#[derive(Debug, Clone, PartialEq)]
pub struct Wording {
    /// The night's set key.
    pub set: String,
    /// Where and when ("Tuesday, Mar 4").
    pub label: String,
    /// What he said that the written version does not have.
    pub lines: Vec<String>,
    /// How much of the written version he said, when known.
    pub on_script_pct: Option<f64>,
}

/// `set:<key>` for a set key (an id that already carries the prefix is kept).
pub fn set_ref(key: &str) -> String {
    if key.starts_with("set:") {
        key.to_string()
    } else {
        format!("set:{key}")
    }
}

/// The newest night whose words differ from the written version (the set page's `script_diff.added`), or none.
pub fn wording(c: &Corpus, jid: &str) -> Option<Wording> {
    let mut ps: Vec<&crate::corpus::Perf> = c.perfs.get(jid)?.iter().collect();
    ps.sort_by(|a, b| b.date.cmp(&a.date));
    ps.into_iter().find_map(|p| {
        let sd = p.script_diff.as_object()?;
        let lines: Vec<String> = sd
            .get("added")
            .and_then(Value::as_array)?
            .iter()
            .filter_map(|x| x.as_str().or_else(|| x.get("quote").and_then(Value::as_str)))
            .map(|l| py::strip(l).to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if lines.is_empty() {
            return None;
        }
        Some(Wording {
            set: p.set.clone(),
            label: venue_short(p.venue.as_str(), Some(&p.date)),
            lines,
            on_script_pct: crate::corpus::num(sd.get("on_script_pct")),
        })
    })
}

/// Everything the join reads, already fetched.
pub struct Sources<'a> {
    /// The bit.
    pub joke: &'a str,
    /// Its newest version's unit, when it has one.
    pub unit: Option<&'a str>,
    /// Its newest version's lines.
    pub lines: &'a [String],
    /// `connections::joke_connections`, or none when the notebook does not hold the bit.
    pub connections: Option<&'a Value>,
    /// `merge::candidates_for`: the judge's group and the closest other bits.
    pub merge: &'a Value,
    /// The bits that sound like it, nearest first (`/similar`'s `like.bits`).
    pub like_bits: &'a [String],
    /// The taped moments that sound like it (`/similar`'s `like.moments` rows).
    pub like_moments: &'a [Value],
    /// The newest night's different wording.
    pub wording: Option<&'a Wording>,
    /// The set lists (the `storyboards` store).
    pub boards: &'a Items,
    /// The pairs he said are not connected (the `links` store).
    pub links: &'a Items,
    /// His merges, the notes filed under bits, the pairs he said are not the same bit.
    pub merges: &'a MergeState,
    /// Items per group, at most.
    pub cap: usize,
}

/// A line cut to what a card carries.
fn clip(line: &str) -> String {
    let t = py::strip(line);
    if py::len(t) <= MAX_LINE_CHARS {
        t.to_string()
    } else {
        let cut: String = t.chars().take(MAX_LINE_CHARS - 1).collect();
        format!("{}…", py::strip(&cut))
    }
}

/// The lines of a text as a card carries them.
fn card_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    lines
        .into_iter()
        .flat_map(|l| l.split('\n'))
        .map(clip)
        .filter(|l| !l.is_empty())
        .take(MAX_WORDS)
        .collect()
}

/// Words too common to tie a card to a line.
const STOP: [&str; 44] = [
    "the", "and", "that", "this", "with", "for", "was", "are", "but", "not", "you", "your", "have", "has", "had",
    "his", "her", "its", "they", "them", "then", "than", "what", "when", "who", "how", "why", "from", "just", "like",
    "all", "out", "about", "there", "their", "would", "could", "said", "says", "get", "got", "into", "now", "one",
];

/// The content words of a text: lowercased runs of letters and digits, three characters or more, common words out.
pub fn content_words(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3 && !STOP.contains(w))
        .map(str::to_string)
        .collect()
}

/// The line of THIS bit a card belongs to: the one sharing the most content words with the card's words, when it
/// clears the bar ([`ANCHOR_MIN_SHARED`], [`ANCHOR_MIN_SHARE`]); the first such line on a tie; else none (the card is
/// about the whole bit). A guess by construction: it decides where a card opens, never what his words are.
pub fn anchor(lines: &[String], words: &[String]) -> Option<(usize, String)> {
    let card = content_words(&words.join(" "));
    if card.is_empty() {
        return None;
    }
    let mut best: Option<(usize, usize)> = None;
    for (i, line) in lines.iter().enumerate() {
        let mine = content_words(line);
        let shared = mine.intersection(&card).count();
        let small = mine.len().min(card.len());
        if shared < ANCHOR_MIN_SHARED || small == 0 || (shared as f64) < ANCHOR_MIN_SHARE * small as f64 {
            continue;
        }
        if best.is_none_or(|(_, n)| shared > n) {
            best = Some((i, shared));
        }
    }
    best.map(|(i, _)| (i, lines[i].clone()))
}

/// What he can do with a card, in the order the sheet offers it. `his` = the tie is already his (nothing to connect).
fn does(group: &str, kind: &str, his: bool) -> Vec<&'static str> {
    match (group, kind) {
        ("same_idea", _) => vec!["merge", "add", "dismiss"],
        (_, "moment") => vec!["link", "add", "dismiss"],
        (_, "wording") => vec!["use", "dismiss"],
        (_, "note") => vec!["add", "dismiss"],
        (_, "idea") => vec!["attach", "add", "dismiss"],
        ("on_stage", _) if his => vec!["dismiss"],
        ("on_stage", _) => vec!["connect", "dismiss"],
        _ if his => vec!["add", "dismiss"],
        _ => vec!["connect", "add", "dismiss"],
    }
}

/// The groups being filled: an item is on the page once, in the first group that takes it.
struct Fill<'a> {
    s: &'a Sources<'a>,
    gone: HashSet<(String, String)>,
    seen: HashSet<String>,
    groups: Vec<Vec<Value>>,
}

/// One card before it is checked and placed.
struct Card<'a> {
    group: &'a str,
    kind: &'a str,
    r#ref: &'a str,
    title: String,
    words: Vec<String>,
    meta: String,
    why: String,
    his: bool,
    edge: Option<&'a str>,
    /// The edge to add when he connects a card that has none: (kind, src, dst).
    connect: Option<(&'a str, &'a str, &'a str)>,
    /// The attach record of a note filed here.
    attach: Option<&'a str>,
}

impl Fill<'_> {
    fn slot(group: &str) -> usize {
        GROUPS.iter().position(|(k, _)| *k == group).unwrap_or(0)
    }

    /// Whether he said this pair is not connected (a computed card only: a stored edge is rejected instead).
    fn dismissed(&self, other: &str) -> bool {
        self.gone.contains(&sorted_pair(self.s.joke, other))
    }

    fn push(&mut self, c: &Card<'_>) {
        if c.r#ref == self.s.joke || c.words.is_empty() || self.seen.contains(c.r#ref) {
            return;
        }
        if c.edge.is_none() && c.attach.is_none() && self.dismissed(c.r#ref) {
            return;
        }
        self.seen.insert(c.r#ref.to_string());
        let line = anchor(self.s.lines, &c.words).map_or(Value::Null, |(i, text)| json!({"i": i, "text": text}));
        let mut m = Map::new();
        m.insert("id".into(), json!(format!("{}:{}", c.group, c.r#ref)));
        m.insert("kind".into(), json!(c.kind));
        m.insert("ref".into(), json!(c.r#ref));
        m.insert("title".into(), json!(c.title));
        m.insert("words".into(), json!(c.words));
        m.insert("meta".into(), json!(c.meta));
        m.insert("why".into(), json!(c.why));
        m.insert("source".into(), json!(if c.his { "his" } else { "ai" }));
        m.insert("edge".into(), json!(c.edge));
        m.insert("line".into(), line);
        m.insert("does".into(), json!(does(c.group, c.kind, c.his)));
        if c.edge.is_none()
            && let Some((kind, src, dst)) = c.connect
        {
            m.insert("connect".into(), json!({"kind": kind, "src": src, "dst": dst}));
        }
        if let Some(a) = c.attach {
            m.insert("attach".into(), json!(a));
        }
        self.groups[Self::slot(c.group)].push(Value::Object(m));
    }
}

fn sget<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// The items of one group of `joke_connections`.
fn conn_items<'a>(conns: Option<&'a Value>, key: &str) -> impl Iterator<Item = &'a Value> {
    conns
        .and_then(|c| c.get("groups"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(move |g| g.get("key").and_then(Value::as_str) == Some(key))
        .flat_map(|g| g.get("items").and_then(Value::as_array).into_iter().flatten())
}

fn his_of(it: &Value) -> bool {
    it.get("source").and_then(Value::as_str) == Some("his")
}

fn has_basis(it: &Value, b: &str) -> bool {
    it.get("basis")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(b)))
}

/// "Same moves: misdirect, list" from a same-shape item's shared labels, else the plain word.
fn shape_why(it: &Value, his: bool) -> String {
    let names: Vec<String> = it
        .get("shared")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(2)
        .map(str::to_string)
        .collect();
    if his {
        "You said these have the same shape".to_string()
    } else if names.is_empty() {
        "Same shape".to_string()
    } else {
        format!("Same moves: {}", names.join(", "))
    }
}

/// The sets a bit is in: his live set lists (not archived, not deleted, not a draft the app made) that hold it, in
/// store order, each with where it sits. `notes` is the bit's bullet notes in that set (none until the `setnotes`
/// store exists).
pub fn sets_of(boards: &Items, jid: &str) -> Vec<Value> {
    boards
        .values()
        .filter_map(Value::as_object)
        .filter(|b| !py::truthy(b.get("deleted")) && !py::truthy(b.get("archived")))
        .filter(|b| b.get("source").and_then(Value::as_str) != Some("inferred"))
        .filter_map(|b| {
            let ids = b.get("joke_ids").and_then(Value::as_array)?;
            let at = ids.iter().position(|x| x.as_str() == Some(jid))?;
            Some(
                json!({"board": b.get("id"), "title": b.get("title"), "position": at + 1, "count": ids.len(),
                        "notes": []}),
            )
        })
        .collect()
}

/// `GET /api/jokes/<id>/around`: the groups and the sets. `bit` hands out another bit as a card shows it, or none for
/// one this reader cannot see (hidden, merged away, unknown): its card is left out.
pub fn build(s: &Sources<'_>, bit: &dyn Fn(&str) -> Option<Bit>) -> Value {
    let mut gone = dismissed_pairs(s.links);
    // "not the same bit" (the merges store) is the same statement about two bits
    gone.extend(s.merges.dismissed.iter().cloned());
    let mut fill = Fill {
        s,
        gone,
        seen: HashSet::new(),
        groups: vec![Vec::new(); GROUPS.len()],
    };

    // same idea, first: the judge's group for this bit
    let members: Vec<&str> = s
        .merge
        .get("group")
        .and_then(|g| g.get("members"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|member| sget(member, "joke_id"))
        .collect();
    for j in members {
        if let Some(b) = bit(j) {
            fill.push(&Card {
                group: "same_idea",
                kind: "bit",
                r#ref: j,
                title: b.title,
                words: card_lines(b.lines.iter().map(String::as_str)),
                meta: b.meta,
                why: "Might be this same bit".into(),
                his: false,
                edge: None,
                connect: None,
                attach: None,
            });
        }
    }

    // loose scraps, first: the notes filed under this bit (his call, or the AI's)
    let mut notes: Vec<&Value> = s
        .merges
        .attached
        .values()
        .filter(|rec| rec.get("into").and_then(Value::as_str) == Some(s.joke))
        .collect();
    notes.sort_by_key(|rec| {
        (
            sget(rec, "date").unwrap_or("").to_string(),
            sget(rec, "id").unwrap_or("").to_string(),
        )
    });
    for rec in notes.into_iter().rev() {
        let (Some(j), Some(aid)) = (sget(rec, "joke"), sget(rec, "id")) else {
            continue;
        };
        let his = rec.get("source").and_then(Value::as_str) == Some("owner");
        let date = sget(rec, "date").unwrap_or("");
        fill.push(&Card {
            group: "loose",
            kind: "note",
            r#ref: j,
            title: if date.is_empty() {
                "Note".into()
            } else {
                format!("Note, {date}")
            },
            words: card_lines(sget(rec, "text")),
            meta: String::new(),
            why: if his {
                "You filed this under the bit".into()
            } else {
                "Filed under this bit".into()
            },
            his,
            edge: None,
            connect: None,
            attach: Some(aid),
        });
    }

    // callbacks: his callback edges either way, then the lines that tag it
    for it in conn_items(s.connections, "callbacks") {
        let Some(j) = sget(it, "joke_id") else { continue };
        let Some(b) = bit(j) else { continue };
        let why = if it.get("direction").and_then(Value::as_str) == Some("calls_back_this") {
            "It calls back to this bit"
        } else {
            "This bit calls back to it"
        };
        fill.push(&Card {
            group: "callbacks",
            kind: "bit",
            r#ref: j,
            title: b.title,
            words: card_lines(b.lines.iter().map(String::as_str)),
            meta: b.meta,
            why: why.into(),
            his: his_of(it),
            edge: sget(it, "edge"),
            connect: None,
            attach: None,
        });
    }
    for it in conn_items(s.connections, "tags") {
        let (Some(unit), Some(line)) = (sget(it, "unit_id"), sget(it, "line")) else {
            continue;
        };
        let owner = sget(it, "joke_id");
        // a line of a bit he cannot see is not shown; a line that belongs to no bit is
        let from = match owner {
            Some(o) => match bit(o) {
                Some(b) => Some(b),
                None => continue,
            },
            None => None,
        };
        let his = his_of(it);
        fill.push(&Card {
            group: "callbacks",
            kind: "line",
            r#ref: unit,
            title: from.as_ref().map_or_else(|| "A line".to_string(), |b| b.title.clone()),
            words: card_lines([line]),
            meta: from.map(|b| b.meta).unwrap_or_default(),
            why: if his {
                "You tagged the bit with this"
            } else {
                "Could tag it: lands on the same note"
            }
            .into(),
            his,
            edge: sget(it, "edge"),
            connect: Some(("tag_of", unit, s.joke)),
            attach: None,
        });
    }

    // on stage: what he does right after it
    for it in conn_items(s.connections, "next") {
        let Some(j) = sget(it, "joke_id") else { continue };
        let Some(b) = bit(j) else { continue };
        let his = his_of(it);
        let why = if has_basis(it, "back_to_back") {
            let n = it.get("nights").and_then(Value::as_u64).unwrap_or(0);
            if n > 1 {
                format!("Done right after this on stage, {n} times")
            } else {
                "Done right after this on stage".to_string()
            }
        } else if his {
            "You put it after this bit".to_string()
        } else {
            "Could come right after this bit".to_string()
        };
        fill.push(&Card {
            group: "on_stage",
            kind: "bit",
            r#ref: j,
            title: b.title,
            words: card_lines(b.lines.iter().map(String::as_str)),
            meta: b.meta,
            why,
            his,
            edge: sget(it, "edge"),
            connect: Some(("segue", s.joke, j)),
            attach: None,
        });
    }

    // same shape
    for it in conn_items(s.connections, "same_shape") {
        let Some(j) = sget(it, "joke_id") else { continue };
        let Some(b) = bit(j) else { continue };
        let his = his_of(it);
        fill.push(&Card {
            group: "same_shape",
            kind: "bit",
            r#ref: j,
            title: b.title,
            words: card_lines(b.lines.iter().map(String::as_str)),
            meta: b.meta,
            why: shape_why(it, his),
            his,
            edge: sget(it, "edge"),
            connect: Some(("same_shape", s.joke, j)),
            attach: None,
        });
    }

    // what sounds like it and is not tied to it yet: a written bit may be the same idea, a typed idea with no
    // version is a loose scrap; the judge's closest others that are loose scraps too
    let near = s.like_bits.iter().map(String::as_str).chain(
        s.merge
            .get("others")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|o| sget(o, "joke_id"))
            .filter(|o| bit(o).is_some_and(|b| !b.written)),
    );
    for j in near.collect::<Vec<_>>() {
        // a note filed under any bit has its home; it is not suggested again
        if s.merges.attached.contains_key(j) {
            continue;
        }
        let Some(b) = bit(j) else { continue };
        let (group, kind, why) = if b.written {
            ("same_idea", "bit", "Sounds like this bit")
        } else {
            ("loose", "idea", "Loose scrap, sounds like this bit")
        };
        fill.push(&Card {
            group,
            kind,
            r#ref: j,
            title: b.title,
            words: card_lines(b.lines.iter().map(String::as_str)),
            meta: b.meta,
            why: why.into(),
            his: false,
            edge: None,
            connect: None,
            attach: None,
        });
    }

    // on stage: taped moments that sound like it and are no bit's yet
    for moment in s.like_moments {
        let Some(pid) = sget(moment, "id") else { continue };
        if moment.get("joke").is_some_and(|j| !j.is_null()) {
            continue;
        }
        let room = sget(moment, "venue_short")
            .or_else(|| sget(moment, "set_title"))
            .unwrap_or("On tape");
        let when = sget(moment, "when_label").unwrap_or("");
        fill.push(&Card {
            group: "on_stage",
            kind: "moment",
            r#ref: pid,
            title: if when.is_empty() {
                room.to_string()
            } else {
                format!("{room}, {when}")
            },
            words: card_lines(sget(moment, "text")),
            meta: sget(moment, "at_label").unwrap_or("").to_string(),
            why: "Sounds like this bit, on tape".into(),
            his: false,
            edge: None,
            connect: Some(("performed_as", pid, s.joke)),
            attach: None,
        });
    }

    // on stage: what he said on the newest night that the written version does not have
    if let Some(said) = s.wording {
        let night = set_ref(&said.set);
        fill.push(&Card {
            group: "on_stage",
            kind: "wording",
            r#ref: &night,
            title: if said.label.is_empty() {
                "What you said on stage".into()
            } else {
                format!("What you said, {}", said.label)
            },
            words: card_lines(said.lines.iter().map(String::as_str)),
            meta: said
                .on_script_pct
                .map(|p| format!("{p:.0}% on script"))
                .unwrap_or_default(),
            why: "On tape, not in the written bit".into(),
            his: true,
            edge: None,
            connect: None,
            attach: None,
        });
    }

    let groups: Vec<Value> = GROUPS
        .iter()
        .zip(fill.groups)
        .map(|((key, title), mut items)| {
            items.truncate(s.cap);
            json!({"key": key, "title": title, "items": items})
        })
        .collect();
    json!({"joke": s.joke, "unit": s.unit, "groups": groups, "sets": sets_of(s.boards, s.joke)})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lines(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|x| (*x).to_string()).collect()
    }

    fn items(v: &Value) -> Items {
        v.as_object().cloned().unwrap()
    }

    /// The notebook of the tests: invented bits, by id.
    fn bits() -> HashMap<&'static str, Bit> {
        let b = |title: &str, ls: &[&str], written: bool, meta: &str| Bit {
            title: title.into(),
            lines: lines(ls),
            written,
            meta: meta.into(),
        };
        HashMap::from([
            (
                "joke:doorbell",
                b(
                    "Smart doorbell",
                    &[
                        "My doorbell shows me who is at the door.",
                        "It is always me. Coming home. Looking tired.",
                    ],
                    true,
                    "never on stage",
                ),
            ),
            (
                "joke:roommate",
                b(
                    "Roommate",
                    &["My roommate labels his food.", "He labeled the salt."],
                    true,
                    "killed twice",
                ),
            ),
            (
                "joke:landlord",
                b(
                    "Landlord text",
                    &["My landlord texts like a ransom note."],
                    true,
                    "5 nights",
                ),
            ),
            (
                "joke:gps",
                b(
                    "GPS in the driveway",
                    &["My GPS says you have arrived in my own driveway."],
                    true,
                    "killed 3 times",
                ),
            ),
            (
                "joke:icemaker",
                b(
                    "The ice maker judges me",
                    &["The ice maker judges me. I can hear it."],
                    false,
                    "never on stage",
                ),
            ),
            (
                "joke:camera",
                b(
                    "Fridge camera",
                    &["The fridge has a camera inside."],
                    false,
                    "never on stage",
                ),
            ),
            (
                "joke:oldnote",
                b(
                    "Note from 2023",
                    &["smart fridge = roommate who only tells you bad news"],
                    true,
                    "never on stage",
                ),
            ),
        ])
    }

    fn fridge() -> Vec<String> {
        lines(&[
            "My fridge has wifi now.",
            "It sends me a notification when the door is open.",
            "I know the door is open. I am standing in it.",
            "That is not a smart fridge, that is a roommate.",
        ])
    }

    fn connections() -> Value {
        json!({"groups": [
            {"key": "next", "items": [
                {"id": "joke:landlord", "joke_id": "joke:landlord", "source": "inferred", "basis": ["back_to_back"], "nights": 3}]},
            {"key": "tags", "items": [
                {"id": "unit:t1", "unit_id": "unit:t1", "line": "It labeled the salt.", "joke_id": "joke:roommate",
                 "source": "proposed", "basis": ["reveal_query"]}]},
            {"key": "same_world", "items": [{"id": "joke:gps", "joke_id": "joke:gps", "source": "proposed"}]},
            {"key": "callbacks", "items": [
                {"id": "joke:roommate", "joke_id": "joke:roommate", "edge": "e:cb1", "source": "his",
                 "direction": "this_calls_back", "basis": ["callback_to"]}]},
            {"key": "same_shape", "items": [
                {"id": "joke:gps", "joke_id": "joke:gps", "source": "proposed", "basis": ["labels"],
                 "shared": ["states the obvious", "act-out"]}]},
            {"key": "contrasts", "items": [{"id": "joke:doorbell", "joke_id": "joke:doorbell", "source": "proposed"}]},
        ]})
    }

    struct World {
        lines: Vec<String>,
        conns: Value,
        merge: Value,
        like_bits: Vec<String>,
        moments: Vec<Value>,
        wording: Option<Wording>,
        boards: Items,
        links: Items,
        merges: MergeState,
    }

    impl World {
        fn new() -> Self {
            let attach = json!({"id": "at:1", "kind": "attach", "status": "attached", "joke": "joke:camera",
                                "into": "joke:fridge", "text": "The fridge has a camera inside.\nIt has watched me eat cheese.",
                                "date": "2026-10-30", "source": null});
            let mut merges = MergeState::default();
            merges.attached.insert("joke:camera".into(), attach);
            Self {
                lines: fridge(),
                conns: connections(),
                merge: json!({"joke_id": "joke:fridge",
                              "group": {"members": [{"joke_id": "joke:fridge"}, {"joke_id": "joke:oldnote"}]},
                              "others": [{"joke_id": "joke:icemaker"}, {"joke_id": "joke:gps"}]}),
                like_bits: vec!["joke:doorbell".into(), "joke:camera".into(), "joke:roommate".into()],
                moments: vec![
                    json!({"id": "perf:p9", "joke": null, "venue_short": "Tuesday", "when_label": "Mar 4",
                           "at_label": "1:35", "text": "my fridge has wifi now and it tells me the door is open"}),
                    json!({"id": "perf:p8", "joke": "joke:gps", "text": "you have arrived"}),
                ],
                wording: Some(Wording {
                    set: "20260313_late".into(),
                    label: "Friday, Mar 13".into(),
                    lines: lines(&["I know the door is open. I am IN it."]),
                    on_script_pct: Some(82.0),
                }),
                boards: items(&json!({
                    "sb:1": {"id": "sb:1", "title": "Friday late show", "source": "his",
                             "joke_ids": ["joke:roommate", "joke:fridge", "joke:landlord"]},
                    "sb:2": {"id": "sb:2", "title": "Put away", "source": "his", "archived": true, "joke_ids": ["joke:fridge"]},
                    "sb:3": {"id": "sb:3", "title": "Gone", "source": "his", "deleted": true, "joke_ids": ["joke:fridge"]},
                    "sb:4": {"id": "sb:4", "title": "From your Insights", "source": "inferred", "joke_ids": ["joke:fridge"]},
                    "sb:5": {"id": "sb:5", "title": "Landlords", "source": "his", "joke_ids": ["joke:landlord"]},
                    "sb:6": {"id": "sb:6", "title": "Smart home stuff", "joke_ids": ["joke:fridge", "joke:gps"]},
                })),
                links: Items::new(),
                merges,
            }
        }

        fn build(&self) -> Value {
            let s = Sources {
                joke: "joke:fridge",
                unit: Some("unit:f3"),
                lines: &self.lines,
                connections: Some(&self.conns),
                merge: &self.merge,
                like_bits: &self.like_bits,
                like_moments: &self.moments,
                wording: self.wording.as_ref(),
                boards: &self.boards,
                links: &self.links,
                merges: &self.merges,
                cap: 6,
            };
            let all = bits();
            build(&s, &|j| all.get(j).cloned())
        }
    }

    fn refs(out: &Value, group: &str) -> Vec<String> {
        out["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["key"] == group)
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["ref"].as_str().unwrap().to_string())
            .collect()
    }

    fn item<'a>(out: &'a Value, id: &str) -> &'a Value {
        out["groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["items"].as_array().unwrap())
            .find(|i| i["id"] == id)
            .unwrap_or_else(|| panic!("no item {id} in {out}"))
    }

    #[test]
    fn each_group_is_filled_from_its_own_source() {
        let out = World::new().build();
        assert_eq!(out["joke"], "joke:fridge");
        assert_eq!(out["unit"], "unit:f3");
        let keys: Vec<&str> = out["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["key"].as_str().unwrap())
            .collect();
        assert_eq!(keys, ["same_idea", "callbacks", "on_stage", "loose", "same_shape"]);
        // the judge's group, then what sounds like it and is written
        assert_eq!(refs(&out, "same_idea"), ["joke:oldnote", "joke:doorbell"]);
        // his callback edge, then the line that could tag it
        assert_eq!(refs(&out, "callbacks"), ["joke:roommate", "unit:t1"]);
        // what follows it on stage, the unlinked moment (the one that is another bit's is left out), the wording
        assert_eq!(
            refs(&out, "on_stage"),
            ["joke:landlord", "perf:p9", "set:20260313_late"]
        );
        // the note filed here, then the judge's unwritten neighbour (a written one is not a scrap)
        assert_eq!(refs(&out, "loose"), ["joke:camera", "joke:icemaker"]);
        assert_eq!(refs(&out, "same_shape"), ["joke:gps"]);
    }

    #[test]
    fn a_card_carries_his_words_whose_it_is_and_what_he_can_do() {
        let out = World::new().build();
        let old = item(&out, "same_idea:joke:oldnote");
        assert_eq!(
            *old,
            json!({"id": "same_idea:joke:oldnote", "kind": "bit", "ref": "joke:oldnote", "title": "Note from 2023",
                   "words": ["smart fridge = roommate who only tells you bad news"], "meta": "never on stage",
                   "why": "Might be this same bit", "source": "ai", "edge": null,
                   "line": {"i": 3, "text": "That is not a smart fridge, that is a roommate."},
                   "does": ["merge", "add", "dismiss"]})
        );
        let cb = item(&out, "callbacks:joke:roommate");
        assert_eq!(
            (&cb["source"], &cb["edge"], &cb["does"]),
            (&json!("his"), &json!("e:cb1"), &json!(["add", "dismiss"])),
            "his edge: nothing left to connect"
        );
        assert_eq!(cb["why"], "This bit calls back to it");
        assert!(cb.get("connect").is_none(), "a stored edge is confirmed, not added");
        let tag = item(&out, "callbacks:unit:t1");
        assert_eq!(
            (&tag["kind"], &tag["words"], &tag["title"]),
            (&json!("line"), &json!(["It labeled the salt."]), &json!("Roommate"))
        );
        assert_eq!(
            tag["connect"],
            json!({"kind": "tag_of", "src": "unit:t1", "dst": "joke:fridge"})
        );
        assert_eq!(tag["does"], json!(["connect", "add", "dismiss"]));
        let next = item(&out, "on_stage:joke:landlord");
        assert_eq!(next["why"], "Done right after this on stage, 3 times");
        assert_eq!(next["does"], json!(["connect", "dismiss"]));
        assert_eq!(
            next["connect"],
            json!({"kind": "segue", "src": "joke:fridge", "dst": "joke:landlord"})
        );
        let moment = item(&out, "on_stage:perf:p9");
        assert_eq!(
            (&moment["title"], &moment["meta"], &moment["does"]),
            (
                &json!("Tuesday, Mar 4"),
                &json!("1:35"),
                &json!(["link", "add", "dismiss"])
            )
        );
        assert_eq!(
            moment["connect"],
            json!({"kind": "performed_as", "src": "perf:p9", "dst": "joke:fridge"})
        );
        let said = item(&out, "on_stage:set:20260313_late");
        assert_eq!(
            (&said["kind"], &said["title"], &said["meta"], &said["does"]),
            (
                &json!("wording"),
                &json!("What you said, Friday, Mar 13"),
                &json!("82% on script"),
                &json!(["use", "dismiss"])
            )
        );
        assert_eq!(
            said["line"],
            json!({"i": 2, "text": "I know the door is open. I am standing in it."}),
            "the wording opens on the line it replaces"
        );
        let note = item(&out, "loose:joke:camera");
        assert_eq!(
            (
                &note["kind"],
                &note["attach"],
                &note["title"],
                &note["does"],
                &note["source"]
            ),
            (
                &json!("note"),
                &json!("at:1"),
                &json!("Note, 2026-10-30"),
                &json!(["add", "dismiss"]),
                &json!("ai")
            )
        );
        assert_eq!(
            note["words"],
            json!(["The fridge has a camera inside.", "It has watched me eat cheese."])
        );
        let scrap = item(&out, "loose:joke:icemaker");
        assert_eq!(
            (&scrap["kind"], &scrap["does"]),
            (&json!("idea"), &json!(["attach", "add", "dismiss"]))
        );
        let shape = item(&out, "same_shape:joke:gps");
        assert_eq!(shape["why"], "Same moves: states the obvious, act-out");
        assert_eq!(
            shape["connect"],
            json!({"kind": "same_shape", "src": "joke:fridge", "dst": "joke:gps"})
        );
    }

    /// The red arm of the dismissal: with `Fill::dismissed` answering false, the three assertions below fail.
    #[test]
    fn a_dismissed_pair_is_gone_and_an_undone_one_is_back() {
        let mut w = World::new();
        let dismiss = |a: &str, b: &str, status: &str| {
            json!({"id": crate::reducers::links::pair_id(a, b), "a": a.min(b), "b": a.max(b), "group": "same_idea",
                   "status": status})
        };
        w.links = [
            dismiss("joke:fridge", "joke:doorbell", "dismissed"),
            dismiss("perf:p9", "joke:fridge", "dismissed"),
            dismiss("joke:fridge", "joke:gps", "dismissed"),
            dismiss("joke:fridge", "joke:icemaker", "undone"),
            // another bit's pair says nothing about this one
            dismiss("joke:roommate", "joke:oldnote", "dismissed"),
            // a stored edge is rejected through the decisions, never hidden by a pair
            dismiss("joke:fridge", "joke:roommate", "dismissed"),
        ]
        .into_iter()
        .map(|v| (v["id"].as_str().unwrap().to_string(), v))
        .collect();
        let out = w.build();
        assert_eq!(refs(&out, "same_idea"), ["joke:oldnote"], "the doorbell is gone");
        assert_eq!(
            refs(&out, "on_stage"),
            ["joke:landlord", "set:20260313_late"],
            "the moment is gone"
        );
        assert!(refs(&out, "same_shape").is_empty(), "the GPS is gone");
        assert_eq!(
            refs(&out, "loose"),
            ["joke:camera", "joke:icemaker"],
            "an undone pair is suggested again"
        );
        assert_eq!(
            refs(&out, "callbacks"),
            ["joke:roommate", "unit:t1"],
            "his edge stays until he rejects the edge"
        );
        // "not the same bit" from the merge screen counts too
        let mut w = World::new();
        w.merges
            .dismissed
            .insert(("joke:doorbell".into(), "joke:fridge".into()));
        assert_eq!(refs(&w.build(), "same_idea"), ["joke:oldnote"]);
    }

    /// The red arm of the membership: with the `position` lookup removed (every board listed), the list below grows.
    #[test]
    fn the_sets_are_his_live_lists_that_hold_the_bit() {
        let out = World::new().build();
        assert_eq!(
            out["sets"],
            json!([
                {"board": "sb:1", "title": "Friday late show", "position": 2, "count": 3, "notes": []},
                {"board": "sb:6", "title": "Smart home stuff", "position": 1, "count": 2, "notes": []},
            ]),
            "not the archived one, the deleted one, the app's draft, or a list without it"
        );
        assert_eq!(
            sets_of(&World::new().boards, "joke:nope"),
            Vec::<Value>::new(),
            "a bit in no set"
        );
    }

    #[test]
    fn a_card_is_on_the_page_once_and_never_for_a_bit_he_cannot_see() {
        let mut w = World::new();
        // the roommate is his callback AND sounds like the bit: one card, in Callbacks
        assert_eq!(
            refs(&w.build(), "same_idea"),
            ["joke:oldnote", "joke:doorbell"],
            "the roommate is not repeated under Same idea"
        );
        // a bit the reader cannot see (merged away): no card
        w.like_bits.push("joke:hidden".into());
        w.merge["group"]["members"]
            .as_array_mut()
            .unwrap()
            .push(json!({"joke_id": "joke:hidden"}));
        let out = w.build();
        assert!(!out.to_string().contains("joke:hidden"), "{out}");
        // the bit itself is never around itself
        w.like_bits.push("joke:fridge".into());
        assert!(!refs(&w.build(), "same_idea").contains(&"joke:fridge".to_string()));
    }

    #[test]
    fn the_cap_cuts_each_group_and_long_lines_are_clipped() {
        let w = World::new();
        let s = Sources {
            joke: "joke:fridge",
            unit: None,
            lines: &w.lines,
            connections: Some(&w.conns),
            merge: &w.merge,
            like_bits: &w.like_bits,
            like_moments: &w.moments,
            wording: w.wording.as_ref(),
            boards: &w.boards,
            links: &w.links,
            merges: &w.merges,
            cap: 1,
        };
        let long = "word ".repeat(100);
        let out = build(&s, &|j| {
            bits().get(j).cloned().map(|mut b| {
                b.lines = vec![long.clone(); 20];
                b
            })
        });
        assert_eq!(out["unit"], Value::Null);
        for g in out["groups"].as_array().unwrap() {
            assert!(g["items"].as_array().unwrap().len() <= 1, "{g}");
        }
        let words = item(&out, "same_idea:joke:oldnote")["words"].as_array().unwrap();
        assert_eq!(words.len(), MAX_WORDS);
        let first = words[0].as_str().unwrap();
        assert_eq!(first.chars().count(), MAX_LINE_CHARS, "{first}");
        assert!(first.ends_with('…'));
    }

    #[test]
    fn the_anchor_needs_two_content_words_and_half_of_the_smaller_side() {
        let ls = fridge();
        assert_eq!(
            anchor(&ls, &lines(&["I know the door is open. I am IN it."])),
            Some((2, ls[2].clone()))
        );
        // one shared word is not a tie
        assert_eq!(anchor(&ls, &lines(&["My doorbell has wifi."])), None);
        // common words never tie a card to a line
        assert_eq!(anchor(&ls, &lines(&["that is not what you said about this"])), None);
        assert_eq!(anchor(&ls, &[]), None);
        assert_eq!(anchor(&[], &lines(&["My fridge has wifi now."])), None);
        // a tie goes to the first line
        let twice = lines(&["smart fridge roommate", "smart fridge roommate"]);
        assert_eq!(anchor(&twice, &lines(&["smart fridge roommate"])).map(|x| x.0), Some(0));
        assert_eq!(
            content_words("The Fridge's wifi, the WIFI!"),
            HashSet::from(["fridge".to_string(), "wifi".to_string()])
        );
    }

    #[test]
    fn nothing_known_is_five_empty_groups() {
        let (boards, links, merges) = (Items::new(), Items::new(), MergeState::default());
        let s = Sources {
            joke: "joke:new",
            unit: None,
            lines: &[],
            connections: None,
            merge: &json!({"joke_id": "joke:new", "group": null, "others": []}),
            like_bits: &[],
            like_moments: &[],
            wording: None,
            boards: &boards,
            links: &links,
            merges: &merges,
            cap: 6,
        };
        let out = build(&s, &|_| None);
        assert_eq!(out["groups"].as_array().unwrap().len(), 5);
        assert!(
            out["groups"]
                .as_array()
                .unwrap()
                .iter()
                .all(|g| g["items"] == json!([]))
        );
        assert_eq!(out["sets"], json!([]));
    }
}
