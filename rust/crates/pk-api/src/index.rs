//! What the app needs that the replayed view does not index (`api-dev/server.py` `AppIndex` / `DBAppIndex`):
//! performances per joke, who touched what, the stage the AI set, the rooms a joke played and how it did there, and
//! the Bits list rows (`summaries`) that `/api/meta`, `/api/pipeline` and later `/api/jokes` are cut from.
//!
//! Built once per snapshot generation in `spawn_blocking` and then only read. Every rule is the Python's, including
//! its arithmetic (`display::py_sum`, `display::r2`), so the rows compare equal as values.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use indexmap::IndexMap;
use pk_core::py::truthy;
use pk_kb::{Bucket, View, bucket};
use pk_wire::api::{JokeSummary, LabelChip};
use regex::Regex;
use serde_json::{Map, Value};

use crate::display::{punch_line, py_sum, r2, short_name_of, short_title};
use crate::snapshot::Rows;

/// `STAGE_ORDER` (the Bits list's order).
pub const STAGE_ORDER: [&str; 5] = ["working", "testing", "to_try", "idea", "retired"];
/// `kb.STAGES` (the order `/api/meta` and `/api/pipeline` list them in).
pub const STAGES: [&str; 5] = ["idea", "to_try", "testing", "working", "retired"];
/// `RATINGS`.
pub const RATINGS: [&str; 4] = ["killed", "worked", "meh", "died"];
const HIT_RATINGS: [&str; 2] = ["killed", "worked"];
/// `ai_overlay.NOT_COUNTED`: performances that are not material.
const NOT_COUNTED: [&str; 2] = ["crowd_work", "banter"];
/// `VS_ROOM_CLIP`.
const VS_ROOM_CLIP: f64 = 3.0;

/// The AI layer the database holds (`analysis` rows) and `load_info.inferred`, as `surreal_kb._fetch` builds it.
#[derive(Debug, Clone, Default)]
pub struct Ai {
    /// Set id -> the analysis meta (`{source, path, run_id, generated_at, summary, set}`), in `ord` order.
    pub analysis: IndexMap<String, Value>,
    /// Set id -> `(analysis row, performance id)` pairs.
    pub rows: IndexMap<String, Vec<(Value, Option<String>)>>,
    /// `load_info.inferred`.
    pub inferred: Value,
}

/// One Bits list row, plus the label ids the filters use (not on the wire).
#[derive(Debug, Clone)]
pub struct Row {
    /// The wire row.
    pub summary: JokeSummary,
    /// Every live label on the joke.
    pub label_ids: Vec<String>,
}

#[derive(Debug, Default)]
struct Touch {
    actions: HashSet<Option<String>>,
    fields: HashSet<String>,
    ends: Vec<String>,
}

/// The per-view index.
#[derive(Debug)]
pub struct AppIndex {
    /// The view it indexes (visible, or with hidden material on request); shared with the routes that need only it.
    pub view: Arc<View>,
    /// Set node ids, by date.
    pub sets: Vec<String>,
    /// How many performance nodes there are.
    pub performances: usize,
    /// The Bits list, in the Desk's order.
    pub summaries: Vec<Row>,
}

/// The per-set files the index reads (`corpus/sets/<dir>/meta.json`), by set folder.
pub type SetMetas = HashMap<String, Map<String, Value>>;

static SET_DIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A([0-9]{4})-?([0-9]{2})-?([0-9]{2})_([A-Za-z0-9_-]+)\n?\z")
        .unwrap_or_else(|e| unreachable!("constant regex: {e}"))
});

/// `AppKB.set_dir`: `set:2024-01-01_x` -> `20240101_x`.
pub fn set_dir(set_id: &str) -> Option<String> {
    let sid = set_id.strip_prefix("set:").unwrap_or(set_id);
    let m = SET_DIR.captures(sid)?;
    Some(format!("{}{}{}_{}", &m[1], &m[2], &m[3], &m[4]))
}

fn num_like(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::Bool(b)) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

fn str_of<'a>(r: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    r.get(k).and_then(Value::as_str)
}

struct Builder<'a> {
    v: &'a View,
    ai: &'a Ai,
    working: &'a BTreeMap<String, (bool, Vec<String>)>,
    metas: &'a SetMetas,
    perf_edge: IndexMap<String, &'a Map<String, Value>>,
    joke_perfs: IndexMap<String, Vec<String>>,
    touch: IndexMap<String, Touch>,
    who: HashMap<String, &'static str>,
}

