//! The one error this crate returns: a metric that could not be registered or rendered, or a `tracing` subscriber
//! that was already installed. Crash reporting never errors: an unset or bad `SENTRY_DSN` means "off".

/// What can go wrong in `pk-obs`.
#[derive(Debug, thiserror::Error)]
pub enum ObsError {
    /// The `prometheus` crate refused a registration (a duplicate name, bad labels) or failed to encode.
    #[error("metrics: {0}")]
    Metrics(#[from] prometheus::Error),
    /// The global `tracing` subscriber could not be installed (one is already set).
    #[error("tracing: {0}")]
    Tracing(String),
}

/// `Result` with [`ObsError`].
pub type ObsResult<T> = Result<T, ObsError>;
