//! "From your notebook" (`creative/notebook.py`): his bits about an idea in his own words. `texts` (his written
//! versions, lead first), `best_line` (the sentence to show, always verbatim), `stage_words` (a night on tape in
//! words), and `related`: his-words BM25 over titles, summaries and written versions, fused by rank, the bits that
//! worked on stage first. The formula is the Python's, written out, so the same bits come back in the same order.
//! Lengths and positions are code points, as in the Python.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::BuildHasher;
use std::sync::LazyLock;

use indexmap::IndexMap;
use pk_core::py;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::ask::words::{month_year, py_word_count, short_title, tokens};
use crate::corpus::{Corpus, rating_order, s, venue_short};

/// `QUOTE_MAX`: the longest quote shown.
pub const QUOTE_MAX: usize = 200;
static SENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^.!?\n]+[.!?]*").expect("a valid regex"));

/// `RATING_WORD`.
pub fn rating_word(r: &str) -> Option<&'static str> {
    match r {
        "killed" => Some("Killed"),
        "worked" => Some("Worked"),
        "meh" => Some("Got some laughs"),
        "died" => Some("Went quiet"),
        _ => None,
    }
}

/// One of his written versions: (node, text, date).
pub type Text = (String, String, Option<String>);

/// `_texts(c, jid)`: each of his written versions, lead first then newest; his own idea's summary when it has none.
pub fn texts(c: &Corpus, jid: &str) -> Vec<Text> {
    let lead = c.lead_unit(jid);
    let date_of = |u: &str| c.nodes.get(u).and_then(|n| s(n, "date")).unwrap_or("").to_string();
    let mut rest: Vec<&String> = c
        .joke_units
        .get(jid)
        .map(|us| us.iter().filter(|u| Some(*u) != lead.as_ref()).collect())
        .unwrap_or_default();
    rest.sort_by(|a, b| (date_of(b), *b).cmp(&(date_of(a), *a)));
    let units: Vec<String> = lead.iter().cloned().chain(rest.into_iter().cloned()).collect();
    let mut out: Vec<Text> = Vec::new();
    for u in units {
        let Some(n) = c.nodes.get(&u) else { continue };
        if let Some(t) = n.get("text").filter(|v| py::truthy(Some(v))) {
            out.push((
                u.clone(),
                py::str(t),
                n.get("date").and_then(Value::as_str).map(str::to_string),
            ));
        }
    }
    if out.is_empty()
        && c.his_nodes.contains(jid)
        && let Some(j) = c.joke(jid)
        && let Some(summary) = j.get("summary").filter(|v| py::truthy(Some(v)))
    {
        let date = ["date", "first_seen", "ts"]
            .iter()
            .find_map(|k| j.get(*k).filter(|v| py::truthy(Some(v))).map(py::str))
            .unwrap_or_default();
        let date: String = date.chars().take(10).collect();
        out.push((jid.to_string(), py::str(summary), (!date.is_empty()).then_some(date)));
    }
    out
}

/// `_sentences(text)`: its sentences of three words or more, stripped.
pub fn sentences(text: &str) -> impl Iterator<Item = &str> {
    SENT.find_iter(text)
        .map(|m| py::strip(m.as_str()))
        .filter(|s| py_word_count(s) >= 3)
}

/// `_clip(sentence, terms)`: a long sentence cut to `QUOTE_MAX` around its first matching word, at word
/// boundaries; (quote, cut at the start, cut at the end).
pub fn clip<S: BuildHasher>(sentence: &str, terms: &HashSet<String, S>) -> (String, bool, bool) {
    let chars: Vec<char> = sentence.chars().collect();
    if chars.len() <= QUOTE_MAX {
        return (sentence.to_string(), false, false);
    }
    let low: Vec<char> = sentence.to_lowercase().chars().collect();
    let find = |hay: &[char], needle: &str, from: usize| -> Option<usize> {
        let n: Vec<char> = needle.chars().collect();
        (from..=hay.len().saturating_sub(n.len())).find(|&i| hay.get(i..i + n.len()) == Some(&n[..]))
    };
    let at = terms.iter().filter_map(|t| find(&low, t, 0)).min().unwrap_or(0);
    let mut start = at.saturating_sub(60);
    if start != 0
        && let Some(sp) = find(&chars, " ", start)
        && sp < at
    {
        start = sp + 1;
    }
    let mut end = chars.len().min(start + QUOTE_MAX);
    if end < chars.len()
        && let Some(sp) = (start..end).rev().find(|&i| chars[i] == ' ')
        && sp > start
    {
        end = sp;
    }
    // `at` is a position in the lower-cased text (which can be longer: 'İ' lowers to two characters), applied to
    // the original as the Python does; an out-of-range slice is empty there, never a panic
    let q: String = chars.get(start..end).map_or_else(String::new, |s| s.iter().collect());
    (py::strip(&q).to_string(), start > 0, end < chars.len())
}

