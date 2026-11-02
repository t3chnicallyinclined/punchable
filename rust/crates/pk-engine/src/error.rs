//! What can go wrong in the engine, typed: a refused config, a refused lesson or card (the evidence rule speaks in
//! words), a store fault passed through from the caller's store, an empty question.

/// Every error the engine returns.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// A ranking config that is out of range or names an unknown field; every problem at once.
    #[error("rank config refused: {0}")]
    Config(String),
    /// A config or a model reply that is not the JSON it must be. The parser's error is in the message (not also a
    /// `source`, so a chained report says it once).
    #[error("{what}: {error}")]
    Json {
        /// What was being read.
        what: &'static str,
        /// The parser's error.
        error: serde_json::Error,
    },
    /// The evidence rule or a card rule refused the write; the text says why, in plain words.
    #[error("{0}")]
    Refused(String),
    /// The question was empty after trimming.
    #[error("the question is empty")]
    EmptyQuestion,
    /// The store the caller handed in failed (its error in the message, once).
    #[error("the store: {0}")]
    Store(Box<dyn std::error::Error + Send + Sync>),
}

/// The engine's result.
pub type EngineResult<T> = Result<T, EngineError>;

impl EngineError {
    /// A store fault from any error type.
    pub fn store(e: impl std::error::Error + Send + Sync + 'static) -> Self {
        EngineError::Store(Box::new(e))
    }

    pub(crate) fn json(what: &'static str) -> impl FnOnce(serde_json::Error) -> Self {
        move |error| EngineError::Json { what, error }
    }
}
