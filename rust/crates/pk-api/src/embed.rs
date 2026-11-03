//! The query side of search's vectors: the embedder this process can run, loaded on first use (`search_api.py`
//! `make_embedder`, which the hosted Python runs at start; here the first search loads it, so a pod that never
//! searches never holds the model). `PK_EMBEDDER=hash` is the word-hash embedder (tests, a PC without the model,
//! never the server); otherwise bge-small from `PK_BGE_DIR` or `<PK_CACHE_DIR>/models/bge-small-en-v1.5`; otherwise
//! none, and search runs on words alone. A model that does not load or fails its warm-up is none too, decided once,
//! as the Python decides it once.
//!
//! The load runs in its own task, so a search the client abandons (search-as-you-type aborts the previous fetch) never
//! cancels it and never starts a second one. Model runs go one at a time through a semaphore taken before the
//! blocking pool, so a request dropped while it waits never reaches the model. Intra-op threads are
//! `engine/embed.py`'s rule, one per CPU up to four (`pk_embed::Bge::load`); Rust counts the CPUs the cgroup allows,
//! so the one-CPU pod gets one thread where the Python, counting the host's, asks for four.

use std::path::PathBuf;
use std::sync::Arc;

use pk_core::Env;
use pk_embed::{EmbedError, Embedder};
use tokio::sync::{OnceCell, Semaphore};

/// Which embedder the settings name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// No model here: words only.
    None,
    /// `PK_BGE_DIR` is set but holds no model (a misbuilt image): words only, said loudly.
    Missing(PathBuf),
    /// `PK_EMBEDDER=hash`.
    Hash,
    /// bge-small from this folder.
    Bge(PathBuf),
}

impl Choice {
    /// `make_embedder`'s order: `PK_EMBEDDER=hash`, then the model folder, then none.
    pub fn from_env(env: &Env) -> Result<Self, pk_core::ConfigError> {
        if env.opt("PK_EMBEDDER") == Some("hash") {
            return Ok(Self::Hash);
        }
        let cfg = pk_embed::EmbedConfig::from_env(env)?;
        Ok(match (cfg.model_dir(), cfg.bge_dir) {
            (Some(dir), _) => Self::Bge(dir),
            (None, Some(named)) => Self::Missing(named),
            (None, None) => Self::None,
        })
    }
}

struct Inner {
    choice: Choice,
    cell: OnceCell<Option<Arc<dyn Embedder>>>,
    /// One model run at a time.
    runs: Arc<Semaphore>,
}

/// The lazily loaded embedder, shared by every request (cheap to clone).
#[derive(Clone)]
pub struct Model(Arc<Inner>);

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model")
            .field("choice", &self.0.choice)
            .field("loaded", &self.0.cell.initialized())
            .finish()
    }
}

/// Why a vector could not be made.
#[derive(Debug, thiserror::Error)]
pub enum VectorError {
    /// The model refused the text.
    #[error(transparent)]
    Embed(#[from] EmbedError),
    /// The blocking task did not finish (a panic, or the runtime shutting down).
    #[error("the embedding task stopped: {0}")]
    Join(#[from] tokio::task::JoinError),
    /// The semaphore was closed (never: it lives as long as the model).
    #[error("the model is shut")]
    Closed(#[from] tokio::sync::AcquireError),
    /// The model answered with no vector.
    #[error("the model gave no vector")]
    Empty,
}

impl Model {
    /// Not loaded yet.
    pub fn new(choice: Choice) -> Self {
        Self(Arc::new(Inner {
            choice,
            cell: OnceCell::new(),
            runs: Arc::new(Semaphore::new(1)),
        }))
    }

    /// Whether it is in memory now (or was decided to be none).
    pub fn is_resolved(&self) -> bool {
        self.0.cell.initialized()
    }

    /// The embedder, loading it in its own task the first time; `None` = words only.
    pub async fn get(&self) -> Option<Arc<dyn Embedder>> {
        if let Some(e) = self.0.cell.get() {
            return e.clone();
        }
        let inner = Arc::clone(&self.0);
        // the caller may be dropped while it waits; the load it started goes on for the next caller
        tokio::spawn(async move { inner.cell.get_or_init(|| resolve(inner.choice.clone())).await.clone() })
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "vectors: the model's load task stopped; words only for this request");
                None
            })
    }

