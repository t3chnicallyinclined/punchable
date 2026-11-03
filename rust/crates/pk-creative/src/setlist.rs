//! A set list as the Storyboard and set-builder screens show it (`creative/story.py` `health`, `best_next`,
//! `export_setlist`, `board_view`; `creative/notebook.py` `card`, `_transition_words`, `_plain_best_next`, `build`,
//! `target_band`). Pure functions over one notebook with its story caches ([`Story`]) and a store item (a board, or
//! `{"id", "joke_ids", "target_s"}` built for a set).
//!
//! Every value is the Python's rule for it, in the Python's order of float operations, so the payloads compare equal
//! as values. One reordering that cannot change an answer: `best_next` ranks its pool by score and id alone, so the
//! transition into each candidate is computed for the `cap` it returns, not for the whole pool.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::BuildHasher;
use std::sync::LazyLock;

use indexmap::IndexMap;
use pk_core::py;
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::ask::words::{is_stop, mmss, short_title};
use crate::corpus::{Corpus, num, round_to, s};
use crate::display::short_name_of;
use crate::notebook::{best_line, stage_words};
use crate::story::Story;

/// A store item or a payload object.
pub type Obj = Map<String, Value>;

/// `ROOMS_MIN`: nights a strong closer needs.
const ROOMS_MIN: usize = 2;
/// `CLOSER_STRONG`: mean `vs_room` of a strong closer.
pub const CLOSER_STRONG: f64 = 1.2;
/// `LOW_ENERGY`: a low bit plays under this.
const LOW_ENERGY: f64 = 0.8;
/// `TARGET_TOL`: within this share of the target is on target.
const TARGET_TOL: f64 = 0.10;
/// `notebook.BAND`: a 12:00 target means "10-12 min" (one minute under 8 min).
const BAND: i64 = 120;

/// The notebook's `VAGUE` words: they pass the filters but never name a subject.
static VAGUE: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    "let lets till until example instantly actually probably basically literally whatever someone something \
     anything everything nothing maybe still first last never always back new old day days year years guy guys thing \
     kind sort lot lots stuff place part point fact case yes yeah nope tried try trying told tell asked ask went goes \
     came put took take saw seen looked feel felt thought guess wanted need needed wait hey hello two three four five six \
     seven eight nine ten twenty thirty forty fifty hundred thousand million"
        .split_whitespace()
        .collect()
});
/// The first `( … )` in a reason.
static PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(([^)]*)\)").expect("a valid regex"));

fn get(m: &Obj, k: &str) -> Value {
    m.get(k).cloned().unwrap_or(Value::Null)
}

/// `int(x or 0)` for a stored number (`bool` is 0/1, a float is cut toward zero).
pub fn int_or_zero(v: Option<&Value>) -> i64 {
    num(v.filter(|x| py::truthy(Some(x)))).map_or(0, |x| x.trunc() as i64)
}

/// The joke ids of a list (`board["joke_ids"]`), as text (a non-text id is no joke here, as a dict lookup with it
/// finds nothing in the Python).
pub fn ids_of(v: Option<&Value>) -> Vec<&str> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|x| x.as_str().unwrap_or(""))
        .collect()
}

/// `notebook.target_band(target_s)`: `(lo, hi)` seconds.
pub fn target_band(t: i64) -> (i64, i64) {
    ((t - if t >= 480 { BAND } else { 60 }).max(60), t)
}

/// `subject_words(ws)`: up to three words that name a subject.
pub fn subject_words<'a>(ws: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    ws.into_iter()
        .filter(|w| !is_stop(w) && !VAGUE.contains(w) && w.chars().count() >= 3)
        .take(3)
        .collect()
}

/// `_subject(ws)`: "Same subject", with the words when any name one.
pub fn subject_phrase<'a>(ws: impl IntoIterator<Item = &'a str>) -> String {
    let good = subject_words(ws);
    if good.is_empty() {
        "Same subject".to_string()
    } else {
        format!("Same subject: {}", good.join(", "))
    }
}

