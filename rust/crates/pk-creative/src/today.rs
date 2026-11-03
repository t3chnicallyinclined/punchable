//! The Today feed (`creative/today.py` `build`) and the parts of `creative/workingset.py` it reads: one night
//! aligned against the working set (`align`), each bit's nights and the wording said on some of them and not others
//! (`bit_history`, `wording_effects` without the night-to-night `changes`, which Today never reads), the notes to
//! try tonight, the perfected score and the status line. Pure functions over a [`Corpus`] and a working set item.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use sha1::Digest as _;

use crate::ask::mmss;
use crate::corpus::{Corpus, Perf, num, prefix_words, round_to, venue_short};
use crate::textdiff::{phrases, present, toks};
use pk_core::py;

/// Fewer nights than this on either side is a hunch, not a finding.
const LOW_N: usize = 3;
/// `vs_room` is clipped here (a near-silent room inflates it).
const VR_CLIP: f64 = 3.0;

type Obj = Map<String, Value>;

/// Nights vector -> (its phrases as `(is_punch, n words, phrase, segment)`, the phrase keys seen).
type Groups = IndexMap<Vec<bool>, (Vec<(bool, usize, String, String)>, HashSet<String>)>;

/// An effect with its nights vector, its rank and its first quote (the sort's keys).
type Ranked = (Effect, Vec<bool>, (f64, usize), String);

fn get(m: &Obj, k: &str) -> Value {
    m.get(k).cloned().unwrap_or(Value::Null)
}

fn sget<'a>(m: &'a Obj, k: &str) -> Option<&'a str> {
    m.get(k).and_then(Value::as_str)
}

fn venue_of(venue: &Value, date: &str) -> String {
    venue_short(venue.as_str(), Some(date))
}

/// `mmss(seconds)` with `None` as `-`.
pub fn mmss_opt(x: Option<f64>) -> String {
    x.map_or_else(|| "-".to_string(), mmss)
}

/// `util.mean`: the mean of the values that are there.
pub fn mean(xs: impl IntoIterator<Item = Option<f64>>) -> Option<f64> {
    let v: Vec<f64> = xs.into_iter().flatten().collect();
    (!v.is_empty()).then(|| py::sum(v.iter().copied()) / v.len() as f64)
}

/// `util.cv`: sample sd / mean; `None` with fewer than two values or a mean at or below zero.
pub fn cv(xs: impl IntoIterator<Item = Option<f64>>) -> Option<f64> {
    let v: Vec<f64> = xs.into_iter().flatten().collect();
    if v.len() < 2 {
        return None;
    }
    let mu = py::sum(v.iter().copied()) / v.len() as f64;
    if mu <= 0.0 {
        return None;
    }
    let var = py::sum(v.iter().map(|x| (x - mu).powi(2))) / (v.len() - 1) as f64;
    Some(var.sqrt() / mu)
}

fn r1(x: f64) -> f64 {
    round_to(x, 1)
}

/// `util.first_line`: the first line of three words or more, cut on a word boundary (always a substring), else the
/// first twelve words.
pub fn first_line(text: &str, max_chars: usize) -> String {
    for line in py::splitlines(text) {
        let s = py::strip(line);
        if py::split(s).count() >= 3 {
            let n = s.chars().count();
            if n <= max_chars {
                return s.to_string();
            }
            let cut: String = s.chars().take(max_chars).collect();
            return match cut.rfind(' ') {
                Some(sp) if cut[..sp].chars().count() > 20 => cut[..sp].to_string(),
                _ => cut,
            };
        }
    }
    prefix_words(text, 12)
}

/// `current(ws)`: the newest version.
fn current(ws: &Obj) -> &Obj {
    static EMPTY: std::sync::LazyLock<Obj> = std::sync::LazyLock::new(Obj::new);
    ws.get("versions")
        .and_then(Value::as_array)
        .and_then(|v| v.last())
        .and_then(Value::as_object)
        .unwrap_or(&EMPTY)
}

fn bits(ver: &Obj) -> Vec<&Obj> {
    ver.get("bits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .collect()
}

fn bit_id(b: &Obj) -> String {
    b.get("joke_id").map(py::str).unwrap_or_default()
}

fn target(b: &Obj) -> f64 {
    num(b.get("target_s")).unwrap_or(0.0)
}

fn lock<'a>(ws: &'a Obj, j: &str) -> Option<&'a Obj> {
    ws.get("locks")?.get(j)?.as_object()
}

/// `bit_locked(ws, j)`.
fn bit_locked(ws: &Obj, j: &str) -> bool {
    lock(ws, j).is_some_and(|l| py::truthy(l.get("bit")))
}

