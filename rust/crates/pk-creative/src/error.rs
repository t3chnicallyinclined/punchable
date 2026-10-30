//! The one error a creative store can answer with: what a reducer refused (the text the comic reads), what a
//! malformed event would have crashed the Python with, and where a replayed log stopped.

/// Why an event did not land.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CreativeError {
    /// The reducer refused the event (Python `ValueError`): the API answers 400 with this text, shown as written.
    #[error("{0}")]
    Refused(String),
    /// A malformed event (Python `KeyError` / `TypeError` / `AttributeError`, a 500 today): the message is the
    /// Python's, so the differential can compare it.
    #[error("{0}")]
    Faulted(String),
    /// A replayed log stopped at `index` (0-based): the line is not an event, or its reducer refused it.
    #[error("event {index}: {msg}")]
    Replay {
        /// Position in the log.
        index: usize,
        /// The reducer's or the parser's message.
        msg: String,
    },
}

impl CreativeError {
    /// A refusal with the message shown to him.
    pub fn refused(msg: impl Into<String>) -> Self {
        Self::Refused(msg.into())
    }

    /// A malformed-event fault.
    pub fn faulted(msg: impl Into<String>) -> Self {
        Self::Faulted(msg.into())
    }

    /// Python's `KeyError(k)` as the reducers raise it on a missing field (`str(e)` is the key in quotes).
    pub fn missing(key: &str) -> Self {
        Self::Faulted(format!("'{key}'"))
    }

    /// The message without the replay position.
    pub fn msg(&self) -> &str {
        match self {
            Self::Refused(m) | Self::Faulted(m) | Self::Replay { msg: m, .. } => m,
        }
    }
}

/// A `pk_core::py` helper's Python exception: a `ValueError` is a refusal, a `TypeError` a fault.
impl From<pk_core::py::PyError> for CreativeError {
    fn from(e: pk_core::py::PyError) -> Self {
        match e {
            pk_core::py::PyError::Value(m) => Self::Refused(m),
            pk_core::py::PyError::Type(m) => Self::Faulted(m),
        }
    }
}

/// `Result` with [`CreativeError`].
pub type CreativeResult<T> = Result<T, CreativeError>;
