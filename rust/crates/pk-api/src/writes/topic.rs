//! The topic as a page (`docs/design/SPEC-2026-11-06-rant-tab.md` §12.3, §12.7; Rust only, R7): his own words on
//! screen. `GET /api/topics/<id>` is the rant so far, what he kept, what the search found and what he dropped;
//! `GET /api/topics/<id>/material` is the page's search and Find more; `GET /api/topics` (in [`super::rant`]) carries
//! the same counts.
//!
//! **Found is a real closeness.** `GET /api/search` sends no distance on its bit rows and a vector search always
//! returns its limit, so the old "12 bits" was the cap, not a count. Found reads the database's own word matches and
//! vector distances instead ([`crate::search::close_hits`]): a hit counts when the words matched, or the meaning is
//! under [`MATCH_CLOSE`]. One per-topic list ([`TopicFound`]) feeds the tab's row and the page's heading through one
//! function ([`found_rows`]), so the row's number and the heading's number are one number.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, RawQuery, State};
use axum::response::Response;
use pk_core::py;
use pk_creative::Items;
use pk_creative::reducers::topics::{is_deleted, rants_of};
use pk_kb::View;
use serde_json::{Map, Value, json};
use tokio::sync::Semaphore;

use super::film::{cut_lines, cut_of};
use super::rant::{str_field, versions_of};
use crate::display::{short_name_of, venue_label};
use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::routes::creative::items_of;
use crate::state::AppState;

/// A hit closer than this counts as being about the topic (`rules/rantRules` `MATCH_CLOSE`, the phone's "Close").
pub const MATCH_CLOSE: f64 = 0.4;
/// Rows one page of Found shows; more than this reads "30+" (§12.11 C17).
pub const FOUND_CAP: usize = 30;
/// Hits kept per topic: enough that keeping one of thirty brings the next one up, and for Find more's next page.
pub const FOUND_DEPTH: usize = 60;
/// A topic's list older than this is computed again in the background the next time the tab reads it.
const FOUND_STALE: Duration = Duration::from_secs(600);
/// Computations running at once for the tab's rows (§12.7: three at a time).
const FOUND_AT_ONCE: usize = 3;
/// Kept bits Find more asks `/similar` from, at most.
const SIMILAR_FROM: usize = 3;
/// Neighbours asked per bit.
const SIMILAR_LIMIT: i64 = 8;

/// A topic's close hits: `(joke or perf id, distance)`, `None` = the words matched; closest first.
pub type Hits = Arc<Vec<(String, Option<f64>)>>;

/// One topic's close hits as last computed.
#[derive(Debug)]
struct Entry {
    text: String,
    at: Instant,
    hits: Hits,
}

/// The per-topic lists (one per topic id). Not a source of truth: a miss computes, a restart forgets.
#[derive(Debug)]
pub struct TopicFound {
    map: Mutex<HashMap<String, Entry>>,
    running: Mutex<HashSet<String>>,
    permits: Arc<Semaphore>,
}

impl Default for TopicFound {
    fn default() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            running: Mutex::new(HashSet::new()),
            permits: Arc::new(Semaphore::new(FOUND_AT_ONCE)),
        }
    }
}

impl TopicFound {
    fn store(&self, id: &str, text: &str, hits: Hits) {
        if let Ok(mut m) = self.map.lock() {
            m.insert(
                id.to_string(),
                Entry {
                    text: text.to_string(),
                    at: Instant::now(),
                    hits,
                },
            );
        }
    }

    /// The list for these words, and whether it is stale; `None` when there is none (or the topic was renamed).
    fn get(&self, id: &str, text: &str) -> Option<(Hits, bool)> {
        let m = self.map.lock().ok()?;
        let e = m.get(id).filter(|e| e.text == text)?;
        Some((e.hits.clone(), e.at.elapsed() > FOUND_STALE))
    }
}

/// The close hits for `q` now, read from the database.
async fn compute(st: &AppState, view: &View, q: &str, hidden: bool) -> Result<Vec<(String, Option<f64>)>, ApiError> {
    let store = st
        .store_handle()
        .ok_or_else(|| ApiError::unavailable(crate::state::NOT_LOADED))?;
    let qvec = crate::routes::search::query_vector(st, q).await;
    crate::search::close_hits(&store, qvec.as_deref(), q, FOUND_DEPTH, MATCH_CLOSE, view, hidden)
        .await
        .map_err(|e| ApiError::internal("StoreError", e))
}

