//! Punchable's API on axum (docs/ARCHITECTURE.md section 2). Lane T9: the read routes that answer from the database
//! snapshot, behind the same Keycloak check, CORS, errors and metrics as `api-dev/server.py`; lane T10: the writes,
//! group by group (`writes`), the Ask conversations, `/api/live` (`live`, `routes::live`), the worker's routes (`writes::worker`) and the
//! vector sweeper (`sweep`). Every other request (the routes not ported yet) is forwarded unchanged to the Python
//! API until T10 is whole (`proxy`).
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
pub mod live;
pub mod pages;
pub mod proxy;
pub mod query;
pub mod routes;
pub mod search;
pub mod setlists;
pub mod snapshot;
pub mod state;
pub mod sweep;
pub mod tape;
pub mod today;
pub mod worker;
pub mod writes;

use std::time::Duration;

use axum::Router;
use axum::handler::Handler;
use axum::middleware::from_fn_with_state;
use axum::routing::{MethodRouter, get, patch, post};
use tower::ServiceBuilder;

pub use config::ApiConfig;
pub use state::AppState;

/// The version `pk_build_info` and the crash reports carry.
pub const VERSION: &str = "0.1.0";

/// The paths this service answers itself (T9). Everything else goes to the Python. The Ingress manifest that moves
/// them lists exactly these (`infra/k8s/prepared/t10-api-rs.yaml`, not applied). A path here whose write is
/// answered here too is in [`OWN_WRITE_PATHS`] as well.
pub const OWN_PATHS: [&str; 28] = [
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
    "/api/merge/candidates",
    "/api/live",
    "/api/worker/decisions",
    "/api/worker/creative",
    "/api/worker/profile/voice",
    "/api/inbox",
    "/api/memos",
    "/api/merge/history",
    "/api/ask/chats",
];

/// The parametric paths answered here (axum patterns). The prepared Ingress moves them with regex paths
/// ([`ingress_regex`] gives each one's path), never with a prefix: every other path under an id stays the Python's,
/// and an id path with more segments than these patterns falls through to the fallback and is forwarded. A write on
/// one of these paths is answered here only when the pattern is in [`OWN_WRITE_PATTERNS`] too (PATCH
/// `/api/boards/<id>`), else forwarded (PATCH `/api/mysets/<id>`); `/api/mysets/match` (the Python's route, not a
/// set id) is forwarded whatever the method.
pub const OWN_PATTERNS: [&str; 20] = [
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
    "/api/memos/{id}",
    "/api/memos/{id}/audio",
    "/api/worker/jobs/{id}/file",
    "/api/merge/{id}",
    "/api/ask/chats/{id}",
];

/// The paths whose writes (POST, PATCH or PUT, as the Python takes them) this service answers itself (T10, groups
/// 1 to 5). An exact Ingress path moves every method: a method the Python has on the path and this service does
/// not is forwarded and answered by it.
pub const OWN_WRITE_PATHS: [&str; 27] = [
    "/api/ideas",
    "/api/sets/transcript",
    "/api/decision",
    "/api/undo",
    "/api/merge",
    "/api/merge/attach",
    "/api/merge/dismiss",
    "/api/merge/undo",
    "/api/tonight",
    "/api/review/rate",
    "/api/set-row",
    "/api/review/reject-match",
    "/api/boards",
    "/api/workingsets",
    "/api/settings",
    "/api/profile/voice",
    "/api/profile/voice/learn",
    "/api/suggestions/not-a-joke/keep",
    "/api/suggestions/not-a-joke/hide",
    "/api/worker/claim",
    "/api/worker/profile/voice",
    "/api/ask",
    "/api/riff",
    "/api/facts",
    "/api/import/youtube",
    "/api/upload",
    "/api/ask/chats",
];

/// The parametric paths whose writes this service answers itself (T10): one bit's version, punch-up, rant and hook, a set
/// list's changes, a set's versions, locks and try notes, a job's result and its drop.
pub const OWN_WRITE_PATTERNS: [&str; 15] = [
    "/api/jokes/{id}/versions",
    "/api/jokes/{id}/punchup",
    "/api/jokes/{id}/rant",
    "/api/jokes/{id}/hook",
    "/api/boards/{id}",
    "/api/boards/{id}/items",
    "/api/boards/{id}/bridge",
    "/api/workingsets/{id}/versions",
    "/api/workingsets/{id}/lock",
    "/api/workingsets/{id}/try",
    "/api/workingsets/{id}/try/{id2}",
    "/api/workingsets/{id}/bits/{id2}/punchup",
    "/api/worker/jobs/{id}/result",
    "/api/jobs/{id}/drop",
    "/api/ask/chats/{id}",
];

/// The nginx regex path the prepared Ingress writes for an [`OWN_PATTERNS`] entry (`{id}` = one segment).
pub fn ingress_regex(pattern: &str) -> String {
    format!("{}$", pattern.replace("{id}", "[^/]+").replace("{id2}", "[^/]+"))
}

