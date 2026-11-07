//! His notebook flattened for Ask (`creative/askchat.py` `sources(c)`, plus the two small maps the golden generator
//! reads beside it): every quotable thing he has, keyed by the node a quote is checked against, then each set's
//! room and each joke's title. [`Notebook::from_corpus`] is the one door.

use indexmap::IndexMap;
use serde_json::{Map, Value};

use super::{Kind, Notebook, Source, Sources};
use crate::corpus::{Corpus, num, s, venue_short};
use crate::notebook::{rating_word, stage_words, texts};
use pk_core::py;

/// `_moment_words(seg)`: how a moment went, in words.
fn moment_words(seg: &Map<String, Value>) -> String {
    match s(seg, "status") {
        Some("banter") => return "Talking to the room".to_string(),
        Some("crowd_work") => return "Crowd work".to_string(),
        _ => {}
    }
    let n = laughs_of(seg);
    let word = rating_word(s(seg, "ai_rating").unwrap_or("")).unwrap_or("On stage");
    if n == 0 {
        word.to_string()
    } else {
        format!("{word} \u{b7} {n} laugh{}", if n == 1 { "" } else { "s" })
    }
}

/// `(seg.get("laughs") or {}).get("count") or 0`.
fn laughs_of(seg: &Map<String, Value>) -> i64 {
    seg.get("laughs")
        .and_then(Value::as_object)
        .and_then(|l| num(l.get("count")))
        .map_or(0, |n| n as i64)
}

/// `x[:10]` on code points.
fn first10(x: &str) -> String {
    x.chars().take(10).collect()
}

impl Notebook {
    /// `sources(c)` over a corpus, with every set's room (`venue_short`) and every joke's title (`Corpus.title`).
    pub fn from_corpus(c: &Corpus) -> Notebook {
        let mut out: Sources = IndexMap::new();
        for jid in &c.jokes {
            let joke = c.joke(jid);
            let (line, _, nights) = stage_words(c, jid);
            let title = super::short_title(joke.and_then(|j| s(j, "title")));
            let title = if title.is_empty() {
                "Untitled".to_string()
            } else {
                title
            };
            for (node, text, date) in texts(c, jid) {
                let n = c.nodes.get(&node);
                let kind = if node == *jid {
                    Kind::Idea
                } else if n.is_some_and(|n| {
                    s(n, "type") == Some("unit") && s(n, "source").filter(|x| !x.is_empty()).unwrap_or("gdoc") != "gdoc"
                }) {
                    Kind::Note
                } else {
                    Kind::Bit
                };
                // the gate reads `c.text_of(node) or text`: set only when it is another string
                let gate_text = c.text_of(&node, None).filter(|g| !g.is_empty() && *g != text);
                let date = date.as_deref().map(first10).filter(|d| !d.is_empty());
                out.insert(
                    node.clone(),
                    Source {
                        id: node,
                        kind,
                        text,
                        title: title.clone(),
                        date,
                        joke_id: Some(jid.clone()),
                        stage_line: line.clone(),
                        nights: Some(i64::try_from(nights).unwrap_or(i64::MAX)),
                        set_id: None,
                        start_ms: None,
                        venue: None,
                        rating: None,
                        status: None,
                        words: None,
                        laughs: None,
                        gate_text,
                    },
                );
            }
        }
        for (key, set) in &c.sets {
            let Some(segs) = c.segments.get(key) else { continue };
            let meta = c.metas.get(key).and_then(Value::as_object);
            let date = meta
                .and_then(|m| s(m, "show_date"))
                .filter(|d| !d.is_empty())
                .or(Some(set.date.as_str()).filter(|d| !d.is_empty()))
                .map_or_else(|| first10(key), str::to_string);
            let room = venue_short(set.venue.as_str(), None);
            for seg in segs {
                let raw = s(seg, "transcript").unwrap_or("");
                let t = py::strip(raw);
                if t.is_empty() {
                    continue;
                }
                let status = s(seg, "status");
                let known = matches!(status, Some("known" | "new"));
                let jid = if known {
                    s(seg, "joke").filter(|j| !j.is_empty())
                } else {
                    None
                };
                let in_book = jid.filter(|j| c.is_joke(j));
                let title = match in_book {
                    Some(j) => super::short_title(Some(&c.title(j))),
                    None => seg
                        .get("title")
                        .filter(|t| py::truthy(Some(t)))
                        .map_or_else(|| "On stage".to_string(), py::str),
                };
                let Some(seg_id) = s(seg, "seg_id") else { continue };
                let gate_text = c.text_of(seg_id, None).filter(|g| !g.is_empty() && g != t);
                out.insert(
                    seg_id.to_string(),
                    Source {
                        id: seg_id.to_string(),
                        kind: Kind::Moment,
                        text: t.to_string(),
                        title,
                        date: Some(first10(&date)),
                        joke_id: in_book.map(str::to_string),
                        stage_line: None,
                        nights: None,
                        set_id: Some(format!("set:{key}")),
                        start_ms: num(seg.get("start_ms")).map(|n| n as i64),
                        venue: Some(room.clone()),
                        rating: if known {
                            s(seg, "ai_rating").map(str::to_string)
                        } else {
                            None
                        },
                        status: status.map(str::to_string),
                        words: Some(moment_words(seg)),
                        laughs: Some(laughs_of(seg)),
                        gate_text,
                    },
                );
            }
        }
        let rooms = c
            .sets
            .iter()
            .map(|(k, set)| (k.clone(), venue_short(set.venue.as_str(), None)))
            .collect();
        let titles = c.jokes.iter().map(|j| (j.clone(), c.title(j))).collect();
        Notebook::new(out, rooms, titles)
    }
}
