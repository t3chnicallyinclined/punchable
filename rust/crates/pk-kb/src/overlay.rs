//! The AI layer: `inferred.json` and the per-set `analysis.json` files laid over `graph.json`.
//!
//! Precedence (`docs/KB_SCHEMA.md`, "AI-first display rule"): the comic's decisions > inferred > proposed. Mirrors
//! `api-dev/ai_overlay.py` `build_overlay` and `remap_ai_ids`, with no IO: the caller hands over the parsed files
//! ([`OverlayFiles`]) and gets back the merged graph and the [`Index`] the side tables are written from.
//!
//! The overlay is tolerant on purpose (the infer pipeline writes these files): a document of the wrong shape is
//! skipped where the Python skips it. Where the Python would raise (`int()` of a non-number start, a key that is
//! not hashable) this returns the error text, which the route answers as the Python's 500.

use indexmap::IndexMap;
use pk_core::py;
use serde_json::{Map, Value, json};

use crate::ids::edge_id;
use crate::model::Rec;

/// The ratings the AI may give a performance.
pub const RATINGS: [&str; 4] = ["killed", "worked", "meh", "died"];
/// The performance kinds that are not counted as bits.
pub const NOT_COUNTED: [&str; 2] = ["crowd_work", "banter"];

/// The parsed input files. Every document is already a JSON object (`AIFiles._load` drops anything else).
#[derive(Debug, Clone, Default)]
pub struct OverlayFiles {
    /// `kb/v1/id_map.json`: v0 id -> v1 id for jokes, units and edges.
    pub id_map: Option<Value>,
    /// `inferred.json`.
    pub inferred: Option<Doc>,
    /// Every set's `analysis.json` whose `jokes` is a list, with the set's folder name; any order (sorted here).
    pub analyses: Vec<(String, Doc)>,
    /// Each set's `meta.json` by folder name (a set that is not in the graph is made from it).
    pub set_meta: IndexMap<String, Value>,
}

/// One AI document with where it came from.
#[derive(Debug, Clone)]
pub struct Doc {
    /// The parsed object.
    pub doc: Value,
    /// `real` or `fixture`.
    pub source: String,
    /// The path it was read from (shown in `load_info`).
    pub path: String,
}

/// What the overlay leaves for the side tables and `load_info`.
#[derive(Debug, Clone, Default)]
pub struct Index {
    /// Set id -> the analysis rows with the performance each landed on.
    pub rows: IndexMap<String, Vec<(Value, String)>>,
    /// Set id -> its analysis meta (`source`, `path`, `run_id`, `generated_at`, `summary`, `set`).
    pub analysis: IndexMap<String, Value>,
    /// The inferred run's meta, when there is one.
    pub inferred: Option<Value>,
    /// The id-map log (`applied`, `pairs`, `unresolved`), when an id map was given.
    pub id_map: Option<Value>,
}

/// `set:YYYY-MM-DD_slug` -> `YYYYMMDD_slug` (`set_dir_of`).
pub fn set_dir_of(sid: &str) -> Option<String> {
    let sid = sid.strip_prefix("set:").unwrap_or(sid);
    let (date, slug) = sid.split_once('_')?;
    let d: Vec<&str> = date.split('-').collect();
    let (y, m, dd) = match d.as_slice() {
        [y, m, dd] => (*y, *m, *dd),
        [one] if one.len() == 8 => (&one[..4], &one[4..6], &one[6..]),
        _ => return None,
    };
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    (digits(y, 4) && digits(m, 2) && digits(dd, 2) && slug_ok(slug)).then(|| format!("{y}{m}{dd}_{slug}"))
}

/// `[A-Za-z0-9_-]+` (and, under `re.match` with `$`, nothing after one final newline is allowed here).
fn slug_ok(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `YYYYMMDD_slug` -> `set:YYYY-MM-DD_slug` (`set_id_of`).
pub fn set_id_of(d: &str) -> Option<String> {
    let (date, slug) = d.split_once('_')?;
    (date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) && slug_ok(slug))
        .then(|| format!("set:{}-{}-{}_{slug}", &date[..4], &date[4..6], &date[6..]))
}

