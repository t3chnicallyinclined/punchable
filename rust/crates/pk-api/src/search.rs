//! Search over everything and "is this like anything?" (`api-dev/search_api.py`, rule for rule): the fused query
//! batch (`search_all`: the four full-text legs, BM25 with the `punch_en` analyzer, and the three HNSW legs when a
//! query vector exists, each family fused by `SurrealDB`'s own `search::rrf`), the vectors near one bit's newest text
//! (`similar`), and the payloads the app reads (`fold_jokes`, `newest_text`, `linked_joke`, `moment_item`,
//! `_snippet`, `bit_why`, `search_payload`, `similar_payload`).
//!
//! The batch is the Python's own text, not `pk-store`'s engine read: that one splits a question into one `@n@` leg
//! per word (any word, scores summed) and fuses differently; search here matches the whole query per leg, as the
//! Python does.
//!
//! The embedding of the stored rows (the Python's `Sweeper`) stays the Python's until T10 moves the writes.

use std::collections::HashSet;
use std::fmt::Write as _;

use pk_core::py::{strip, truthy};
use pk_kb::{Bucket, Rec, View, bucket};
use pk_store::{Store, from_db, to_db};
use serde_json::{Map, Value, json};

use crate::display::{at_label, short_name_of, venue_label, when_label};
use crate::index::AppIndex;

/// `MAX_LIMIT`.
pub const MAX_LIMIT: i64 = 50;
/// `SNIPPET`: characters of a moment's text or a matched line.
pub const SNIPPET: usize = 240;
/// The HNSW breadth every vector leg asks for.
const EF: usize = 150;

/// Raw fused hits (`search_all`'s answer): `(unit or joke id, score)` best first, `(performance id, score)` best
/// first, and whether a query vector took part. A score is whatever the row carried (`x.get("score")`).
#[derive(Debug, Clone, Default)]
pub struct Raw {
    /// Bits, through a unit or a joke.
    pub hits: Vec<(String, Value)>,
    /// Taped moments.
    pub moments: Vec<(String, Value)>,
    /// A query vector was used.
    pub vectors: bool,
}

/// `max(20, min(200, limit * f))`, with Python's unbounded integers kept in range.
pub fn depth(limit: i64, f: i64) -> usize {
    usize::try_from(limit.saturating_mul(f).clamp(20, 200)).unwrap_or(20)
}

/// The `search_all` batch, statement for statement; the last two statements are the bits and the moments.
pub fn search_sql(k: usize, include_hidden: bool, vector: bool) -> String {
    let hid = if include_hidden { "" } else { " AND hidden != true" };
    let mut sql = format!(
        "LET $ft = SELECT id, search::score(1) AS score FROM unit WHERE text @1@ $q{hid} ORDER BY score DESC LIMIT {k};\
         LET $jt = SELECT id, search::score(1) AS score FROM joke WHERE title @1@ $q{hid} ORDER BY score DESC LIMIT {k};\
         LET $js = SELECT id, search::score(1) AS score FROM joke WHERE summary @1@ $q{hid} ORDER BY score DESC LIMIT {k};\
         LET $pt = SELECT id, search::score(1) AS score FROM performance WHERE text @1@ $q ORDER BY score DESC LIMIT {k};"
    );
    let (legs_j, legs_p) = if vector {
        let _ = write!(
            sql,
            "LET $vs = SELECT id, vector::distance::knn() AS distance FROM unit WHERE embedding <|{k},{EF}|> $qvec{hid} ORDER BY distance;\
             LET $jv = SELECT id, vector::distance::knn() AS distance FROM joke WHERE embedding <|{k},{EF}|> $qvec{hid} ORDER BY distance;\
             LET $pv = SELECT id, vector::distance::knn() AS distance FROM performance WHERE embedding <|{k},{EF}|> $qvec ORDER BY distance;"
        );
        ("$ft, $jt, $js, $vs, $jv", "$pt, $pv")
    } else {
        ("$ft, $jt, $js", "$pt")
    };
    let _ = write!(
        sql,
        "RETURN search::rrf([{legs_j}], {k}, 60);RETURN search::rrf([{legs_p}], {k}, 60);"
    );
    sql
}

