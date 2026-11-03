//! His sets (`creative/mysets.py`): many sets kept at once, each opened and worked on. A set is a working set (the
//! `workingsets` store); what this adds is computed from his tapes at read time: what's working per bit
//! ([`bit_label`]), the tough spots with their actions (swap, tag, bridge, move, cut, add), "make a 5 from my 10"
//! ([`trim`]), the recording a set lines up with, and the set screen ([`view`]) and its list row ([`summary`]).
//!
//! Words only in every `words` / `label_words` / `status_words` / `why` field, as the Python writes them.

use std::collections::HashSet;
use std::fmt::Write as _;

use indexmap::IndexMap;
use pk_core::py;
use serde_json::{Map, Value, json};

use crate::ask::words::{mmss, short_title};
use crate::connections::first_tag;
use crate::corpus::{Corpus, Perf, median, num, round_to, s};
use crate::display::{UNKNOWN_VENUE, venue_label};
use crate::setlist::{
    Obj, Transitions, best_next, card, int_or_zero, length_status_words, plain_best_next, target_band, transition_words,
};
use crate::story::Story;

/// `DEAD_ROOM`: a set whose whole room laughed under this (laugh-s/min) barely laughed: its nights don't count.
const DEAD_ROOM: f64 = 2.0;
/// `KILL_VR`, `GOOD_VR`, `SOME_VR`: vs the room's own laugh rate that night.
const KILL_VR: f64 = 1.2;
const GOOD_VR: f64 = 0.8;
const SOME_VR: f64 = 0.5;
/// `LONG_QUIET_S`, `LONG_QUIET_SHARE`: a stretch with no laugh this long and this share of the bit.
const LONG_QUIET_S: f64 = 40.0;
const LONG_QUIET_SHARE: f64 = 0.4;
/// `RAN_LONG`: last night ran this many times its usual length (and 30 s over).
const RAN_LONG: f64 = 1.5;
/// `MATCH_MIN`: a recording lines up with a set at this score or better (and 2+ shared bits).
const MATCH_MIN: f64 = 0.3;
/// `LENGTHS`: the set lengths the app offers.
pub const LENGTHS: [i64; 4] = [300, 420, 600, 900];

/// `night_outcome(p)`: one night of one bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Killed.
    Killed,
    /// Worked.
    Good,
    /// Some laughs.
    Some,
    /// Quiet.
    Quiet,
}

impl Outcome {
    fn word(self) -> &'static str {
        match self {
            Self::Killed => "Killed",
            Self::Good => "Worked",
            Self::Some => "Some laughs",
            Self::Quiet => "Quiet",
        }
    }

    fn rank(self) -> i64 {
        match self {
            Self::Killed => 3,
            Self::Good => 2,
            Self::Some => 1,
            Self::Quiet => 0,
        }
    }

    fn hit(self) -> bool {
        matches!(self, Self::Killed | Self::Good)
    }
}

/// `night_outcome(p)`, or `None` when the whole room barely laughed (not counted).
pub fn night_outcome(p: &Perf) -> Option<Outcome> {
    if p.set_laugh_s_per_min.is_some_and(|r| r < DEAD_ROOM) {
        return None;
    }
    let r = p.ai_rating.as_str();
    let Some(vr) = p.vs_room else {
        return Some(match r {
            "killed" => Outcome::Killed,
            "worked" => Outcome::Good,
            "meh" => Outcome::Some,
            _ => Outcome::Quiet,
        });
    };
    Some(if r == "died" || vr <= 0.0 {
        Outcome::Quiet
    } else if vr >= KILL_VR || (r == "killed" && vr >= 1.0) {
        Outcome::Killed
    } else if vr >= GOOD_VR {
        Outcome::Good
    } else if vr >= SOME_VR {
        Outcome::Some
    } else {
        Outcome::Quiet
    })
}

/// `_where(p)`: "at Mabel's '24" / "on stage '24".
fn where_(venue: Option<&str>, date: Option<&str>) -> String {
    let (name, _) = venue_label(venue);
    let yy = match date.filter(|d| !d.is_empty()) {
        Some(d) => format!(" '{}", d.chars().skip(2).take(2).collect::<String>()),
        None => String::new(),
    };
    let at = if name == UNKNOWN_VENUE {
        "on stage".to_string()
    } else {
        format!("at {name}")
    };
    format!("{at}{yy}")
}

fn where_p(p: &Perf) -> String {
    where_(p.venue.as_str(), Some(&p.date))
}

fn perfs<'c>(c: &'c Corpus, jid: &str) -> &'c [Perf] {
    c.perfs.get(jid).map_or(&[][..], Vec::as_slice)
}

/// What's working for one bit (`bit_label`).
#[derive(Debug, Clone, PartialEq)]
pub struct BitLabel {
    /// killed / solid / mixed / quiet / untested.
    pub label: &'static str,
    /// In words.
    pub label_words: String,
    /// The last counted night in words, if any.
    pub last_words: Option<String>,
    /// Nights counted (not in a room that barely laughed).
    pub counted: usize,
}