fn s_of(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

/// `x.get(k) or default` for a string result: the string when truthy.
fn truthy_str(v: Option<&Value>) -> Option<&str> {
    s_of(v).filter(|s| !s.is_empty())
}

fn get_num(v: Option<&Value>) -> f64 {
    // `p.get("start_ms") or 0` over a number; anything else counts as 0 (the Python would raise on a string)
    v.and_then(Value::as_f64).unwrap_or(0.0)
}

fn rec_get<'a>(nodes: &'a IndexMap<String, Rec>, id: Option<&str>) -> Option<&'a Rec> {
    id.and_then(|i| nodes.get(i))
}

fn type_of<'a>(nodes: &'a IndexMap<String, Rec>, id: Option<&str>) -> Option<&'a str> {
    s_of(rec_get(nodes, id).and_then(|n| n.get("type")))
}

fn overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

/// The performance fields an inferred node never overwrites (`SPAN`).
const SPAN: [&str; 8] = ["id", "type", "set", "start_ms", "end_ms", "text", "status", "source"];

/// The mapping log `remap_ai_ids` fills.
#[derive(Debug, Clone, Default)]
struct MapLog {
    applied: usize,
    pairs: Vec<String>,
    unresolved: Vec<String>,
}

impl MapLog {
    fn to_value(&self) -> Value {
        json!({"applied": self.applied, "pairs": self.pairs, "unresolved": self.unresolved})
    }
}

/// `remap_ai_ids`: resolve v0 ids in an inferred / analysis document through the id map. `Some(copy)` when
/// something was remapped, `None` when the document is to be used as it is.
fn remap(doc: &Value, inferred: bool, idmap: &Value, known: &IndexMap<String, Rec>, log: &mut MapLog) -> Option<Value> {
    let map_of = |k: &str| idmap.get(k).and_then(Value::as_object).cloned().unwrap_or_default();
    let (jm, um, em) = (map_of("jokes"), map_of("units"), map_of("edges"));
    let mut changed: Vec<(String, String)> = Vec::new();
    let mut node = |x: &Value, log: &mut MapLog| -> Value {
        let Some(s) = x.as_str().filter(|s| !s.is_empty()) else {
            return x.clone();
        };
        if known.contains_key(s) {
            return x.clone();
        }
        let y = if s.starts_with("joke:") {
            s_of(jm.get(s))
        } else if s.starts_with("unit:") {
            s_of(um.get(s))
        } else {
            None
        };
        if let Some(y) = y.filter(|y| !y.is_empty() && known.contains_key(*y)) {
            changed.push((s.to_string(), y.to_string()));
            return Value::String(y.to_string());
        }
        if (s.starts_with("joke:") || s.starts_with("unit:")) && !log.unresolved.iter().any(|u| u == s) {
            log.unresolved.push(s.to_string());
        }
        x.clone()
    };
    let mut d = doc.clone();
    if inferred {
        match d.get_mut("jokes") {
            Some(Value::Array(js)) => {
                for j in js.iter_mut().filter_map(Value::as_object_mut) {
                    for k in ["id", "joke_id"] {
                        if let Some(v) = j.get(k).cloned() {
                            j.insert(k.into(), node(&v, log));
                        }
                    }
                }
            }
            Some(Value::Object(js)) => {
                for j in js.values_mut().filter_map(Value::as_object_mut) {
                    for k in ["id", "joke_id"] {
                        if let Some(v) = j.get(k).cloned() {
                            j.insert(k.into(), node(&v, log));
                        }
                    }
                }
                let renamed: Map<String, Value> = std::mem::take(js)
                    .into_iter()
                    .map(|(k, v)| {
                        let nk = match node(&Value::String(k), log) {
                            Value::String(s) => s,
                            _ => String::new(),
                        };
                        (nk, v)
                    })
                    .collect();
                *js = renamed;
            }
            _ => {}
        }
        if let Some(Value::Array(es)) = d.get_mut("edges") {
            for e in es.iter_mut().filter_map(Value::as_object_mut) {
                let (s0, t0) = (
                    e.get("src").cloned().unwrap_or(Value::Null),
                    e.get("dst").cloned().unwrap_or(Value::Null),
                );
                let (s1, t1) = (node(&s0, log), node(&t0, log));
                e.insert("src".into(), s1.clone());
                e.insert("dst".into(), t1.clone());
                if let Some(old) = s_of(e.get("id")).map(str::to_string)
                    && let Some(new) = truthy_str(em.get(&old))
                    && (s1 != s0 || t1 != t0)
                {
                    e.insert("id".into(), Value::String(new.to_string()));
                }
            }
        }
    } else {
        if let Some(Value::Array(rows)) = d.get_mut("jokes") {
            for row in rows.iter_mut().filter_map(Value::as_object_mut) {
                for k in ["joke_id", "version_unit_id"] {
                    if py::truthy(row.get(k)) {
                        let v = row[k].clone();
                        row.insert(k.into(), node(&v, log));
                    }
                }
                if let Some(Value::Array(sim)) = row.get_mut("similar") {
                    for x in sim.iter_mut().filter_map(Value::as_object_mut) {
                        if py::truthy(x.get("joke_id")) {
                            let v = x["joke_id"].clone();
                            x.insert("joke_id".into(), node(&v, log));
                        }
                    }
                }
            }
        }
        if let Some(Value::Object(sm)) = d.get_mut("summary") {
            for k in ["best", "weakest"] {
                if let Some(v) = sm.get(k).filter(|v| v.is_string()).cloned() {
                    sm.insert(k.into(), node(&v, log));
                }
            }
        }
    }
    if changed.is_empty() {
        return None;
    }
    log.applied += changed.len();
    for (a, b) in changed {
        let pair = format!("{a} -> {b}");
        if !log.pairs.contains(&pair) {
            log.pairs.push(pair);
        }
    }
    Some(d)
}

