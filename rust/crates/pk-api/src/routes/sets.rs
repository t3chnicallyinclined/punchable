//! The taped sets and one bit's page (`server.py` `do_GET`): `/api/sets`, `/api/sets/<id>/analysis`,
//! `/api/sets/<id>/review`, `/api/jokes/<id>`. `/api/sets/<id>/clips`, `/api/jokes/<id>/connections` and
//! `/api/jokes/<id>/map` are the creative layer's and `/api/jokes/<id>/similar` needs the model: their paths have
//! two segments after the id, so they never match here and go to the Python.

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

use crate::error::{ApiError, ApiResult, json_bytes, ok};
use crate::pages::{compute, joke_set_dirs, set_dirs_of};
use crate::query::Query;
use crate::snapshot::Data;
use crate::state::AppState;
use crate::tape::Transcripts;

/// The transcripts of `dirs` for this request (from the shared cache, fetching only what it lacks).
pub async fn transcripts(st: &AppState, d: &Data, dirs: Vec<String>) -> Result<Transcripts, ApiError> {
    if dirs.is_empty() {
        return Ok(Transcripts::new());
    }
    let store = st
        .store_handle()
        .ok_or_else(|| ApiError::unavailable("the database is not connected"))?;
    st.transcripts()
        .get(&store, &d.tape.transcripts, dirs)
        .await
        .map_err(|e| ApiError::internal("StoreError", e))
}

fn is_set(d: &crate::index::AppIndex, sid: &str) -> bool {
    d.view
        .nodes
        .get(sid)
        .is_some_and(|s| s.get("type").and_then(serde_json::Value::as_str) == Some("set"))
}

/// `GET /api/sets`: every set's card, newest first.
pub async fn sets(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let d = super::prepare_rows(&st, &q).await?;
    let body = d.sets_body(q.include_hidden()).await?;
    Ok(json_bytes(StatusCode::OK, body))
}

/// `GET /api/sets/<id>/analysis`.
pub async fn analysis(
    State(st): State<AppState>,
    Path(sid): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (d, idx) = super::prepare(&st, &q).await?;
    if !is_set(&idx, &sid) {
        return Err(ApiError::not_found("no such set"));
    }
    let body = compute(idx, d, Transcripts::new(), move |c| c.set_analysis(&sid)).await?;
    // `set_analysis` is `None` only for a missing set, refused above
    body.map(|b| ok(&b)).ok_or_else(|| ApiError::not_found("no such set"))
}

/// `GET /api/sets/<id>/review`: the card, the steps to rate, and the recording (never on the hosted pod).
pub async fn review(
    State(st): State<AppState>,
    Path(sid): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (d, idx) = super::prepare(&st, &q).await?;
    if !is_set(&idx, &sid) {
        return Err(ApiError::not_found("no such set"));
    }
    let tx = transcripts(&st, &d, set_dirs_of(&sid)).await?;
    let body = compute(
        idx,
        d,
        tx,
        move |c| json!({"set": c.set_card(&sid), "steps": c.review_steps(&sid), "audio": null}),
    )
    .await?;
    Ok(ok(&body))
}

/// `GET /api/jokes/<id>`: one bit's page.
pub async fn joke(State(st): State<AppState>, Path(jid): Path<String>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (d, idx) = super::prepare(&st, &q).await?;
    let tx = transcripts(&st, &d, joke_set_dirs(&idx.view, &jid)).await?;
    let body = compute(idx, d, tx, move |c| c.joke_page(&jid)).await?;
    body.map(|b| ok(&b)).ok_or_else(|| ApiError::not_found("no such joke"))
}
