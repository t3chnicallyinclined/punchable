//! `pk-load`: the load script the API's memory and latency numbers are taken with (`infra/README.md` "Memory",
//! `docs/RUST.md` step 0, T17). It times the cold `/api/map/cloud`, then replays a fixed, weighted mix of the app's
//! GET routes (and, against a scratch API only, a few writes) with bounded concurrency, and reports p50 / p95 / max
//! and errors per route group and per route. With `--pid` it also samples the API process's resident memory (same
//! machine only): idle, after the cold cloud, the peak under load, and after. The same command runs against the
//! Python API and, later, the Rust one, so the two are measured the same way.
//!
//! ```text
//! pk-load --base http://127.0.0.1:8787 --pid 1234 --writes           (a scratch API: tools/load/scratch_api.py)
//! pk-load --base https://punchable.io --token-env PK_TOKEN            (production: reads only; the owner's call)
//! ```

mod mem;
mod plan;
mod stats;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode, Url};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::task::JoinSet;

use plan::{Ids, Step};
use stats::{Row, Sample};

/// Replay the app's read mix against a Punchable API and report latency per route group (and memory with `--pid`).
#[derive(Debug, Parser)]
#[command(version)]
struct Args {
    /// The API's base URL, without `/api` (`http://127.0.0.1:8787`, `https://punchable.io`).
    #[arg(long)]
    base: Url,
    /// The name of an environment variable holding an owner bearer token (production); the value is never printed.
    #[arg(long)]
    token_env: Option<String>,
    /// Requests in flight at once.
    #[arg(long, default_value_t = 8)]
    concurrency: usize,
    /// Steps in the mix (a board cycle is one step of four calls).
    #[arg(long, default_value_t = 2000)]
    requests: usize,
    /// Mix in writes (a jot, a set list created / archived / unarchived / deleted): one step in 20. Refused unless
    /// the base URL is loopback: writes are for a scratch database.
    #[arg(long, conflicts_with = "token_env")]
    writes: bool,
    /// The API's process id on this machine: sample its resident memory.
    #[arg(long)]
    pid: Option<u32>,
    /// Search words, comma-separated (the default is generic words, nothing of the comic's).
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "dog,airport,parking,mom,wedding,traffic,dentist,phone"
    )]
    queries: Vec<String>,
    /// Per-request timeout, seconds.
    #[arg(long, default_value_t = 60)]
    timeout_s: u64,
    /// Skip the cold `/api/map/cloud` call (it is also in the mix, warm).
    #[arg(long)]
    no_cloud: bool,
    /// Write the report as JSON here too.
    #[arg(long)]
    json: Option<std::path::PathBuf>,
    /// Failed calls tolerated before the run exits non-zero (a run of errors is not a measurement).
    #[arg(long, default_value_t = 0)]
    max_errors: usize,
}

