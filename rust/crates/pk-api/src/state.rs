//! The one shared state: settings, the token verifier, the metrics, the HTTP client, the database connection and
//! the current [`Data`]. Cheap to clone (one `Arc`); nothing global and nothing mutable outside it.
//!
//! Freshness: every read asks `meta:state` for its generation first (one round trip) and waits for data at least
//! that new, so a GET answered here right after a write the Python answered sees that write (the read-after-write
//! gate of docs/ARCHITECTURE.md section 2). Serving an older generation to "callers who did not write" is not safe
//! here: the app refetches on the Python's `change` event (`app/src/lib/api.ts` `useApi`) and does not refetch
//! again, so a stale answer hides the change until the next one. What makes the wait short instead: one rebuild in
//! flight (latest wins), the rebuild only patches or reads the rows, and each route builds only the view or index it
//! needs, once per generation.

use std::sync::atomic::{AtomicU64, Ordering};
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
    /// The data and the ticket of the newest rebuild state read it is current for (see [`AppState::data`]).
    data: RwLock<Option<(Arc<Data>, u64)>>,
    /// How many rebuild state reads have started: the ticket of the next one.
    reads: AtomicU64,
    /// `ticket + 1` of the last rebuild that finished, succeeded or not (0: none yet).
    attempted: AtomicU64,
    /// How many generations have been built (patched or read whole) since start.
    builds: AtomicU64,
    rebuild: Arc<tokio::sync::Mutex<()>>,
    /// One CPU build (a view, an index) at a time, across generations.
    cpu: Arc<tokio::sync::Semaphore>,
    failed: Mutex<Option<Failed>>,
    draining: std::sync::atomic::AtomicBool,
    cloud: tokio::sync::Mutex<Option<CloudCache>>,
    transcripts: crate::tape::TranscriptCache,
    src_texts: crate::creative::SrcTexts,
    vectors: crate::creative::VectorCache,
    attached: crate::creative::Attached,
    stores: crate::creative::Stores,
    model: crate::embed::Model,
    /// The knowledge-base writes of this process run one at a time (`kb.lock` in the Python): a write validates
    /// against the current view, then appends; two of them interleaved could both pass validation against the same
    /// view. Across processes the database's unique `seq` index arbitrates (`writes`).
    kb_lock: tokio::sync::Mutex<()>,
    /// The creative-store writes of this process, one at a time (`cr.lock`); taken after `kb_lock` when both are
    /// needed, never before it.
    creative_lock: tokio::sync::Mutex<()>,
    /// The merge index last built, for placing a jot (see [`AppState::last_merge_index`]).
    last_merge_index: Mutex<Option<(Option<i64>, Arc<pk_creative::merge::Index>)>>,
}

/// The last cloud answered: `(rev, cap, include_hidden, when, body)` (`cloud_api._cache`, one entry, five minutes).
pub type CloudCache = (String, i64, bool, std::time::Instant, axum::body::Bytes);

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
/// An incremental patch gives up after this (then the full read has its own [`LOAD_TIMEOUT`]).
pub const PATCH_TIMEOUT: Duration = Duration::from_secs(5);
/// Connecting gives up after this.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A failed build for one state is not retried sooner than this.
pub const BUILD_RETRY: Duration = Duration::from_secs(5);