/// `joke_stats` per joke: `(avg_vs_room, avg_laugh_s_per_min, last_performed)`.
type Stats = (Option<f64>, Option<f64>, Option<String>);

struct Rooms {
    sets: usize,
    hit_sets: usize,
    ai_hit_sets: usize,
}

impl<'a> Builder<'a> {
    fn new(v: &'a View, ai: &'a Ai, working: &'a BTreeMap<String, (bool, Vec<String>)>, metas: &'a SetMetas) -> Self {
        let mut b = Self {
            v,
            ai,
            working,
            metas,
            perf_edge: IndexMap::new(),
            joke_perfs: IndexMap::new(),
            touch: IndexMap::new(),
            who: HashMap::new(),
        };
        b.perfs();
        b.touches();
        b
    }

    fn perfs(&mut self) {
        let v = self.v;
        for e in v.edges.values() {
            if str_of(e, "kind") != Some("performed_as") {
                continue;
            }
            let (mut p, mut j) = (str_of(e, "src"), str_of(e, "dst"));
            if v.ntype(p) != Some("performance") {
                std::mem::swap(&mut p, &mut j);
            }
            let (Some(p), Some(_)) = (p, j) else { continue };
            if v.ntype(Some(p)) != Some("performance") || v.ntype(j) != Some("joke") {
                continue;
            }
            let replace = self.perf_edge.get(p).is_none_or(|cur| {
                bucket(cur.get("status")) == Bucket::Rejected && bucket(e.get("status")) != Bucket::Rejected
            });
            if replace {
                self.perf_edge.insert(p.to_string(), e);
            }
        }
        for (p, e) in &self.perf_edge {
            if bucket(e.get("status")) != Bucket::Rejected {
                let dst = str_of(e, "dst");
                let j = if v.ntype(dst) == Some("joke") {
                    dst
                } else {
                    str_of(e, "src")
                };
                if let Some(j) = j {
                    self.joke_perfs.entry(j.to_string()).or_default().push(p.clone());
                }
            }
        }
    }

    fn touches(&mut self) {
        let v = self.v;
        for (i, d) in v.decisions.iter().enumerate() {
            let Some(id) = d.id() else { continue };
            if v.is_undone(i) || d.has_undo_of() {
                continue;
            }
            let after = d.after();
            let before = d.before();
            let mut act = d.0.get("action").and_then(Value::as_str).map(str::to_string);
            if act.as_deref() == Some("edit")
                && after
                    .keys()
                    .all(|k| matches!(k.as_str(), "rating" | "rating_auto" | "max_bars"))
                && truthy(after.get("rating"))
                && after.get("rating") == before.get("ai_rating")
            {
                act = Some("confirm".into());
            }
            let t = self.touch.entry(id.to_string()).or_default();
            t.actions.insert(act);
            t.fields.extend(after.keys().cloned());
            for k in ["src", "dst"] {
                if let Some(x) = before.get(k).filter(|x| truthy(Some(x))).and_then(Value::as_str)
                    && !t.ends.iter().any(|e| e == x)
                {
                    t.ends.push(x.to_string());
                }
            }
        }
        let mut who: HashMap<String, &'static str> = HashMap::new();
        for (tid, t) in &self.touch {
            let mut js = self.jokes_of(tid);
            for end in &t.ends {
                if v.ntype(Some(end)) != Some("label") {
                    for j in self.jokes_of(end) {
                        if !js.contains(&j) {
                            js.push(j);
                        }
                    }
                }
            }
            let w = if t.actions.iter().any(|a| a.as_deref() != Some("confirm")) {
                "edited"
            } else {
                "confirmed"
            };
            for j in js {
                if who.get(&j) != Some(&"edited") {
                    who.insert(j, w);
                }
            }
        }
        self.who = who;
    }

    fn joke_of(&self, perf: &str) -> Option<String> {
        let e = self.perf_edge.get(perf)?;
        let dst = str_of(e, "dst");
        let j = if self.v.ntype(dst) == Some("joke") {
            dst
        } else {
            str_of(e, "src")
        };
        j.map(str::to_string)
    }

