//! `POST /api/ideas` (`server.py` `idea_result`): a jot. One `add` line for a joke at stage `idea` (its first seven
//! words as the title unless he gave one), then the creative half: the jot is placed under the bit it clearly is
//! (`merge_api.after_idea`, the AI's `attach` in the merges store, undoable), what he kept of a draft teaches the
//! voice profile (`learn_kept`), and an `embed` job is queued for the hosted worker (his typed idea gets its vector
//! from it; the Python's sweeper fills it too while that process runs).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::Extension;
use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::HeaderMap;
use axum::response::Response;
use pk_core::py;
use pk_core::text::head;
use serde_json::{Map, Value, json};

use super::{Creative, Kb, actor, body_json, detached};
use crate::auth::Caller;
use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::snapshot::Data;
use crate::state::AppState;

/// `(b.get(k) or "").strip()`: a string stripped, nothing for anything falsy; a truthy non-string is the
/// `AttributeError` the Python answers 500 with, answered 400 here (documented in the T10 row).
fn text_field<'a>(b: &'a Map<String, Value>, k: &str) -> ApiResult<&'a str> {
    match b.get(k) {
        Some(Value::String(s)) => Ok(py::strip(s)),
        Some(v) if py::truthy(Some(v)) => Err(ApiError::bad_request(format!("{k} must be text"))),
        _ => Ok(""),
    }
}

/// How many generations behind the merge index used to place a jot may be (one per decision; a jot adds no units).
pub const MERGE_INDEX_LAG: i64 = 50;

/// `POST /api/ideas`.
pub async fn ideas(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let b = body_json(&headers, body).await?;
    let by = actor(&st, &caller, &b)?;
    let hidden = q.include_hidden();
    let out = detached(async move { idea_result(&st, hidden, &b, &by).await }).await?;
    Ok(ok(&out))
}

