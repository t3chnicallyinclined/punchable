//! The replay: `graph.json` plus every decision, in order, into the state the app reads.
//!
//! Mirrors `tools/explorer/kb.py` `View.__init__`, `_apply`, `_resolve_merges` and `_index`, including the private
//! markers the read models rely on (`_by_tristan`, `_last`, `_edited`, `_merged_from`). The Python keeps records in
//! dicts in insertion order; so does this (`IndexMap`), because the order reaches the app (the Bits list, versions).

use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::decision::Decision;
use crate::model::{Bucket, Rec, bucket, ekind, truthy, truthy_str};

/// The replayed state. Built from scratch whenever an input changes; never mutated by a read.
#[derive(Debug, Clone)]
pub struct View {
    /// `{version, generated_at, runs}` of the graph.
    pub graph_meta: Map<String, Value>,
    /// Every node by id, in graph order then decision order.
    pub nodes: IndexMap<String, Rec>,
    /// Every edge by id (after duplicates from a label merge are dropped).
    pub edges: IndexMap<String, Rec>,
    /// The decisions, as replayed.
    pub decisions: Vec<Decision>,
    /// Edge ids the KB report lists under "first 50", in order.
    pub report: Vec<String>,
    /// Decisions whose target no longer existed and could not be brought back from `before`.
    pub orphans: usize,
    /// Indexes of decisions that a later line undid.
    pub undone: Vec<usize>,
    /// A merged label -> the label it now is (chains resolved).
    pub alias: IndexMap<String, String>,
    /// Node id -> the ids of the edges touching it, in edge order. A null end is keyed `"null"`.
    pub adj: IndexMap<String, Vec<String>>,
    /// Joke ids, in node order.
    pub jokes: Vec<String>,
    /// Label ids that are not merged away, in node order.
    pub labels: Vec<String>,
    /// Unit id -> its joke (through a `part_of` edge that is not rejected).
    pub unit_joke: IndexMap<String, String>,
    /// Joke id -> `(unit id, edge id)` for every `part_of` edge, rejected ones included.
    pub joke_units: IndexMap<String, Vec<(String, String)>>,
    /// Node id -> how many edges touch it.
    pub degree: IndexMap<String, usize>,
    /// The actor id on the comic's own edits.
    pub owner_id: String,
}

impl View {
    /// Replay `graph` (an object with `nodes` and `edges` arrays) and `decisions`.
    pub fn new(graph: &Value, decisions: Vec<Decision>, report: Vec<String>, owner_id: &str) -> Self {
        let mut graph_meta = Map::new();
        for k in ["version", "generated_at", "runs"] {
            graph_meta.insert(k.to_string(), graph.get(k).cloned().unwrap_or(Value::Null));
        }
        let mut nodes = IndexMap::new();
        for n in graph.get("nodes").and_then(Value::as_array).into_iter().flatten() {
            if let Value::Object(o) = n
                && let Some(id) = truthy_str(o.get("id"))
            {
                nodes.insert(id.to_string(), o.clone());
            }
        }
        let mut edges = IndexMap::new();
        for e in graph.get("edges").and_then(Value::as_array).into_iter().flatten() {
            if let Value::Object(o) = e
                && let Some(id) = truthy_str(o.get("id"))
            {
                edges.insert(id.to_string(), o.clone());
            }
        }
        let undone: Vec<usize> = decisions.iter().filter_map(Decision::undo_of).collect();
        let mut v = Self {
            graph_meta,
            nodes,
            edges,
            decisions,
            report,
            orphans: 0,
            undone,
            alias: IndexMap::new(),
            adj: IndexMap::new(),
            jokes: Vec::new(),
            labels: Vec::new(),
            unit_joke: IndexMap::new(),
            joke_units: IndexMap::new(),
            degree: IndexMap::new(),
            owner_id: owner_id.to_string(),
        };
        let ds = v.decisions.clone();
        for (i, d) in ds.iter().enumerate() {
            v.apply(i, d);
        }
        v.resolve_merges();
        v.index();
        v
    }

