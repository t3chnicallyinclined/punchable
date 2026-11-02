//! What the worker takes from its command line and environment (`worker/__main__.py` `main`, `infra/k8s/
//! 35-worker.yaml`): the API and Keycloak, who it is, which kinds it claims, where the heartbeat and the spend ledger
//! live, the metrics port, how the prompts name the comic. Built once by the binary from `pk_core::config::Env`.

use std::path::PathBuf;
use std::time::Duration;

use pk_core::{ConfigError, Env};

use crate::api::{Credentials, Secret};
use crate::kinds::{Kind, TRANSCRIBER_KINDS};
use crate::voice::Names;

/// The public API when `PK_API` is unset.
pub const DEFAULT_API: &str = "https://punchable.io";
/// The realm's token endpoint when `PK_TOKEN_URL` is unset.
pub const DEFAULT_TOKEN_URL: &str = "https://auth.punchable.io/realms/punchable/protocol/openid-connect/token";
/// `PK_METRICS_PORT` when unset or not a number.
pub const DEFAULT_METRICS_PORT: u16 = 9100;
/// On a stop, how long a running job may still finish ([`WorkerConfig::stop_grace`]).
pub const STOP_GRACE: Duration = Duration::from_secs(20);
/// The most jobs one worker runs at once (`PK_WORKER_JOBS`).
pub const MAX_JOBS: usize = 4;

/// The command line (`python -m worker` compatible where it matters).
#[derive(Debug, Clone, Default, clap::Args)]
pub struct RunArgs {
    /// The API's base URL (default `PK_API`, else the public one).
    #[arg(long)]
    pub api: Option<String>,
    /// Keycloak's token endpoint (default `PK_TOKEN_URL`).
    #[arg(long)]
    pub token_url: Option<String>,
    /// The local dev API on loopback, auth off.
    #[arg(long)]
    pub no_auth: bool,
    /// One pass: run until nothing is queued, then exit (1 on a loop error).
    #[arg(long)]
    pub once: bool,
    /// Comma-separated kinds (default: every hosted kind this worker has).
    #[arg(long)]
    pub kinds: Option<String>,
    /// Seconds between polls when nothing is queued.
    #[arg(long, default_value_t = 15.0)]
    pub idle: f64,
    /// Accepted for the Python's entrypoint; this worker is always the hosted one.
    #[arg(long)]
    pub hosted: bool,
}

/// Everything the loop needs that is not a client.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// The API's base URL.
    pub api: String,
    /// Keycloak client credentials; `None` = auth off (loopback only).
    pub creds: Option<Credentials>,
    /// `PK_WORKER_NAME` (the pod), `COMPUTERNAME` or "home", 40 characters.
    pub worker_name: String,
    /// One pass, then exit.
    pub once: bool,
    /// Between polls when nothing is queued.
    pub idle: Duration,
    /// What the claim asks for.
    pub kinds: Vec<Kind>,
    /// The liveness file.
    pub heartbeat: PathBuf,
    /// `0` = no listener.
    pub metrics_port: u16,
    /// Jobs at once (1 unless `PK_WORKER_JOBS` says otherwise).
    pub jobs: usize,
    /// How the prompts name him.
    pub names: Names,
    /// The private legal rules file, if the pod has one (it does not today).
    pub legal_rules: PathBuf,
    /// The spend ledger.
    pub spend_file: PathBuf,
    /// `PK_BUILD_SHA` (`dev` when unset).
    pub build_sha: String,
    /// On a stop, how long a running job may still finish before it is handed back (inside the pod's 60 s grace).
    pub stop_grace: Duration,
    /// `PK_OWNER_ID`: the actor id of his own edits ("owner").
    pub owner_id: String,
}

fn invalid(key: &str, reason: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        key: key.to_string(),
        reason: reason.into(),
    }
}

/// `--kinds a,b`: transcriber kinds dropped; an unknown kind or one not ported yet is refused at start.
pub fn parse_kinds(s: &str) -> Result<Vec<Kind>, ConfigError> {
    let mut out = Vec::new();
    for name in s.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        if TRANSCRIBER_KINDS.contains(&name) {
            continue;
        }
        let k = Kind::parse(name).ok_or_else(|| invalid("--kinds", format!("unknown kind {name}")))?;
        if !k.ported() {
            return Err(invalid(
                "--kinds",
                format!("{name} is not ported to the Rust worker yet"),
            ));
        }
        if !out.contains(&k) {
            out.push(k);
        }
    }
    Ok(out)
}