/// The layers of an app route: CORS, metrics, auth, in that order.
fn guarded<H, T>(st: &AppState, h: H) -> impl Handler<T, AppState>
where
    H: Handler<T, AppState>,
    T: 'static,
{
    h.layer(
        ServiceBuilder::new()
            .layer(from_fn_with_state(st.clone(), cors::cors))
            .layer(from_fn_with_state(st.clone(), instrument::instrument))
            .layer(from_fn_with_state(st.clone(), auth::authorize)),
    )
}

/// [`guarded`] for a route that reads a body: the Python reads `Content-Length` and refuses a body over its own
/// limit before reading it (`body_json`); axum's default 2 MiB limit would answer a plain 413 first.
fn guarded_write<H, T>(st: &AppState, h: H) -> impl Handler<T, AppState>
where
    H: Handler<T, AppState>,
    T: 'static,
{
    guarded(st, h).layer(axum::extract::DefaultBodyLimit::disable())
}

/// The `/api/live` socket: CORS and auth, never the request metrics (`instrument(skip=("/api/live", ...))`: the
/// open sockets are `pk_live_clients`), the upgrade answered here and any other method forwarded.
fn live_route(st: &AppState) -> MethodRouter<AppState> {
    let h = routes::live::live.layer(
        ServiceBuilder::new()
            .layer(from_fn_with_state(st.clone(), cors::cors))
            .layer(from_fn_with_state(st.clone(), auth::authorize)),
    );
    finish(st, get(h))
}

/// The preflight: CORS and metrics, no auth.
fn preflight(st: &AppState) -> impl Handler<((),), AppState> {
    (async || ()).layer(
        ServiceBuilder::new()
            .layer(from_fn_with_state(st.clone(), cors::cors))
            .layer(from_fn_with_state(st.clone(), instrument::instrument)),
    )
}

/// The methods a path answers here, finished: OPTIONS is the preflight, and any other method goes to the Python
/// (an Ingress path moved here must never swallow a write it does not answer).
fn finish(st: &AppState, methods: MethodRouter<AppState>) -> MethodRouter<AppState> {
    methods.options(preflight(st)).fallback(proxy::forward)
}

/// A read route: GET (and HEAD) answered here.
fn read_route<H, T>(st: &AppState, h: H) -> MethodRouter<AppState>
where
    H: Handler<T, AppState>,
    T: 'static,
{
    finish(st, get(guarded(st, h)))
}

/// A write route: POST answered here.
fn write_route<H, T>(st: &AppState, h: H) -> MethodRouter<AppState>
where
    H: Handler<T, AppState>,
    T: 'static,
{
    finish(st, post(guarded_write(st, h)))
}

