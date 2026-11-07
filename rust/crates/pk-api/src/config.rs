//! What the API takes from the environment, read once by the binary from `pk_core::config::Env`. The names are the
//! hosted Python's (`api/entry.py`, `api-dev/auth.py`, `api-dev/server.py`), so the same Secret and manifest serve
//! both during T9.

use std::path::PathBuf;

use pk_core::{ConfigError, Env};
use serde_json::{Map, Value};

/// The API's settings.
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// `HOST` (default `0.0.0.0`).
    pub host: String,
    /// `PORT` (default 8080).
    pub port: u16,
    /// The token check: `None` = auth off (allowed only on loopback with `PK_ALLOW_NO_AUTH=1`).
    pub auth: Option<AuthConfig>,
    /// Browser origins allowed besides loopback (`PUBLIC_URL` without a trailing `/`).
    pub origins: Vec<String>,
    /// `PK_APP_FLAGS`: switches the phone reads off `/api/health` (an object; anything else = none).
    pub flags: Map<String, Value>,
    /// `PK_OWNER_ID` (default `owner`): the actor on his own edits.
    pub owner_id: String,
    /// `PK_DATA` (default `/tmp/punchable`): only for the paths `/api/health` names, as the Python pod names them.
    pub data: PathBuf,
    /// `PK_PYTHON_URL`: where every request this service does not answer goes during T9 (deleted at T10). `None` =
    /// those answer 404.
    pub python_url: Option<String>,
    /// The database.
    pub store: pk_store::Config,
    /// `PK_BUILD_SHA` (the image's git sha; `dev` when unset).
    pub build_sha: String,
    /// Search's embedder: `PK_EMBEDDER=hash`, else the bge-small folder (`PK_BGE_DIR` /
    /// `PK_CACHE_DIR`), else none (loaded on first use: `embed`).
    pub embedder: crate::embed::Choice,
    /// The private legal rules file (`PUNCHABLE_LEGAL_RULES`, else `~/.punchable/legal_rules.json`); the binary reads
    /// it into [`ApiConfig::legal`]. Hosted, the Secret punchable-legal-rules is mounted there, as for the Python API.
    pub legal_rules: PathBuf,
    /// The court-material rule every stored text goes through (`refuse_legal`): the generic markers until the binary
    /// adds the file's.
    pub legal: pk_core::legal::Legal,
    /// `PK_VEC_SWEEP_S` (default 60): how often the vector sweeper runs; `0` = never (a test pair, a pod that must
    /// not embed).
    pub sweep_every: Option<std::time::Duration>,
    /// `PK_LIVE_PING_S` (default 20): a keep-alive ping to every `/api/live` socket this often, on its own clock (an
    /// event never moves it).
    pub live_ping: std::time::Duration,
    /// `PK_LIVE_IDLE_S` (default 40): a socket that has sent no frame (its pong included) this long is half open and
    /// is closed. A test sets both to a few hundred milliseconds.
    pub live_idle: std::time::Duration,
    /// `PK_UPLOAD_DB` = `1` (default off): uploads are rows in the database (docs/adr/0014, accepted on the lead's defaults). Off, `POST
    /// /api/upload` answers 503 here and the inbox and the transcriber's fetch are the Python's (the pod's inbox
    /// folder), so a moved path can never silently diverge from it.
    pub upload_db: bool,
    /// `PK_UPLOAD_KEEP_S` (default 7 days): how long a recording waits for a transcriber before the expiry pass
    /// deletes it (docs/adr/0014).
    pub upload_keep: std::time::Duration,
    /// `PK_SCHEMA_OWNER` (default on; `0` = off): this process applies `db/schema_extra.surql` when the database holds
    /// another sha of it, at connect and every supervise round (T10 round 5, group F: the Python no longer has to).
    /// Off, it only reports the difference (uploads answer 503 until the database has this build's tables).
    pub schema_owner: bool,
    /// `PK_UPLOAD_SWEEP_S` (default an hour): how often the expiry pass runs; `0` = never.
    pub upload_sweep_every: Option<std::time::Duration>,
    /// `PK_COMIC_NAME` as set: how the AI prompts name him (the Ask conversations' system line). Unset, empty and
    /// the placeholder all read "a stand-up comic's".
    pub comic_name: Option<String>,
}

