//! Punchable's API on axum (docs/ARCHITECTURE.md section 2). Lane T9: the read routes that answer from the database
//! snapshot, behind the same Keycloak check, CORS, errors and metrics as `api-dev/server.py`; lane T10: the writes,
//! group by group (`writes`). Every other request (`/api/worker/*`, `/api/live`, the routes not ported yet) is
//! forwarded unchanged to the Python API until T10 is whole (`proxy`).
//!
//! [`app`] builds the router over an [`AppState`]; route tests drive it with `tower::ServiceExt::oneshot`, no
//! socket. [`serve`] binds, supervises the database connection and shuts down the way the manifest expects.

pub mod auth;
pub mod cloud;
pub mod config;
pub mod cors;
pub mod creative;
pub mod display;
pub mod embed;
pub mod error;
pub mod index;
pub mod instrument;
pub mod jokes;
pub mod pages;
pub mod proxy;
pub mod query;
pub mod routes;
pub mod search;
pub mod setlists;
pub mod snapshot;
pub mod state;
pub mod tape;
pub mod today;
pub mod writes;

use std::time::Duration;

use axum::Router;
use axum::handler::Handler;
use axum::middleware::from_fn_with_state;
use axum::routing::{MethodRouter, get, post};
use tower::ServiceBuilder;

pub use config::ApiConfig;
pub use state::AppState;

/// The version `pk_build_info` and the crash reports carry.
pub const VERSION: &str = "0.1.0";

/// The paths this service answers itself (T9). Everything else goes to the Python. The Ingress manifest that moves
/// them lists exactly these (`infra/k8s/prepared/t10-api-rs.yaml`, not applied).
pub const OWN_PATHS: [&str; 19] = [
    "/api/jokes",
    "/api/search",
    "/api/today",
    "/api/map/cloud",
    "/api/sets",
    "/api/health",
    "/api/labels",
    "/api/decisions",
    "/api/meta",
    "/api/pipeline",
    "/api/learned",
    "/api/settings",
    "/api/boards",
    "/api/mysets",
    "/api/tonight/screen",
    "/api/profile/voice",
    "/api/suggestions/not-a-joke",
    "/api/workingsets",
    "/api/notebook/related",
];

/// The parametric paths answered here (axum patterns). The prepared Ingress moves them with regex paths
/// ([`ingress_regex`] gives each one's path), never with a prefix: `/api/jokes/<id>/versions` (POST) and every
/// other path under an id stays the Python's, and an id path with more segments than these patterns falls through to
/// the fallback and is forwarded. A write on one of these paths (PATCH `/api/boards/<id>`, PATCH `/api/mysets/<id>`)
/// is forwarded too (`read_route`), and `/api/mysets/match` (the Python's route, not a set id) is forwarded whatever
/// the method.
pub const OWN_PATTERNS: [&str; 15] = [
    "/api/jokes/{id}",
    "/api/jokes/{id}/similar",
    "/api/sets/{id}/analysis",
    "/api/sets/{id}/review",
    "/api/boards/{id}",
    "/api/boards/{id}/build",
    "/api/mysets/{id}",
    "/api/mysets/{id}/trim",
    "/api/jokes/{id}/connections",
    "/api/jokes/{id}/map",
    "/api/jobs/{id}",
    "/api/workingsets/{id}",
    "/api/workingsets/{id}/nights",
    "/api/workingsets/{id}/bits/{id2}",
    "/api/sets/{id}/clips",
];

/// The paths whose POST this service answers itself (T10, groups 1 and 2). An exact Ingress path moves every
/// method: a GET on one of these (none exists on the Python) is forwarded and answered by it.
pub const OWN_WRITE_PATHS: [&str; 8] = [
    "/api/ideas",
    "/api/sets/transcript",
    "/api/decision",
    "/api/undo",
    "/api/merge",
    "/api/merge/attach",
    "/api/merge/dismiss",
    "/api/merge/undo",
];

/// The parametric paths whose POST this service answers itself (T10): his new version of a bit.
pub const OWN_WRITE_PATTERNS: [&str; 1] = ["/api/jokes/{id}/versions"];

/// The nginx regex path the prepared Ingress writes for an [`OWN_PATTERNS`] entry (`{id}` = one segment).
pub fn ingress_regex(pattern: &str) -> String {
    format!("{}$", pattern.replace("{id}", "[^/]+").replace("{id2}", "[^/]+"))
}

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

