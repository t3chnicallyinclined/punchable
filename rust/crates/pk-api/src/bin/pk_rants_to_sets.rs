//! `pk-rants-to-sets`: rants and topics become set lists (redesign slice 6, `pk_api::migrate`). Reads the same
//! environment as pk-api (run it inside the pk-api pod: `kubectl exec deploy/<pk-api> -- pk-rants-to-sets`).
//!
//! - no flag: a dry run; prints the counts only (no words of his) and writes nothing;
//! - `--list`: also prints each set list it would make, its title and how many bits (for the owner's eyes);
//! - `--write --i-have-a-backup`: writes the `create` events in one transaction. `--write` alone refuses: the
//!   owner's go and a fresh backup come first. Rollback: archive the lists it printed, by id.
//!
//! Exit codes: 0 done, 2 bad arguments, 1 anything else.

use anyhow::Context as _;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let known = ["--list", "--write", "--i-have-a-backup"];
    if let Some(a) = args.iter().find(|a| !known.contains(&a.as_str())) {
        eprintln!("unknown argument {a}; use --list, --write --i-have-a-backup");
        std::process::exit(2);
    }
    let has = |f: &str| args.iter().any(|a| a == f);
    let write = has("--write");
    if write && !has("--i-have-a-backup") {
        eprintln!("--write needs --i-have-a-backup: take a backup first (ship.sh backup-now), then run again");
        std::process::exit(2);
    }
    let env = pk_core::Env::from_process();
    pk_obs::init_tracing(env.opt("RUST_LOG").or(Some("warn")))?;
    let config = pk_api::ApiConfig::from_env(&env)?;
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .build()?
        .block_on(run(config, write, has("--list")))
}

async fn run(config: pk_api::ApiConfig, write: bool, list: bool) -> anyhow::Result<()> {
    let store = pk_store::Store::connect(config.store.clone())
        .await
        .context("connect to SurrealDB")?;
    let st = pk_api::state(config)?;
    st.set_store(Some(store));
    st.data().await.map_err(|e| anyhow::anyhow!("first load: {}", e.msg))?;
    let r = pk_api::migrate::rants_to_sets(&st, write)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.msg))?;
    println!("{}", serde_json::to_string_pretty(&r.plan.counts)?);
    if list {
        for s in &r.plan.sets {
            println!("{} · {} bits", s.title, s.joke_ids.len());
        }
    }
    if r.wrote {
        // an id carries the start of its title (his words): printed only with --list, which is for his eyes (security
        // review); the rollback is archiving these ids, or every list whose `seed` names a topic or a rant
        println!("made {} set lists", r.made.len());
        if list {
            for id in &r.made {
                println!("{id}");
            }
        }
    } else {
        println!("dry run: nothing written");
    }
    Ok(())
}