/// A page's read: computed now, and what the tab's row reads from then on.
async fn found_now(st: &AppState, view: &View, id: &str, text: &str, hidden: bool) -> Result<Hits, ApiError> {
    let hits = Arc::new(compute(st, view, text, hidden).await?);
    // the tab reads without hidden bits: a read that includes them is never what its row counts
    if !hidden {
        st.topic_found().store(id, text, hits.clone());
    }
    Ok(hits)
}

/// Takes a topic off the running list when its background task ends, however it ends (a panic included).
struct Running(AppState, String);

impl Drop for Running {
    fn drop(&mut self) {
        if let Ok(mut r) = self.0.topic_found().running.lock() {
            r.remove(&self.1);
        }
    }
}

/// The tab's read: what the list says now, computed in the background when it is missing or stale (three at a time).
fn found_cached(st: &AppState, view: &Arc<View>, id: &str, text: &str) -> Option<Hits> {
    let cached = st.topic_found().get(id, text);
    let (hits, stale) = match cached {
        Some((h, s)) => (Some(h), s),
        None => (None, true),
    };
    if stale {
        let fresh = st
            .topic_found()
            .running
            .lock()
            .is_ok_and(|mut r| r.insert(id.to_string()));
        if fresh {
            let (st, view, id, text) = (st.clone(), view.clone(), id.to_string(), text.to_string());
            let permits = st.topic_found().permits.clone();
            tokio::spawn(async move {
                let _running = Running(st.clone(), id.clone());
                let _permit = permits.acquire_owned().await;
                if st.draining() {
                    return;
                }
                match compute(&st, &view, &text, false).await {
                    Ok(h) => st.topic_found().store(&id, &text, Arc::new(h)),
                    Err(e) => tracing::warn!(error = ?e, "a topic's found list did not compute"),
                }
            });
        }
    }
    hits
}

/// One piece of material as the page shows it (§12.7 `Mat`), or `None` for an id with no words now (merged away,
/// hidden). The words are the newest version's, whole; a typed idea with no version reads as a jot.
pub fn mat_of(view: &View, r: &str, distance: Option<f64>) -> Option<Map<String, Value>> {
    let mut m = Map::new();
    m.insert("ref".into(), json!(r));
    match view.ntype(Some(r))? {
        "joke" => {
            let j = view.node(r)?;
            let short = short_name_of(str_field(j, "title").unwrap_or(""), str_field(j, "short_name"));
            let vs = versions_of(view, r);
            let newest = vs
                .iter()
                .rev()
                .find(|(_, n)| str_field(n, "text").is_some_and(|t| !py::strip(t).is_empty()));
            if let Some((_, n)) = newest {
                m.insert("kind".into(), json!("bit"));
                m.insert("label".into(), json!(format!("{short} v{}", vs.len())));
                m.insert("v".into(), json!(vs.len()));
                m.insert("at".into(), n.get("date").cloned().unwrap_or(Value::Null));
                m.insert("words".into(), json!(str_field(n, "text").unwrap_or("")));
                if let Some(p) = n.get("punch").filter(|p| p.is_array()) {
                    m.insert("punch".into(), p.clone());
                }
            } else {
                let idea = crate::search::newest_text(view, r);
                if py::strip(&idea).is_empty() {
                    return None;
                }
                m.insert("kind".into(), json!("jot"));
                m.insert("label".into(), json!(short));
                m.insert("v".into(), Value::Null);
                m.insert(
                    "at".into(),
                    j.get("date")
                        .or_else(|| j.get("created"))
                        .cloned()
                        .unwrap_or(Value::Null),
                );
                m.insert("words".into(), json!(idea));
            }
            m.insert("short_name".into(), json!(short));
        }
        "performance" => {
            let p = view.node(r)?;
            let words = str_field(p, "text").filter(|t| !py::strip(t).is_empty())?;
            let set = str_field(p, "set");
            let s = set.and_then(|sid| view.node(sid));
            m.insert("kind".into(), json!("moment"));
            m.insert("label".into(), json!("On tape"));
            m.insert("short_name".into(), Value::Null);
            m.insert("v".into(), Value::Null);
            let date = s
                .and_then(|s| {
                    s.get("show_date")
                        .filter(|d| py::truthy(Some(d)))
                        .or_else(|| s.get("date"))
                })
                .cloned()
                .unwrap_or(Value::Null);
            m.insert("at".into(), date);
            m.insert("words".into(), json!(words));
            m.insert(
                "venue".into(),
                json!(venue_label(s.and_then(|s| str_field(s, "venue"))).0),
            );
            m.insert("set".into(), set.map_or(Value::Null, |x| json!(x)));
            m.insert("start_ms".into(), p.get("start_ms").cloned().unwrap_or(Value::Null));
        }
        _ => return None,
    }
    m.insert("distance".into(), distance.map_or(Value::Null, |d| json!(d)));
    Some(m)
}

