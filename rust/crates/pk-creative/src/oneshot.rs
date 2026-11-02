//! The one-shot Ask (`engine/ask.py`, the `ask` worker kind), the halves with no IO: plan, retrieve, answer
//! prompt, verify.
//!
//! 1. **plan**: the model turns the question into searches, label ids, a year range, performed / never, named
//!    jokes and by-year ([`plan_messages`], [`PLAN_SCHEMA`]); [`Plan::settle`] then keeps only known labels, adds a
//!    label the question names, takes the years the question names and drops a year it never named.
//! 2. **retrieve** ([`retrieve`]): full text (`SQLite` FTS5 + bm25, run by the caller through `pk-fts` on
//!    [`fts_expr`]) and bge vectors fused by reciprocal rank (`1 / (60 + r)`), plus the graph routes (label members,
//!    a named joke's units and performances), filtered by year and performed, at most [`PER_JOKE`] per joke and
//!    [`MAX_SOURCES`] in all.
//! 3. **answer**: the sources numbered by id ([`answer_messages`], [`ANSWER_SCHEMA`]).
//! 4. **verify** ([`verify`]): every citation's quote through the one verbatim gate or dropped and flagged, ids
//!    normalised, "quoted" prose in no source cut, bracketed ids that were not retrieved marked.
//!
//! The corpus ([`AskCorpus`]) is the Python's own, over the `kbstore` view of `kb/v0/graph.json` with the decision
//! log replayed ([`Kb`]): written units and voice notes, performances (their set's date and venue, the joke they
//! were performed as), jokes as unquotable `title. summary` cards, and set transcripts in windows of
//! [`TX_WINDOW`] segments not more than half covered by a performance. Insertion orders are the Python's dict
//! orders, sorts are stable as Python's are, and the vector order breaks a tie by document order.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;

use indexmap::IndexMap;
use pk_core::py;
use pk_kb::{Decision, View};
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::corpus::CorpusError;
use crate::verbatim::{normalize_ids, strip_unverified_inline_quotes, verbatim};

/// Sources one answer sees (48 when a joke is named).
pub const MAX_SOURCES: usize = 32;
/// Sources per joke unless it is the named one (then 40).
pub const PER_JOKE: usize = 3;
/// A source's text in the prompt is cut to this many characters.
pub const SNIPPET: usize = 900;
/// Transcript segments per window.
pub const TX_WINDOW: usize = 8;
/// Hits per search leg.
pub const K: usize = 50;
/// The characters of a document's text that are embedded.
pub const EMBED_CHARS: usize = 2000;
/// Statuses that take an edge out (`kbstore.REJECTED`).
pub const REJECTED: [&str; 3] = ["rejected", "ruled_out", "superseded"];
/// The answer when nothing survives the filters.
pub const NOTHING: &str = "Nothing in your material matches this.";

/// `STOP`: words never searched.
pub static STOP: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    "the and for with that this what have has had about all any are was were you your my mine our from into \
     jokes joke bits bit wrote written write ever every which where when who how did does not never"
        .split(' ')
        .collect()
});

fn obj(props: &Value) -> Value {
    let required: Vec<&String> = props.as_object().map(|m| m.keys().collect()).unwrap_or_default();
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

/// `S_PLAN`.
pub static PLAN_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    obj(&json!({
        "search_queries": {"type": "array", "items": {"type": "string"}},
        "label_ids": {"type": "array", "items": {"type": "string"}},
        "year_min": {"type": "integer"}, "year_max": {"type": "integer"},
        "performed": {"type": "string", "enum": ["any", "performed", "never"]},
        "joke_titles": {"type": "array", "items": {"type": "string"}},
        "by_year": {"type": "boolean"}}))
});

/// `S_ANSWER`.
pub static ANSWER_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    obj(&json!({
        "found": {"type": "boolean"}, "answer": {"type": "string"},
        "citations": {"type": "array", "items": obj(&json!({"source_id": {"type": "string"},
                                                            "quote": {"type": "string"},
                                                            "note": {"type": "string"}}))},
        "not_found": {"type": "string"}}))
});

fn rx(p: &str) -> Regex {
    Regex::new(p).expect("regex")
}