/// The `similar` statements, in the Python's order: the three vector legs near one passage vector (unit, joke,
/// performance; the moments never skip hidden rows, as in the Python).
pub fn similar_sql(k: usize, include_hidden: bool) -> [String; 3] {
    let hid = if include_hidden { "" } else { " AND hidden != true" };
    ["unit", "joke", "performance"].map(|table| {
        let hid = if table == "performance" { "" } else { hid };
        format!("SELECT id, vector::distance::knn() AS d FROM {table} WHERE embedding <|{k},{EF}|> $v{hid} ORDER BY d;")
    })
}

/// A vector as the database takes it: floats (each `f32` exactly, as Python's `float()` of it).
fn db_vector(v: &[f32]) -> Value {
    Value::Array(v.iter().map(|x| json!(f64::from(*x))).collect())
}

/// The rows of one statement's result, as JSON objects.
fn rows(v: Option<Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a,
        _ => Vec::new(),
    }
}

/// `(id, row[key])` of every row that is an object with an id (`isinstance(x, dict) and x.get("id") is not None`).
fn pairs(rows: Vec<Value>, key: &str) -> Vec<(String, Value)> {
    rows.into_iter()
        .filter_map(|mut x| {
            let o = x.as_object_mut()?;
            let id = match o.get("id")? {
                Value::Null => return None,
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            Some((id, o.remove(key).unwrap_or(Value::Null)))
        })
        .collect()
}

/// `search_all`: one round trip, the store's own fusion.
pub async fn search_all(
    store: &Store,
    qvec: Option<&[f32]>,
    q: &str,
    limit: i64,
    include_hidden: bool,
) -> pk_store::Result<Raw> {
    let k = depth(limit, 3);
    let sql = search_sql(k, include_hidden, qvec.is_some());
    let mut vars = vec![("q", to_db(&json!(q)))];
    if let Some(v) = qvec {
        vars.push(("qvec", to_db(&db_vector(v))));
    }
    let mut res: Vec<Value> = store
        .query(&sql, pk_store::vars(vars))
        .await?
        .into_iter()
        .map(from_db)
        .collect();
    let p = res.pop();
    let j = res.pop();
    Ok(Raw {
        hits: pairs(rows(j), "score"),
        moments: pairs(rows(p), "score"),
        vectors: qvec.is_some(),
    })
}

/// The close hits of `q`, for a topic's Found (`docs/design/SPEC-2026-11-06-rant-tab.md` §12.7; Rust only): the rows
/// whose words matched (`None`), best first, then the rows whose vector is under `close`, nearest first, each bit or
/// moment once. The fused `search_all` answers neither question: a score there is a rank, and a vector leg always
/// returns its limit, which is why a count of its hits only ever said the cap.
pub async fn close_hits(
    store: &Store,
    qvec: Option<&[f32]>,
    q: &str,
    limit: usize,
    close: f64,
    v: &View,
    include_hidden: bool,
) -> pk_store::Result<Vec<(String, Option<f64>)>> {
    let k = limit.saturating_mul(3).clamp(20, 200);
    let hid = if include_hidden { "" } else { " AND hidden != true" };
    let mut sql = format!(
        "SELECT id, search::score(1) AS score FROM unit WHERE text @1@ $q{hid} ORDER BY score DESC LIMIT {k};\
         SELECT id, search::score(1) AS score FROM joke WHERE title @1@ $q{hid} ORDER BY score DESC LIMIT {k};\
         SELECT id, search::score(1) AS score FROM joke WHERE summary @1@ $q{hid} ORDER BY score DESC LIMIT {k};\
         SELECT id, search::score(1) AS score FROM performance WHERE text @1@ $q ORDER BY score DESC LIMIT {k};"
    );
    let mut vars = vec![("q", to_db(&json!(q)))];
    if let Some(vec) = qvec {
        let _ = write!(
            sql,
            "SELECT id, vector::distance::knn() AS d FROM unit WHERE embedding <|{k},{EF}|> $qvec{hid} ORDER BY d;\
             SELECT id, vector::distance::knn() AS d FROM joke WHERE embedding <|{k},{EF}|> $qvec{hid} ORDER BY d;\
             SELECT id, vector::distance::knn() AS d FROM performance WHERE embedding <|{k},{EF}|> $qvec ORDER BY d;"
        );
        vars.push(("qvec", to_db(&db_vector(vec))));
    }
    let res: Vec<Vec<Value>> = store
        .query(&sql, pk_store::vars(vars))
        .await?
        .into_iter()
        .map(|x| rows(Some(from_db(x))))
        .collect();
    let (words, near) = res.split_at(res.len().min(4));
    let words: Vec<String> = words
        .iter()
        .flat_map(|legs| pairs(legs.clone(), "score"))
        .map(|(id, _)| id)
        .collect();
    let near: Vec<(String, Value)> = near.iter().flat_map(|legs| pairs(legs.clone(), "d")).collect();
    Ok(rank_close(words, near, close, v, limit))
}

/// [`close_hits`]' ranking: a unit folds into its bit, ids not in this view go, the word matches lead in the order they
/// came, then the vector rows under `close`, nearest first (a bit at its nearest); at most `limit`.
pub fn rank_close(
    words: Vec<String>,
    near: Vec<(String, Value)>,
    close: f64,
    v: &View,
    limit: usize,
) -> Vec<(String, Option<f64>)> {
    let fold = |id: &str| -> Option<String> {
        let j = if id.starts_with("unit:") {
            v.unit_joke.get(id)?.as_str()
        } else {
            id
        };
        v.nodes.contains_key(j).then(|| j.to_string())
    };
    let mut seen = HashSet::new();
    let mut out: Vec<(String, Option<f64>)> = Vec::new();
    for id in words {
        if let Some(j) = fold(&id)
            && seen.insert(j.clone())
        {
            out.push((j, None));
        }
    }
    let mut near: Vec<(String, f64)> = near
        .into_iter()
        .filter_map(|(id, d)| Some((fold(&id)?, d.as_f64()?)))
        .filter(|(_, d)| d.is_finite() && *d < close)
        .collect();
    near.sort_by(|a, b| a.1.total_cmp(&b.1));
    for (j, d) in near {
        if seen.insert(j.clone()) {
            out.push((j, Some(d)));
        }
    }
    out.truncate(limit);
    out
}

/// `similar`'s one round trip: the unit, joke and performance rows near a passage vector.
pub async fn similar_rows(
    store: &Store,
    vector: &[f32],
    limit: i64,
    include_hidden: bool,
) -> pk_store::Result<[Vec<Value>; 3]> {
    // three independent reads (the Python's one batch runs them one after another, each its own transaction): asked
    // at once, the database answers them side by side
    let v = to_db(&db_vector(vector));
    let leg = |sql: String| {
        let vars = pk_store::vars([("v", v.clone())]);
        async move { Ok::<_, pk_store::StoreError>(rows(store.query(&sql, vars).await?.pop().map(from_db))) }
    };
    let [u, j, p] = similar_sql(depth(limit, 4), include_hidden);
    let (u, j, p) = tokio::try_join!(leg(u), leg(j), leg(p))?;
    Ok([u, j, p])
}

/// `fold_jokes`: unit and joke hits, best first, as joke ids, each once, only jokes the view knows.
pub fn fold_jokes(hits: &[(String, Value)], v: &View) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for (nid, _) in hits {
        let jid = if nid.starts_with("joke:") {
            Some(nid.as_str())
        } else {
            v.unit_joke.get(nid).map(String::as_str)
        };
        if let Some(j) = jid
            && v.nodes.contains_key(j)
            && seen.insert(j.to_string())
        {
            out.push(j.to_string());
        }
    }
    out
}

