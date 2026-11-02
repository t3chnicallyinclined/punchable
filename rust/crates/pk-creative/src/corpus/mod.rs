//! His notebook as the writing kinds read it: `creative/data.py` `Corpus` ported as-is. The v1 graph with his
//! decisions replayed on top (his own lines and imports of his material only, never `ai:*`, never `auto`; v0 ids
//! mapped through `id_map`), every taped set's segments merged into one performance per bit per night, the running
//! order and what he did back to back, the labels with their weights and idf, the bge vectors.
//!
//! Pure: the caller hands in the parsed files (`Corpus::from_files` takes the workspace as path → text, the way the
//! worker mirrors the `srcfile` rows); nothing here reads a disk or the environment. Numbers follow Python's
//! arithmetic (floats summed in the same order, `round` correctly rounded), so the goldens compare as values.

mod vectors;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

use indexmap::IndexMap;
use pk_core::py;
use serde_json::{Map, Value, json};

pub use vectors::{DIM, VectorPart, Vectors};

/// `RATING_ORDER`.
pub fn rating_order(r: &str) -> i64 {
    match r {
        "killed" => 3,
        "worked" => 2,
        "meh" => 1,
        _ => 0,
    }
}

/// `WPM`: his taped pace.
pub const WPM: f64 = 157.0;

/// What a corpus cannot be built without.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CorpusError {
    /// A file the corpus needs is not in the workspace (`kb/v1/graph.json`).
    #[error("[Errno 2] No such file or directory: '{0}'")]
    Missing(String),
    /// A file is not the JSON it should be.
    #[error("{path}: {msg}")]
    Json {
        /// The workspace path.
        path: String,
        /// The parser's words.
        msg: String,
    },
}

/// Python's `round(x, n)`: correctly rounded on the exact binary value, as `format!` is.
pub fn round_to(x: f64, n: usize) -> f64 {
    format!("{x:.n$}").parse().unwrap_or(x)
}

/// A JSON value read as a Python number (`int`, `float`; `bool` counts as 0/1), else `None`.
pub fn num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

/// `x or 0` for a number field.
fn num_or0(v: Option<&Value>) -> f64 {
    num(v).unwrap_or(0.0)
}

/// `d.get(k)` as a `&str` when it is a string.
pub fn s<'a>(m: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    m.get(k).and_then(Value::as_str)
}

/// `d.get(k) or default` on a string field (an empty string falls through, as Python's `or`).
pub fn s_or<'a>(m: &'a Map<String, Value>, k: &str, default: &'a str) -> &'a str {
    s(m, k).filter(|x| !x.is_empty()).unwrap_or(default)
}

/// `his_material(by)`: his own action or an import of his own material.
pub fn his_material(by: Option<&Value>, owner: &str) -> bool {
    match by {
        Some(Value::String(b)) => b == owner || b.starts_with("import:"),
        _ => false,
    }
}

/// `set_key_from_dir`: `20240719_FakeSet0006` → `2024-07-19_FakeSet0006`.
pub fn set_key_from_dir(d: &str) -> String {
    let c: Vec<char> = d.chars().collect();
    if c.len() > 9 && c[8] == '_' && c[..8].iter().all(char::is_ascii_digit) {
        let p = |a: usize, b: usize| c[a..b].iter().collect::<String>();
        return format!("{}-{}-{}_{}", p(0, 4), p(4, 6), p(6, 8), p(9, c.len()));
    }
    d.to_string()
}

/// `venue_short(venue, date)`: the room in prose ("Mabel's '23"), the older server rule kept byte for byte.
pub fn venue_short(venue: Option<&str>, date: Option<&str>) -> String {
    let v = py::strip(venue.unwrap_or(""));
    let low = v.to_lowercase();
    let mut name = if let Some(i) = low.find("maybe ") {
        // `low.index` is a code-point index into the lower-cased text, applied to the original
        let ci = low[..i].chars().count();
        let rest: String = v.chars().skip(ci + 6).collect();
        let head = rest.split(')').next().unwrap_or("");
        // `.strip(" '\"")`: only those three characters
        format!("{}?", head.trim_matches(|c| c == ' ' || c == '\'' || c == '"'))
    } else if v.is_empty() || low.starts_with("unknown") {
        "Unknown room".to_string()
    } else {
        let mut n = py::strip(v.split('(').next().unwrap_or("").split(',').next().unwrap_or("")).to_string();
        for tail in [" Comedy Club", " Poets Cafe", " Comedy"] {
            if n.ends_with(tail) && n.chars().count() > tail.chars().count() + 2 {
                n.truncate(n.len() - tail.len());
            }
        }
        if n.to_lowercase().starts_with("comix at ") {
            n = n.chars().skip(9).collect();
        }
        n
    };
    if let Some(d) = date.filter(|d| d.chars().count() >= 4) {
        let yy: String = d.chars().skip(2).take(2).collect();
        let _ = write!(name, " '{yy}");
    }
    name
}