fn lock_lines<'a>(ws: &'a Obj, j: &str) -> Vec<&'a str> {
    lock(ws, j)
        .and_then(|l| l.get("lines"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|x| x.get("text").and_then(Value::as_str))
        .collect()
}

/// `is_locked_line(ws, j, line)`.
fn is_locked_line(ws: &Obj, j: &str, line: &str) -> bool {
    lock_lines(ws, j)
        .iter()
        .any(|t| present(t, line, 0.5) || present(line, t, 0.5))
}

/// `_lcs(a, b)`.
fn lcs(xs: &[String], ys: &[String]) -> Vec<String> {
    let (alen, blen) = (xs.len(), ys.len());
    let mut memo = vec![vec![0usize; blen + 1]; alen + 1];
    for i in (0..alen).rev() {
        for j in (0..blen).rev() {
            memo[i][j] = if xs[i] == ys[j] {
                memo[i + 1][j + 1] + 1
            } else {
                memo[i + 1][j].max(memo[i][j + 1])
            };
        }
    }
    let (mut out, mut i, mut j) = (Vec::new(), 0, 0);
    while i < alen && j < blen {
        if xs[i] == ys[j] {
            out.push(xs[i].clone());
            i += 1;
            j += 1;
        } else if memo[i + 1][j] >= memo[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn known_or_new(x: &Obj) -> bool {
    matches!(sget(x, "status"), Some("known" | "new"))
}

/// `align(c, ws, set_key)`: one night against the working set's current version.
pub fn align(c: &Corpus, ws: &Obj, set_key: &str) -> Value {
    let ver = current(ws);
    let vbits = bits(ver);
    let plan: Vec<String> = vbits.iter().map(|b| bit_id(b)).collect();
    let Some(s) = c.sets.get(set_key) else {
        return Value::Null;
    };
    let segs: Vec<&Obj> = c
        .segments
        .get(set_key)
        .into_iter()
        .flatten()
        .filter(|x| sget(x, "status") != Some("banter"))
        .collect();
    let mut first_pos: HashMap<String, usize> = HashMap::new();
    for (pos, x) in segs.iter().enumerate() {
        if let Some(j) = x.get("joke").filter(|v| py::truthy(Some(v))).map(py::str)
            && known_or_new(x)
        {
            first_pos.entry(j).or_insert(pos + 1);
        }
    }
    let mut performed: Vec<String> = plan.iter().filter(|j| first_pos.contains_key(*j)).cloned().collect();
    performed.sort_by_key(|j| first_pos[j]);
    let in_order = lcs(&plan, &performed);
    let mut perf_by: HashMap<&str, &Perf> = HashMap::new();
    for j in &plan {
        for p in c.perfs.get(j).into_iter().flatten().filter(|p| p.set == set_key) {
            perf_by.insert(p.joke_id.as_str(), p);
        }
    }
    let mut rows = Vec::new();
    // the Python adds these with `sum()` (compensated since 3.12): collect, then `py::sum`
    let (mut done_s, mut ls, mut wsum) = (Vec::new(), Vec::new(), Vec::new());
    for (i, b) in vbits.iter().enumerate() {
        let j = bit_id(b);
        let p = perf_by.get(j.as_str());
        let mut row = json!({"joke_id": j, "title": c.title(&j), "planned_pos": i + 1, "target_s": get(b, "target_s"),
                             "locked": bit_locked(ws, &j), "performed": p.is_some()});
        if let (Some(p), Value::Object(r)) = (p, &mut row) {
            let sd = p.script_diff.as_object();
            let sdk = |k: &str| sd.and_then(|m| m.get(k)).cloned().unwrap_or(Value::Null);
            let more = json!({
                "performed_pos": first_pos.get(&j), "segments": p.segments, "time_s": p.duration_s,
                "delta_s": r1(p.duration_s - target(b)), "laugh_s": p.laugh_s, "laugh_s_per_min": p.laugh_s_per_min,
                "vs_room": p.vs_room, "ai_rating": p.ai_rating, "max_bars": p.max_bars,
                "time_to_first_laugh_s": p.time_to_first_laugh_s,
                "script_diff": {"on_script_pct": sdk("on_script_pct"), "dropped": sdk("dropped"), "added": sdk("added")},
                "script_diff_basis": if py::truthy(Some(&p.version_unit)) {
                    json!(format!("vs written version {}", py::str(&p.version_unit)))
                } else { Value::Null },
                "in_order": in_order.contains(&j),
                "evidence": c.seg_evidence(&p.main_seg).into_iter().collect::<Vec<_>>(),
            });
            r.extend(more.as_object().cloned().unwrap_or_default());
            done_s.push(p.duration_s);
            ls.push(p.laugh_s);
            wsum.push(p.window_s);
        }
        rows.push(row);
    }
    let mut added = Vec::new();
    let mut added_s = Vec::new();
    for x in &segs {
        let j = x.get("joke").map(py::str);
        if j.as_ref().is_some_and(|j| plan.contains(j)) && known_or_new(x) {
            continue;
        }
        let t = r1((num(x.get("end_ms")).unwrap_or(0.0) - num(x.get("start_ms")).unwrap_or(0.0)) / 1000.0);
        added_s.push(t);
        added.push(
            json!({"idx": get(x, "idx"), "title": get(x, "title"), "status": get(x, "status"),
                          "joke_id": get(x, "joke"), "time_s": t, "laugh_s_per_min": get(x, "laugh_s_per_min"),
                          "vs_room": get(x, "vs_room"), "ai_rating": get(x, "ai_rating")}),
        );
    }
    let summ = s.summary.as_object().cloned().unwrap_or_default();
    let n_done = done_s.len();
    let (ls, wsum) = (py::sum(ls), py::sum(wsum));
    let in_set: HashSet<&String> = performed.iter().collect();
    json!({
        "working_set": get(ws, "id"), "version": get(ver, "v"), "set": set_key, "date": s.date, "venue": s.venue,
        "venue_label": venue_of(&s.venue, &s.date),
        "order": {"planned": plan, "performed": performed, "in_order": in_order,
                  "moved": performed.iter().filter(|j| !in_order.contains(j)).collect::<Vec<_>>(),
                  "same_order": performed == plan.iter().filter(|j| in_set.contains(j)).cloned().collect::<Vec<_>>()},
        "dropped": plan.iter().filter(|j| !first_pos.contains_key(*j)).collect::<Vec<_>>(),
        "added": added,
        "bits": rows,
        "totals": {"planned_target_s": py::sum(vbits.iter().map(|b| target(b))),
                   "performed_planned_s": r1(py::sum(done_s)), "added_s": r1(py::sum(added_s)),
                   "bits_performed": n_done, "bits_planned": plan.len(),
                   "ws_laugh_s_per_min": if wsum == 0.0 { Value::Null } else { json!(round_to(ls / (wsum / 60.0), 2)) },
                   "set_laugh_s_per_min": get(&summ, "laugh_s_per_min"), "room_heat": get(&summ, "room_heat"),
                   "material_s": get(&summ, "material_s")},
    })
}

/// One wording group's effect (`wording_effects`), the fields Today reads.
#[derive(Debug, Clone)]
struct Effect {
    delta_laugh_s: f64,
    delta_vs_room: Option<f64>,
    statement: String,
    low_n: bool,
    example: Value,
    said_in_last: bool,
    weight: f64,
}

fn night_texts(c: &Corpus, p: &Perf) -> Vec<(String, String)> {
    p.seg_ids
        .iter()
        .map(|sid| (sid.clone(), c.text_of(sid, None).unwrap_or_default()))
        .collect()
}

/// `wording_effects(c, jid)["effects"][:cap]`: phrases said on some nights and not others, grouped by the exact
/// nights, with the laughs on the nights with them against the nights without.
fn wording_effects(c: &Corpus, jid: &str, cap: usize) -> Vec<Effect> {
    let ps = c.perfs.get(jid).map_or(&[][..], Vec::as_slice);
    if ps.len() < 2 {
        return Vec::new();
    }
    let texts: Vec<String> = ps
        .iter()
        .map(|p| {
            night_texts(c, p)
                .into_iter()
                .map(|(_, t)| t)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    // vec -> (phrases (is_punch, n words, phrase, seg), seen keys), in first-seen order
    let mut groups: Groups = IndexMap::new();
    for (i, p) in ps.iter().enumerate() {
        let punch: Vec<String> = p
            .punchlines
            .iter()
            .filter(|x| num(x.get("bars")).unwrap_or(0.0) >= 1.0)
            .map(|x| x.get("text").map(py::str).unwrap_or_default())
            .collect();
        for (sid, t) in night_texts(c, p) {
            for ph in phrases(&t) {
                let vec: Vec<bool> = (0..ps.len()).map(|k| k == i || present(&ph, &texts[k], 0.6)).collect();
                if vec.iter().all(|x| *x) || !vec.iter().any(|x| *x) {
                    continue;
                }
                let g = groups.entry(vec).or_default();
                let key = toks(&ph).join(" ");
                if !g.1.insert(key) {
                    continue;
                }
                let is_punch = punch.iter().any(|x| present(x, &ph, 0.7) || present(&ph, x, 0.7));
                let n = ph.split_whitespace().count();
                g.0.push((is_punch, n, ph, sid.clone()));
            }
        }
    }
    let mut effects: Vec<Ranked> = Vec::new();
    for (vec, (mut phs, _)) in groups {
        let w: Vec<&Perf> = ps.iter().zip(&vec).filter(|(_, v)| **v).map(|(p, _)| p).collect();
        let wo: Vec<&Perf> = ps.iter().zip(&vec).filter(|(_, v)| !**v).map(|(p, _)| p).collect();
        let d_ls = mean(w.iter().map(|p| Some(p.laugh_s))).unwrap_or(0.0)
            - mean(wo.iter().map(|p| Some(p.laugh_s))).unwrap_or(0.0);
        let vr = |xs: &[&Perf]| mean(xs.iter().map(|p| p.vs_room.map(|v| v.min(VR_CLIP))));
        let d_vr = match (vr(&w), vr(&wo)) {
            (Some(a), Some(b)) => Some(a - b),
            _ => None,
        };
        phs.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)).then_with(|| a.2.cmp(&b.2)));
        let reps: Vec<(bool, &str, &str)> = phs
            .iter()
            .take(3)
            .filter(|(_, _, ph, sid)| c.ev(sid, ph).is_some())
            .map(|(pn, _, ph, sid)| (*pn, ph.as_str(), sid.as_str()))
            .collect();
        let Some(first) = reps.first() else {
            continue;
        };
        let lo = w.len().min(wo.len());
        let low = lo < LOW_N;
        let sign = if d_ls >= 0.0 { "+" } else { "-" };
        let statement = format!(
            "{sign}{:.1} laugh-s over {} night{} with it vs {} without{}{}",
            d_ls.abs(),
            w.len(),
            if w.len() == 1 { "" } else { "s" },
            wo.len(),
            d_vr.map_or_else(String::new, |d| format!(" (vs room {d:+.2})")),
            if low { " · low n, a hunch not a finding" } else { "" }
        );
        let root = ((lo as f64) / LOW_N as f64).sqrt().min(1.0);
        let weight = round_to(d_vr.unwrap_or(0.0).abs().min(1.0) * root, 3);
        let rank0 = d_vr.unwrap_or(0.0).abs() * (lo as f64).sqrt() + if first.0 { 0.05 } else { 0.0 };
        let quote = first.1.to_string();
        effects.push((
            Effect {
                delta_laugh_s: round_to(d_ls, 1),
                delta_vs_room: d_vr.map(|d| round_to(d, 2)),
                statement,
                low_n: low,
                example: json!({"node": first.2, "quote": first.1, "punchline": first.0}),
                said_in_last: vec.last().copied().unwrap_or(false),
                weight,
            },
            vec,
            (rank0, phs.len()),
            quote,
        ));
    }
    effects.sort_by(|a, b| {
        b.2.0
            .partial_cmp(&a.2.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.2.1.cmp(&a.2.1))
            .then_with(|| a.3.cmp(&b.3))
    });
    let mut seen: HashSet<Vec<bool>> = HashSet::new();
    let mut kept = Vec::new();
    for (e, vec, _, _) in effects {
        // a group and its mirror (said on exactly the other nights) tell one story: keep one
        if seen.contains(&vec.iter().map(|x| !x).collect::<Vec<_>>()) {
            continue;
        }
        seen.insert(vec);
        kept.push(e);
    }
    kept.truncate(cap);
    kept
}

/// `bit_history(c, jid)`: the fields Today reads.
struct History {
    joke_id: String,
    title: String,
    stability: Option<f64>,
    effects: Vec<Effect>,
}

fn bit_history(c: &Corpus, jid: &str) -> History {
    let ps = c.perfs.get(jid).map_or(&[][..], Vec::as_slice);
    let k = cv(ps.iter().map(|p| p.laugh_s_per_min));
    History {
        joke_id: jid.to_string(),
        title: c.title(jid),
        stability: k.map(|k| round_to((1.0 - k).max(0.0), 2)),
        effects: wording_effects(c, jid, 5),
    }
}

fn note(j: &str, title: &str, kind: &str, score: f64, text: &str, evidence: &Value, low_n: bool) -> Value {
    json!({"joke_id": j, "title": title, "kind": kind, "score": score, "text": text, "evidence": evidence,
           "source": "inferred", "low_n": low_n})
}

/// `try_tonight(c, ws, cap=6, hist)`: what to try tonight, best first.
fn try_tonight(c: &Corpus, ws: &Obj, hist: &[History], cap: usize) -> Vec<Value> {
    let last = c.set_keys().last().map(|k| align(c, ws, k));
    let mut by_bit: HashMap<String, &Obj> = HashMap::new();
    if let Some(Value::Object(l)) = &last {
        for b in l
            .get("bits")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
        {
            by_bit.insert(bit_id(b), b);
        }
    }
    let empty = Obj::new();
    let mut notes = Vec::new();
    for h in hist {
        let j = h.joke_id.as_str();
        if bit_locked(ws, j) {
            continue; // a locked bit is perfected: no notes
        }
        let lb = by_bit.get(j).copied().unwrap_or(&empty);
        let title = h.title.as_str();
        for e in &h.effects {
            let quote = e.example.get("quote").and_then(Value::as_str).unwrap_or("");
            if is_locked_line(ws, j, quote) {
                continue;
            }
            let dvr = e.delta_vs_room.unwrap_or(0.0);
            if e.delta_laugh_s > 0.0 && dvr > 0.0 && !e.said_in_last {
                let text = format!("{title}: put back \"{quote}\". {}.", e.statement);
                let score = round_to(0.3 + 0.6 * e.weight, 3);
                notes.push(note(j, title, "put_back", score, &text, &json!([e.example]), e.low_n));
                break;
            }
            if e.delta_laugh_s < 0.0 && dvr < 0.0 && e.said_in_last {
                let text = format!("{title}: try it without \"{quote}\". {}.", e.statement);
                let score = round_to(0.25 + 0.6 * e.weight, 3);
                notes.push(note(
                    j,
                    title,
                    "try_without",
                    score,
                    &text,
                    &json!([e.example]),
                    e.low_n,
                ));
                break;
            }
        }
        let performed = py::truthy(lb.get("performed"));
        let (time_s, target_s) = (
            num(lb.get("time_s")).unwrap_or(0.0),
            num(lb.get("target_s")).unwrap_or(0.0),
        );
        if let Some(Value::Object(l)) = &last
            && !performed
        {
            let text = format!(
                "{title}: you skipped it last night ({}); it's #{} in v{}.",
                l.get("venue_label").map(py::str).unwrap_or_default(),
                lb.get("planned_pos").map_or_else(|| "None".to_string(), py::str),
                l.get("version").map(py::str).unwrap_or_default()
            );
            notes.push(note(j, title, "dropped", 0.5, &text, &json!([]), false));
        } else if performed && target_s != 0.0 && time_s > 1.25 * target_s {
            let score = round_to((0.3 + (time_s / target_s - 1.0) * 0.3).min(0.9), 3);
            let text = format!(
                "{title}: ran {} last night vs a {} target. Trim the setup.",
                mmss(time_s),
                mmss(target_s)
            );
            notes.push(note(j, title, "over_time", score, &text, &get(lb, "evidence"), false));
        }
        let ttfl = num(lb.get("time_to_first_laugh_s"));
        let bestt: Vec<f64> = c
            .perfs
            .get(j)
            .into_iter()
            .flatten()
            .filter_map(|p| num(Some(&p.time_to_first_laugh_s)))
            .collect();
        let fastest = bestt.iter().copied().fold(f64::INFINITY, f64::min);
        if let Some(t) = ttfl
            && !bestt.is_empty()
            && t > 20.0
            && t > fastest + 10.0
        {
            let text = format!(
                "{title}: the first laugh came {t:.0}s in last night; your fastest night got one in {fastest:.0}s."
            );
            notes.push(note(
                j,
                title,
                "slow_start",
                0.35,
                &text,
                &get(lb, "evidence"),
                bestt.len() < LOW_N,
            ));
        }
    }
    let key = |n: &Value| {
        (
            num(n.get("score")).unwrap_or(0.0),
            n.get("joke_id").map(py::str).unwrap_or_default(),
            n.get("kind").map(py::str).unwrap_or_default(),
        )
    };
    notes.sort_by(|a, b| {
        let (ka, kb) = (key(a), key(b));
        kb.0.partial_cmp(&ka.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| ka.1.cmp(&kb.1))
            .then_with(|| ka.2.cmp(&kb.2))
    });
    notes.truncate(cap);
    notes
}

/// `perfected(c, ws, hist)["perfected"]`: half the locked share (by target time), half the stability.
fn perfected(c: &Corpus, ws: &Obj, hist: &[History]) -> f64 {
    let hist: HashMap<&str, &History> = hist.iter().map(|h| (h.joke_id.as_str(), h)).collect();
    let vbits = bits(current(ws));
    let total = py::sum(vbits.iter().map(|b| target(b)));
    let tot = if total == 0.0 { 1.0 } else { total };
    let (mut locked_s, mut stab_w, mut stab_sum) = (0.0, 0.0, 0.0);
    for b in &vbits {
        let j = bit_id(b);
        let lines = lock_lines(ws, &j);
        let share = if bit_locked(ws, &j) {
            1.0
        } else if !lines.is_empty() {
            let words: usize = c.perfs.get(&j).and_then(|ps| ps.last()).map_or(0, |p| {
                p.seg_ids
                    .iter()
                    .map(|s| toks(&c.text_of(s, None).unwrap_or_default()).len())
                    .sum()
            });
            let lw: usize = lines.iter().map(|t| toks(t).len()).sum();
            if words == 0 {
                0.0
            } else {
                (lw as f64 / words as f64).min(1.0)
            }
        } else {
            0.0
        };
        locked_s += share * target(b);
        if let Some(st) = hist.get(j.as_str()).and_then(|h| h.stability) {
            stab_w += target(b);
            stab_sum += st * target(b);
        }
    }
    let stability = if stab_w == 0.0 { 0.0 } else { stab_sum / stab_w };
    round_to(0.5 * (locked_s / tot) + 0.5 * stability, 2)
}

/// `status(c, ws, notes)`: the working set's line on Today.
fn status(c: &Corpus, ws: &Obj, n_notes: usize) -> Value {
    let al: Vec<Value> = c
        .set_keys()
        .iter()
        .map(|k| align(c, ws, k))
        .filter(|a| num(a.pointer("/totals/bits_performed")).unwrap_or(0.0) != 0.0)
        .collect();
    let last = al.last();
    let prev = (al.len() > 1).then(|| &al[al.len() - 2]);
    let rate = |a: Option<&Value>| a.and_then(|a| num(a.pointer("/totals/ws_laugh_s_per_min")));
    let (lr, pr) = (rate(last), rate(prev));
    let arrow = match (lr, pr) {
        (Some(l), Some(p)) if l > p * 1.05 => Some("up"),
        (Some(l), Some(p)) if l < p * 0.95 => Some("down"),
        (Some(_), Some(_)) => Some("flat"),
        _ => None,
    };
    let ver = current(ws);
    let target_s = py::sum(bits(ver).iter().map(|b| target(b)));
    let mut line = format!(
        "{} v{} · {}",
        ws.get("title").map(py::str).unwrap_or_default(),
        ver.get("v").map(py::str).unwrap_or_default(),
        mmss(target_s)
    );
    if let Some(l) = lr {
        let a = match arrow {
            Some("up") => "▲",
            Some("down") => "▼",
            _ => "=",
        };
        let _ = write!(line, " · last night {l:.1} laugh-s/min {a}");
    }
    let _ = write!(line, " · {} things to try tonight", n_notes.min(3));
    let last_night = last.map_or(Value::Null, |a| {
        json!({"set": a["set"], "date": a["date"], "venue": a["venue_label"], "ws_laugh_s_per_min": lr,
               "bits_performed": a["totals"]["bits_performed"], "bits_planned": a["totals"]["bits_planned"]})
    });
    json!({"working_set": get(ws, "id"), "title": get(ws, "title"), "version": get(ver, "v"), "target_s": target_s,
           "last_night": last_night, "vs_previous": arrow, "line": line})
}

fn joke_ref(c: &Corpus, v: &Value) -> Value {
    if !py::truthy(Some(v)) {
        return v.clone();
    }
    let j = c.map_joke(&py::str(v));
    json!({"joke_id": j, "title": c.title(&j)})
}

/// `latest_set(c)`.
fn latest_set(c: &Corpus) -> Value {
    let Some(k) = c.set_keys().last().cloned() else {
        return Value::Null;
    };
    let s = &c.sets[&k];
    let sm = s.summary.as_object().cloned().unwrap_or_default();
    json!({"set": k, "date": s.date, "venue": s.venue, "venue_label": venue_of(&s.venue, &s.date), "title": s.title,
           "material": mmss_opt(num(sm.get("material_s"))), "laugh_s_per_min": get(&sm, "laugh_s_per_min"),
           "room_heat": get(&sm, "room_heat"), "pct_laughing": get(&sm, "pct_laughing"),
           "jokes_counted": get(&sm, "jokes_counted"), "new_material": get(&sm, "new_material"),
           "best": joke_ref(c, &get(&sm, "best")), "weakest": joke_ref(c, &get(&sm, "weakest"))})
}

/// `new_material(c)`: the new moments of the latest set.
fn new_material(c: &Corpus) -> Vec<Value> {
    let Some(k) = c.set_keys().last().cloned() else {
        return Vec::new();
    };
    c.segments
        .get(&k)
        .into_iter()
        .flatten()
        .filter(|s| sget(s, "status") == Some("new"))
        .map(|s| {
            let similar: Vec<String> = s
                .get("similar")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|x| c.map_joke(&x.get("joke_id").map(py::str).unwrap_or_default()))
                .take(3)
                .collect();
            let ev = sget(s, "seg_id").and_then(|id| c.seg_evidence_n(id, 12));
            json!({"idx": get(s, "idx"), "title": get(s, "title"), "joke_id": get(s, "joke"),
                   "ai_rating": get(s, "ai_rating"), "laugh_s_per_min": get(s, "laugh_s_per_min"),
                   "vs_room": get(s, "vs_room"), "similar": similar, "evidence": ev.into_iter().collect::<Vec<_>>()})
        })
        .collect()
}

fn rank(key: &str, day: &str) -> String {
    hex::encode(sha1::Sha1::digest(format!("{day}|{key}").as_bytes()))
}

fn list<'a>(v: &'a Value, a: &str, b: &str) -> Vec<&'a Value> {
    v.get(a)
        .and_then(|x| x.get(b))
        .and_then(Value::as_array)
        .map(|x| x.iter().collect())
        .unwrap_or_default()
}