fn sget<'a>(n: &'a Rec, k: &str) -> Option<&'a str> {
    n.get(k).and_then(Value::as_str)
}

/// `n.get(k) or ""` for a text field.
fn s_or<'a>(n: &'a Rec, k: &str) -> &'a str {
    sget(n, k).unwrap_or("")
}

/// The not-hidden versions of a bit as `(date or "", text or "")`, newest first, ties in version order (Python's
/// stable sort with `reverse=True` keeps equal keys in order).
fn versions<'a>(v: &'a View, jid: &str, need_text: bool) -> Vec<(&'a str, &'a str)> {
    let mut out: Vec<(&str, &str)> = v
        .joke_units
        .get(jid)
        .into_iter()
        .flatten()
        .filter_map(|(u, _)| v.nodes.get(u))
        .filter(|n| !truthy(n.get("hidden")) && (!need_text || !strip(s_or(n, "text")).is_empty()))
        .map(|n| (s_or(n, "date"), s_or(n, "text")))
        .collect();
    out.sort_by(|a, b| b.0.cmp(a.0));
    out
}

/// `newest_text`: what the bit says now, its newest version, else the typed idea.
pub fn newest_text(v: &View, jid: &str) -> String {
    if let Some((_, t)) = versions(v, jid, true).first() {
        return (*t).to_string();
    }
    strip(v.nodes.get(jid).map_or("", |n| s_or(n, "summary"))).to_string()
}

