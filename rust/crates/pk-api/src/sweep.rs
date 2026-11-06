//! The vector sweeper (`search_api.Sweeper`, `pending`, `sweep`): every typed idea (a joke with no unit), every
//! taped moment and every unit without a vector gets its bge-small vector, with the sha256 of the text it was made
//! from beside it, so an edited text is embedded again and an unchanged one never is. A pass `PK_VEC_SWEEP_S`
//! (60 s) after start and then every `PK_VEC_SWEEP_S`; the model is loaded on the first pass that has something to
//! embed, so a pod with nothing pending never holds it.
//!
//! One owner: two API processes serve during T10 and both run a sweeper, so a pass takes the `lease:embedder` row
//! first (`pk_store::Store::lease_acquire`) and renews it before every batch; the other process finds it held and
//! skips its pass, and a process whose lease lapsed mid-pass stops. A process with no model to run gives the lease
//! up and stops sweeping for good, so the other one embeds. Never two embedders over one database.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use pk_store::{RecordId, Store};

use crate::state::AppState;
use crate::worker::Lease;

/// Texts embedded per model run (`BATCH`).
pub const BATCH: usize = 16;
/// The default `PK_VEC_SWEEP_S`.
pub const DEFAULT_EVERY: Duration = Duration::from_secs(60);

/// What one pass embedded, per table.
pub type Done = BTreeMap<&'static str, usize>;

/// Why a pass did not finish.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SweepError {
    /// No embedder here (none configured, or the model did not load): the lease was given up and the sweeper
    /// stops for good.
    #[error("no model here: nothing embedded; the lease was released")]
    NoModel,
    /// The lease lapsed between two batches: another process may be embedding.
    #[error("the embedder lease lapsed mid-pass; stopped")]
    Lapsed,
    /// The database or the model failed; the next pass tries again.
    #[error("{0}")]
    Failed(String),
}

/// `Sweeper.status()`: how many passes ran, the last pass's counts, the last error, whether the lease is held.
#[derive(Debug, Default)]
pub struct Status {
    runs: AtomicU64,
    held: AtomicBool,
    last: Mutex<Option<Done>>,
    error: Mutex<Option<String>>,
}

impl Status {
    /// Passes attempted (a pass that found the lease held counts, as the Python's does).
    pub fn runs(&self) -> u64 {
        self.runs.load(Ordering::SeqCst)
    }

    /// Whether this process held the lease on its last pass.
    pub fn held(&self) -> bool {
        self.held.load(Ordering::SeqCst)
    }

    /// The last pass's counts.
    pub fn last(&self) -> Option<Done> {
        self.last.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The last pass's error, if it failed.
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn record(&self, r: &Result<Option<Done>, SweepError>) {
        self.runs.fetch_add(1, Ordering::SeqCst);
        match r {
            Ok(Some(done)) => {
                self.held.store(true, Ordering::SeqCst);
                *self.last.lock().unwrap_or_else(PoisonError::into_inner) = Some(done.clone());
                *self.error.lock().unwrap_or_else(PoisonError::into_inner) = None;
            }
            Ok(None) => self.held.store(false, Ordering::SeqCst),
            Err(e) => {
                self.held.store(false, Ordering::SeqCst);
                *self.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(e.to_string());
            }
        }
    }
}

/// Who this process is to the lease: `pk-api:<host>:<pid>`.
pub fn me() -> String {
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "local".to_string());
    format!("pk-api:{host}:{}", std::process::id())
}

/// `Sweeper._loop`: a pass every `every`, the first one `every` after start. Runs until the process ends, or until
/// a pass finds no model to run (then the other process's sweeper has the lease).
pub async fn run(st: AppState, lease: Lease, every: Duration) {
    loop {
        tokio::time::sleep(every).await;
        let r = once(&st, &lease).await;
        st.sweep().record(&r);
        match &r {
            Ok(Some(done)) if done.values().sum::<usize>() > 0 => tracing::info!(?done, "vectors: embedded"),
            Ok(Some(_)) => tracing::debug!("vectors: nothing pending"),
            Ok(None) => tracing::debug!("vectors: the lease is held elsewhere; this pass skipped"),
            Err(SweepError::NoModel) => {
                tracing::warn!("vectors: {}; the sweeper stops", SweepError::NoModel);
                return;
            }
            Err(e) => tracing::warn!("vectors: sweep failed ({e})"),
        }
    }
}

fn failed(e: impl std::fmt::Display) -> SweepError {
    SweepError::Failed(e.to_string())
}

/// The typed ideas of the visible view (`v.jokes` with no unit and a summary with words), as records.
async fn typed_ideas(st: &AppState) -> Result<Option<Vec<RecordId>>, SweepError> {
    let d = st.data().await.map_err(|e| failed(e.msg))?;
    if d.state.loading {
        return Ok(None);
    }
    let v = d.view(false).await.map_err(|e| failed(e.msg))?;
    Ok(Some(
        v.jokes
            .iter()
            .filter(|j| !v.joke_units.contains_key(*j))
            .filter(|j| {
                v.nodes
                    .get(*j)
                    .and_then(|n| n.get("summary"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|s| !pk_core::py::strip(s).is_empty())
            })
            .filter_map(|j| pk_store::ids::parse_node_rid(j).ok())
            .collect(),
    ))
}

/// `Sweeper.once`: take the lease, embed and store every pending row, batch by batch with the lease renewed before
/// each. `Ok(None)` when another process holds the lease; the counts per table otherwise. Nothing while a reload
/// runs (`pending` reads `meta:state.loading`).
pub async fn once(st: &AppState, lease: &Lease) -> Result<Option<Done>, SweepError> {
    let store: std::sync::Arc<Store> = st
        .store_handle()
        .ok_or_else(|| failed("the database is not connected"))?;
    if !lease.acquire(&store).await.map_err(failed)? {
        return Ok(None);
    }
    let Some(jokes) = typed_ideas(st).await? else {
        return Ok(Some(Done::new()));
    };
    let mut done = Done::new();
    for (table, rows) in store.vectors_pending(jokes).await.map_err(failed)? {
        let mut n = 0;
        for chunk in rows.chunks(BATCH) {
            if !lease.acquire(&store).await.map_err(failed)? {
                return Err(SweepError::Lapsed);
            }
            let Some(e) = st.model().get().await else {
                if let Err(e) = lease.release(&store).await {
                    tracing::warn!("vectors: the lease could not be released: {e}");
                }
                return Err(SweepError::NoModel);
            };
            let texts: Vec<String> = chunk.iter().map(|(_, t)| t.clone()).collect();
            let vecs = st.model().vectors(e, texts, false).await.map_err(failed)?;
            let batch: Vec<(RecordId, String, Vec<f32>)> = chunk
                .iter()
                .zip(vecs)
                .map(|((id, text), vec)| (id.clone(), text.clone(), vec))
                .collect();
            store.vectors_set(table, &batch).await.map_err(failed)?;
            n += chunk.len();
        }
        done.insert(table, n);
    }
    Ok(Some(done))
}