/// One night of one bit: every segment of it in that set, merged (`Corpus._perf`).
#[derive(Debug, Clone, PartialEq)]
pub struct Perf {
    /// The bit.
    pub joke_id: String,
    /// The set key.
    pub set: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    /// The room, as the graph or the set's meta names it.
    pub venue: Value,
    /// Segment indexes.
    pub segments: Vec<Value>,
    /// `seg:<set>#<idx>` per segment.
    pub seg_ids: Vec<String>,
    /// The longest segment.
    pub main_seg: String,
    /// First segment's start.
    pub start_ms: Value,
    /// Last segment's end.
    pub end_ms: Value,
    /// Seconds on stage.
    pub duration_s: f64,
    /// Seconds of laughter.
    pub laugh_s: f64,
    /// Seconds the laughs were counted over.
    pub window_s: f64,
    /// Laugh seconds per minute.
    pub laugh_s_per_min: Option<f64>,
    /// Against the room's rate.
    pub vs_room: Option<f64>,
    /// The best rating of its segments.
    pub ai_rating: String,
    /// The biggest laugh meter.
    pub max_bars: f64,
    /// The first laugh.
    pub time_to_first_laugh_s: Value,
    /// The longest silence.
    pub dead_air_max_s: f64,
    /// Each punchline with its `seg_id`.
    pub punchlines: Vec<Map<String, Value>>,
    /// The main segment's script diff.
    pub script_diff: Value,
    /// The main segment's version unit.
    pub version_unit: Value,
    /// The set's laugh rate.
    pub set_laugh_s_per_min: Option<f64>,
    /// The set's room heat.
    pub room_heat: Value,
}

impl Perf {
    /// The dict the Python builds.
    pub fn to_value(&self) -> Value {
        json!({
            "joke_id": self.joke_id, "set": self.set, "date": self.date, "venue": self.venue,
            "segments": self.segments, "seg_ids": self.seg_ids, "main_seg": self.main_seg,
            "start_ms": self.start_ms, "end_ms": self.end_ms, "duration_s": self.duration_s,
            "laugh_s": self.laugh_s, "window_s": self.window_s, "laugh_s_per_min": self.laugh_s_per_min,
            "vs_room": self.vs_room, "ai_rating": self.ai_rating, "max_bars": self.max_bars,
            "time_to_first_laugh_s": self.time_to_first_laugh_s, "dead_air_max_s": self.dead_air_max_s,
            "punchlines": self.punchlines, "script_diff": self.script_diff, "version_unit": self.version_unit,
            "set_laugh_s_per_min": self.set_laugh_s_per_min, "room_heat": self.room_heat,
        })
    }
}

/// A set as `Corpus.sets` holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct SetInfo {
    /// The key.
    pub set: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    /// The room.
    pub venue: Value,
    /// Its title.
    pub title: Value,
    /// `analysis.json` `summary`.
    pub summary: Value,
    /// `set:<key>`.
    pub set_node: String,
}

/// Two bits done one right after the other on a night.
#[derive(Debug, Clone, PartialEq)]
pub struct BackToBack {
    /// The set.
    pub set: String,
    /// Its date.
    pub date: String,
    /// Its room.
    pub venue: Value,
    /// The first bit's segment.
    pub a_seg: String,
    /// The second bit's segment.
    pub b_seg: String,
}

/// The parsed workspace files a corpus is built from (`Corpus.load`).
#[derive(Debug, Clone, Default)]
pub struct Parts {
    /// `kb/v1/graph.json`.
    pub graph: Value,
    /// `kb/v1/id_map.json`, when there is one.
    pub id_map: Value,
    /// The decision lines, oldest first.
    pub decisions: Vec<Value>,
    /// `analysis.json` per set key, in set-folder order.
    pub analyses: IndexMap<String, Value>,
    /// `meta.json` per set key.
    pub metas: HashMap<String, Value>,
    /// `laughs_auto.json` events per set key: laugh id → event.
    pub laughs: HashMap<String, IndexMap<String, Value>>,
    /// `kb/v0/inferred.json`.
    pub inferred: Value,
    /// `kb/v0/insights.json` (gems, orphans, chunks), `Null` when there is none.
    pub insights: Value,
    /// The vectors (shared: the worker keeps one copy across jobs).
    pub vectors: Arc<Vectors>,
    /// `PK_OWNER_ID` ("owner" unless set).
    pub owner: String,
}

fn parse(path: &str, text: &str) -> Result<Value, CorpusError> {
    serde_json::from_str(text).map_err(|e| CorpusError::Json {
        path: path.to_string(),
        msg: e.to_string(),
    })
}

