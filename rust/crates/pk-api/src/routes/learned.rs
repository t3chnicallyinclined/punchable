//! `GET /api/learned`: the learning file (`kb/v0/learning.json`, a `srcfile` row), as it is.

use axum::extract::{RawQuery, State};
use axum::response::Response;
use serde_json::Value;

use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::state::AppState;

/// The handler.
pub async fn learned(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    // the file needs no index: no `?include_hidden=1` build here
    let d = st.data().await?;
    super::pick(&q)?;
    let Some(text) = d.learning.as_deref() else {
        return Err(ApiError::not_found("nothing learned yet (no learning.json)"));
    };
    // The Python's `json.loads` error text (a `ValueError`, so a 400) has no Rust twin; the status is kept.
    let v: Value = serde_json::from_str(text).map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(ok(&v))
}
