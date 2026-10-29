//! The read models over a [`View`]: what the API serves for a node, an edge, the Bits list, a search, one bit's
//! page and the label index. Mirrors `tools/explorer/kb.py` `brief`, `edge_view`, `joke_summaries`, `search`,
//! `joke_detail`, `label_index`.

use std::cmp::Ordering;

use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::decision::actor_of;
use crate::model::{Bucket, LABEL_KINDS, Rec, bucket, ekind, head, public, truthy, truthy_str};
use crate::view::View;

/// Laughs on a stretch of tape: `(set id, start_ms, end_ms) -> the laughs doc`, or None when there is no tape data.
/// Storage provides it; the pure view only calls it.
pub trait LaughsFor {
    /// The laughs between `start_ms` and `end_ms` (+3 s) of `set_id`, as the API shows them.
    fn laughs_for(&self, set_id: Option<&str>, start_ms: Option<&Value>, end_ms: Option<&Value>) -> Value;
}

/// No tape data at all.
pub struct NoLaughs;

impl LaughsFor for NoLaughs {
    fn laughs_for(&self, _: Option<&str>, _: Option<&Value>, _: Option<&Value>) -> Value {
        Value::Null
    }
}

impl View {
    /// A one-line card for any node: id, type, a title, and the fields a row needs.
    pub fn brief(&self, nid: Option<&str>) -> Map<String, Value> {
        let idv = nid.map_or(Value::Null, |s| Value::String(s.to_string()));
        let Some(n) = nid.and_then(|id| self.nodes.get(id)) else {
            let mut b = Map::new();
            b.insert("id".into(), idv.clone());
            b.insert("type".into(), json!("missing"));
            b.insert("title".into(), idv);
            return b;
        };
        let nid = nid.expect("present");
        let t = n.get("type").and_then(Value::as_str);
        let mut b = Map::new();
        b.insert("id".into(), json!(nid));
        b.insert("type".into(), n.get("type").cloned().unwrap_or(Value::Null));
        let get = |k: &str| n.get(k).cloned().unwrap_or(Value::Null);
        let or_id = |k: &str| if truthy(n.get(k)) { get(k) } else { json!(nid) };
        match t {
            Some("joke") => {
                b.insert("title".into(), or_id("title"));
                b.insert("stage".into(), get("stage"));
                b.insert("summary".into(), get("summary"));
            }
            Some("label") => {
                b.insert("title".into(), or_id("name"));
                b.insert("kind".into(), get("kind"));
                b.insert("definition".into(), get("definition"));
            }
            Some("unit") => {
                b.insert(
                    "title".into(),
                    json!(head(truthy_str(n.get("text")).unwrap_or(""), 140)),
                );
                b.insert("kind".into(), get("kind"));
                b.insert("date".into(), get("date"));
                b.insert("doc_title".into(), get("doc_title"));
                let j = self.unit_joke.get(nid);
                b.insert("joke".into(), j.map_or(Value::Null, |s| json!(s)));
                if let Some(j) = j {
                    b.insert(
                        "joke_title".into(),
                        self.nodes
                            .get(j)
                            .and_then(|x| x.get("title"))
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                }
            }
            Some("performance") => {
                let set = n.get("set").and_then(Value::as_str).and_then(|s| self.nodes.get(s));
                let sget = |k: &str| set.and_then(|s| s.get(k)).cloned().unwrap_or(Value::Null);
                b.insert(
                    "title".into(),
                    json!(head(truthy_str(n.get("text")).unwrap_or(""), 140)),
                );
                b.insert("set".into(), get("set"));
                b.insert("date".into(), sget("date"));
                b.insert("venue".into(), sget("venue"));
                b.insert("start_ms".into(), get("start_ms"));
                b.insert("end_ms".into(), get("end_ms"));
            }
            Some("set") => {
                b.insert("title".into(), or_id("title"));
                b.insert("date".into(), get("date"));
                b.insert("venue".into(), get("venue"));
            }
            _ => {
                let title = if truthy(n.get("title")) {
                    get("title")
                } else if truthy(n.get("name")) {
                    get("name")
                } else {
                    json!(nid)
                };
                b.insert("title".into(), title);
            }
        }
        if truthy(n.get("_by_tristan")) {
            b.insert("yours".into(), Value::Bool(true));
        }
        b
    }

    /// An edge as the app sees it: the public fields plus bucket, display kind, who made it, both ends' briefs and,
    /// with a `focus` node, the other end and the direction.
    pub fn edge_view(&self, e: &Rec, focus: Option<&str>) -> Map<String, Value> {
        let mut v = public(e);
        v.insert("bucket".into(), json!(bucket(e.get("status")).as_str()));
        v.insert("display_kind".into(), json!(ekind(e)));
        v.insert("yours".into(), Value::Bool(truthy(e.get("_by_tristan"))));
        v.insert("edited".into(), Value::Bool(truthy(e.get("_edited"))));
        let src = e.get("src").and_then(Value::as_str);
        let dst = e.get("dst").and_then(Value::as_str);
        v.insert("src_brief".into(), Value::Object(self.brief(src)));
        v.insert("dst_brief".into(), Value::Object(self.brief(dst)));
        if truthy(e.get("_merged_from")) {
            v.insert("merged_from".into(), e["_merged_from"].clone());
        }
        if let Some(f) = focus {
            let (other, dir) = if src == Some(f) { (dst, "out") } else { (src, "in") };
            v.insert("other".into(), Value::Object(self.brief(other)));
            v.insert("direction".into(), json!(dir));
        }
        v
    }

    /// The Bits list: one row per joke with its counts, labels, when it was last touched, how many versions.
    pub fn joke_summaries(&self) -> Vec<Map<String, Value>> {
        let mut out = Vec::with_capacity(self.jokes.len());
        for jid in &self.jokes {
            let j = &self.nodes[jid];
            let mut counts = IndexMap::from([("proposed", 0u64), ("confirmed", 0), ("rejected", 0)]);
            let mut labels: IndexMap<String, Bucket> = IndexMap::new();
            for (_, e) in self.joke_edges(jid) {
                let b = bucket(e.get("status"));
                *counts.get_mut(b.as_str()).expect("key") += 1;
                if e.get("kind").and_then(Value::as_str) == Some("has_label")
                    && b != Bucket::Rejected
                    && let Some(lid) = e.get("dst").and_then(Value::as_str)
                    && self.ntype(Some(lid)) == Some("label")
                    && labels.get(lid) != Some(&Bucket::Confirmed)
                {
                    labels.insert(lid.to_string(), b);
                }
            }
            let touched = j
                .get("_last")
                .and_then(Value::as_u64)
                .and_then(|i| self.decisions.get(usize::try_from(i).ok()?))
                .map_or(Value::Null, |d| d.ts().map_or(Value::Null, |t| json!(t)));
            let mut row = Map::new();
            row.insert("id".into(), json!(jid));
            row.insert(
                "title".into(),
                if truthy(j.get("title")) {
                    j["title"].clone()
                } else {
                    json!(jid)
                },
            );
            row.insert("stage".into(), j.get("stage").cloned().unwrap_or(Value::Null));
            row.insert("summary".into(), j.get("summary").cloned().unwrap_or(Value::Null));
            row.insert("first_seen".into(), j.get("first_seen").cloned().unwrap_or(Value::Null));
            row.insert("last_seen".into(), j.get("last_seen").cloned().unwrap_or(Value::Null));
            row.insert("touched".into(), touched);
            row.insert("counts".into(), json!(counts));
            row.insert(
                "labels".into(),
                Value::Array(
                    labels
                        .iter()
                        .map(|(k, b)| {
                            json!({"id": k, "name": self.nodes.get(k).and_then(|n| n.get("name")).cloned().unwrap_or(Value::Null), "bucket": b.as_str()})
                        })
                        .collect(),
                ),
            );
            row.insert("units".into(), json!(self.joke_units.get(jid).map_or(0, Vec::len)));
            row.insert("yours".into(), Value::Bool(truthy(j.get("_by_tristan"))));
            out.push(row);
        }
        out
    }

    /// Joke ids whose title, summary, id, versions or labels contain every word of `q` (the files backend's search).
    pub fn search(&self, q: &str) -> Vec<String> {
        let lower = q.to_lowercase();
        let words: Vec<&str> = lower.split_whitespace().collect();
        if words.is_empty() {
            return self.jokes.clone();
        }
        let mut hits = Vec::new();
        for jid in &self.jokes {
            let j = &self.nodes[jid];
            let mut parts: Vec<String> = vec![
                truthy_str(j.get("title")).unwrap_or("").to_string(),
                truthy_str(j.get("summary")).unwrap_or("").to_string(),
                jid.clone(),
            ];
            for (u, _) in self.joke_units.get(jid).into_iter().flatten() {
                parts.push(
                    self.nodes
                        .get(u)
                        .and_then(|n| truthy_str(n.get("text")))
                        .unwrap_or("")
                        .to_string(),
                );
            }
            for (_, e) in self.joke_edges(jid) {
                if e.get("kind").and_then(Value::as_str) == Some("has_label")
                    && bucket(e.get("status")) != Bucket::Rejected
                {
                    let ln = e.get("dst").and_then(Value::as_str).and_then(|d| self.nodes.get(d));
                    parts.push(ln.and_then(|n| truthy_str(n.get("name"))).unwrap_or("").to_string());
                    parts.push(ln.and_then(|n| truthy_str(n.get("kind"))).unwrap_or("").to_string());
                }
            }
            let hay = parts.join(" ").to_lowercase();
            if words.iter().all(|w| hay.contains(w)) {
                hits.push(jid.clone());
            }
        }
        hits
    }

    /// One bit's page: the joke, its versions grouped by date, its tapes, its other connections, its history.
    pub fn joke_detail(&self, jid: &str, laughs: &dyn LaughsFor) -> Option<Map<String, Value>> {
        let j = self.nodes.get(jid)?;
        if j.get("type").and_then(Value::as_str) != Some("joke") {
            return None;
        }
        let mut units: Vec<Map<String, Value>> = Vec::new();
        for (uid, peid) in self.joke_units.get(jid).into_iter().flatten() {
            let mut u = self.nodes.get(uid).map(public).unwrap_or_default();
            u.insert(
                "edge".into(),
                Value::Object(self.edge_view(&self.edges[peid], Some(jid))),
            );
            units.push(u);
        }
        units.sort_by(|a, b| {
            let ka = (str_or_empty(a.get("date")), str_or_empty(a.get("id")));
            let kb = (str_or_empty(b.get("date")), str_or_empty(b.get("id")));
            ka.cmp(&kb)
        });
        let mut groups: IndexMap<String, Vec<Value>> = IndexMap::new();
        for u in units {
            let key = truthy_str(u.get("date")).unwrap_or("undated").to_string();
            groups.entry(key).or_default().push(Value::Object(u));
        }
        let scope = self.joke_scope(jid);
        let mut perfs: Vec<Map<String, Value>> = Vec::new();
        let mut edges: Vec<Map<String, Value>> = Vec::new();
        for (_, e) in self.joke_edges(jid) {
            let src = e.get("src").and_then(Value::as_str);
            let dst = e.get("dst").and_then(Value::as_str);
            let focus = (src == Some(jid) || dst == Some(jid)).then_some(jid);
            let mut ev = self.edge_view(e, focus);
            if ev.get("other").is_none() {
                // an edge on one of the joke's units
                let unit_end = if src.is_some_and(|s| scope.iter().any(|x| x == s)) {
                    src
                } else {
                    dst
                };
                ev.insert("via_unit".into(), Value::Object(self.brief(unit_end)));
                let other = if unit_end == src { dst } else { src };
                ev.insert("other".into(), Value::Object(self.brief(other)));
            }
            let kind = e.get("kind").and_then(Value::as_str);
            let other_type = ev.get("other").and_then(|o| o.get("type")).and_then(Value::as_str);
            if kind == Some("part_of") && other_type == Some("unit") && (dst == Some(jid) || src == Some(jid)) {
                continue; // shown in the units section
            }
            if kind == Some("performed_as") && other_type == Some("performance") {
                let pid = ev.get("other").and_then(|o| o.get("id")).and_then(Value::as_str);
                let p = pid.and_then(|id| self.nodes.get(id)).cloned().unwrap_or_default();
                let set_id = p.get("set").and_then(Value::as_str);
                let s = set_id.and_then(|s| self.nodes.get(s));
                let set_v = s.map_or_else(
                    || json!({"id": p.get("set").cloned().unwrap_or(Value::Null)}),
                    |s| Value::Object(public(s)),
                );
                let l = laughs.laughs_for(set_id, p.get("start_ms"), p.get("end_ms"));
                let mut row = Map::new();
                row.insert("perf".into(), Value::Object(public(&p)));
                row.insert("set".into(), set_v);
                row.insert("edge".into(), Value::Object(ev));
                row.insert("laughs".into(), l);
                perfs.push(row);
                continue;
            }
            edges.push(ev);
        }
        perfs.sort_by(|a, b| {
            let ka = (
                str_or_empty(a["set"].get("date")),
                num_or_zero(a["perf"].get("start_ms")),
            );
            let kb = (
                str_or_empty(b["set"].get("date")),
                num_or_zero(b["perf"].get("start_ms")),
            );
            ka.0.cmp(&kb.0).then(ka.1.partial_cmp(&kb.1).unwrap_or(Ordering::Equal))
        });
        edges.sort_by(|a, b| {
            let rank = |m: &Map<String, Value>| match m.get("bucket").and_then(Value::as_str) {
                Some("proposed") => 0,
                Some("confirmed") => 1,
                _ => 2,
            };
            rank(a)
                .cmp(&rank(b))
                .then_with(|| str_or_empty(a.get("display_kind")).cmp(&str_or_empty(b.get("display_kind"))))
                .then_with(|| {
                    num_or_zero(b.get("confidence"))
                        .partial_cmp(&num_or_zero(a.get("confidence")))
                        .unwrap_or(Ordering::Equal)
                })
        });
        let edge_ids: std::collections::HashSet<&String> = self.joke_edges(jid).into_iter().map(|(k, _)| k).collect();
        let mut history: Vec<Value> = Vec::new();
        for (i, d) in self.decisions.iter().enumerate() {
            let hit = d
                .id()
                .is_some_and(|id| scope.iter().any(|x| x == id) || edge_ids.contains(&id.to_string()));
            if hit {
                let mut m = d.0.clone();
                m.insert("index".into(), json!(i));
                m.insert("undone".into(), Value::Bool(self.is_undone(i)));
                history.push(Value::Object(m));
            }
        }
        let history = if history.len() > 30 {
            history.split_off(history.len() - 30)
        } else {
            history
        };
        let mut out = Map::new();
        out.insert("joke".into(), Value::Object(public(j)));
        out.insert("yours".into(), Value::Bool(truthy(j.get("_by_tristan"))));
        out.insert(
            "unit_groups".into(),
            Value::Array(
                groups
                    .into_iter()
                    .map(|(k, v)| json!({"date": k, "units": v}))
                    .collect(),
            ),
        );
        out.insert(
            "performances".into(),
            Value::Array(perfs.into_iter().map(Value::Object).collect()),
        );
        out.insert(
            "edges".into(),
            Value::Array(edges.into_iter().map(Value::Object).collect()),
        );
        out.insert("history".into(), Value::Array(history));
        Some(out)
    }

    /// Every live label with the jokes that carry it, in kind order then name order.
    pub fn label_index(&self) -> Vec<Map<String, Value>> {
        let mut labs: Vec<&String> = self.labels.iter().collect();
        labs.sort_by_key(|lid| {
            let n = &self.nodes[*lid];
            let kind = n.get("kind").and_then(Value::as_str).unwrap_or("");
            let ki = LABEL_KINDS.iter().position(|k| *k == kind).unwrap_or(99);
            (ki, truthy_str(n.get("name")).unwrap_or("").to_lowercase())
        });
        let mut out = Vec::new();
        for lid in labs {
            let lab = &self.nodes[lid];
            let mut carriers: Vec<Value> = Vec::new();
            for eid in self.adj.get(lid).into_iter().flatten() {
                let e = &self.edges[eid];
                if e.get("kind").and_then(Value::as_str) != Some("has_label")
                    || e.get("dst").and_then(Value::as_str) != Some(lid)
                {
                    continue;
                }
                let mut src = e.get("src").and_then(Value::as_str);
                let mut via: Option<&str> = None;
                if self.ntype(src) == Some("unit") {
                    via = src;
                    src = src.and_then(|s| self.unit_joke.get(s).map(String::as_str)).or(src);
                }
                carriers.push(json!({
                    "edge": eid, "bucket": bucket(e.get("status")).as_str(), "confidence": e.get("confidence").cloned().unwrap_or(Value::Null),
                    "joke": self.brief(src), "via_unit": via,
                }));
            }
            let mut merged: Vec<&String> = self.alias.iter().filter(|(_, v)| *v == lid).map(|(k, _)| k).collect();
            merged.sort();
            let count = |b: &str| carriers.iter().filter(|c| c["bucket"] == b).count();
            let mut row = public(lab);
            row.insert("yours".into(), Value::Bool(truthy(lab.get("_by_tristan"))));
            row.insert("carriers".into(), Value::Array(carriers.clone()));
            row.insert("merged_from".into(), json!(merged));
            row.insert(
                "counts".into(),
                json!({"proposed": count("proposed"), "confirmed": count("confirmed"), "rejected": count("rejected")}),
            );
            out.push(row);
        }
        out
    }

    /// Who made each decision (`by`, corrected for the legacy importer lines).
    pub fn actors(&self) -> Vec<Option<&str>> {
        self.decisions.iter().map(|d| actor_of(d, &self.owner_id)).collect()
    }
}

fn str_or_empty(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).unwrap_or("").to_string()
}

fn num_or_zero(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(0.0)
}