fn of(k: usize, n: usize) -> String {
    if k == n {
        match n {
            2 => "both nights".into(),
            n if n > 2 => "every night".into(),
            _ => String::new(),
        }
    } else {
        format!("{k} of {n} nights")
    }
}

/// The `max` / `min` key over a counted night: its outcome, `vs_room` clipped at 3, the date.
fn outcome_key<'a>(x: &(&'a Perf, Outcome)) -> (i64, f64, &'a str) {
    (x.1.rank(), x.0.vs_room.unwrap_or(0.0).min(3.0), x.0.date.as_str())
}

/// `bit_label(c, jid)`.
pub fn bit_label(c: &Corpus, jid: &str) -> BitLabel {
    let ps = perfs(c, jid);
    let counted: Vec<(&Perf, Outcome)> = ps.iter().filter_map(|p| night_outcome(p).map(|o| (p, o))).collect();
    let n = counted.len();
    if n == 0 {
        let words = if ps.is_empty() {
            "Not on stage yet"
        } else {
            "Only on tape in a room that barely laughed"
        };
        return BitLabel {
            label: "untested",
            label_words: words.into(),
            last_words: None,
            counted: 0,
        };
    }
    let count = |o: Outcome| counted.iter().filter(|x| x.1 == o).count();
    let k = count(Outcome::Killed);
    let hits = k + count(Outcome::Good);
    let q = count(Outcome::Quiet);
    let best = counted
        .iter()
        .fold(&counted[0], |b, x| if outcome_key(x) > outcome_key(b) { x } else { b });
    let worst = counted
        .iter()
        .fold(&counted[0], |b, x| if outcome_key(x) < outcome_key(b) { x } else { b });
    let (label, mut words) = if q == 0 && k > 0 && 2 * k >= n && 3 * hits >= 2 * n {
        (
            "killed",
            if n == 1 {
                format!("Killed {}", where_p(best.0))
            } else {
                format!("Killed {}", of(k, n))
            },
        )
    } else if q == 0 && 3 * hits >= 2 * n {
        (
            "solid",
            if n == 1 {
                format!("Worked {}", where_p(best.0))
            } else {
                format!(
                    "Worked {}{}",
                    of(hits, n),
                    if k > 0 { format!(", killed {k}") } else { String::new() }
                )
            },
        )
    } else if hits == 0 || 3 * q >= 2 * n {
        (
            "quiet",
            if q == 0 {
                format!(
                    "Some laughs, never a big one{}",
                    if n > 1 {
                        format!(" · {n} nights")
                    } else {
                        String::new()
                    }
                )
            } else if n == 1 {
                format!("Quiet {}", where_p(worst.0))
            } else {
                format!("Quiet {}", of(q, n))
            },
        )
    } else {
        (
            "mixed",
            format!(
                "Mixed: {} {}, {} {}",
                best.1.word().to_lowercase(),
                where_p(best.0),
                worst.1.word().to_lowercase(),
                where_p(worst.0)
            ),
        )
    };
    if n > 1 && label != "mixed" && !words.contains("nights") {
        let _ = write!(words, " · {n} nights");
    }
    let (lp, lo) = counted[n - 1];
    BitLabel {
        label,
        label_words: words,
        last_words: Some(format!("Last time: {} {}", lo.word().to_lowercase(), where_p(lp))),
        counted: n,
    }
}

fn label_order(l: &str) -> i64 {
    match l {
        "killed" => 4,
        "solid" => 3,
        "mixed" => 2,
        "untested" => 1,
        _ => 0,
    }
}

/// `strength(c, jid)`: (label order, energy or 0, nights counted), compared as a tuple.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Strength(i64, f64, usize);

/// `strength(c, jid, lab)`.
pub fn strength(c: &Corpus, jid: &str, lab: &BitLabel) -> Strength {
    Strength(label_order(lab.label), c.energy(jid).0.unwrap_or(0.0), lab.counted)
}

/// `workingset.current(ws)`: the newest version.
pub fn current(ws: &Obj) -> Option<&Obj> {
    ws.get("versions")?.as_array()?.last()?.as_object()
}

/// `workingset.bit_locked(ws, j)`.
pub fn bit_locked(ws: &Obj, j: &str) -> bool {
    ws.get("locks")
        .and_then(|l| l.get(j))
        .is_some_and(|l| py::truthy(l.get("bit")))
}