fn joined(v: Option<&Value>, n: usize) -> String {
    v.and_then(Value::as_array)
        .map(|xs| xs.iter().take(n).map(py::str).collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}

/// `serendipity(c, day, n=3)`: two gems (never taped, built like one that works) then orphans, rotating daily.
fn serendipity(c: &Corpus, day: &str, n: usize) -> Vec<Value> {
    let mut gems = list(&c.insights, "gems", "gems");
    gems.sort_by_cached_key(|g| rank(&g.get("joke").map(py::str).unwrap_or_default(), day));
    let mut orphans = list(&c.insights, "orphans", "active_orphans");
    orphans.sort_by_cached_key(|o| rank(&py::str(o), day));
    let mut picks: Vec<Value> = Vec::new();
    for g in gems {
        if picks.len() >= 2 {
            break;
        }
        let jid = c.map_joke(&g.get("joke").map(py::str).unwrap_or_default());
        let like = g.get("like").filter(|v| !v.is_null()).map(|v| c.map_joke(&py::str(v)));
        if !c.is_joke(&jid) || like.as_deref() == Some(jid.as_str()) || c.perfs.get(&jid).is_some_and(|p| !p.is_empty())
        {
            continue; // merged into its reference joke in v1, or on tape after all
        }
        let u = c.lead_unit(&jid);
        let first = u.as_ref().map(|u| first_line(c.node_text(u), 160)).unwrap_or_default();
        let topics = g.get("shared_topics").filter(|v| py::truthy(Some(v)));
        let why = format!(
            "Never on tape, but built like {}: shares {}{}",
            g.get("like_title").map_or_else(|| "None".to_string(), py::str),
            joined(g.get("shared_shape"), 3),
            topics.map_or_else(String::new, |t| format!(" and {}", joined(Some(t), 2)))
        );
        let evidence: Vec<Value> = match &u {
            Some(u) if !first.is_empty() => c.ev(u, &first).into_iter().collect(),
            _ => Vec::new(),
        };
        picks.push(
            json!({"kind": "gem", "joke_id": jid, "title": c.title(&jid), "stage": c.stage(&jid).0, "why": why,
                          "score": g.get("score").cloned().unwrap_or(Value::Null), "source": "proposed",
                          "evidence": evidence}),
        );
    }
    for o in orphans {
        if picks.len() >= n {
            break;
        }
        let jid = c.map_joke(&py::str(o));
        if !c.is_joke(&jid) || picks.iter().any(|p| p["joke_id"] == jid.as_str()) {
            continue;
        }
        let Some(u) = c.lead_unit(&jid) else {
            continue;
        };
        let line = first_line(c.node_text(&u), 160);
        let written = c
            .joke(&jid)
            .and_then(|n| n.get("first_seen"))
            .filter(|v| py::truthy(Some(v)))
            .map_or_else(|| "?".to_string(), py::str);
        picks.push(json!({"kind": "orphan", "joke_id": jid, "title": c.title(&jid), "stage": c.stage(&jid).0,
                          "why": format!("An orphan: no link to anything else and never on tape (written {written}). \
                                          Give it a home on a storyboard or retire it."),
                          "score": null, "source": "inferred", "evidence": c.ev(&u, &line).into_iter().collect::<Vec<_>>()}));
    }
    picks.truncate(n);
    picks
}

/// `minutes(c)`: minutes of material by stage, measured from tape where possible.
fn minutes(c: &Corpus) -> Value {
    let (mut by, mut measured, mut n) = ([0.0f64; 2], [0.0f64; 2], [0usize; 2]);
    for jid in &c.jokes {
        let i = match c.stage(jid).0.as_str() {
            Some("working") => 0,
            Some("testing") => 1,
            _ => continue,
        };
        let (d, basis) = c.duration(jid);
        by[i] += d;
        n[i] += 1;
        if basis.starts_with("tape") {
            measured[i] += d;
        }
    }
    let proven = by[0] / 60.0;
    let both = (by[0] + by[1]) / 60.0;
    #[expect(clippy::cast_possible_truncation, reason = "a percentage")]
    let pct = |x: f64, g: f64| ((100.0 * x / g).round_ties_even() as i64).min(100);
    let toward: Vec<Value> = [5.0, 15.0, 30.0, 60.0]
        .iter()
        .map(|g| json!({"goal_min": *g as i64, "working_pct": pct(proven, *g), "with_testing_pct": pct(both, *g)}))
        .collect();
    json!({"working_min": r1(proven), "testing_min": r1(by[1] / 60.0), "working_plus_testing_min": r1(both),
           "measured_share": r1(100.0 * (measured[0] + measured[1]) / (by[0] + by[1]).max(1.0)),
           "toward": toward, "n_working": n[0], "n_testing": n[1],
           "basis": "stage from inferred.json (AI) unless you changed it; minutes = median taped duration, \
                     else written words at 157 wpm"})
}

/// `today.build(c, ws, day)`.
pub fn build(c: &Corpus, ws: Option<&Obj>, day: &str) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("date".into(), json!(day));
    out.insert("latest_set".into(), latest_set(c));
    out.insert("working_set".into(), Value::Null);
    out.insert("try_tonight".into(), json!([]));
    out.insert("serendipity".into(), json!(serendipity(c, day, 3)));
    out.insert("new_material".into(), json!(new_material(c)));
    out.insert("minutes".into(), minutes(c));
    if let Some(ws) = ws.filter(|w| !w.is_empty()) {
        let hist: Vec<History> = bits(current(ws)).iter().map(|b| bit_history(c, &bit_id(b))).collect();
        let notes = try_tonight(c, ws, &hist, 6);
        let mut st = status(c, ws, notes.len());
        if let Value::Object(m) = &mut st {
            m.insert("perfected".into(), json!(perfected(c, ws, &hist)));
        }
        out.insert("working_set".into(), st);
        out.insert(
            "try_tonight".into(),
            json!(notes.into_iter().take(3).collect::<Vec<_>>()),
        );
    }
    if let Some(tl) = c.his_lists.get("list:tonight").filter(|tl| !tl.is_empty()) {
        let items: Vec<Value> = tl
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|x| {
                let j = c.map_joke(&x.get("joke").map(py::str).unwrap_or_default());
                json!({"joke_id": j, "title": c.title(&j), "done": x.get("done").cloned().unwrap_or(json!(false))})
            })
            .collect();
        out.insert("his_tonight_list".into(), json!(items));
    }
    out
}