impl Parts {
    /// `Corpus.load` over the workspace: `files` maps a `srcfile` path (`kb/v1/graph.json`, `corpus/sets/<dir>/
    /// analysis.json`, …) to its text. `graph.json` is required, as in the Python.
    pub fn from_files<T: AsRef<str>>(
        files: &BTreeMap<String, T>,
        decisions: Vec<Value>,
        vectors: Arc<Vectors>,
        owner: &str,
    ) -> Result<Self, CorpusError> {
        let graph_path = "kb/v1/graph.json";
        let graph_text = files
            .get(graph_path)
            .ok_or_else(|| CorpusError::Missing(graph_path.into()))?;
        let graph = parse(graph_path, graph_text.as_ref())?;
        let opt =
            |p: &str| -> Result<Value, CorpusError> { files.get(p).map_or(Ok(Value::Null), |t| parse(p, t.as_ref())) };
        let id_map = opt("kb/v1/id_map.json")?;
        let inferred = opt("kb/v0/inferred.json")?;
        let insights = opt("kb/v0/insights.json")?;
        let (mut analyses, mut metas, mut laughs) = (IndexMap::new(), HashMap::new(), HashMap::new());
        // the set folders in name order (BTreeMap: `sorted(os.listdir(sets))`)
        let mut dirs: Vec<&str> = files
            .keys()
            .filter_map(|p| p.strip_prefix("corpus/sets/")?.strip_suffix("/analysis.json"))
            .filter(|d| !d.contains('/'))
            .collect();
        // `sorted(os.listdir(sets))`: by folder name ("Show" before "Show-2"), not by the whole path
        dirs.sort_unstable();
        for d in dirs {
            let a = parse(
                "analysis.json",
                files[&format!("corpus/sets/{d}/analysis.json")].as_ref(),
            )?;
            let key = a
                .get("set")
                .filter(|v| py::truthy(Some(v)))
                .map_or_else(|| set_key_from_dir(d), py::str);
            let m = opt(&format!("corpus/sets/{d}/meta.json"))?;
            if py::truthy(Some(&m)) {
                metas.insert(key.clone(), m);
            }
            let la = opt(&format!("corpus/sets/{d}/laughs_auto.json"))?;
            if let Some(list) = la.get("laughs").and_then(Value::as_array) {
                let events: IndexMap<String, Value> = list
                    .iter()
                    .filter_map(|x| {
                        let id = x.as_object()?.get("id").filter(|v| py::truthy(Some(v)))?;
                        Some((py::str(id), x.clone()))
                    })
                    .collect();
                laughs.insert(key.clone(), events);
            }
            analyses.insert(key, a);
        }
        Ok(Self {
            graph,
            id_map,
            decisions,
            analyses,
            metas,
            laughs,
            inferred,
            insights,
            vectors,
            owner: owner.to_string(),
        })
    }
}

/// The notebook. Built once per job; read-only after.
#[derive(Debug, Clone, Default)]
pub struct Corpus {
    /// The actor id of his own edits.
    pub owner: String,
    id_map: Value,
    /// Every node by id, in graph order (his added jokes after).
    pub nodes: IndexMap<String, Map<String, Value>>,
    /// Every edge, his replayed on top.
    pub edges: Vec<Map<String, Value>>,
    /// Nodes he edited or added.
    pub his_nodes: HashSet<String>,
    /// Jokes whose stage he set.
    pub his_stage: HashSet<String>,
    /// His edits to a performance (`perf:<set>@<start_ms>`).
    pub perf_overrides: IndexMap<String, Map<String, Value>>,
    /// His set lists.
    pub his_lists: IndexMap<String, Map<String, Value>>,
    /// The jokes (not hidden), in node order.
    pub jokes: Vec<String>,
    joke_set: HashSet<String>,
    /// The labels, in node order.
    pub labels: Vec<String>,
    /// Edge indexes by node.
    pub edges_by_node: HashMap<String, Vec<usize>>,
    /// Each unit's joke (its best `part_of`), units in id order.
    pub unit_joke: IndexMap<String, String>,
    /// Each joke's units.
    pub joke_units: HashMap<String, Vec<String>>,
    /// joke → label → (weight, edge index).
    pub joke_labels: HashMap<String, IndexMap<String, (f64, usize)>>,
    /// Jokes holding each label at weight ≥ 0.5.
    pub label_df: IndexMap<String, usize>,
    /// `log(n / (1 + df)) + 1`.
    pub idf: IndexMap<String, f64>,
    /// `inferred.json` jokes by mapped id.
    pub inferred_jokes: HashMap<String, Value>,
    /// The sets by key, in the order they were built.
    pub sets: IndexMap<String, SetInfo>,
    /// Each set's segments (with `joke`, `seg_id`, his overrides).
    pub segments: HashMap<String, Vec<Map<String, Value>>>,
    /// Each set's running order without banter: (joke or none, segment index).
    pub order: IndexMap<String, Vec<(Option<String>, usize)>>,
    /// Each joke's nights, in date order.
    pub perfs: IndexMap<String, Vec<Perf>>,
    /// (a, b) → the nights b came right after a.
    pub back_to_back: IndexMap<(String, String), Vec<BackToBack>>,
    /// Set key → laugh id → event.
    pub laughs: HashMap<String, IndexMap<String, Value>>,
    /// The vectors.
    pub vectors: Arc<Vectors>,
    /// `insights.json` (`self.insights = insights or {}`; `Null` when there is none).
    pub insights: Value,
}

impl Corpus {
    /// Build it (`Corpus.__init__`).
    pub fn new(p: Parts) -> Self {
        let mut c = Self {
            owner: p.owner,
            id_map: p.id_map,
            laughs: p.laughs,
            vectors: p.vectors,
            insights: p.insights,
            ..Self::default()
        };
        c.build_graph(p.graph, &p.decisions, &p.inferred);
        c.build_sets(&p.analyses, &p.metas);
        c
    }

    fn mapped(&self, table: &str, id: &str) -> Option<String> {
        // a null in the map is Python's None: the id stays as it is
        self.id_map.get(table)?.get(id).filter(|v| !v.is_null()).map(py::str)
    }

    /// `map_joke(jid)`.
    pub fn map_joke(&self, jid: &str) -> String {
        self.mapped("jokes", jid).unwrap_or_else(|| jid.to_string())
    }

    /// `map_unit(uid)`.
    pub fn map_unit(&self, uid: &str) -> String {
        self.mapped("units", uid).unwrap_or_else(|| uid.to_string())
    }