// Python's `\s` on a str pattern is `str.isspace`, which also takes U+001C..U+001F; Rust's is Unicode White_Space.
static TERMS: LazyLock<Regex> = LazyLock::new(|| rx(r"[A-Za-z0-9']+"));
static WORDS: LazyLock<Regex> = LazyLock::new(|| rx(r"[a-z]+"));
static YEARS: LazyLock<Regex> = LazyLock::new(|| rx(r"\b(19[89][0-9]|20[0-4][0-9])\b"));
static A_YEAR: LazyLock<Regex> = LazyLock::new(|| rx(r"\b(19|20)[0-9][0-9]\b"));
static SET_DIR: LazyLock<Regex> = LazyLock::new(|| rx(r"^([0-9]{4})-?([0-9]{2})-?([0-9]{2})_(.+)$"));
static PROSE_IDS: LazyLock<Regex> = LazyLock::new(|| rx(r#"(?:unit|perf|tx|joke):[^\]\s\x1C-\x1F,;)'"]+"#));
static ANSWER_IDS: LazyLock<Regex> = LazyLock::new(|| rx(r"(?:unit|perf|tx|joke):[^\]\s\x1C-\x1F,;)]+"));
static BRACKET_IDS: LazyLock<Regex> = LazyLock::new(|| rx(r"\[((?:unit|perf|tx|joke):[^\]\s\x1C-\x1F,;]+)"));

fn rejected_status(e: &Map<String, Value>) -> bool {
    e.get("status")
        .and_then(Value::as_str)
        .is_some_and(|s| REJECTED.contains(&s))
}

fn key3(e: &Map<String, Value>) -> String {
    json!([e.get("src"), e.get("kind"), e.get("dst")]).to_string()
}

/// `engine/kbstore.KB` as the Ask reads it: the replayed view plus what the decisions say.
#[derive(Debug, Clone)]
pub struct Kb {
    /// The view (`tools/explorer/kb.py` `View`).
    pub view: View,
    rejected_nodes: HashSet<String>,
    rejected_edges: HashSet<String>,
    rejected_edge_keys: HashSet<String>,
    /// `performed_jokes`: the jokes a live `performed_as` edge points at.
    pub performed: HashSet<String>,
}

impl Kb {
    /// `KB.reload` + `_derive` over `kb/v0/graph.json` and the decision lines (every object line, undone or not, as
    /// `read_decisions` keeps them).
    pub fn new(graph: &Value, decisions: &[Value], owner: &str) -> Self {
        let ds: Vec<Decision> = decisions
            .iter()
            .filter_map(|d| d.as_object().map(|m| Decision(m.clone())))
            .collect();
        let view = View::new(graph, ds, Vec::new(), owner);
        let mut rejected_edges: HashSet<String> = HashSet::new();
        let mut rejected_edge_keys = HashSet::new();
        for (eid, e) in &view.edges {
            if rejected_status(e) {
                rejected_edges.insert(eid.clone());
                rejected_edge_keys.insert(key3(e));
            }
        }
        let mut rejected_nodes = HashSet::new();
        for d in decisions.iter().filter_map(Value::as_object) {
            let id = d.get("id").and_then(Value::as_str).map(str::to_string);
            let (target, action) = (
                d.get("target").and_then(Value::as_str),
                d.get("action").and_then(Value::as_str),
            );
            if action == Some("delete") {
                let into = if target == Some("edge") {
                    &mut rejected_edges
                } else {
                    &mut rejected_nodes
                };
                into.extend(id.clone());
            }
            if target == Some("node") && action == Some("reject") {
                rejected_nodes.extend(id);
            }
        }
        let performed = view
            .edges
            .values()
            .filter(|e| e.get("kind").and_then(Value::as_str) == Some("performed_as") && !rejected_status(e))
            .filter_map(|e| e.get("dst").and_then(Value::as_str).map(str::to_string))
            .collect();
        Self {
            view,
            rejected_nodes,
            rejected_edges,
            rejected_edge_keys,
            performed,
        }
    }

    /// Whether a node was deleted or rejected by a decision.
    pub fn is_rejected_node(&self, nid: &str) -> bool {
        self.rejected_nodes.contains(nid)
    }

    /// `is_rejected_edge(e)`.
    pub fn is_rejected_edge(&self, eid: &str, e: &Map<String, Value>) -> bool {
        let end = |k: &str| {
            e.get(k)
                .and_then(Value::as_str)
                .is_some_and(|n| self.rejected_nodes.contains(n))
        };
        self.rejected_edges.contains(eid) || self.rejected_edge_keys.contains(&key3(e)) || end("src") || end("dst")
    }

    /// `title(nid)`: the title, else the name, else the first 80 characters of the text, else the id (Python's
    /// `or` chain: any falsy value falls through).
    pub fn title(&self, nid: &str) -> Value {
        let n = self.view.nodes.get(nid);
        let get = |k: &str| n.and_then(|n| n.get(k)).filter(|v| py::truthy(Some(v))).cloned();
        if let Some(v) = get("title").or_else(|| get("name")) {
            return v;
        }
        let text: String = get("text")
            .and_then(|v| v.as_str().map(|s| s.chars().take(80).collect()))
            .unwrap_or_default();
        if text.is_empty() { json!(nid) } else { json!(text) }
    }

    /// `{n["id"]: n.get("name", "")}` over the live labels, in node order (`None` when a label has no name key).
    pub fn labels(&self) -> Vec<(String, Option<Value>)> {
        self.view
            .labels
            .iter()
            .map(|id| (id.clone(), self.view.nodes.get(id).and_then(|n| n.get("name")).cloned()))
            .collect()
    }
}

/// What a source is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    /// A written unit.
    Written,
    /// A voice note (a unit whose source is a memo).
    Memo,
    /// A performance on a tape.
    Performed,
    /// A window of a set transcript no performance covers.
    Transcript,
    /// A joke card (title + summary written by the system): never quotable as his words.
    Joke,
}

impl DocKind {
    /// The `kind` the result carries.
    pub fn name(self) -> &'static str {
        match self {
            DocKind::Written => "written",
            DocKind::Memo => "memo",
            DocKind::Performed => "performed",
            DocKind::Transcript => "transcript",
            DocKind::Joke => "joke",
        }
    }

    fn head(self) -> &'static str {
        match self {
            DocKind::Written => "written doc",
            DocKind::Memo => "voice note",
            DocKind::Performed => "performed live",
            DocKind::Transcript => "set transcript",
            DocKind::Joke => "joke card (title + summary written by the system)",
        }
    }
}