/// `ids_of(ws)`: the bits of the current version.
pub fn ids_of(ws: &Obj) -> Vec<&str> {
    current(ws)
        .and_then(|v| v.get("bits"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|b| b.get("joke_id").and_then(Value::as_str).unwrap_or(""))
        .collect()
}

/// `runtime(c, ids)`: the run time (a plain `+=`, rounded to 0.1 s) and how many bits are estimates.
pub fn runtime(c: &Corpus, ids: &[&str]) -> (f64, usize) {
    let (mut total, mut est) = (0.0, 0);
    for j in ids {
        total += c.duration(j).0;
        est += usize::from(perfs(c, j).is_empty());
    }
    (round_to(total, 1), est)
}

/// `length_status(total, target_s)`: `(status, words)`.
pub fn length_status(total: f64, target_s: i64) -> (&'static str, String) {
    if target_s == 0 {
        return ("none", mmss(total));
    }
    let (lo, hi) = target_band(target_s);
    length_status_words(total, lo, hi, "short")
}

/// `length_words(target_s)`: "5 min", "7.5 min".
pub fn length_words(target_s: f64) -> String {
    let m = target_s / 60.0;
    #[expect(clippy::float_cmp, reason = "the Python's `m == int(m)`, exactly")]
    let whole = m == m.trunc();
    if whole {
        format!("{} min", m as i64)
    } else {
        format!("{m:.1} min")
    }
}

/// `TSCORE[_tkind(t)]`: how well one bit leads into the next.
fn tscore(t: &Value) -> i64 {
    let label = t.get("label").and_then(Value::as_str).unwrap_or("");
    if label == "jump" {
        return 0;
    }
    if transition_words(t).1 {
        return 1;
    }
    match label {
        "proven" => 3,
        "link" => 2,
        _ => 0,
    }
}

fn score(tr: &Transitions<'_, '_>, a: Option<&str>, b: Option<&str>) -> i64 {
    match (a, b) {
        (Some(a), Some(b)) => tscore(&tr.get(a, b)),
        _ => 2,
    }
}

fn with_label(c: &Corpus, jid: &str, extra: Vec<(&'static str, Value)>) -> Value {
    let lab = bit_label(c, jid);
    let mut fields = vec![("label", json!(lab.label)), ("label_words", json!(lab.label_words))];
    fields.extend(extra);
    card(c, jid, &HashSet::new(), fields)
}

/// A swap candidate: on the same subject, its strength, minus the length difference, the bit, the shared words, its label.
type SwapCand<'a> = (bool, Strength, f64, &'a String, Vec<String>, BitLabel);

/// `swap_options(c, ids, jid, cap=2)`: stronger bits from his notebook that could take this bit's place.
fn swap_options(story: &Story<'_>, c: &Corpus, ids: &[&str], jid: &str) -> Vec<Value> {
    let i = ids.iter().position(|x| *x == jid).unwrap_or(0);
    let near: Vec<&str> = [i.checked_sub(1).map(|k| ids[k]), Some(jid), ids.get(i + 1).copied()]
        .into_iter()
        .flatten()
        .filter(|x| !x.is_empty())
        .collect();
    let d0 = c.duration(jid).0;
    let mine = strength(c, jid, &bit_label(c, jid));
    let mut out: Vec<SwapCand<'_>> = Vec::new();
    for o in c.perfs.keys() {
        if ids.contains(&o.as_str()) || !c.is_joke(o) || c.stage(o).0.as_str() == Some("retired") {
            continue;
        }
        let lab = bit_label(c, o);
        if !matches!(lab.label, "killed" | "solid") {
            continue;
        }
        let st = strength(c, o, &lab);
        if st <= mine {
            continue;
        }
        let d = c.duration(o).0;
        if !(0.4 * d0 <= d && d <= 2.0 * d0 + 30.0) {
            continue;
        }
        let (sc, words, _) = story.same_subject(&near, o);
        out.push((sc >= 0.35, st, -(d - d0).abs(), o, words, lab));
    }
    out.sort_by(|a, b| {
        (!a.0)
            .cmp(&!b.0)
            .then(b.1.0.cmp(&a.1.0))
            .then(b.1.1.total_cmp(&a.1.1))
            .then(b.2.total_cmp(&a.2))
            .then_with(|| a.3.cmp(b.3))
    });
    out.into_iter()
        .take(2)
        .map(|(subj, _, _, o, words, lab)| {
            let tail = if subj && !words.is_empty() {
                format!(
                    "; same subject ({})",
                    words.iter().take(2).cloned().collect::<Vec<_>>().join(", ")
                )
            } else {
                String::new()
            };
            let why = format!("{}{tail}", lab.label_words);
            json!({"kind": "swap", "joke_id": jid, "with": with_label(c, o, vec![("why", json!(why))]),
                   "words": format!("Swap in {}", short_title(Some(&c.title(o))))})
        })
        .collect()
}

/// `tag_option(c, jid)`: a line he already wrote that could tag this bit (the joke map's "Could tag it").
fn tag_option(c: &Corpus, jid: &str) -> Option<Value> {
    let it = first_tag(c, jid)?;
    let title = it.get("title").map(py::str).unwrap_or_default();
    Some(json!({"kind": "tag", "joke_id": jid, "tag": it, "words": format!("Tag it with {title}")}))
}

/// `move_option(c, ids, jid, tr)`: the spot where this bit links best to what's around it, when it is better than
/// where it is now.
fn move_option(c: &Corpus, ids: &[&str], jid: &str, tr: &Transitions<'_, '_>) -> Option<Value> {
    let i = ids.iter().position(|x| *x == jid)?;
    let rest: Vec<&str> = ids.iter().copied().filter(|x| *x != jid).collect();
    let at = |seq: &[&str], k: usize| -> i64 {
        let prev = k.checked_sub(1).map(|p| seq[p]);
        score(tr, prev, Some(seq[k])) + score(tr, Some(seq[k]), seq.get(k + 1).copied())
    };
    let now = at(ids, i)
        - if i > 0 && i + 1 < ids.len() {
            score(tr, Some(ids[i - 1]), Some(ids[i + 1]))
        } else {
            0
        };
    let (lo, hi) = if rest.len() >= 2 {
        (1, rest.len())
    } else {
        (0, rest.len() + 1)
    };
    let mut best: Option<(i64, usize, Vec<&str>)> = None;
    for k in lo..hi {
        if k == i {
            continue;
        }
        let mut seq = rest[..k].to_vec();
        seq.push(jid);
        seq.extend_from_slice(&rest[k..]);
        let gain = at(&seq, k)
            - if k > 0 && k < rest.len() {
                score(tr, Some(rest[k - 1]), Some(rest[k]))
            } else {
                0
            };
        if best.as_ref().is_none_or(|b| gain > b.0) {
            best = Some((gain, k, seq));
        }
    }
    let (gain, k, seq) = best?;
    if gain <= now {
        return None;
    }
    let prev = k.checked_sub(1).map(|p| seq[p]);
    let tw = prev.map(|p| transition_words(&tr.get(p, jid)).0);
    let head = prev.map_or_else(
        || "Open with it".to_string(),
        |p| format!("Move it after {}", short_title(Some(&c.title(p)))),
    );
    let tail = tw.map_or(String::new(), |w| format!(" ({})", w.to_lowercase()));
    Some(json!({"kind": "move", "joke_id": jid, "to": k, "words": format!("{head}{tail}")}))
}

fn spot(kind: &str, words: &str, joke_id: Option<&str>, actions: Vec<Value>, extra: &[(&str, &str)]) -> Value {
    let mut m = Map::new();
    m.insert("kind".into(), json!(kind));
    m.insert("joke_id".into(), json!(joke_id));
    m.insert("words".into(), json!(words));
    m.insert("actions".into(), Value::Array(actions));
    for (k, v) in extra {
        m.insert((*k).to_string(), json!(v));
    }
    Value::Object(m)
}

/// `tough_spots(c, ws, with_actions)`: per bit (a locked bit is perfected: no spots of its own) and for the set.
#[allow(clippy::too_many_lines)]
pub fn tough_spots(
    story: &Story<'_>,
    c: &Corpus,
    ws: &Obj,
    with_actions: bool,
    tr: &Transitions<'_, '_>,
) -> (IndexMap<String, Vec<Value>>, Vec<Value>) {
    let empty = Obj::new();
    let ver = current(ws).unwrap_or(&empty);
    let ids = ids_of(ws);
    let mut per: IndexMap<String, Vec<Value>> = ids.iter().map(|j| ((*j).to_string(), Vec::new())).collect();
    let mut set_spots = Vec::new();
    for (i, &j) in ids.iter().enumerate() {
        let locked = bit_locked(ws, j);
        if i > 0 {
            let prev = ids[i - 1];
            let t = tr.get(prev, j);
            let (words, weak) = transition_words(&t);
            if weak {
                let mut acts = Vec::new();
                if with_actions {
                    acts.push(json!({"kind": "bridge", "from": prev, "to": j, "words": "Get a bridge idea (draft)"}));
                    if !locked && let Some(m) = move_option(c, &ids, j, tr) {
                        acts.push(m);
                    }
                }
                let w = format!("From {}: {}", short_title(Some(&c.title(prev))), words.to_lowercase());
                per[j].push(spot("transition", &w, Some(j), acts, &[("from", prev), ("to", j)]));
            }
        }
        if locked {
            continue;
        }
        let lab = bit_label(c, j);
        let ps: Vec<(&Perf, Outcome)> = perfs(c, j)
            .iter()
            .filter_map(|p| night_outcome(p).map(|o| (p, o)))
            .collect();
        let acts = || -> Vec<Value> {
            if !with_actions {
                return Vec::new();
            }
            let mut a = swap_options(story, c, &ids, j);
            a.extend(tag_option(c, j));
            a
        };
        if lab.label == "quiet" {
            let w = format!("Hasn't been landing: {}", lab.label_words);
            per[j].push(spot("quiet", &w, Some(j), acts(), &[]));
        } else if let [before @ .., (lp, lo)] = ps.as_slice()
            && !before.is_empty()
            && matches!(lo, Outcome::Quiet | Outcome::Some)
            && before.iter().any(|(_, o)| o.hit())
        {
            fn key(p: &Perf) -> (f64, &str) {
                (p.vs_room.unwrap_or(0.0).min(3.0), p.date.as_str())
            }
            let hit = before
                .iter()
                .fold(&before[0], |b, x| if key(x.0) > key(b.0) { x } else { b });
            let w = format!(
                "Dropping lately: {} {} after {} {}",
                lo.word().to_lowercase(),
                where_p(lp),
                hit.1.word().to_lowercase(),
                where_p(hit.0)
            );
            per[j].push(spot("dropping", &w, Some(j), acts(), &[]));
        }
        if let Some((last, _)) = ps.last() {
            let da = last.dead_air_max_s;
            let dur = if last.duration_s == 0.0 { 1.0 } else { last.duration_s };
            if da >= LONG_QUIET_S && da >= LONG_QUIET_SHARE * dur && lab.label != "quiet" {
                let t = if with_actions { tag_option(c, j) } else { None };
                let w = format!(
                    "A {}-second stretch with no laughs {}",
                    da.trunc() as i64,
                    where_p(last)
                );
                per[j].push(spot("stretch", &w, Some(j), t.into_iter().collect(), &[]));
            }
            let usual = median(ps.iter().map(|(p, _)| p.duration_s).collect());
            if ps.len() >= 2 && last.duration_s > RAN_LONG * usual && last.duration_s - usual > 30.0 {
                let w = format!(
                    "Ran {} {}, usually {}: trim the setup",
                    mmss(last.duration_s),
                    where_p(last),
                    mmss(usual)
                );
                per[j].push(spot("long", &w, Some(j), Vec::new(), &[]));
            }
        }
    }
    // the set as a whole
    let target_v = ver.get("target_s");
    let target = int_or_zero(target_v);
    let target_f = num(target_v).unwrap_or(0.0);
    let (total, _) = runtime(c, &ids);
    let (st, words) = length_status(total, target);
    let (lo, hi) = target_band(target);
    if st == "over" && ids.len() > 2 {
        let mut acts = Vec::new();
        if with_actions {
            let mut cands: Vec<(Strength, &str)> = ids[1..ids.len() - 1]
                .iter()
                .filter(|j| !bit_locked(ws, j))
                .map(|j| (strength(c, j, &bit_label(c, j)), *j))
                .collect();
            cands.sort_by(|a, b| {
                a.0.0
                    .cmp(&b.0.0)
                    .then(a.0.1.total_cmp(&b.0.1))
                    .then(a.0.2.cmp(&b.0.2))
                    .then_with(|| a.1.cmp(b.1))
            });
            for (_, j) in cands.into_iter().take(2) {
                let d = c.duration(j).0;
                let lw = bit_label(c, j).label_words;
                let mut chars = lw.chars();
                let lw = chars
                    .next()
                    .map_or(String::new(), |f| f.to_lowercase().collect::<String>() + chars.as_str());
                acts.push(json!({"kind": "cut", "joke_id": j,
                                 "words": format!("Cut {} ({}, {lw})", short_title(Some(&c.title(j))), mmss(d))}));
            }
        }
        set_spots.push(spot(
            "over",
            &format!("{words} your {}", length_words(target_f)),
            None,
            acts,
            &[],
        ));
    } else if st == "short" && !ids.is_empty() {
        let mut acts = Vec::new();
        if with_actions {
            let last = short_title(Some(&c.title(ids[ids.len() - 1])));
            for x in best_next(story, c, &ids, 2) {
                let xj = x.get("joke_id").and_then(Value::as_str).unwrap_or("");
                let why = plain_best_next(x.get("why").and_then(Value::as_str).unwrap_or(""), &last);
                acts.push(
                    json!({"kind": "add", "joke_id": xj, "card": with_label(c, xj, vec![("why", json!(why))]),
                                 "words": format!("Add {}", short_title(Some(&c.title(xj))))}),
                );
            }
        }
        set_spots.push(spot(
            "short",
            &format!("{words} of {}", length_words(lo as f64)),
            None,
            acts,
            &[],
        ));
    }
    let _ = hi;
    if ids.len() >= 3 && !bit_locked(ws, ids[ids.len() - 1]) {
        let last = ids[ids.len() - 1];
        if matches!(bit_label(c, last).label, "quiet" | "mixed") {
            let sug = ids[..ids.len() - 1]
                .iter()
                .filter(|j| bit_label(c, j).label == "killed")
                .map(|j| (strength(c, j, &bit_label(c, j)), *j))
                .fold(None::<(Strength, &str)>, |b, x| match b {
                    Some(b) if x.partial_cmp(&b) != Some(std::cmp::Ordering::Greater) => Some(b),
                    _ => Some(x),
                })
                .map(|x| x.1);
            let acts = match sug {
                Some(s) if with_actions => vec![json!({"kind": "move", "joke_id": s, "to": ids.len() - 1,
                                                       "words": format!("Close on {}", short_title(Some(&c.title(s))))})],
                _ => Vec::new(),
            };
            let tail = sug.map_or(String::new(), |s| {
                format!(": {} kills more", short_title(Some(&c.title(s))))
            });
            set_spots.push(spot(
                "closer",
                &format!("The ending could be stronger{tail}"),
                Some(last),
                acts,
                &[],
            ));
        }
    }
    (per, set_spots)
}

/// `health(c, ws, spots)`: one word for the set, and how many tough spots.
pub fn health(c: &Corpus, ws: &Obj, spots: &(IndexMap<String, Vec<Value>>, Vec<Value>)) -> (String, usize) {
    let ids = ids_of(ws);
    let n = spots.0.values().map(Vec::len).sum::<usize>() + spots.1.len();
    let word = if ids.is_empty() {
        "empty".to_string()
    } else if n > 0 {
        format!("{n} tough spot{}", if n == 1 { "" } else { "s" })
    } else if ids.iter().all(|j| bit_label(c, j).label == "untested") {
        "untested".to_string()
    } else {
        "solid".to_string()
    };
    (word, n)
}

fn made_by(by: Option<&Value>, owner: &str) -> &'static str {
    if by.and_then(Value::as_str) == Some(owner) {
        "you"
    } else {
        "Punchable"
    }
}