/// The idea flow as a value (`idea_result`): the node add line, the auto-attach, the learned section, the embed job.
pub async fn idea_result(st: &AppState, hidden: bool, b: &Map<String, Value>, by: &str) -> ApiResult<Value> {
    let text = text_field(b, "text")?;
    if text.is_empty() || py::len(text) > 4000 {
        return Err(ApiError::bad_request("an idea needs its text (up to 4000 characters)"));
    }
    let given = text_field(b, "title")?;
    let title = if given.is_empty() {
        let seven: Vec<&str> = py::split(text).take(7).collect();
        py::strip_chars(&seven.join(" "), " ,.;:!?\"'").to_string()
    } else {
        given.to_string()
    };
    let title = head(&title, 120);
    let title = if title.is_empty() { "New line" } else { title.as_str() };
    let frm: Option<&Map<String, Value>> = b.get("from").and_then(Value::as_object).filter(|f| !f.is_empty());
    let raw_reason = match b.get("reason") {
        Some(Value::String(s)) => s.as_str(),
        Some(v) if py::truthy(Some(v)) => return Err(ApiError::bad_request("reason must be text")),
        _ => "",
    };
    let reason = if raw_reason.is_empty() {
        if frm.is_some() { "saved from review" } else { "" }
    } else {
        raw_reason
    };
    let owner = st.config().owner_id.clone();
    // the jot's vector (db/embedder.py: a passage vector) is computed while the decision is built and appended: it needs
    // the text only. None when the model cannot run, then only an unmistakable word match can attach.
    let embed = tokio::spawn(embed_jot(st.clone(), text.to_string()));
    let req = json!({"target": "node", "action": "add",
                     "after": {"type": "joke", "title": title, "stage": "idea", "summary": text}});
    // where the words came from (a taped moment): added to the line after the builder validated the request, the
    // keys that are there, never a null (`d["after"]["from"] = ...`)
    let origin: Option<Map<String, Value>> = frm.map(|f| {
        ["set", "perf", "start_ms", "end_ms", "note"]
            .iter()
            .filter_map(|k| {
                f.get(*k)
                    .filter(|v| !v.is_null())
                    .map(|v| ((*k).to_string(), v.clone()))
            })
            .collect()
    });
    let t_decide = std::time::Instant::now();
    let (d, idx, line) = {
        let _kb = st.kb_lock().lock().await;
        let kb = Kb::new(st.clone(), hidden);
        let (d, _) = kb.view().await?;
        let now = pk_core::time::now_iso();
        let (idx, line) = kb
            .decide_patched(&req, &now, reason, by, |built| {
                if let Some(f) = &origin {
                    built
                        .after
                        .get_or_insert_with(Map::new)
                        .insert("from".into(), Value::Object(f.clone()));
                }
            })
            .await?;
        (d, idx, line)
    };
    tracing::debug!(
        step = "view+decide",
        ms = t_decide.elapsed().as_secs_f64() * 1000.0,
        "jot"
    );
    let id = line.id().unwrap_or("").to_string();
    let mut out = json!({"ok": true, "indexes": [idx], "id": id, "decisions": [Value::Object(line.0)]});
    // a jot that is clearly an existing bit attaches to it (merge_api; undoable)
    let src = if let Some(s) = by.strip_prefix("import:") {
        s.to_string()
    } else if raw_reason.starts_with("[gdocs]") {
        "gdocs".to_string()
    } else {
        "app".to_string()
    };
    let cr = Creative::new(st.clone());
    let vec = embed.await.ok().flatten();
    let t_place = std::time::Instant::now();
    let attach = after_idea(st, &cr, &d, &id, text, &src, vec).await;
    tracing::debug!(
        step = "match+attach",
        ms = t_place.elapsed().as_secs_f64() * 1000.0,
        "jot"
    );
    let into = attach
        .as_object()
        .and_then(|a| a.get("into"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    out["attach"] = attach;
    if by == owner {
        // a riff draft he took as his own ("Use as mine": `draft: true`, or the text is a finished riff job's draft
        // word for word): the voice profile's learned section remembers it and the drafts he passed over beside it;
        // his own typed line is no draft and writes nothing there
        let _cr = st.creative_lock().lock().await;
        let draft = b.get("draft") == Some(&Value::Bool(true));
        let bit = into.as_deref().unwrap_or(&id);
        out["learned"] = super::learned::learn_kept(&cr, text, Some(bit), by, draft)
            .await
            .unwrap_or(Value::Null);
        // hosted: no model in this process, so his typed idea gets its vector from the worker (job `embed`, one
        // pending at a time); the idea is saved either way
        let t_q = std::time::Instant::now();
        if let Err(e) = super::jobs::enqueue_followup(&cr, "embed", &owner).await {
            tracing::warn!("embed job not queued: {e}");
        }
        tracing::debug!(step = "enqueue", ms = t_q.elapsed().as_secs_f64() * 1000.0, "jot");
    }
    Ok(out)
}

/// The jot's passage vector, or none when the model cannot run (logged).
async fn embed_jot(st: AppState, text: String) -> Option<Vec<f32>> {
    let t = std::time::Instant::now();
    let e = st.model().get().await?;
    let v = match st.model().vector(e, text, false).await {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("no vector for the jot ({e}); words only");
            None
        }
    };
    tracing::debug!(step = "embed", ms = t.elapsed().as_secs_f64() * 1000.0, "jot");
    v
}

/// `merge_api.after_idea`: place the new idea when it is clearly one existing bit (`merge.match_text` over the
/// merge index last built, "the corpus as it was a moment ago": the new idea has no units yet, so a notebook a
/// generation old places it the same; with the jot's own vector when the model is here). Never fails: a failure
/// leaves a new idea, with the error in the answer.
pub async fn after_idea(
    st: &AppState,
    cr: &Creative,
    d: &Arc<Data>,
    jid: &str,
    text: &str,
    src: &str,
    vec: Option<Vec<f32>>,
) -> Value {
    match place(st, cr, d, jid, text, src, vec).await {
        Ok(v) => v,
        Err(e) => json!({"decision": "new", "error": e.msg}),
    }
}

async fn place(
    st: &AppState,
    cr: &Creative,
    d: &Arc<Data>,
    jid: &str,
    text: &str,
    src: &str,
    vec: Option<Vec<f32>>,
) -> ApiResult<Value> {
    let ix = if let Some(ix) = st.last_merge_index(d.state.generation, MERGE_INDEX_LAG) {
        ix
    } else {
        let ix = d.merge_index(st).await?;
        st.set_last_merge_index(d.state.generation, &ix);
        ix
    };
    let exclude: HashSet<String> = HashSet::from([jid.to_string()]);
    let owned = text.to_string();
    // pair scoring is CPU work: one build at a time across the service (the pod has one CPU)
    let permit = st
        .cpu()
        .acquire_owned()
        .await
        .map_err(|e| ApiError::internal("AcquireError", e))?;
    let r = tokio::task::spawn_blocking(move || {
        let r = pk_creative::merge::match_text(&ix, &owned, vec.as_deref(), &exclude, &HashMap::new());
        drop(permit);
        r
    })
    .await
    .map_err(|e| ApiError::internal("JoinError", e))?;
    if r["decision"] != "attach" {
        return Ok(json!({"decision": r["decision"], "reasons": r["reasons"]}));
    }
    let into = r["joke_id"].as_str().unwrap_or("").to_string();
    let aid = format!("at:{}", pk_creative::merge::sha8(&[jid.to_string(), into.clone()]));
    let rec = {
        let _cr = st.creative_lock().lock().await;
        cr.append(
            "merges",
            "attach",
            &aid,
            json!({"joke": jid, "into": into, "text": text, "date": head(&pk_core::time::now_iso(), 10),
                   "score": r.get("score").cloned().unwrap_or(Value::Null), "reasons": r["reasons"], "source": src}),
            "ai:attach",
        )
        .await?
    };
    Ok(
        json!({"decision": "attach", "id": aid, "into": into, "title": r["title"], "reasons": r["reasons"],
              "undo": {"POST": "/api/merge/undo", "merge_id": aid}, "record": super::merge::brief(&rec)}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_is_the_first_seven_words_without_punctuation() {
        let seven: Vec<&str> = py::split("  one, two three four five six seven eight nine ")
            .take(7)
            .collect();
        assert_eq!(
            py::strip_chars(&seven.join(" "), " ,.;:!?\"'"),
            "one, two three four five six seven"
        );
        let short: Vec<&str> = py::split("\"Why?\"").take(7).collect();
        assert_eq!(py::strip_chars(&short.join(" "), " ,.;:!?\"'"), "Why");
        assert_eq!(head(&"x".repeat(130), 120).len(), 120);
    }

    #[test]
    fn text_fields_strip_and_refuse_a_number() {
        let b = json!({"text": "  hi ", "title": 5, "from": {}})
            .as_object()
            .cloned()
            .unwrap();
        assert_eq!(text_field(&b, "text").unwrap(), "hi");
        assert_eq!(text_field(&b, "reason").unwrap(), "");
        assert!(text_field(&b, "title").is_err());
        assert!(
            b.get("from")
                .and_then(Value::as_object)
                .is_none_or(serde_json::Map::is_empty)
        );
    }
}