/// A topic's material row (`material[ref]`).
fn material(t: &Value) -> Option<&Map<String, Value>> {
    t.get("material").and_then(Value::as_object)
}

/// The Found rows for a topic: the close hits minus its rants, its kept and its dropped, those with words now.
/// **Both numbers go through here.** `(rows, more)`: at most [`FOUND_CAP`], `more` when there are others.
pub fn found_rows(view: &View, t: &Value, hits: &[(String, Option<f64>)]) -> (Vec<Map<String, Value>>, bool) {
    let all = found_all(view, t, hits);
    let more = all.len() > FOUND_CAP;
    (all.into_iter().take(FOUND_CAP).collect(), more)
}

/// [`found_rows`] uncut, for Find more's next page.
fn found_all(view: &View, t: &Value, hits: &[(String, Option<f64>)]) -> Vec<Map<String, Value>> {
    let rants = rants_of(t);
    let marked = material(t);
    hits.iter()
        .filter(|(r, _)| !rants.contains(&r.as_str()) && !marked.is_some_and(|m| m.contains_key(r)))
        .filter_map(|(r, d)| mat_of(view, r, *d))
        .collect()
}

/// The refs in `state`, jots first, each group newest first (a jot "lands first under Kept", §12.3).
fn marked_rows(view: &View, t: &Value, state: &str) -> Vec<Map<String, Value>> {
    let Some(m) = material(t) else { return Vec::new() };
    let mut rows: Vec<(bool, &str, Map<String, Value>)> = m
        .iter()
        .filter(|(_, x)| x.get("state").and_then(Value::as_str) == Some(state))
        .filter_map(|(r, x)| {
            let jot = x.get("jot") == Some(&Value::Bool(true));
            let mut row = mat_of(view, r, None)?;
            if jot {
                row.insert("kind".into(), json!("jot"));
            }
            Some((jot, x.get("at").and_then(Value::as_str).unwrap_or(""), row))
        })
        .collect();
    // stable: on the same stamp the later keep stays first (the reversal below)
    rows.reverse();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(a.1)));
    rows.into_iter().map(|(_, _, row)| row).collect()
}

/// What a draft on this topic draws on when he named nothing (§12.3 C4): Kept, else the Found he has not dropped.
pub async fn default_sources(st: &AppState, view: &View, t: &Value, id: &str, hidden: bool) -> ApiResult<Vec<String>> {
    let kept = marked_rows(view, t, "kept");
    let rows = if kept.is_empty() {
        let text = t.get("text").and_then(Value::as_str).unwrap_or("");
        let hits = found_now(st, view, id, text, hidden).await?;
        found_rows(view, t, &hits).0
    } else {
        kept
    };
    Ok(rows
        .iter()
        .filter_map(|m| m.get("ref").and_then(Value::as_str).map(str::to_string))
        .collect())
}

