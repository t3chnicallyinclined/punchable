//! The Rant tab (`docs/design/SPEC-2026-11-06-rant-tab.md` §2, §3, §7; slice 1). Rust only (R7: new server work is
//! written once, here; the Python never answers these paths and has no twin of them):
//!
//! - `GET /api/topics`: his topics, newest first (the `topics` creative store, `pk_creative::reducers::topics`).
//! - `POST /api/topics {text}`: a typed topic; the same words twice (case and spacing aside) answer the one he has.
//! - `PATCH /api/topics/<id> {ranted: <joke id>} | {delete: true}`: mark it ranted with the rant it became, or
//!   delete it.
//! - `POST /api/rant/draft {topic, topic_id?, sources: [id], fresh?}`: queue a `rant_draft` job (pk-worker) over his
//!   words for the ticked sources, read here from the KB (a bit's newest version, else its idea; a taped moment's
//!   text), never text the phone sends; the legal rules run on all of it first. 202 with the job, or 200 with a
//!   cached result (`jobs::enqueue`).
//! - `GET /api/rants`: the bits that have a rant cut (a version whose source is `rant` or `rant_*`, `film::cut_of`),
//!   newest rant first, for the tab's Drafts (the phone splits Finished off with its own filmed takes).

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_creative::reducers::topics::{MAX_TOPIC_CHARS, clean_topic, is_topic_id};
use pk_kb::View;
use serde_json::{Map, Value, json};
use sha1::{Digest as _, Sha1};

use super::{Creative, Kb, Pre, Req, Written, actor, detached, http, is_id, refuse_legal, request};
use crate::auth::Caller;
use crate::display::short_name_of;
use crate::error::{ApiResult, json_response, ok};
use crate::query::Query;
use crate::routes::creative::items_of;
use crate::state::AppState;

/// The store.
pub const STORE: &str = "topics";
/// The job kind.
pub const KIND: &str = "rant_draft";
/// Sources one draft may draw on, at most (`pk_worker::kinds::rant_draft::MAX_SOURCES`).
pub const MAX_SOURCES: usize = 12;
/// Topics kept, at most: a list he reads on one screen, not an archive. The cap is the route's, never the reducer's
/// (a lower cap there later would refuse the history already written).
pub const MAX_TOPICS: usize = 200;

fn hex_of(seed: &str, n: usize) -> String {
    hex::encode(Sha1::digest(seed.as_bytes()))[..n].to_string()
}

/// The topics, newest first.
fn topics_newest_first(items: &pk_creative::Items) -> Vec<Value> {
    let mut v: Vec<Value> = items.values().cloned().collect();
    v.reverse();
    v
}

/// `GET /api/topics`.
pub async fn topics(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let items = items_of(&st, |s| &s.topics).await?;
    Ok(ok(&json!({"topics": topics_newest_first(&items)})))
}

/// The same topic, for "the one he has": case and spacing aside.
fn same_topic(a: &str, b: &str) -> bool {
    clean_topic(a).to_lowercase() == clean_topic(b).to_lowercase()
}

/// `POST /api/topics {text, from_trend?}`.
pub async fn add_topic(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let text = clean_topic(b.get("text").and_then(Value::as_str).unwrap_or(""));
    if text.is_empty() {
        return Err(http(400, "text is required (the topic)"));
    }
    if py::len(&text) > MAX_TOPIC_CHARS {
        return Err(http(400, format!("a topic is at most {MAX_TOPIC_CHARS} characters")));
    }
    refuse_legal(&st, [text.as_str()])?;
    let by = actor(&st, &caller, &Map::new())?;
    let out = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let lines = cr.lines(STORE).await?;
        let id = format!(
            "topic:{}",
            hex_of(&format!("{text}|{}|{}", lines.len(), pk_core::time::now_iso()), 12)
        );
        let t = text.clone();
        let have = move |items: &pk_creative::Items| -> ApiResult<Pre> {
            Ok(
                match items
                    .values()
                    .find(|x| x.get("text").and_then(Value::as_str).is_some_and(|s| same_topic(s, &t)))
                {
                    Some(x) => Pre::Done(StatusCode::OK, x.clone()),
                    None if items.len() >= MAX_TOPICS => {
                        return Err(http(400, format!("at most {MAX_TOPICS} topics: delete one first")));
                    }
                    None => Pre::Go,
                },
            )
        };
        let topic = match cr
            .append_if(STORE, "add", &id, json!({"text": text, "from_trend": null}), &by, have)
            .await?
        {
            Written::Landed(v) | Written::Skipped(_, v) => v,
        };
        Ok(json!({"ok": true, "topic": topic}))
    })
    .await?;
    Ok(ok(&out))
}

