//! Logs: one `tracing` subscriber on stderr, filtered by the `RUST_LOG` directive the binary passes in (default
//! `info`; a directive that does not parse is `info` too), in the compact format. Colour only when stderr is a terminal,
//! so pod logs stay plain.

use std::io::IsTerminal;

use tracing_subscriber::EnvFilter;

use crate::error::{ObsError, ObsResult};

/// Install the global subscriber with `filter` (`ObsConfig::log_filter`, the `RUST_LOG` directive). Errors when one is
/// already installed (a second call in the same process).
pub fn init_tracing(filter: Option<&str>) -> ObsResult<()> {
    let filter = filter
        .and_then(|f| EnvFilter::try_new(f).ok())
        .unwrap_or_else(|| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .compact()
        .try_init()
        .map_err(|e| ObsError::Tracing(e.to_string()))
}