    fn jokes_of(&self, nid: &str) -> Vec<String> {
        let v = self.v;
        match v.ntype(Some(nid)) {
            Some("joke") => vec![nid.to_string()],
            Some("unit") => v.unit_joke.get(nid).cloned().into_iter().collect(),
            Some("performance") => self.joke_of(nid).into_iter().collect(),
            _ => {
                let Some(e) = v.edges.get(nid) else { return Vec::new() };
                let mut out: Vec<String> = Vec::new();
                for end in [str_of(e, "src"), str_of(e, "dst")].into_iter().flatten() {
                    if v.ntype(Some(end)) != Some("label") {
                        for j in self.jokes_of(end) {
                            if !out.contains(&j) {
                                out.push(j);
                            }
                        }
                    }
                }
                out
            }
        }
    }

    fn perf_rating(p: &Map<String, Value>) -> Option<&Value> {
        if truthy(p.get("rating")) {
            p.get("rating")
        } else if truthy(p.get("ai_rating")) {
            p.get("ai_rating")
        } else {
            None
        }
    }

    fn rooms(&self, jid: &str) -> Rooms {
        let (mut sets, mut hit, mut ai_hits): (HashSet<String>, HashSet<String>, HashSet<String>) = Default::default();
        let empty = Map::new();
        for pid in self.joke_perfs.get(jid).into_iter().flatten() {
            let p = self.v.nodes.get(pid).unwrap_or(&empty);
            if str_of(p, "kind").is_some_and(|k| NOT_COUNTED.contains(&k)) {
                continue;
            }
            let set = pk_core::py::repr(p.get("set").unwrap_or(&Value::Null));
            sets.insert(set.clone());
            if let Some(e) = self.perf_edge.get(pid)
                && bucket(e.get("status")) == Bucket::Confirmed
                && str_of(p, "rating").is_some_and(|r| HIT_RATINGS.contains(&r))
            {
                hit.insert(set.clone());
            }
            if Self::perf_rating(p)
                .and_then(Value::as_str)
                .is_some_and(|r| HIT_RATINGS.contains(&r))
            {
                ai_hits.insert(set);
            }
        }
        let hit_sets = self.working.get(jid).map_or(hit.len(), |w| w.1.len());
        Rooms {
            sets: sets.len(),
            hit_sets,
            ai_hit_sets: ai_hits.len(),
        }
    }

    fn stage_ai(&self, jid: &str) -> (bool, Option<f64>) {
        let empty = Map::new();
        let n = self.v.nodes.get(jid).unwrap_or(&empty);
        let ai = n.get("_ai").and_then(Value::as_object).filter(|a| !a.is_empty());
        let Some(ai) = ai else { return (false, None) };
        let t = self.touch.get(jid);
        let on = n.get("stage") == ai.get("stage")
            && t.is_none_or(|t| !t.fields.contains("stage") && !t.actions.contains(&Some("confirm".to_string())));
        (
            on,
            if on {
                ai.get("stage_confidence").and_then(Value::as_f64)
            } else {
                None
            },
        )
    }

    /// `(show_date, audio_unreliable)` of a set (`AppIndex.set_info`, the two fields the stats read).
    fn set_info(&self, sid: &str) -> (Option<String>, bool) {
        let empty = Map::new();
        let s = self.v.nodes.get(sid).unwrap_or(&empty);
        let meta = set_dir(sid).and_then(|d| self.metas.get(&d)).unwrap_or(&empty);
        let date = [s.get("show_date"), meta.get("show_date")]
            .into_iter()
            .find(|x| truthy(*x))
            .flatten()
            .and_then(Value::as_str)
            .map(str::to_string);
        (date, s.get("audio_unreliable") == Some(&Value::Bool(true)))
    }