    /// Whether decision `i` was undone.
    pub fn is_undone(&self, i: usize) -> bool {
        self.undone.contains(&i)
    }

    fn apply(&mut self, i: usize, d: &Decision) {
        let after = d.after();
        let before = d.before();
        let Some(did) = d.id().map(str::to_string) else { return };
        let is_edge = match d.target() {
            Some("edge") => true,
            Some("node") => false,
            _ => return,
        };
        let store = if is_edge { &mut self.edges } else { &mut self.nodes };
        match d.action() {
            Some("add") => {
                let mut rec = after;
                rec.insert("id".into(), Value::String(did.clone()));
                if is_edge && !rec.contains_key("status") {
                    rec.insert("status".into(), json!("confirmed"));
                }
                rec.insert("_by_tristan".into(), Value::Bool(true));
                rec.insert("_last".into(), json!(i));
                store.insert(did, rec);
            }
            Some("delete") => {
                store.shift_remove(&did);
            }
            Some(action @ ("confirm" | "reject" | "edit")) => {
                if !store.contains_key(&did) {
                    // the pipeline dropped it on a re-run; the decision keeps it alive
                    let revive = if is_edge {
                        truthy(before.get("src")) && truthy(before.get("dst"))
                    } else {
                        truthy(before.get("type"))
                    };
                    if revive {
                        let mut rec = before;
                        rec.insert("id".into(), Value::String(did.clone()));
                        store.insert(did.clone(), rec);
                    } else {
                        self.orphans += 1;
                        return;
                    }
                }
                let rec = store.get_mut(&did).expect("present");
                for (k, v) in &after {
                    if k != "id" {
                        rec.insert(k.clone(), v.clone());
                    }
                }
                rec.insert("_last".into(), json!(i));
                if is_edge {
                    if d.has_undo_of() {
                        rec.shift_remove("_edited");
                    } else if action == "edit" {
                        rec.insert("_edited".into(), Value::Bool(true));
                    }
                }
            }
            _ => {}
        }
    }

    fn resolve_merges(&mut self) {
        let mut raw: IndexMap<String, String> = IndexMap::new();
        for (nid, n) in &self.nodes {
            if let Some(into) = truthy_str(n.get("merged_into"))
                && self.nodes.contains_key(into)
                && into != nid
            {
                raw.insert(nid.clone(), into.to_string());
            }
        }
        let resolve = |mut x: String| {
            let mut seen = std::collections::HashSet::new();
            while let Some(next) = raw.get(&x) {
                if !seen.insert(x.clone()) {
                    break;
                }
                x.clone_from(next);
            }
            x
        };
        self.alias = raw.keys().map(|k| (k.clone(), resolve(k.clone()))).collect();
        if self.alias.is_empty() {
            return;
        }
        // one edge per (src, kind, dst) after aliasing: the better status wins, the other is dropped
        let mut best: IndexMap<(Option<String>, String, Option<String>), String> = IndexMap::new();
        let ids: Vec<String> = self.edges.keys().cloned().collect();
        for eid in &ids {
            let e = self.edges.get(eid).expect("present");
            let src = e.get("src").and_then(Value::as_str).map(str::to_string);
            let dst = e.get("dst").and_then(Value::as_str).map(str::to_string);
            let s = src
                .as_ref()
                .and_then(|x| self.alias.get(x).cloned())
                .or_else(|| src.clone());
            let t = dst
                .as_ref()
                .and_then(|x| self.alias.get(x).cloned())
                .or_else(|| dst.clone());
            let kind = ekind(e);
            let changed = s != src || t != dst;
            let this_bucket = bucket(e.get("status"));
            if changed {
                let e = self.edges.get_mut(eid).expect("present");
                e.insert("_merged_from".into(), json!({"src": src, "dst": dst}));
                e.insert("src".into(), s.clone().map_or(Value::Null, Value::String));
                e.insert("dst".into(), t.clone().map_or(Value::Null, Value::String));
            }
            let key = (s, kind, t);
            if let Some(other_id) = best.get(&key).cloned() {
                let other_bucket = bucket(self.edges.get(&other_id).and_then(|o| o.get("status")));
                if this_bucket.rank() > other_bucket.rank() {
                    self.edges
                        .get_mut(&other_id)
                        .expect("present")
                        .insert("_dup_of".into(), json!(eid));
                    best.insert(key, eid.clone());
                } else {
                    self.edges
                        .get_mut(eid)
                        .expect("present")
                        .insert("_dup_of".into(), json!(other_id));
                }
            } else {
                best.insert(key, eid.clone());
            }
        }
        self.edges.retain(|_, e| !truthy(e.get("_dup_of")));
    }

