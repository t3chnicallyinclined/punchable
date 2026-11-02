//! `pk-worker`: the hosted worker binary. `pk-worker [run] [--once] [--kinds …]` runs the loop until SIGTERM;
//! `pk-worker probe --max-age 20m` is the liveness probe (exit 1 when the heartbeat file is older, or missing).
//! The environment is read once here (`pk_core::config::Env`); everything below takes typed config.

use std::net::{Ipv4Addr, SocketAddr};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use pk_core::Env;
use pk_llm::{Budget, LlmConfig, Router};
use pk_obs::{ObsConfig, WorkerMetrics};
use pk_worker::kinds::Pages;
use pk_worker::legal::Legal;
use pk_worker::{Parts, RunArgs, SERVICE, VERSION, Worker, WorkerConfig, heartbeat, metrics_server};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Parser)]
#[command(name = "pk-worker", about = "Punchable's hosted worker", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    run: RunArgs,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the loop (the default).
    Run(RunArgs),
    /// Exit 0 when the heartbeat file is younger than `--max-age`, else 1 (the liveness probe).
    Probe {
        /// `20m`, `90s`, `1h`.
        #[arg(long, default_value = "20m")]
        max_age: String,
        /// The file (default `PK_HEARTBEAT_FILE`, else the workspace's `heartbeat`).
        #[arg(long)]
        file: Option<std::path::PathBuf>,
    },
    /// The swap's crash-report proof: a job task that panics and one failed job, both sent through the same Sentry
    /// client, panic hook and scrubber the loop uses, to `SENTRY_DSN`. Exit 1 when no DSN is set. Run by hand in the
    /// canary pod only (`docs/RUST_WORKSTREAM.md` T8 swap runbook); it claims nothing and touches no API. Without
    /// `--send` it only says where the events would go (host only): on a desk the DSN comes from `secrets.env` too.
    CrashTest {
        /// Really send the two events.
        #[arg(long)]
        send: bool,
    },
}

/// The words the crash test sends: a quoted phrase stands in for his material, which the scrubber must cut.
pub const CRASH_TEST: &str = "pk-worker crash test: a forced job panic, not a real failure ('a line of his must not \
                              arrive')";

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Probe { max_age, file }) => probe(&max_age, file),
        Some(Command::CrashTest { send }) => crash_test(send),
        Some(Command::Run(args)) => run(&args),
        None => run(&cli.run),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("pk-worker: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn probe(max_age: &str, file: Option<std::path::PathBuf>) -> anyhow::Result<ExitCode> {
    let max = heartbeat::parse_duration(max_age).map_err(anyhow::Error::msg)?;
    let file = if let Some(f) = file {
        f
    } else {
        let env = Env::from_process();
        let args = RunArgs {
            no_auth: true,
            api: Some("http://127.0.0.1".into()),
            ..RunArgs::default()
        };
        WorkerConfig::from_env(&env, &args)?.heartbeat
    };
    match heartbeat::age(&file) {
        Ok(age) if age < max => Ok(ExitCode::SUCCESS),
        Ok(age) => {
            eprintln!(
                "heartbeat {} is {}s old (max {}s)",
                file.display(),
                age.as_secs(),
                max.as_secs()
            );
            Ok(ExitCode::FAILURE)
        }
        Err(e) => {
            eprintln!("heartbeat {}: {e}", file.display());
            Ok(ExitCode::FAILURE)
        }
    }
}