fn get(m: &Obj, k: &str) -> Value {
    m.get(k).cloned().unwrap_or(Value::Null)
}

/// `summary(c, ws)`: one row of `GET /api/mysets`.
pub fn summary(story: &Story<'_>, c: &Corpus, ws: &Obj, owner: &str) -> Value {
    let empty = Obj::new();
    let ver = current(ws).unwrap_or(&empty);
    let ids = ids_of(ws);
    let target = ver.get("target_s");
    let (total, est) = runtime(c, &ids);
    let (st, words) = length_status(total, int_or_zero(target));
    let tr = Transitions::new(story);
    let spots = tough_spots(story, c, ws, false, &tr);
    let (word, n) = health(c, ws, &spots);
    json!({"id": get(ws, "id"), "title": get(ws, "title"), "target_s": target.cloned().unwrap_or(Value::Null),
           "length_words": length_words(num(target).unwrap_or(0.0)), "n": ids.len(), "runtime_s": total,
           "runtime": mmss(total), "estimated_n": est, "status": st, "status_words": words, "health": word,
           "tough_n": n, "version": get(ver, "v"), "storyboard_id": get(ws, "storyboard_id"),
           "updated": get(ws, "updated_at"), "source": get(ws, "source"), "made_by": made_by(ws.get("by"), owner)})
}

