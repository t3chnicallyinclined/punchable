//! What the services take from the environment for logs and crash reports, in one struct a binary fills from
//! `pk_core::config::Env` ([`ObsConfig::from_env`]) or a test from parts ([`ObsConfig::new`]). The crate itself never
//! reads `std::env` (ADR 0010).

use pk_core::{ConfigError, Env};

use crate::crash::DEFAULT_ENVIRONMENT;

/// Where crash reports go and how logs are filtered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObsConfig {
    /// `SENTRY_DSN`, trimmed; empty means crash reporting is off.
    pub sentry_dsn: String,
    /// `SENTRY_ENVIRONMENT`, trimmed; [`DEFAULT_ENVIRONMENT`] when unset.
    pub sentry_environment: String,
    /// `RUST_LOG`: the `tracing` filter directive, when set.
    pub log_filter: Option<String>,
}

impl ObsConfig {
    /// From explicit parts.
    pub fn new(
        sentry_dsn: impl Into<String>,
        sentry_environment: impl Into<String>,
        log_filter: Option<String>,
    ) -> Self {
        Self {
            sentry_dsn: sentry_dsn.into(),
            sentry_environment: sentry_environment.into(),
            log_filter,
        }
    }

    /// Crash reporting off, the default environment, the default log filter.
    pub fn off() -> Self {
        Self::new("", DEFAULT_ENVIRONMENT, None)
    }

    /// From the environment the binary read. Never refuses: every value has a default.
    pub fn from_env(env: &Env) -> Result<Self, ConfigError> {
        let trimmed = |k: &str| env.opt(k).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        Ok(Self {
            sentry_dsn: trimmed("SENTRY_DSN").unwrap_or_default(),
            sentry_environment: trimmed("SENTRY_ENVIRONMENT").unwrap_or_else(|| DEFAULT_ENVIRONMENT.to_string()),
            log_filter: env.opt("RUST_LOG").map(str::to_string),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_trimming() {
        let c = ObsConfig::from_env(&Env::from_map([("SENTRY_ENVIRONMENT", "  ")])).unwrap();
        assert_eq!(c, ObsConfig::off());
        let c = ObsConfig::from_env(&Env::from_map([
            ("SENTRY_DSN", " https://k@bugs.example/1 "),
            ("SENTRY_ENVIRONMENT", "preview"),
            ("RUST_LOG", "debug"),
        ]))
        .unwrap();
        assert_eq!(
            c,
            ObsConfig::new("https://k@bugs.example/1", "preview", Some("debug".into()))
        );
    }
}