/// One source.
#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    /// `unit:…`, `perf:…`, `tx:<set dir>@<ms>`, `joke:…`.
    pub id: String,
    /// What it is.
    pub kind: DocKind,
    /// The words.
    pub text: String,
    /// `YYYY-MM-DD` mostly; kept as the graph has it (a set with a null date stays null).
    pub date: Value,
    /// The doc title, the room, or the joke title.
    pub title: Value,
    /// The joke it belongs to.
    pub joke: Option<String>,
}

impl Doc {
    fn date_str(&self) -> &str {
        self.date.as_str().unwrap_or("")
    }

    fn year(&self) -> Option<i64> {
        let y: String = self.date_str().chars().take(4).collect();
        (!y.is_empty() && y.chars().all(|c| c.is_ascii_digit()))
            .then(|| y.parse().ok())
            .flatten()
    }

    /// The text the vector is made from (`d["text"][:2000]`).
    pub fn embed_text(&self) -> String {
        self.text.chars().take(EMBED_CHARS).collect()
    }
}

fn value_str(v: Option<&Value>) -> String {
    match v {
        None => String::new(),
        Some(v) => py::str(v),
    }
}

/// `(x or default)` for a JSON value: a falsy value falls through to `default`.
fn or_else(v: Option<&Value>, default: Value) -> Value {
    v.filter(|v| py::truthy(Some(v))).cloned().unwrap_or(default)
}

fn number(v: Option<&Value>) -> f64 {
    crate::corpus::num(v).unwrap_or(0.0)
}

/// The Ask's corpus (`engine/ask.py` `Corpus`, without its indexes: the caller builds the FTS table and the vectors
/// over [`AskCorpus::docs`] in this order).
#[derive(Debug, Clone, Default)]
pub struct AskCorpus {
    /// Every source by id, in the Python's dict order.
    pub docs: IndexMap<String, Doc>,
}