/// A write route: POST answered here behind CORS, metrics and auth; OPTIONS is the preflight; any other method goes
/// to the Python.
fn write_route<H, T>(st: &AppState, h: H) -> MethodRouter<AppState>
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
    // the Python reads `Content-Length` and refuses a body over its own limit before reading it (`body_json`): axum's
    // default 2 MiB limit would answer a plain 413 first
    post(h.layer(layers).layer(axum::extract::DefaultBodyLimit::disable()))
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
        .route("/api/jokes", read_route(&st, routes::jokes::jokes))
        .route("/api/search", read_route(&st, routes::search::search))
        .route("/api/jokes/{id}/similar", read_route(&st, routes::search::similar))
        .route("/api/today", read_route(&st, routes::today::today))
        .route("/api/sets", read_route(&st, routes::sets::sets))
        .route("/api/sets/{id}/analysis", read_route(&st, routes::sets::analysis))
        .route("/api/sets/{id}/review", read_route(&st, routes::sets::review))
        .route("/api/jokes/{id}", read_route(&st, routes::sets::joke))
        .route("/api/settings", read_route(&st, routes::creative::settings))
        .route("/api/boards", read_route(&st, routes::creative::boards))
        .route("/api/mysets", read_route(&st, routes::creative::mysets))
        .route("/api/tonight/screen", read_route(&st, routes::creative::tonight_screen))
        .route("/api/boards/{id}", read_route(&st, routes::sets_detail::board))
        .route(
            "/api/boards/{id}/build",
            read_route(&st, routes::sets_detail::board_build),
        )
        // `/api/mysets/match` is the Python's (`mysets_api` matches it before an id): never read as a set id here
        .route("/api/mysets/match", axum::routing::any(proxy::forward))
        .route("/api/mysets/{id}", read_route(&st, routes::sets_detail::myset))
        .route("/api/mysets/{id}/trim", read_route(&st, routes::sets_detail::trim))
        .route(
            "/api/jokes/{id}/connections",
            read_route(&st, routes::sets_detail::connections),
        )
        .route("/api/jokes/{id}/map", read_route(&st, routes::sets_detail::joke_map))
        .route("/api/profile/voice", read_route(&st, routes::stores::voice))
        .route(
            "/api/suggestions/not-a-joke",
            read_route(&st, routes::stores::not_a_joke),
        )
        .route("/api/jobs/{id}", read_route(&st, routes::stores::job))
        .route("/api/workingsets", read_route(&st, routes::sets_detail::workingsets))
        .route("/api/notebook/related", read_route(&st, routes::sets_detail::related))
        .route("/api/sets/{id}/clips", read_route(&st, routes::sets_detail::clips))
        .route(
            "/api/workingsets/{id}",
            read_route(&st, routes::sets_detail::workingset),
        )
        .route(
            "/api/workingsets/{id}/nights",
            read_route(&st, routes::sets_detail::ws_nights),
        )
        .route(
            "/api/workingsets/{id}/bits/{id2}",
            read_route(&st, routes::sets_detail::ws_bit),
        )
        .route("/metrics", get(routes::metrics::metrics).fallback(proxy::forward))
        // T10 group 1: what the app does most
        .route("/api/ideas", write_route(&st, writes::ideas::ideas))
        .route("/api/sets/transcript", write_route(&st, writes::transcript::transcript))
        // T10 group 2: one bit's writes
        .route("/api/jokes/{id}/versions", write_route(&st, writes::bits::versions))
        .route("/api/decision", write_route(&st, writes::bits::decision))
        .route("/api/undo", write_route(&st, writes::bits::undo))
        .route("/api/merge", write_route(&st, writes::merge::merge))
        .route("/api/merge/attach", write_route(&st, writes::merge::attach))
        .route("/api/merge/dismiss", write_route(&st, writes::merge::dismiss))
        .route("/api/merge/undo", write_route(&st, writes::merge::undo));
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
    // the private legal rules, when this process has the file (the hosted pod does not): the generic markers plus his
    let mut config = config;
    config.legal = match std::fs::read_to_string(&config.legal_rules) {
        Ok(text) => pk_core::legal::Legal::from_rules_json(&text),
        Err(_) => pk_core::legal::Legal::default(),
    };
    tracing::info!(markers = config.legal.len(), "legal rules loaded");
    Ok(AppState::new(config, verifier, metrics, http))
}

/// Serve until SIGTERM / Ctrl-C: then readiness goes 503, 10 s pass for the Service to stop sending, and the open
/// requests drain.
pub async fn serve(config: ApiConfig) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.host, config.port);
    let st = state(config)?;
    tokio::spawn(st.clone().supervise());
    let listener = bind(&addr).await?;
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

/// The listener: every accepted connection has Nagle's algorithm off (`TCP_NODELAY`). axum 0.8's `serve` leaves it
/// on, and a forwarded answer is streamed in more than one write when the Python's answer arrives in pieces (the
/// head, then each chunk): with Nagle on, a small write waits for the client's ACK of the one before, which the
/// client may delay (up to 200 ms on Windows, 40 ms on Linux). Hardening, not the round-5 slowdown: on the owner's
/// PC the forward measured within 2 ms of the Python direct with Nagle on (`docs/RUST_WORKSTREAM.md` T9 round 6).
pub async fn bind(addr: &str) -> std::io::Result<Listener> {
    use axum::serve::ListenerExt as _;
    Ok(tokio::net::TcpListener::bind(addr)
        .await?
        .tap_io(nodelay as fn(&mut tokio::net::TcpStream)))
}

/// What [`bind`] returns.
pub type Listener = axum::serve::TapIo<tokio::net::TcpListener, fn(&mut tokio::net::TcpStream)>;

fn nodelay(tcp: &mut tokio::net::TcpStream) {
    if let Err(e) = tcp.set_nodelay(true) {
        tracing::warn!(error = %e, "TCP_NODELAY could not be set on a connection");
    }
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
