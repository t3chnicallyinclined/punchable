//! The one shared state: settings, the token verifier, the metrics, the HTTP client, the database connection and
//! the current [`Data`]. Cheap to clone (one `Arc`); nothing global and nothing mutable outside it.
//!
//! Freshness: every read asks `meta:state` for its generation first (one round trip) and rebuilds when it moved, so
//! a GET answered here right after a write the Python answered sees that write (the read-after-write gate of
//! docs/ARCHITECTURE.md section 2). One rebuild runs at a time; the others wait for it and read what it built.

use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use pk_obs::ApiMetrics;
use pk_store::Store;
use serde_json::{Value, json};

use crate::auth::Verifier;
use crate::config::ApiConfig;
use crate::error::ApiError;
use crate::snapshot::{self, Data};

/// The 503 text before any data is there (`api/entry.py`).
pub const NOT_LOADED: &str = "the data is not loaded yet (infra/ship.sh dbload)";

/// How the service is doing before it serves data (`api/entry.py` `STATE` / `boot_status`).
#[derive(Debug, Clone, Default)]
struct Boot {
    surreal: bool,
    loaded: bool,
    no_bundle: bool,
    error: Option<String>,
}

struct Inner {
    config: ApiConfig,
    verifier: Option<Verifier>,
    metrics: ApiMetrics,
    http: reqwest::Client,
    proxy_permits: Arc<tokio::sync::Semaphore>,
    db: RwLock<Option<Arc<Store>>>,
    boot: Mutex<Boot>,
    data: RwLock<Option<Arc<Data>>>,
    rebuild: tokio::sync::Mutex<()>,
    failed: Mutex<Option<Failed>>,
    draining: std::sync::atomic::AtomicBool,
}

/// The state a build last failed for, and when.
type Failed = ((Option<i64>, Value), std::time::Instant);

/// The shared state handed to every handler.
#[derive(Clone)]
pub struct AppState(Arc<Inner>);

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState").finish_non_exhaustive()
    }
}

/// How many requests may be in flight to the Python at once (T9's fallback).
pub const PROXY_CONCURRENCY: usize = 64;
/// The per-request `meta:state` read gives up after this (the readiness probe's own timeout is 2 s).
pub const STATE_TIMEOUT: Duration = Duration::from_millis(1500);
/// A snapshot read gives up after this.
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(30);
/// Connecting gives up after this.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A failed build for one state is not retried sooner than this.
pub const BUILD_RETRY: Duration = Duration::from_secs(5);

