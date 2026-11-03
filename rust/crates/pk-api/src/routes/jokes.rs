//! `GET /api/jokes` (`server.py` `do_GET`): the Bits list with its filters (`?q=&mode=hybrid` = the fused full-text
//! and vector search, best first), the notes auto-attached under a bit taken out (`merge_api.desk_filter`), the
//! Desk's sorts, one page, and each row's running time from the notebook (`Corpus.duration`: his taped median, else
//! his written words at his pace).

use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::response::Response;
use serde_json::{Value, json};

use crate::error::{ApiError, ApiResult, ok};
use crate::jokes::{MAX_JOKES_PAGE, filtered, sort_jokes};
use crate::query::Query;
use crate::search::fold_jokes;
use crate::state::AppState;

/// `GET /api/jokes?q=&mode=&stage=&label=&who=&sort=&offset=&limit=`.
pub async fn jokes(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let qs = pk_core::py::strip(q.get("q").unwrap_or("")).to_string();
    // `?q=&mode=hybrid`: the fused search reads the database, not the generation, so it starts right after this
    // request's state read and runs while the data is made ready
    let hybrid = (!qs.is_empty() && q.get("mode") == Some("hybrid")).then(|| q.int("limit", "40"));
    let (d, idx, found) = if let Some(Ok(l)) = &hybrid {
        let (st2, qs2, l, hidden) = (st.clone(), qs.clone(), *l, q.include_hidden());
        let work = async move { super::search::fused(&st2, &qs2, l, hidden).await };
        let (d, idx, h) = super::prepare_and(&st, &q, work).await?;
        (d, idx, Some(h))
    } else {
        let (d, idx) = super::prepare(&st, &q).await?;
        (d, idx, None)
    };
    let ranked = match (hybrid, found) {
        (Some(Err(e)), _) => return Err(e),
        (_, Some(h)) => Some(fold_jokes(&super::finish(h).await??.hits, &idx.view)),
        _ => None,
    };
    // the notebook and the merges store do not depend on the filters: read them while the filters run
    let corpus = {
        let (d, st) = (Arc::clone(&d), st.clone());
        tokio::spawn(async move { d.corpus(&st).await })
    };
    let mut items = filtered(&idx, &q, ranked.as_deref());
    if !q.include_hidden() {
        let store = st.store_handle();
        if let Some(att) = st.attached().get(store.as_deref(), &st.config().owner_id).await
            && !att.is_empty()
        {
            items.retain(|x| !att.contains(&x.summary.id));
        }
    }
    if let Some(how) = q.get("sort") {
        sort_jokes(&mut items, how)?;
    }
    let off = q.int("offset", "0")?.max(0);
    let lim = q.int("limit", "40")?.clamp(1, MAX_JOKES_PAGE);
    let c = corpus.await.map_err(|e| ApiError::internal("JoinError", e))??;
    let start = usize::try_from(off).unwrap_or(usize::MAX).min(items.len());
    let end = start
        .saturating_add(usize::try_from(lim).unwrap_or(usize::MAX))
        .min(items.len());
    let page: Vec<Value> = items[start..end]
        .iter()
        .map(|x| {
            let mut s = x.summary.clone();
            let (secs, basis) = c.duration(&s.id);
            s.duration_s = Some(secs);
            s.duration_estimated = Some(!basis.starts_with("tape"));
            serde_json::to_value(s)
        })
        .collect::<Result<_, _>>()
        .map_err(|e| ApiError::internal("JSONError", e))?;
    Ok(ok(
        &json!({"items": page, "total": items.len(), "offset": off, "limit": lim}),
    ))
}