impl AskCorpus {
    /// The sources over the view and the mirrored files (`corpus/sets/<dir>/transcript.json` and `meta.json`).
    pub fn build<T: AsRef<str>>(kb: &Kb, files: &BTreeMap<String, T>) -> Result<Self, CorpusError> {
        let v = &kb.view;
        let mut docs: IndexMap<String, Doc> = IndexMap::new();
        let sets: HashMap<&str, &Map<String, Value>> = v
            .nodes
            .iter()
            .filter(|(_, n)| n.get("type").and_then(Value::as_str) == Some("set"))
            .map(|(id, n)| (id.as_str(), n))
            .collect();
        let mut joke_of_perf: HashMap<&str, &str> = HashMap::new();
        for e in v.edges.values() {
            if e.get("kind").and_then(Value::as_str) == Some("performed_as")
                && !rejected_status(e)
                && let (Some(src), Some(dst)) = (
                    e.get("src").and_then(Value::as_str),
                    e.get("dst").and_then(Value::as_str),
                )
            {
                joke_of_perf.entry(src).or_insert(dst);
            }
        }
        let mut perf_spans: HashMap<String, Vec<(f64, f64)>> = HashMap::new();
        for (nid, n) in &v.nodes {
            let text = n.get("text").filter(|t| py::truthy(Some(t))).map(py::str);
            match n.get("type").and_then(Value::as_str) {
                Some("unit") if text.is_some() => {
                    let kind = if n.get("source").and_then(Value::as_str) == Some("memo") {
                        DocKind::Memo
                    } else {
                        DocKind::Written
                    };
                    let date: String = n
                        .get("date")
                        .filter(|d| py::truthy(Some(d)))
                        .map(py::str)
                        .unwrap_or_default()
                        .chars()
                        .take(10)
                        .collect();
                    docs.insert(
                        nid.clone(),
                        Doc {
                            id: nid.clone(),
                            kind,
                            text: text.unwrap_or_default(),
                            date: json!(date),
                            title: or_else(n.get("doc_title"), json!("")),
                            joke: v.unit_joke.get(nid).filter(|j| !j.is_empty()).cloned(),
                        },
                    );
                }
                Some("performance") if text.is_some() => {
                    let set_key = n.get("set").and_then(Value::as_str);
                    let s = set_key.and_then(|k| sets.get(k));
                    let date = s.map_or(json!(""), |s| s.get("date").cloned().unwrap_or(json!("")));
                    let title = s.map_or(json!(""), |s| {
                        or_else(s.get("venue"), s.get("title").cloned().unwrap_or(json!("")))
                    });
                    docs.insert(
                        nid.clone(),
                        Doc {
                            id: nid.clone(),
                            kind: DocKind::Performed,
                            text: text.unwrap_or_default(),
                            date,
                            title,
                            joke: joke_of_perf.get(nid.as_str()).map(|j| (*j).to_string()),
                        },
                    );
                    perf_spans
                        .entry(n.get("set").map(py::str).unwrap_or_default())
                        .or_default()
                        .push((number(n.get("start_ms")), number(n.get("end_ms"))));
                }
                Some("joke") if !kb.is_rejected_node(nid) => {
                    let get = |k: &str| n.get(k).map_or(String::new(), py::str);
                    docs.insert(
                        nid.clone(),
                        Doc {
                            id: nid.clone(),
                            kind: DocKind::Joke,
                            text: format!("{}. {}", get("title"), get("summary")),
                            date: n.get("first_seen").cloned().unwrap_or(json!("")),
                            title: n.get("title").cloned().unwrap_or(json!("")),
                            joke: Some(nid.clone()),
                        },
                    );
                }
                _ => {}
            }
        }
        // set transcripts: the folders in name order, windows not already covered by a performance
        let mut dirs: Vec<&str> = files
            .keys()
            .filter_map(|p| p.strip_prefix("corpus/sets/")?.strip_suffix("/transcript.json"))
            .filter(|d| !d.contains('/'))
            .collect();
        dirs.sort_unstable();
        for d in dirs {
            let (date, set_id) = match SET_DIR.captures(d) {
                Some(m) => {
                    let date = format!("{}-{}-{}", &m[1], &m[2], &m[3]);
                    let set_id = format!("set:{date}_{}", &m[4]);
                    (date, set_id)
                }
                None => (String::new(), format!("set:{d}")),
            };
            let mut venue = or_else(sets.get(set_id.as_str()).and_then(|s| s.get("venue")), json!(""));
            if !py::truthy(Some(&venue))
                && let Some(meta) = files.get(&format!("corpus/sets/{d}/meta.json"))
                && let Ok(Value::Object(m)) = serde_json::from_str::<Value>(meta.as_ref())
            {
                venue = m.get("venue_guess").cloned().unwrap_or(json!(""));
            }
            let path = format!("corpus/sets/{d}/transcript.json");
            let tp: Value = serde_json::from_str(files[&path].as_ref()).map_err(|e| CorpusError::Json {
                path: path.clone(),
                msg: e.to_string(),
            })?;
            let segments: Vec<Value> = tp
                .get("segments")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let spans = perf_spans.get(&set_id).map(Vec::as_slice).unwrap_or_default();
            for w in segments.chunks(TX_WINDOW) {
                // `int(x * 1000)`: toward zero
                let a = (number(w[0].get("start")) * 1000.0).trunc();
                let b = (number(w[w.len() - 1].get("end")) * 1000.0).trunc();
                let covered: f64 = spans.iter().map(|&(s, e)| (b.min(e) - a.max(s)).max(0.0)).sum();
                if covered > 0.5 * (b - a) {
                    continue;
                }
                let tid = format!("tx:{d}@{}", a as i64);
                let text: Vec<&str> = w
                    .iter()
                    .map(|s| s.get("text").and_then(Value::as_str).map_or("", py::strip))
                    .collect();
                docs.insert(
                    tid.clone(),
                    Doc {
                        id: tid,
                        kind: DocKind::Transcript,
                        text: text.join(" "),
                        date: json!(date),
                        title: or_else(Some(&venue), json!(d)),
                        joke: None,
                    },
                );
            }
        }
        Ok(Self { docs })
    }

    /// The `(id, title, text)` rows of the FTS table, in document order (a non-string title is its `str`).
    pub fn fts_rows(&self) -> Vec<(String, String, String)> {
        self.docs
            .values()
            .map(|d| {
                let title = match &d.title {
                    Value::Null => String::new(),
                    t => py::str(t),
                };
                (d.id.clone(), title, d.text.clone())
            })
            .collect()
    }
}

/// `Corpus.fts`'s query: the question's words of 3+ characters, not stop words, each quoted, joined by `OR`; `None`
/// when no word is left (no search).
pub fn fts_expr(q: &str) -> Option<String> {
    let lower = q.to_lowercase();
    let terms: Vec<String> = TERMS
        .find_iter(&lower)
        .map(|m| m.as_str())
        .filter(|t| t.len() > 2 && !STOP.contains(t))
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

/// The plan (`plan` in the result).
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// The searches.
    pub search_queries: Vec<String>,
    /// Label ids.
    pub label_ids: Vec<String>,
    /// 0 when not asked.
    pub year_min: i64,
    /// 0 when not asked.
    pub year_max: i64,
    /// `any`, `performed`, `never`.
    pub performed: String,
    /// Named bits.
    pub joke_titles: Vec<String>,
    /// Ordered by year.
    pub by_year: bool,
    /// The planner's failure, when it failed (retrieval still runs on the question).
    pub plan_error: Option<String>,
}