/// Why one call failed.
#[derive(Debug, thiserror::Error)]
enum CallError {
    /// The server answered outside 2xx.
    #[error("HTTP {0}")]
    Status(StatusCode),
    /// The request never got an answer (connect, timeout, reset).
    #[error("transport: {0}")]
    Transport(#[from] reqwest::Error),
    /// A write's answer did not carry what the next step needs.
    #[error("the answer has no {0}")]
    Shape(&'static str),
}

/// The HTTP side: one pooled client and the base URL.
#[derive(Clone)]
struct Api {
    http: reqwest::Client,
    base: Url,
}

impl Api {
    fn new(base: Url, token: Option<String>, timeout: Duration, pool: usize) -> Result<Self> {
        let mut headers = HeaderMap::new();
        if let Some(t) = token {
            let mut v = HeaderValue::from_str(&format!("Bearer {t}")).context("the token is not a header value")?;
            v.set_sensitive(true);
            headers.insert(AUTHORIZATION, v);
        }
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(timeout)
            .pool_max_idle_per_host(pool)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("pk-load/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("building the HTTP client")?;
        Ok(Self { http, base })
    }

    /// `/api/<segments>?<query>` on the base URL, each part percent-encoded.
    /// `main` checked the base (`check_base`), so it always takes path segments.
    fn url(&self, segments: &[String], query: &[(&str, String)]) -> Url {
        let mut url = self.base.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().push("api").extend(segments);
        }
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        url
    }

    /// One call, timed to the last byte of the body.
    async fn call(&self, method: Method, url: Url, body: Option<&Value>) -> (Duration, Result<Vec<u8>, CallError>) {
        let t = Instant::now();
        let mut req = self.http.request(method, url);
        if let Some(b) = body {
            req = req.json(b);
        }
        let out = async {
            let resp = req.send().await?;
            let status = resp.status();
            let bytes = resp.bytes().await?;
            if status.is_success() {
                Ok(bytes.to_vec())
            } else {
                Err(CallError::Status(status))
            }
        }
        .await;
        (t.elapsed(), out)
    }

    /// A GET whose JSON answer the run needs (health, the id lists, the cloud).
    async fn get_json(&self, segments: &[&str], query: &[(&str, String)]) -> Result<(Duration, Value)> {
        let segs: Vec<String> = segments.iter().map(|s| (*s).to_owned()).collect();
        let url = self.url(&segs, query);
        let (took, out) = self.call(Method::GET, url.clone(), None).await;
        let bytes = out.with_context(|| format!("GET {}", url.path()))?;
        let v = serde_json::from_slice(&bytes).with_context(|| format!("GET {}: not JSON", url.path()))?;
        Ok((took, v))
    }
}

fn sample(group: &'static str, route: &'static str, took: Duration, out: Result<Vec<u8>, CallError>) -> Sample {
    Sample {
        group,
        route,
        elapsed: took,
        error: out.err().map(|e| e.to_string()),
    }
}

/// Run one step of the plan; a board cycle stops at its first failure.
async fn run_step(api: &Api, step: &Step, samples: &mut Vec<Sample>) {
    match step {
        Step::Get {
            group,
            route,
            segments,
            query,
        } => {
            let url = api.url(segments, query);
            let (took, out) = api.call(Method::GET, url, None).await;
            samples.push(sample(group, route, took, out));
        }
        Step::Jot { n } => {
            let body = json!({"text": format!("load test jot {n}: the parking meter that talks back at the airport")});
            let (took, out) = api
                .call(Method::POST, api.url(&["ideas".into()], &[]), Some(&body))
                .await;
            samples.push(sample("writes", "POST /api/ideas", took, out));
        }
        Step::BoardCycle { n, jokes } => {
            let body = json!({"title": format!("Load test list {n}"), "joke_ids": jokes});
            let (took, out) = api
                .call(Method::POST, api.url(&["boards".into()], &[]), Some(&body))
                .await;
            // the answer must carry the new board's id; without it the cycle is an error, not a shorter cycle
            let out = out.and_then(|bytes| {
                serde_json::from_slice::<Value>(&bytes)
                    .ok()
                    .and_then(|v| v["board"]["id"].as_str().map(str::to_owned))
                    .ok_or(CallError::Shape("board.id"))
            });
            let (out, id) = match out {
                Ok(id) => (Ok(Vec::new()), Some(id)),
                Err(e) => (Err(e), None),
            };
            samples.push(sample("writes", "POST /api/boards", took, out));
            let Some(id) = id else { return };
            for (route, patch) in [
                ("PATCH /api/boards/{id} archive", json!({"archive": true})),
                ("PATCH /api/boards/{id} unarchive", json!({"unarchive": true})),
                ("PATCH /api/boards/{id} delete", json!({"delete": true})),
            ] {
                let (took, out) = api
                    .call(
                        Method::PATCH,
                        api.url(&["boards".into(), id.clone()], &[]),
                        Some(&patch),
                    )
                    .await;
                let failed = out.is_err();
                samples.push(sample("writes", route, took, out));
                if failed {
                    break;
                }
            }
        }
    }
}

/// The mix with `concurrency` workers pulling the next step from a shared cursor: at most that many in flight.
async fn run_mix(api: &Api, plan: Vec<Step>, concurrency: usize) -> Result<Vec<Sample>> {
    let plan = Arc::new(plan);
    let next = Arc::new(AtomicUsize::new(0));
    let mut workers = JoinSet::new();
    for _ in 0..concurrency.max(1) {
        let (api, plan, next) = (api.clone(), plan.clone(), next.clone());
        workers.spawn(async move {
            let mut samples = Vec::new();
            while let Some(step) = plan.get(next.fetch_add(1, Ordering::Relaxed)) {
                run_step(&api, step, &mut samples).await;
            }
            samples
        });
    }
    let mut all = Vec::with_capacity(plan.len());
    while let Some(done) = workers.join_next().await {
        all.extend(done.context("a load worker panicked")?);
    }
    Ok(all)
}

/// The cold cloud map: the first call times PCA + k-means (when the API's cache is empty), the second the cache.
#[derive(Debug, Serialize)]
struct Cloud {
    first_ms: f64,
    second_ms: f64,
    nodes: usize,
    clusters: usize,
}

async fn cloud(api: &Api) -> Result<Cloud> {
    let q = [("cap", "1600".to_string())];
    let (first, v) = api.get_json(&["map", "cloud"], &q).await?;
    let (second, _) = api.get_json(&["map", "cloud"], &q).await?;
    let count = |k: &str| v.get(k).and_then(Value::as_array).map_or(0, Vec::len);
    Ok(Cloud {
        first_ms: first.as_secs_f64() * 1000.0,
        second_ms: second.as_secs_f64() * 1000.0,
        nodes: count("nodes"),
        clusters: count("clusters"),
    })
}

/// Resident memory of the API process at each point, MiB (`--pid` only).
#[derive(Debug, Default, Serialize)]
#[allow(clippy::struct_field_names)] // the unit is in every name on purpose: the JSON is read by people
struct Memory {
    idle_mib: Option<f64>,
    after_cloud_mib: Option<f64>,
    peak_mib: Option<f64>,
    after_mib: Option<f64>,
}

#[derive(Debug, Serialize)]
struct Report {
    at: String,
    base: String,
    concurrency: usize,
    steps: usize,
    calls: usize,
    writes: bool,
    wall_s: f64,
    calls_per_s: f64,
    ids: [usize; 3],
    cloud: Option<Cloud>,
    memory: Memory,
    groups: Vec<Row>,
    routes: Vec<Row>,
}

fn to_mib(b: Option<u64>) -> Option<f64> {
    b.map(|b| (b as f64 / 1_048_576.0 * 10.0).round() / 10.0)
}

/// The base must be a plain `scheme://host[:port][/path]`: no credentials (they would be printed and saved in the
/// report), no query or fragment, and a URL that takes path segments.
fn check_base(base: &Url) -> Result<()> {
    ensure!(
        base.scheme() == "http" || base.scheme() == "https",
        "--base must be http(s)"
    );
    ensure!(!base.cannot_be_a_base(), "--base {base} cannot take a path");
    ensure!(
        base.username().is_empty() && base.password().is_none(),
        "--base carries credentials: pass a token with --token-env instead"
    );
    ensure!(
        base.query().is_none() && base.fragment().is_none(),
        "--base has a query or a fragment"
    );
    Ok(())
}

fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else { return false };
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[tokio::main]
async fn main() -> Result<()> {
    let a = Args::parse();
    ensure!(
        a.concurrency >= 1 && a.requests >= 1,
        "--concurrency and --requests are at least 1"
    );
    check_base(&a.base)?;
    if a.writes && !is_loopback(&a.base) {
        bail!(
            "--writes is for a scratch API on this machine; {} is not loopback",
            a.base.host_str().unwrap_or("?")
        );
    }
    let token = match &a.token_env {
        Some(name) => {
            Some(std::env::var(name).with_context(|| format!("--token-env {name}: the variable is not set"))?)
        }
        None => None,
    };
    if token.is_some() && a.base.scheme() == "http" && !is_loopback(&a.base) {
        bail!(
            "refusing to send a token over plain http to {}",
            a.base.host_str().unwrap_or("?")
        );
    }
    let api = Api::new(a.base.clone(), token, Duration::from_secs(a.timeout_s), a.concurrency)?;
    let mut probe = a.pid.map(mem::Probe::new);
    let mut memory = Memory::default();

    let (_, health) = api
        .get_json(&["health"], &[])
        .await
        .context("the API does not answer /api/health")?;
    println!(
        "pk-load: {} answers (rev {}), {} in flight, {} steps{}",
        a.base,
        health.get("rev").and_then(Value::as_str).unwrap_or("?"),
        a.concurrency,
        a.requests,
        if a.writes { ", writes on" } else { "" }
    );
    if let Some(p) = probe.as_mut() {
        memory.idle_mib = to_mib(p.resident());
    }

    let cloud = if a.no_cloud {
        None
    } else {
        let c = cloud(&api).await.context("the cold /api/map/cloud")?;
        println!(
            "cloud map: first call {:.0} ms, second {:.0} ms ({} bits, {} groups)",
            c.first_ms, c.second_ms, c.nodes, c.clusters
        );
        Some(c)
    };
    if let Some(p) = probe.as_mut() {
        memory.after_cloud_mib = to_mib(p.resident());
    }

    let limit = [("sort", "recent".to_string()), ("limit", "500".to_string())];
    let ids = Ids {
        jokes: Ids::of(&api.get_json(&["jokes"], &limit).await?.1, "items"),
        sets: Ids::of(&api.get_json(&["sets"], &[]).await?.1, "sets"),
        boards: Ids::of(&api.get_json(&["boards"], &[]).await?.1, "boards"),
    };
    println!(
        "ids: {} bits, {} sets, {} set lists",
        ids.jokes.len(),
        ids.sets.len(),
        ids.boards.len()
    );
    ensure!(
        !ids.jokes.is_empty(),
        "GET /api/jokes listed no bits: nothing to spread the reads over"
    );

    let plan = plan::build(&ids, &a.queries, a.requests, a.writes);
    // open every pooled connection first, so the mix's first calls do not carry connect (and TLS) time
    let mut warm = JoinSet::new();
    for _ in 0..a.concurrency {
        let api = api.clone();
        warm.spawn(async move { api.get_json(&["health"], &[]).await.map(|_| ()) });
    }
    while let Some(w) = warm.join_next().await {
        w.context("a warm-up call panicked")??;
    }
    let peak = probe.take().map(|p| mem::Peak::start(p, Duration::from_millis(100)));
    let t = Instant::now();
    let samples = run_mix(&api, plan, a.concurrency).await?; // on an error the dropped `Peak` stops its sampler
    let wall = t.elapsed();
    if let Some(peak) = peak {
        let (bytes, mut p) = peak.finish().await.context("the memory sampler")?;
        memory.peak_mib = to_mib((bytes > 0).then_some(bytes));
        tokio::time::sleep(Duration::from_secs(2)).await;
        memory.after_mib = to_mib(p.resident());
    }

    let groups = stats::rows_by(&samples, |s| s.group);
    let routes = stats::rows_by(&samples, |s| s.route);
    print!(
        "\n{}\n{}",
        stats::table("group", &groups),
        stats::table("route", &routes)
    );
    let calls_per_s = samples.len() as f64 / wall.as_secs_f64().max(1e-9);
    println!(
        "\n{} calls in {:.1} s ({calls_per_s:.0}/s)",
        samples.len(),
        wall.as_secs_f64()
    );
    if a.pid.is_some() {
        let f = |v: Option<f64>| v.map_or_else(|| "n/a".into(), |m| format!("{m:.1}"));
        println!(
            "memory (resident, MiB): idle {}  after the cold cloud {}  peak under load {}  after {}",
            f(memory.idle_mib),
            f(memory.after_cloud_mib),
            f(memory.peak_mib),
            f(memory.after_mib)
        );
    }
    let errors = samples.iter().filter(|s| s.error.is_some()).count();
    let report = Report {
        at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        base: a.base.to_string(),
        concurrency: a.concurrency,
        steps: a.requests,
        calls: samples.len(),
        writes: a.writes,
        wall_s: wall.as_secs_f64(),
        calls_per_s,
        ids: [ids.jokes.len(), ids.sets.len(), ids.boards.len()],
        cloud,
        memory,
        groups,
        routes,
    };
    if let Some(path) = &a.json {
        let text = serde_json::to_string_pretty(&report).context("the report as JSON")?;
        std::fs::write(path, text + "\n").with_context(|| format!("writing {}", path.display()))?;
        println!("report: {}", path.display());
    }
    ensure!(
        errors <= a.max_errors,
        "{errors} of {} calls failed (see the first error per row); --max-errors is {}",
        report.calls,
        a.max_errors
    );
    Ok(())
}