    /// One text's vector (`query` = the search prefix): a turn at the model, then the blocking pool.
    pub async fn vector(&self, e: Arc<dyn Embedder>, text: String, query: bool) -> Result<Vec<f32>, VectorError> {
        let turn = Arc::clone(&self.0.runs).acquire_owned().await?;
        tokio::task::spawn_blocking(move || {
            let _turn = turn;
            e.embed(&[text.as_str()], query)
        })
        .await??
        .into_iter()
        .next()
        .ok_or(VectorError::Empty)
    }
}

async fn resolve(choice: Choice) -> Option<Arc<dyn Embedder>> {
    match choice {
        Choice::None => {
            tracing::info!("vectors: no bge-small model here (PK_BGE_DIR); search runs on words alone");
            None
        }
        Choice::Missing(dir) => {
            tracing::warn!(dir = %dir.display(), "vectors: PK_BGE_DIR holds no model; search runs on words alone");
            None
        }
        Choice::Hash => {
            tracing::info!("vectors: PK_EMBEDDER=hash (word-hash vectors: tests and dev only)");
            Some(Arc::new(pk_embed::Hash) as Arc<dyn Embedder>)
        }
        Choice::Bge(dir) => load(dir).await,
    }
}

/// bge-small from `dir`, warmed up the way `make_embedder` warms it (one embed), or none with the reason logged.
async fn load(dir: PathBuf) -> Option<Arc<dyn Embedder>> {
    let t = std::time::Instant::now();
    let shown = dir.display().to_string();
    let loaded = tokio::task::spawn_blocking(move || {
        let bge = pk_embed::Bge::load(&dir)?;
        bge.embed(&["warm up"], false)?;
        Ok::<_, EmbedError>(bge)
    })
    .await;
    match loaded {
        Ok(Ok(bge)) => {
            tracing::info!(dir = %shown, ms = t.elapsed().as_millis(), "vectors: bge-small-en-v1.5 loaded");
            Some(Arc::new(bge))
        }
        Ok(Err(e)) => {
            tracing::warn!(dir = %shown, error = %e, "vectors: the model did not load; search runs on words alone");
            None
        }
        Err(e) => {
            tracing::warn!(dir = %shown, error = %e, "vectors: loading the model stopped; search runs on words alone");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choice_follows_make_embedder() {
        let env = |pairs: Vec<(&str, &str)>| Env::from_map(pairs);
        assert_eq!(Choice::from_env(&env(vec![])).unwrap(), Choice::None);
        assert_eq!(
            Choice::from_env(&env(vec![("PK_EMBEDDER", "hash"), ("PK_BGE_DIR", "/nowhere")])).unwrap(),
            Choice::Hash
        );
        assert_eq!(
            Choice::from_env(&env(vec![("PK_BGE_DIR", "/nowhere")])).unwrap(),
            Choice::Missing(PathBuf::from("/nowhere"))
        );
    }

    #[tokio::test]
    async fn no_model_is_words_only_and_decided_once() {
        let m = Model::new(Choice::None);
        assert!(m.get().await.is_none());
        assert!(m.is_resolved());
        let broken = tempfile::tempdir().expect("tempdir");
        let m = Model::new(Choice::Bge(broken.path().to_path_buf()));
        assert!(m.get().await.is_none(), "a folder without the model is words only");
    }

    #[tokio::test]
    async fn a_dropped_caller_does_not_cancel_the_load() {
        let m = Model::new(Choice::Hash);
        let first = m.clone();
        let h = tokio::spawn(async move { first.get().await });
        h.abort();
        let _ = h.await;
        assert!(m.get().await.is_some());
        assert!(m.is_resolved());
    }

    #[tokio::test]
    async fn the_hash_embedder_gives_unit_vectors() {
        let m = Model::new(Choice::Hash);
        let e = m.get().await.expect("hash");
        let v = m.vector(e, "parking meter".into(), true).await.expect("vector");
        assert_eq!(v.len(), pk_embed::DIM);
        let n: f32 = v.iter().map(|x| x * x).sum();
        assert!((n - 1.0).abs() < 1e-5);
    }
}