/// `_transition_words(t)`: the way into a bit in words, and whether it is weak (nothing links the two, or only a
/// shared world with no line between them yet).
pub fn transition_words(t: &Value) -> (String, bool) {
    let label = t.get("label").and_then(Value::as_str);
    if label == Some("proven") {
        return ("Done back to back on stage".to_string(), false);
    }
    let via: Vec<&Value> = t.get("via").and_then(Value::as_array).into_iter().flatten().collect();
    if label == Some("link") {
        fn kind(v: &Value) -> Option<&str> {
            v.get("kind").and_then(Value::as_str)
        }
        let has = |k: &str| via.iter().any(|v| kind(v) == Some(k));
        if let Some(v) = via.iter().find(|v| kind(v) == Some("segue")) {
            let his = v.get("source").and_then(Value::as_str) == Some("his");
            return ((if his { "Your segue" } else { "Could lead in" }).to_string(), false);
        }
        if has("callback_to") {
            return ("Callback".to_string(), false);
        }
        if has("same_premise") {
            return ("Same premise".to_string(), false);
        }
        if has("persona") || has("topic") {
            let names: Vec<String> = via
                .iter()
                .filter(|v| matches!(kind(v), Some("persona" | "topic")))
                .filter_map(|v| v.get("name").filter(|n| py::truthy(Some(n))).map(py::str))
                .take(2)
                .collect();
            let tail = if names.is_empty() {
                String::new()
            } else {
                format!(": {}", names.join(", "))
            };
            return (format!("Same world{tail}, needs a line between"), true);
        }
        if has("subject") {
            let ws: Vec<&str> = via
                .iter()
                .find(|v| kind(v) == Some("subject"))
                .and_then(|v| v.get("words"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            return (format!("{}, needs a line between", subject_phrase(ws)), true);
        }
        return ("Linked".to_string(), py::truthy(t.get("needs_bridge")));
    }
    ("No link yet".to_string(), true)
}

/// `_plain_best_next(why, last_title)`: `best_next`'s reasons in his words (no ratios, no "x the room").
pub fn plain_best_next(why: &str, last_title: &str) -> String {
    let mut parts: IndexMap<String, ()> = IndexMap::new();
    let last = short_title(Some(last_title));
    for p in why.split("; ") {
        let p = py::strip(p);
        let paren = || PAREN.captures(p).map(|m| m[1].to_string());
        let part = if p.starts_with("same subject as") {
            let inside = paren();
            Some(subject_phrase(
                inside
                    .as_deref()
                    .map(|x| x.split(", ").collect::<Vec<_>>())
                    .unwrap_or_default(),
            ))
        } else if p.starts_with("done right after") {
            Some(format!("You did it right after {last} on stage"))
        } else if p.starts_with("segue") {
            Some(format!("Could follow {last}"))
        } else if p.starts_with("same_premise") || p.starts_with("same premise") {
            Some("Same premise".to_string())
        } else if p.starts_with("callback") {
            Some("Calls back".to_string())
        } else if p.starts_with("same world as") {
            Some(paren().map_or_else(|| "Same world".to_string(), |m| format!("Same world: {m}")))
        } else if p.starts_with("strong on tape") {
            Some("Strong on stage".to_string())
        } else {
            None
        };
        if let Some(x) = part {
            parts.insert(x, ());
        }
    }
    if parts.is_empty() {
        "From your notebook".to_string()
    } else {
        parts.keys().cloned().collect::<Vec<_>>().join("; ")
    }
}

/// `notebook.card(c, jid, terms, **extra)`: a bit as the app shows it anywhere (short title, his line and its date,
/// one stage line, length), with `extra` laid over it.
pub fn card<S: BuildHasher>(
    c: &Corpus,
    jid: &str,
    terms: &HashSet<String, S>,
    extra: impl IntoIterator<Item = (&'static str, Value)>,
) -> Value {
    let n = c.joke(jid);
    let title = n.and_then(|n| s(n, "title"));
    let line = best_line(c, jid, terms);
    let (stage_line, proven, nights) = stage_words(c, jid);
    let (d, _) = c.duration(jid);
    let first_seen: String = n
        .and_then(|n| s(n, "first_seen"))
        .unwrap_or("")
        .chars()
        .take(10)
        .collect();
    let date = match line.as_ref().and_then(|l| l.date.clone()).filter(|d| !d.is_empty()) {
        Some(d) => Value::String(d),
        None if !first_seen.is_empty() => Value::String(first_seen),
        None => Value::Null,
    };
    let mut out = json!({
        "joke_id": jid, "title": short_title(title),
        "short_name": short_name_of(title.unwrap_or(""), n.and_then(|n| s(n, "short_name"))),
        "stage": c.stage(jid).0,
        "quote": line.as_ref().map(|l| l.quote.clone()), "quote_node": line.as_ref().map(|l| l.quote_node.clone()),
        "cut_start": line.as_ref().is_some_and(|l| l.cut_start), "cut_end": line.as_ref().is_some_and(|l| l.cut_end),
        "date": date, "stage_line": stage_line, "proven": proven, "nights": nights,
        "duration_s": d, "estimated": nights == 0,
    });
    if let Value::Object(m) = &mut out {
        for (k, v) in extra {
            m.insert(k.to_string(), v);
        }
    }
    out
}

/// `notebook.related(c, text, cap, hybrid, exclude)`: his bits about an idea as cards: his-words search fused by rank
/// with the database's fused search (`hybrid`, joke ids best first) when there is one, the excluded bits left out, then
/// among the five closest the ones that worked on stage first.
pub fn related(
    c: &Corpus,
    ix: &crate::notebook::Index,
    text: &str,
    cap: usize,
    hybrid: Option<&[String]>,
    exclude: &[&str],
) -> Value {
    let terms: HashSet<String> = crate::ask::words::tokens(text).into_iter().collect();
    let mut fused: IndexMap<String, f64> = IndexMap::new();
    for (rank, (j, _)) in ix.lexical(text).into_iter().take(60).enumerate() {
        *fused.entry(j).or_default() += 1.0 / (60.0 + rank as f64);
    }
    for (rank, j) in hybrid.unwrap_or(&[]).iter().take(12).enumerate() {
        let m = c.map_joke(j);
        let j = if m.is_empty() { j.clone() } else { m };
        if c.is_joke(&j) {
            *fused.entry(j).or_default() += 1.0 / (60.0 + rank as f64);
        }
    }
    let mut ranked: Vec<(String, f64)> = fused.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut items: Vec<(bool, usize, Value)> = ranked
        .into_iter()
        .filter(|(j, _)| !exclude.contains(&j.as_str()))
        .take(cap)
        .enumerate()
        .map(|(r, (j, _))| {
            let it = card(c, &j, &terms, [("rank", json!(r)), ("why", json!(ix.why(&j, &terms)))]);
            (py::truthy(it.get("proven")) && r < 5, r, it)
        })
        .collect();
    items.sort_by_key(|(first, r, _)| (usize::from(!*first), *r));
    let items: Vec<Value> = items.into_iter().map(|x| x.2).collect();
    json!({"query": text, "total": items.len(), "items": items})
}

/// `story.heat(energy)`: 1-4 for the bars.
pub fn heat(e: Option<f64>) -> Value {
    match e {
        None => Value::Null,
        Some(e) if e < 0.5 => json!(1),
        Some(e) if e < LOW_ENERGY + 0.1 => json!(2),
        Some(e) if e < 1.3 => json!(3),
        Some(_) => json!(4),
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// One bit of a board in `health`: what the checks read of it, and its row.
struct HealthBit<'a> {
    id: &'a str,
    title: String,
    energy: Option<f64>,
    nights: usize,
    value: Value,
}

/// `story.health(c, board)`: the bits on the board in the notebook with their length, energy and moves, every
/// transition, and the five checks (runtime, jumps, repeated moves, the closer, energy dips).
#[allow(clippy::too_many_lines)]
pub fn health(story: &Story<'_>, c: &Corpus, board: &Obj) -> Value {
    let ids: Vec<&str> = ids_of(board.get("joke_ids"))
        .into_iter()
        .filter(|j| c.is_joke(j))
        .collect();
    let mut total = 0.0;
    let devices = |j: &str| c.devices(j, 3, 0.55);
    let name = |l: &str| c.label_field(l, "name");
    let bits: Vec<HealthBit<'_>> = ids
        .iter()
        .map(|j| {
            let (d, basis) = c.duration(j);
            let (e, n) = c.energy(j);
            total += d;
            let devs: Vec<Value> = devices(j).iter().map(|l| name(l)).collect();
            let title = c.title(j);
            let value = json!({"joke_id": j, "title": title, "stage": c.stage(j).0, "duration_s": d,
                               "duration_basis": basis, "energy": e, "heat": heat(e), "nights": n, "devices": devs});
            HealthBit {
                id: j,
                title,
                energy: e,
                nights: n,
                value,
            }
        })
        .collect();
    let trans: Vec<Value> = ids.windows(2).map(|w| story.transition(w[0], w[1])).collect();
    let mut checks: Vec<Value> = Vec::new();
    let tgt_v = board
        .get("target_s")
        .filter(|v| py::truthy(Some(v)))
        .cloned()
        .unwrap_or(json!(0));
    let tgt = num(Some(&tgt_v)).unwrap_or(0.0);
    // R1 runtime
    #[allow(clippy::float_cmp)] // `if not tgt`
    let no_target = tgt == 0.0;
    if no_target {
        checks.push(json!({"rule": "runtime", "status": "info", "text": format!("{} total (no target)", mmss(total))}));
    } else if (total - tgt).abs() <= TARGET_TOL * tgt {
        checks.push(json!({"rule": "runtime", "status": "ok",
                           "text": format!("{} of {}: on target", mmss(total), mmss(tgt))}));
    } else if total < tgt {
        checks.push(json!({"rule": "runtime", "status": "info",
                           "text": format!("{} of {}: {} to fill", mmss(total), mmss(tgt), mmss(tgt - total))}));
    } else {
        checks.push(json!({"rule": "runtime", "status": "warn",
                           "text": format!("{} of {}: {} over", mmss(total), mmss(tgt), mmss(total - tgt))}));
    }
    // R2 unbridged jumps
    let label = |t: &Value| t.get("label").and_then(Value::as_str).unwrap_or("").to_string();
    let end = |t: &Value, k: &str| t.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let jumps: Vec<&Value> = trans.iter().filter(|t| label(t) == "jump").collect();
    if jumps.is_empty() {
        let nb = trans.iter().filter(|t| py::truthy(t.get("needs_bridge"))).count();
        let tail = if nb == 0 {
            String::new()
        } else {
            format!(" ({nb} persona/topic link{} could use a bridge line)", plural(nb))
        };
        checks.push(json!({"rule": "jumps", "status": "ok", "text": format!("every transition has a link{tail}")}));
    } else {
        let names: Vec<String> = jumps
            .iter()
            .map(|t| format!("{} -> {}", c.title(&end(t, "from")), c.title(&end(t, "to"))))
            .collect();
        let pairs: Vec<Value> = jumps.iter().map(|t| json!([end(t, "from"), end(t, "to")])).collect();
        checks.push(json!({"rule": "jumps", "status": "warn",
                           "text": format!("{} topic jump{} with no bridge: {}", jumps.len(), plural(jumps.len()), names.join("; ")),
                           "pairs": pairs}));
    }
    // R3 consecutive repeated devices
    let mut reps: Vec<(&str, &str, Vec<Value>)> = Vec::new();
    for w in ids.windows(2) {
        let (da, db) = (devices(w[0]), devices(w[1]));
        let mut shared: Vec<&String> = da.iter().filter(|l| db.contains(l)).collect();
        shared.sort();
        shared.dedup();
        if !shared.is_empty() {
            reps.push((w[0], w[1], shared.iter().map(|l| name(l)).collect()));
        }
    }
    if let Some((a, b, devs)) = reps.first() {
        let more = if reps.len() > 1 {
            format!(" and {} more", reps.len() - 1)
        } else {
            String::new()
        };
        let repeats: Vec<Value> = reps
            .iter()
            .map(|(a, b, d)| json!({"pair": [a, b], "devices": d}))
            .collect();
        checks.push(json!({"rule": "repeated_devices", "status": "warn",
                           "text": format!("\"{}\" twice in a row ({} -> {}){more}", py::str(&devs[0]), c.title(a), c.title(b)),
                           "repeats": repeats}));
    } else {
        checks.push(json!({"rule": "repeated_devices", "status": "ok",
                           "text": "device variety: no move repeats back-to-back"}));
    }
    // R4 closer strength
    if let Some(last) = bits.last() {
        let killed_once = c
            .perfs
            .get(last.id)
            .is_some_and(|ps| ps.iter().any(|p| p.ai_rating == "killed"));
        let strong = last
            .energy
            .is_some_and(|e| (e >= CLOSER_STRONG && last.nights >= ROOMS_MIN) || (killed_once && e >= 1.0));
        if strong {
            let e = last.energy.unwrap_or(0.0);
            checks.push(json!({"rule": "closer", "status": "ok",
                               "text": format!("ends on a big one: {} ({e:.2}x the room, {} night{})",
                                               last.title, last.nights, plural(last.nights))}));
        } else {
            let sug = bits[..bits.len() - 1]
                .iter()
                .filter(|b| b.energy.is_some_and(|e| e >= CLOSER_STRONG))
                .fold(None::<&HealthBit<'_>>, |best, b| match best {
                    Some(x) if (b.energy, b.id) <= (x.energy, x.id) => Some(x),
                    _ => Some(b),
                });
            let mut text = match last.energy {
                None => format!("closer untested: {} has no tape", last.title),
                Some(e) => format!("closer could be stronger: {} averages {e:.2}x the room", last.title),
            };
            if let Some(b) = sug {
                let _ = write!(text, " (try {}, {:.2}x)", b.title, b.energy.unwrap_or(0.0));
            }
            checks.push(json!({"rule": "closer", "status": "warn", "text": text, "suggest": sug.map(|b| b.id)}));
        }
    }
    // R5 energy dip: two taped low bits in a row
    let low = |b: &HealthBit<'_>| b.energy.is_some_and(|e| e < LOW_ENERGY);
    let dips: Vec<(&str, &str)> = bits
        .windows(2)
        .filter(|w| low(&w[0]) && low(&w[1]))
        .map(|w| (w[0].id, w[1].id))
        .collect();
    if let Some((a, b)) = dips.first() {
        let pairs: Vec<Value> = dips.iter().map(|(a, b)| json!([a, b])).collect();
        checks.push(json!({"rule": "energy_dip", "status": "warn",
                           "text": format!("energy dips: {} and {} both play under {LOW_ENERGY}x the room", c.title(a), c.title(b)),
                           "pairs": pairs}));
    } else {
        checks.push(json!({"rule": "energy_dip", "status": "ok", "text": "no two low bits in a row"}));
    }
    let untaped: Vec<&str> = bits
        .iter()
        .filter(|b| b.energy.is_none())
        .map(|b| b.title.as_str())
        .collect();
    let curve: Vec<Value> = bits
        .iter()
        .map(|b| json!({"joke_id": b.id, "energy": b.energy, "heat": heat(b.energy)}))
        .collect();
    let bit_values: Vec<Value> = bits.iter().map(|b| b.value.clone()).collect();
    json!({"board": get(board, "id"), "title": get(board, "title"), "runtime_s": round_to(total, 1),
           "runtime": mmss(total), "target_s": tgt_v, "bits": bit_values, "transitions": trans,
           "energy_curve": curve, "checks": checks, "untaped": untaped,
           "notes": "energy = mean vs_room over taped nights (1.0 = the room's own laugh rate); heat 1-4 for bars."})
}

/// One candidate `best_next` scored.
struct Pick {
    score: f64,
    why: IndexMap<String, ()>,
    source: &'static str,
}

/// `story.best_next(c, board, cap)`: the bits that could come next, best first.
#[allow(clippy::too_many_lines)]
pub fn best_next(story: &Story<'_>, c: &Corpus, joke_ids: &[&str], cap: usize) -> Vec<Value> {
    let ids: Vec<&str> = joke_ids.iter().copied().filter(|j| c.is_joke(j)).collect();
    let on: HashSet<&str> = ids.iter().copied().collect();
    let mut pool: IndexMap<String, Pick> = IndexMap::new();
    let bump = |pool: &mut IndexMap<String, Pick>, j: &str, sc: f64, why: String, src: &'static str| {
        if on.contains(j) || !c.is_joke(j) || c.stage(j).0.as_str() == Some("retired") {
            return;
        }
        let p = pool.entry(j.to_string()).or_insert_with(|| Pick {
            score: 0.0,
            why: IndexMap::new(),
            source: "proposed",
        });
        p.score += sc;
        p.why.insert(why, ());
        if src == "his" || (src == "inferred" && p.source == "proposed") {
            p.source = src;
        }
    };
    let sagging;
    if let Some(&last) = ids.last() {
        let lt = c.title(last);
        for ((a, b), rows) in &c.back_to_back {
            if a == last {
                bump(
                    &mut pool,
                    b,
                    0.5 + 0.1 * rows.len() as f64,
                    format!("done right after {lt} {}x on tape", rows.len()),
                    "inferred",
                );
            }
        }
        for &i in c.edges_by_node.get(last).map_or(&[][..], Vec::as_slice) {
            let e = &c.edges[i];
            let (src, dst) = (s(e, "src").unwrap_or(""), s(e, "dst").unwrap_or(""));
            match s(e, "kind") {
                Some("segue") if src == last => {
                    let conf = num(e.get("confidence").filter(|v| py::truthy(Some(v)))).unwrap_or(0.5);
                    let reason: String = e
                        .get("reason")
                        .filter(|v| py::truthy(Some(v)))
                        .map(py::str)
                        .unwrap_or_default()
                        .chars()
                        .take(90)
                        .collect();
                    bump(&mut pool, dst, 0.4 * conf, format!("segue: {reason}"), c.source_of(e));
                }
                Some(k @ ("same_premise" | "callback_to")) => {
                    let o = if src == last { dst } else { src };
                    bump(
                        &mut pool,
                        o,
                        0.25,
                        format!("{} with {lt}", k.replace('_', " ")),
                        c.source_of(e),
                    );
                }
                _ => {}
            }
        }
        for o in &c.jokes {
            let sp = story.shared_labels(last, o, &["persona"], 0.5);
            if let Some(l) = sp.first()
                && o != last
            {
                let n = py::str(&c.label_field(l, "name"));
                bump(&mut pool, o, 0.2, format!("same world as {lt} ({n})"), "proposed");
            }
        }
        sagging = ids[ids.len().saturating_sub(2)..]
            .iter()
            .all(|j| c.energy(j).0.is_some_and(|e| e < 1.0));
    } else {
        sagging = false;
    }
    // same subject as the board: pulling his old jokes on the same premise in is the point
    let mut topical = false;
    if !ids.is_empty() {
        for o in &c.jokes {
            if on.contains(o.as_str()) {
                continue;
            }
            let (sc, ws, near) = story.same_subject(&ids, o);
            if sc >= 0.35 {
                topical = true;
                let near = near.map_or_else(|| "None".into(), |n| c.title(&n));
                let tail = if ws.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", ws.join(", "))
                };
                bump(
                    &mut pool,
                    o,
                    0.6 * sc,
                    format!("same subject as {near}{tail}"),
                    "inferred",
                );
            }
        }
    }
    // performance energy: anything taped, scaled by how it plays against the room
    for o in c.perfs.keys() {
        let (e, n) = c.energy(o);
        let Some(e) = e else { continue };
        if e >= CLOSER_STRONG && n >= ROOMS_MIN {
            let base = (if topical { 0.1 } else { 0.25 }) + if sagging { 0.1 } else { 0.0 };
            let lift = if sagging { "; lifts the energy" } else { "" };
            bump(
                &mut pool,
                o,
                base,
                format!("strong on tape ({e:.2}x the room over {n} nights){lift}"),
                "inferred",
            );
        } else if let Some(p) = pool.get_mut(o) {
            p.score += 0.05 * e.min(2.0);
        }
    }
    let mut ranked: Vec<(f64, String, Pick)> = pool
        .into_iter()
        .map(|(j, p)| (round_to(p.score.min(1.0), 3), j, p))
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    ranked.truncate(cap);
    ranked
        .into_iter()
        .map(|(score, j, p)| {
            let t = ids.last().map(|last| story.transition(last, &j));
            let (e, n) = c.energy(&j);
            let why: String = p
                .why
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
                .chars()
                .take(200)
                .collect();
            let ev: Vec<Value> = t
                .as_ref()
                .and_then(|t| t.get("evidence"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(1)
                .cloned()
                .collect();
            json!({"joke_id": j, "title": c.title(&j), "stage": c.stage(&j).0, "score": score, "why": why,
                   "source": p.source, "transition": t.as_ref().map_or(Value::Null, |t| get_v(t, "label")),
                   "energy": e, "nights": n, "evidence": ev})
        })
        .collect()
}

fn get_v(v: &Value, k: &str) -> Value {
    v.get(k).cloned().unwrap_or(Value::Null)
}

/// `story.export_setlist(c, board)`: the paper set list.
pub fn export_setlist(c: &Corpus, board: &Obj) -> Value {
    let mut total = 0.0;
    let mut lines = Vec::new();
    for (i, j) in ids_of(board.get("joke_ids")).into_iter().enumerate() {
        let (d, _) = c.duration(j);
        total += d;
        lines.push(format!("{}. {} ({})", i + 1, c.title(j), mmss(d)));
    }
    let title = board.get("title").map_or_else(|| "None".into(), py::str);
    json!({"board": get(board, "id"), "title": get(board, "title"), "lines": lines, "runtime": mmss(total),
           "text": format!("{title} · {}\n{}", mmss(total), lines.join("\n"))})
}

/// `story.board_view(c, board)`: everything the Storyboard screen needs in one payload.
pub fn board_view(story: &Story<'_>, c: &Corpus, board: &Obj) -> Value {
    let mut b = board.clone();
    b.insert("archived".into(), Value::Bool(py::truthy(board.get("archived"))));
    json!({"board": b, "health": health(story, c, board),
           "best_next": best_next(story, c, &ids_of(board.get("joke_ids")), 3),
           "setlist": export_setlist(c, board)})
}

/// The `(status, words)` of a run time against a target band: `none` without a target, `short` ("1:00 <short>"),
/// `over` ("1:00 over") or `on`. The set builder says "to fill", his sets say "short".
pub fn length_status_words(total: f64, lo: i64, hi: i64, short: &str) -> (&'static str, String) {
    if hi == 0 {
        ("none", mmss(total))
    } else if total < lo as f64 {
        ("short", format!("{} {short}", mmss(lo as f64 - total)))
    } else if total > hi as f64 {
        ("over", format!("{} over", mmss(total - hi as f64)))
    } else {
        ("on", "on target".to_string())
    }
}

/// `notebook.build(c, board, cap)`: a board as a set: the run time from his tapes (estimates marked), the target band,
/// every transition in words with the weak ones flagged, the best next bits, and the notes on the closer and dips.
pub fn build(story: &Story<'_>, c: &Corpus, board: &Obj, cap: usize) -> Value {
    let h = health(story, c, board);
    let (lo, hi) = target_band(int_or_zero(board.get("target_s")));
    let none = HashSet::new();
    let hb: &[Value] = h.get("bits").and_then(Value::as_array).map_or(&[], Vec::as_slice);
    let ht: &[Value] = h
        .get("transitions")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let mut steps: Vec<Value> = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    let mut weak_n = 0;
    for (i, b) in hb.iter().enumerate() {
        let j = b.get("joke_id").and_then(Value::as_str).unwrap_or("");
        let tin = if i == 0 {
            Value::Null
        } else {
            let t = &ht[i - 1];
            let (words, weak) = transition_words(t);
            weak_n += usize::from(weak);
            json!({"words": words, "weak": weak, "kind": get_v(t, "label"), "from": get_v(t, "from"), "to": get_v(t, "to")})
        };
        steps.push(card(c, j, &none, [("position", json!(i)), ("transition_in", tin)]));
        ids.push(j.to_string());
    }
    let total = h.get("runtime_s").and_then(Value::as_f64).unwrap_or(0.0);
    let (status, words) = length_status_words(total, lo, hi, "to fill");
    let last = ids.last().map(|j| c.title(j)).unwrap_or_default();
    let nxt: Vec<Value> = best_next(story, c, &ids_of(board.get("joke_ids")), cap)
        .into_iter()
        .map(|x| {
            let j = x.get("joke_id").and_then(Value::as_str).unwrap_or("");
            let why = plain_best_next(x.get("why").and_then(Value::as_str).unwrap_or(""), &last);
            let proven = x.get("transition").and_then(Value::as_str) == Some("proven");
            card(c, j, &none, [("why", json!(why)), ("proven_pair", json!(proven))])
        })
        .collect();
    let mut notes = Vec::new();
    for ch in h.get("checks").and_then(Value::as_array).into_iter().flatten() {
        let (rule, st) = (
            ch.get("rule").and_then(Value::as_str),
            ch.get("status").and_then(Value::as_str),
        );
        if rule == Some("closer") && st == Some("warn") && !ids.is_empty() {
            let sug = ch.get("suggest").and_then(Value::as_str).filter(|x| !x.is_empty());
            let tail = sug.map_or(String::new(), |x| {
                format!(": try closing on {}", short_title(Some(&c.title(x))))
            });
            notes.push(
                json!({"rule": "closer", "text": format!("The ending could be stronger{tail}"),
                              "suggest": get_v(ch, "suggest")}),
            );
        }
        if rule == Some("energy_dip") && st == Some("warn") {
            let pair = ch.get("pairs").and_then(|p| p.get(0));
            let at = |k: usize| pair.and_then(|p| p.get(k)).and_then(Value::as_str).unwrap_or("");
            notes.push(json!({"rule": "dip", "text": format!("Two quiet bits in a row: {}, {}",
                                                             short_title(Some(&c.title(at(0)))),
                                                             short_title(Some(&c.title(at(1)))))}));
        }
    }
    let estimated_n = steps.iter().filter(|s| py::truthy(s.get("estimated"))).count();
    json!({"board": {"id": get(board, "id"), "title": get(board, "title"), "target_s": hi, "target_lo_s": lo},
           "runtime_s": total, "runtime": mmss(total), "estimated_n": estimated_n, "status": status,
           "status_words": words, "weak_n": weak_n, "steps": steps, "next": nxt, "notes": notes})
}

/// A per-request transition cache (`mysets._T`): `story.transition` is the expensive part of a set's view.
pub struct Transitions<'s, 'c> {
    story: &'s Story<'c>,
    memo: std::cell::RefCell<HashMap<(String, String), std::rc::Rc<Value>>>,
}

impl<'s, 'c> Transitions<'s, 'c> {
    /// Empty, over `story`.
    pub fn new(story: &'s Story<'c>) -> Self {
        Self {
            story,
            memo: std::cell::RefCell::new(HashMap::new()),
        }
    }

    /// `tr(a, b)`.
    pub fn get(&self, a: &str, b: &str) -> std::rc::Rc<Value> {
        let k = (a.to_string(), b.to_string());
        if let Some(t) = self.memo.borrow().get(&k) {
            return std::rc::Rc::clone(t);
        }
        let t = std::rc::Rc::new(self.story.transition(a, b));
        self.memo.borrow_mut().insert(k, std::rc::Rc::clone(&t));
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_target_band_is_two_minutes_under_from_eight_minutes() {
        assert_eq!(target_band(720), (600, 720));
        assert_eq!(target_band(300), (240, 300));
        assert_eq!(target_band(0), (60, 0));
        assert_eq!(target_band(480), (360, 480));
    }

    #[test]
    fn plain_reasons_lose_the_ratios_and_keep_the_first_paren() {
        let why = "done right after Alpha bit (x) 2x on tape; same subject as Bravo bit (the dog one) (leash, park); \
                   strong on tape (1.40x the room over 3 nights); same world as Alpha (Mom)";
        assert_eq!(
            plain_best_next(why, "Alpha bit (x)"),
            "You did it right after Alpha bit on stage; Same subject: the dog one; Strong on stage; Same world: Mom"
        );
        assert_eq!(plain_best_next("", "x"), "From your notebook");
        assert_eq!(plain_best_next("segue: a; segue: b", "Zed"), "Could follow Zed");
    }

    #[test]
    fn transition_words_follow_the_python_order() {
        let t = |label: &str, via: Value| json!({"label": label, "via": via, "needs_bridge": false});
        assert!(!transition_words(&t("proven", json!([]))).1);
        assert_eq!(
            transition_words(&t("link", json!([{"kind": "segue", "source": "his"}]))),
            ("Your segue".to_string(), false)
        );
        assert_eq!(
            transition_words(&t(
                "link",
                json!([{"kind": "topic", "name": "Cars"}, {"kind": "persona", "name": "Mom"}])
            )),
            ("Same world: Cars, Mom, needs a line between".to_string(), true)
        );
        assert_eq!(
            transition_words(&t(
                "link",
                json!([{"kind": "subject", "words": ["the", "laundromat", "maybe"]}])
            )),
            ("Same subject: laundromat, needs a line between".to_string(), true)
        );
        assert_eq!(
            transition_words(&t("jump", json!([]))),
            ("No link yet".to_string(), true)
        );
    }

    #[test]
    fn heat_bands_are_the_python_s() {
        // `LOW_ENERGY + 0.1` is 0.9 in floats too: 0.9 is the next band
        assert_eq!(heat(Some(0.9)), json!(3));
        assert_eq!(heat(Some(0.899)), json!(2));
        assert_eq!(heat(Some(0.49)), json!(1));
        assert_eq!(heat(Some(1.29)), json!(3));
        assert_eq!(heat(Some(1.3)), json!(4));
        assert_eq!(heat(None), Value::Null);
    }
}