/// `x.get("status") in (None, "proposed")`, or with `"inferred"` too: not judged yet by anyone stronger.
fn is_open(x: &Rec, with_inferred: bool) -> bool {
    match x.get("status") {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s == "proposed" || (with_inferred && s == "inferred"),
        _ => false,
    }
}

fn arr(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// `str(x)` of an optional JSON value as an f-string writes it (`None` when absent).
fn fstr(v: Option<&Value>) -> String {
    v.map_or_else(|| "None".to_string(), py::str)
}

/// The edges of a kind, grouped by the performance end: `by_perf` / `pa_by_perf`.
fn perf_end(x: &Rec) -> String {
    let src = x.get("src");
    if py::str(src.unwrap_or(&Value::Null)).starts_with("perf:") {
        fstr(src)
    } else {
        fstr(x.get("dst"))
    }
}

/// `build_overlay`: the merged graph (`graph.json` with the AI layer applied) and the index.
#[expect(
    clippy::too_many_lines,
    reason = "one function in the Python; kept in its order so the two can be read side by side"
)]
pub fn build_overlay(graph: &Value, files: &OverlayFiles) -> Result<(Value, Index), String> {
    let mut nodes: IndexMap<String, Rec> = IndexMap::new();
    for n in arr(graph.get("nodes")).iter().filter_map(Value::as_object) {
        if let Some(id) = truthy_str(n.get("id")) {
            nodes.insert(id.to_string(), n.clone());
        }
    }
    let mut edges: IndexMap<String, Rec> = IndexMap::new();
    for e in arr(graph.get("edges")).iter().filter_map(Value::as_object) {
        if let Some(id) = truthy_str(e.get("id")) {
            edges.insert(id.to_string(), e.clone());
        }
    }
    let mut index = Index::default();
    let idmap = files.id_map.as_ref().filter(|m| py::truthy(Some(m)));
    let mut maplog = MapLog::default();

    // ---- inferred.json
    if let Some(inf) = &files.inferred {
        let doc = match idmap.and_then(|m| remap(&inf.doc, true, m, &nodes, &mut maplog)) {
            Some(d) => d,
            None => inf.doc.clone(),
        };
        if py::truthy(Some(&doc)) {
            apply_inferred(&doc, inf, &mut nodes, &mut edges, &mut index);
        }
    }

    // ---- analysis.json, per set
    let mut perfs_by_set: IndexMap<String, Vec<String>> = IndexMap::new();
    for (id, n) in &nodes {
        if s_of(n.get("type")) == Some("performance") {
            perfs_by_set.entry(fstr(n.get("set"))).or_default().push(id.clone());
        }
    }
    let mut pa_by_perf: IndexMap<String, Vec<String>> = IndexMap::new();
    for e in edges.values() {
        if s_of(e.get("kind")) == Some("performed_as") {
            pa_by_perf.entry(perf_end(e)).or_default().push(fstr(e.get("id")));
        }
    }
    let mut analyses: Vec<&(String, Doc)> = files.analyses.iter().collect();
    analyses.sort_by(|a, b| a.0.cmp(&b.0));
    for (d, an) in analyses {
        let doc = match idmap.and_then(|m| remap(&an.doc, false, m, &nodes, &mut maplog)) {
            Some(x) => x,
            None => an.doc.clone(),
        };
        let Some(sid) = set_id_of(d) else { continue };
        if !nodes.contains_key(&sid) {
            let meta = files.set_meta.get(d).cloned().unwrap_or_else(|| json!({}));
            let title = truthy_str(meta.get("title")).unwrap_or(d);
            let dur = if py::truthy(meta.get("duration")) {
                meta["duration"].clone()
            } else if let Some(x) = doc
                .get("summary")
                .and_then(|s| s.get("duration_s"))
                .filter(|v| py::truthy(Some(v)))
            {
                x.clone()
            } else {
                json!(0)
            };
            let mut n = Rec::new();
            n.insert("id".into(), json!(sid));
            n.insert("type".into(), json!("set"));
            n.insert("title".into(), json!(title));
            n.insert("date".into(), json!(&sid[4..14]));
            n.insert("venue".into(), json!(truthy_str(meta.get("venue_guess")).unwrap_or("")));
            n.insert("duration_s".into(), dur);
            n.insert("_ai_node".into(), json!(true));
            nodes.insert(sid.clone(), n);
        }
        let run = truthy_str(doc.get("run_id")).unwrap_or("analysis").to_string();
        let summary = if py::truthy(doc.get("summary")) {
            doc["summary"].clone()
        } else {
            json!({})
        };
        index.analysis.insert(
            sid.clone(),
            json!({"source": an.source, "path": an.path, "run_id": run,
                   "generated_at": doc.get("generated_at").cloned().unwrap_or(Value::Null),
                   "summary": summary, "set": d}),
        );
        let mut used: Vec<String> = Vec::new();
        let mut rows: Vec<(Value, String)> = Vec::new();
        for row in arr(doc.get("jokes")) {
            let Some(r) = row.as_object() else { continue };
            let Some(start) = r.get("start_ms").filter(|v| !v.is_null()) else {
                continue;
            };
            let s0 = py::int(Some(start)).map_err(|e| e.to_string())?;
            let end = if py::truthy(r.get("end_ms")) {
                r.get("end_ms")
            } else {
                Some(start)
            };
            let s1 = py::int(end).map_err(|e| e.to_string())?;
            let (s0f, s1f) = (s0 as f64, s1 as f64);
            let mut best: Option<String> = None;
            let mut bo = 0.0_f64;
            for pid in perfs_by_set.get(&sid).map_or(&[][..], Vec::as_slice) {
                if used.contains(pid) {
                    continue;
                }
                let p = &nodes[pid];
                let (p0, p1) = (get_num(p.get("start_ms")), get_num(p.get("end_ms")));
                let mut o = overlap(s0f, s1f, p0, p1);
                if (p0 - s0f).abs() <= 3000.0 || o >= 0.5 * (s1f - s0f).min(p1 - p0).max(1.0) {
                    o = o.max(1.0);
                    if o > bo {
                        best = Some(pid.clone());
                        bo = o;
                    }
                }
            }
            let status = match s_of(r.get("status")) {
                Some(s @ ("known" | "new" | "crowd_work" | "banter")) => s,
                _ if py::truthy(r.get("joke_id")) => "known",
                _ => "new",
            };
            let pid = if let Some(b) = best {
                b
            } else {
                let pid = format!("perf:{}@{s0}", &sid[4..]);
                if !nodes.contains_key(&pid) {
                    let mut pn = Rec::new();
                    pn.insert("id".into(), json!(pid));
                    pn.insert("type".into(), json!("performance"));
                    pn.insert("set".into(), json!(sid));
                    pn.insert("start_ms".into(), json!(s0));
                    pn.insert("end_ms".into(), json!(s1));
                    pn.insert(
                        "text".into(),
                        json!(if py::truthy(r.get("transcript")) {
                            r["transcript"].clone()
                        } else {
                            json!("")
                        }),
                    );
                    pn.insert("kind".into(), json!("bit"));
                    pn.insert(
                        "name".into(),
                        json!(if py::truthy(r.get("title")) {
                            r["title"].clone()
                        } else {
                            json!("")
                        }),
                    );
                    pn.insert("_ai_node".into(), json!(true));
                    nodes.insert(pid.clone(), pn);
                    perfs_by_set.entry(sid.clone()).or_default().push(pid.clone());
                }
                pid
            };
            used.push(pid.clone());
            let pn = nodes.get_mut(&pid).ok_or("performance vanished")?;
            if let Some(rt) = s_of(r.get("ai_rating")).filter(|x| RATINGS.contains(x)) {
                pn.insert("ai_rating".into(), json!(rt));
                pn.insert(
                    "rating_basis".into(),
                    if py::truthy(r.get("rating_basis")) {
                        r["rating_basis"].clone()
                    } else {
                        json!("")
                    },
                );
            }
            if NOT_COUNTED.contains(&status) {
                pn.insert("kind".into(), json!(status));
            } else if s_of(pn.get("kind")).is_some_and(|k| NOT_COUNTED.contains(&k)) {
                pn.insert("kind".into(), json!("bit"));
            }
            let kind = pn.get("kind").cloned().unwrap_or(Value::Null);
            pn.insert("_ai_kind".into(), kind);
            let jid = truthy_str(r.get("joke_id")).map(str::to_string);
            if status == "known" && type_of(&nodes, jid.as_deref()) == Some("joke") {
                let jid = jid.unwrap_or_default();
                let eid = edge_id(&pid, "performed_as", &jid);
                for other in pa_by_perf.get(&pid).cloned().unwrap_or_default() {
                    if other != eid
                        && let Some(x) = edges.get_mut(&other)
                        && is_open(x, true)
                    {
                        x.insert("status".into(), json!("superseded"));
                    }
                }
                let pl: Vec<&Map<String, Value>> = arr(r.get("punchlines"))
                    .iter()
                    .filter_map(Value::as_object)
                    .filter(|x| py::truthy(x.get("text")))
                    .collect();
                if let Some(x) = edges.get_mut(&eid) {
                    x.insert("status".into(), json!("inferred"));
                    let conf = match r.get("match_confidence") {
                        Some(c) => c.clone(),
                        None => x.get("confidence").cloned().unwrap_or(Value::Null),
                    };
                    x.insert("confidence".into(), conf);
                    if py::truthy(r.get("match_reason")) {
                        x.insert("reason".into(), r["match_reason"].clone());
                    }
                } else {
                    let evidence = pl.first().map_or_else(
                        || json!([]),
                        |p| json!([{"node": pid, "quote": p["text"], "start_ms": p.get("at_ms").cloned().unwrap_or(Value::Null)}]),
                    );
                    let mut e = Rec::new();
                    e.insert("id".into(), json!(eid));
                    e.insert("src".into(), json!(pid));
                    e.insert("dst".into(), json!(jid));
                    e.insert("kind".into(), json!("performed_as"));
                    e.insert("status".into(), json!("inferred"));
                    e.insert(
                        "confidence".into(),
                        if py::truthy(r.get("match_confidence")) {
                            r["match_confidence"].clone()
                        } else {
                            json!(0.5)
                        },
                    );
                    e.insert(
                        "reason".into(),
                        if py::truthy(r.get("match_reason")) {
                            r["match_reason"].clone()
                        } else {
                            json!("")
                        },
                    );
                    e.insert("evidence".into(), evidence);
                    e.insert("source".into(), json!(format!("auto:{run}")));
                    edges.insert(eid.clone(), e);
                    pa_by_perf.entry(pid.clone()).or_default().push(eid);
                }
            } else if status == "new" {
                for other in pa_by_perf.get(&pid).cloned().unwrap_or_default() {
                    if let Some(x) = edges.get_mut(&other)
                        && is_open(x, true)
                    {
                        x.insert("status".into(), json!("superseded"));
                    }
                }
            }
            rows.push((row.clone(), pid));
        }
        index.rows.insert(sid, rows);
    }
    if idmap.is_some() {
        index.id_map = Some(maplog.to_value());
    }
    let mut merged = Map::new();
    for k in ["version", "generated_at", "runs"] {
        merged.insert(k.into(), graph.get(k).cloned().unwrap_or(Value::Null));
    }
    merged.insert(
        "nodes".into(),
        Value::Array(nodes.into_values().map(Value::Object).collect()),
    );
    merged.insert(
        "edges".into(),
        Value::Array(edges.into_values().map(Value::Object).collect()),
    );
    Ok((Value::Object(merged), index))
}