    fn index(&mut self) {
        let mut adj: IndexMap<String, Vec<String>> = IndexMap::new();
        for (eid, e) in &self.edges {
            for end in ["src", "dst"] {
                adj.entry(end_key(e.get(end))).or_default().push(eid.clone());
            }
        }
        self.jokes = self
            .nodes
            .iter()
            .filter(|(_, n)| ntype_is(n, "joke"))
            .map(|(id, _)| id.clone())
            .collect();
        self.labels = self
            .nodes
            .iter()
            .filter(|(id, n)| ntype_is(n, "label") && !self.alias.contains_key(id.as_str()))
            .map(|(id, _)| id.clone())
            .collect();
        let mut unit_joke = IndexMap::new();
        let mut joke_units: IndexMap<String, Vec<(String, String)>> = IndexMap::new();
        for (eid, e) in &self.edges {
            if e.get("kind").and_then(Value::as_str) != Some("part_of") {
                continue;
            }
            let (mut u, mut j) = (
                e.get("src").and_then(Value::as_str),
                e.get("dst").and_then(Value::as_str),
            );
            if self.ntype(u) == Some("joke") && self.ntype(j) == Some("unit") {
                std::mem::swap(&mut u, &mut j);
            }
            if self.ntype(u) == Some("unit")
                && self.ntype(j) == Some("joke")
                && let (Some(u), Some(j)) = (u, j)
            {
                joke_units
                    .entry(j.to_string())
                    .or_default()
                    .push((u.to_string(), eid.clone()));
                if bucket(e.get("status")) != Bucket::Rejected {
                    unit_joke.insert(u.to_string(), j.to_string());
                }
            }
        }
        self.degree = adj.iter().map(|(k, v)| (k.clone(), v.len())).collect();
        self.adj = adj;
        self.unit_joke = unit_joke;
        self.joke_units = joke_units;
    }

    /// A node's `type`, if the node exists.
    pub fn ntype(&self, nid: Option<&str>) -> Option<&str> {
        nid.and_then(|id| self.nodes.get(id))
            .and_then(|n| n.get("type"))
            .and_then(Value::as_str)
    }

    /// The node, if it exists.
    pub fn node(&self, nid: &str) -> Option<&Rec> {
        self.nodes.get(nid)
    }

    /// The edge, if it exists.
    pub fn edge(&self, eid: &str) -> Option<&Rec> {
        self.edges.get(eid)
    }

    /// A joke and its units.
    pub fn joke_scope(&self, jid: &str) -> Vec<String> {
        let mut out = vec![jid.to_string()];
        if let Some(us) = self.joke_units.get(jid) {
            out.extend(us.iter().map(|(u, _)| u.clone()));
        }
        out
    }

    /// Every edge touching the joke or one of its units, each once, in adjacency order.
    pub fn joke_edges(&self, jid: &str) -> Vec<(&String, &Rec)> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for nid in self.joke_scope(jid) {
            for eid in self.adj.get(&nid).into_iter().flatten() {
                if seen.insert(eid.clone())
                    && let Some((k, e)) = self.edges.get_key_value(eid)
                {
                    out.push((k, e));
                }
            }
        }
        out
    }
}

fn ntype_is(n: &Rec, t: &str) -> bool {
    n.get("type").and_then(Value::as_str) == Some(t)
}

/// An adjacency key for an edge end: the id, or `"null"` for a missing one (Python's `None` dict key, as JSON).
fn end_key(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => "null".to_string(),
        Some(other) => other.to_string(),
    }
}