/// `PATCH /api/topics/<id> {ranted: <joke id>} | {delete: true}`.
pub async fn patch_topic(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(tid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    if !is_topic_id(&tid) {
        return Err(http(400, "bad topic id"));
    }
    let (op, data) = match (b.get("ranted"), b.get("delete")) {
        (Some(Value::String(j)), None) if j.starts_with("joke:") && is_id(j) => ("ranted", json!({"joke": j})),
        (None, Some(Value::Bool(true))) => ("delete", json!({})),
        _ => return Err(http(400, "send {ranted: <the rant's joke id>} or {delete: true}")),
    };
    let by = actor(&st, &caller, &Map::new())?;
    let out = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let id = tid.clone();
        let exists = move |items: &pk_creative::Items| -> ApiResult<Pre> {
            if items.contains_key(&id) {
                Ok(Pre::Go)
            } else {
                Err(http(404, format!("no topic {id}")))
            }
        };
        let topic = match cr.append_if(STORE, op, &tid, data, &by, exists).await? {
            Written::Landed(v) | Written::Skipped(_, v) => v,
        };
        Ok(json!({"ok": true, "id": tid, "topic": if op == "delete" { Value::Null } else { topic }}))
    })
    .await?;
    Ok(ok(&out))
}

/// A bit's versions now (a `part_of` edge that is not rejected, the unit not hidden), oldest first by date, a day's
/// versions in the order they were added (the app's stepper: rules/bitLines).
fn versions_of<'a>(view: &'a View, jid: &str) -> Vec<(&'a str, &'a pk_kb::Rec)> {
    let mut vs: Vec<(&str, &pk_kb::Rec)> = view
        .joke_units
        .get(jid)
        .into_iter()
        .flatten()
        .filter(|(u, _)| view.unit_joke.get(u).is_some_and(|j| j == jid))
        .filter_map(|(u, _)| view.node(u).map(|n| (u.as_str(), n)))
        .filter(|(_, n)| !py::truthy(n.get("hidden")))
        .collect();
    vs.sort_by(|a, b| {
        str_field(a.1, "date")
            .unwrap_or("")
            .cmp(str_field(b.1, "date").unwrap_or(""))
    });
    vs
}

fn str_field<'a>(n: &'a pk_kb::Rec, k: &str) -> Option<&'a str> {
    n.get(k).and_then(Value::as_str)
}

/// One source's words and its label, from the KB: a bit's newest version (else its typed idea), a taped moment's
/// text. `None` for an id that is neither, or one with no words.
fn source_of(view: &View, id: &str) -> Option<Value> {
    match view.ntype(Some(id))? {
        "joke" => {
            let j = view.node(id)?;
            let title = str_field(j, "title").unwrap_or("");
            let short = short_name_of(title, str_field(j, "short_name"));
            // what the bit says now: its newest version (the stepper's order: by date, a day's in the order they were
            // added; `search::newest_text` keeps the FIRST of a day's, so a version saved this morning would hide one
            // saved this afternoon), else the typed idea
            let vs = versions_of(view, id);
            let newest = vs
                .iter()
                .rev()
                .filter_map(|(_, n)| str_field(n, "text"))
                .find(|t| !py::strip(t).is_empty());
            if let Some(text) = newest {
                return Some(json!({"id": id, "kind": "bit", "label": format!("{short} v{}", vs.len()), "text": text}));
            }
            let idea = crate::search::newest_text(view, id);
            (!py::strip(&idea).is_empty()).then(|| json!({"id": id, "kind": "idea", "label": short, "text": idea}))
        }
        "performance" => {
            let p = view.node(id)?;
            str_field(p, "text")
                .filter(|t| !py::strip(t).is_empty())
                .map(|text| json!({"id": id, "kind": "moment", "label": "On tape", "text": text}))
        }
        _ => None,
    }
}