/// One rant as the script reads it: the newest rant cut of the bit, its lines word for word, each with where it came
/// from when the `script` store's row for that version still lines up (a row that does not line up shows no marks,
/// never a wrong one).
pub fn rant_of(view: &View, jid: &str, script: &Items) -> Option<Value> {
    let vs = versions_of(view, jid);
    let (i, (unit, n)) = vs
        .iter()
        .enumerate()
        .rev()
        .find(|(_, (_, n))| cut_of(n.get("source")) == "rant")?;
    let j = view.node(jid)?;
    let title = str_field(j, "title").unwrap_or("");
    let lines = cut_lines(str_field(n, "text").unwrap_or(""));
    let marks = script
        .get(*unit)
        .and_then(|r| r.get("lines"))
        .and_then(Value::as_array)
        .filter(|m| m.len() == lines.len());
    let rows: Vec<Value> = lines
        .iter()
        .enumerate()
        .map(|(k, text)| {
            let mut row = marks
                .and_then(|m| m.get(k))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            row.insert("text".into(), json!(text));
            Value::Object(row)
        })
        .collect();
    let words: usize = lines.iter().map(|l| py::split(l).count()).sum();
    Some(json!({
        "id": jid, "title": title, "short_name": short_name_of(title, str_field(j, "short_name")),
        "v": i + 1, "versions": vs.len(), "at": n.get("date").cloned().unwrap_or(Value::Null),
        "unit": unit, "lines": rows, "words": words,
    }))
}

/// A topic's rants, newest first, those that still have a rant cut.
fn rants_now(view: &View, t: &Value, script: &Items) -> Vec<Value> {
    rants_of(t)
        .into_iter()
        .rev()
        .filter_map(|jid| rant_of(view, jid, script))
        .collect()
}

/// One row of `GET /api/topics`.
pub fn topic_row(st: &AppState, view: &Arc<View>, t: &Value, script: &Items) -> Value {
    let id = t.get("id").and_then(Value::as_str).unwrap_or("");
    let text = t.get("text").and_then(Value::as_str).unwrap_or("");
    let rants = rants_now(view, t, script);
    let kept = marked_rows(view, t, "kept");
    let jots = kept
        .iter()
        .filter(|m| m.get("kind").and_then(Value::as_str) == Some("jot"))
        .count();
    let found = found_cached(st, view, id, text).map(|hits| found_rows(view, t, &hits));
    let mut row = t.as_object().cloned().unwrap_or_default();
    row.remove("material");
    row.insert("kept".into(), json!(kept.len() - jots));
    row.insert("jots".into(), json!(jots));
    row.insert("dropped".into(), json!(marked_rows(view, t, "dropped").len()));
    row.insert(
        "found".into(),
        found.as_ref().map_or(Value::Null, |(r, _)| json!(r.len())),
    );
    row.insert(
        "found_more".into(),
        json!(found.as_ref().is_some_and(|(_, more)| *more)),
    );
    row.insert(
        "touched".into(),
        t.get("touched")
            .or_else(|| t.get("created"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    row.insert(
        "rants".into(),
        json!(rants.iter().filter_map(|r| r["id"].as_str()).collect::<Vec<_>>()),
    );
    let newest = rants.first();
    let last: Vec<&Value> = newest
        .and_then(|r| r["lines"].as_array())
        .map(|l| l.iter().rev().take(2).rev().filter_map(|x| x.get("text")).collect())
        .unwrap_or_default();
    row.insert("last_lines".into(), json!(last));
    for k in ["v", "at", "words"] {
        row.insert(k.into(), newest.map_or(Value::Null, |r| r[k].clone()));
    }
    row.insert("rant_id".into(), newest.map_or(Value::Null, |r| r["id"].clone()));
    Value::Object(row)
}

/// The live topic with that id, or the 404.
fn live_topic(items: &Items, id: &str) -> Result<Value, ApiError> {
    items
        .get(id)
        .filter(|t| !is_deleted(t))
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("no topic {id}")))
}

/// `GET /api/topics/<id>`.
pub async fn page(State(st): State<AppState>, Path(id): Path<String>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_d, view) = crate::routes::prepare_view(&st, &q).await?;
    let items = items_of(&st, |s| &s.topics).await?;
    let script = items_of(&st, |s| &s.script).await?;
    let t = live_topic(&items, &id)?;
    let text = t.get("text").and_then(Value::as_str).unwrap_or("").to_string();
    let hits = found_now(&st, &view, &id, &text, q.include_hidden()).await?;
    let (found, found_more) = found_rows(&view, &t, &hits);
    let mut topic = t.as_object().cloned().unwrap_or_default();
    topic.remove("material");
    Ok(ok(&json!({
        "topic": topic,
        "rants": rants_now(&view, &t, &script),
        "kept": marked_rows(&view, &t, "kept"),
        "found": found,
        "found_more": found_more,
        "dropped": marked_rows(&view, &t, "dropped"),
    })))
}

