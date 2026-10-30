//! The tier-0 leaf of the Punchable workspace (ADR 0010): no internal dependencies, and everything may depend on it.
//!
//! - [`py`]: what Python prints and how it judges a value, for the texts the app shows and the prompts a model reads
//!   that must equal the Python's: `repr`, `str`, `float_repr`, `json.dumps`, `int`, `str.isspace`, truthiness.
//!   Anything named after a Python builtin lives here and nowhere else.
//! - [`text`]: code-point slicing (`s[:n]`).
//! - [`time`]: the ISO form the services write (`2026-10-29T12:00:00+00:00`), a [`time::Clock`] so a test passes a
//!   fixed time.
//! - [`config`]: [`config::Env`], the one place that reads `secrets.env` and the process environment; library crates
//!   take typed values from it and never call `std::env` themselves.
//!
//! Dependencies: `serde_json`, `chrono`, `thiserror`. No IO crate, no regex.

pub mod config;
pub mod py;
pub mod text;
pub mod time;

pub use config::{ConfigError, Env};