/// `performed_in(c, set_key)`: the bits done in a recording, in order, once each.
fn performed_in(c: &Corpus, key: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for x in c.segments.get(key).into_iter().flatten() {
        if let Some(j) = x.get("joke").filter(|v| py::truthy(Some(v))).map(py::str)
            && matches!(s(x, "status"), Some("known" | "new"))
            && !out.contains(&j)
        {
            out.push(j);
        }
    }
    out
}

/// `match_score(plan, performed)`: (score, shared bits).
fn match_score(plan: &[&str], performed: &[String]) -> (f64, usize) {
    let shared: Vec<String> = plan
        .iter()
        .filter(|j| performed.iter().any(|p| p == *j))
        .map(|j| (*j).to_string())
        .collect();
    if plan.is_empty() || performed.is_empty() || shared.len() < plan.len().min(2) {
        return (0.0, shared.len());
    }
    let rec = shared.len() as f64 / plan.len() as f64;
    let prec = shared.len() as f64 / performed.len() as f64;
    let f1 = 2.0 * rec * prec / (rec + prec);
    let in_order: Vec<String> = performed.iter().filter(|p| shared.contains(p)).cloned().collect();
    let order = crate::today::lcs(&shared, &in_order).len() as f64 / shared.len() as f64;
    (round_to(f1 * (0.8 + 0.2 * order), 3), shared.len())
}