/// Keycloak.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthConfig {
    /// `OIDC_ISSUER`, matched exactly against `iss`.
    pub issuer: String,
    /// `OIDC_AUDIENCE` (default `pk-api`), must be in `aud`.
    pub audience: String,
    /// `OIDC_JWKS_URL` (default `<issuer>/protocol/openid-connect/certs`).
    pub jwks_url: String,
}

const LOOPBACK: [&str; 3] = ["127.0.0.1", "localhost", "::1"];

impl ApiConfig {
    /// Read the settings; refuses auth off on anything but loopback, and auth on without an issuer.
    pub fn from_env(env: &Env) -> Result<Self, ConfigError> {
        let host = env.opt("HOST").unwrap_or("0.0.0.0").to_string();
        let port = match env.u64("PORT")? {
            None => 8080,
            Some(p) => u16::try_from(p).map_err(|e| ConfigError::Invalid {
                key: "PORT".into(),
                reason: e.to_string(),
            })?,
        };
        let auth = if env.opt("PK_AUTH").unwrap_or("1") == "1" {
            let issuer = env.str("OIDC_ISSUER")?.to_string();
            let audience = env.opt("OIDC_AUDIENCE").unwrap_or("pk-api").to_string();
            let jwks_url = env.opt("OIDC_JWKS_URL").filter(|s| !s.is_empty()).map_or_else(
                || format!("{}/protocol/openid-connect/certs", issuer.trim_end_matches('/')),
                str::to_string,
            );
            Some(AuthConfig {
                issuer,
                audience,
                jwks_url,
            })
        } else {
            if env.opt("PK_ALLOW_NO_AUTH") != Some("1") || !LOOPBACK.contains(&host.as_str()) {
                return Err(ConfigError::Invalid {
                    key: "PK_AUTH".into(),
                    reason: "auth off needs PK_ALLOW_NO_AUTH=1 and a loopback HOST".into(),
                });
            }
            None
        };
        let origins = env
            .opt("PUBLIC_URL")
            .map(|u| u.trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty())
            .into_iter()
            .collect();
        let flags = env
            .opt("PK_APP_FLAGS")
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let ns = env.opt("SURREAL_NS").unwrap_or(pk_store::config::NS).to_string();
        let db = env.opt("SURREAL_DB").unwrap_or(pk_store::config::DB).to_string();
        Ok(Self {
            host,
            port,
            auth,
            origins,
            flags,
            owner_id: env.opt("PK_OWNER_ID").unwrap_or(pk_kb::DEFAULT_OWNER_ID).to_string(),
            data: env.path("PK_DATA").unwrap_or_else(|| PathBuf::from("/tmp/punchable")),
            python_url: env
                .opt("PK_PYTHON_URL")
                .map(|u| u.trim_end_matches('/').to_string())
                .filter(|u| !u.is_empty()),
            store: pk_store::Config::from_env(env, None, Some(&ns), Some(&db))?,
            build_sha: env.opt("PK_BUILD_SHA").unwrap_or("").to_string(),
            embedder: crate::embed::Choice::from_env(env)?,
            legal_rules: env.path("PUNCHABLE_LEGAL_RULES").unwrap_or_else(|| {
                env.path("PUNCHABLE_HOME")
                    .or_else(|| env.home().map(|h| h.join(".punchable")))
                    .unwrap_or_default()
                    .join("legal_rules.json")
            }),
            legal: pk_core::legal::Legal::default(),
            live_ping: std::time::Duration::from_secs_f64(
                env.f64("PK_LIVE_PING_S")?.filter(|s| *s > 0.0).unwrap_or(20.0),
            ),
            live_idle: std::time::Duration::from_secs_f64(
                env.f64("PK_LIVE_IDLE_S")?.filter(|s| *s > 0.0).unwrap_or(40.0),
            ),
            upload_db: env.opt("PK_UPLOAD_DB") == Some("1"),
            schema_owner: env.opt("PK_SCHEMA_OWNER") != Some("0"),
            upload_keep: match env.f64("PK_UPLOAD_KEEP_S")? {
                Some(s) if s > 0.0 => std::time::Duration::from_secs_f64(s),
                _ => crate::expiry::DEFAULT_KEEP,
            },
            upload_sweep_every: match env.f64("PK_UPLOAD_SWEEP_S")? {
                None => Some(crate::expiry::DEFAULT_EVERY),
                Some(s) if s > 0.0 => Some(std::time::Duration::from_secs_f64(s)),
                Some(_) => None,
            },
            comic_name: env.opt("PK_COMIC_NAME").map(str::to_string),
            sweep_every: match env.f64("PK_VEC_SWEEP_S")? {
                None => Some(crate::sweep::DEFAULT_EVERY),
                Some(s) if s > 0.0 => Some(std::time::Duration::from_secs_f64(s)),
                Some(_) => None,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Vec<(&'static str, &'static str)> {
        vec![("SURREAL_PASS", "pw")]
    }

    #[test]
    fn auth_is_on_by_default_and_needs_an_issuer() {
        assert!(ApiConfig::from_env(&Env::from_map(base())).is_err());
        let mut m = base();
        m.push(("OIDC_ISSUER", "https://auth.example/realms/punchable/"));
        let c = ApiConfig::from_env(&Env::from_map(m)).unwrap();
        let a = c.auth.unwrap();
        assert_eq!(a.audience, "pk-api");
        assert_eq!(
            a.jwks_url,
            "https://auth.example/realms/punchable/protocol/openid-connect/certs"
        );
    }

    #[test]
    fn auth_off_only_on_loopback_with_the_switch() {
        let mut m = base();
        m.push(("PK_AUTH", "0"));
        assert!(ApiConfig::from_env(&Env::from_map(m.clone())).is_err());
        m.push(("PK_ALLOW_NO_AUTH", "1"));
        assert!(
            ApiConfig::from_env(&Env::from_map(m.clone())).is_err(),
            "0.0.0.0 is not loopback"
        );
        m.push(("HOST", "127.0.0.1"));
        assert!(ApiConfig::from_env(&Env::from_map(m)).unwrap().auth.is_none());
    }

    #[test]
    fn flags_must_be_an_object() {
        let mut m = base();
        m.extend([
            ("PK_AUTH", "0"),
            ("PK_ALLOW_NO_AUTH", "1"),
            ("HOST", "::1"),
            ("PK_APP_FLAGS", "[1]"),
        ]);
        assert!(ApiConfig::from_env(&Env::from_map(m.clone())).unwrap().flags.is_empty());
        m.push(("PK_APP_FLAGS", r#"{"glass_accessory": false}"#));
        let c = ApiConfig::from_env(&Env::from_map(m)).unwrap();
        assert_eq!(c.flags["glass_accessory"], false);
    }

    #[test]
    fn the_sweep_period_is_sixty_seconds_unless_set_and_zero_is_off() {
        let mut m = base();
        m.push(("OIDC_ISSUER", "https://auth.example/realms/punchable"));
        assert_eq!(
            ApiConfig::from_env(&Env::from_map(m.clone())).unwrap().sweep_every,
            Some(std::time::Duration::from_secs(60))
        );
        m.push(("PK_VEC_SWEEP_S", "86400"));
        assert_eq!(
            ApiConfig::from_env(&Env::from_map(m.clone())).unwrap().sweep_every,
            Some(std::time::Duration::from_hours(24))
        );
        m.push(("PK_VEC_SWEEP_S", "0"));
        assert_eq!(ApiConfig::from_env(&Env::from_map(m)).unwrap().sweep_every, None);
    }
}
