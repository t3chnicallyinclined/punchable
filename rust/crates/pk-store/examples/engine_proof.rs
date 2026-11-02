//! The engine fork's offline proof runner (docs/adr/0013-fork-the-upstream-engine.md, step 3): questions in, the
//! fork's packets and timings out, against a SCRATCH `SurrealDB` on this machine that a loader filled from an export.
//! It can only reach `127.0.0.1` (the URL is built from a port, never read), so it cannot touch the hosted database.
//!
//! ```text
//! PK_PROOF_PASS=<scratch root password> cargo run -p pk-store --example engine_proof --release -- \
//!     <port> <ns> <db> <questions.json> <out.json> <bge dir> [repeats]
//! ```
//!
//! `questions.json` is `[{"id": "q01", "q": "..."}]`; keep it outside the repository when it quotes his material.
//! Per question: one embed (bge query form), then `pk_engine::ask::open` under the default config `repeats` times
//! (the first is reported apart as cold), the same read ordered by every ladder arm under Punchable's caps, and a
//! words-only open (no vector) for the latency of the full-text legs alone.

use std::path::PathBuf;
use std::time::Instant;

use pk_embed::Bge;
use pk_embed::Embedder;
use pk_engine::rank_config::{RankConfig, ladder_arms};
use pk_engine::{EngineStore, ReadQuery, ask, pack};
use pk_store::{Auth, Config, Store};
use serde_json::{Value, json};

fn us(t: Instant) -> u64 {
    u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX)
}

fn median(mut v: Vec<u64>) -> u64 {
    v.sort_unstable();
    v.get(v.len() / 2).copied().unwrap_or(0)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() >= 6,
        "usage: engine_proof <port> <ns> <db> <questions.json> <out.json> <bge dir> [repeats]"
    );
    let port: u16 = args[0].parse()?;
    let (ns, db) = (args[1].clone(), args[2].clone());
    // A scratch marker on top of the loopback-only URL: the proof never opens a namespace that is not a proof's.
    anyhow::ensure!(
        ns.starts_with("proof"),
        "the namespace must start with `proof` (scratch only), not {ns:?}"
    );
    let questions: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&args[3])?)?;
    let out_path = PathBuf::from(&args[4]);
    let bge_dir = PathBuf::from(&args[5]);
    let repeats: usize = args.get(6).map_or(Ok(5), |r| r.parse())?;
    let pass = std::env::var("PK_PROOF_PASS").map_err(|_| anyhow::anyhow!("PK_PROOF_PASS (the scratch password)"))?;

    let cfg = Config::new(
        format!("ws://127.0.0.1:{port}"),
        Auth::Root {
            user: "root".into(),
            pass,
        },
        ns,
        db,
    );
    let store = Store::connect(cfg).await?;
    let version = store.check_version().await?;
    // The engine tables on the scratch namespace, so every open reads lessons (none yet) as it would live.
    store.apply_engine_schema().await?;
    let bge = tokio::task::spawn_blocking(move || Bge::load(&bge_dir)).await??;
    let bge = std::sync::Arc::new(bge);
    let rank = RankConfig::default();
    let arms = ladder_arms(&rank.ladder);

    let mut rows = Vec::new();
    for q in &questions {
        let id = q["id"].as_str().unwrap_or_default().to_owned();
        let text = q["q"].as_str().unwrap_or_default().to_owned();
        let (embed_us, vector) = {
            let bge = std::sync::Arc::clone(&bge);
            let t = text.clone();
            tokio::task::spawn_blocking(move || {
                let start = Instant::now();
                let v = bge.embed(&[t.as_str()], true);
                (us(start), v)
            })
            .await?
        };
        let vector = vector?.into_iter().next().unwrap_or_default();
        let mut totals = Vec::new();
        let mut reads = Vec::new();
        let mut opened = None;
        for _ in 0..repeats.max(1) {
            let o = ask::open(&store, &text, Some(&vector), &rank).await?;
            totals.push(o.packed.trace.us.get("total").copied().unwrap_or(0));
            reads.push(o.packed.trace.us.get("read").copied().unwrap_or(0));
            opened = Some(o);
        }
        let opened = opened.ok_or_else(|| anyhow::anyhow!("no open ran"))?;
        let words_start = Instant::now();
        let words_only = ask::open(&store, &text, None, &rank).await?;
        let words_us = us(words_start);

        // Every ladder arm over the same read, under Punchable's caps.
        let read = store
            .read(ReadQuery {
                question: text.trim(),
                vector: Some(&vector),
                depth: rank.read_depth,
            })
            .await?;
        let mut by_arm = serde_json::Map::new();
        for (name, order) in &arms {
            let c = RankConfig {
                order: order.clone(),
                ..rank.clone()
            };
            let p = pack::pack(text.trim(), c.max_hits, vec![], read.hits.clone(), &c);
            by_arm.insert(
                (*name).to_owned(),
                json!(p.hits.iter().map(|h| h.key.clone()).collect::<Vec<_>>()),
            );
        }
        by_arm.insert(
            "words-only-default".to_owned(),
            json!(words_only.packed.hits.iter().map(|h| h.key.clone()).collect::<Vec<_>>()),
        );
        rows.push(json!({
            "id": id,
            "packet": opened.candidates.iter().map(|c| c.key.clone()).collect::<Vec<_>>(),
            "arms": by_arm,
            "pool": read.hits.len(),
            "us": {
                "embed": embed_us,
                "open_cold": totals.first().copied().unwrap_or(0),
                "open_warm_median": median(totals.iter().skip(1).copied().collect()),
                "read_warm_median": median(reads.iter().skip(1).copied().collect()),
                "words_only_open": words_us,
            },
        }));
    }
    let out = json!({
        "surreal": format!("{}.{}.{}", version.0, version.1, version.2),
        "config": rank.sha256(),
        "repeats": repeats,
        "questions": rows,
    });
    std::fs::write(&out_path, serde_json::to_string_pretty(&out)?)?;
    println!("wrote {} questions to {}", questions.len(), out_path.display());
    Ok(())
}
