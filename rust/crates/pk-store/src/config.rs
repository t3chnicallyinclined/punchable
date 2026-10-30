//! Where the connection comes from: the `SURREAL_*` values of `pk_core::config::Env` (`~/.punchable/secrets.env`
//! or `PUNCH_SECRETS`, overridden by the environment, read the way `db/sdb.py` reads it; a binary builds the `Env`
//! once). Nothing in that file is committed.

use pk_core::{ConfigError, Env};

/// The local server's port when `SURREAL_PORT` is not set.
pub const DEFAULT_PORT: u16 = 8797;
/// The namespace the real data lives in.
pub const NS: &str = "punchable";
/// The database the real data lives in.
pub const DB: &str = "main";

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
    pub fn from_env(
        env: &Env,
        port: Option<u16>,
        ns: Option<&str>,
        db: Option<&str>,
    ) -> std::result::Result<Self, ConfigError> {
        let url = if let (None, Some(u)) = (port, env.opt("SURREAL_URL")) {
            u.to_string()
        } else {
            let p = match port {
                Some(p) => p,
                None => match env.u64("SURREAL_PORT")? {
                    None => DEFAULT_PORT,
                    Some(p) => u16::try_from(p).map_err(|e| ConfigError::Invalid {
                        key: "SURREAL_PORT".into(),
                        reason: e.to_string(),
                    })?,
                },
            };
            format!("ws://127.0.0.1:{p}")
        };
        let db_user = match (port, env.opt("SURREAL_API_USER"), env.opt("SURREAL_API_PASS")) {
            (None, Some(u), Some(p)) => Some((u.to_string(), p.to_string())),
            _ => None,
        };
        let (auth, dns, ddb) = if let Some((user, pass)) = db_user {
            let dns = env.opt("SURREAL_NS").unwrap_or(NS).to_string();
            let ddb = env.opt("SURREAL_DB").unwrap_or(DB).to_string();
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
            (root_auth(env)?, NS.to_string(), DB.to_string())
        };
        Ok(Self {
            url,
            auth,
            ns: ns.map_or(dns, str::to_string),
            db: db.map_or(ddb, str::to_string),
        })
    }

    /// A scratch server on `127.0.0.1:<port>` started with the secrets file's root user (tests).
    pub fn scratch(env: &Env, port: u16, ns: &str, db: &str) -> std::result::Result<Self, ConfigError> {
        Ok(Self::new(format!("ws://127.0.0.1:{port}"), root_auth(env)?, ns, db))
    }

    /// The URL the crate connects to: the configured one without a trailing `/rpc`.
    pub fn endpoint(&self) -> &str {
        self.url.strip_suffix("/rpc").unwrap_or(&self.url).trim_end_matches('/')
    }
}

/// Root with `SURREAL_USER` (default `root`) and `SURREAL_PASS` (required).
fn root_auth(env: &Env) -> std::result::Result<Auth, ConfigError> {
    Ok(Auth::Root {
        user: env.opt("SURREAL_USER").unwrap_or("root").to_string(),
        pass: env.str("SURREAL_PASS")?.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_connection_follows_sdb_py() {
        let root = Env::from_map([
            ("SURREAL_PASS", "pw"),
            ("SURREAL_PORT", "9000"),
            ("SURREAL_URL", "wss://db.example"),
        ]);
        let c = Config::from_env(&root, None, None, None).unwrap();
        assert_eq!(c.url, "wss://db.example", "SURREAL_URL wins when no port is asked for");
        assert_eq!(
            c.auth,
            Auth::Root {
                user: "root".into(),
                pass: "pw".into()
            }
        );
        assert_eq!((c.ns.as_str(), c.db.as_str()), (NS, DB));
        let c = Config::from_env(&root, Some(1), Some("a"), Some("b")).unwrap();
        assert_eq!(c.url, "ws://127.0.0.1:1");
        assert_eq!((c.ns.as_str(), c.db.as_str()), ("a", "b"));
        let port_only = Env::from_map([("SURREAL_PASS", "pw"), ("SURREAL_PORT", "9000")]);
        assert_eq!(
            Config::from_env(&port_only, None, None, None).unwrap().url,
            "ws://127.0.0.1:9000"
        );
        assert_eq!(
            Config::from_env(&Env::from_map([("SURREAL_PASS", "pw")]), None, None, None)
                .unwrap()
                .url,
            "ws://127.0.0.1:8797"
        );
        let api = Env::from_map([
            ("SURREAL_API_USER", "api"),
            ("SURREAL_API_PASS", "pw"),
            ("SURREAL_NS", "n"),
        ]);
        let c = Config::from_env(&api, None, None, None).unwrap();
        assert_eq!(
            c.auth,
            Auth::Database {
                user: "api".into(),
                pass: "pw".into(),
                ns: "n".into(),
                db: DB.into()
            }
        );
        assert_eq!(c.ns, "n");
        let c = Config::scratch(&root, 7, "x", "y").unwrap();
        assert_eq!(
            (c.url.as_str(), c.ns.as_str(), c.db.as_str()),
            ("ws://127.0.0.1:7", "x", "y")
        );
        let e = Config::from_env(&Env::from_map([("SURREAL_PASS", "")]), None, None, None).unwrap_err();
        assert!(
            matches!(e, ConfigError::Missing { ref key, .. } if key == "SURREAL_PASS"),
            "{e}"
        );
        let e = Config::from_env(
            &Env::from_map([("SURREAL_PASS", "pw"), ("SURREAL_PORT", "70000")]),
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(
            matches!(e, ConfigError::Invalid { ref key, .. } if key == "SURREAL_PORT"),
            "{e}"
        );
    }

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
