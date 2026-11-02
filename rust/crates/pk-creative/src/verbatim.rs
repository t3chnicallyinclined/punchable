//! The verbatim gate: a quote is kept only as an exact substring of its source, typography forgiven (curly quotes,
//! dash style, whitespace runs, an ellipsis character), words and case never.
//!
//! One copy for every caller: the implementation lives in `pk_engine::verbatim` since the upstream fork
//! (docs/adr/0013-fork-the-upstream-engine.md), where the engine's answer check uses it too. This module re-exports
//! it so Ask ([`crate::ask`]) and this crate's goldens (`tests/goldens.rs`, against both Python copies) and property
//! tests (`tests/verbatim_props.rs`) keep proving the same function.

pub use pk_engine::verbatim::{QUOTE_MIN, verbatim};
