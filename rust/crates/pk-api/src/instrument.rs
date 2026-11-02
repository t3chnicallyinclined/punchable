//! Request metrics and the request span (`api-dev/metrics.py` `instrument`): every request this service answers is
//! timed and counted by method, route pattern (`pk_obs::route_pattern`: ids become `{id}`, the query is never read)
//! and status class. Logged with method, route, status and milliseconds only: never a body, a query or an id.

use std::time::Instant;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::state::AppState;

/// The in-flight gauge, decremented even when the client goes away and the request future is dropped.
struct InFlight(prometheus::IntGauge);

impl InFlight {
    fn enter(g: &prometheus::IntGauge) -> Self {
        g.inc();
        Self(g.clone())
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.dec();
    }
}

/// The middleware.
pub async fn instrument(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let m = st.metrics();
    let flight = InFlight::enter(&m.in_flight);
    let t = Instant::now();
    let r = next.run(req).await;
    let secs = t.elapsed().as_secs_f64();
    drop(flight);
    let status = r.status().as_u16();
    m.record(&method, &path, i64::from(status), secs);
    tracing::debug!(
        method = %method,
        route = %pk_obs::route_pattern(&path),
        status,
        ms = secs * 1000.0,
        "request"
    );
    r
}
