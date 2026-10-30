//! bge-small-en-v1.5 vectors for search and candidates, never a verdict: the port of `engine/embed.py`,
//! `db/embedder.py` and the `HashEmbedder` of `api-dev/search_api.py`.
//!
//! The same model and tokenizer files, CLS pooling (the first token), L2 normalisation, 384 dimensions, the query
//! prefix, the 2,000-character cap and length-sorted batches of eight, so a vector made here lands in the space of
//! the vectors already stored. [`Bge`] is the model over `ort` + `tokenizers`; [`Hash`] is the deterministic
//! word-hash embedder for tests and a PC without the model. Sync: one call embeds; the API wraps it in
//! `spawn_blocking`. The model files come from `tools/kb/fetch_bge.py` (hash-checked, fingerprinted) into
//! `PK_BGE_DIR` or `<PK_CACHE_DIR>/models/bge-small-en-v1.5`; [`EmbedConfig::model_dir`] finds them the way
//! `engine/config.bge_dir_override` does, over values a binary took from `pk_core::config::Env` (this crate never
//! reads the environment).

use std::path::{Path, PathBuf};

use pk_core::{ConfigError, Env};

mod bge;
mod hash;

pub use bge::Bge;
pub use hash::Hash;

/// The vector width of bge-small.
pub const DIM: usize = 384;
/// Characters kept of a text before tokenising: 512 tokens is about 2,000 characters of English, anything past that
/// is cut by the tokenizer anyway, and the cap keeps a batch's padding bounded by the texts that matter.
pub const MAX_CHARS: usize = 2000;
/// The tokenizer's truncation, in tokens including `[CLS]` and `[SEP]`.
pub const MAX_TOKENS: usize = 512;
/// Texts per model run; the batch is sorted by length first so padding stays small.
pub const BATCH: usize = 8;
/// What bge prepends to a query so it lands near the passages that answer it.
pub const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";
/// The model's name, as the API logs it and the model folder is named.
pub const MODEL_NAME: &str = "bge-small-en-v1.5";
/// The fixed public sentence `tools/kb/fetch_bge.py` embeds to check a downloaded model.
pub const FINGERPRINT_SENTENCE: &str = "The quick brown fox jumps over the lazy dog.";
/// The first six values of that sentence's passage vector, as `fetch_bge.py` publishes them (the values every cached
/// vector was made with).
pub const FINGERPRINT: [f32; 6] = [-0.10406, -0.01369, -0.0095, 0.10715, 0.0106, 0.00699];

/// Everything that can go wrong here.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    /// The folder has no `onnx/model.onnx` + `tokenizer.json`.
    #[error("no bge-small model in {}: run python tools/kb/fetch_bge.py", .0.display())]
    NoModel(PathBuf),
    /// The tokenizer refused the file or the text.
    #[error("tokenizer: {0}")]
    Tokenizer(#[from] tokenizers::Error),
    /// ONNX Runtime refused the model or the run.
    #[error("onnxruntime: {0}")]
    Ort(#[from] ort::Error),
    /// The model answered with something other than `[batch, tokens, 384]`.
    #[error("the model's output is not [batch, tokens, {DIM}]: {0}")]
    Shape(String),
    /// An earlier call panicked while holding the session.
    #[error("the embedder is unusable: an earlier call panicked")]
    Poisoned,
}

/// Texts in, unit vectors out. `query` prepends [`QUERY_PREFIX`] (the `db/embedder.py` contract); the hash
/// embedder ignores it, as the Python one does.
pub trait Embedder: Send + Sync {
    /// One vector per text, in the texts' order, each of length [`DIM`] and L2-normalised.
    fn embed(&self, texts: &[&str], query: bool) -> Result<Vec<Vec<f32>>, EmbedError>;
    /// What the API logs: `bge-small-en-v1.5` or `hash`.
    fn name(&self) -> &'static str;
}

/// The text the tokenizer sees for `text`: the prefix when `query`, a lone space for an empty text (the tokenizer
/// would otherwise give only `[CLS] [SEP]`), then the first [`MAX_CHARS`] characters. The prefix goes on before the
/// cut, as `db/embedder.py` does before `engine/embed.py` cuts.
pub fn prepare(text: &str, query: bool) -> String {
    let body = if text.is_empty() { " " } else { text };
    let prefix = if query { QUERY_PREFIX } else { "" };
    prefix.chars().chain(body.chars()).take(MAX_CHARS).collect()
}

/// Where the model may be, as the environment names it: `PK_BGE_DIR`, else `<PK_CACHE_DIR>/models/bge-small-en-v1.5`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EmbedConfig {
    /// `PK_BGE_DIR`.
    pub bge_dir: Option<PathBuf>,
    /// `PK_CACHE_DIR`.
    pub cache_dir: Option<PathBuf>,
}