impl Plan {
    /// The plan before the model: the question as the one search.
    pub fn new(question: &str) -> Self {
        Self {
            search_queries: vec![question.to_string()],
            label_ids: Vec::new(),
            year_min: 0,
            year_max: 0,
            performed: "any".into(),
            joke_titles: Vec::new(),
            by_year: false,
            plan_error: None,
        }
    }

    /// `plan.update(c.data)`: the schema-checked answer replaces each field it carries.
    pub fn update(&mut self, data: &Value) {
        let strs = |k: &str| -> Option<Vec<String>> {
            data.get(k)?.as_array().map(|xs| {
                xs.iter()
                    .map(|x| x.as_str().map_or_else(|| py::str(x), str::to_string))
                    .collect()
            })
        };
        let int = |k: &str| {
            data.get(k)
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        };
        if let Some(x) = strs("search_queries") {
            self.search_queries = x;
        }
        if let Some(x) = strs("label_ids") {
            self.label_ids = x;
        }
        if let Some(x) = int("year_min") {
            self.year_min = x;
        }
        if let Some(x) = int("year_max") {
            self.year_max = x;
        }
        if let Some(x) = data.get("performed").and_then(Value::as_str) {
            self.performed = x.to_string();
        }
        if let Some(x) = strs("joke_titles") {
            self.joke_titles = x;
        }
        if let Some(x) = data.get("by_year").and_then(Value::as_bool) {
            self.by_year = x;
        }
    }

    /// `plan["plan_error"] = str(e)[:200]`.
    pub fn failed(&mut self, why: &str) {
        self.plan_error = Some(why.chars().take(200).collect());
    }

    /// After the model: unknown labels dropped, a label the question names added, the question's years taken when
    /// the plan has none, a year the question never named dropped, the question itself searched last.
    pub fn settle(&mut self, question: &str, labels: &[(String, Option<Value>)]) {
        let known: HashSet<&str> = labels.iter().map(|(id, _)| id.as_str()).collect();
        self.label_ids.retain(|x| known.contains(x.as_str()));
        let ql = question.to_lowercase();
        for (lid, name) in labels {
            let name = name
                .as_ref()
                .filter(|v| py::truthy(Some(v)))
                .map(py::str)
                .unwrap_or_default()
                .to_lowercase();
            let n = py::strip(name.split('(').next().unwrap_or("").split('/').next().unwrap_or(""));
            if n.chars().count() > 3
                && Regex::new(&format!(r"\b{}\b", regex::escape(n))).is_ok_and(|r| r.is_match(&ql))
                && !self.label_ids.contains(lid)
            {
                self.label_ids.push(lid.clone());
            }
        }
        let years: Vec<i64> = YEARS
            .captures_iter(question)
            .filter_map(|c| c[1].parse().ok())
            .collect();
        if !years.is_empty() && self.year_min == 0 && self.year_max == 0 {
            self.year_min = years.iter().copied().min().unwrap_or(0);
            self.year_max = years.iter().copied().max().unwrap_or(0);
        }
        if self.year_min != 0 && !A_YEAR.is_match(question) {
            self.year_min = 0;
            self.year_max = 0;
        }
        if !self.search_queries.iter().any(|q| q == question) {
            self.search_queries.truncate(4);
            self.search_queries.push(question.to_string());
        }
    }

    /// The plan as the result carries it.
    pub fn to_value(&self) -> Value {
        let mut m = Map::new();
        m.insert("search_queries".into(), json!(self.search_queries));
        m.insert("label_ids".into(), json!(self.label_ids));
        m.insert("year_min".into(), json!(self.year_min));
        m.insert("year_max".into(), json!(self.year_max));
        m.insert("performed".into(), json!(self.performed));
        m.insert("joke_titles".into(), json!(self.joke_titles));
        m.insert("by_year".into(), json!(self.by_year));
        if let Some(e) = &self.plan_error {
            m.insert("plan_error".into(), json!(e));
        }
        Value::Object(m)
    }
}

/// The planner's messages: `(role, content)`.
pub fn plan_messages(question: &str, labels: &[(String, Option<Value>)]) -> [(&'static str, String); 2] {
    let list: Vec<String> = labels
        .iter()
        .map(|(k, v)| format!("{k} ({})", value_str(v.as_ref())))
        .collect();
    [
        (
            "system",
            "You turn a comic's question about his own joke archive into a search plan.".to_string(),
        ),
        (
            "user",
            format!(
                "QUESTION: {question}\n\nsearch_queries: 1-4 short searches with the words his jokes would actually \
                 contain (include synonyms, spelled the way he would say it). label_ids: only if the question asks for \
                 a device, structure, form or topic from this list, else []. year_min/year_max: 0 when not asked. \
                 performed: 'never' if he asks for material never performed, 'performed' if only performed, else \
                 'any'. joke_titles: names of specific bits he mentions (e.g. 'airport bit'), else []. by_year: true \
                 if he wants it ordered or grouped by year.\n\nLABEL IDS: {}",
                list.join(", ")
            ),
        ),
    ]
}

/// The vectors the ranking reads: one row per document (in [`AskCorpus::docs`] order), plus the query vectors by
/// query text (every search query, the question, every named joke title), all float32 and unit length.
#[derive(Debug, Clone, Copy)]
pub struct Vecs<'a> {
    /// `n x 384`, row-major.
    pub docs: &'a [f32],
    /// Query text -> vector.
    pub queries: &'a HashMap<String, Vec<f32>>,
}

