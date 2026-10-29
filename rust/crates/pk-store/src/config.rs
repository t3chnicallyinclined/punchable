//! Where the connection comes from: `~/.punchable/secrets.env` (or `PUNCH_SECRETS`), overridden by the environment,
//! read the way `db/sdb.py` reads it. Nothing in that file is committed.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::{Result, StoreError};

/// The local server's port when `SURREAL_PORT` is not set.
pub const DEFAULT_PORT: u16 = 8797;
/// The namespace the real data lives in.
pub const NS: &str = "punchable";
/// The database the real data lives in.
pub const DB: &str = "main";

const ENV_KEYS: [&str; 8] = [
    "SURREAL_USER",
    "SURREAL_PASS",
    "SURREAL_PORT",
    "SURREAL_URL",
    "SURREAL_API_USER",
    "SURREAL_API_PASS",
    "SURREAL_NS",
    "SURREAL_DB",
];

/// `PUNCH_SECRETS`, else `~/.punchable/secrets.env`.
pub fn secrets_path() -> PathBuf {
    if let Some(p) = std::env::var_os("PUNCH_SECRETS") {
        return PathBuf::from(p);
    }
    std::env::home_dir()
        .unwrap_or_default()
        .join(".punchable")
        .join("secrets.env")
}

/// The `KEY=value` lines of the secrets file (comments and blanks skipped), with the `SURREAL_*` environment
/// variables on top.
pub fn secrets() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(secrets_path()) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                out.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    for k in ENV_KEYS {
        if let Ok(v) = std::env::var(k)
            && !v.is_empty()
        {
            out.insert(k.to_string(), v);
        }
    }
    out
}

/// Who signs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    /// The root user (the PC, the scratch servers in tests).
    Root {
        /// `SURREAL_USER`, default `root`.
        user: String,
        /// `SURREAL_PASS`.
        pass: String,
    },
    /// A database-level user, EDITOR on one database (the hosted API: `SURREAL_API_USER`/`SURREAL_API_PASS`). It never
    /// defines namespaces or users.
    Database {
        /// The user.
        user: String,
        /// Its password.
        pass: String,
        /// The namespace it belongs to.
        ns: String,
        /// The database it belongs to.
        db: String,
    },
}

/// A connection: where, who, and which namespace and database to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// `ws://host:port` or `wss://host` (a trailing `/rpc` is accepted and dropped: the crate adds it).
    pub url: String,
    /// Who signs in.
    pub auth: Auth,
    /// The namespace to use.
    pub ns: String,
    /// The database to use.
    pub db: String,
}

impl Config {
    /// A connection from explicit parts.
    pub fn new(url: impl Into<String>, auth: Auth, ns: impl Into<String>, db: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            auth,
            ns: ns.into(),
            db: db.into(),
        }
    }

    /// The connection `db/sdb.py` would open: `SURREAL_URL` when it is set and no port was asked for, else
    /// `ws://127.0.0.1:<port or SURREAL_PORT or 8797>`; the database user when `SURREAL_API_USER`/`PASS` are set and
    /// no port was asked for, else root with `SURREAL_PASS` (required).
    pub fn from_secrets(port: Option<u16>, ns: Option<&str>, db: Option<&str>) -> Result<Self> {
        let s = secrets();
        let url = if let (None, Some(u)) = (port, s.get("SURREAL_URL")) {
            u.clone()
        } else {
            let p = match port {
                Some(p) => p,
                None => s
                    .get("SURREAL_PORT")
                    .map_or(Ok(DEFAULT_PORT), |v| v.parse::<u16>())
                    .map_err(|e| StoreError::Config(format!("SURREAL_PORT: {e}")))?,
            };
            format!("ws://127.0.0.1:{p}")
        };
        let db_user = match (port, s.get("SURREAL_API_USER"), s.get("SURREAL_API_PASS")) {
            (None, Some(u), Some(p)) if !u.is_empty() && !p.is_empty() => Some((u.clone(), p.clone())),
            _ => None,
        };
        let (auth, dns, ddb) = if let Some((user, pass)) = db_user {
            let dns = s.get("SURREAL_NS").cloned().unwrap_or_else(|| NS.to_string());
            let ddb = s.get("SURREAL_DB").cloned().unwrap_or_else(|| DB.to_string());
            (
                Auth::Database {
                    user,
                    pass,
                    ns: dns.clone(),
                    db: ddb.clone(),
                },
                dns,
                ddb,
            )
        } else {
            let pass = s
                .get("SURREAL_PASS")
                .filter(|p| !p.is_empty())
                .ok_or_else(|| StoreError::Config(format!("no SURREAL_PASS in {}", secrets_path().display())))?;
            let user = s.get("SURREAL_USER").cloned().unwrap_or_else(|| "root".to_string());
            (
                Auth::Root {
                    user,
                    pass: pass.clone(),
                },
                NS.to_string(),
                DB.to_string(),
            )
        };
        Ok(Self {
            url,
            auth,
            ns: ns.map_or(dns, str::to_string),
            db: db.map_or(ddb, str::to_string),
        })
    }

    /// A scratch server on `127.0.0.1:<port>` started with the secrets file's root user (tests).
    pub fn scratch(port: u16, ns: &str, db: &str) -> Result<Self> {
        let s = secrets();
        let pass = s
            .get("SURREAL_PASS")
            .filter(|p| !p.is_empty())
            .ok_or_else(|| StoreError::Config(format!("no SURREAL_PASS in {}", secrets_path().display())))?;
        Ok(Self::new(
            format!("ws://127.0.0.1:{port}"),
            Auth::Root {
                user: s.get("SURREAL_USER").cloned().unwrap_or_else(|| "root".to_string()),
                pass: pass.clone(),
            },
            ns,
            db,
        ))
    }

    /// The URL the crate connects to: the configured one without a trailing `/rpc`.
    pub fn endpoint(&self) -> &str {
        self.url.strip_suffix("/rpc").unwrap_or(&self.url).trim_end_matches('/')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_drops_rpc() {
        let c = Config::new(
            "ws://surrealdb:8000/rpc",
            Auth::Root {
                user: "r".into(),
                pass: "p".into(),
            },
            "a",
            "b",
        );
        assert_eq!(c.endpoint(), "ws://surrealdb:8000");
        let c = Config::new(
            "ws://127.0.0.1:1",
            Auth::Root {
                user: "r".into(),
                pass: "p".into(),
            },
            "a",
            "b",
        );
        assert_eq!(c.endpoint(), "ws://127.0.0.1:1");
    }
}