impl AppState {
    /// A state with no database yet (`connect` or the supervisor supplies it).
    pub fn new(config: ApiConfig, verifier: Option<Verifier>, metrics: ApiMetrics, http: reqwest::Client) -> Self {
        let model = crate::embed::Model::new(config.embedder.clone());
        Self(Arc::new(Inner {
            model,
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
            reads: AtomicU64::new(0),
            attempted: AtomicU64::new(0),
            builds: AtomicU64::new(0),
            rebuild: Arc::new(tokio::sync::Mutex::new(())),
            cpu: Arc::new(tokio::sync::Semaphore::new(1)),
            failed: Mutex::new(None),
            cloud: tokio::sync::Mutex::new(None),
            transcripts: crate::tape::TranscriptCache::default(),
            src_texts: crate::creative::SrcTexts::default(),
            vectors: crate::creative::VectorCache::default(),
            attached: crate::creative::Attached::default(),
            stores: crate::creative::Stores::default(),
            draining: std::sync::atomic::AtomicBool::new(false),
            kb_lock: tokio::sync::Mutex::new(()),
            creative_lock: tokio::sync::Mutex::new(()),
            last_merge_index: Mutex::new(None),
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

    /// Search's embedder, loaded on first use.
    pub fn model(&self) -> &crate::embed::Model {
        &self.0.model
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

    /// The shared database handle for a route's own query (never cloned into a new session).
    pub fn store_handle(&self) -> Option<Arc<Store>> {
        self.store()
    }

    /// The transcripts fetched so far (by `srcfile` path and hash), shared by every generation.
    pub fn transcripts(&self) -> &crate::tape::TranscriptCache {
        &self.0.transcripts
    }

    /// The `srcfile` texts the creative layer read so far (by path and hash), shared by every generation.
    pub fn src_texts(&self) -> &crate::creative::SrcTexts {
        &self.0.src_texts
    }

    /// The notebook's vectors, as last parsed from the `vectors/` rows.
    pub fn vectors(&self) -> &crate::creative::VectorCache {
        &self.0.vectors
    }

    /// The merges store's attached notes, as last replayed (`desk_filter`).
    pub fn attached(&self) -> &crate::creative::Attached {
        &self.0.attached
    }

    /// The creative stores the reads replay, each with its last replay.
    pub fn stores(&self) -> &crate::creative::Stores {
        &self.0.stores
    }

    /// The one-at-a-time lock of this process's knowledge-base writes (`kb.lock`).
    pub fn kb_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.0.kb_lock
    }

    /// The one-at-a-time lock of this process's creative-store writes (`cr.lock`); after [`AppState::kb_lock`] when
    /// both are held.
    pub fn creative_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.0.creative_lock
    }

    /// The court-material rule (`refuse_legal`).
    pub fn legal(&self) -> &pk_core::legal::Legal {
        &self.0.config.legal
    }

    /// The cloud cache; held across a build, so two misses build once.
    pub fn cloud_cache(&self) -> &tokio::sync::Mutex<Option<CloudCache>> {
        &self.0.cloud
    }

    fn cached(&self) -> Option<Arc<Data>> {
        self.current().map(|(d, _)| d)
    }

    /// The data and the ticket of the newest rebuild state read it is known to be current for.
    fn current(&self) -> Option<(Arc<Data>, u64)> {
        self.0.data.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn set_current(&self, d: &Arc<Data>, ticket: u64) {
        *self.0.data.write().unwrap_or_else(PoisonError::into_inner) = Some((Arc::clone(d), ticket));
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

    /// The Python's booting answer (`if not self.kbs: return self.booting(path)`) for a route that reads no
    /// generation (the creative stores only): the 503 before the first load, else nothing to wait for.
    pub fn require_loaded(&self) -> Result<(), ApiError> {
        if self.cached().is_some() {
            Ok(())
        } else {
            Err(self.booting_error())
        }
    }

    fn booting_error(&self) -> ApiError {
        let msg = self.boot(|b| b.error.clone()).unwrap_or_else(|| NOT_LOADED.to_string());
        ApiError::unavailable(msg)
    }

    /// The data as it is now, without asking the database: for a caller with no stake in freshness (a signed-out
    /// `/api/health`, which is what the kubelet's probes are). `None` before the first load.
    pub fn data_now(&self) -> Option<Arc<Data>> {
        self.cached()
    }

    /// How many generations have been built since start (the coalescing test reads it).
    pub fn builds(&self) -> u64 {
        self.0.builds.load(Ordering::Relaxed)
    }

    /// How many rebuilds have started (each reads `meta:state` once under the lock; the coalescing test reads it).
    pub fn rebuild_reads(&self) -> u64 {
        self.0.reads.load(Ordering::SeqCst)
    }

    /// The bound on CPU builds of views and indexes (one at a time: the pod has one CPU).
    pub fn cpu(&self) -> Arc<tokio::sync::Semaphore> {
        Arc::clone(&self.0.cpu)
    }

    /// The current data: at least as new as the `meta:state` this request reads first (read-after-write, docs/
    /// ARCHITECTURE.md section 2: the app refetches after a `change` event and must see it). Before the first load:
    /// the 503 the Python answers while booting. If the database stops answering after a load, the last data keeps
    /// being served.
    ///
    /// Rebuilds are coalesced: one runs at a time (in its own task, so a caller that hangs up does not cancel it);
    /// every request that read its state before that rebuild read its own is answered by it, and the requests that
    /// queued meanwhile are answered by the next one, which reads the newest state (one in flight, latest wins). The
    /// rebuild itself is cheap: a patch or a read of the rows; the views and the index are built by the routes that
    /// need them ([`Data::view`], [`Data::index`]).
    pub async fn data(&self) -> Result<Arc<Data>, ApiError> {
        self.data_after(|| ()).await
    }

    /// The data a write validates against: [`AppState::data`], then proven current against `meta:state` read again
    /// (one more round trip): a write never validates against the last data served while the database is unreachable
    /// or a rebuild failed (the Python's `full_view` raises there). A generation that moved in between (another
    /// writer) is read again, a few times.
    pub async fn data_for_write(&self) -> Result<Arc<Data>, ApiError> {
        let store = self
            .store()
            .ok_or_else(|| ApiError::internal("ConnectionError", "the database is not connected"))?;
        // the generation as of this write's start: the data must be at least that new (another writer may move it
        // on while the data is made ready; that is fine, the builder validates against whatever is newest)
        let Some(st) = self.read_state(&store).await else {
            return Err(ApiError::unavailable(
                "SurrealDB did not answer in time; try again in a moment",
            ));
        };
        if st.loading {
            return Err(ApiError::unavailable(
                "the database is reloading; try again in a few seconds",
            ));
        }
        let d = self.data().await?;
        let fresh = match (d.state.generation, st.generation) {
            (Some(have), Some(want)) => have >= want,
            (None, None) => true,
            _ => false,
        };
        if !fresh || d.state.loading {
            return Err(ApiError::unavailable(
                "the data could not be refreshed for this write; try again in a moment",
            ));
        }
        Ok(d)
    }

    /// The merge index last built, when it is at most `within` generations behind `generation`
    /// (`merge_api.after_idea` places a jot on the corpus "as it was a moment ago": the Python's last built corpus,
    /// refreshed by its reads; a jot adds no units, so a notebook a few decisions old places it the same).
    pub fn last_merge_index(&self, generation: Option<i64>, within: i64) -> Option<Arc<pk_creative::merge::Index>> {
        let last = self.0.last_merge_index.lock().unwrap_or_else(PoisonError::into_inner);
        let (built_at, ix) = last.as_ref()?;
        let fresh_enough = match (built_at, generation) {
            (Some(b), Some(g)) => g - b <= within,
            _ => false,
        };
        fresh_enough.then(|| Arc::clone(ix))
    }

    /// Keep an index as the last built, with the generation it was built for.
    pub fn set_last_merge_index(&self, generation: Option<i64>, ix: &Arc<pk_creative::merge::Index>) {
        *self.0.last_merge_index.lock().unwrap_or_else(PoisonError::into_inner) = Some((generation, Arc::clone(ix)));
    }

    /// [`AppState::data`], with `work` started (in its own task) right after this request's `meta:state` read, so
    /// whatever `work` reads from the database is at least as new as that state, and it runs while the data is made
    /// ready. An error here returns at once; the task is not waited for (it finishes on its own and is dropped).
    pub async fn data_and<T: Send + 'static>(
        &self,
        work: impl Future<Output = T> + Send + 'static,
    ) -> Result<(Arc<Data>, tokio::task::JoinHandle<T>), ApiError> {
        let mut work = Some(work);
        let mut handle = None;
        let d = self.data_after(|| handle = work.take().map(tokio::spawn)).await?;
        let handle = match (handle, work) {
            (Some(h), _) => h,
            (None, Some(w)) => tokio::spawn(w),
            (None, None) => {
                return Err(ApiError::internal(
                    "StateError",
                    "the work was neither started nor kept",
                ));
            }
        };
        Ok((d, handle))
    }

    /// [`AppState::data`], calling `on_state` once, right after the `meta:state` read (or where it would have been).
    async fn data_after(&self, on_state: impl FnOnce()) -> Result<Arc<Data>, ApiError> {
        let Some(store) = self.store() else {
            on_state();
            return self.cached().ok_or_else(|| self.booting_error());
        };
        let st = self.read_state(&store).await;
        on_state();
        let Some(st) = st else {
            return self.cached().ok_or_else(|| self.booting_error());
        };
        // every rebuild state read that starts from here on sees at least what this one saw
        let mine = self.0.reads.load(Ordering::SeqCst);
        if !st.has_bundle {
            self.boot(|b| {
                b.no_bundle = true;
                b.loaded = false;
                b.error = Some(NOT_LOADED.into());
            });
            return Err(ApiError::unavailable(NOT_LOADED));
        }
        // `loading`: a reload is running and the rows are half written; the last data is served until it ends
        // (as before this rebuild scheme, and as the Python does)
        if let Some((d, ticket)) = self.current()
            && (d.state.key() == st.key() || st.loading || ticket >= mine)
        {
            return Ok(d);
        }
        let guard = Arc::clone(&self.0.rebuild).lock_owned().await;
        if let Some((d, ticket)) = self.current()
            && ticket >= mine
        {
            return Ok(d); // the rebuild this request waited for read the state after this request did
        }
        if self.0.attempted.load(Ordering::SeqCst) > mine {
            // that rebuild read a newer state than this request did and failed: do not try again for every waiter
            return self.cached().ok_or_else(|| self.booting_error());
        }
        let me = self.clone();
        let task = tokio::spawn(async move {
            let ticket = me.0.reads.fetch_add(1, Ordering::SeqCst);
            let r = me.rebuild(&store, ticket).await;
            // recorded before the lock is released, success or not (ticket + 1: 0 means no attempt yet)
            me.0.attempted.fetch_max(ticket + 1, Ordering::SeqCst);
            drop(guard);
            r
        });
        match task.await {
            Ok(r) => r,
            // a panic in the rebuild is a bug: a 500, and the last data stays
            Err(e) => Err(ApiError::internal("JoinError", e)),
        }
    }

    /// One rebuild, under the rebuild lock: read the newest state, then patch or read the rows for it.
    /// `ticket` was taken just before this state read started.
    async fn rebuild(&self, store: &Store, ticket: u64) -> Result<Arc<Data>, ApiError> {
        let Some(st) = self.read_state(store).await else {
            return self.cached().ok_or_else(|| self.booting_error());
        };
        if let Some(d) = self.cached()
            && (d.state.key() == st.key() || st.loading)
        {
            // nothing moved since that data was built: it is current as of this read too (tickets only grow under
            // the lock). While a reload runs the last data is served on purpose, as above.
            self.set_current(&d, ticket);
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
        let owner = self.0.config.owner_id.clone();
        let prev = self.cached();
        let read = async {
            // only decisions appended since the last generation: patch it (a few ms of reads, not the whole graph)
            if let Some(prev) = prev {
                match tokio::time::timeout(PATCH_TIMEOUT, snapshot::patch(store, prev, st.clone(), &owner)).await {
                    Ok(Ok(Some(d))) => return Ok((d, true)),
                    Ok(Ok(None)) => {}
                    Ok(Err(e)) => tracing::warn!(error = %e, "snapshot patch failed; reading everything"),
                    Err(_) => tracing::warn!("snapshot patch timed out; reading everything"),
                }
            }
            snapshot::load(store, st.clone(), &owner, self.cpu())
                .await
                .map(|d| (d, false))
        };
        let loaded = match tokio::time::timeout(LOAD_TIMEOUT, read).await {
            Ok(r) => r,
            Err(_) => Err(pk_store::StoreError::mapping("the snapshot read timed out")),
        };
        self.0.metrics.db_duration.observe(t.elapsed().as_secs_f64());
        match loaded {
            Ok((d, patched)) => {
                let d = Arc::new(d);
                self.set_current(&d, ticket);
                self.0.builds.fetch_add(1, Ordering::Relaxed);
                self.boot(|b| {
                    b.loaded = true;
                    b.no_bundle = false;
                    b.error = None;
                });
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                let fetch_ms = d.fetch_s * 1000.0;
                tracing::info!(rev = %d.rev(), decisions = d.decisions().len(), patched, fetch_ms, ms, "snapshot built");
                Ok(d)
            }
            Err(e) => {
                *self.0.failed.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some((st.key(), std::time::Instant::now()));
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
