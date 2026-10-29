//! Logs: one `tracing` subscriber on stderr, filtered by `RUST_LOG` (default `info`), in the compact format. Colour
//! only when stderr is a terminal, so pod logs stay plain.

use std::io::IsTerminal;

use tracing_subscriber::EnvFilter;

use crate::error::{ObsError, ObsResult};

/// Install the global subscriber. Errors when one is already installed (a second call in the same process).
pub fn init_tracing() -> ObsResult<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .compact()
        .try_init()
        .map_err(|e| ObsError::Tracing(e.to_string()))
}