impl WorkerConfig {
    /// From the environment the binary read and the command line.
    pub fn from_env(env: &Env, args: &RunArgs) -> Result<Self, ConfigError> {
        let api = args
            .api
            .clone()
            .or_else(|| env.opt("PK_API").map(str::to_string))
            .unwrap_or_else(|| DEFAULT_API.to_string());
        let creds = if args.no_auth {
            let host = reqwest::Url::parse(&api)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_default();
            if host != "127.0.0.1" && host != "localhost" {
                return Err(invalid("--no-auth", "--no-auth is only for the loopback dev API"));
            }
            None
        } else {
            Some(Credentials {
                token_url: args
                    .token_url
                    .clone()
                    .or_else(|| env.opt("PK_TOKEN_URL").map(str::to_string))
                    .unwrap_or_else(|| DEFAULT_TOKEN_URL.to_string()),
                client_id: env.opt("PK_WORKER_CLIENT_ID").unwrap_or("pk-worker").to_string(),
                secret: Secret::new(env.str("PK_WORKER_SECRET")?),
            })
        };
        let workspace = env
            .path("PK_WORKSPACE")
            .or_else(|| env.path("PK_CACHE_DIR").map(|p| p.join("worker")))
            .or_else(|| env.home().map(|h| h.join(".punchable-worker")))
            .unwrap_or_else(|| PathBuf::from(".punchable-worker"));
        let kinds = match &args.kinds {
            Some(s) => parse_kinds(s)?,
            None => Kind::claimable(false),
        };
        let jobs = match env.u64("PK_WORKER_JOBS")? {
            None => 1,
            Some(n) if (1..=MAX_JOBS as u64).contains(&n) => n as usize,
            Some(n) => return Err(invalid("PK_WORKER_JOBS", format!("{n} is not 1 to {MAX_JOBS}"))),
        };
        let idle =
            Duration::try_from_secs_f64(args.idle).map_err(|_| invalid("--idle", "must be a number of seconds"))?;
        let legal_rules = env.path("PUNCHABLE_LEGAL_RULES").unwrap_or_else(|| {
            env.path("PUNCHABLE_HOME")
                .or_else(|| env.home().map(|h| h.join(".punchable")))
                .unwrap_or_default()
                .join("legal_rules.json")
        });
        Ok(Self {
            api,
            creds,
            worker_name: env
                .opt("PK_WORKER_NAME")
                .or_else(|| env.opt("COMPUTERNAME"))
                .unwrap_or("home")
                .chars()
                .take(40)
                .collect(),
            once: args.once,
            idle,
            kinds,
            heartbeat: env
                .path("PK_HEARTBEAT_FILE")
                .unwrap_or_else(|| workspace.join("heartbeat")),
            metrics_port: env
                .opt("PK_METRICS_PORT")
                .and_then(|p| p.trim().parse().ok())
                .unwrap_or(DEFAULT_METRICS_PORT),
            jobs,
            names: Names::new(env.opt("PK_COMIC_NAME"), env.opt("PK_COMIC_FIRST")),
            legal_rules,
            spend_file: env
                .path("PK_AI_SPEND_FILE")
                .unwrap_or_else(|| workspace.join("ai_spend.json")),
            build_sha: env.opt("PK_BUILD_SHA").unwrap_or("dev").to_string(),
            stop_grace: STOP_GRACE,
            owner_id: env.opt("PK_OWNER_ID").unwrap_or("owner").to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Env {
        Env::from_map([("PK_WORKER_SECRET", "s"), ("PK_WORKSPACE", "/tmp/pk-worker")])
    }

    #[test]
    fn defaults_are_the_manifest_s() {
        let c = WorkerConfig::from_env(&base(), &RunArgs::default()).unwrap();
        assert_eq!(c.api, DEFAULT_API);
        assert_eq!(c.worker_name, "home");
        assert_eq!(c.heartbeat, PathBuf::from("/tmp/pk-worker/heartbeat"));
        assert_eq!(c.spend_file, PathBuf::from("/tmp/pk-worker/ai_spend.json"));
        assert_eq!((c.metrics_port, c.jobs), (9100, 1));
        assert_eq!(c.kinds, Kind::claimable(false));
        assert!(c.creds.is_some());
    }

    #[test]
    fn refusals() {
        let e = WorkerConfig::from_env(&Env::from_map([("PK_API", "http://x")]), &RunArgs::default());
        assert!(matches!(e, Err(ConfigError::Missing { .. })), "no secret");
        let args = RunArgs {
            no_auth: true,
            api: Some("https://punchable.io".into()),
            ..RunArgs::default()
        };
        assert!(WorkerConfig::from_env(&base(), &args).is_err(), "no-auth off loopback");
        let args = RunArgs {
            no_auth: true,
            api: Some("http://127.0.0.1:8796".into()),
            ..RunArgs::default()
        };
        assert!(WorkerConfig::from_env(&base(), &args).unwrap().creds.is_none());
        assert!(parse_kinds("bridge").is_err());
        assert!(parse_kinds("nope").is_err());
        assert_eq!(parse_kinds("process_set,facts,facts").unwrap(), vec![Kind::Facts]);
        let mut env = Env::from_map([
            ("PK_WORKER_SECRET", "s"),
            ("PK_WORKER_JOBS", "9"),
            ("PK_METRICS_PORT", "x"),
        ]);
        assert!(WorkerConfig::from_env(&env, &RunArgs::default()).is_err());
        env = Env::from_map([
            ("PK_WORKER_SECRET", "s"),
            ("PK_METRICS_PORT", "x"),
            ("COMPUTERNAME", "desk"),
        ]);
        let c = WorkerConfig::from_env(&env, &RunArgs::default()).unwrap();
        assert_eq!((c.metrics_port, c.worker_name.as_str()), (9100, "desk"));
    }
}