impl EmbedConfig {
    /// From explicit parts.
    pub fn new(bge_dir: Option<PathBuf>, cache_dir: Option<PathBuf>) -> Self {
        Self { bge_dir, cache_dir }
    }

    /// From the environment the binary read (an empty value counts as unset, as in Python).
    pub fn from_env(env: &Env) -> Result<Self, ConfigError> {
        Ok(Self::new(env.path("PK_BGE_DIR"), env.path("PK_CACHE_DIR")))
    }

    /// The first of the two folders that has the model (`engine/config.bge_dir_override`), or `None`.
    pub fn model_dir(&self) -> Option<PathBuf> {
        self.bge_dir
            .iter()
            .cloned()
            .chain(self.cache_dir.as_ref().map(|c| c.join("models").join(MODEL_NAME)))
            .find(|d| has_model(d))
    }
}

fn has_model(dir: &Path) -> bool {
    dir.join("onnx").join("model.onnx").is_file() && dir.join("tokenizer.json").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_follows_the_python_order() {
        assert_eq!(prepare("", false), " ");
        assert_eq!(prepare("   ", false), "   ");
        assert_eq!(prepare("", true), format!("{QUERY_PREFIX} "));
        let long = "é".repeat(3000);
        let cut = prepare(&long, false);
        assert_eq!(cut.chars().count(), MAX_CHARS, "cut by characters, not bytes");
        let q = prepare(&long, true);
        assert_eq!(q.chars().count(), MAX_CHARS);
        assert!(q.starts_with(QUERY_PREFIX), "the prefix goes on before the cut");
    }

    #[test]
    fn the_model_folder_is_found_like_python() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bge = tmp.path().join("bge");
        let cache = tmp.path().join("cache");
        let both = EmbedConfig::new(Some(bge.clone()), Some(cache.clone()));
        assert_eq!(both.model_dir(), None);
        let put = |d: &Path| {
            std::fs::create_dir_all(d.join("onnx")).expect("mkdir");
            std::fs::write(d.join("onnx").join("model.onnx"), b"x").expect("write");
            std::fs::write(d.join("tokenizer.json"), b"{}").expect("write");
        };
        put(&cache.join("models").join(MODEL_NAME));
        assert_eq!(
            both.model_dir(),
            Some(cache.join("models").join(MODEL_NAME)),
            "PK_BGE_DIR without the files falls through to the cache"
        );
        let empty = EmbedConfig::from_env(&Env::from_map([("PK_BGE_DIR", "")])).expect("config");
        assert_eq!(empty, EmbedConfig::default(), "empty counts as unset");
        assert_eq!(empty.model_dir(), None);
        put(&bge);
        assert_eq!(both.model_dir(), Some(bge.clone()));
        let from_env =
            EmbedConfig::from_env(&Env::from_map([("PK_BGE_DIR", bge.to_str().expect("utf-8"))])).expect("config");
        assert_eq!(from_env.model_dir(), Some(bge));
    }

    #[test]
    fn the_embedders_are_objects_the_api_can_share() {
        fn shared<T: Send + Sync + 'static>() {}
        shared::<Bge>();
        shared::<Hash>();
        let e: Box<dyn Embedder> = Box::new(Hash);
        assert_eq!(e.name(), "hash");
    }
}