/// The id of the set a recording lines up with (`match_recording(...)["match"]["id"]`): his pick at import, else
/// the closest at `MATCH_MIN` or better.
fn matched_set(c: &Corpus, sets: &[&Obj], key: &str) -> Option<String> {
    let perf = performed_in(c, key);
    let mut ranked: Vec<(f64, usize, String)> = sets
        .iter()
        .filter(|w| !py::truthy(w.get("deleted")))
        .map(|w| {
            let (sc, n) = match_score(&ids_of(w), &perf);
            (sc, n, w.get("id").map(py::str).unwrap_or_default())
        })
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)).then_with(|| a.2.cmp(&b.2)));
    if let Some(p) = meta_pick(c, key)
        && ranked.iter().any(|r| r.2 == p)
    {
        return Some(p);
    }
    ranked.first().filter(|r| r.0 >= MATCH_MIN).map(|r| r.2.clone())
}

/// `c.metas[set_key]["align_working_set"]`: his own pick at import.
fn meta_pick(c: &Corpus, key: &str) -> Option<String> {
    c.metas
        .get(key)
        .and_then(|m| m.get("align_working_set"))
        .filter(|v| !v.is_null())
        .map(py::str)
}

/// `tapes_summary(c, sets, ws)`: the recordings that line up with this set, and the last one in words.
fn tapes_summary(c: &Corpus, sets: &[&Obj], ws: &Obj) -> Value {
    let id = ws.get("id").map(py::str).unwrap_or_default();
    let keys: Vec<String> = c
        .set_keys()
        .into_iter()
        .filter(|k| matched_set(c, sets, k).as_deref() == Some(id.as_str()))
        .collect();
    let Some(lastk) = keys.last() else {
        return json!({"n": 0, "words": "No tape of this set yet. Import a recording and it lines up here.", "last": null});
    };
    let night = crate::today::align(c, ws, lastk);
    let totals = night.get("totals").cloned().unwrap_or(Value::Null);
    let tv = |k: &str| totals.get(k).cloned().unwrap_or(Value::Null);
    let same_order = night
        .get("order")
        .and_then(|o| o.get("same_order"))
        .cloned()
        .unwrap_or(Value::Null);
    // `t["performed_planned_s"] + t["added_s"]`: an int when both are
    let run_s = match (totals.get("performed_planned_s"), totals.get("added_s")) {
        (Some(x), Some(y)) if x.is_i64() && y.is_i64() => json!(x.as_i64().unwrap_or(0) + y.as_i64().unwrap_or(0)),
        (Some(x), Some(y)) => json!(x.as_f64().unwrap_or(0.0) + y.as_f64().unwrap_or(0.0)),
        _ => Value::Null,
    };
    let last = json!({"set": lastk, "date": night.get("date"), "venue_label": night.get("venue_label"),
                      "bits_done": tv("bits_performed"), "bits_planned": tv("bits_planned"),
                      "dropped": night.get("dropped"), "same_order": same_order, "run_s": run_s});
    let where_s = where_(
        night.get("venue").and_then(Value::as_str),
        night.get("date").and_then(Value::as_str),
    );
    let words = format!(
        "Last tape {where_s}: {} of {} bits{}",
        py::str(&tv("bits_performed")),
        py::str(&tv("bits_planned")),
        if py::truthy(Some(&same_order)) {
            ""
        } else {
            ", in a different order"
        }
    );
    json!({"n": keys.len(), "words": words, "last": last, "keys": keys})
}

