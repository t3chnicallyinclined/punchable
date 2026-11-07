//! `GET /api/search` and `GET /api/jokes/<id>/similar` (`server.py` `do_GET` over `api-dev/search_api.py`): the
//! fused search over bits and taped moments, and where a bit was done and what sounds like it.
//!
//! The work that reads the database rather than the generation runs while the generation is made ready: the query
//! vector and the fused search start with the data read (a request right after a write waits for that write's
//! generation, and the search does not need it); `/similar` needs only the view for the bit's text, so its vector
//! and its neighbours are fetched while the index (the Bits-list rows the answer carries) is built. The answers and
//! the order of the refusals are the Python's: the data (503) and `?backend=` first, then the request's own checks.

use std::sync::Arc;

use axum::extract::{Path, RawQuery, State};
use axum::response::Response;
use pk_core::py::strip;

use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::search::{
    self, Like, MAX_LIMIT, Raw, first_words, like_vectors, like_words, newest_text, performed, search_all,
    search_payload, similar_payload,
};
use crate::state::AppState;

/// The query's vector, or none (no model, or the model refused it: `_qvec` logs and searches on words).
pub async fn query_vector(st: &AppState, q: &str) -> Option<Vec<f32>> {
    let e = st.model().get().await?;
    match st.model().vector(e, q.to_string(), true).await {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(error = %e, "search: no query vector; words only");
            None
        }
    }
}

fn store(st: &AppState) -> Result<Arc<pk_store::Store>, ApiError> {
    st.store_handle()
        .ok_or_else(|| ApiError::unavailable(crate::state::NOT_LOADED))
}

/// `search_all` for `q` with its query vector (`kb.search_all`).
pub async fn fused(st: &AppState, q: &str, limit: i64, include_hidden: bool) -> Result<Raw, ApiError> {
    let qvec = query_vector(st, q).await;
    search_all(&*store(st)?, qvec.as_deref(), q, limit, include_hidden)
        .await
        .map_err(|e| ApiError::internal("StoreError", e))
}

/// `GET /api/search?q=<words>[&limit=20]`.
pub async fn search(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let qs = strip(q.get("q").unwrap_or("")).to_string();
    let lim = q.int("limit", "20").map(|l| l.clamp(1, MAX_LIMIT));
    // the search starts only for a request it will answer, right after this request's state read
    let (idx, found) = match &lim {
        Ok(l) if !qs.is_empty() => {
            let (st2, qs2, l, hidden) = (st.clone(), qs.clone(), *l, q.include_hidden());
            let (_d, idx, h) = super::prepare_and(&st, &q, async move { fused(&st2, &qs2, l, hidden).await }).await?;
            (idx, Some(h))
        }
        _ => (super::prepare(&st, &q).await?.1, None),
    };
    if qs.is_empty() {
        return Err(ApiError::bad_request("q is required"));
    }
    let lim = lim?;
    let raw = match found {
        Some(h) => super::finish(h).await??,
        None => return Err(ApiError::internal("SearchError", "the search did not start")),
    };
    Ok(ok(&search_payload(&idx, &raw, &qs, usize::try_from(lim).unwrap_or(1))))
}

/// What sounds like a bit: its newest text's neighbours (with the model), or a word search on its first words.
pub async fn like_of(st: &AppState, v: &pk_kb::View, jid: &str, lim: i64, hidden: bool) -> Result<Like, ApiError> {
    let text = newest_text(v, jid);
    if text.is_empty() {
        return Ok(Like::default());
    }
    let limit = usize::try_from(lim).unwrap_or(1);
    if let Some(e) = st.model().get().await {
        let vec = st
            .model()
            .vector(e, text.clone(), false)
            .await
            .map_err(|e| ApiError::internal("EmbedError", e))?;
        let [units, jokes, perfs] = search::similar_rows(&*store(st)?, &vec, lim, hidden)
            .await
            .map_err(|e| ApiError::internal("StoreError", e))?;
        Ok(like_vectors(v, jid, text, units, jokes, perfs, limit))
    } else {
        let raw = search_all(&*store(st)?, None, &first_words(&text), lim.saturating_mul(2), hidden)
            .await
            .map_err(|e| ApiError::internal("StoreError", e))?;
        Ok(like_words(v, jid, text, raw, limit))
    }
}

/// `GET /api/jokes/<id>/similar[?limit=8]`.
pub async fn similar(
    State(st): State<AppState>,
    Path(jid): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (d, v) = super::prepare_view(&st, &q).await?;
    let lim = q.int("limit", "8")?.clamp(1, MAX_LIMIT);
    let limit = usize::try_from(lim).unwrap_or(1);
    if v.ntype(Some(&jid)) != Some("joke") {
        return Err(ApiError::not_found("no such joke"));
    }
    let hidden = q.include_hidden();
    let like = like_of(&st, &v, &jid, lim, hidden);
    let (idx, like) = tokio::join!(d.index(hidden), like);
    let (idx, like) = (idx?, like?);
    Ok(ok(&similar_payload(&idx, performed(&v, &jid), &like, limit)))
}