/// `POST /api/rant/draft {topic, topic_id?, sources: [id], fresh?}`.
pub async fn draft(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q, b } = request(st, raw.as_deref(), &headers, body).await?;
    let topic = clean_topic(b.get("topic").and_then(Value::as_str).unwrap_or(""));
    if topic.is_empty() {
        return Err(http(400, "topic is required (what the rant is about)"));
    }
    if py::len(&topic) > MAX_TOPIC_CHARS {
        return Err(http(400, format!("a topic is at most {MAX_TOPIC_CHARS} characters")));
    }
    let topic_id = match b.get("topic_id") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(t)) if is_topic_id(t) => json!(t),
        Some(_) => return Err(http(400, "bad topic id")),
    };
    let bad = || {
        http(
            400,
            format!("sources must be a list of at most {MAX_SOURCES} bit or moment ids"),
        )
    };
    let ids: Vec<String> = match b.get("sources") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(xs)) if xs.len() <= MAX_SOURCES => {
            let mut out: Vec<String> = Vec::with_capacity(xs.len());
            for x in xs {
                match x.as_str().filter(|s| is_id(s)) {
                    Some(s) if !out.iter().any(|o| o == s) => out.push(s.to_string()),
                    Some(_) => {}
                    None => return Err(bad()),
                }
            }
            out
        }
        Some(_) => return Err(bad()),
    };
    let (_, view) = {
        let _kb = st.kb_lock().lock().await;
        Kb::new(st.clone(), q.include_hidden()).view().await?
    };
    let mut sources = Vec::with_capacity(ids.len());
    for id in &ids {
        match source_of(&view, id) {
            Some(s) => sources.push(s),
            None => return Err(http(404, format!("{id} is not a bit or a taped moment with words"))),
        }
    }
    refuse_legal(
        &st,
        std::iter::once(topic.as_str()).chain(sources.iter().filter_map(|s| s["text"].as_str())),
    )?;
    // the same topic over the same words is the same draft, whatever order he ticked them in; a new version of a
    // ticked bit changes its text, so it is a fresh draft
    let mut keyed: Vec<String> = sources.iter().map(|s| format!("{} {}", s["id"], s["text"])).collect();
    keyed.sort_unstable();
    let key = format!(
        "{KIND}|{}",
        hex_of(&format!("{}|{}", topic.to_lowercase(), keyed.join("|")), 16)
    );
    let input = json!({"topic": topic, "topic_id": topic_id, "sources": sources});
    let fresh = py::truthy(b.get("fresh"));
    let owner = st.config().owner_id.clone();
    let (s, out) = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        super::jobs::enqueue(&cr, KIND, &key, input, fresh, &owner).await
    })
    .await?;
    Ok(json_response(s, &out))
}

/// One row of the tab's Drafts: a bit with a rant cut.
fn rant_row(view: &View, jid: &str) -> Option<(String, usize, Value)> {
    let vs = versions_of(view, jid);
    let rants: Vec<(usize, &pk_kb::Rec)> = vs
        .iter()
        .enumerate()
        .filter(|(_, (_, n))| super::film::cut_of(n.get("source")) == "rant")
        .map(|(i, (_, n))| (i, *n))
        .collect();
    let (i, newest) = *rants.last()?;
    let j = view.node(jid)?;
    let title = str_field(j, "title").unwrap_or("");
    let at = str_field(newest, "date").unwrap_or("").to_string();
    let hook = vs.iter().any(|(_, n)| str_field(n, "source") == Some("rant_hook"));
    let row = json!({
        "id": jid, "title": title, "short_name": short_name_of(title, str_field(j, "short_name")),
        "versions": vs.len(), "v": i + 1, "unit": vs[i].0, "at": if at.is_empty() { Value::Null } else { json!(at) },
        "hook": hook, "written": vs.iter().any(|(_, n)| str_field(n, "source") == Some("rant_draft")),
    });
    Some((at, i, row))
}

/// `GET /api/rants`: newest rant first (the newest rant version's day, then the joke's place, newest last).
pub async fn rants(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let view = st.data().await?.view(q.include_hidden()).await?;
    let mut rows: Vec<(String, usize, Value)> = view
        .jokes
        .iter()
        .enumerate()
        .filter_map(|(k, jid)| rant_row(&view, jid).map(|(at, _, row)| (at, k, row)))
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    Ok(ok(&json!({"rants": rows.into_iter().map(|r| r.2).collect::<Vec<_>>()})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_topic_is_case_and_spacing_aside() {
        assert!(same_topic("Self-checkout  machines", " self-checkout machines\n"));
        assert!(!same_topic("self-checkout", "self checkout"));
    }
}
