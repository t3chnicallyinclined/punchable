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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Probe { max_age, file }) => probe(&max_age, file),
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
        .max_blocking_threads(4)
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