impl Vecs<'_> {
    /// `M @ q`, float32 like numpy's.
    fn sims(&self, q: &str, n: usize) -> Vec<f32> {
        let Some(qv) = self.queries.get(q) else {
            return vec![0.0; n];
        };
        let dim = qv.len().max(1);
        self.docs
            .chunks(dim)
            .take(n)
            .map(|row| row.iter().zip(qv).map(|(a, b)| a * b).sum())
            .collect()
    }
}

/// `Corpus.vec`: the best `k` documents by cosine, best first; a tie keeps document order.
#[allow(clippy::many_single_char_names)] // the Python's names, so the port reads line for line
pub fn vec_hits(c: &AskCorpus, v: &Vecs<'_>, q: &str, k: usize) -> Vec<String> {
    let s = v.sims(q, c.docs.len());
    let mut idx: Vec<usize> = (0..s.len()).collect();
    idx.sort_by(|&a, &b| s[b].total_cmp(&s[a]));
    idx.into_iter()
        .take(k)
        .map(|i| c.docs.get_index(i).map(|(id, _)| id.clone()).unwrap_or_default())
        .collect()
}

/// Step 2: the sources, in the order the answer sees them. `fts` runs one FTS5 query (an error = no hits).
#[allow(clippy::many_single_char_names)] // the Python's names, so the port reads line for line
pub fn retrieve(
    c: &AskCorpus,
    kb: &Kb,
    plan: &Plan,
    question: &str,
    fts: &dyn Fn(&str) -> Vec<String>,
    v: &Vecs<'_>,
) -> Vec<String> {
    let n = c.docs.len();
    let mut score: IndexMap<String, f64> = IndexMap::new();
    let mut add = |i: &str, s: f64| *score.entry(i.to_string()).or_insert(0.0) += s;
    for q in &plan.search_queries {
        if let Some(expr) = fts_expr(q) {
            for (r, i) in fts(&expr).iter().enumerate() {
                add(i, 1.0 / (60.0 + r as f64));
            }
        }
        for (r, i) in vec_hits(c, v, q, K).iter().enumerate() {
            add(i, 1.0 / (60.0 + r as f64));
        }
    }
    let qsim = v.sims(question, n);
    let doc_sim = |i: usize| f64::from(qsim.get(i).copied().unwrap_or(0.0));
    let mut focus: HashSet<String> = HashSet::new();
    for lid in &plan.label_ids {
        let members: HashSet<&str> = kb
            .view
            .edges
            .iter()
            .filter(|(eid, e)| {
                e.get("kind").and_then(Value::as_str) == Some("has_label")
                    && e.get("dst").and_then(Value::as_str) == Some(lid.as_str())
                    && !kb.is_rejected_edge(eid, e)
            })
            .filter_map(|(_, e)| e.get("src").and_then(Value::as_str))
            .collect();
        let mut docs: Vec<usize> = (0..n)
            .filter(|&i| {
                let d = &c.docs[i];
                members.contains(d.id.as_str()) || d.joke.as_deref().is_some_and(|j| members.contains(j))
            })
            .collect();
        docs.sort_by(|&a, &b| (-doc_sim(a)).total_cmp(&-doc_sim(b)));
        for i in docs {
            add(&c.docs[i].id, 0.5 + 0.1 * doc_sim(i));
        }
    }
    for title in &plan.joke_titles {
        let js = v.sims(title, n);
        let lower = title.to_lowercase();
        let tw: HashSet<&str> = WORDS
            .find_iter(&lower)
            .map(|m| m.as_str())
            .filter(|w| !STOP.contains(w))
            .collect();
        let key = |i: usize| -> f64 {
            let t = c.docs[i].title.as_str().unwrap_or("").to_lowercase();
            let dw: HashSet<&str> = WORDS.find_iter(&t).map(|m| m.as_str()).collect();
            -(f64::from(js[i]) + 0.2 * tw.intersection(&dw).count() as f64)
        };
        let mut jd: Vec<usize> = (0..n).filter(|&i| c.docs[i].kind == DocKind::Joke).collect();
        jd.sort_by(|&a, &b| key(a).total_cmp(&key(b)));
        if let Some(&top) = jd.first() {
            let jid = c.docs[top].id.clone();
            for x in c.docs.values() {
                if x.joke.as_deref() == Some(jid.as_str()) {
                    add(&x.id, 1.0);
                }
            }
            focus.insert(jid);
        }
    }
    let keep = |d: &Doc| -> bool {
        let y = d.year();
        if plan.year_min != 0 && y.is_none_or(|y| y < plan.year_min) {
            return false;
        }
        if plan.year_max != 0 && y.is_none_or(|y| y > plan.year_max) {
            return false;
        }
        let performed = d.joke.as_deref().is_some_and(|j| kb.performed.contains(j));
        if plan.performed == "never"
            && (!matches!(d.kind, DocKind::Written | DocKind::Memo | DocKind::Joke) || performed)
        {
            return false;
        }
        if plan.performed == "performed" && matches!(d.kind, DocKind::Written | DocKind::Memo) && !performed {
            return false;
        }
        true
    };
    let mut ranked: Vec<(&String, f64)> = score.iter().map(|(k, s)| (k, *s)).collect();
    ranked.sort_by(|a, b| (-a.1).total_cmp(&-b.1));
    let cap_all = if focus.is_empty() { MAX_SOURCES } else { 48 };
    let mut picked: Vec<String> = Vec::new();
    let mut per: HashMap<String, usize> = HashMap::new();
    for (i, _) in ranked {
        let Some(d) = c.docs.get(i.as_str()) else { continue };
        if !keep(d) {
            continue;
        }
        let j = d.joke.clone().unwrap_or_else(|| i.clone());
        let cap = if focus.contains(&j) { 40 } else { PER_JOKE };
        let seen = per.entry(j).or_insert(0);
        if *seen >= cap {
            continue;
        }
        *seen += 1;
        picked.push(i.clone());
        if picked.len() >= cap_all {
            break;
        }
    }
    if plan.by_year {
        let date = |i: &String| -> String {
            let d = &c.docs[i.as_str()].date;
            if py::truthy(Some(d)) {
                py::str(d)
            } else {
                "9999".to_string()
            }
        };
        picked.sort_by_key(date);
    }
    picked
}