/// `s[:n]` over characters.
fn head(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map_or(s, |(i, _)| &s[..i])
}

/// `linked_joke`: the bit a moment is performed as (an edge either way, not rejected).
pub fn linked_joke<'a>(v: &'a View, pid: &str) -> Option<&'a str> {
    for eid in v.adj.get(pid).into_iter().flatten() {
        let Some(e) = v.edges.get(eid) else { continue };
        if sget(e, "kind") == Some("performed_as") && bucket(e.get("status")) != Bucket::Rejected {
            let other = if sget(e, "dst") == Some(pid) {
                sget(e, "src")
            } else {
                sget(e, "dst")
            };
            if v.ntype(other) == Some("joke") {
                return other;
            }
        }
    }
    None
}

/// `moment_item`: one taped moment as a row, or `None` for anything that is not a performance.
pub fn moment_item(v: &View, pid: &str, score: Value) -> Option<Map<String, Value>> {
    let n = v.nodes.get(pid)?;
    if sget(n, "type") != Some("performance") {
        return None;
    }
    let empty = Rec::new();
    let s = sget(n, "set").and_then(|sid| v.nodes.get(sid)).unwrap_or(&empty);
    let j = linked_joke(v, pid);
    let jn = j.and_then(|j| v.nodes.get(j)).filter(|jn| !jn.is_empty());
    let date = if truthy(s.get("show_date")) {
        s.get("show_date").cloned()
    } else {
        s.get("date").cloned()
    }
    .unwrap_or(Value::Null);
    let (venue_short, venue_guessed) = venue_label(sget(s, "venue"));
    let get = |m: &Rec, k: &str| m.get(k).cloned().unwrap_or(Value::Null);
    let mut out = Map::new();
    out.insert("id".into(), json!(pid));
    out.insert("set".into(), get(n, "set"));
    out.insert("set_title".into(), get(s, "title"));
    out.insert("venue".into(), get(s, "venue"));
    out.insert("when_label".into(), json!(when_label(Some(&date))));
    out.insert("date".into(), date);
    out.insert("start_ms".into(), get(n, "start_ms"));
    out.insert("end_ms".into(), get(n, "end_ms"));
    out.insert("text".into(), json!(head(strip(s_or(n, "text")), SNIPPET)));
    out.insert("joke".into(), j.map_or(Value::Null, |j| json!(j)));
    out.insert("joke_title".into(), jn.map_or(Value::Null, |jn| get(jn, "title")));
    out.insert("score".into(), score);
    out.insert("venue_short".into(), json!(venue_short));
    out.insert("venue_guessed".into(), json!(venue_guessed));
    out.insert("at_label".into(), at_label(n.get("start_ms")));
    out.insert(
        "short_name".into(),
        jn.map_or(Value::Null, |jn| {
            let title = if truthy(jn.get("title")) {
                sget(jn, "title").unwrap_or("")
            } else {
                j.unwrap_or("")
            };
            json!(short_name_of(title, sget(jn, "short_name")))
        }),
    );
    Some(out)
}