/// The line shown for a bit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Line {
    /// The words, verbatim.
    pub quote: String,
    /// The node they are in.
    pub quote_node: String,
    /// Its date.
    pub date: Option<String>,
    /// Cut at the start.
    pub cut_start: bool,
    /// Cut at the end.
    pub cut_end: bool,
    /// `doc` or `stage`.
    pub from: &'static str,
}

/// `best_line(c, jid, terms)`: the sentence sharing most of the idea's words, else the reveal, else the punch line
/// from his best night, else his first full sentence.
pub fn best_line<S: BuildHasher>(c: &Corpus, jid: &str, terms: &HashSet<String, S>) -> Option<Line> {
    let reveal = c
        .joke(jid)
        .and_then(|j| j.get("anatomy"))
        .and_then(|a| a.get("reveal"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let lead = c.lead_unit(jid);
    let mut best: Option<(String, String, Option<String>)> = None;
    let mut best_sc = -1.0_f64;
    for (node, text, date) in texts(c, jid) {
        for (k, sent) in sentences(&text).enumerate() {
            let ws: HashSet<String> = tokens(sent).into_iter().collect();
            let mut sc = ws.iter().filter(|w| terms.contains(*w)).count() as f64 * 2.0;
            if !reveal.is_empty() && sent.to_lowercase().contains(&reveal.to_lowercase()) {
                sc += 1.5;
            }
            sc -= 0.01 * k as f64;
            if lead.as_deref().unwrap_or(&node) == node {
                sc += 0.1;
            }
            if sc > best_sc {
                best = Some((node.clone(), sent.to_string(), date.clone()));
                best_sc = sc;
            }
        }
    }
    let line = |node: String, sent: &str, date: Option<String>, from| {
        let (quote, cut_start, cut_end) = clip(sent, terms);
        Line {
            quote,
            quote_node: node,
            date,
            cut_start,
            cut_end,
            from,
        }
    };
    let taped = c.perfs.get(jid).is_some_and(|p| !p.is_empty());
    if let Some((node, sent, date)) = &best
        && (best_sc >= 1.0 || !taped)
    {
        return Some(line(node.clone(), sent, date.clone(), "doc"));
    }
    let mut ps: Vec<&crate::corpus::Perf> = c.perfs.get(jid).map(|p| p.iter().collect()).unwrap_or_default();
    ps.sort_by(|a, b| (rating_order(&a.ai_rating), &a.date).cmp(&(rating_order(&b.ai_rating), &b.date)));
    for p in ps.iter().rev() {
        for pl in p.punchlines.iter().rev() {
            let t = py::strip(pl.get("text").and_then(Value::as_str).unwrap_or(""));
            let seg = pl.get("seg_id").and_then(Value::as_str).unwrap_or("");
            if !t.is_empty() && !seg.is_empty() && c.text_of(seg, None).unwrap_or_default().contains(t) {
                return Some(line(seg.to_string(), t, Some(p.date.clone()), "stage"));
            }
        }
    }
    best.map(|(node, sent, date)| line(node, &sent, date, "doc"))
}

/// `stage_words(c, jid)`: (one line, proven, nights); `None` when never on tape.
pub fn stage_words(c: &Corpus, jid: &str) -> (Option<String>, bool, usize) {
    let Some(ps) = c.perfs.get(jid).filter(|p| !p.is_empty()) else {
        return (None, false, 0);
    };
    let key = |p: &crate::corpus::Perf| (rating_order(&p.ai_rating), p.vs_room.unwrap_or(0.0), p.date.clone());
    let mut best = &ps[0];
    for p in &ps[1..] {
        let (a, b) = (key(p), key(best));
        #[allow(clippy::float_cmp)] // Python's tuple comparison: exact
        let later_wins = a.0 > b.0 || (a.0 == b.0 && (a.1 > b.1 || (a.1 == b.1 && a.2 > b.2)));
        if later_wins {
            best = p;
        }
    }
    let name = venue_short(best.venue.as_str(), None);
    let name = py::strip(&name);
    let where_ = if name.is_empty() || name.starts_with("Unknown") {
        "on stage".to_string()
    } else {
        format!("at {name}")
    };
    let word = rating_word(if best.ai_rating.is_empty() {
        "died"
    } else {
        &best.ai_rating
    })
    .unwrap_or("On stage");
    let mut line = format!("{word} {where_}, {}", month_year(Some(&best.date)));
    if ps.len() > 1 {
        let _ = write!(line, " · {} nights", ps.len());
    }
    let proven = ps.iter().any(|p| p.ai_rating == "killed" || p.ai_rating == "worked");
    (Some(line), proven, ps.len())
}

/// The his-words index (`_index`): term counts per joke (title words twice), title words, document frequency,
/// the mean document length. Built once per corpus.
#[derive(Debug, Clone)]
pub struct Index {
    docs: IndexMap<String, IndexMap<String, usize>>,
    title_terms: HashMap<String, HashSet<String>>,
    df: HashMap<String, usize>,
    avg: f64,
}

impl Index {
    /// Index the corpus.
    pub fn new(c: &Corpus) -> Self {
        let (mut docs, mut title_terms, mut df) = (IndexMap::new(), HashMap::new(), HashMap::new());
        for j in &c.jokes {
            let n = &c.nodes[j];
            let t = tokens(s(n, "title").unwrap_or(""));
            let mut parts: Vec<String> = vec![s(n, "summary").unwrap_or("").to_string()];
            parts.extend(texts(c, j).into_iter().map(|x| x.1));
            let mut tf: IndexMap<String, usize> = IndexMap::new();
            for w in t.iter().chain(t.iter()).cloned().chain(tokens(&parts.join(" "))) {
                *tf.entry(w).or_default() += 1;
            }
            for w in tf.keys() {
                *df.entry(w.clone()).or_default() += 1;
            }
            title_terms.insert(j.clone(), t.into_iter().collect());
            docs.insert(j.clone(), tf);
        }
        let total: usize = docs.values().map(|tf| tf.values().sum::<usize>()).sum();
        let avg = total as f64 / docs.len().max(1) as f64;
        Self {
            docs,
            title_terms,
            df,
            avg: if avg == 0.0 { 1.0 } else { avg },
        }
    }

    /// `lexical(c, text)`: BM25 over his titles, summaries and written versions, best first. The first line of a
    /// several-line idea is its premise (its words count double); a word on more than a quarter of the jokes is
    /// skipped.
    pub fn lexical(&self, text: &str) -> Vec<(String, f64)> {
        let n = self.docs.len().max(1);
        let lines: Vec<&str> = py_splitlines(text)
            .into_iter()
            .filter(|x| !py::strip(x).is_empty())
            .collect();
        let head: HashSet<String> = if lines.len() > 1 {
            tokens(lines[0]).into_iter().collect()
        } else {
            HashSet::new()
        };
        let cap = 3.max(n / 4);
        let mut q: Vec<String> = Vec::new();
        for w in tokens(text) {
            let d = self.df.get(&w).copied().unwrap_or(0);
            if d > 0 && d <= cap && !q.contains(&w) {
                q.push(w);
            }
        }
        if q.is_empty() {
            return Vec::new();
        }
        let weight = |w: &str| if head.is_empty() || head.contains(w) { 1.0 } else { 0.5 };
        let nf = n as f64;
        let mut out = Vec::new();
        for (j, tf) in &self.docs {
            let dl = tf.values().sum::<usize>().max(1) as f64;
            let mut sc = 0.0;
            for w in &q {
                let f = tf.get(w).copied().unwrap_or(0);
                if f == 0 {
                    continue;
                }
                let d = self.df[w] as f64;
                let idf = (1.0 + (nf - d + 0.5) / (d + 0.5)).ln();
                let f = f as f64;
                sc += weight(w) * idf * f * 2.2 / (f + 1.2 * (0.25 + 0.75 * dl / self.avg));
                if self.title_terms[j].contains(w) {
                    sc += weight(w) * 0.5 * idf;
                }
            }
            if sc > 0.0 {
                out.push((j.clone(), sc));
            }
        }
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    /// `_related_why`: up to three of the idea's words the bit shares.
    pub fn why(&self, jid: &str, terms: &HashSet<String>) -> String {
        let mut shared: Vec<&String> = terms
            .iter()
            .filter(|w| self.docs.get(jid).is_some_and(|tf| tf.get(*w).is_some_and(|f| *f > 0)))
            .collect();
        shared.sort();
        if shared.is_empty() {
            "Close to the idea".to_string()
        } else {
            let ws: Vec<&str> = shared.iter().take(3).map(|w| w.as_str()).collect();
            format!("Shares: {}", ws.join(", "))
        }
    }
}

/// `str.splitlines()`: Python's line boundaries.
pub fn py_splitlines(text: &str) -> Vec<&str> {
    pk_core::py::splitlines(text)
}

/// A bit as `related` lists it (the card fields the writing kinds read).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Related {
    /// The bit.
    pub joke_id: String,
    /// Its short title.
    pub title: String,
    /// His line.
    pub quote: Option<String>,
    /// Where the line is.
    pub quote_node: Option<String>,
    /// One line of stage history.
    pub stage_line: Option<String>,
    /// It worked on stage.
    pub proven: bool,
    /// Nights on tape.
    pub nights: usize,
    /// Its rank before the proven-first reorder.
    pub rank: usize,
    /// Why it came up.
    pub why: String,
}

/// `related(c, text, cap)` without the database's hybrid leg: his bits about an idea, the proven among the first
/// five first.
pub fn related(c: &Corpus, ix: &Index, text: &str, cap: usize) -> Vec<Related> {
    let terms: HashSet<String> = tokens(text).into_iter().collect();
    let mut fused: IndexMap<String, f64> = IndexMap::new();
    for (rank, (j, _)) in ix.lexical(text).into_iter().take(60).enumerate() {
        *fused.entry(j).or_default() += 1.0 / (60.0 + rank as f64);
    }
    let mut ranked: Vec<(String, f64)> = fused.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut items: Vec<Related> = ranked
        .into_iter()
        .take(cap)
        .enumerate()
        .map(|(rank, (j, _))| {
            let line = best_line(c, &j, &terms);
            let (stage_line, proven, nights) = stage_words(c, &j);
            Related {
                title: short_title(c.joke(&j).and_then(|n| s(n, "title"))),
                quote: line.as_ref().map(|l| l.quote.clone()),
                quote_node: line.map(|l| l.quote_node),
                stage_line,
                proven,
                nights,
                rank,
                why: ix.why(&j, &terms),
                joke_id: j,
            }
        })
        .collect();
    items.sort_by_key(|it| (usize::from(!(it.proven && it.rank < 5)), it.rank));
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitlines_is_pythons() {
        assert_eq!(py_splitlines("a\r\nb\rc\n\nd"), ["a", "b", "c", "", "d"]);
        assert_eq!(py_splitlines("a\n"), ["a"]);
        assert!(py_splitlines("").is_empty());
    }

    #[test]
    fn a_long_sentence_is_clipped_on_word_boundaries() {
        // the lower-cased sentence is longer than the sentence: Python slices past the end and gets ""
        let s = format!("{}{}needle.", "\u{130}".repeat(100), " x".repeat(60));
        let terms: HashSet<String> = ["needle".to_string()].into_iter().collect();
        assert_eq!(clip(&s, &terms), (String::new(), true, false));
        let s = format!("{} needle {}", "word ".repeat(40), "tail ".repeat(40));
        let terms: HashSet<String> = ["needle".to_string()].into_iter().collect();
        let (q, a, b) = clip(&s, &terms);
        assert!(a && b && q.contains("needle") && q.chars().count() <= QUOTE_MAX);
        assert!(s.contains(&q));
    }
}