/// `creative_api.latest_ws(stores)`: the working set touched last (not deleted).
pub fn latest_ws(items: &Map<String, Value>) -> Option<&Obj> {
    items
        .values()
        .filter_map(Value::as_object)
        .filter(|w| !py::truthy(w.get("deleted")))
        // Python's `max` keeps the first of equals: the last of the reversed order
        .rev()
        .max_by(|a, b| {
            let k = |w: &Obj| (w.get("updated_at").map(py::str), w.get("id").map(py::str));
            k(a).cmp(&k(b))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_python_helpers() {
        assert_eq!(cv([Some(1.0), Some(3.0), None]), Some(2.0f64.sqrt() / 2.0));
        assert_eq!(cv([Some(1.0)]), None);
        assert_eq!(cv([Some(-1.0), Some(1.0)]), None);
        assert_eq!(mean([None, Some(2.0), Some(4.0)]), Some(3.0));
        assert_eq!(mmss_opt(None), "-");
        assert_eq!(first_line("hi\n  three words here  \nx", 160), "three words here");
        assert_eq!(first_line("one two", 160), "one two");
        let long = "word ".repeat(50);
        assert_eq!(first_line(&long, 30), "word word word word word word");
        let a: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| (*s).to_string()).collect();
        let b: Vec<String> = ["b", "a", "d"].iter().map(|s| (*s).to_string()).collect();
        // ties go to skipping in `a` first, as the Python's `>=` does
        assert_eq!(lcs(&a, &b), ["b", "d"]);
    }
}