/// Step 3: the answer's messages: `(role, content)`.
pub fn answer_messages(
    c: &AskCorpus,
    kb: &Kb,
    question: &str,
    picked: &[String],
    by_year: bool,
    comic: &str,
) -> [(&'static str, String); 2] {
    let lines: Vec<String> = picked
        .iter()
        .filter_map(|i| c.docs.get(i.as_str()))
        .map(|d| {
            let jt = match &d.joke {
                Some(j) if *j != d.id => py::str(&kb.title(j)),
                _ => String::new(),
            };
            let txt = if d.text.chars().count() <= SNIPPET {
                d.text.clone()
            } else {
                format!("{} [...]", d.text.chars().take(SNIPPET).collect::<String>())
            };
            let date = if py::truthy(Some(&d.date)) {
                py::str(&d.date)
            } else {
                "undated".into()
            };
            let joke = if jt.is_empty() {
                String::new()
            } else {
                format!(" | joke: {jt}")
            };
            format!(
                "[{}] {} | {date} | {}{joke}\n{txt}",
                d.id,
                d.kind.head(),
                py::str(&d.title)
            )
        })
        .collect();
    let order = if by_year { " and ordered by year" } else { "" };
    [
        ("system", system(comic)),
        (
            "user",
            format!(
                "QUESTION: {question}\n\nSOURCES:\n\n{}\n\nAnswer from these sources only. Be complete: list every \
                 source that is relevant (not just a sample), grouped sensibly{order}. Cite source ids in square \
                 brackets in the answer, one short line per item (plain text, no tables, no quotes in the answer: \
                 quotes go in citations only). Read the question literally (e.g. 'my dad' means his father, not him \
                 being a dad). citations: one per relevant source, quote = a short exact line from it. Sources that \
                 are off-topic must be ignored. If the sources do not answer the question, set found=false, say so \
                 plainly in answer, and say what is missing in not_found.",
                lines.join("\n\n")
            ),
        ),
    ]
}

/// `SYS`, naming him as `PK_COMIC_NAME` does (default `the comic`).
pub fn system(comic: &str) -> String {
    format!(
        "You answer questions about stand-up comic {comic}'s own material (his written joke docs, recorded sets and \
         voice notes). Use ONLY the numbered SOURCES given. Never use outside knowledge and never invent material. \
         Quotes must be copied character for character from the source they cite."
    )
}

/// Step 4's outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct Verified {
    /// The model said found and something verified survived.
    pub found: bool,
    /// The answer, ids normalised, unverified quotes cut, unretrieved ids marked.
    pub answer: String,
    /// What the model said is missing.
    pub not_found: Value,
    /// The citations kept (a failed quote kept as its source with the quote emptied and flagged).
    pub citations: Vec<Value>,
    /// What was removed and why.
    pub removed: Vec<Value>,
}

fn s_of<'a>(m: &'a Map<String, Value>, k: &str) -> &'a str {
    m.get(k).and_then(Value::as_str).unwrap_or("")
}