/// `trim(c, ws, target_s)`: "make a 5 from my 10": the opener and the closer, then the strongest bits by stage
/// history (locked bits first), in the set's own order, until the length is reached. Nothing is written.
pub fn trim(c: &Corpus, ws: &Obj, target_s: i64) -> Value {
    let ids = ids_of(ws);
    let (lo, hi) = target_band(target_s);
    let dur = |j: &str| c.duration(j).0;
    let keep: Vec<&str> = if ids.len() <= 2 {
        ids.clone()
    } else {
        let ends = [ids[0], ids[ids.len() - 1]];
        let mut budget = hi as f64 - py::sum(ends.iter().map(|j| dur(j)));
        let mut mid: Vec<(bool, Strength, &str)> = ids[1..ids.len() - 1]
            .iter()
            .map(|j| (!bit_locked(ws, j), strength(c, j, &bit_label(c, j)), *j))
            .collect();
        mid.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(b.1.0.cmp(&a.1.0))
                .then(b.1.1.total_cmp(&a.1.1))
                .then(b.1.2.cmp(&a.1.2))
                .then_with(|| a.2.cmp(b.2))
        });
        let mut chosen: HashSet<&str> = HashSet::new();
        let mut have = py::sum(ends.iter().map(|j| dur(j)));
        for (_, _, j) in mid {
            let weak = matches!(bit_label(c, j).label, "quiet" | "untested") && !bit_locked(ws, j);
            let d = dur(j);
            if d <= budget && !(weak && have >= lo as f64) {
                chosen.insert(j);
                budget -= d;
                have += d;
            }
        }
        ids.iter()
            .copied()
            .filter(|j| ends.contains(j) || chosen.contains(j))
            .collect()
    };
    let total = round_to(py::sum(keep.iter().map(|j| dur(j))), 1);
    let (st, words) = length_status(total, target_s);
    let cut: Vec<&str> = ids.iter().copied().filter(|j| !keep.contains(j)).collect();
    let title = ws.get("title").map_or_else(|| "None".into(), py::str);
    json!({"from": get(ws, "id"), "title": format!("{} from {title}", length_words(target_s as f64)),
           "target_s": target_s, "runtime_s": total, "runtime": mmss(total), "status": st, "status_words": words,
           "keep": keep.iter().enumerate().map(|(i, j)| with_label(c, j, vec![("position", json!(i))])).collect::<Vec<_>>(),
           "cut": cut.iter().map(|j| with_label(c, j, Vec::new())).collect::<Vec<_>>(),
           "why": "Keeps your opener and your closer, then the bits that do best on stage, in the same order."})
}

