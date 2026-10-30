//! The Anthropic Messages client, the task router, schema validation and the daily budget: the Rust `pk-llm` of
//! docs/RUST.md step 2, replacing `engine/providers.py` (the `AnthropicBackend` and the `Router`), `engine/config.py`
//! with `engine/config.hosted.toml`, and `engine/budget.py`.
//!
//! There is no official Rust SDK for the Anthropic API, so the wire is `reqwest` plus typed serde models of the
//! Messages API (`api`), SSE parsed by `eventsource-stream` and accumulated the way the Python SDK's
//! `get_final_message()` does (`client`). Everything else is a straight port: the request the backend builds per
//! task (`backend`), the task → provider → model routing with one retry on invalid JSON and the `fallbacks`
//! models after a refusal (`router`), the SDK's `transform_schema` for `output_config.format` (`schema`), the TOML
//! loader (`config`) with the `LlmConfig` a binary fills from `pk_core::config::Env` (keys, base URL, budget: this
//! crate never reads the environment), the UTC-day USD ledger (`budget`). The plain-words errors the app shows are in `error`, copied
//! from `providers.py`.
//!
//! Proof: `rust/tools/gen_llm_goldens.py` records what the pinned Python SDK puts on the wire for synthetic prompts;
//! `tests/goldens.rs` builds the same requests here and compares as parsed JSON.

pub mod api;
pub mod backend;
pub mod budget;
pub mod client;
pub mod config;
pub mod error;
pub mod router;
pub mod schema;

pub use api::{Request, Response, Usage};
pub use backend::{AnthropicBackend, Called, ChatMessage};
pub use budget::{Budget, BudgetConfig};
pub use client::Client;
pub use config::{Config, LlmConfig, ProviderCfg, TaskCfg};
pub use error::LlmError;
pub use router::{Completion, Router};
