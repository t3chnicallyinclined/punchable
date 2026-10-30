//! The one reader of the environment. A binary builds an [`Env`] once with [`Env::from_process`]: the `KEY=value`
//! lines of `secrets.env` (`PUNCH_SECRETS`, else `~/.punchable/secrets.env`, read the way `db/sdb.py` reads it:
//! comments and blanks skipped, keys and values trimmed) with the process environment laid over them. Every other
//! crate exposes an `XConfig::from_env(&Env)` that takes typed values out and never touches `std::env` (ADR 0010).
//! Tests build one with [`Env::from_map`].
//!
//! An empty value counts as unset, the Python `os.environ.get(k) or default` idiom, in every getter.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What a typed getter can refuse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The variable is unset or empty; `place` names the file and the environment that were looked at.
    #[error("{key} is not set in {place}")]
    Missing {
        /// The variable's name.
        key: String,
        /// Where it was looked for.
        place: String,
    },
    /// The variable is set but does not parse as asked.
    #[error("{key}: {reason}")]
    Invalid {
        /// The variable's name.
        key: String,
        /// What was wrong with it.
        reason: String,
    },
}

/// The environment as the process sees it: the secrets file under the process variables.
#[derive(Debug, Clone, Default)]
pub struct Env {
    vars: BTreeMap<String, String>,
    place: String,
    home: Option<PathBuf>,
}

impl Env {
    /// The secrets file (`PUNCH_SECRETS`, else `~/.punchable/secrets.env`; a missing file is an empty one) with the
    /// process environment over it. The only `std::env` reads in the workspace's library crates.
    pub fn from_process() -> Self {
        let home = std::env::home_dir();
        let file = std::env::var_os("PUNCH_SECRETS").map_or_else(
            || home.clone().unwrap_or_default().join(".punchable").join("secrets.env"),
            PathBuf::from,
        );
        let mut vars = parse_secrets(&std::fs::read_to_string(&file).unwrap_or_default());
        for (k, v) in std::env::vars_os() {
            if let (Ok(k), Ok(v)) = (k.into_string(), v.into_string())
                && !v.is_empty()
            {
                vars.insert(k, v);
            }
        }
        Self {
            vars,
            place: format!("{} or the environment", file.display()),
            home,
        }
    }

    /// An environment from given pairs (tests, fixtures); no home directory unless [`Env::with_home`] sets one.
    pub fn from_map<K: Into<String>, V: Into<String>>(pairs: impl IntoIterator<Item = (K, V)>) -> Self {
        Self {
            vars: pairs.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
            place: "the given environment".to_string(),
            home: None,
        }
    }

    /// The same environment with a home directory (what `~` means).
    #[must_use]
    pub fn with_home(mut self, home: impl Into<PathBuf>) -> Self {
        self.home = Some(home.into());
        self
    }

    /// The user's home directory, when the process has one.
    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    /// Where values were looked for, for an error message.
    pub fn place(&self) -> &str {
        &self.place
    }

    /// The value when it is set and not empty.
    pub fn opt(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str).filter(|v| !v.is_empty())
    }

    /// The value, or [`ConfigError::Missing`].
    pub fn str(&self, key: &str) -> Result<&str, ConfigError> {
        self.opt(key).ok_or_else(|| self.missing(key))
    }

    /// The value as an unsigned integer, or `None` when unset; [`ConfigError::Invalid`] when it does not parse.
    pub fn u64(&self, key: &str) -> Result<Option<u64>, ConfigError> {
        self.opt(key)
            .map(|v| v.parse::<u64>().map_err(|e| invalid(key, &e)))
            .transpose()
    }

    /// The value as a float, or `None` when unset; [`ConfigError::Invalid`] when it does not parse.
    pub fn f64(&self, key: &str) -> Result<Option<f64>, ConfigError> {
        self.opt(key)
            .map(|v| v.parse::<f64>().map_err(|e| invalid(key, &e)))
            .transpose()
    }

    /// The value as a path, or `None` when unset.
    pub fn path(&self, key: &str) -> Option<PathBuf> {
        self.opt(key).map(PathBuf::from)
    }

    /// The first of `keys` that is set, as a path.
    pub fn first_path(&self, keys: &[&str]) -> Option<PathBuf> {
        keys.iter().find_map(|k| self.path(k))
    }

    fn missing(&self, key: &str) -> ConfigError {
        ConfigError::Missing {
            key: key.to_string(),
            place: self.place.clone(),
        }
    }
}

fn invalid(key: &str, e: &dyn std::fmt::Display) -> ConfigError {
    ConfigError::Invalid {
        key: key.to_string(),
        reason: e.to_string(),
    }
}

/// The `KEY=value` lines of a secrets file: comments and blanks skipped, keys and values trimmed, the first `=`
/// splits, a later line wins.
pub fn parse_secrets(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_lines_parse_like_sdb_py() {
        let m = parse_secrets("# c\n\nSURREAL_USER = root \nSURREAL_PASS=a=b\nbad line\nSURREAL_USER=again\n");
        assert_eq!(m["SURREAL_USER"], "again");
        assert_eq!(m["SURREAL_PASS"], "a=b");
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn typed_getters() {
        let env = Env::from_map([("A", "1"), ("B", ""), ("F", "2.5"), ("P", "x/y"), ("N", "no")]).with_home("/h");
        assert_eq!(env.str("A").unwrap(), "1");
        assert_eq!(env.opt("B"), None, "empty counts as unset");
        assert_eq!(
            env.str("B").unwrap_err(),
            ConfigError::Missing {
                key: "B".into(),
                place: "the given environment".into()
            }
        );
        assert_eq!(env.u64("A").unwrap(), Some(1));
        assert_eq!(env.u64("B").unwrap(), None);
        assert!(matches!(env.u64("N").unwrap_err(), ConfigError::Invalid { key, .. } if key == "N"));
        assert_eq!(env.f64("F").unwrap(), Some(2.5));
        assert_eq!(env.path("P"), Some(PathBuf::from("x/y")));
        assert_eq!(env.first_path(&["B", "P"]), Some(PathBuf::from("x/y")));
        assert_eq!(env.home(), Some(Path::new("/h")));
        assert_eq!(
            env.str("B").unwrap_err().to_string(),
            "B is not set in the given environment"
        );
    }
}
