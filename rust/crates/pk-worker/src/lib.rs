//! Punchable's hosted worker (T8): the Rust replacement for `python -m worker --hosted` (`worker/__main__.py`,
//! `worker/online.py`, `worker/execute.py`). It speaks HTTP to the API only (ADR 0011): claim a job from
//! `/api/worker/claim`, fetch his voice profile, run the kind on the Anthropic API through `pk-llm` (one daily USD
//! budget for the process), post the files and ideas it made and then its result. A heartbeat file for the liveness
//! probe, `:9100/metrics` for Prometheus, crash reports to the self-hosted Bugsink through `pk-obs`.
//!
//! - [`config`]: the command line and the environment.
//! - [`api`]: the one client for the API, with the Keycloak token.
//! - [`runner`]: the loop, the job slots, the hand-back on SIGTERM.
//! - [`kinds`]: one module per job kind, dispatched by an exhaustive `match`.
//! - [`voice`]: the profile and learned blocks every writing prompt starts with.
//! - [`legal`]: the court-material net and the paths a worker may write.
//! - `pyjson`: the files written byte for byte as the Python writes them.
//! - [`heartbeat`], [`metrics_server`]: liveness and observability.
//! - [`error`]: how a failed job reaches the API.
//!
//! Proof: `rust/tools/gen_worker_goldens.py` runs the Python worker against a fixture API, a mock Anthropic and a
//! page server; `tests/differential.rs` runs this worker against the same fixtures and compares every request as
//! parsed JSON.

pub mod api;
pub mod config;
pub mod error;
pub mod heartbeat;
pub mod kinds;
pub mod legal;
pub mod metrics_server;
pub mod mirror;
pub(crate) mod pyjson;
pub mod runner;
pub mod voice;

pub use config::{RunArgs, WorkerConfig};
pub use runner::{LoopError, Parts, Worker};

/// The service name in the crash-report release (`punchable-worker@<version>+<sha>`).
pub const SERVICE: &str = "punchable-worker";
/// This build's version, the `version` label of `pk_worker_build_info`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
