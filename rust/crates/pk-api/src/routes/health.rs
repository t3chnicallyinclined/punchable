//! `GET /api/health`: liveness, readiness (`?ready=1`), the `rev` the app compares, and `flags`, the only way to turn
//! a phone feature off without a build (`app/src/lib/flags.ts`). Signed out (auth on, no valid token) it answers
//! `{ok, rev, auth: "required", flags}` and nothing else; signed in, the Python's full shape.

use axum::Extension;
use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Value, json};

use crate::auth::Caller;
use crate::error::{ApiResult, json_response, ok};
use crate::query::Query;
use crate::snapshot::ai_status;
use crate::state::AppState;

fn path_str(p: &std::path::Path) -> String {
    p.display().to_string()
}

/// The handler.
pub async fn health(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let ready_probe = raw.as_deref().unwrap_or("").split('&').any(|p| p == "ready=1");
    if ready_probe && st.draining() {
        // new behaviour (replaces the manifest's `preStop: sleep 10`): not ready while the pod drains
        let mut b = st.boot_status();
        b["ready"] = Value::Bool(false);
        b["error"] = json!("shutting down");
        return Ok(json_response(StatusCode::SERVICE_UNAVAILABLE, &b));
    }
    let d = match st.data().await {
        Ok(d) => d,
        Err(_) if !st.loaded() => {
            // `server.py` `booting`: `ready=1` is the readiness probe (200 only once this pod serves data)
            let b = st.boot_status();
            let up = if ready_probe {
                b["ready"] == true
            } else {
                b["surreal"] == true
            };
            let code = if up {
                StatusCode::OK
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            return Ok(json_response(code, &b));
        }
        Err(e) => return Err(e),
    };
    super::pick(&q)?;
    let flags = Value::Object(st.config().flags.clone());
    if st.verifier().is_some() && caller.0.is_none() {
        return Ok(ok(
            &json!({"ok": true, "rev": d.rev(), "auth": "required", "flags": flags}),
        ));
    }
    let idx = d.index(q.include_hidden()).await?;
    let data = &st.config().data;
    let comedy = data.join("comedy");
    let graph = d
        .load_info
        .get("graph")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            d.load_info
                .get("bundle")
                .and_then(|b| b.get("graph"))
                .and_then(Value::as_str)
                .map(|g| path_str(&comedy.join(g)))
        })
        .unwrap_or_default();
    Ok(ok(&json!({
        "ok": true,
        "flags": flags,
        "graph": graph,
        "graph_state": "ok",
        "decisions": path_str(&comedy.join("kb").join("v0").join("decisions.jsonl")),
        "decision_lines": idx.view.decisions.len(),
        "bad_lines": d.load_info.get("bad_lines").cloned().unwrap_or(json!(0)),
        "inbox": path_str(&data.join("inbox")),
        "corpus": path_str(&comedy.join("corpus")),
        "tapes": path_str(&data.join("tapes")),
        "rev": d.rev(),
        "ai": ai_status(&d),
    })))
}
