//! The one error this crate returns: what `SurrealDB` said, a statement that failed, a write that wrote nothing, a
//! row that does not fit the mapping, a change feed that is missing, or a write refused before the database saw it.

use std::collections::BTreeMap;

/// Everything that can go wrong between the services and `SurrealDB`.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The official crate failed: a connection, a sign-in, a transport error.
    #[error("surrealdb: {0}")]
    Db(#[from] surrealdb::Error),
    /// Statement `index` (counting from zero) of a batch was not OK. Nothing of a transaction landed.
    #[error("statement {index} failed: {msg}\n--- {sql}")]
    Statement {
        /// Which statement.
        index: usize,
        /// What the database said.
        msg: String,
        /// The head of the batch, for the log.
        sql: String,
    },
    /// A write statement returned no rows: denied, or its target does not exist (trap T5).
    #[error("write returned nothing (denied, or the target doesn't exist):\n--- {sql}")]
    EmptyWrite {
        /// The head of the statement.
        sql: String,
    },
    /// A write returned rows, but not as many as expected.
    #[error("write returned {rows} rows, expected {expected}:\n--- {sql}")]
    WrongCount {
        /// The head of the statement.
        sql: String,
        /// How many rows came back.
        rows: usize,
        /// How many were expected.
        expected: usize,
    },
    /// A row or a KB dict does not fit the mapping (`db/kbmap.py`'s `ValueError`s).
    #[error("{0}")]
    Mapping(String),
    /// An id that is not a KB id.
    #[error(transparent)]
    Id(#[from] pk_kb::KbError),
    /// Tables whose change feed is missing, with their definition (trap T6): `{table: definition}`.
    #[error("change feed missing on {}", .0.keys().cloned().collect::<Vec<_>>().join(", "))]
    Changefeed(BTreeMap<String, String>),
    /// A write refused before the database saw it, with the HTTP status the API answers with
    /// (`kb.DecisionError(msg, status)` in the Python).
    #[error("{msg}")]
    Refused {
        /// 409 (out of step with the file), 503 (reloading, never loaded).
        status: u16,
        /// The plain-words reason.
        msg: String,
    },
    /// No credentials, or a configuration value that cannot be used.
    #[error("{0}")]
    Config(String),
    /// A store's `creative_event` rows are not `0..n`.
    #[error("creative_event {store}: sequence has gaps")]
    Gap {
        /// The store (`storyboards`, `workingsets`, `venues`, `jobs`).
        store: String,
    },
    /// A line that is not JSON where JSON was required.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl StoreError {
    /// A [`StoreError::Mapping`].
    pub fn mapping(msg: impl Into<String>) -> Self {
        Self::Mapping(msg.into())
    }

    /// A [`StoreError::Refused`].
    pub fn refused(status: u16, msg: impl Into<String>) -> Self {
        Self::Refused {
            status,
            msg: msg.into(),
        }
    }
}

/// `Result` with [`StoreError`].
pub type Result<T> = std::result::Result<T, StoreError>;

/// The first `n` characters of a statement, for an error message.
pub(crate) fn head(sql: &str, n: usize) -> String {
    sql.chars().take(n).collect()
}
