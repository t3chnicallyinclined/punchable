//! One bit's connections (`creative/connections.py` `joke_connections`): ranked, capped groups by creative intent
//! (what could come next, lines that could tag it, the same world, callbacks, the same shape, the same topic from
//! another angle), each item with a score, a one-line why, verbatim evidence and its source; the stage trend across
//! nights and the rule-based "land it harder" notes. And the joke page as a map (`creative/notebook.py` `joke_map`).
//!
//! The "could tag it" query is the reveal's vector against every other unit's: numpy's `np.dot` on float32 rows goes
//! through BLAS, whose summation order is the kernel's, so a score here equals the Python's to float32 precision
//! (the z-score that gates a hit is computed from them with numpy's own pairwise mean and standard deviation).

use std::collections::{BTreeSet, HashMap, HashSet};

use indexmap::IndexMap;
use pk_core::py;
use serde_json::{Map, Value, json};

use crate::ask::words::short_title;
use crate::corpus::{Corpus, median, num, round_to, s, venue_short};
use crate::notebook::{clip, sentences, texts};
use crate::setlist::{best_next, card, plain_best_next};
use crate::story::Story;
use crate::today::{first_line, mean};

/// `GROUPS`: (key, title).
pub const GROUPS: [(&str, &str); 6] = [
    ("next", "What could come next"),
    ("tags", "Lines that could tag it"),
    ("same_world", "Same world (persona / premise)"),
    ("callbacks", "Callbacks"),
    ("same_shape", "Same shape"),
    ("contrasts", "Same topic, other angle"),
];
/// `notebook.GROUPS`: the joke map's groups.
pub const MAP_GROUPS: [(&str, &str); 5] = [
    ("next", "Could come next"),
    ("tags", "Could tag it"),
    ("same_world", "Same world"),
    ("callbacks", "Callbacks"),
    ("versions", "Older versions"),
];

fn src_rank(s: &str) -> i64 {
    match s {
        "his" => 2,
        "inferred" => 1,
        _ => 0,
    }
}

fn stage_w(c: &Corpus, j: &str) -> f64 {
    match c.stage(j).0.as_str() {
        Some("working" | "testing") => 1.0,
        Some("to_try") => 0.95,
        Some("retired") => 0.8,
        _ => 0.9,
    }
}

/// One group's items being built (`_Group`), in insertion order.
#[derive(Default)]
struct Group {
    items: IndexMap<String, Item>,
}

struct Item {
    score: f64,
    why: Vec<String>,
    source: &'static str,
    basis: Vec<&'static str>,
    evidence: Vec<Value>,
    extra: Map<String, Value>,
}

impl Group {
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        key: &str,
        score: f64,
        why: &str,
        source: &'static str,
        evidence: Vec<Value>,
        basis: &'static str,
        extra: impl FnOnce() -> Map<String, Value>,
    ) {
        let it = self.items.entry(key.to_string()).or_insert_with(|| Item {
            score: 0.0,
            why: Vec::new(),
            source,
            basis: Vec::new(),
            evidence: Vec::new(),
            extra: extra(),
        });
        let bonus = if !it.basis.is_empty() && !it.basis.contains(&basis) {
            0.05
        } else {
            0.0
        };
        it.score = it.score.max(score) + bonus;
        if !why.is_empty() && !it.why.iter().any(|w| w == why) {
            it.why.push(why.to_string());
        }
        if src_rank(source) > src_rank(it.source) {
            it.source = source;
        }
        if !it.basis.contains(&basis) {
            it.basis.push(basis);
        }
        for e in evidence {
            if py::truthy(Some(&e)) && !it.evidence.contains(&e) {
                it.evidence.push(e);
            }
        }
    }

    /// `ranked(c, cap)`.
    fn ranked(&self, c: &Corpus, cap: usize) -> Vec<Value> {
        let mut out: Vec<(f64, i64, &String, Value)> = self
            .items
            .iter()
            .map(|(k, it)| {
                let score = round_to(it.score.min(1.0), 3);
                let mut m = Map::new();
                m.insert("id".into(), json!(k));
                m.insert("score".into(), json!(score));
                m.insert(
                    "why".into(),
                    json!(it.why.join("; ").chars().take(240).collect::<String>()),
                );
                m.insert("source".into(), json!(it.source));
                m.insert("basis".into(), json!(it.basis));
                m.insert("evidence".into(), json!(it.evidence.iter().take(2).collect::<Vec<_>>()));
                for (ek, ev) in &it.extra {
                    m.insert(ek.clone(), ev.clone());
                }
                if let Some(j) = it
                    .extra
                    .get("joke_id")
                    .and_then(Value::as_str)
                    .filter(|j| !j.is_empty())
                {
                    m.insert("title".into(), json!(c.title(j)));
                    m.insert("stage".into(), c.stage(j).0);
                }
                (score, src_rank(it.source), k, Value::Object(m))
            })
            .collect();
        out.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)).then_with(|| a.2.cmp(b.2)));
        out.into_iter().take(cap).map(|x| x.3).collect()
    }
}

