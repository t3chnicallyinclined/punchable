//! gsync in Rust: Google Docs and Drive <-> Punchable, run on the owner's PC as a scheduled task (T11 round 4).
//!
//! A port **as-is** of `gsync/` (the Python, kept until the T11 rollback window closes): the same command line
//! (`auth`, `status`, `run [--dry-run] [--watch [--interval N]]`, `undo RUN_ID`), the same requests in the same order,
//! the same state files under `<data folder>/kb/gsync/` (`map.json`, `units.json`, `memos.json`, the `*.jsonl` logs,
//! `mirror/`), the same token files under `~/.punchable/` (read and rewritten, never printed) and the same error words.
//! The differential (`tests/differential.rs`) replays tapes the Python recorded (`rust/tools/gen_gsync_tapes.py`).
//!
//! What it does not carry over, on purpose (README.md, "Differences"): the local-API mode (`PK_ONLINE_FIRST` unset;
//! the PC runs online-first), `auth --desktop` (the fallback client flow; a Desktop token file still works for `run`),
//! and the voice-memo transcriber (`python -m gsync.memos process`, faster-whisper): the Drive half of the memos is
//! here and starts that Python processor exactly as before.

pub mod auth;
pub mod cli;
pub mod config;
pub mod drive;
pub mod error;
pub mod http;
pub mod memos;
pub mod merge;
pub mod punchable;
pub mod pyjson;
pub mod rules;
pub mod services;
pub mod store;
pub mod sync;
pub mod units;

pub use error::{GsError, Result};
