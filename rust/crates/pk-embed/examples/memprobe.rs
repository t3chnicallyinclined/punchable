//! The `pk-embed` memory measurement (T15; `docs/ARCHITECTURE.md` §2 "Memory budget"): what bge-small costs a
//! process once it is loaded the way `pk-api` will load it (one intra-op thread, no arena), after one search query
//! and after 1,000 more, then after a sweeper's worth of passages in batches of eight. It prints, per phase, the
//! `Anonymous:` and `Rss:` lines of `/proc/self/smaps_rollup` on Linux (anonymous memory is what the cgroup `anon`
//! target in `docs/RUST.md` is stated in) and the resident set from the OS on every platform (the working set on
//! Windows), plus the per-call time.
//!
//! ```text
//! PK_BGE_DIR=<model dir> cargo run -p pk-embed --release --example memprobe [-- --n 1000 --threads 1 --dir <model dir>]
//! ```
//!
//! The number that sets the API's target is the Linux one, taken in the distroless image on the server
//! (`infra/README.md` "Memory"); a run on any other OS is a smoke test of the same code path.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use pk_embed::{Bge, DIM, Embedder};

/// What the command line asked for.
struct Args {
    dir: PathBuf,
    n: usize,
    threads: usize,
}

fn args() -> Result<Args> {
    let mut dir = std::env::var_os("PK_BGE_DIR").map(PathBuf::from);
    let (mut n, mut threads) = (1000, 1);
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = || it.next().with_context(|| format!("{a} needs a value"));
        match a.as_str() {
            "--dir" => dir = Some(PathBuf::from(value()?)),
            "--n" => n = value()?.parse().context("--n is a count")?,
            "--threads" => threads = value()?.parse().context("--threads is a count")?,
            other => bail!("unknown argument {other} (--dir, --n, --threads)"),
        }
    }
    let dir = dir.context("no model: set PK_BGE_DIR or pass --dir (python tools/kb/fetch_bge.py fetches it)")?;
    Ok(Args { dir, n, threads })
}

/// The process's memory now: `smaps_rollup`'s two lines in KiB (Linux only) and the resident set in bytes.
struct Memory {
    anonymous_kib: Option<u64>,
    rss_kib: Option<u64>,
    resident_bytes: Option<u64>,
}

fn smaps_rollup() -> (Option<u64>, Option<u64>) {
    let Ok(text) = std::fs::read_to_string("/proc/self/smaps_rollup") else {
        return (None, None);
    };
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(name))
            .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
    };
    (field("Anonymous:"), field("Rss:"))
}

/// One `kB` line of `/proc/self/status` (`VmHWM:` = the peak resident set), Linux only.
fn status_kib(name: &str) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix(name))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
}

fn memory(sys: &mut sysinfo::System, pid: sysinfo::Pid) -> Memory {
    let (anonymous_kib, rss_kib) = smaps_rollup();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::nothing().with_memory(),
    );
    Memory {
        anonymous_kib,
        rss_kib,
        resident_bytes: sys.process(pid).map(sysinfo::Process::memory),
    }
}

fn mib_kib(kib: Option<u64>) -> String {
    kib.map_or_else(|| "n/a".into(), |k| format!("{:.1}", k as f64 / 1024.0))
}

fn mib(bytes: Option<u64>) -> String {
    bytes.map_or_else(|| "n/a".into(), |b| format!("{:.1}", b as f64 / 1_048_576.0))
}

/// The nearest-rank percentile of sorted durations, in milliseconds.
fn pct_ms(sorted: &[Duration], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1].as_secs_f64() * 1000.0
}

fn report(phase: &str, m: &Memory, times: &mut [Duration]) {
    times.sort_unstable();
    println!(
        "{phase:<22} anon {:>7} MiB  rss {:>7} MiB  resident {:>7} MiB  calls {:>5}  p50 {:>7.2} ms  p95 {:>7.2} ms",
        mib_kib(m.anonymous_kib),
        mib_kib(m.rss_kib),
        mib(m.resident_bytes),
        times.len(),
        pct_ms(times, 50.0),
        pct_ms(times, 95.0),
    );
    if let Some(a) = m.anonymous_kib {
        println!("  Anonymous: {a} kB");
    }
}

