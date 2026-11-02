//! Punchable's API on axum (docs/ARCHITECTURE.md section 2). Lane T9: the read routes that answer from the database
//! snapshot, behind the same Keycloak check, CORS, errors and metrics as `api-dev/server.py`; every other request
//! (every write, `/api/worker/*`, `/api/live`, the routes not ported yet) is forwarded unchanged to the Python API
//! until T10 (`proxy`).
//!
//! [`app`] builds the router over an [`AppState`]; route tests drive it with `tower::ServiceExt::oneshot`, no
//! socket. [`serve`] binds, supervises the database connection and shuts down the way the manifest expects.

pub mod auth;
pub mod cloud;
pub mod config;
pub mod cors;
pub mod display;
pub mod error;
pub mod index;
pub mod instrument;
pub mod proxy;
pub mod query;
pub mod routes;
pub mod snapshot;
pub mod state;

use std::time::Duration;

use axum::Router;
use axum::handler::Handler;
use axum::middleware::from_fn_with_state;
use axum::routing::{MethodRouter, get};
use tower::ServiceBuilder;

pub use config::ApiConfig;
pub use state::AppState;

/// The version `pk_build_info` and the crash reports carry.
pub const VERSION: &str = "0.1.0";

/// The paths this service answers itself (T9). Everything else goes to the Python. The Ingress manifest that moves
/// them lists exactly these (`infra/k8s/prepared/t9-api-rs.yaml`, not applied).
pub const OWN_PATHS: [&str; 7] = [
    "/api/map/cloud",
    "/api/health",
    "/api/labels",
    "/api/decisions",
    "/api/meta",
    "/api/pipeline",
    "/api/learned",
];

/// A read route: GET (and HEAD) answered here behind CORS, metrics and auth; OPTIONS is the preflight; any other
/// method goes to the Python (an Ingress path moved here must never swallow a write).
fn read_route<H, T>(st: &AppState, h: H) -> MethodRouter<AppState>
where
    H: Handler<T, AppState>,
    T: 'static,
{
    let layers = ServiceBuilder::new()
        .layer(from_fn_with_state(st.clone(), cors::cors))
        .layer(from_fn_with_state(st.clone(), instrument::instrument))
        .layer(from_fn_with_state(st.clone(), auth::authorize));
    let preflight = ServiceBuilder::new()
        .layer(from_fn_with_state(st.clone(), cors::cors))
        .layer(from_fn_with_state(st.clone(), instrument::instrument));
    get(h.layer(layers))
        .options((async || ()).layer(preflight))
        .fallback(proxy::forward)
}

/// The router.
pub fn app(st: AppState) -> Router {
    let r = Router::new()
        .route("/api/health", read_route(&st, routes::health::health))
        .route("/api/labels", read_route(&st, routes::kb::labels))
        .route("/api/decisions", read_route(&st, routes::kb::decisions))
        .route("/api/meta", read_route(&st, routes::kb::meta))
        .route("/api/pipeline", read_route(&st, routes::kb::pipeline))
        .route("/api/learned", read_route(&st, routes::learned::learned))
        .route("/api/map/cloud", read_route(&st, routes::cloud::map_cloud))
        .route("/metrics", get(routes::metrics::metrics).fallback(proxy::forward));
    #[cfg(feature = "diagnostics")]
    let r = r.route("/api/_t9/summaries", read_route(&st, routes::kb::summaries));
    r.fallback(proxy::forward).with_state(st)
}

/// Build the state from the settings (the verifier, the metrics, one HTTP client).
pub fn state(config: ApiConfig) -> anyhow::Result<AppState> {
    // jsonwebtoken picks its crypto backend from crate features and panics on every verify if a dependency ever
    // enables a second one: name ours explicitly (an `Err` = one is installed already, which is fine)
    let _ = jsonwebtoken::crypto::CryptoProvider::install_default(&jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER);
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(Duration::from_secs(300))
        .pool_idle_timeout(Duration::from_secs(60))
        .build()?;
    let verifier = config.auth.as_ref().map(|a| {
        let clock = auth::verifier::system_clock();
        let jwks = auth::Jwks::http(a.jwks_url.clone(), http.clone(), clock.clone());
        auth::Verifier::new(a.issuer.clone(), a.audience.clone(), jwks, clock)
    });
    let metrics = pk_obs::ApiMetrics::new(VERSION, &config.build_sha)?;
    Ok(AppState::new(config, verifier, metrics, http))
}

/// Serve until SIGTERM / Ctrl-C: then readiness goes 503, 10 s pass for the Service to stop sending, and the open
/// requests drain.
pub async fn serve(config: ApiConfig) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.host, config.port);
    let st = state(config)?;
    tokio::spawn(st.clone().supervise());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, auth = st.verifier().is_some(), "pk-api listening");
    let drain = st.clone();
    axum::serve(listener, app(st))
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            drain.start_draining();
            tracing::info!("shutting down: not ready; draining in 10 s");
            tokio::time::sleep(Duration::from_secs(10)).await;
        })
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = term => {},
    }
}
