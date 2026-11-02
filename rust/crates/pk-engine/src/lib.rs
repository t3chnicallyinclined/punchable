//! Punchable's retrieval and evidence engine, forked from upstream (Rust branch 9dfedad; what came from where,
//! what was dropped and what changed: `FORK.md` beside this crate; the decision: docs/adr/0013).
//!
//! What it is for: answering a question about his notebook from his own words, and learning without guessing.
//! - [`verbatim`]: the one gate every line that claims to be his passes (Ask and the engine share it).
//! - [`check`]: an answer's quotes against what its candidates say now (read, cited, unchanged, word for word).
//! - [`rerank`], [`rank_config`], [`pack`]: a read made into a packet: the store's full-text and vector legs and
//!   BM25 over the read, fused, under a versioned config that names itself by its hash.
//! - [`ask`] over [`store::EngineStore`]: the async seam; `pk-store` implements it over `SurrealDB`.
//! - [`fact`]: lessons with their source and the evidence rule; [`learning`]: the live config, the offline replay,
//!   the fence that rolls back, the learning switch; [`proposer`]: a config scored over cached pools, the typed
//!   diff a model may propose.
//! - [`card`], [`bridge`]: cards about a bit and links between bits, ranked with no model.
//! - [`frame`]: the one frame around text a model must read but never obey; [`edge`]: the engine in the app's
//!   words (no grade ever reaches the app).
//!
//! Pure: no IO, no environment, no processes (a test reads this crate's sources and fails on any).

pub mod ask;
pub mod bridge;
pub mod card;
pub mod check;
pub mod edge;
pub mod error;
pub mod fact;
pub mod frame;
pub mod learning;
pub mod locator;
pub mod pack;
pub mod proposer;
pub mod rank_config;
pub mod rerank;
pub mod store;
pub mod verbatim;

pub use error::{EngineError, EngineResult};
pub use pack::Hit;
pub use rank_config::RankConfig;
pub use store::{EngineStore, Read, ReadQuery};

/// The engine's tables (`schema/engine.surql`): applied only by an explicit call, never on connect.
pub const SCHEMA: &str = include_str!("../schema/engine.surql");
