//! The worker's one listener (`worker/metrics.py` `serve`): `GET /metrics` (the `pk_worker_*` and `pk_ai_*`
//! registry) and `GET /healthz` (200 while the heartbeat is under 20 minutes old), on `PK_METRICS_PORT` (9100). The
//! `NetworkPolicy` admits only the observability namespace. Rendering reads the spend ledger, so it runs off the
//! async threads.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use pk_obs::WorkerMetrics;
use tokio_util::sync::CancellationToken;

/// `/healthz` fails once the heartbeat is this old.
pub const HEALTHY: Duration = Duration::from_secs(20 * 60);

fn text(status: StatusCode, ctype: &'static str, body: String) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, ctype), (header::CACHE_CONTROL, "no-store")],
        body,
    )
        .into_response()
}

async fn metrics(State(m): State<Arc<WorkerMetrics>>) -> Response {
    match tokio::task::spawn_blocking(move || m.render()).await {
        Ok(Ok(body)) => text(StatusCode::OK, pk_obs::CONTENT_TYPE, body),
        _ => text(
            StatusCode::INTERNAL_SERVER_ERROR,
            "text/plain; charset=utf-8",
            "metrics unavailable\n".into(),
        ),
    }
}

async fn healthz(State(m): State<Arc<WorkerMetrics>>) -> Response {
    match m.heartbeat_age() {
        Some(age) if age < HEALTHY => text(
            StatusCode::OK,
            "text/plain; charset=utf-8",
            format!("ok heartbeat {:.0}s ago\n", age.as_secs_f64()),
        ),
        _ => text(
            StatusCode::SERVICE_UNAVAILABLE,
            "text/plain; charset=utf-8",
            "no recent heartbeat\n".into(),
        ),
    }
}

async fn not_found() -> Response {
    text(StatusCode::NOT_FOUND, "text/plain; charset=utf-8", "not found\n".into())
}

/// The two routes over the registry.
pub fn router(m: Arc<WorkerMetrics>) -> Router {
    Router::new()
        .route("/metrics", get(metrics))
        .route("/healthz", get(healthz))
        .fallback(not_found)
        .with_state(m)
}

/// Bind and serve until `stop`; returns the bound address once listening (port 0 picks one).
pub async fn serve(
    addr: SocketAddr,
    m: Arc<WorkerMetrics>,
    stop: CancellationToken,
) -> std::io::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let served = axum::serve(listener, router(m))
            .with_graceful_shutdown(async move { stop.cancelled().await })
            .await;
        if let Err(e) = served {
            tracing::warn!("metrics listener stopped: {e}");
        }
    });
    Ok((bound, task))
}
