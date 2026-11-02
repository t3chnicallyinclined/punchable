//! `GET /api/jokes` (`server.py` `do_GET`): the Bits list with its filters, the notes auto-attached under a bit
//! taken out (`merge_api.desk_filter`), the Desk's sorts, one page, and each row's running time from the notebook
//! (`Corpus.duration`: his taped median, else his written words at his pace).

use std::sync::Arc;

use axum::extract::{RawQuery, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use serde_json::{Value, json};

use crate::error::{ApiError, ApiResult, ok};
use crate::jokes::{MAX_JOKES_PAGE, filtered, sort_jokes};
use crate::query::Query;
use crate::state::AppState;

/// `?q=<words>&mode=hybrid` (fused full text and vectors) is the Python's until the model runs here: forwarded
/// before this service's own layers, so the Python's auth, CORS and metrics answer it alone.
pub async fn hybrid_to_python(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let q = Query::parse(req.uri().query());
    let hybrid = q.get("mode") == Some("hybrid") && !pk_core::py::strip(q.get("q").unwrap_or("")).is_empty();
    if hybrid && matches!(*req.method(), axum::http::Method::GET | axum::http::Method::HEAD) {
        return crate::proxy::forward(State(st), req).await;
    }
    next.run(req).await
}

/// `GET /api/jokes?q=&stage=&label=&who=&sort=&offset=&limit=`.
pub async fn jokes(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (d, idx) = super::prepare(&st, &q).await?;
    // the notebook and the merges store do not depend on the filters: read them while the filters run
    let corpus = {
        let (d, st) = (Arc::clone(&d), st.clone());
        tokio::spawn(async move { d.corpus(&st).await })
    };
    let mut items = filtered(&idx, &q);
    if !q.include_hidden() {
        let store = st
            .store_handle()
            .ok_or_else(|| ApiError::unavailable("the database is not connected"))?;
        if let Some(att) = crate::creative::attached(&store, &st.config().owner_id).await
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
            serde_json::to_value(s).unwrap_or(Value::Null)
        })
        .collect();
    Ok(ok(
        &json!({"items": page, "total": items.len(), "offset": off, "limit": lim}),
    ))
}