/// `GET /api/topics/<id>/material?q=&offset=`: `q` = the page's search, close hits with their state on this topic;
/// no `q` = Find more: the next page of Found, then what sounds like the bits he kept (else the first found).
pub async fn material_route(
    State(st): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_d, view) = crate::routes::prepare_view(&st, &q).await?;
    let items = items_of(&st, |s| &s.topics).await?;
    let t = live_topic(&items, &id)?;
    let hidden = q.include_hidden();
    let offset = usize::try_from(q.int("offset", "0")?.max(0)).unwrap_or(0);
    let words = py::strip(q.get("q").unwrap_or("")).to_string();
    let state_of = |r: &str| {
        material(&t)
            .and_then(|m| m.get(r))
            .and_then(|x| x.get("state"))
            .cloned()
            .unwrap_or(Value::Null)
    };
    let with_state = |mut m: Map<String, Value>| {
        let s = m.get("ref").and_then(Value::as_str).map_or(Value::Null, state_of);
        m.insert("state".into(), s);
        m
    };
    if !words.is_empty() {
        let hits = compute(&st, &view, &words, hidden).await?;
        let all: Vec<_> = hits.iter().filter_map(|(r, d)| mat_of(&view, r, *d)).collect();
        let more = all.len() > offset + FOUND_CAP;
        let rows: Vec<_> = all.into_iter().skip(offset).take(FOUND_CAP).map(with_state).collect();
        return Ok(ok(&json!({"q": words, "rows": rows, "more": more})));
    }
    let text = t.get("text").and_then(Value::as_str).unwrap_or("").to_string();
    let hits = found_now(&st, &view, &id, &text, hidden).await?;
    let all = found_all(&view, &t, &hits);
    let more = all.len() > offset + FOUND_CAP;
    let mut rows: Vec<_> = all
        .iter()
        .skip(offset)
        .take(FOUND_CAP)
        .cloned()
        .map(with_state)
        .collect();
    if !more {
        // past the close ones: what sounds like what he kept, neither repeated nor already marked
        let mut seen: HashSet<String> = all
            .iter()
            .filter_map(|m| m.get("ref").and_then(Value::as_str).map(str::to_string))
            .collect();
        let rants = rants_of(&t);
        let kept = marked_rows(&view, &t, "kept");
        let from = if kept.is_empty() {
            all.as_slice()
        } else {
            kept.as_slice()
        };
        let bits = from
            .iter()
            .filter(|m| m.get("kind").and_then(Value::as_str) == Some("bit"))
            .filter_map(|m| m.get("ref").and_then(Value::as_str))
            .take(SIMILAR_FROM)
            .collect::<Vec<_>>();
        for jid in bits {
            let like = crate::routes::search::like_of(&st, &view, jid, SIMILAR_LIMIT, hidden).await?;
            for (r, d) in like.bits.iter().chain(like.moments.iter()) {
                if rants.contains(&r.as_str())
                    || material(&t).is_some_and(|m| m.contains_key(r))
                    || !seen.insert(r.clone())
                {
                    continue;
                }
                if let Some(m) = mat_of(&view, r, d.as_f64()) {
                    rows.push(with_state(m));
                }
            }
        }
    }
    Ok(ok(&json!({"q": Value::Null, "rows": rows, "more": more})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_answers_only_for_the_same_words() {
        let c = TopicFound::default();
        c.store("topic:a", "x", Arc::new(vec![("joke:1".to_string(), None)]));
        assert_eq!(c.get("topic:a", "x").map(|(h, s)| (h.len(), s)), Some((1, false)));
        assert!(c.get("topic:a", "renamed").is_none(), "a renamed topic is a miss");
        assert!(c.get("topic:b", "x").is_none());
    }
}