/// `_words`: the query lowercased, split on whitespace.
pub fn words(q: &str) -> Vec<String> {
    pk_core::py::split(&q.to_lowercase()).map(str::to_string).collect()
}

/// `hay.find(needle)` over characters (a non-empty needle).
fn find(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// `_snippet`: from just before the first query word the text holds, `SNIPPET` characters, `…` when cut at the
/// front; `None` when no word is in it.
pub fn snippet(text: &str, words: &[String]) -> Option<String> {
    let t: Vec<char> = strip(text).chars().collect();
    if t.is_empty() {
        return None;
    }
    // Python indexes `low` and slices `t` with the same numbers, even where lowercasing changed the length
    let low: Vec<char> = t.iter().collect::<String>().to_lowercase().chars().collect();
    let pos = words
        .iter()
        .filter_map(|w| find(&low, &w.chars().collect::<Vec<_>>()))
        .min()?;
    // `low.rfind(" ", 0, max(0, pos - 60)) + 1`, at least 0
    let end = pos.saturating_sub(60).min(low.len());
    let start = low[..end].iter().rposition(|c| *c == ' ').map_or(0, |i| i + 1);
    let from = start.min(t.len());
    let to = start.saturating_add(SNIPPET).min(t.len());
    let piece: String = t[from..to.max(from)].iter().collect();
    Some(format!("{}{}", if start > 0 { "…" } else { "" }, strip(&piece)))
}

/// `bit_why`: the line that matched, for the row under the bit: a version (newest first) or the typed idea.
pub fn bit_why(v: &View, jid: &str, words: &[String]) -> Option<String> {
    let summary = v.nodes.get(jid).map_or("", |n| s_or(n, "summary"));
    versions(v, jid, false)
        .into_iter()
        .map(|(_, t)| t)
        .chain(std::iter::once(summary))
        .find_map(|t| snippet(t, words))
}

/// One Bits-list row (`_summaries_by_id`: without `label_ids`) as a JSON object.
fn summary_row(idx: &AppIndex, jid: &str) -> Option<Map<String, Value>> {
    let row = idx.summaries.get(*idx.by_id.get(jid)?)?;
    match serde_json::to_value(&row.summary) {
        Ok(Value::Object(o)) => Some(o),
        _ => None,
    }
}

/// `search_payload`: `GET /api/search`.
pub fn search_payload(idx: &AppIndex, raw: &Raw, q: &str, limit: usize) -> Value {
    let v = &idx.view;
    let ws = words(q);
    let mut bits = Vec::new();
    for jid in fold_jokes(&raw.hits, v) {
        let Some(mut it) = summary_row(idx, &jid) else { continue };
        it.insert("why".into(), bit_why(v, &jid, &ws).map_or(Value::Null, Value::String));
        bits.push(Value::Object(it));
        if bits.len() >= limit {
            break;
        }
    }
    let moments: Vec<Value> = raw
        .moments
        .iter()
        .filter_map(|(pid, sc)| moment_item(v, pid, sc.clone()))
        .take(limit)
        .map(Value::Object)
        .collect();
    json!({"query": q, "bits": bits, "moments": moments, "vectors": raw.vectors})
}

/// `similar`'s raw answer: other bits `(jid, distance)`, moments `(pid, distance)`, whether vectors were used, and
/// the text that was compared.
#[derive(Debug, Clone, Default)]
pub struct Like {
    /// Other bits, nearest first.
    pub bits: Vec<(String, Value)>,
    /// Taped moments, nearest first.
    pub moments: Vec<(String, Value)>,
    /// Vectors were used.
    pub vectors: bool,
    /// The bit's newest text.
    pub text: String,
}

/// `float(x.get("d") or 0.0)`.
fn distance(d: &Value) -> f64 {
    if truthy(Some(d)) {
        d.as_f64().unwrap_or(0.0)
    } else {
        0.0
    }
}

/// `similar` with a passage vector of the bit's newest text: the unit and joke rows folded into other bits (each
/// at its nearest, first seen first on a tie, never one of its own versions or itself), and the moments as they
/// came.
pub fn like_vectors(
    v: &View,
    jid: &str,
    text: String,
    units: Vec<Value>,
    jokes: Vec<Value>,
    perfs: Vec<Value>,
    limit: usize,
) -> Like {
    let own: HashSet<&str> = v
        .joke_units
        .get(jid)
        .into_iter()
        .flatten()
        .map(|(u, _)| u.as_str())
        .collect();
    let mut best: indexmap::IndexMap<&str, f64> = indexmap::IndexMap::new();
    let rows = pairs(units, "d").into_iter().chain(pairs(jokes, "d"));
    let rows: Vec<(String, Value)> = rows.collect();
    for (nid, d) in &rows {
        if own.contains(nid.as_str()) {
            continue;
        }
        let j = if nid.starts_with("joke:") {
            v.nodes.get_key_value(nid.as_str()).map(|(k, _)| k.as_str())
        } else {
            v.unit_joke
                .get(nid)
                .and_then(|j| v.nodes.get_key_value(j.as_str()))
                .map(|(k, _)| k.as_str())
        };
        let Some(j) = j.filter(|j| *j != jid) else { continue };
        let d = distance(d);
        match best.get_mut(j) {
            Some(b) if d < *b => *b = d,
            Some(_) => {}
            None => {
                best.insert(j, d);
            }
        }
    }
    let mut bits: Vec<(&str, f64)> = best.into_iter().collect();
    bits.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    bits.truncate(limit);
    Like {
        bits: bits.into_iter().map(|(j, d)| (j.to_string(), json!(d))).collect(),
        moments: pairs(perfs, "d")
            .into_iter()
            .map(|(p, d)| (p, json!(distance(&d))))
            .collect(),
        vectors: true,
        text,
    }
}

/// `similar` without a model: the word search over the bit's first twelve words.
pub fn like_words(v: &View, jid: &str, text: String, raw: Raw, limit: usize) -> Like {
    Like {
        bits: fold_jokes(&raw.hits, v)
            .into_iter()
            .filter(|j| j != jid)
            .take(limit)
            .map(|j| (j, Value::Null))
            .collect(),
        moments: raw.moments.into_iter().take(limit).collect(),
        vectors: false,
        text,
    }
}

/// The words query `similar` sends without a model: `" ".join(text.split()[:12])`.
pub fn first_words(text: &str) -> String {
    pk_core::py::split(text).take(12).collect::<Vec<_>>().join(" ")
}

/// `similar_payload`'s first half: the moments linked to the bit (not rejected), each with its edge's bucket, the
/// newest night first.
pub fn performed(v: &View, jid: &str) -> Vec<Map<String, Value>> {
    let mut out: Vec<Map<String, Value>> = Vec::new();
    for (_, e) in v.joke_edges(jid) {
        let b = bucket(e.get("status"));
        if sget(e, "kind") != Some("performed_as") || b == Bucket::Rejected {
            continue;
        }
        let pid = if v.ntype(sget(e, "src")) == Some("performance") {
            sget(e, "src")
        } else {
            sget(e, "dst")
        };
        if let Some(mut m) = pid.and_then(|p| moment_item(v, p, Value::Null)) {
            m.insert("status".into(), json!(b.as_str()));
            out.push(m);
        }
    }
    let date = |m: &Map<String, Value>| m.get("date").and_then(Value::as_str).unwrap_or("").to_string();
    out.sort_by_cached_key(|m| std::cmp::Reverse(date(m)));
    out
}

/// `similar_payload`: `GET /api/jokes/<id>/similar` once the bit is known to exist.
pub fn similar_payload(idx: &AppIndex, performed: Vec<Map<String, Value>>, like: &Like, limit: usize) -> Value {
    let v = &idx.view;
    let linked: HashSet<String> = performed
        .iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let bits: Vec<Value> = like
        .bits
        .iter()
        .filter_map(|(j, d)| {
            let mut it = summary_row(idx, j)?;
            let newest = newest_text(v, j);
            let base = if newest.is_empty() {
                it.get("summary").and_then(Value::as_str).unwrap_or("").to_string()
            } else {
                newest
            };
            let why = head(&base, SNIPPET);
            it.insert("distance".into(), d.clone());
            it.insert("why".into(), if why.is_empty() { Value::Null } else { json!(why) });
            Some(Value::Object(it))
        })
        .take(limit)
        .collect();
    let moments: Vec<Value> = like
        .moments
        .iter()
        .filter(|(pid, _)| !linked.contains(pid))
        .filter_map(|(pid, d)| moment_item(v, pid, d.clone()))
        .take(limit)
        .map(Value::Object)
        .collect();
    json!({
        "performed": Value::Array(performed.into_iter().map(Value::Object).collect()),
        "like": {"bits": bits, "moments": moments},
        "vectors": like.vectors,
        "text": head(&like.text, SNIPPET),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(graph: &Value) -> View {
        View::new(graph, Vec::new(), Vec::new(), "owner")
    }

    fn graph() -> Value {
        json!({"nodes": [
            {"id": "joke:a", "type": "joke", "title": "Alpha bit", "summary": "  typed idea about parking  "},
            {"id": "unit:a1", "type": "unit", "text": "Old version about the meter.", "date": "2020-01-01"},
            {"id": "unit:a2", "type": "unit", "text": "Newest version about the meter.", "date": "2021-01-01"},
            {"id": "unit:a3", "type": "unit", "text": "Hidden one.", "date": "2022-01-01", "hidden": true},
            {"id": "joke:b", "type": "joke", "title": "Bravo", "summary": "just typed"},
            {"id": "set:s", "type": "set", "title": "A night", "date": "2023-04-05", "venue": "The Loft"},
            {"id": "perf:p", "type": "performance", "set": "set:s", "start_ms": 61000, "end_ms": 70000, "text": " the meter line "},
            {"id": "perf:q", "type": "performance", "set": "set:s", "start_ms": 1000, "text": "other"}
        ], "edges": [
            {"id": "e:1", "src": "unit:a1", "dst": "joke:a", "kind": "part_of", "status": "confirmed"},
            {"id": "e:2", "src": "unit:a2", "dst": "joke:a", "kind": "part_of", "status": "confirmed"},
            {"id": "e:3", "src": "unit:a3", "dst": "joke:a", "kind": "part_of", "status": "confirmed"},
            {"id": "e:4", "src": "perf:p", "dst": "joke:a", "kind": "performed_as", "status": "proposed"},
            {"id": "e:5", "src": "perf:q", "dst": "joke:b", "kind": "performed_as", "status": "rejected"}
        ]})
    }

    #[test]
    fn close_is_a_word_match_or_a_near_meaning_and_never_the_cap() {
        let v = view(&graph());
        let words = vec!["unit:a2".to_string(), "joke:a".to_string(), "joke:gone".to_string()];
        let near = vec![
            ("joke:b".to_string(), json!(0.31)),
            ("perf:q".to_string(), json!(0.55)),
            ("perf:p".to_string(), json!(0.12)),
            ("unit:a1".to_string(), json!(0.05)),
        ];
        let got = rank_close(words.clone(), near.clone(), 0.4, &v, 60);
        assert_eq!(
            got,
            [
                ("joke:a".to_string(), None),
                ("perf:p".to_string(), Some(0.12)),
                ("joke:b".to_string(), Some(0.31))
            ],
            "a unit folds into its bit once, the far moment and the unknown id go, the words lead"
        );
        assert_eq!(
            rank_close(words.clone(), near.clone(), 0.4, &v, 2).len(),
            2,
            "at most the limit"
        );
        // the red arm: with no closeness rule every vector row counts, which is the old "12 bits" (the cap)
        assert_eq!(rank_close(words, near, f64::INFINITY, &v, 60).len(), 4);
    }

    #[test]
    fn the_batch_is_the_pythons_text() {
        let sql = search_sql(20, false, true);
        assert!(sql.starts_with(
            "LET $ft = SELECT id, search::score(1) AS score FROM unit WHERE text @1@ $q AND hidden != true ORDER BY score DESC LIMIT 20;"
        ));
        assert!(sql.contains("FROM performance WHERE text @1@ $q ORDER BY score DESC LIMIT 20;"));
        assert!(sql.contains("FROM joke WHERE embedding <|20,150|> $qvec AND hidden != true ORDER BY distance;"));
        assert!(sql.ends_with(
            "RETURN search::rrf([$ft, $jt, $js, $vs, $jv], 20, 60);RETURN search::rrf([$pt, $pv], 20, 60);"
        ));
        let words_only = search_sql(60, true, false);
        assert!(!words_only.contains("knn") && !words_only.contains("hidden"));
        assert!(words_only.ends_with("RETURN search::rrf([$ft, $jt, $js], 60, 60);RETURN search::rrf([$pt], 60, 60);"));
        assert_eq!(
            (depth(6, 3), depth(-5, 3), depth(1000, 3), depth(8, 4)),
            (20, 20, 200, 32)
        );
    }

    #[test]
    fn newest_text_and_why_read_the_newest_visible_version() {
        let v = view(&graph());
        assert_eq!(newest_text(&v, "joke:a"), "Newest version about the meter.");
        assert_eq!(newest_text(&v, "joke:b"), "just typed");
        let w = words("  The METER ");
        assert_eq!(w, ["the", "meter"]);
        assert_eq!(
            bit_why(&v, "joke:a", &["parking".into()]).as_deref(),
            Some("typed idea about parking")
        );
        assert_eq!(
            bit_why(&v, "joke:a", &["meter".into()]).as_deref(),
            Some("Newest version about the meter.")
        );
        assert_eq!(
            bit_why(&v, "joke:a", &["hidden".into()]),
            None,
            "a hidden version is never the why"
        );
    }

    #[test]
    fn the_snippet_starts_at_a_word_sixty_characters_before() {
        let text = format!("{} needle and after", "word ".repeat(30));
        let s = snippet(&text, &["needle".into()]).expect("found");
        assert!(s.starts_with('…'));
        let pos = text.find("needle").expect("pos");
        let start = text[..pos - 60].rfind(' ').expect("space") + 1;
        assert_eq!(s, format!("…{}", text[start..].trim()));
        assert_eq!(
            snippet("short needle", &["needle".into()]).as_deref(),
            Some("short needle")
        );
        assert_eq!(snippet("nothing", &["needle".into()]), None);
        assert_eq!(snippet("   ", &["x".into()]), None);
    }

    #[test]
    fn moments_carry_the_bit_they_are_performed_as() {
        let v = view(&graph());
        let m = moment_item(&v, "perf:p", json!(0.5)).expect("moment");
        assert_eq!(m["joke"], "joke:a");
        assert_eq!(m["text"], "the meter line");
        assert_eq!(m["at_label"], "1:01");
        assert_eq!(m["when_label"], "Apr 2023");
        assert_eq!(m["short_name"], short_name_of("Alpha bit", None));
        let q = moment_item(&v, "perf:q", Value::Null).expect("moment");
        assert_eq!(q["joke"], Value::Null, "a rejected edge links nothing");
        assert!(moment_item(&v, "joke:a", Value::Null).is_none());
        let p = performed(&v, "joke:a");
        assert_eq!(p.len(), 1);
        assert_eq!(p[0]["status"], "proposed");
    }

    #[test]
    fn similar_keeps_each_bit_at_its_nearest_and_never_itself() {
        let v = view(&graph());
        let units = vec![json!({"id": "unit:a1", "d": 0.01}), json!({"id": "unit:zz", "d": 0.02})];
        let jokes = vec![
            json!({"id": "joke:b", "d": 0.4}),
            json!({"id": "joke:a", "d": 0.0}),
            json!({"id": "joke:b", "d": 0.3}),
        ];
        let perfs = vec![json!({"id": "perf:p", "d": null}), json!({"id": "perf:q", "d": 0.2})];
        let l = like_vectors(&v, "joke:a", "t".into(), units, jokes, perfs, 8);
        assert_eq!(l.bits, vec![("joke:b".to_string(), json!(0.3))]);
        assert_eq!(l.moments[0], ("perf:p".to_string(), json!(0.0)));
        assert_eq!(first_words(" a b  c d e f g h i j k l m n "), "a b c d e f g h i j k l");
    }
}