impl AppState {
    /// A state with no database yet (`connect` or the supervisor supplies it).
    pub fn new(config: ApiConfig, verifier: Option<Verifier>, metrics: ApiMetrics, http: reqwest::Client) -> Self {
        Self(Arc::new(Inner {
            config,
            verifier,
            metrics,
            http,
            proxy_permits: Arc::new(tokio::sync::Semaphore::new(PROXY_CONCURRENCY)),
            db: RwLock::new(None),
            boot: Mutex::new(Boot {
                error: Some("starting".into()),
                ..Boot::default()
            }),
            data: RwLock::new(None),
            rebuild: tokio::sync::Mutex::new(()),
            failed: Mutex::new(None),
            draining: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    /// SIGTERM arrived: readiness answers 503 from now on so the Service stops sending, while requests drain.
    pub fn start_draining(&self) {
        self.0.draining.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether the pod is shutting down.
    pub fn draining(&self) -> bool {
        self.0.draining.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The settings.
    pub fn config(&self) -> &ApiConfig {
        &self.0.config
    }

    /// The token check, when auth is on.
    pub fn verifier(&self) -> Option<&Verifier> {
        self.0.verifier.as_ref()
    }

    /// The metrics registry.
    pub fn metrics(&self) -> &ApiMetrics {
        &self.0.metrics
    }

    /// The outbound HTTP client (JWKS, the Python fallback).
    pub fn http(&self) -> &reqwest::Client {
        &self.0.http
    }

    /// The bound on concurrent fallback requests (a permit lives as long as the forwarded response body).
    pub fn proxy_permits(&self) -> Arc<tokio::sync::Semaphore> {
        Arc::clone(&self.0.proxy_permits)
    }

    fn boot<T>(&self, f: impl FnOnce(&mut Boot) -> T) -> T {
        f(&mut self.0.boot.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The one connection, shared. Never `Store::clone()` per request: a clone of the SDK client is a new session
    /// that signs in and selects the namespace again on its first query, about 17 ms here (measured 2026-11-02 on the
    /// owner's PC: 0.4 ms on the shared handle, 16-19 ms through a fresh clone), which made every read 10x slower.
    fn store(&self) -> Option<Arc<Store>> {
        self.0.db.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn cached(&self) -> Option<Arc<Data>> {
        self.0.data.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Use this connection (tests, and the supervisor once it connects).
    pub fn set_store(&self, s: Option<Store>) {
        let up = s.is_some();
        *self.0.db.write().unwrap_or_else(PoisonError::into_inner) = s.map(Arc::new);
        self.boot(|b| b.surreal = up);
    }

    /// `boot_status()`: `{ok, surreal, loaded, ready, error}`.
    pub fn boot_status(&self) -> Value {
        self.boot(|b| {
            json!({
                "ok": b.loaded, "surreal": b.surreal, "loaded": b.loaded,
                "ready": b.loaded || (b.surreal && b.no_bundle), "error": b.error,
            })
        })
    }

    /// Whether data has been served (the routes answer; before that, the boot status does).
    pub fn loaded(&self) -> bool {
        self.boot(|b| b.loaded)
    }

    fn booting_error(&self) -> ApiError {
        let msg = self.boot(|b| b.error.clone()).unwrap_or_else(|| NOT_LOADED.to_string());
        ApiError::unavailable(msg)
    }

    /// The current data, rebuilt first if the database moved. Before the first load: the 503 the Python answers
    /// while booting. If the database stops answering after a load, the last data keeps being served.
    pub async fn data(&self) -> Result<Arc<Data>, ApiError> {
        let Some(store) = self.store() else {
            return self.cached().ok_or_else(|| self.booting_error());
        };
        let Some(st) = self.read_state(&store).await else {
            return self.cached().ok_or_else(|| self.booting_error());
        };
        if !st.has_bundle {
            self.boot(|b| {
                b.no_bundle = true;
                b.loaded = false;
                b.error = Some(NOT_LOADED.into());
            });
            return Err(ApiError::unavailable(NOT_LOADED));
        }
        if let Some(d) = self.cached()
            && (d.state.key() == st.key() || st.loading)
        {
            return Ok(d);
        }
        let _g = self.0.rebuild.lock().await;
        // another request may have rebuilt while this one waited: read the state again rather than build for the
        // generation this request saw first
        let Some(st) = self.read_state(&store).await else {
            return self.cached().ok_or_else(|| self.booting_error());
        };
        if let Some(d) = self.cached()
            && (d.state.key() == st.key() || st.loading)
        {
            return Ok(d);
        }
        // a build that just failed for this state is not retried by every request (1 CPU): serve what there is
        let recent_failure = self
            .0
            .failed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|(k, at)| *k == st.key() && at.elapsed() < BUILD_RETRY);
        if recent_failure {
            return self.cached().ok_or_else(|| self.booting_error());
        }
        let t = std::time::Instant::now();
        let loaded = match tokio::time::timeout(
            LOAD_TIMEOUT,
            snapshot::load(&store, st.clone(), &self.0.config.owner_id),
        )
        .await
        {
            Ok(r) => r,
            Err(_) => Err(pk_store::StoreError::mapping("the snapshot read timed out")),
        };
        self.0.metrics.db_duration.observe(t.elapsed().as_secs_f64());
        if loaded.is_err() {
            *self.0.failed.lock().unwrap_or_else(PoisonError::into_inner) = Some((st.key(), std::time::Instant::now()));
        }
        match loaded {
            Ok(d) => {
                let d = Arc::new(d);
                *self.0.data.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&d));
                self.boot(|b| {
                    b.loaded = true;
                    b.no_bundle = false;
                    b.error = None;
                });
                let jokes = d.visible().view.jokes.len();
                let ms = t.elapsed().as_millis() as u64;
                tracing::info!(rev = %d.rev(), jokes, ms, "snapshot built");
                Ok(d)
            }
            Err(e) => {
                tracing::error!(error = %e, "snapshot build failed");
                pk_obs::crash::report(&e, Some("snapshot"), Some(500), &[]);
                self.cached().ok_or_else(|| ApiError::internal("StoreError", e))
            }
        }
    }

    /// `meta:state`, bounded: a database that does not answer within [`STATE_TIMEOUT`] counts as down for this
    /// request (the last data is served, or the boot 503).
    async fn read_state(&self, store: &Store) -> Option<snapshot::State> {
        let t = std::time::Instant::now();
        let r = tokio::time::timeout(STATE_TIMEOUT, snapshot::state(store)).await;
        self.0.metrics.db_duration.observe(t.elapsed().as_secs_f64());
        tracing::debug!(ms = t.elapsed().as_secs_f64() * 1000.0, "meta:state");
        match r {
            Ok(Ok(st)) => Some(st),
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "meta:state read failed");
                self.boot(|b| b.error = Some(format!("{e}")));
                None
            }
            Err(_) => {
                tracing::warn!("meta:state read timed out");
                self.boot(|b| b.error = Some("SurrealDB did not answer in time".into()));
                None
            }
        }
    }

    /// Keep the connection and the data current: connect (every 5 s until it works), then warm the data, then poll
    /// every 15 s so a new bundle is picked up without a request. A failed read drops the connection so the next
    /// round reconnects. Runs until the process ends.
    pub async fn supervise(self) {
        loop {
            if self.store().is_none() {
                let connect = tokio::time::timeout(CONNECT_TIMEOUT, Store::connect(self.0.config.store.clone()));
                match connect
                    .await
                    .unwrap_or_else(|_| Err(pk_store::StoreError::mapping("connect timed out")))
                {
                    Ok(s) => {
                        self.set_store(Some(s));
                        tracing::info!("connected to SurrealDB");
                    }
                    Err(e) => {
                        self.boot(|b| {
                            b.surreal = false;
                            b.error = Some(format!("{e}"));
                        });
                        tracing::warn!(error = %e, "SurrealDB connect failed");
                    }
                }
            }
            if let Some(store) = self.store()
                && self.read_state(&store).await.is_none()
            {
                self.set_store(None);
            } else if self.store().is_some() {
                let _ = self.data().await;
            }
            let wait = if self.loaded() { 15 } else { 5 };
            tokio::time::sleep(Duration::from_secs(wait)).await;
        }
    }
}