/// The `inferred.json` half of `build_overlay`.
#[expect(clippy::too_many_lines, reason = "one block in the Python")]
fn apply_inferred(
    doc: &Value,
    inf: &Doc,
    nodes: &mut IndexMap<String, Rec>,
    edges: &mut IndexMap<String, Rec>,
    index: &mut Index,
) {
    let run = truthy_str(doc.get("run_id")).unwrap_or("inferred").to_string();
    index.inferred = Some(json!({"source": inf.source, "path": inf.path, "run_id": run,
                                 "generated_at": doc.get("generated_at").cloned().unwrap_or(Value::Null),
                                 "notes": doc.get("notes").cloned().unwrap_or(Value::Null)}));
    // an inferred performance that is a graph.json performance under another start time
    let mut alias: IndexMap<String, String> = IndexMap::new();
    let same_perf = |nodes: &IndexMap<String, Rec>, n: &Rec| -> Option<String> {
        let mut best: Option<String> = None;
        let mut bo = -1.0_f64;
        for (id, p) in nodes {
            if s_of(p.get("type")) != Some("performance")
                || p.get("set") != n.get("set")
                || py::truthy(p.get("_ai_node"))
            {
                continue;
            }
            let (s0, s1) = (get_num(n.get("start_ms")), get_num(n.get("end_ms")));
            let (p0, p1) = (get_num(p.get("start_ms")), get_num(p.get("end_ms")));
            let o = overlap(s0, s1, p0, p1);
            if ((p0 - s0).abs() <= 3000.0 || o >= 0.5 * (s1 - s0).min(p1 - p0).max(1.0)) && o > bo {
                best = Some(id.clone());
                bo = o;
            }
        }
        best
    };
    for n in arr(doc.get("nodes")).iter().filter_map(Value::as_object) {
        let (Some(id), Some(ty)) = (truthy_str(n.get("id")), truthy_str(n.get("type"))) else {
            continue;
        };
        let mut target = nodes.contains_key(id).then(|| id.to_string());
        if target.is_none()
            && ty == "performance"
            && let Some(m) = same_perf(nodes, n)
        {
            alias.insert(id.to_string(), m.clone());
            target = Some(m);
        }
        let Some(target) = target else {
            let mut fresh: Rec = n
                .iter()
                .filter(|(k, _)| *k != "status")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            fresh.insert("_ai_node".into(), json!(true));
            nodes.insert(id.to_string(), fresh);
            continue;
        };
        if let Some(x) = nodes.get_mut(&target) {
            for (k, val) in n {
                if !SPAN.contains(&k.as_str()) && !val.is_null() {
                    x.insert(k.clone(), val.clone());
                }
            }
        }
    }
    let jokes: Vec<Value> = match doc.get("jokes") {
        Some(Value::Object(m)) => m
            .iter()
            .filter_map(|(k, v)| {
                let mut o = Map::new();
                o.insert("id".into(), json!(k));
                for (k2, v2) in v.as_object()? {
                    o.insert(k2.clone(), v2.clone());
                }
                Some(Value::Object(o))
            })
            .collect(),
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    };
    for j in jokes.iter().filter_map(Value::as_object) {
        let jid = truthy_str(j.get("id"))
            .or_else(|| truthy_str(j.get("joke_id")))
            .map(str::to_string);
        if type_of(nodes, jid.as_deref()) != Some("joke") {
            continue;
        }
        let jid = jid.unwrap_or_default();
        let stage = truthy_str(j.get("stage")).or_else(|| truthy_str(j.get("inferred_stage")));
        if let Some(stage) = stage.filter(|s| ["idea", "to_try", "testing", "working", "retired"].contains(s)) {
            let n = nodes.get_mut(&jid).expect("checked above");
            let conf = j
                .get("stage_confidence")
                .or_else(|| j.get("confidence"))
                .cloned()
                .unwrap_or(Value::Null);
            let reason = truthy_str(j.get("stage_reason"))
                .or_else(|| truthy_str(j.get("reason")))
                .unwrap_or("");
            let ai = json!({"stage": stage, "from_stage": n.get("stage").cloned().unwrap_or(Value::Null),
                            "stage_confidence": conf, "stage_reason": reason, "run_id": run});
            n.insert("stage".into(), json!(stage));
            n.insert("_ai".into(), ai);
        }
        // `isinstance(hit_sets, int)`: a bool is an int in Python
        if let Some(h) = j
            .get("hit_sets")
            .filter(|h| h.is_boolean() || h.as_i64().is_some() || h.as_u64().is_some())
            && let Some(n) = nodes.get_mut(&jid)
        {
            n.insert("_ai_hit_sets".into(), h.clone());
        }
        for p in arr(j.get("performances")) {
            let Some(po) = p.as_object() else { continue };
            let pid = s_of(po.get("perf")).map(|x| alias.get(x).map_or(x, String::as_str).to_string());
            let r = truthy_str(po.get("ai_rating")).or_else(|| truthy_str(po.get("rating")));
            let (Some(pid), Some(r)) = (pid, r) else { continue };
            if type_of(nodes, Some(&pid)) == Some("performance") && RATINGS.contains(&r) {
                let pn = nodes.get_mut(&pid).expect("checked above");
                pn.insert("ai_rating".into(), json!(r));
                if py::truthy(po.get("rating_basis")) {
                    pn.insert("rating_basis".into(), po["rating_basis"].clone());
                }
            }
        }
    }
    for e in arr(doc.get("edges")).iter().filter_map(Value::as_object) {
        let Some(orig_id) = truthy_str(e.get("id")) else {
            continue;
        };
        let st = if py::truthy(e.get("status")) {
            e["status"].clone()
        } else {
            json!("inferred")
        };
        let ali = |k: &str| -> Option<Value> {
            e.get(k).map(|v| match v.as_str().and_then(|s| alias.get(s)) {
                Some(a) => json!(a),
                None => v.clone(),
            })
        };
        let (src, dst) = (ali("src"), ali("dst"));
        let moved = src.as_ref() != e.get("src") || dst.as_ref() != e.get("dst");
        let mut eid = orig_id.to_string();
        if !edges.contains_key(&eid) && moved && py::truthy(e.get("kind")) {
            let kind = if py::truthy(e.get("custom_kind")) {
                e.get("custom_kind")
            } else {
                e.get("kind")
            };
            eid = edge_id(&fstr(src.as_ref()), &fstr(kind), &fstr(dst.as_ref()));
        }
        if let Some(x) = edges.get_mut(&eid) {
            for k in ["status", "confidence", "reason", "evidence", "source"] {
                if let Some(v) = e.get(k).filter(|v| !v.is_null()) {
                    x.insert(k.into(), v.clone());
                }
            }
            x.insert("status".into(), st);
        } else if s_of(src.as_ref()).is_some_and(|s| nodes.contains_key(s))
            && s_of(dst.as_ref()).is_some_and(|s| nodes.contains_key(s))
            && py::truthy(e.get("kind"))
        {
            let mut ne: Rec = e.clone();
            ne.insert("id".into(), json!(eid));
            ne.insert("src".into(), src.unwrap_or(Value::Null));
            ne.insert("dst".into(), dst.unwrap_or(Value::Null));
            ne.insert("status".into(), st);
            let source = if py::truthy(e.get("source")) {
                e["source"].clone()
            } else {
                json!(format!("auto:{run}"))
            };
            ne.insert("source".into(), source);
            edges.insert(eid, ne);
        }
    }
    // inferred > proposed: an inferred match for a performance retires the other proposed matches for it
    let mut by_perf: IndexMap<String, Vec<String>> = IndexMap::new();
    for x in edges.values() {
        if s_of(x.get("kind")) == Some("performed_as") {
            by_perf.entry(perf_end(x)).or_default().push(fstr(x.get("id")));
        }
    }
    for ids in by_perf.values() {
        if ids
            .iter()
            .any(|i| edges.get(i).and_then(|x| s_of(x.get("status"))) == Some("inferred"))
        {
            for i in ids {
                if let Some(x) = edges.get_mut(i)
                    && is_open(x, false)
                {
                    x.insert("status".into(), json!("superseded"));
                }
            }
        }
    }
}
