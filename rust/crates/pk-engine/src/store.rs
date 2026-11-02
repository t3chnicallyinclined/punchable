//! The seam to the store: what the engine needs from a database, as an async trait the caller implements. The engine
//! never opens a connection; `pk-store` implements this over the official `surrealdb` client (its `engine` module),
//! and the tests implement it in memory.
//!
//! Upstream calls its store inside each function over blocking `ureq` (`upstream-ops/src/store.rs`); here the store is
//! async (tokio, the official crate), and the engine awaits it without blocking a runtime thread: the engine's own
//! work after a read is a few milliseconds of pure ranking over at most a few hundred candidates.

use std::future::Future;

use crate::pack::Hit;

/// What one read asks of the store.
#[derive(Debug, Clone, Copy)]
pub struct ReadQuery<'a> {
    /// The question, trimmed and non-empty.
    pub question: &'a str,
    /// The question's bge vector (the query form, `pk-embed`), when the caller has one; `None` reads words only.
    pub vector: Option<&'a [f32]>,
    /// How deep each leg reads.
    pub depth: usize,
}

/// What one read returns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Read {
    /// The union of the legs, in the store's given order, each with its leg ranks.
    pub hits: Vec<Hit>,
    /// How many records the read could have reached, when the store counts it on the read path; 0 when it does not
    /// (`pk-store` counts it apart, once, because the count is three full scans).
    pub corpus_total: usize,
}

/// The store the engine reads through.
pub trait EngineStore: Sync {
    /// The store's error.
    type Error: std::error::Error + Send + Sync + 'static;

    /// The full-text and vector legs over his material, merged: every record either leg returned, at most `depth`
    /// per leg, with its rank in each.
    fn read(&self, q: ReadQuery<'_>) -> impl Future<Output = Result<Read, Self::Error>> + Send;

    /// Live lessons matching the question, best first, each as the record that is its evidence with the lesson on
    /// the hit; a lesson whose evidence is gone is left out.
    fn lessons(&self, question: &str, k: usize) -> impl Future<Output = Result<Vec<Hit>, Self::Error>> + Send;
}
