//! `GET /api/today`: the older feed with `today.build` merged in (`crate::today`, `pk_creative::today`).

use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::response::Response;
use serde_json::{Map, Value};

use crate::error::{ApiError, ApiResult, ok};
use crate::pages::{compute, set_dirs_of};
use crate::query::Query;
use crate::state::AppState;

/// The working set touched last, replayed from the `workingsets` store's lines (read per request: creative writes
/// do not move the generation). `Err` is the text the Python puts in `creative_error`.
async fn latest_ws(st: &AppState) -> Result<Option<Map<String, Value>>, String> {
    let store = st
        .store_handle()
        .ok_or("ConnectionError: the database is not connected")?;
    let lines = store
        .creative_lines("workingsets")
        .await
        .map_err(|e| format!("HttpError: {e}"))?;
    let reducer =
        pk_creative::reducers::for_store("workingsets", &st.config().owner_id).ok_or("KeyError: 'workingsets'")?;
    let ws = pk_creative::store::Store::replay(reducer, &lines).map_err(|e| format!("ValueError: {e}"))?;
    Ok(pk_creative::today::latest_ws(ws.items()).cloned())
}

/// `GET /api/today`.
pub async fn today(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (d, idx) = super::prepare(&st, &q).await?;
    let corpus = {
        let (d, st) = (Arc::clone(&d), st.clone());
        tokio::spawn(async move { d.corpus(&st).await })
    };
    let dirs = idx.sets.last().map(|s| set_dirs_of(s)).unwrap_or_default();
    let tx = super::sets::transcripts(&st, &d, dirs).await?;
    let ws = latest_ws(&st).await;
    // `date.today()` on the Python pod: its local day
    let day = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();
    let c = corpus.await.map_err(|e| ApiError::internal("JoinError", e))?;
    let body = compute(idx, d, tx, move |ctx| {
        let old = crate::today::old_today(ctx);
        // the creative half never takes the old feed down (`creative_error`)
        match (c, ws) {
            (Ok(c), Ok(ws)) => {
                let t = pk_creative::today::build(&c, ws.as_ref(), &day);
                crate::today::merged_today(old, t)
            }
            (Err(e), _) => with_error(old, &e.msg),
            (_, Err(e)) => with_error(old, &e),
        }
    })
    .await?;
    Ok(ok(&Value::Object(body)))
}

fn with_error(mut old: Map<String, Value>, msg: &str) -> Map<String, Value> {
    tracing::warn!("today: creative part failed: {msg}");
    old.insert("creative_error".into(), Value::String(msg.to_string()));
    old
}
