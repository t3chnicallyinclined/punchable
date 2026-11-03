//! `GET /api/merge/candidates` (`merge_api.handle`): every same-idea group with the counts; `?joke=<id>` one joke's
//! group, its closest other jokes, what it is attached to and its history; `?jokes=a,b,...` the inbox's one call
//! for several cards. The judge's answers are the `srcfile` row `kb/creative/merge_judge.json`, read by key like
//! every file; the groups are memoised on the generation ([`crate::snapshot::Data::merge_groups`]).

use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use pk_creative::merge::{MergeState, candidates_batch, candidates_for, history};
use serde_json::{Value, json};

use crate::error::{ApiError, ApiResult, json_bytes, ok};
use crate::query::Query;
use crate::state::AppState;
use crate::writes::{Creative, http, is_id};

/// `MAX_BATCH`: how many cards the inbox may ask about at once.
pub const MAX_BATCH: usize = 50;

/// The whole-groups answer: the memoised groups and the stamp the Python adds (`generated_at`).
#[derive(serde::Serialize)]
struct Stamped<'a> {
    #[serde(flatten)]
    groups: &'a Value,
    generated_at: String,
}

/// `GET /api/merge/candidates[?joke=<id>|?jokes=a,b]`.
pub async fn candidates(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    // the Python: `cr.corpus(kb)` (the 503 while booting, then `?backend=`) before the arguments; here the
    // arguments are judged before the groups are built, which costs nothing when they are good and spares a build
    // when they are not
    let d = super::prepare_rows(&st, &q).await?;
    let many: Vec<String> = q
        .all("jokes")
        .iter()
        .flat_map(|v| v.split(','))
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .collect();
    if !many.is_empty() && (many.len() > MAX_BATCH || !many.iter().all(|x| is_id(x))) {
        return Err(http(400, format!("jokes must be 1-{MAX_BATCH} joke ids")));
    }
    let joke = q.get("joke").filter(|j| !j.is_empty()).map(str::to_string);
    if many.is_empty()
        && let Some(j) = &joke
        && !is_id(j)
    {
        return Err(http(400, "bad joke id"));
    }
    let ix = d.merge_index(&st).await?;
    let cr = Creative::new(st.clone());
    let (lines, items) = cr.lines_and_items("merges").await?;
    let groups = d.merge_groups(&st, lines.len(), &items).await?;
    if !many.is_empty() {
        let state = MergeState::from_items(&items);
        let judge = Arc::clone(&groups.judge);
        let g = Arc::clone(&groups);
        let body = blocking(
            &st,
            move || json!({"items": candidates_batch(&ix, &many, &judge, &state, g.list())}),
        )
        .await?;
        return Ok(ok(&body));
    }
    if let Some(jid) = joke {
        let state = MergeState::from_items(&items);
        let judge = Arc::clone(&groups.judge);
        let g = Arc::clone(&groups);
        let items = Arc::clone(&items);
        let body = blocking(&st, move || {
            let mut out = candidates_for(&ix, &jid, &judge, &state, 5, g.list());
            out["attached_to"] = state.attached.get(&jid).cloned().unwrap_or(Value::Null);
            out["history"] = history(&items, &jid);
            out
        })
        .await?;
        return Ok(ok(&body));
    }
    // the whole answer, stamped: serialised from the memoised groups without a copy of them
    let bytes = serde_json::to_vec(&Stamped {
        groups: &groups.groups,
        generated_at: pk_core::time::now_iso(),
    })
    .map_err(|e| ApiError::internal("TypeError", e))?;
    Ok(json_bytes(StatusCode::OK, bytes.into()))
}

/// Pair scoring is CPU work: off the async threads, one build at a time across the service (the CPU permit).
async fn blocking<T: Send + 'static>(st: &AppState, f: impl FnOnce() -> T + Send + 'static) -> Result<T, ApiError> {
    let permit = st
        .cpu()
        .acquire_owned()
        .await
        .map_err(|e| ApiError::internal("AcquireError", e))?;
    tokio::task::spawn_blocking(move || {
        let out = f();
        drop(permit);
        out
    })
    .await
    .map_err(|e| ApiError::internal("JoinError", e))
}