    fn stats(&self) -> HashMap<String, Stats> {
        let mut rowmap: HashMap<&str, (&str, &Value)> = HashMap::new();
        for (sid, rows) in &self.ai.rows {
            for (row, pid) in rows {
                if let Some(pid) = pid {
                    rowmap.insert(pid.as_str(), (sid.as_str(), row));
                }
            }
        }
        let empty = Map::new();
        let mut out = HashMap::new();
        for (jid, pids) in &self.joke_perfs {
            // set id -> (laugh_s, window_s, laugh-s/min values)
            let mut nights: BTreeMap<String, (f64, f64, Vec<f64>)> = BTreeMap::new();
            let mut last: Option<String> = None;
            for pid in pids {
                let p = self.v.nodes.get(pid).unwrap_or(&empty);
                if str_of(p, "kind").is_some_and(|k| NOT_COUNTED.contains(&k)) {
                    continue;
                }
                let sid = str_of(p, "set");
                let (show, unreliable) = sid.map_or((None, false), |s| self.set_info(s));
                let day = show.or_else(|| {
                    sid.and_then(|s| self.v.nodes.get(s))
                        .and_then(|n| n.get("date"))
                        .filter(|d| truthy(Some(d)))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
                if let Some(d) = day
                    && last.as_ref().is_none_or(|l| d > *l)
                {
                    last = Some(d);
                }
                if unreliable {
                    continue;
                }
                let Some((_, row)) = rowmap.get(pid.as_str()) else {
                    continue;
                };
                let lg = row.get("laughs").and_then(Value::as_object);
                let n = nights.entry(sid.unwrap_or("").to_string()).or_default();
                let (ls, ws) = (
                    num_like(lg.and_then(|l| l.get("laugh_s"))),
                    num_like(lg.and_then(|l| l.get("window_s"))),
                );
                if let (Some(ls), Some(ws)) = (ls, ws) {
                    n.0 += ls;
                    n.1 += ws;
                } else if let Some(x) = num_like(row.get("laugh_s_per_min")) {
                    n.2.push(x);
                }
            }
            let (mut rates, mut rooms) = (Vec::new(), Vec::new());
            for (sid, (ls, ws, lspm)) in &nights {
                let rate = if *ws > 0.0 {
                    ls / (ws / 60.0)
                } else if !lspm.is_empty() {
                    py_sum(lspm) / lspm.len() as f64
                } else {
                    continue;
                };
                rates.push(rate);
                let set_rate = self
                    .ai
                    .analysis
                    .get(sid)
                    .and_then(|m| m.get("summary"))
                    .and_then(|s| num_like(s.get("laugh_s_per_min")));
                if let Some(sr) = set_rate.filter(|r| *r > 0.0) {
                    let x = rate / sr;
                    rooms.push(if x < VS_ROOM_CLIP { x } else { VS_ROOM_CLIP });
                }
            }
            let avg = |xs: &[f64]| (!xs.is_empty()).then(|| r2(py_sum(xs) / xs.len() as f64));
            out.insert(jid.clone(), (avg(&rooms), avg(&rates), last));
        }
        out
    }

    fn punch(&self, jid: &str) -> Option<String> {
        let mut units: Vec<&Map<String, Value>> = self
            .v
            .joke_units
            .get(jid)
            .into_iter()
            .flatten()
            .filter_map(|(u, _)| self.v.nodes.get(u))
            .filter(|u| truthy(u.get("text")))
            .collect();
        if units.is_empty() {
            let j = self.v.nodes.get(jid)?;
            if !truthy(j.get("_by_tristan")) {
                return None;
            }
            return punch_line(str_of(j, "summary").unwrap_or(""));
        }
        units.sort_by_key(|u| {
            (
                str_of(u, "date").unwrap_or("").to_string(),
                str_of(u, "text").map_or(0, |t| t.chars().count()),
            )
        });
        units.last().and_then(|u| punch_line(str_of(u, "text").unwrap_or("")))
    }

    fn summaries(&self) -> Vec<Row> {
        let stats = self.stats();
        let mut out = Vec::with_capacity(self.v.jokes.len());
        for s in self.v.joke_summaries() {
            let id = s.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let title = match s.get("title") {
                Some(Value::String(t)) => t.clone(),
                Some(other) => pk_core::py::str(other),
                None => id.clone(),
            };
            let rooms = self.rooms(&id);
            let mut chips: Vec<LabelChip> = s
                .get("labels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|l| LabelChip {
                    id: l.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
                    name: l.get("name").and_then(Value::as_str).map(str::to_string),
                    bucket: l.get("bucket").and_then(Value::as_str).unwrap_or("").to_string(),
                })
                .collect();
            let label_ids: Vec<String> = chips.iter().map(|c| c.id.clone()).collect();
            chips.sort_by(|a, b| {
                (a.bucket != "confirmed", a.name.as_deref().unwrap_or(""))
                    .cmp(&(b.bucket != "confirmed", b.name.as_deref().unwrap_or("")))
            });
            chips.truncate(6);
            let (stage_ai, stage_confidence) = self.stage_ai(&id);
            let (avg_vs_room, avg_lspm, last_performed) = stats.get(&id).cloned().unwrap_or_default();
            let yours = s.get("yours") == Some(&Value::Bool(true));
            let node_short = self.v.nodes.get(&id).and_then(|n| str_of(n, "short_name"));
            let get_str = |k: &str| s.get(k).and_then(Value::as_str).map(str::to_string);
            let summary = JokeSummary {
                title: title.clone(),
                stage: get_str("stage"),
                summary: if truthy(s.get("summary")) {
                    s.get("summary").map_or_else(String::new, pk_core::py::str)
                } else {
                    String::new()
                },
                first_seen: get_str("first_seen"),
                last_seen: get_str("last_seen"),
                touched: get_str("touched"),
                versions: s.get("units").and_then(Value::as_u64).unwrap_or(0),
                sets: rooms.sets as u64,
                hit_sets: rooms.hit_sets as u64,
                ai_hit_sets: rooms.ai_hit_sets as u64,
                to_review: s
                    .get("counts")
                    .and_then(|c| c.get("proposed"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                labels: chips,
                yours,
                stage_ai,
                stage_confidence,
                who: if yours {
                    "edited".into()
                } else {
                    self.who.get(&id).copied().unwrap_or("ai").into()
                },
                short_title: short_title(&title),
                punch: self.punch(&id),
                short_name: short_name_of(&title, node_short),
                nights: rooms.sets as u64,
                avg_vs_room,
                avg_laugh_s_per_min: avg_lspm,
                last_performed,
                duration_s: None,
                duration_estimated: None,
                id,
            };
            out.push(Row { summary, label_ids });
        }
        let inv = |s: Option<&String>| -> Vec<i64> {
            s.map(|x| x.chars().map(|c| 255 - i64::from(u32::from(c))).collect())
                .unwrap_or_default()
        };
        out.sort_by_cached_key(|r| {
            let x = &r.summary;
            (
                x.stage
                    .as_deref()
                    .and_then(|s| STAGE_ORDER.iter().position(|o| *o == s))
                    .unwrap_or(9),
                std::cmp::Reverse(x.sets),
                inv(x.last_seen.as_ref().filter(|s| !s.is_empty())),
                x.title.to_lowercase(),
            )
        });
        out
    }
}

impl AppIndex {
    /// Index `view`; `working` is the database's Working per joke (`joke.working`, `joke.working_sets`).
    pub fn build(view: Arc<View>, ai: &Ai, working: &BTreeMap<String, (bool, Vec<String>)>, metas: &SetMetas) -> Self {
        let (summaries, sets, performances) = {
            let b = Builder::new(&view, ai, working, metas);
            let mut sets: Vec<(&str, &String)> = view
                .nodes
                .iter()
                .filter(|(_, n)| str_of(n, "type") == Some("set"))
                .map(|(id, n)| (str_of(n, "date").unwrap_or(""), id))
                .collect();
            sets.sort_by_key(|(d, _)| *d);
            let performances = view
                .nodes
                .values()
                .filter(|n| str_of(n, "type") == Some("performance"))
                .count();
            (
                b.summaries(),
                sets.into_iter().map(|(_, id)| id.clone()).collect(),
                performances,
            )
        };
        Self {
            view,
            sets,
            performances,
            summaries,
        }
    }
}

/// The ids of the `hidden: true` nodes (jokes and units the comic hid).
pub fn hidden_ids(snap: &Rows) -> HashSet<&str> {
    snap.nodes
        .iter()
        .filter(|n| n.get("hidden") == Some(&Value::Bool(true)))
        .filter_map(|n| n.get("id").and_then(Value::as_str))
        .collect()
}

/// The view over the database rows (`Snapshot::view`): everything, or `visible_view` (without `hidden: true` jokes
/// and units and the edges that touch them, re-indexed) when `include_hidden` is false. Built straight from the rows
/// with [`View::from_records`]: one clone of each record and no `graph.json` value in between.
pub fn view_of(snap: &Rows, owner_id: &str, include_hidden: bool) -> View {
    let hidden = if include_hidden {
        HashSet::new()
    } else {
        hidden_ids(snap)
    };
    let shown = |id: Option<&Value>| !id.and_then(Value::as_str).is_some_and(|s| hidden.contains(s));
    let meta = snap.load_info.get("graph_meta").unwrap_or(&Value::Null);
    let mut v = View::from_records(
        meta,
        snap.nodes.iter().filter(|n| shown(n.get("id"))).map(|n| (**n).clone()),
        snap.edges
            .iter()
            .filter(|e| shown(e.get("src")) && shown(e.get("dst")))
            .map(|e| (**e).clone()),
        Vec::new(),
        Vec::new(),
        owner_id,
    );
    v.undone = snap.decisions.iter().filter_map(|d| d.undo_of()).collect();
    v.decisions = snap.decisions.iter().map(|d| (**d).clone()).collect();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(hidden: bool) -> pk_store::Snapshot {
        let rec = |v: Value| match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        };
        let d = |v: Value| pk_kb::Decision(rec(v));
        pk_store::Snapshot {
            generation: Some(3),
            loading: false,
            orphans: 0,
            load_info: serde_json::json!({"graph_meta": {"version": 2, "generated_at": "2026-01-01", "runs": ["r"]}}),
            nodes: vec![
                rec(serde_json::json!({"id": "joke:a", "type": "joke", "title": "A", "stage": "idea"})),
                rec(serde_json::json!({"id": "joke:b", "type": "joke", "title": "B", "hidden": hidden})),
                rec(serde_json::json!({"id": "label:x", "type": "label", "name": "x"})),
                rec(serde_json::json!({"id": "unit:b1", "type": "unit", "text": "b one"})),
            ],
            edges: vec![
                rec(serde_json::json!({"id": "e1", "src": "joke:a", "kind": "tagged", "dst": "label:x"})),
                rec(serde_json::json!({"id": "e2", "src": "joke:b", "kind": "tagged", "dst": "label:x"})),
                rec(serde_json::json!({"id": "e3", "src": "unit:b1", "kind": "part_of", "dst": "joke:b"})),
            ],
            decisions: vec![
                d(serde_json::json!({"target": "node", "id": "joke:a", "action": "edit", "after": {"stage": "idea"}})),
                d(serde_json::json!({"target": "node", "id": "joke:a", "action": "edit", "undo_of": 0})),
            ],
            working: BTreeMap::new(),
        }
    }

    /// The previous build of the visible view: a `graph.json` value, then `View::new` (the reference `view_of` must
    /// equal, field for field and in order).
    fn visible_by_json(snap: &pk_store::Snapshot, owner: &str) -> View {
        let rows = Rows::from(snap.clone());
        let hidden = hidden_ids(&rows);
        let shown = |id: Option<&Value>| !id.and_then(Value::as_str).is_some_and(|s| hidden.contains(s));
        let meta = snap.load_info.get("graph_meta").cloned().unwrap_or(Value::Null);
        let g = serde_json::json!({
            "version": meta.get("version").cloned().unwrap_or(Value::Null),
            "generated_at": meta.get("generated_at").cloned().unwrap_or(Value::Null),
            "runs": meta.get("runs").cloned().unwrap_or(Value::Null),
            "nodes": snap.nodes.iter().filter(|n| shown(n.get("id"))).collect::<Vec<_>>(),
            "edges": snap.edges.iter().filter(|e| shown(e.get("src")) && shown(e.get("dst"))).collect::<Vec<_>>(),
        });
        let mut v = View::new(&g, Vec::new(), Vec::new(), owner);
        v.undone = snap.decisions.iter().filter_map(pk_kb::Decision::undo_of).collect();
        v.decisions.clone_from(&snap.decisions);
        v
    }

    #[test]
    fn view_of_is_the_json_built_view() {
        for hidden in [true, false] {
            let s = snap(hidden);
            let rows = Rows::from(s.clone());
            assert_eq!(hidden_ids(&rows).len(), usize::from(hidden));
            let whole = format!("{:?}", view_of(&rows, "owner", true));
            assert_eq!(whole, format!("{:?}", s.view(Vec::new(), "owner")));
            let vis = view_of(&rows, "owner", false);
            assert_eq!(format!("{vis:?}"), format!("{:?}", visible_by_json(&s, "owner")));
            assert_eq!(vis.nodes.contains_key("joke:b"), !hidden);
            assert_eq!(vis.edges.len(), if hidden { 1 } else { 3 });
            assert_eq!(vis.undone, vec![0]);
        }
    }

    #[test]
    fn set_dirs() {
        assert_eq!(set_dir("set:2024-01-01_x").as_deref(), Some("20240101_x"));
        assert_eq!(set_dir("20240101_x-y").as_deref(), Some("20240101_x-y"));
        assert_eq!(set_dir("set:2024-1-01_x"), None);
        assert_eq!(set_dir("set:2024-01-01_"), None);
    }
}
