//! The one error the job knows: a message that ends up as `backup: FAIL <message>` on stderr.

/// A failed step. Carries the text only; [`crate::redact::redact`] cleans it before it is printed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct BackupError(pub String);

/// The result of every step.
pub type Result<T> = std::result::Result<T, BackupError>;

/// `Err(BackupError(msg))`, for the places that read as `raise BackupError(msg)` in the Python.
pub fn fail<T>(msg: impl Into<String>) -> Result<T> {
    Err(BackupError(msg.into()))
}