fn crash_test(send: bool) -> anyhow::Result<ExitCode> {
    let env = Env::from_process();
    let obs = ObsConfig::from_env(&env)?;
    let sha = env.opt("PK_BUILD_SHA").unwrap_or("dev").to_string();
    let dsn = obs.sentry_dsn.trim();
    if dsn.is_empty() {
        eprintln!("SENTRY_DSN is not set: nothing to prove");
        return Ok(ExitCode::FAILURE);
    }
    // the host only: the key in front of `@` is never printed
    let host = reqwest::Url::parse(dsn)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "<an unparsable DSN>".into());
    if !send {
        eprintln!("would send two crash-test events to {host}; add --send to send them");
        return Ok(ExitCode::from(2));
    }
    let Some(guard) = pk_obs::init_sentry(&obs, SERVICE, VERSION, &sha) else {
        eprintln!("SENTRY_DSN is not set: nothing to prove");
        return Ok(ExitCode::FAILURE);
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let joined = rt.block_on(async {
        // a job task that panics, as the JoinSet in the loop would see one
        tokio::spawn(async { panic!("{CRASH_TEST}") }).await
    });
    let panicked = sentry::last_event_id();
    let failed = pk_worker::error::JobError::Value(CRASH_TEST.to_string());
    pk_obs::crash::report(&failed, None, None, &[("kind", "crash_test"), ("retry", "false")]);
    let reported = sentry::last_event_id();
    drop(guard); // flushes both events before the process ends
    println!(
        "crash test sent: panic event {} (task ended {}), failed-job event {}",
        panicked.map_or_else(|| "none".into(), |u| u.simple().to_string()),
        if joined.is_err() { "by panic" } else { "normally?" },
        reported.map_or_else(|| "none".into(), |u| u.simple().to_string()),
    );
    Ok(if panicked.is_some() && reported.is_some() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn run(args: &RunArgs) -> anyhow::Result<ExitCode> {
    let env = Env::from_process();
    let obs = ObsConfig::from_env(&env)?;
    pk_obs::init_tracing(obs.log_filter.as_deref())?;
    let cfg = WorkerConfig::from_env(&env, args)?;
    let _sentry = pk_obs::init_sentry(&obs, SERVICE, VERSION, &cfg.build_sha);

    let llm_cfg = match env.path("ENGINE_CONFIG") {
        Some(p) => pk_llm::Config::load(&p).with_context(|| format!("engine config {}", p.display()))?,
        None => pk_llm::Config::hosted(),
    };
    anyhow::ensure!(
        llm_cfg.providers().values().all(|p| p.kind == "anthropic"),
        "the hosted worker runs the Anthropic API only; the engine config names another provider"
    );
    let mut lc = LlmConfig::from_env(&llm_cfg, &env)?;
    lc.budget.path.clone_from(&cfg.spend_file);
    anyhow::ensure!(
        !lc.api_keys.is_empty(),
        "ANTHROPIC_API_KEY is not set: no task would have a provider"
    );
    let budget = Arc::new(Budget::from_config(&lc.budget));
    let router = Router::hosted(llm_cfg, &lc, None)?;
    let legal =
        std::fs::read_to_string(&cfg.legal_rules).map_or_else(|_| Legal::default(), |t| Legal::from_rules_json(&t));
    let metrics = Arc::new(WorkerMetrics::new(VERSION, &pk_obs::sha12(&cfg.build_sha))?);
    let api = pk_worker::api::ApiClient::new(&cfg.api, cfg.creds.clone())?;
    // the embedding kinds' model: loaded by the first embed / memolink job, then shared; a model that will not load
    // fails those jobs only (the Python's lazy `Embedder`)
    let embedder = cfg.bge_dir.clone().map(pk_worker::kinds::Model::lazy);
    let port = cfg.metrics_port;
    let once = cfg.once;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        // every job may hold a blocking thread for long work (the first Ask embeds the whole notebook) while the
        // HTTP client's DNS lookups need one too: twice the most jobs, so a full house never stalls a lookup
        .max_blocking_threads(2 * pk_worker::config::MAX_JOBS)
        .build()?;
    rt.block_on(async move {
        let stop = CancellationToken::new();
        tokio::spawn(stop_on_signal(stop.clone()));
        let listener = if port == 0 {
            None
        } else {
            let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
            let (bound, task) = metrics_server::serve(addr, Arc::clone(&metrics), stop.clone()).await?;
            tracing::info!(%bound, "metrics on /metrics");
            Some(task)
        };
        tracing::info!(
            daily_usd = budget.daily_usd(),
            spent_usd = budget.spent_today(),
            legal_markers = legal.len(),
            sqlite = pk_fts::sqlite_version(),
            "hosted: the Anthropic API"
        );
        let worker = Worker::new(Parts {
            cfg,
            api,
            router,
            budget,
            metrics,
            legal,
            pages: Pages::new()?,
            embedder,
        });
        if let Some(m) = worker.model() {
            tracing::info!(model = %m, "writes with");
        }
        let outcome = worker.run(stop.clone()).await;
        stop.cancel();
        if let Some(t) = listener {
            let _ = tokio::time::timeout(Duration::from_secs(5), t).await;
        }
        // a loop error ends only a `--once` run; the long-running loop backs off instead
        anyhow::Ok(if outcome.is_err() && once {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        })
    })
}

/// SIGTERM (the pod stopping) or Ctrl-C: cancel the token once.
async fn stop_on_signal(stop: CancellationToken) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    tracing::info!("stopping: no new claims; running jobs go back to the queue");
    stop.cancel();
}