fn obj(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> Map<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

fn names(c: &Corpus, ls: &[&String]) -> Vec<Value> {
    ls.iter().map(|l| c.label_field(l, "name")).collect()
}

fn names_str(c: &Corpus, ls: &[&String]) -> String {
    ls.iter()
        .map(|l| py::str(&c.label_field(l, "name")))
        .collect::<Vec<_>>()
        .join(", ")
}

fn label_ev(c: &Corpus, jid: &str, l: &str) -> Vec<Value> {
    c.joke_labels
        .get(jid)
        .and_then(|ls| ls.get(l))
        .map(|(_, e)| c.edge_evidence(&c.edges[*e], 1))
        .unwrap_or_default()
}

/// `_idf_overlap(c, a, b, kinds, min_w)`: the idf-weighted share of the labels both hold, and those shared (rarest
/// first).
fn idf_overlap(c: &Corpus, a: &str, b: &str, kinds: &[&str], min_w: f64) -> (f64, Vec<String>) {
    let la = c.labels_of(a, Some(kinds), min_w);
    let lb = c.labels_of(b, Some(kinds), min_w);
    if la.is_empty() || lb.is_empty() {
        return (0.0, Vec::new());
    }
    let idf = |l: &str| c.idf.get(l).copied().unwrap_or(1.0);
    let mut shared: Vec<String> = la.keys().filter(|l| lb.contains_key(*l)).cloned().collect();
    shared.sort_by(|x, y| idf(y).total_cmp(&idf(x)).then_with(|| x.cmp(y)));
    let num = py::sum(shared.iter().map(|l| idf(l)));
    let union: BTreeSet<&String> = la.keys().chain(lb.keys()).collect();
    let den = py::sum(union.into_iter().map(|l| idf(l)));
    (if den == 0.0 { 0.0 } else { num / den }, shared)
}

/// numpy's pairwise sum of a contiguous float64 run (what `ndarray.mean` / `std` reduce with).
fn pairwise_sum(xs: &[f64]) -> f64 {
    let n = xs.len();
    if n < 8 {
        return xs.iter().fold(-0.0_f64, |a, x| a + x);
    }
    if n <= 128 {
        let mut r = [xs[0], xs[1], xs[2], xs[3], xs[4], xs[5], xs[6], xs[7]];
        let mut i = 8;
        while i < n - (n % 8) {
            for (k, rk) in r.iter_mut().enumerate() {
                *rk += xs[i + k];
            }
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        for x in &xs[i..] {
            res += x;
        }
        return res;
    }
    let mut n2 = n / 2;
    n2 -= n2 % 8;
    pairwise_sum(&xs[..n2]) + pairwise_sum(&xs[n2..])
}

/// `np.array(xs).mean()` and `.std()` (population).
fn mean_std(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let mu = pairwise_sum(xs) / n;
    let sq: Vec<f64> = xs.iter().map(|x| (x - mu) * (x - mu)).collect();
    (mu, (pairwise_sum(&sq) / n).sqrt())
}

/// The float32 dot product of two rows, as a float.
fn dot32(a: &[f32], b: &[f32]) -> f64 {
    f64::from(a.iter().zip(b).fold(0.0_f32, |acc, (x, y)| acc + x * y))
}

/// The "could tag it" items: his tag edges into the bit, then lines whose reveal lands on the same note as its
/// reveal.
fn tags_group(c: &Corpus, jid: &str, cap: usize, group: &mut Group) {
    let lead = c.lead_unit(jid);
    let Some(qvec) = lead.as_deref().and_then(|u| c.vectors.reveal(u)) else {
        return;
    };
    let reveal = py::strip(
        c.joke(jid)
            .and_then(|n| n.get("anatomy"))
            .and_then(|a| a.get("reveal"))
            .and_then(Value::as_str)
            .unwrap_or(""),
    )
    .to_string();
    let mut scored: Vec<(f64, &String)> = Vec::new();
    for (u, owner) in &c.unit_joke {
        let kind = c.nodes.get(u).and_then(|n| s(n, "kind"));
        if owner == jid || !matches!(kind, Some("tag" | "topper" | "setup+punch" | "callback")) {
            continue;
        }
        if let Some(v) = c.vectors.reveal(u) {
            scored.push((dot32(qvec, v), u));
        }
    }
    if scored.len() < 10 {
        return;
    }
    let vals: Vec<f64> = scored.iter().map(|x| x.0).collect();
    let (mu, sd) = mean_std(&vals);
    let sd = if sd == 0.0 { 1.0 } else { sd };
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    for (sc, u) in scored.into_iter().take(cap * 2) {
        let z = (sc - mu) / sd;
        if z < 2.5 {
            break;
        }
        let un = &c.nodes[u];
        let text = s(un, "text").unwrap_or("");
        let rv = py::strip(
            un.get("anatomy")
                .and_then(|a| a.get("reveal"))
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        let line = if rv.split_whitespace().count() >= 3 && text.contains(rv) {
            rv.to_string()
        } else {
            first_line(text, 160)
        };
        let Some(ev) = c.ev(u, &line) else { continue };
        let owner = c.unit_joke.get(u).cloned();
        let ot = owner.as_deref().map_or_else(|| "None".to_string(), |o| c.title(o));
        let rq = if reveal.is_empty() {
            String::new()
        } else {
            format!(" '{reveal}'")
        };
        let why = format!("Lands on the same note as your reveal{rq} ({z:.1} sd above typical); from '{ot}'");
        let weight = owner.as_deref().map_or(0.9, |o| stage_w(c, o));
        let score = (0.45 + 0.08 * (z - 2.5)).min(1.0) * weight;
        group.add(u, score, &why, "proposed", vec![ev], "reveal_query", || {
            obj([("unit_id", json!(u)), ("line", json!(line)), ("joke_id", json!(owner))])
        });
    }
}

#[allow(clippy::too_many_lines)]
fn groups(story: &Story<'_>, c: &Corpus, jid: &str, cap: usize) -> IndexMap<&'static str, Group> {
    let mut gs: IndexMap<&'static str, Group> = GROUPS.iter().map(|(k, _)| (*k, Group::default())).collect();
    let is_joke = |j: &str| c.is_joke(j);
    // explicit edges
    for &i in c.edges_by_node.get(jid).map_or(&[][..], Vec::as_slice) {
        let e = &c.edges[i];
        let Some(k @ ("segue" | "same_premise" | "callback_to" | "tag_of" | "same_shape")) = s(e, "kind") else {
            continue;
        };
        let (src, dst) = (s(e, "src").unwrap_or(""), s(e, "dst").unwrap_or(""));
        let source = c.source_of(e);
        let ev = c.edge_evidence(e, 2);
        let conf = num(e.get("confidence").filter(|v| py::truthy(Some(v)))).unwrap_or(0.5);
        let reason = e
            .get("reason")
            .filter(|v| py::truthy(Some(v)))
            .map(py::str)
            .unwrap_or_default();
        let reason = py::strip(&reason);
        let eid = e.get("id").cloned().unwrap_or(Value::Null);
        let other = if src == jid { dst } else { src };
        match k {
            "segue" if src == jid && is_joke(dst) => {
                gs["next"].add(dst, conf, reason, source, ev, "segue", || {
                    obj([("joke_id", json!(dst)), ("edge", eid)])
                });
            }
            "same_premise" if is_joke(other) => {
                gs["same_world"].add(other, conf, reason, source, ev, "same_premise", || {
                    obj([("joke_id", json!(other)), ("edge", eid)])
                });
            }
            "callback_to" if is_joke(other) => {
                let dir = if src == jid {
                    "this_calls_back"
                } else {
                    "calls_back_this"
                };
                gs["callbacks"].add(other, conf, reason, source, ev, "callback_to", || {
                    obj([("joke_id", json!(other)), ("edge", eid), ("direction", json!(dir))])
                });
            }
            "same_shape" if is_joke(other) => {
                gs["same_shape"].add(other, conf, reason, source, ev, "same_shape", || {
                    obj([("joke_id", json!(other)), ("edge", eid)])
                });
            }
            "tag_of" if dst == jid && src.starts_with("unit:") => {
                let owner = c.unit_joke.get(src).filter(|o| !o.is_empty() && *o != jid).cloned();
                let q = ev.first().and_then(|x| x.get("quote")).cloned().unwrap_or(Value::Null);
                gs["tags"].add(src, conf, reason, source, ev, "tag_of", || {
                    obj([
                        ("unit_id", json!(src)),
                        ("line", q),
                        ("joke_id", json!(owner)),
                        ("edge", eid),
                    ])
                });
            }
            _ => {}
        }
    }
    // proven back to back
    {
        for ((a, b), rows) in &c.back_to_back {
            if a == jid && is_joke(b) {
                let n = rows.len();
                let ev: Vec<Value> = rows[n.saturating_sub(2)..]
                    .iter()
                    .map(|r| c.seg_evidence(&r.b_seg).unwrap_or(Value::Null))
                    .collect();
                let nights: Vec<String> = rows
                    .iter()
                    .map(|r| venue_short(r.venue.as_str(), Some(&r.date)))
                    .collect();
                let why = format!("Done right after it {n}x on tape ({})", nights.join(", "));
                gs["next"].add(
                    b,
                    (0.55 + 0.15 * n as f64).min(1.0),
                    &why,
                    "inferred",
                    ev,
                    "back_to_back",
                    || obj([("joke_id", json!(b)), ("nights", json!(n))]),
                );
            }
        }
    }
    tags_group(c, jid, cap, &mut gs["tags"]);
    // same world: persona + specific topics
    let my_personas = c.labels_of(jid, Some(&["persona"]), 0.5);
    let my_topics = c.labels_of(jid, Some(&["topic"]), 0.55);
    if !my_personas.is_empty() || !my_topics.is_empty() {
        for o in c.jokes.iter().filter(|o| *o != jid) {
            let theirs = c.labels_of(o, Some(&["persona"]), 0.5);
            let mut sp: Vec<&String> = my_personas.keys().filter(|l| theirs.contains_key(*l)).collect();
            sp.sort();
            let (ov_t, st) = idf_overlap(c, jid, o, &["topic"], 0.55);
            if sp.is_empty() && ov_t < 0.34 {
                continue;
            }
            let score = (if sp.is_empty() { 0.0 } else { 0.55 } + 0.45 * ov_t) * stage_w(c, o);
            let st3: Vec<&String> = st.iter().take(3).collect();
            let mut parts = Vec::new();
            if !sp.is_empty() {
                parts.push(format!("same persona: {}", names_str(c, &sp)));
            }
            if !st.is_empty() {
                parts.push(format!("shared topics: {}", names_str(c, &st3)));
            }
            let both: Vec<&String> = sp.iter().copied().chain(st.iter()).collect();
            let mut ev = Vec::new();
            for l in both.iter().take(2) {
                ev.extend(label_ev(c, o, l));
            }
            ev.truncate(2);
            let src = if both.iter().any(|l| {
                c.joke_labels
                    .get(o)
                    .and_then(|ls| ls.get(*l))
                    .is_some_and(|(_, e)| c.source_of(&c.edges[*e]) != "proposed")
            }) {
                "inferred"
            } else {
                "proposed"
            };
            gs["same_world"].add(o, score, &parts.join("; "), src, ev, "labels", || {
                obj([
                    ("joke_id", json!(o)),
                    ("personas", json!(names(c, &sp))),
                    ("topics", json!(names(c, &st3))),
                ])
            });
        }
    }
    // same chunk (the Insights communities)
    for ch in c
        .insights
        .get("chunks")
        .and_then(|x| x.get("chunks"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let members: Vec<String> = ch
            .get("jokes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|x| c.map_joke(&py::str(x)))
            .collect();
        if members.iter().any(|m| m == jid) {
            let why = format!(
                "same chunk in your Insights: '{}'",
                py::str(ch.get("name").unwrap_or(&Value::Null))
            );
            for o in &members {
                if !o.is_empty() && o != jid && is_joke(o) {
                    gs["same_world"].add(o, 0.42 * stage_w(c, o), &why, "proposed", Vec::new(), "chunk", || {
                        obj([("joke_id", json!(o))])
                    });
                }
            }
        }
    }
    // same shape: devices / structures + text
    for o in c.jokes.iter().filter(|o| *o != jid) {
        let (ov, shared) = idf_overlap(c, jid, o, &["device", "structure"], 0.55);
        if ov < 0.3 || shared.len() < 2 {
            continue;
        }
        let cs = story.cos(jid, o);
        let score = (0.7 * ov + 0.3 * cs.unwrap_or(0.5)) * stage_w(c, o);
        let s3: Vec<&String> = shared.iter().take(3).collect();
        let mut ev = Vec::new();
        for l in shared.iter().take(2) {
            ev.extend(label_ev(c, o, l));
        }
        ev.truncate(2);
        gs["same_shape"].add(
            o,
            score,
            &format!("same moves: {}", names_str(c, &s3)),
            "proposed",
            ev,
            "labels",
            || obj([("joke_id", json!(o)), ("shared", json!(names(c, &s3)))]),
        );
    }
    // contrasts: the same topic from another angle
    let att = |j: &str| -> BTreeSet<String> { c.labels_of(j, Some(&["attitude"]), 0.5).into_keys().collect() };
    let my_att = att(jid);
    let my_moves: HashSet<String> = c.devices(jid, 3, 0.55).into_iter().collect();
    for o in c.jokes.iter().filter(|o| *o != jid) {
        let (ov_t, st) = idf_overlap(c, jid, o, &["topic"], 0.55);
        if st.is_empty() || ov_t < 0.25 {
            continue;
        }
        let o_att = att(o);
        let o_moves: BTreeSet<String> = c.devices(o, 3, 0.55).into_iter().collect();
        if o_moves.iter().any(|m| my_moves.contains(m)) {
            continue;
        }
        let cs = story.cos(jid, o);
        let flip = !my_att.is_empty() && !o_att.is_empty() && my_att.is_disjoint(&o_att);
        let score = (ov_t * (1.0 - cs.unwrap_or(0.6)) * 2.2 + if flip { 0.15 } else { 0.0 }) * stage_w(c, o);
        if score < 0.2 {
            continue;
        }
        let angle = if flip {
            let oa: Vec<&String> = o_att.iter().collect();
            let ma: Vec<&String> = my_att.iter().collect();
            format!("{} instead of {}", names_str(c, &oa), names_str(c, &ma))
        } else if o_moves.is_empty() {
            "a different move".to_string()
        } else {
            let om: Vec<&String> = o_moves.iter().collect();
            let ns: Vec<String> = om.iter().take(2).map(|l| py::str(&c.label_field(l, "name"))).collect();
            format!("built on {}", ns.join(", "))
        };
        let st2: Vec<&String> = st.iter().take(2).collect();
        let ev = label_ev(c, o, &st[0]);
        let why = format!("Also about {}, other angle: {angle}", names_str(c, &st2));
        gs["contrasts"].add(o, score, &why, "proposed", ev, "labels", || {
            obj([("joke_id", json!(o)), ("topics", json!(names(c, &st2)))])
        });
    }
    gs
}

/// `trend(values_by_date)`: one mean per date, the least-squares slope over date order × (n - 1) / mean; up above
/// +0.20, down below -0.20.
pub(crate) fn trend(xs: &[(&str, Option<f64>)]) -> (Value, Value) {
    let mut by: IndexMap<&str, Vec<f64>> = IndexMap::new();
    for (d, v) in xs {
        if let Some(v) = v {
            by.entry(d).or_default().push(*v);
        }
    }
    let mut dates: Vec<&str> = by.keys().copied().collect();
    dates.sort_unstable();
    let vals: Vec<f64> = dates
        .iter()
        .map(|d| {
            let v = &by[d];
            py::sum(v.iter().copied()) / v.len() as f64
        })
        .collect();
    let n = vals.len();
    if n < 2 {
        return (Value::Null, Value::Null);
    }
    let mu = py::sum(vals.iter().copied()) / n as f64;
    if mu <= 0.0 {
        return (json!("flat"), json!(0.0));
    }
    let xm = (n as f64 - 1.0) / 2.0;
    let num = py::sum(vals.iter().enumerate().map(|(i, v)| (i as f64 - xm) * (v - mu)));
    let den = py::sum((0..n).map(|i| (i as f64 - xm).powi(2)));
    let rel = num / den * (n as f64 - 1.0) / mu;
    let t = if rel > 0.20 {
        "up"
    } else if rel < -0.20 {
        "down"
    } else {
        "flat"
    };
    (json!(t), json!(round_to(rel, 2)))
}

fn r2(x: Option<f64>) -> Value {
    x.map_or(Value::Null, |x| json!(round_to(x, 2)))
}

fn opt(x: Option<f64>) -> Value {
    x.map_or(Value::Null, |x| json!(x))
}

/// `stage_trend(c, jid)`.
fn stage_trend(c: &Corpus, jid: &str) -> Value {
    let ps = c.perfs.get(jid).map_or(&[][..], Vec::as_slice);
    let nights: Vec<Value> = ps
        .iter()
        .map(|p| {
            json!({"date": p.date, "set": p.set, "venue": p.venue, "label": venue_short(p.venue.as_str(), Some(&p.date)),
                   "laugh_s_per_min": opt(p.laugh_s_per_min), "vs_room": opt(p.vs_room), "ai_rating": p.ai_rating,
                   "laugh_s": p.laugh_s, "duration_s": p.duration_s, "time_to_first_laugh_s": p.time_to_first_laugh_s,
                   "seg_ids": p.seg_ids})
        })
        .collect();
    let (lspm_t, lspm_rel) = trend(
        &ps.iter()
            .map(|p| (p.date.as_str(), p.laugh_s_per_min))
            .collect::<Vec<_>>(),
    );
    let (vr_t, vr_rel) = trend(&ps.iter().map(|p| (p.date.as_str(), p.vs_room)).collect::<Vec<_>>());
    let best = if ps.is_empty() {
        Value::Null
    } else {
        // `max(ps, key=…)`: the first of the greatest
        let pick = |key: &dyn Fn(&crate::corpus::Perf) -> (f64, &str)| {
            ps.iter().fold(&ps[0], |b, p| if key(p) > key(b) { p } else { b })
        };
        let b = pick(&|p| (p.laugh_s_per_min.unwrap_or(0.0), p.date.as_str()));
        let mut best = json!({"date": b.date, "set": b.set, "venue": b.venue,
                              "label": venue_short(b.venue.as_str(), Some(&b.date)),
                              "laugh_s_per_min": opt(b.laugh_s_per_min), "vs_room": opt(b.vs_room),
                              "ai_rating": b.ai_rating,
                              "basis": "highest laugh-s/min (same rule as inferred.json best_set)"});
        let br = pick(&|p| (p.vs_room.unwrap_or(0.0), p.date.as_str()));
        if !std::ptr::eq(br, b) {
            best["best_vs_room"] = json!({"date": br.date, "label": venue_short(br.venue.as_str(), Some(&br.date)),
                                          "vs_room": opt(br.vs_room)});
        }
        best
    };
    json!({"nights": nights, "trend": lspm_t, "trend_rel_change": lspm_rel, "vs_room_trend": vr_t, "vs_room_rel_change": vr_rel,
           "avg_laugh_s_per_min": r2(mean(ps.iter().map(|p| p.laugh_s_per_min))),
           "avg_vs_room": r2(mean(ps.iter().map(|p| p.vs_room))), "best_night": best, "low_n": ps.len() < 3,
           "note": "Raw laugh-s/min moves with the room; read it against vs_room (1.0 = the room's own rate)."})
}

/// `land_harder(c, jid, cap=4)`: rule-based notes from his quality note and the laugh metrics, each with its basis.
fn land_harder(c: &Corpus, jid: &str) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let n = c.joke(jid);
    let note = n
        .and_then(|n| n.get("quality"))
        .and_then(|q| q.get("notes"))
        .and_then(Value::as_str)
        .map_or("", py::strip);
    if !note.is_empty() {
        let ev = c
            .text_of(jid, Some("quality.notes"))
            .filter(|t| t.contains(note))
            .map(|_| json!({"node": jid, "quote": note, "field": "quality.notes"}));
        out.push(
            json!({"text": format!("Your writing note: {note}"), "basis": "quality note (catalog pass)",
                        "source": "proposed", "evidence": ev.into_iter().collect::<Vec<_>>()}),
        );
    }
    let ps = c.perfs.get(jid).map_or(&[][..], Vec::as_slice);
    if let Some(first) = ps.first() {
        let best = ps.iter().fold(first, |b, p| {
            if (p.laugh_s_per_min.unwrap_or(0.0), p.date.as_str()) > (b.laugh_s_per_min.unwrap_or(0.0), b.date.as_str())
            {
                p
            } else {
                b
            }
        });
        let others: Vec<&crate::corpus::Perf> = ps.iter().filter(|p| !std::ptr::eq(*p, best)).collect();
        let bars = |pl: &Map<String, Value>| num(pl.get("bars").filter(|v| py::truthy(Some(v)))).unwrap_or(0.0);
        let mut pls: Vec<&Map<String, Value>> = best.punchlines.iter().collect();
        pls.sort_by(|a, b| {
            let at = |p: &Map<String, Value>| num(p.get("at_ms").filter(|v| py::truthy(Some(v)))).unwrap_or(0.0);
            bars(b).total_cmp(&bars(a)).then(at(a).total_cmp(&at(b)))
        });
        // 1. a punch line that hit on the best night and was not said on the weaker nights
        for pl in pls {
            let text = pl.get("text").and_then(Value::as_str).unwrap_or("");
            let missing: Vec<&&crate::corpus::Perf> = others
                .iter()
                .filter(|p| {
                    !p.seg_ids
                        .iter()
                        .any(|sid| crate::textdiff::present(text, &c.text_of(sid, None).unwrap_or_default(), 0.6))
                })
                .collect();
            if !others.is_empty() && !missing.is_empty() && bars(pl) >= 2.0 {
                let seg = pl.get("seg_id").and_then(Value::as_str).unwrap_or("");
                if let Some(ev) = c.ev(seg, text) {
                    let where_: Vec<String> = missing
                        .iter()
                        .map(|p| venue_short(p.venue.as_str(), Some(&p.date)))
                        .collect();
                    let b = py::str(pl.get("bars").unwrap_or(&Value::Null));
                    out.push(json!({"text": format!("Keep \"{text}\": a {b}-bar laugh at {}, and you didn't say it at {}.",
                                                     venue_short(best.venue.as_str(), Some(&best.date)), where_.join(", ")),
                                    "basis": "best night's punchlines vs the other nights' transcripts",
                                    "source": "inferred", "evidence": [ev]}));
                    break;
                }
            }
        }
        // 2. a slow first laugh
        let ttfl = |p: &crate::corpus::Perf| num(Some(&p.time_to_first_laugh_s));
        let tt: Vec<f64> = ps.iter().filter_map(ttfl).collect();
        if !tt.is_empty() && median(tt.clone()) > 15.0 {
            let fast = ps.iter().fold(first, |b, p| {
                if ttfl(p).unwrap_or(1e9) < ttfl(b).unwrap_or(1e9) {
                    p
                } else {
                    b
                }
            });
            let ev: Vec<Value> = c.seg_evidence(&fast.main_seg).into_iter().collect();
            out.push(json!({"text": format!("The first laugh takes {:.0}s on average (fastest {:.0}s at {}). Get to the first punch sooner.",
                                             median(tt), ttfl(fast).unwrap_or(0.0),
                                             venue_short(fast.venue.as_str(), Some(&fast.date))),
                            "basis": "time_to_first_laugh_s across nights", "source": "inferred", "evidence": ev}));
        }
        // 3. a long stretch with no laugh inside the bit
        let da = ps
            .iter()
            .fold(first, |b, p| if p.dead_air_max_s > b.dead_air_max_s { p } else { b });
        if da.dead_air_max_s >= 25.0 {
            let ev: Vec<Value> = c.seg_evidence(&da.main_seg).into_iter().collect();
            out.push(
                json!({"text": format!("{:.0}s without a laugh inside the bit at {}. Add a tag or cut that stretch.",
                                             da.dead_air_max_s, venue_short(da.venue.as_str(), Some(&da.date))),
                            "basis": "dead_air_max_s", "source": "inferred", "evidence": ev}),
            );
        }
        // 4. below the room most nights
        let below = ps.iter().filter(|p| p.vs_room.is_some_and(|v| v < 0.9)).count();
        if ps.len() >= 2 && below * 2 > ps.len() {
            let avg = mean(ps.iter().map(|p| p.vs_room)).unwrap_or(0.0);
            out.push(json!({"text": format!("It plays below the room on {below} of {} nights (avg {avg:.2}x the room).", ps.len()),
                            "basis": "vs_room", "source": "inferred", "evidence": []}));
        }
    }
    out.truncate(4);
    out
}

/// `_joke_head(c, jid)`.
fn joke_head(c: &Corpus, jid: &str) -> Value {
    let n = c.joke(jid);
    let field = |k: &str| n.and_then(|n| n.get(k)).cloned().unwrap_or(Value::Null);
    let (st, src) = c.stage(jid);
    let (d, basis) = c.duration(jid);
    let ps = c.perfs.get(jid).map_or(&[][..], Vec::as_slice);
    let rooms: HashSet<String> = ps.iter().map(|p| p.venue.to_string()).collect();
    let devs: Vec<Value> = c
        .devices(jid, 3, 0.55)
        .iter()
        .map(|l| c.label_field(l, "name"))
        .collect();
    json!({"id": jid, "title": field("title"), "stage": st, "stage_source": src, "summary": field("summary"),
           "devices": devs, "duration_s": d, "duration_basis": basis, "rooms": rooms.len(), "nights": ps.len()})
}

/// `joke_connections(c, jid, cap)`; `None` when `jid` is not a joke in the notebook (the Python's `KeyError`).
pub fn joke_connections(story: &Story<'_>, c: &Corpus, jid: &str, cap: usize) -> Option<Value> {
    if !c.is_joke(jid) {
        return None;
    }
    let g = groups(story, c, jid, cap);
    let groups: Vec<Value> = GROUPS
        .iter()
        .map(|(k, t)| json!({"key": k, "title": t, "items": g[*k].ranked(c, cap)}))
        .collect();
    Some(json!({
        "joke": joke_head(c, jid), "groups": groups, "stage_trend": stage_trend(c, jid), "land_harder": land_harder(c, jid),
        "notes": ["Scores are 0-1 within a group. Labels are machine proposals; laughs come from automatic detection. \
                   Source 'his' = you confirmed it, 'inferred' = AI-accepted, 'proposed' = a suggestion."],
    }))
}

/// `_conn_why(it, group)`: why an item is on the map, in his words.
fn conn_why(it: &Value, group: &str) -> String {
    let basis: Vec<&str> = it
        .get("basis")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let his = it.get("source").and_then(Value::as_str) == Some("his");
    if basis.contains(&"back_to_back") {
        let n = num(it.get("nights").filter(|v| py::truthy(Some(v)))).unwrap_or(0.0) as i64;
        return format!(
            "You did it right after this on stage{}",
            if n > 1 { format!(", {n} times") } else { String::new() }
        );
    }
    match group {
        "next" => (if his { "You linked these" } else { "Could lead into it" }).to_string(),
        "tags" => (if his {
            "You tagged it with this"
        } else {
            "Lands on the same note"
        })
        .to_string(),
        "callbacks" => (if it.get("direction").and_then(Value::as_str) == Some("calls_back_this") {
            "It calls back to this"
        } else {
            "This calls back to it"
        })
        .to_string(),
        "same_world" => {
            let names: Vec<String> = ["personas", "topics"]
                .iter()
                .flat_map(|k| it.get(*k).and_then(Value::as_array).cloned().unwrap_or_default())
                .map(|v| py::str(&v))
                .collect();
            if basis.contains(&"same_premise") {
                (if his {
                    "You marked it the same premise"
                } else {
                    "Same premise"
                })
                .to_string()
            } else if !names.is_empty() {
                format!(
                    "Same world: {}",
                    names.iter().take(2).cloned().collect::<Vec<_>>().join(", ")
                )
            } else if basis.contains(&"chunk") {
                "Goes in the same chunk".to_string()
            } else {
                "Connected".to_string()
            }
        }
        _ => "Connected".to_string(),
    }
}

/// The joke map's groups being filled (`put` in `joke_map`).
struct MapGroups<'c> {
    c: &'c Corpus,
    cap: usize,
    groups: IndexMap<&'static str, Vec<Value>>,
    seen: HashMap<&'static str, HashSet<String>>,
}

impl<'c> MapGroups<'c> {
    fn new(c: &'c Corpus, jid: &str, cap: usize) -> Self {
        Self {
            c,
            cap,
            groups: MAP_GROUPS.iter().map(|(k, _)| (*k, Vec::new())).collect(),
            seen: MAP_GROUPS
                .iter()
                .map(|(k, _)| (*k, HashSet::from([jid.to_string()])))
                .collect(),
        }
    }

    /// `put(g, oid, why, mine, **extra)`.
    fn put(&mut self, g: &'static str, oid: &str, why: &str, mine: bool, extra: Vec<(&'static str, Value)>) {
        let key = extra
            .iter()
            .find(|(k, v)| *k == "unit_id" && py::truthy(Some(v)))
            .map_or_else(|| oid.to_string(), |(_, v)| py::str(v));
        if oid.is_empty()
            || self.seen[g].contains(&key)
            || self.groups[g].len() >= self.cap
            || (!self.c.is_joke(oid) && g != "tags")
        {
            return;
        }
        self.seen.get_mut(g).map(|s| s.insert(key));
        let mut fields: Vec<(&'static str, Value)> = vec![("why", json!(why)), ("mine", json!(mine))];
        fields.extend(extra);
        let it = if self.c.is_joke(oid) {
            card(self.c, oid, &HashSet::new(), fields)
        } else {
            let mut m = Map::new();
            m.insert("joke_id".into(), Value::Null);
            for (k, v) in fields {
                m.insert(k.into(), v);
            }
            Value::Object(m)
        };
        self.groups[g].push(it);
    }

    /// The tags group from the connections' tags: his first, then the lines that land on the same note.
    fn tags(&mut self, items: &[Value]) {
        let c = self.c;
        let mut its: Vec<&Value> = items.iter().collect();
        its.sort_by(|a, b| {
            let k = |x: &Value| {
                (
                    i64::from(x.get("source").and_then(Value::as_str) != Some("his")),
                    -x.get("score").and_then(Value::as_f64).unwrap_or(0.0),
                )
            };
            let (ka, kb) = (k(a), k(b));
            ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
        });
        for it in its {
            let u = it.get("unit_id").and_then(Value::as_str).unwrap_or("");
            let line = it.get("line").and_then(Value::as_str).unwrap_or("");
            if u.is_empty() || line.is_empty() || !c.text_of(u, None).unwrap_or_default().contains(line) {
                continue;
            }
            let owner = it
                .get("joke_id")
                .and_then(Value::as_str)
                .filter(|o| !o.is_empty())
                .map(str::to_string)
                .or_else(|| c.unit_joke.get(u).cloned());
            let Some(owner) = owner.filter(|o| !o.is_empty() && c.is_joke(o)) else {
                continue;
            };
            let mine = it.get("source").and_then(Value::as_str) == Some("his");
            self.put("tags", &owner, &conn_why(it, "tags"), mine, vec![("unit_id", json!(u))]);
            if let Some(last) = self.groups["tags"].last_mut()
                && last.get("unit_id").and_then(Value::as_str) == Some(u)
            {
                let date = c
                    .nodes
                    .get(u)
                    .and_then(|n| n.get("date"))
                    .filter(|d| py::truthy(Some(d)))
                    .cloned()
                    .or_else(|| last.get("date").cloned())
                    .unwrap_or(Value::Null);
                for (k, v) in [
                    ("quote", json!(line)),
                    ("quote_node", json!(u)),
                    ("cut_start", json!(false)),
                    ("cut_end", json!(false)),
                    ("date", date),
                ] {
                    last[k] = v;
                }
            }
        }
    }
}

fn items_of<'a>(g: &'a [Value], key: &str) -> &'a [Value] {
    g.iter()
        .find(|x| x.get("key").and_then(Value::as_str) == Some(key))
        .and_then(|x| x.get("items"))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// `MAX_EDGES_PER_NODE`: the links one bit keeps on the bird's-eye map.
const MAX_EDGES_PER_NODE: usize = 6;

/// One link of the bird's-eye map: `(a, b, kind)` is its key; the strongest of two is kept.
struct MapLink {
    a: String,
    b: String,
    kind: &'static str,
    w: f64,
    why: String,
}

/// `notebook.all_map(c, cap, same_idea)`: every bit as a node (the top `cap` by nights, then laughs) and the links
/// between them, grouped like the per-joke map: next (his segues, done back to back), tags (a line of one bit tagging
/// another), same_world (same premise, a shared persona, the same Insights chunk), callbacks, versions (the merge
/// judge's same-idea groups, `same_idea`). Names only: no bodies.
#[allow(clippy::too_many_lines)]
pub fn all_map(c: &Corpus, cap: usize, same_idea: &[Value]) -> Value {
    let heat = |j: &str| -> Option<f64> {
        let vs: Vec<f64> = c.perfs.get(j).into_iter().flatten().filter_map(|p| p.vs_room).collect();
        (!vs.is_empty()).then(|| round_to(py::sum(vs.iter().copied()) / vs.len() as f64, 2))
    };
    // `rows.sort(...)` three times: the last full key is (-nights, -heat, id)
    let mut rows: Vec<(usize, f64, String, String)> = c
        .jokes
        .iter()
        .map(|j| {
            let nights: BTreeSet<&str> = c.perfs.get(j).into_iter().flatten().map(|p| p.set.as_str()).collect();
            let last: String = c
                .joke(j)
                .and_then(|n| n.get("last_seen"))
                .filter(|v| py::truthy(Some(v)))
                .map(py::str)
                .unwrap_or_default()
                .chars()
                .take(10)
                .collect();
            (nights.len(), heat(j).unwrap_or(0.0), last, j.clone())
        })
        .collect();
    rows.sort_by(|x, y| y.0.cmp(&x.0).then(y.1.total_cmp(&x.1)).then_with(|| x.3.cmp(&y.3)));
    let keep: Vec<&String> = rows.iter().take(cap).map(|r| &r.3).collect();
    let ks: HashSet<&str> = keep.iter().map(|j| j.as_str()).collect();
    let mut edges: IndexMap<(String, String, &'static str), MapLink> = IndexMap::new();
    let mut link = |a: &str, b: &str, kind: &'static str, w: f64, why: String| {
        if a == b || !ks.contains(a) || !ks.contains(b) {
            return;
        }
        let key = (a.min(b).to_string(), a.max(b).to_string(), kind);
        if edges.get(&key).is_none_or(|e| w > e.w) {
            let (lo, hi) = (key.0.clone(), key.1.clone());
            edges.insert(
                key,
                MapLink {
                    a: lo,
                    b: hi,
                    kind,
                    w: round_to(w, 3),
                    why,
                },
            );
        }
    };
    for e in &c.edges {
        let (kind, src, dst) = (s(e, "kind"), s(e, "src").unwrap_or(""), s(e, "dst").unwrap_or(""));
        let conf = e
            .get("confidence")
            .filter(|v| py::truthy(Some(v)))
            .and_then(Value::as_f64)
            .unwrap_or(0.5);
        let his = c.source_of(e) == "his";
        let w = if his { 1.0 } else { conf };
        match kind {
            Some("segue") => link(
                src,
                dst,
                "next",
                w,
                if his { "Your segue" } else { "Could lead in" }.to_string(),
            ),
            Some("same_premise") => link(src, dst, "same_world", w, "Same premise".to_string()),
            Some("callback_to") => link(src, dst, "callbacks", w, "Calls back".to_string()),
            Some("tag_of") if src.starts_with("unit:") => {
                if let Some(owner) = c.unit_joke.get(src).filter(|o| o.as_str() != dst) {
                    link(owner, dst, "tags", w, "A line of it tags this".to_string());
                }
            }
            _ => {}
        }
    }
    for ((a, b), nights) in &c.back_to_back {
        #[allow(clippy::cast_precision_loss)]
        let w = (0.55 + 0.15 * nights.len() as f64).min(1.0);
        link(
            a,
            b,
            "next",
            w,
            format!("Done back to back on stage, {}x", nights.len()),
        );
    }
    // a shared persona: each bit links to at most 4 others carrying the same persona label (strongest first)
    let mut by_persona: IndexMap<String, Vec<(f64, &String)>> = IndexMap::new();
    for j in &keep {
        for (l, (w, _)) in c.labels_of(j, Some(&["persona"]), 0.5) {
            by_persona.entry(l).or_default().push((w, j));
        }
    }
    for (l, members) in &mut by_persona {
        members.sort_by(|x, y| y.0.total_cmp(&x.0).then_with(|| x.1.cmp(y.1)));
        let name = c
            .nodes
            .get(l)
            .and_then(|n| n.get("name"))
            .filter(|v| py::truthy(Some(v)))
            .map_or_else(|| "persona".to_string(), py::str);
        for (i, (w, j)) in members.iter().take(40).enumerate() {
            for (w2, o) in members.iter().skip(i + 1).take(4) {
                link(
                    j,
                    o,
                    "same_world",
                    0.4 + 0.3 * w.min(*w2),
                    format!("Same world: {name}"),
                );
            }
        }
    }
    let chunks: &[Value] = c
        .insights
        .get("chunks")
        .and_then(|x| x.get("chunks"))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    for ch in chunks {
        let ms: Vec<String> = ch
            .get("jokes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|x| c.map_joke(x))
            .filter(|m| ks.contains(m.as_str()))
            .take(12)
            .collect();
        let name = ch.get("name").map_or_else(|| "None".to_string(), py::str);
        for (i, j) in ms.iter().enumerate() {
            for o in ms.iter().skip(i + 1).take(3) {
                link(j, o, "same_world", 0.35, format!("Same chunk: {name}"));
            }
        }
    }
    for g in same_idea {
        let ms: Vec<&str> = g
            .get("members")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|m| m.get("joke_id").and_then(Value::as_str))
            .collect();
        for (i, j) in ms.iter().enumerate() {
            for o in ms.iter().skip(i + 1) {
                link(j, o, "versions", 0.9, "Looks like the same idea".to_string());
            }
        }
    }
    // cap: at most MAX_EDGES_PER_NODE links per bit (strongest kept), then nodes * 4 overall
    let mut deg: HashMap<&str, usize> = HashMap::new();
    let mut out_edges: Vec<&MapLink> = Vec::new();
    let mut all: Vec<&MapLink> = edges.values().collect();
    all.sort_by(|x, y| {
        y.w.total_cmp(&x.w)
            .then_with(|| x.a.cmp(&y.a))
            .then_with(|| x.b.cmp(&y.b))
            .then_with(|| x.kind.cmp(y.kind))
    });
    for e in all {
        if deg.get(e.a.as_str()).copied().unwrap_or(0) >= MAX_EDGES_PER_NODE
            || deg.get(e.b.as_str()).copied().unwrap_or(0) >= MAX_EDGES_PER_NODE
        {
            continue;
        }
        *deg.entry(e.a.as_str()).or_insert(0) += 1;
        *deg.entry(e.b.as_str()).or_insert(0) += 1;
        out_edges.push(e);
        if out_edges.len() >= keep.len() * 4 {
            break;
        }
    }
    let nodes: Vec<Value> = rows
        .iter()
        .take(cap)
        .map(|(nights, h, last, j)| {
            let n = c.joke(j);
            let title = n.and_then(|n| n.get("title")).filter(|v| py::truthy(Some(v))).map(py::str).unwrap_or_default();
            json!({"id": j,
                   "short_name": crate::display::short_name_of(&title, n.and_then(|n| n.get("short_name")).and_then(Value::as_str)),
                   "title": short_title(Some(&title)), "stage": c.stage(j).0, "nights": nights,
                   "heat": if *h == 0.0 { Value::Null } else { json!(h) },
                   "last_seen": if last.is_empty() { Value::Null } else { json!(last) },
                   "degree": deg.get(j.as_str()).copied().unwrap_or(0)})
        })
        .collect();
    let mut by_kind: Map<String, Value> = ["next", "tags", "same_world", "callbacks", "versions"]
        .iter()
        .map(|k| ((*k).to_string(), json!(0)))
        .collect();
    for e in &out_edges {
        if let Some(n) = by_kind.get_mut(e.kind) {
            *n = json!(n.as_u64().unwrap_or(0) + 1);
        }
    }
    json!({"cap": cap, "nodes": nodes,
           "edges": out_edges.iter().map(|e| json!({"a": e.a, "b": e.b, "kind": e.kind, "w": e.w, "why": e.why})).collect::<Vec<_>>(),
           "counts": {"jokes": c.jokes.len(), "nodes": nodes.len(), "edges": out_edges.len(), "by_kind": by_kind},
           "groups": MAP_GROUPS.iter().map(|(k, t)| json!({"key": k, "title": t})).collect::<Vec<_>>()})
}

/// `notebook.joke_map(c, jid, cap)`: the joke page as a map, its connections grouped, each with a plain-words reason.
/// `None` when `jid` is not a joke in the notebook.
#[allow(clippy::too_many_lines)]
pub fn joke_map(story: &Story<'_>, c: &Corpus, jid: &str, cap: usize) -> Option<Value> {
    let conn = joke_connections(story, c, jid, cap * 2)?;
    let cg: &[Value] = conn.get("groups").and_then(Value::as_array).map_or(&[], Vec::as_slice);
    let mut m = MapGroups::new(c, jid, cap);
    // could come next: proven on stage and his own links first, then the board engine's picks
    let mut nexts: Vec<&Value> = items_of(cg, "next").iter().collect();
    let b2b = |x: &Value| {
        x.get("basis")
            .and_then(Value::as_array)
            .is_some_and(|b| b.iter().any(|v| v.as_str() == Some("back_to_back")))
    };
    nexts.sort_by(|a, b| {
        let k = |x: &Value| {
            let r = if b2b(x) {
                0
            } else if x.get("source").and_then(Value::as_str) == Some("his") {
                1
            } else {
                2
            };
            (r, -x.get("score").and_then(Value::as_f64).unwrap_or(0.0))
        };
        let (ka, kb) = (k(a), k(b));
        ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
    });
    for it in nexts {
        let oid = it.get("joke_id").and_then(Value::as_str).unwrap_or("");
        let mine = it.get("source").and_then(Value::as_str) == Some("his");
        m.put(
            "next",
            oid,
            &conn_why(it, "next"),
            mine,
            vec![("proven_pair", json!(b2b(it)))],
        );
    }
    let title = c.title(jid);
    for x in best_next(story, c, &[jid], cap) {
        let oid = x.get("joke_id").and_then(Value::as_str).unwrap_or("");
        let why = plain_best_next(x.get("why").and_then(Value::as_str).unwrap_or(""), &title);
        let proven = x.get("transition").and_then(Value::as_str) == Some("proven");
        m.put("next", oid, &why, false, vec![("proven_pair", json!(proven))]);
    }
    m.tags(items_of(cg, "tags"));
    // same world: the same subject first (rare shared words + similarity), then persona / topic, then same chunk
    let mut subj: Vec<(f64, &String, Vec<String>)> = Vec::new();
    for o in c.jokes.iter().filter(|o| *o != jid) {
        let (sc, ws, _) = story.same_subject(&[jid], o);
        if sc >= 0.35 {
            subj.push((sc, o, ws));
        }
    }
    subj.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    for (sc, o, ws) in subj {
        let phrase = crate::setlist::subject_phrase(ws.iter().map(String::as_str));
        m.put(
            "same_world",
            o,
            &phrase,
            false,
            vec![("subject", json!(round_to(sc, 3)))],
        );
    }
    let mut sw: Vec<&Value> = items_of(cg, "same_world").iter().collect();
    sw.sort_by(|a, b| {
        let k = |x: &Value| {
            (
                i64::from(x.get("source").and_then(Value::as_str) != Some("his")),
                -x.get("score").and_then(Value::as_f64).unwrap_or(0.0),
            )
        };
        let (ka, kb) = (k(a), k(b));
        ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
    });
    for it in sw {
        let oid = it.get("joke_id").and_then(Value::as_str).unwrap_or("");
        let mine = it.get("source").and_then(Value::as_str) == Some("his");
        m.put("same_world", oid, &conn_why(it, "same_world"), mine, Vec::new());
    }
    for it in items_of(cg, "callbacks") {
        let oid = it.get("joke_id").and_then(Value::as_str).unwrap_or("");
        let mine = it.get("source").and_then(Value::as_str) == Some("his");
        m.put("callbacks", oid, &conn_why(it, "callbacks"), mine, Vec::new());
    }
    // older versions: his other written versions of this bit, newest first (the lead version is the center)
    let lead = c.lead_unit(jid);
    let none = HashSet::new();
    let all_texts = texts(c, jid);
    for (node, text, date) in &all_texts {
        if Some(node) == lead.as_ref() || m.groups["versions"].len() >= cap {
            continue;
        }
        let sent = sentences(text)
            .next()
            .map_or_else(|| py::strip(text).to_string(), str::to_string);
        let (q, cl, cr) = clip(&sent, &none);
        let t = c
            .nodes
            .get(node)
            .and_then(|n| n.get("doc_title"))
            .filter(|v| py::truthy(Some(v)))
            .cloned()
            .unwrap_or_else(|| json!("Written version"));
        m.groups["versions"].push(
            json!({"joke_id": jid, "unit_id": node, "title": t, "quote": q, "quote_node": node,
                                         "cut_start": cl, "cut_end": cr, "date": date, "why": "An older version",
                                         "stage_line": null, "proven": false}),
        );
    }
    let mut center = card(c, jid, &none, []);
    center["text"] = json!(all_texts.first().map_or("", |t| t.1.as_str()));
    let groups: Vec<Value> = MAP_GROUPS
        .iter()
        .map(|(k, t)| json!({"key": k, "title": t, "items": m.groups[*k]}))
        .collect();
    Some(json!({"joke": center, "groups": groups}))
}

/// `tag_option(c, jid)`'s pick: the first item of the map's "Could tag it" group at `cap=3`, built from the
/// connections' tags alone (the only group it reads); `None` when there is none or `jid` is not a joke.
pub fn first_tag(c: &Corpus, jid: &str) -> Option<Value> {
    if !c.is_joke(jid) {
        return None;
    }
    let cap = 3;
    let mut g = Group::default();
    // the tag edges (the only explicit edges the tags group reads), then the reveal query, at the map's cap × 2
    for &i in c.edges_by_node.get(jid).map_or(&[][..], Vec::as_slice) {
        let e = &c.edges[i];
        let (src, dst) = (s(e, "src").unwrap_or(""), s(e, "dst").unwrap_or(""));
        if s(e, "kind") != Some("tag_of") || dst != jid || !src.starts_with("unit:") {
            continue;
        }
        let ev = c.edge_evidence(e, 2);
        let conf = num(e.get("confidence").filter(|v| py::truthy(Some(v)))).unwrap_or(0.5);
        let reason = e
            .get("reason")
            .filter(|v| py::truthy(Some(v)))
            .map(py::str)
            .unwrap_or_default();
        let owner = c.unit_joke.get(src).filter(|o| !o.is_empty() && *o != jid).cloned();
        let q = ev.first().and_then(|x| x.get("quote")).cloned().unwrap_or(Value::Null);
        let eid = e.get("id").cloned().unwrap_or(Value::Null);
        g.add(src, conf, py::strip(&reason), c.source_of(e), ev, "tag_of", || {
            obj([
                ("unit_id", json!(src)),
                ("line", q),
                ("joke_id", json!(owner)),
                ("edge", eid),
            ])
        });
    }
    tags_group(c, jid, cap * 2, &mut g);
    let items = g.ranked(c, cap * 2);
    let mut m = MapGroups::new(c, jid, cap);
    m.tags(&items);
    m.groups["tags"].first().cloned()
}

/// `stage_w` for callers that score bits the way the connections do.
pub fn stage_weight(c: &Corpus, j: &str) -> f64 {
    stage_w(c, j)
}

/// `short_title(c.title(j))`.
pub fn short(c: &Corpus, j: &str) -> String {
    short_title(Some(&c.title(j)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numpy_pairwise_mean_and_population_std() {
        let xs: Vec<f64> = (0..300).map(|i| f64::from(i) * 0.1).collect();
        let (mu, sd) = mean_std(&xs);
        assert!((mu - 14.95).abs() < 1e-12);
        // numpy printed 8.660205925188308
        assert!((sd - 8.660_205_925_188_308).abs() < 1e-12);
        assert!(mean_std(&[1.0, 1.0]).1.abs() < f64::EPSILON);
    }

    #[test]
    fn trend_is_the_slope_over_date_order() {
        let (t, r) = trend(&[
            ("2024-01-01", Some(1.0)),
            ("2024-02-01", Some(2.0)),
            ("2024-03-01", None),
        ]);
        assert_eq!(t, json!("up"));
        assert_eq!(r, json!(0.67));
        assert_eq!(trend(&[("2024-01-01", Some(1.0))]).0, Value::Null);
        assert_eq!(
            trend(&[("a", Some(0.0)), ("b", Some(0.0))]),
            (json!("flat"), json!(0.0))
        );
    }

    #[test]
    fn a_new_basis_adds_a_bonus_and_the_best_source_wins() {
        let mut g = Group::default();
        g.add("x", 0.4, "a", "proposed", vec![], "segue", Map::new);
        g.add(
            "x",
            0.3,
            "b",
            "his",
            vec![json!(null), json!({"q": 1})],
            "labels",
            Map::new,
        );
        g.add("x", 0.3, "b", "inferred", vec![json!({"q": 1})], "labels", Map::new);
        let it = &g.items["x"];
        assert!((it.score - 0.45).abs() < 1e-12);
        assert_eq!(it.why, ["a", "b"]);
        assert_eq!(it.source, "his");
        assert_eq!(it.evidence, [json!({"q": 1})]);
    }
}