    /// `map_edge(eid)`.
    pub fn map_edge(&self, eid: &str) -> String {
        self.mapped("edges", eid).unwrap_or_else(|| eid.to_string())
    }

    /// Whether `jid` is a joke (not hidden).
    pub fn is_joke(&self, jid: &str) -> bool {
        self.joke_set.contains(jid)
    }

    /// The joke's node, when it is a joke.
    pub fn joke(&self, jid: &str) -> Option<&Map<String, Value>> {
        if self.is_joke(jid) { self.nodes.get(jid) } else { None }
    }

    #[allow(clippy::too_many_lines)]
    fn build_graph(&mut self, mut graph: Value, decisions: &[Value], inferred: &Value) {
        // the graph is ours: its nodes and edges move into the corpus, never copied
        let mut objs = |k: &str| -> Vec<Map<String, Value>> {
            match graph.get_mut(k).map(Value::take) {
                Some(Value::Array(a)) => a
                    .into_iter()
                    .filter_map(|x| match x {
                        Value::Object(m) => Some(m),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            }
        };
        for n in objs("nodes") {
            self.nodes.insert(n.get("id").map(py::str).unwrap_or_default(), n);
        }
        let mut edges: IndexMap<String, Map<String, Value>> = IndexMap::new();
        for e in objs("edges") {
            edges.insert(e.get("id").map(py::str).unwrap_or_default(), e);
        }
        for d in decisions {
            let Some(d) = d.as_object() else { continue };
            let owner = Value::String(self.owner.clone());
            if !his_material(Some(d.get("by").unwrap_or(&owner)), &self.owner) || py::truthy(d.get("auto")) {
                continue;
            }
            let (tgt, act) = (s(d, "target"), s(d, "action").unwrap_or(""));
            let after: Map<String, Value> = d.get("after").and_then(Value::as_object).cloned().unwrap_or_default();
            let without_id = |m: &Map<String, Value>| -> Vec<(String, Value)> {
                m.iter()
                    .filter(|(k, _)| *k != "id")
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            };
            if tgt == Some("edge") {
                let eid = self.map_edge(&d.get("id").map(py::str).unwrap_or_default());
                if act == "reject" || act == "delete" {
                    edges.shift_remove(&eid);
                } else if act == "add" && py::truthy(after.get("src")) && py::truthy(after.get("dst")) {
                    let mut e = after.clone();
                    e.entry("id").or_insert_with(|| Value::String(eid.clone()));
                    e.insert("_his".into(), Value::Bool(true));
                    edges.insert(e.get("id").map(py::str).unwrap_or_default(), e);
                } else if let Some(e) = edges.get_mut(&eid) {
                    for (k, v) in without_id(&after) {
                        e.insert(k, v);
                    }
                    e.insert("_his".into(), Value::Bool(true));
                }
            } else if tgt == Some("node") {
                let mut nid = s(d, "id").unwrap_or("").to_string();
                if nid.starts_with("joke:") {
                    nid = self.map_joke(&nid);
                }
                if nid.starts_with("perf:") {
                    let o = self.perf_overrides.entry(nid).or_default();
                    for (k, v) in after {
                        o.insert(k, v);
                    }
                } else if nid.starts_with("list:") {
                    if act == "delete" {
                        self.his_lists.shift_remove(&nid);
                    } else {
                        let o = self.his_lists.entry(nid).or_default();
                        for (k, v) in after {
                            o.insert(k, v);
                        }
                    }
                } else if let Some(n) = self.nodes.get_mut(&nid) {
                    if act == "delete" {
                        n.insert("hidden".into(), Value::Bool(true));
                    } else {
                        for (k, v) in without_id(&after) {
                            n.insert(k, v);
                        }
                        self.his_nodes.insert(nid.clone());
                        if after.contains_key("stage") {
                            self.his_stage.insert(nid);
                        }
                    }
                } else if act == "add" && s(&after, "type") == Some("joke") {
                    let mut n = after;
                    n.insert("id".into(), Value::String(nid.clone()));
                    self.nodes.insert(nid.clone(), n);
                    self.his_nodes.insert(nid);
                }
            }
        }
        self.edges = edges.into_values().collect();
        for (k, n) in &self.nodes {
            if s(n, "type") == Some("joke") && !py::truthy(n.get("hidden")) {
                self.jokes.push(k.clone());
            } else if s(n, "type") == Some("label") {
                self.labels.push(k.clone());
            }
        }
        self.joke_set = self.jokes.iter().cloned().collect();
        let label_set: HashSet<&String> = self.labels.iter().collect();
        let mut best_part: BTreeMap<String, (f64, String)> = BTreeMap::new();
        for (i, e) in self.edges.iter().enumerate() {
            let (src, dst) = (
                s(e, "src").unwrap_or("").to_string(),
                s(e, "dst").unwrap_or("").to_string(),
            );
            self.edges_by_node.entry(src.clone()).or_default().push(i);
            self.edges_by_node.entry(dst.clone()).or_default().push(i);
            match s(e, "kind") {
                Some("part_of") if src.starts_with("unit:") && self.joke_set.contains(&dst) => {
                    let c = num_or0(e.get("confidence"));
                    if c > best_part.get(&src).map_or(-1.0, |b| b.0) {
                        best_part.insert(src, (c, dst));
                    }
                }
                Some("has_label") if self.joke_set.contains(&src) && label_set.contains(&dst) => {
                    let w = if self.source_of(e) == "his" {
                        1.0
                    } else {
                        num_or0(e.get("confidence"))
                    };
                    let ls = self.joke_labels.entry(src).or_default();
                    if w > ls.get(&dst).map_or(-1.0, |x| x.0) {
                        ls.insert(dst, (w, i));
                    }
                }
                _ => {}
            }
        }
        for (u, (_, j)) in best_part {
            if self.nodes.get(&u).is_some_and(|n| !py::truthy(n.get("hidden"))) {
                self.joke_units.entry(j.clone()).or_default().push(u.clone());
                self.unit_joke.insert(u, j);
            }
        }
        // label document frequency: labels held at weight >= 0.5 (counted in joke order; only the counts matter)
        let mut df: IndexMap<String, usize> = IndexMap::new();
        for j in &self.jokes {
            for (l, (w, _)) in self.joke_labels.get(j).into_iter().flatten() {
                if *w >= 0.5 {
                    *df.entry(l.clone()).or_default() += 1;
                }
            }
        }
        let n = self.jokes.len().max(1) as f64;
        self.idf = df
            .iter()
            .map(|(l, c)| (l.clone(), (n / (1.0 + *c as f64)).ln() + 1.0))
            .collect();
        self.label_df = df;
        for j in inferred.get("jokes").and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = j.get("id").and_then(Value::as_str).filter(|x| !x.is_empty()) {
                self.inferred_jokes.insert(self.map_joke(id), j.clone());
            }
        }
    }

    /// `source_of(e)`: `his`, `inferred` or `proposed`.
    pub fn source_of(&self, e: &Map<String, Value>) -> &'static str {
        if py::truthy(e.get("_his")) || s(e, "by") == Some(self.owner.as_str()) {
            "his"
        } else if matches!(s(e, "status"), Some("inferred" | "confirmed" | "confirmed_auto")) {
            "inferred"
        } else {
            "proposed"
        }
    }

