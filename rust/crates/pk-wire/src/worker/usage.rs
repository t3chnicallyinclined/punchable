//! `POST /api/worker/usage`: the model calls the worker made, for the usage meter (SPEC 2026-11-17 §13). Counts
//! and names only: never a prompt, an answer, a line of his material or an id of his material. pk-api prices each
//! call from the one price table (`pk_llm::budget`) and keeps it as a `usage_call` row keyed by `call_id`, so a
//! batch posted twice (a timeout after the API wrote it) is stored once.

use serde::{Deserialize, Serialize};

/// One model call the budget charged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct UsageCall {
    /// The router's call id (12 hex characters, unique per call): the row's key.
    pub call_id: String,
    /// When the call ended, UTC, ISO 8601.
    pub at: String,
    /// The job it was made for, when the worker knew it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// The job's kind (`riff`, `ask`, `redline`, ...), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The router task (`creative_riff`, `ask_chat`, ...).
    pub task: String,
    /// The model that answered.
    pub model: String,
    /// Uncached input tokens.
    pub tokens_in: u64,
    /// Output tokens.
    pub tokens_out: u64,
    /// Tokens read from the prompt cache.
    pub cache_read: u64,
    /// Tokens written to the prompt cache.
    pub cache_write: u64,
}

/// The body: a batch of calls (at most [`MAX_BATCH`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct UsagePost {
    /// The calls, oldest first.
    pub calls: Vec<UsageCall>,
}

/// The answer: how many calls the batch carried that are now stored (a repeat counts again; it is stored once).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct UsagePostResponse {
    /// Calls stored.
    pub stored: usize,
}

/// The most calls one post may carry.
pub const MAX_BATCH: usize = 500;
