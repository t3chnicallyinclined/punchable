//! The one error a knowledge-base write can refuse with: a message for the comic and an HTTP status, exactly like
//! the Python `DecisionError(msg, status)`.

/// A refused decision: `msg` is shown to him as written; `status` is the HTTP status the API answers with.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{msg}")]
pub struct KbError {
    /// The HTTP status: 400 (bad request), 404 (unknown node or edge), 409 (already exists / nothing to undo).
    pub status: u16,
    /// The plain-words reason.
    pub msg: String,
}

impl KbError {
    /// A 400.
    pub fn bad(msg: impl Into<String>) -> Self {
        Self {
            status: 400,
            msg: msg.into(),
        }
    }

    /// Any status.
    pub fn with_status(status: u16, msg: impl Into<String>) -> Self {
        Self {
            status,
            msg: msg.into(),
        }
    }
}

/// `Result` with [`KbError`].
pub type KbResult<T> = Result<T, KbError>;