/// A PATCH-only route (a try note's tick).
fn patch_route<H, T>(st: &AppState, h: H) -> MethodRouter<AppState>
where
    H: Handler<T, AppState>,
    T: 'static,
{
    finish(st, patch(guarded_write(st, h)))
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
        .route(
            "/api/settings",
            finish(
                &st,
                get(guarded(&st, routes::creative::settings))
                    .patch(guarded_write(&st, writes::account::settings_patch)),
            ),
        )
        .route(
            "/api/boards",
            finish(
                &st,
                get(guarded(&st, routes::creative::boards)).post(guarded_write(&st, writes::boards::create)),
            ),
        )
        .route("/api/mysets", read_route(&st, routes::creative::mysets))
        .route("/api/tonight/screen", read_route(&st, routes::creative::tonight_screen))
        .route(
            "/api/boards/{id}",
            finish(
                &st,
                get(guarded(&st, routes::sets_detail::board)).patch(guarded_write(&st, writes::boards::patch)),
            ),
        )
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
        .route(
            "/api/profile/voice",
            finish(
                &st,
                get(guarded(&st, routes::stores::voice)).put(guarded_write(&st, writes::account::profile_put)),
            ),
        )
        .route(
            "/api/suggestions/not-a-joke",
            read_route(&st, routes::stores::not_a_joke),
        )
        .route("/api/jobs/{id}", read_route(&st, routes::stores::job))
        .route(
            "/api/workingsets",
            finish(
                &st,
                get(guarded(&st, routes::sets_detail::workingsets))
                    .post(guarded_write(&st, writes::workingsets::create)),
            ),
        )
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
        .route("/api/merge/undo", write_route(&st, writes::merge::undo))
        // T10 group 3: the Set page, the Tonight list, the set lists and his sets
        .route("/api/tonight", write_route(&st, writes::review::tonight))
        .route("/api/review/rate", write_route(&st, writes::review::rate))
        .route("/api/set-row", write_route(&st, writes::review::set_row))
        .route(
            "/api/review/reject-match",
            write_route(&st, writes::review::reject_match),
        )
        .route("/api/boards/{id}/items", write_route(&st, writes::boards::item))
        .route("/api/boards/{id}/bridge", write_route(&st, writes::boards::bridge))
        .route(
            "/api/workingsets/{id}/versions",
            write_route(&st, writes::workingsets::version),
        )
        .route(
            "/api/workingsets/{id}/lock",
            write_route(&st, writes::workingsets::lock),
        )
        .route(
            "/api/workingsets/{id}/try",
            write_route(&st, writes::workingsets::try_add),
        )
        .route(
            "/api/workingsets/{id}/try/{id2}",
            patch_route(&st, writes::workingsets::try_patch),
        )
        .route(
            "/api/workingsets/{id}/bits/{id2}/punchup",
            write_route(&st, writes::workingsets::punchup),
        )
        .route(
            "/api/jokes/{id}/punchup",
            write_route(&st, writes::workingsets::joke_punchup),
        )
        // Film it: "Make it a rant" and "Pick a hook" on one version of a bit
        .route("/api/jokes/{id}/rant", write_route(&st, writes::film::rant))
        .route("/api/jokes/{id}/hook", write_route(&st, writes::film::hook))
        // T10 group 4: the voice profile's learn, the "not a joke?" answers (the settings PATCH and the profile PUT
        // are on their read routes above)
        .route(
            "/api/profile/voice/learn",
            write_route(&st, writes::account::profile_learn),
        )
        .route(
            "/api/suggestions/not-a-joke/keep",
            write_route(&st, writes::account::keep),
        )
        .route(
            "/api/suggestions/not-a-joke/hide",
            write_route(&st, writes::account::hide),
        )
        // T10: the merge candidates, with the judge memoised on the generation
        .route("/api/merge/candidates", read_route(&st, routes::merge::candidates))
        // T10 group 6b: what was folded into a bit, and one merge record (`/api/merge/<id>` is a pattern: the static
        // routes above win, and any other id is the Python's "not found" answered here)
        .route("/api/merge/history", read_route(&st, routes::merge::history_route))
        .route("/api/merge/{id}", read_route(&st, routes::merge::one))
        // T10 group 5: the live socket and the hosted worker's routes
        .route("/api/live", live_route(&st))
        .route("/api/worker/claim", write_route(&st, writes::worker::claim))
        .route("/api/worker/jobs/{id}/result", write_route(&st, writes::worker::result))
        // T10 group 6a: the uploads (their bytes are rows in the database now), the inbox, the voice memos
        .route("/api/worker/jobs/{id}/file", read_route(&st, writes::uploads::file))
        .route("/api/upload", write_route(&st, writes::uploads::upload))
        .route("/api/inbox", read_route(&st, writes::uploads::inbox))
        .route("/api/memos", read_route(&st, routes::memos::memos))
        .route("/api/memos/{id}", read_route(&st, routes::memos::memo))
        .route("/api/memos/{id}/audio", read_route(&st, routes::memos::audio))
        .route("/api/worker/decisions", read_route(&st, writes::worker::decisions))
        .route("/api/worker/creative", read_route(&st, writes::worker::creative))
        .route(
            "/api/worker/profile/voice",
            finish(
                &st,
                get(guarded(&st, writes::worker::profile_get)).put(guarded_write(&st, writes::worker::profile_put)),
            ),
        )
        // T10 round 5, group A: the conversations (`/api/ask/chats*`)
        .route(
            "/api/ask/chats",
            finish(
                &st,
                get(guarded(&st, writes::chats::list)).post(guarded_write(&st, writes::chats::create)),
            ),
        )
        .route(
            "/api/ask/chats/{id}",
            finish(
                &st,
                get(guarded_write(&st, writes::chats::one))
                    .post(guarded_write(&st, writes::chats::one))
                    .patch(guarded_write(&st, writes::chats::one)),
            ),
        )
        // T10 group 6, the cheap half: the one-shot questions and the imports that only queue a job
        .route("/api/ask", write_route(&st, writes::ask::ask))
        .route("/api/riff", write_route(&st, writes::ask::riff))
        .route("/api/facts", write_route(&st, writes::ask::facts))
        .route("/api/import/youtube", write_route(&st, writes::ask::youtube))
        .route("/api/jobs/{id}/drop", write_route(&st, writes::ask::drop));
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
    // the live hub: one subscription, flushed every 150 ms
    tokio::spawn(st.live().clone().run(st.clone()));
    tokio::spawn(st.live().clone().flush_loop());
    // the vector sweeper, when there is a model to run and `PK_VEC_SWEEP_S` is not 0
    if let Some(every) = st.config().sweep_every
        && !matches!(st.config().embedder, embed::Choice::None | embed::Choice::Missing(_))
    {
        tokio::spawn(sweep::run(
            st.clone(),
            worker::Lease::embedder(sweep::me(), every),
            every,
        ));
    }
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
