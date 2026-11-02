//! Punchable's API binary: read the environment once, start logging and crash reports, serve.
//!
//! Sized for one CPU (docs/ARCHITECTURE.md section 2): two runtime workers, at most four blocking threads (the index
//! build runs there).

fn main() -> anyhow::Result<()> {
    let env = pk_core::Env::from_process();
    pk_obs::init_tracing(env.opt("RUST_LOG"))?;
    let obs = pk_obs::ObsConfig::from_env(&env)?;
    let config = pk_api::ApiConfig::from_env(&env)?;
    let _sentry = pk_obs::init_sentry(&obs, "punchable-api", pk_api::VERSION, &config.build_sha);
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .build()?
        .block_on(pk_api::serve(config))
}