/// Step 4 over the model's schema-checked answer.
pub fn verify(c: &AskCorpus, kb: &Kb, picked: &[String], data: &Value) -> Verified {
    let picked_set: HashSet<&str> = picked.iter().map(String::as_str).collect();
    let mut kept: Vec<Value> = Vec::new();
    let mut removed: Vec<Value> = Vec::new();
    let empty = Vec::new();
    for cit in data.get("citations").and_then(Value::as_array).unwrap_or(&empty) {
        let Some(cm) = cit.as_object() else { continue };
        let sid = normalize_ids(s_of(cm, "source_id"));
        let sid = py::strip(&sid).trim_matches(|ch| ch == '[' || ch == ']').to_string();
        let doc = c.docs.get(sid.as_str()).filter(|_| picked_set.contains(sid.as_str()));
        let Some(d) = doc else {
            let mut r = cm.clone();
            r.insert("why".into(), json!("cites a source that was not retrieved"));
            removed.push(Value::Object(r));
            continue;
        };
        let note = cm.get("note").cloned().unwrap_or(Value::Null);
        match verbatim(s_of(cm, "quote"), &d.text) {
            None => {
                let mut r = cm.clone();
                r.insert("why".into(), json!(format!("quote not found verbatim in {sid}")));
                removed.push(Value::Object(r));
                kept.push(
                    json!({"source_id": sid, "quote": "", "note": note, "kind": d.kind.name(),
                                 "date": d.date, "title": d.title, "flag": "quote removed"}),
                );
            }
            Some(exact) => {
                let joke = d.joke.as_deref().map_or(Value::Null, |j| kb.title(j));
                kept.push(
                    json!({"source_id": sid, "quote": exact, "note": note, "kind": d.kind.name(),
                                 "date": d.date, "title": d.title, "joke": joke}),
                );
            }
        }
    }
    let mut ans = data.get("answer").and_then(Value::as_str).unwrap_or("").to_string();
    if kept.is_empty() {
        // the model put everything in the prose: the ids it cited, without quotes
        let mut seen: HashSet<String> = HashSet::new();
        for m in PROSE_IDS.find_iter(&ans) {
            let sid = normalize_ids(m.as_str());
            if !seen.insert(sid.clone()) {
                continue;
            }
            if let Some(d) = c.docs.get(sid.as_str()).filter(|_| picked_set.contains(sid.as_str())) {
                kept.push(
                    json!({"source_id": sid, "quote": "", "note": "cited in the answer", "kind": d.kind.name(),
                                 "date": d.date, "title": d.title}),
                );
            }
        }
    }
    let mut ids: Vec<String> = Vec::new();
    for m in ANSWER_IDS.find_iter(&ans) {
        if !ids.iter().any(|x| x == m.as_str()) {
            ids.push(m.as_str().to_string());
        }
    }
    for sid in ids {
        let norm = normalize_ids(&sid);
        if c.docs.contains_key(norm.as_str()) {
            ans = ans.replace(&sid, &norm);
        }
    }
    let texts: Vec<&str> = picked
        .iter()
        .filter_map(|i| c.docs.get(i.as_str()))
        .map(|d| d.text.as_str())
        .collect();
    let (mut answer, inline) = strip_unverified_inline_quotes(&ans, &texts, 4);
    removed.extend(
        inline.into_iter().map(
            |q| json!({"source_id": null, "quote": q, "why": "quoted in the answer but not verbatim in any source"}),
        ),
    );
    let mut bad: Vec<String> = BRACKET_IDS
        .captures_iter(&answer)
        .map(|m| m[1].trim_end_matches([':', '.']).to_string())
        .filter(|x| !picked_set.contains(x.as_str()))
        .collect();
    bad.sort();
    bad.dedup();
    for x in &bad {
        answer = answer.replace(x.as_str(), &format!("{x} (not a retrieved source)"));
    }
    if !bad.is_empty() {
        removed.push(json!({"source_id": null, "quote": "",
                            "why": format!("answer cited ids that were not retrieved: {}", py::repr(&json!(bad)))}));
    }
    let said = data.get("found").is_some_and(|f| py::truthy(Some(f)));
    let found = said && !kept.is_empty();
    if !found && said {
        answer = format!("No verified support: {answer}");
    }
    Verified {
        found,
        answer,
        not_found: data.get("not_found").cloned().unwrap_or(json!("")),
        citations: kept,
        removed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fts_query_is_the_python_one() {
        assert_eq!(
            fts_expr("What have I written about my DAD's airport bit?").as_deref(),
            Some("\"dad's\" OR \"airport\"")
        );
        assert_eq!(fts_expr("did I ever"), None);
    }

    #[test]
    fn settle_keeps_known_labels_and_named_years_only() {
        let labels = vec![
            ("label:travel".to_string(), Some(json!("Travel"))),
            ("label:x".to_string(), None),
        ];
        let mut p = Plan::new("travel bits from 2024");
        p.update(
            &json!({"search_queries": ["a", "b", "c", "d", "e"], "label_ids": ["label:nope"], "year_min": 0,
                         "year_max": 0, "performed": "any", "joke_titles": [], "by_year": false}),
        );
        p.settle("travel bits from 2024", &labels);
        assert_eq!(p.label_ids, ["label:travel"]);
        assert_eq!((p.year_min, p.year_max), (2024, 2024));
        assert_eq!(p.search_queries.len(), 5);
        assert_eq!(p.search_queries[4], "travel bits from 2024");
        let mut q = Plan::new("no year here");
        q.update(&json!({"year_min": 2019, "year_max": 2020}));
        q.settle("no year here", &labels);
        assert_eq!(
            (q.year_min, q.year_max),
            (0, 0),
            "a year the question never named is dropped"
        );
    }
}
