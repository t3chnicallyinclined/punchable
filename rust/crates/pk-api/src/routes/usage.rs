//! The usage meter's two routes (SPEC 2026-11-17 §13): `GET /api/admin/usage?from&to[&format=csv]` (the owner's
//! page: owner only, pinned by `PK_OWNER_SUB`) and `POST /api/worker/usage` (the worker's model calls; a worker
//! token, as every `/api/worker/*` route). pk-api only: the Python has neither, and nothing forwards them.

use axum::Extension;
use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pk_wire::worker::usage::{UsagePost, UsagePostResponse};
use serde_json::{Value, json};

use crate::auth::{Caller, owner_only};
use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::state::AppState;
use crate::writes::worker::WORKER_BODY;
use crate::writes::{detached, request_within};

/// `GET /api/admin/usage`: this month so far by default; `from` / `to` (`YYYY-MM-DD`, UTC) for another range;
/// `format=csv` for the export (one line per day).
pub async fn admin(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    owner_only(&st, &caller)?;
    let q = Query::parse(raw.as_deref());
    let (from, to) = (q.get("from").map(str::to_string), q.get("to").map(str::to_string));
    let csv = match q.get("format") {
        None | Some("json") => false,
        Some("csv") => true,
        Some(_) => return Err(ApiError::bad_request("format must be json or csv")),
    };
    let store = st.store_handle().ok_or_else(crate::writes::not_connected)?;
    let report = detached(async move {
        let account = st.config().owner_id.clone();
        st.usage()
            .report(&store, &account, from.as_deref(), to.as_deref())
            .await
    })
    .await?;
    if !csv {
        return Ok(ok(&report));
    }
    let mut r = (StatusCode::OK, crate::usage::csv(&report)).into_response();
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"punchable-usage.csv\""),
    );
    Ok(r)
}

/// `GET /api/usage/access`: `{owner, pinned}`: whether this token may read the Usage page, and whether the server has
/// an owner pinned at all. The app shows Settings › Usage when `owner`, or when nothing is pinned yet (the page then
/// says it is not switched on); a tester never sees a row that only refuses. Any signed-in app token may ask.
pub async fn access(State(st): State<AppState>, Extension(caller): Extension<Caller>) -> ApiResult<Response> {
    let pinned = st.config().owner_sub.is_some();
    let owner = owner_only(&st, &caller).is_ok();
    Ok(ok(&json!({"owner": owner, "pinned": pinned})))
}

/// `POST /api/worker/usage {calls: [...]}`: stored and priced, `{stored: n}`. A batch posted twice is stored once.
pub async fn worker(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let req = request_within(st, raw.as_deref(), &headers, body, WORKER_BODY).await?;
    let post: UsagePost = serde_json::from_value(Value::Object(req.b))
        .map_err(|e| ApiError::bad_request(format!("not a usage post: {e}")))?;
    let st = req.st;
    let store = st.store_handle().ok_or_else(crate::writes::not_connected)?;
    let stored = detached(async move {
        let account = st.config().owner_id.clone();
        st.usage().ingest(&store, &account, &post.calls).await
    })
    .await?;
    Ok(ok(&json!(UsagePostResponse { stored })))
}