/// A synthetic query or passage that differs per call (nothing of the comic's).
fn text(i: usize, passage: bool) -> String {
    const WORDS: [&str; 16] = [
        "parking",
        "meter",
        "airport",
        "dentist",
        "uncle",
        "wedding",
        "laundromat",
        "traffic",
        "diet",
        "gym",
        "landlord",
        "printer",
        "subway",
        "karaoke",
        "grocery",
        "neighbor",
    ];
    let w = |k: usize| WORDS[(i.wrapping_mul(7).wrapping_add(k * 5)) % WORDS.len()];
    if passage {
        format!(
            "So my {} told me the {} was closed, and I said that is the {} of every {} I have ever loved. \
             Then the {} called back about the {} and I hung up on a recording, which is the only argument I win.",
            w(0),
            w(1),
            w(2),
            w(3),
            w(4),
            w(5)
        )
    } else {
        format!("the bit about the {} and the {} {i}", w(0), w(1))
    }
}

fn main() -> Result<()> {
    let a = args()?;
    let pid = sysinfo::get_current_pid().map_err(|e| anyhow::anyhow!("no pid for this process: {e}"))?;
    let mut sys = sysinfo::System::new();
    println!(
        "memprobe: pk-embed {} threads={} n={} os={} arch={}",
        pk_embed::MODEL_NAME,
        a.threads,
        a.n,
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    report("start", &memory(&mut sys, pid), &mut []);

    let t = Instant::now();
    let bge = Bge::load_with_threads(&a.dir, a.threads).with_context(|| format!("loading {}", a.dir.display()))?;
    let load = t.elapsed();
    report("model loaded", &memory(&mut sys, pid), &mut [load]);

    let embed_one = |i: usize, query: bool| -> Result<Duration> {
        let t = Instant::now();
        let v = bge.embed(&[text(i, !query).as_str()], query)?;
        if v.len() != 1 || v[0].len() != DIM {
            bail!(
                "the model returned {} vectors of {} values",
                v.len(),
                v.first().map_or(0, Vec::len)
            );
        }
        Ok(t.elapsed())
    };
    let first = embed_one(0, true)?;
    report("after 1 query", &memory(&mut sys, pid), &mut [first]);

    let mut times = (1..=a.n).map(|i| embed_one(i, true)).collect::<Result<Vec<_>>>()?;
    report(
        &format!("after {} queries", a.n + 1),
        &memory(&mut sys, pid),
        &mut times,
    );

    // the sweeper's shape: passages, eight per model run
    let mut batches = Vec::with_capacity(a.n / pk_embed::BATCH + 1);
    for b in 0..a.n.div_ceil(pk_embed::BATCH) {
        let texts: Vec<String> = (0..pk_embed::BATCH)
            .map(|k| text(b * pk_embed::BATCH + k, true))
            .collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let t = Instant::now();
        let v = bge.embed(&refs, false)?;
        batches.push(t.elapsed());
        if v.len() != refs.len() {
            bail!("a batch of {} came back as {}", refs.len(), v.len());
        }
    }
    report(
        &format!("after {} passages", batches.len() * pk_embed::BATCH),
        &memory(&mut sys, pid),
        &mut batches,
    );

    // the worst batch: eight passages each past the 2,000-character cap, so every row pads to the token limit and
    // the activations are the largest the model ever allocates
    let long: Vec<String> = (0..pk_embed::BATCH)
        .map(|k| (0..12).map(|j| text(k * 12 + j, true)).collect::<Vec<_>>().join(" "))
        .collect();
    let refs: Vec<&str> = long.iter().map(String::as_str).collect();
    let mut worst = Vec::with_capacity(20);
    for _ in 0..20 {
        let t = Instant::now();
        bge.embed(&refs, false)?;
        worst.push(t.elapsed());
    }
    report("after 20 max batches", &memory(&mut sys, pid), &mut worst);
    if let Some(hwm) = status_kib("VmHWM:") {
        println!("peak resident (VmHWM) over the whole run: {} MiB", mib_kib(Some(hwm)));
    }
    Ok(())
}
