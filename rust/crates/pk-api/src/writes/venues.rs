//! `PATCH /api/venues/<id>` (T10 round 5, group B; `creative_api.Routes.venue_patch`): he moves a room's pin
//! (`lat` and `lon`), renames it (`name`) or puts both back (`reset`). The ops of one body are checked on a copy of
//! the store before any is written and land in one transaction; the answer is the room and its map feature as
//! `GET /api/venues` and `/api/venues.geojson` now show them.

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::HeaderMap;
use axum::response::Response;
use pk_core::py;
use serde_json::{Value, json};

use super::{Creative, ProbeError, Req, detached, http, request};
use crate::auth::Caller;
use crate::error::{ApiError, ApiResult, ok};
use crate::state::AppState;

/// `PATCH /api/venues/<id> {lat?, lon?, name?, reset?}`.
pub async fn patch(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path(vid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _cr = st.creative_lock().lock().await;
        let d = st.data_for_write().await?;
        let c = d.corpus(&st).await?;
        let cr = Creative::new(st.clone());
        let known = {
            let (c, vid) = (std::sync::Arc::clone(&c), vid.clone());
            tokio::task::spawn_blocking(move || {
                pk_creative::venues::venue_stats(&c)
                    .iter()
                    .any(|v| v.get("venue_id").and_then(Value::as_str) == Some(vid.as_str()))
            })
            .await
            .map_err(|e| ApiError::internal("JoinError", e))?
        };
        if !known {
            return Err(http(404, "no such venue"));
        }
        let mut ops: Vec<(String, String, Value)> = Vec::new();
        if b.contains_key("lat") || b.contains_key("lon") {
            ops.push((
                "pin".into(),
                vid.clone(),
                json!({"lat": b.get("lat").unwrap_or(&Value::Null), "lon": b.get("lon").unwrap_or(&Value::Null)}),
            ));
        }
        if b.contains_key("name") {
            ops.push((
                "rename".into(),
                vid.clone(),
                json!({"name": b.get("name").unwrap_or(&Value::Null)}),
            ));
        }
        if py::truthy(b.get("reset")) {
            ops = vec![("reset".into(), vid.clone(), json!({}))];
        }
        if ops.is_empty() {
            return Err(http(400, "send lat+lon, name, or reset"));
        }
        // a name that is not text is the Python's AttributeError (a 500), not a refusal: `(d.get("name") or "").strip()`
        cr.probe("venues", &ops, &owner).await.map_err(|e| match e {
            ProbeError::Failed(a) if a.msg.contains("has no attribute 'strip'") => {
                ApiError::internal("AttributeError", a.msg)
            }
            other => other.into(),
        })?;
        cr.append_all("venues", &ops, &owner).await?;
        let edits = cr.items("venues").await?;
        let built = crate::routes::venues::built(&st, &d, c, edits).await?;
        let venue = built
            .venues
            .iter()
            .find(|v| v.get("venue_id").and_then(Value::as_str) == Some(vid.as_str()))
            .map(|v| Value::Object(v.clone()))
            .ok_or_else(|| ApiError::internal("StopIteration", ""))?;
        let feature = built
            .features
            .iter()
            .find(|f| f["properties"]["venue_id"].as_str() == Some(vid.as_str()))
            .cloned()
            .ok_or_else(|| ApiError::internal("StopIteration", ""))?;
        Ok(json!({"venue": venue, "feature": feature}))
    })
    .await?;
    Ok(ok(&out))
}