    /// `stage(jid)`: (stage, his | inferred | proposed).
    pub fn stage(&self, jid: &str) -> (Value, &'static str) {
        let n = self.joke(jid);
        let st = n.and_then(|n| n.get("stage")).cloned().unwrap_or(Value::Null);
        if self.his_stage.contains(jid) {
            return (st, "his");
        }
        if let Some(ij) = self.inferred_jokes.get(jid)
            && py::truthy(ij.get("stage"))
        {
            return (ij["stage"].clone(), "inferred");
        }
        (st, "proposed")
    }

    /// `title(nid)`: title, else name, else the id.
    pub fn title(&self, nid: &str) -> String {
        let n = self.nodes.get(nid);
        for k in ["title", "name"] {
            if let Some(v) = n.and_then(|n| n.get(k)).filter(|v| py::truthy(Some(v))) {
                return py::str(v);
            }
        }
        nid.to_string()
    }

    /// A label's field (`kind`, `name`).
    pub fn label_field(&self, l: &str, k: &str) -> Value {
        self.nodes.get(l).and_then(|n| n.get(k)).cloned().unwrap_or(Value::Null)
    }

    /// `labels_of(jid, kinds, min_w)`: label → (weight, edge index), in the joke's label order.
    pub fn labels_of(&self, jid: &str, kinds: Option<&[&str]>, min_w: f64) -> IndexMap<String, (f64, usize)> {
        self.joke_labels
            .get(jid)
            .map(|ls| {
                ls.iter()
                    .filter(|(l, (w, _))| {
                        *w >= min_w
                            && kinds.is_none_or(|ks| {
                                self.nodes
                                    .get(*l)
                                    .and_then(|n| s(n, "kind"))
                                    .is_some_and(|k| ks.contains(&k))
                            })
                    })
                    .map(|(l, v)| (l.clone(), *v))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `devices(jid)`: its strongest device / structure labels.
    pub fn devices(&self, jid: &str, n: usize, min_w: f64) -> Vec<String> {
        let mut ls: Vec<(String, f64)> = self
            .labels_of(jid, Some(&["device", "structure"]), min_w)
            .into_iter()
            .map(|(l, (w, _))| (l, w))
            .collect();
        ls.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        ls.into_iter().take(n).map(|(l, _)| l).collect()
    }

    fn text_len(&self, u: &str) -> usize {
        self.nodes
            .get(u)
            .and_then(|n| s(n, "text"))
            .map_or(0, |t| t.chars().count())
    }

    /// `lead_unit(jid)`: the joke's named lead unit, else its longest unit.
    pub fn lead_unit(&self, jid: &str) -> Option<String> {
        let n = self.joke(jid);
        if let Some(lu) = n.and_then(|n| n.get("lead_unit")).filter(|v| py::truthy(Some(v))) {
            let u = self.map_unit(&py::str(lu));
            if self.nodes.get(&u).is_some_and(|n| !py::truthy(n.get("hidden"))) {
                return Some(u);
            }
        }
        self.joke_units
            .get(jid)?
            .iter()
            .max_by(|a, b| self.text_len(a).cmp(&self.text_len(b)).then_with(|| a.cmp(b)))
            .cloned()
    }

    /// A node's `text` (empty when it has none).
    pub fn node_text(&self, nid: &str) -> &str {
        self.nodes.get(nid).and_then(|n| s(n, "text")).unwrap_or("")
    }

    /// `seg(seg_id)`: a set segment by `seg:<set>#<idx>`.
    pub fn seg(&self, seg_id: &str) -> Option<&Map<String, Value>> {
        let rest = seg_id.get(4..)?;
        let (key, idx) = rest.rsplit_once('#').unwrap_or(("", rest));
        self.segments
            .get(key)?
            .iter()
            .find(|x| x.get("idx").map(py::str).as_deref() == Some(idx))
    }

    /// `text_of(node, field)`: a segment's transcript, a node's text (a label's definition), or a named field.
    pub fn text_of(&self, node: &str, field: Option<&str>) -> Option<String> {
        if node.starts_with("seg:") {
            return self.seg(node).and_then(|x| s(x, "transcript")).map(str::to_string);
        }
        let n = self.nodes.get(node)?;
        if let Some(f) = field {
            let mut parts = f.split('.');
            let first = parts.next().unwrap_or("");
            let mut cur = n.get(first);
            for part in parts {
                cur = cur.and_then(Value::as_object).and_then(|o| o.get(part));
            }
            return cur.and_then(Value::as_str).map(str::to_string);
        }
        if let Some(t) = n.get("text").filter(|v| py::truthy(Some(v))) {
            return t.as_str().map(str::to_string);
        }
        if s(n, "type") == Some("label") {
            return s(n, "definition").map(str::to_string);
        }
        None
    }

    /// `ev(node, quote)`: an evidence item when the quote is verbatim in the node, else `None`.
    pub fn ev(&self, node: &str, quote: &str) -> Option<Value> {
        let t = self.text_of(node, None)?;
        (!quote.is_empty() && t.contains(quote)).then(|| json!({"node": node, "quote": quote}))
    }

    /// `edge_evidence(e, n)`: up to `n` of the edge's quotes that are verbatim in their (mapped) node.
    pub fn edge_evidence(&self, e: &Map<String, Value>, n: usize) -> Vec<Value> {
        let mut out = Vec::new();
        for x in e.get("evidence").and_then(Value::as_array).into_iter().flatten() {
            let node = self.map_unit(x.get("node").and_then(Value::as_str).unwrap_or(""));
            if let Some(ev) = self.ev(&node, x.get("quote").and_then(Value::as_str).unwrap_or("")) {
                out.push(ev);
            }
            if out.len() >= n {
                break;
            }
        }
        out
    }

    /// `seg_evidence(seg_id)`: the first ten words of a segment, as evidence.
    pub fn seg_evidence(&self, seg_id: &str) -> Option<Value> {
        let t = self.text_of(seg_id, None).unwrap_or_default();
        let q = prefix_words(&t, 10);
        if q.is_empty() { None } else { self.ev(seg_id, &q) }
    }

    /// `duration(jid)`: (seconds, basis).
    pub fn duration(&self, jid: &str) -> (f64, String) {
        let ds: Vec<f64> = self
            .perfs
            .get(jid)
            .map(|ps| ps.iter().map(|p| p.duration_s).collect())
            .unwrap_or_default();
        if !ds.is_empty() {
            let n = ds.len();
            return (
                round_to(median(ds), 1),
                format!("tape ({n} night{})", if n == 1 { "" } else { "s" }),
            );
        }
        let words = self.lead_unit(jid).map_or(0, |u| pk_creative_words(self.node_text(&u)));
        if words == 0 {
            return (30.0, "default 0:30 (no text)".into());
        }
        (
            round_to((words as f64 / WPM * 60.0).min(300.0), 1),
            format!("estimate from {words} written words at {} wpm", WPM as i64),
        )
    }

    /// `energy(jid)`: mean `vs_room` (clipped at 3) over his nights, and how many nights.
    pub fn energy(&self, jid: &str) -> (Option<f64>, usize) {
        let ps = self.perfs.get(jid).map_or(&[][..], Vec::as_slice);
        let vs: Vec<f64> = ps.iter().filter_map(|p| p.vs_room).map(|v| v.min(3.0)).collect();
        let m = if vs.is_empty() {
            None
        } else {
            Some(round_to(vs.iter().sum::<f64>() / vs.len() as f64, 2))
        };
        (m, ps.len())
    }

    /// `set_keys()`: by date, then key.
    pub fn set_keys(&self) -> Vec<String> {
        let mut ks: Vec<&SetInfo> = self.sets.values().collect();
        ks.sort_by(|a, b| (&a.date, &a.set).cmp(&(&b.date, &b.set)));
        ks.into_iter().map(|x| x.set.clone()).collect()
    }

    #[allow(clippy::too_many_lines)]
    fn build_sets(&mut self, analyses: &IndexMap<String, Value>, metas: &HashMap<String, Value>) {
        let mut keys: Vec<&String> = analyses.keys().collect();
        let sort_key = |k: &String| -> (String, String) {
            let set = analyses[k].get("set").map_or_else(|| k.clone(), py::str);
            (set.chars().take(10).collect(), k.clone())
        };
        keys.sort_by_key(|k| sort_key(k));
        for key in keys {
            let a = &analyses[key];
            let date: String = key.chars().take(10).collect();
            let gset = self.nodes.get(&format!("set:{key}")).cloned().unwrap_or_default();
            let meta = metas.get(key).and_then(Value::as_object).cloned().unwrap_or_default();
            let pick = |m: &Map<String, Value>, k: &str| m.get(k).filter(|v| py::truthy(Some(v))).cloned();
            let venue = pick(&gset, "venue")
                .or_else(|| pick(&meta, "venue_guess"))
                .unwrap_or_else(|| json!("Unknown"));
            let summary = a.get("summary").cloned().unwrap_or_else(|| json!({}));
            self.sets.insert(
                key.clone(),
                SetInfo {
                    set: key.clone(),
                    date: date.clone(),
                    venue: venue.clone(),
                    title: pick(&gset, "title")
                        .or_else(|| pick(&meta, "title"))
                        .unwrap_or(Value::Null),
                    summary: summary.clone(),
                    set_node: format!("set:{key}"),
                },
            );
            let mut segs: Vec<Map<String, Value>> = Vec::new();
            for seg in a.get("jokes").and_then(Value::as_array).into_iter().flatten() {
                let mut seg = seg.as_object().cloned().unwrap_or_default();
                let joke = seg
                    .get("joke_id")
                    .filter(|v| py::truthy(Some(v)))
                    .map_or(Value::Null, |j| Value::String(self.map_joke(&py::str(j))));
                seg.insert("joke".into(), joke);
                let idx = seg.get("idx").map_or_else(|| "None".to_string(), py::str);
                seg.insert("seg_id".into(), Value::String(format!("seg:{key}#{idx}")));
                let start = seg.get("start_ms").map_or_else(|| "None".to_string(), py::str);
                if let Some(ov) = self.perf_overrides.get(&format!("perf:{key}@{start}")) {
                    if let Some(k @ ("crowd_work" | "banter")) = s(ov, "kind") {
                        seg.insert("status".into(), json!(k));
                    }
                    if let Some(r) = ov.get("rating").filter(|v| py::truthy(Some(v))) {
                        seg.insert("ai_rating".into(), r.clone());
                        seg.insert("rating_by".into(), json!("his"));
                    }
                }
                segs.push(seg);
            }
            let known = |x: &Map<String, Value>| matches!(s(x, "status"), Some("known" | "new"));
            let mut by_joke: IndexMap<String, Vec<usize>> = IndexMap::new();
            for (i, seg) in segs.iter().enumerate() {
                if let Some(j) = s(seg, "joke").filter(|j| !j.is_empty())
                    && known(seg)
                {
                    by_joke.entry(j.to_string()).or_default().push(i);
                }
            }
            let set_rate = num(summary.get("laugh_s_per_min"));
            let room_heat = summary.get("room_heat").cloned().unwrap_or(Value::Null);
            for (jid, ix) in by_joke {
                let ss: Vec<&Map<String, Value>> = ix.iter().map(|i| &segs[*i]).collect();
                let p = perf(key, &date, &venue, set_rate, &room_heat, &jid, &ss);
                self.perfs.entry(jid).or_default().push(p);
            }
            let mut order: Vec<(Option<String>, usize)> = Vec::new();
            for (i, seg) in segs.iter().enumerate() {
                if s(seg, "status") == Some("banter") {
                    continue;
                }
                let j = if known(seg) {
                    s(seg, "joke").filter(|j| !j.is_empty()).map(str::to_string)
                } else {
                    None
                };
                if j.is_some() && order.last().is_some_and(|o| o.0 == j) {
                    continue;
                }
                order.push((j, i));
            }
            for w in order.windows(2) {
                if let ((Some(ja), sa), (Some(jb), sb)) = (&w[0], &w[1])
                    && ja != jb
                {
                    self.back_to_back
                        .entry((ja.clone(), jb.clone()))
                        .or_default()
                        .push(BackToBack {
                            set: key.clone(),
                            date: date.clone(),
                            venue: venue.clone(),
                            a_seg: s(&segs[*sa], "seg_id").unwrap_or("").to_string(),
                            b_seg: s(&segs[*sb], "seg_id").unwrap_or("").to_string(),
                        });
                }
            }
            self.order.insert(key.clone(), order);
            self.segments.insert(key.clone(), segs);
        }
        for ps in self.perfs.values_mut() {
            ps.sort_by(|a, b| (&a.date, &a.set).cmp(&(&b.date, &b.set)));
        }
    }
}

fn pk_creative_words(text: &str) -> usize {
    crate::ask::words::py_word_count(text)
}

/// `median(xs)` over a non-empty list.
fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(f64::total_cmp);
    let m = xs.len() / 2;
    if xs.len() % 2 == 1 {
        return xs[m];
    }
    // the Python's own formula, not `midpoint` (the goldens compare the bits)
    #[allow(clippy::manual_midpoint)]
    let mid = (xs[m - 1] + xs[m]) / 2.0;
    mid
}

fn laugh_field(x: &Map<String, Value>, k: &str) -> f64 {
    x.get("laughs")
        .and_then(|l| l.get(k))
        .map_or(0.0, |v| num(Some(v)).unwrap_or(0.0))
}

/// `Corpus._perf`.
fn perf(
    key: &str,
    date: &str,
    venue: &Value,
    set_rate: Option<f64>,
    room_heat: &Value,
    jid: &str,
    ss: &[&Map<String, Value>],
) -> Perf {
    let (laugh_s, window_s, lspm, vs_room) = if let [x] = ss {
        (
            laugh_field(x, "laugh_s"),
            laugh_field(x, "window_s"),
            num(x.get("laugh_s_per_min")),
            num(x.get("vs_room")),
        )
    } else {
        let laugh_s = ss.iter().fold(0.0, |a, x| a + laugh_field(x, "laugh_s"));
        let window_s = ss.iter().fold(0.0, |a, x| a + laugh_field(x, "window_s"));
        let lspm = if window_s == 0.0 {
            0.0
        } else {
            round_to(laugh_s / (window_s / 60.0), 2)
        };
        let vs = set_rate.filter(|r| *r != 0.0).map(|r| round_to(lspm / r, 2));
        (laugh_s, window_s, Some(lspm), vs)
    };
    let span = |x: &Map<String, Value>| num_or0(x.get("end_ms")) - num_or0(x.get("start_ms"));
    let mut main = ss[0];
    for x in &ss[1..] {
        // max by (span, -idx): a later segment wins only when strictly longer, or as long with a smaller idx
        let (a, b) = (
            (span(x), -num_or0(x.get("idx"))),
            (span(main), -num_or0(main.get("idx"))),
        );
        #[allow(clippy::float_cmp)] // Python's tuple comparison: exact
        let later_wins = a.0 > b.0 || (a.0 == b.0 && a.1 > b.1);
        if later_wins {
            main = x;
        }
    }
    let seg_id = |x: &Map<String, Value>| s(x, "seg_id").unwrap_or("").to_string();
    let rating = |x: &Map<String, Value>| {
        s(x, "ai_rating")
            .filter(|r| !r.is_empty())
            .unwrap_or("died")
            .to_string()
    };
    let mut best_rating = rating(ss[0]);
    for x in &ss[1..] {
        let r = rating(x);
        if rating_order(&r) > rating_order(&best_rating) {
            best_rating = r;
        }
    }
    let duration = ss.iter().fold(0.0, |a, x| a + span(x) / 1000.0);
    Perf {
        joke_id: jid.to_string(),
        set: key.to_string(),
        date: date.to_string(),
        venue: venue.clone(),
        segments: ss
            .iter()
            .map(|x| x.get("idx").cloned().unwrap_or(Value::Null))
            .collect(),
        seg_ids: ss.iter().map(|x| seg_id(x)).collect(),
        main_seg: seg_id(main),
        start_ms: ss[0].get("start_ms").cloned().unwrap_or(Value::Null),
        end_ms: ss[ss.len() - 1].get("end_ms").cloned().unwrap_or(Value::Null),
        duration_s: round_to(duration, 1),
        laugh_s: round_to(laugh_s, 2),
        window_s: round_to(window_s, 2),
        laugh_s_per_min: lspm,
        vs_room,
        ai_rating: best_rating,
        max_bars: ss
            .iter()
            .map(|x| {
                x.get("laughs")
                    .and_then(|l| l.get("max_bars"))
                    .map_or(0.0, |v| num(Some(v)).unwrap_or(0.0))
            })
            .fold(f64::NEG_INFINITY, f64::max),
        time_to_first_laugh_s: ss
            .iter()
            .find_map(|x| x.get("time_to_first_laugh_s").filter(|v| !v.is_null()).cloned())
            .unwrap_or(Value::Null),
        dead_air_max_s: ss
            .iter()
            .map(|x| num_or0(x.get("dead_air_max_s")))
            .fold(f64::NEG_INFINITY, f64::max),
        punchlines: ss
            .iter()
            .flat_map(|x| {
                x.get("punchlines")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(move |p| {
                        let mut p = p.as_object()?.clone();
                        p.insert("seg_id".into(), Value::String(seg_id(x)));
                        Some(p)
                    })
            })
            .collect(),
        script_diff: main.get("script_diff").cloned().unwrap_or(Value::Null),
        version_unit: main.get("version_unit_id").cloned().unwrap_or(Value::Null),
        set_laugh_s_per_min: set_rate,
        room_heat: room_heat.clone(),
    }
}

/// `prefix_words(text, n)`: the first `n` whitespace-separated words, verbatim.
pub fn prefix_words(text: &str, n: usize) -> String {
    let words: Vec<(usize, usize)> = word_spans(text);
    if words.is_empty() {
        return String::new();
    }
    // `ms[min(n, len) - 1]`: n = 0 is index -1, the last word, in Python
    let end = words[n.min(words.len()).checked_sub(1).unwrap_or(words.len() - 1)].1;
    text[words[0].0..end].to_string()
}

/// `suffix_words(text, n)`: the last `n` words, verbatim.
pub fn suffix_words(text: &str, n: usize) -> String {
    let words = word_spans(text);
    if words.is_empty() {
        return String::new();
    }
    let start = words[words.len().saturating_sub(n)].0;
    text[start..words[words.len() - 1].1].to_string()
}

/// Byte spans of `\S+` runs (Python's `\s` is Unicode whitespace, the same set `str.isspace` names).
pub(crate) fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if py::isspace(c) {
            if let Some(st) = start.take() {
                out.push((st, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        out.push((st, text.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_cmp)]
    fn round_is_python_round() {
        assert_eq!(round_to(2.675, 2), 2.67);
        assert_eq!(round_to(0.125, 2), 0.12);
        assert_eq!(round_to(1.45, 1), 1.4);
    }

    #[test]
    fn words_and_keys() {
        assert_eq!(prefix_words("  a b  c d ", 2), "a b");
        assert_eq!(suffix_words("a b c", 2), "b c");
        assert_eq!(set_key_from_dir("20240719_Fake"), "2024-07-19_Fake");
        assert_eq!(set_key_from_dir("2024071_Fake"), "2024071_Fake");
    }
}