/// `view(c, ws, sets)`: the set screen: the bits in order with run time against the length, what's working per
/// bit, every tough spot with its actions, locks, the version history and the last tape that lined up with it.
pub fn view(story: &Story<'_>, c: &Corpus, ws: &Obj, sets: &[&Obj], owner: &str) -> Value {
    let tr = Transitions::new(story);
    let empty = Obj::new();
    let ver = current(ws).unwrap_or(&empty);
    let ids = ids_of(ws);
    let spots = tough_spots(story, c, ws, true, &tr);
    let (total, est) = runtime(c, &ids);
    let target_v = ver.get("target_s");
    let target = int_or_zero(target_v);
    let (st, words) = length_status(total, target);
    let (lo, hi) = target_band(target);
    let none = HashSet::new();
    let bits: Vec<Value> = ids
        .iter()
        .enumerate()
        .map(|(i, j)| {
            let lab = bit_label(c, j);
            let tin = if i == 0 {
                Value::Null
            } else {
                let t = tr.get(ids[i - 1], j);
                let (tw, weak) = transition_words(&t);
                json!({"words": tw, "weak": weak, "kind": t.get("label"), "from": ids[i - 1], "to": j})
            };
            let tough = spots.0.get(*j).cloned().unwrap_or_default();
            card(
                c,
                j,
                &none,
                [
                    ("position", json!(i)),
                    ("label", json!(lab.label)),
                    ("label_words", json!(lab.label_words)),
                    ("last_words", json!(lab.last_words)),
                    ("locked", json!(bit_locked(ws, j))),
                    ("transition_in", tin),
                    ("tough", Value::Array(tough)),
                ],
            )
        })
        .collect();
    let (word, n) = health(c, ws, &spots);
    let nxt: Vec<Value> = if ids.is_empty() {
        Vec::new()
    } else {
        let last = short_title(Some(&c.title(ids[ids.len() - 1])));
        best_next(story, c, &ids, 3)
            .into_iter()
            .map(|x| {
                let xj = x.get("joke_id").and_then(Value::as_str).unwrap_or("");
                let why = plain_best_next(x.get("why").and_then(Value::as_str).unwrap_or(""), &last);
                with_label(c, xj, vec![("why", json!(why))])
            })
            .collect()
    };
    let versions: Vec<Value> = ws
        .get("versions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
        .filter_map(Value::as_object)
        .map(|v| {
            let bits = v.get("bits").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
            let titles: Vec<String> = bits
                .iter()
                .map(|b| short_title(b.get("title").and_then(Value::as_str)))
                .collect();
            json!({"v": get(v, "v"), "created_at": get(v, "created_at"), "note": get(v, "note"), "n": bits.len(),
                   "target_s": get(v, "target_s"), "by": made_by(v.get("by"), owner), "titles": titles})
        })
        .collect();
    let tough_set = spots.1.clone();
    json!({
        "set": {"id": get(ws, "id"), "title": get(ws, "title"), "target_s": hi, "target_lo_s": lo,
                "length_words": length_words(num(target_v).unwrap_or(0.0)), "version": get(ver, "v"),
                "storyboard_id": get(ws, "storyboard_id"), "created_at": get(ws, "created_at"),
                "updated_at": get(ws, "updated_at"), "made_by": made_by(ws.get("by"), owner)},
        "runtime_s": total, "runtime": mmss(total), "estimated_n": est, "status": st, "status_words": words,
        "health": word, "tough_n": n, "bits": bits, "tough": tough_set, "next": nxt,
        "tapes": tapes_summary(c, sets, ws), "versions": versions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_words_are_minutes_with_one_decimal_when_needed() {
        assert_eq!(length_words(300.0), "5 min");
        assert_eq!(length_words(450.0), "7.5 min");
        assert_eq!(length_words(0.0), "0 min");
    }

    #[test]
    fn the_match_score_weighs_order() {
        let perf: Vec<String> = ["a", "b", "c"].iter().map(|x| (*x).to_string()).collect();
        assert_eq!(match_score(&["a", "b", "c"], &perf), (1.0, 3));
        assert_eq!(match_score(&["c", "b", "a"], &perf).1, 3);
        assert!(match_score(&["c", "b", "a"], &perf).0 < 1.0);
        assert_eq!(match_score(&["a", "x"], &perf), (0.0, 1));
        assert_eq!(match_score(&[], &perf), (0.0, 0));
    }

    #[test]
    fn of_counts_nights_in_words() {
        assert_eq!(of(2, 2), "both nights");
        assert_eq!(of(3, 3), "every night");
        assert_eq!(of(1, 1), "");
        assert_eq!(of(1, 3), "1 of 3 nights");
    }
}
