//! The Prometheus registries of the hosted API and the hosted worker: the exact metric names, label sets, types,
//! help strings and histogram buckets of `api-dev/metrics.py` and `worker/metrics.py`, on the `prometheus` crate.
//! Gauges that are cheaper to read than to track (queue depths, today's spend, the heartbeat's age) are custom
//! collectors computed at scrape time ([`CallbackGauge`]). Label values come from [`crate::labels`] and are never
//! the comic's material and never an id.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Instant;

use prometheus::core::{Collector, Desc};
use prometheus::proto::{Gauge as ProtoGauge, LabelPair, Metric, MetricFamily, MetricType};
use prometheus::{
    CounterVec, Histogram, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
    TextEncoder,
};
use serde_json::Value;

use crate::error::ObsResult;
use crate::labels::{
    job_stats, kind_label, method_label, model_label, now_secs, route_pattern, status_class, task_label,
};
use crate::sha12;

/// The `Content-Type` of the text exposition.
pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";
/// `engine/prom.py` `DEFAULT_BUCKETS`: HTTP request latency.
pub const DEFAULT_BUCKETS: [f64; 13] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
];
/// `pk_db_query_duration_seconds`: a `SurrealDB` round trip.
pub const DB_BUCKETS: [f64; 13] = [
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];
/// `pk_worker_job_duration_seconds`: a job's wall time.
pub const JOB_BUCKETS: [f64; 12] = [
    1.0, 2.5, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0,
];

/// What a scrape-time gauge produces: one value per label-value list (empty for a label-less gauge).
pub type Series = Vec<(Vec<String>, f64)>;

/// A gauge whose series are computed when the registry is scraped (`engine/prom.py` `Gauge(collect=...)`).
pub struct CallbackGauge {
    desc: Desc,
    collect: Arc<dyn Fn() -> Series + Send + Sync>,
}

impl std::fmt::Debug for CallbackGauge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackGauge")
            .field("name", &self.desc.fq_name)
            .finish_non_exhaustive()
    }
}

impl CallbackGauge {
    /// A gauge named `name` with `labels`, read through `collect` at every scrape.
    pub fn new(
        name: &str,
        help: &str,
        labels: &[&str],
        collect: impl Fn() -> Series + Send + Sync + 'static,
    ) -> ObsResult<Self> {
        let desc = Desc::new(
            name.to_string(),
            help.to_string(),
            labels.iter().map(ToString::to_string).collect(),
            HashMap::new(),
        )?;
        Ok(Self {
            desc,
            collect: Arc::new(collect),
        })
    }
}

impl Collector for CallbackGauge {
    fn desc(&self) -> Vec<&Desc> {
        vec![&self.desc]
    }

    fn collect(&self) -> Vec<MetricFamily> {
        let want = self.desc.variable_labels.len();
        let metrics: Vec<Metric> = (self.collect)()
            .into_iter()
            .filter(|(labels, _)| labels.len() == want)
            .map(|(labels, value)| {
                let mut pairs: Vec<LabelPair> = self
                    .desc
                    .variable_labels
                    .iter()
                    .zip(labels)
                    .map(|(n, v)| {
                        let mut p = LabelPair::default();
                        p.set_name(n.clone());
                        p.set_value(v);
                        p
                    })
                    .collect();
                pairs.sort_by(|a, b| a.name().cmp(b.name()));
                let mut g = ProtoGauge::default();
                g.set_value(value);
                let mut m = Metric::from_label(pairs);
                m.set_gauge(g);
                m
            })
            .collect();
        let mut mf = MetricFamily::default();
        mf.set_name(self.desc.fq_name.clone());
        mf.set_help(self.desc.help.clone());
        mf.set_field_type(MetricType::GAUGE);
        mf.set_metric(metrics);
        vec![mf]
    }
}

/// The text exposition of a registry.
pub fn render(registry: &Registry) -> ObsResult<String> {
    Ok(TextEncoder::new().encode_to_string(&registry.gather())?)
}

fn build_info(registry: &Registry, name: &str, help: &str, version: &str, sha: &str) -> ObsResult<IntGaugeVec> {
    let g = IntGaugeVec::new(Opts::new(name, help), &["version", "sha"])?;
    registry.register(Box::new(g.clone()))?;
    g.with_label_values(&[version, &sha12(sha)]).set(1);
    Ok(g)
}

fn shared<T>(lock: &RwLock<Option<T>>) -> Option<T>
where
    T: Clone,
{
    lock.read().unwrap_or_else(PoisonError::into_inner).clone()
}

// ------------------------------------------------------------------------------------------------------- the API

/// Where the API's scrape-time job gauges read from: the job store's items (`status`, `kind`, `waiting_for`,
/// `created_at`), as JSON objects.
pub type JobsSource = Arc<dyn Fn() -> Vec<Value> + Send + Sync>;

/// What the hosted API measures (`GET /metrics`, open inside the cluster only).
///
/// ```text
/// pk_http_requests_total{method,route,status}          counter
/// pk_http_request_duration_seconds{method,route}       histogram
/// pk_http_requests_in_flight                           gauge (the WebSocket /api/live is not counted here)
/// pk_live_clients                                      gauge: open /api/live sockets
/// pk_jobs_queued{kind}                                 gauge, read at scrape time from the job store
/// pk_jobs_waiting_transcriber                          gauge: queued jobs no worker can run (transcription = phone)
/// pk_jobs_waiting_transcriber_oldest_seconds           gauge: age of the oldest of those (0 when none)
/// pk_db_query_duration_seconds                         histogram: every SurrealDB round trip
/// pk_build_info{version,sha}                           gauge = 1
/// ```
pub struct ApiMetrics {
    registry: Registry,
    /// Requests by method, route pattern and status class.
    pub http_requests: IntCounterVec,
    /// Request latency by method and route pattern.
    pub http_duration: HistogramVec,
    /// Requests being served right now (not `/api/live`).
    pub in_flight: IntGauge,
    /// Open `/api/live` WebSocket clients.
    pub live_clients: IntGauge,
    /// `SurrealDB` round trips.
    pub db_duration: Histogram,
    /// `1`, labelled with the version and the git sha of this image.
    pub build_info: IntGaugeVec,
    jobs: Arc<RwLock<Option<JobsSource>>>,
}

impl std::fmt::Debug for ApiMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiMetrics")
            .field("metrics", &self.registry.gather().len())
            .finish_non_exhaustive()
    }
}

impl ApiMetrics {
    /// The API's registry; `version` and `sha` label `pk_build_info` (`sha` is cut to 12, empty = `dev`).
    pub fn new(version: &str, sha: &str) -> ObsResult<Self> {
        let registry = Registry::new();
        let http_requests = IntCounterVec::new(
            Opts::new(
                "pk_http_requests_total",
                "HTTP requests by method, route pattern and status",
            ),
            &["method", "route", "status"],
        )?;
        let http_duration = HistogramVec::new(
            HistogramOpts::new(
                "pk_http_request_duration_seconds",
                "HTTP request latency by route pattern",
            )
            .buckets(DEFAULT_BUCKETS.to_vec()),
            &["method", "route"],
        )?;
        let in_flight = IntGauge::new(
            "pk_http_requests_in_flight",
            "Requests being served right now (not /api/live)",
        )?;
        let live_clients = IntGauge::new("pk_live_clients", "Open /api/live WebSocket clients")?;
        let db_duration = Histogram::with_opts(
            HistogramOpts::new("pk_db_query_duration_seconds", "SurrealDB round trips (Rpc.query)")
                .buckets(DB_BUCKETS.to_vec()),
        )?;
        registry.register(Box::new(http_requests.clone()))?;
        registry.register(Box::new(http_duration.clone()))?;
        registry.register(Box::new(in_flight.clone()))?;
        registry.register(Box::new(live_clients.clone()))?;
        registry.register(Box::new(db_duration.clone()))?;
        let build_info = build_info(
            &registry,
            "pk_build_info",
            "1, labelled with the version and the git sha of this image",
            version,
            sha,
        )?;

        let jobs: Arc<RwLock<Option<JobsSource>>> = Arc::new(RwLock::new(None));
        let stats = {
            let jobs = Arc::clone(&jobs);
            move || {
                shared(&jobs)
                    .map(|src| job_stats(&src(), now_secs()))
                    .unwrap_or_default()
            }
        };
        let s = stats.clone();
        registry.register(Box::new(CallbackGauge::new(
            "pk_jobs_queued",
            "Queued jobs by kind",
            &["kind"],
            move || s().queued.into_iter().map(|(k, n)| (vec![k], n as f64)).collect(),
        )?))?;
        let s = stats.clone();
        registry.register(Box::new(CallbackGauge::new(
            "pk_jobs_waiting_transcriber",
            "Queued jobs waiting for a transcriber (the phone)",
            &[],
            move || vec![(Vec::new(), s().waiting as f64)],
        )?))?;
        registry.register(Box::new(CallbackGauge::new(
            "pk_jobs_waiting_transcriber_oldest_seconds",
            "Age of the oldest job waiting for a transcriber",
            &[],
            move || vec![(Vec::new(), stats().oldest_waiting_s)],
        )?))?;

        Ok(Self {
            registry,
            http_requests,
            http_duration,
            in_flight,
            live_clients,
            db_duration,
            build_info,
            jobs,
        })
    }

    /// The underlying registry (for a `/metrics` handler that wants the families).
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The text exposition.
    pub fn render(&self) -> ObsResult<String> {
        render(&self.registry)
    }

    /// One finished request: counted by method, route pattern and status class, and timed.
    pub fn record(&self, method: &str, path: &str, status: i64, seconds: f64) {
        let m = method_label(method);
        let r = route_pattern(path);
        self.http_requests
            .with_label_values(&[m, &r, &status_class(status)])
            .inc();
        self.http_duration.with_label_values(&[m, &r]).observe(seconds.max(0.0));
    }

    /// Time one `SurrealDB` round trip.
    pub fn time_db<T>(&self, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let out = f();
        self.db_duration.observe(t.elapsed().as_secs_f64());
        out
    }

    /// Install where the job gauges read from at scrape time (the job store's items).
    pub fn set_jobs_source(&self, f: impl Fn() -> Vec<Value> + Send + Sync + 'static) {
        *self.jobs.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(f));
    }
}

// ---------------------------------------------------------------------------------------------------- the worker

/// What the worker's spend gauges read at scrape time (`engine/budget.py` `Budget.status()`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BudgetStatus {
    /// Today's cap, `PK_AI_DAILY_USD`.
    pub daily_usd: f64,
    /// USD spent today (UTC).
    pub spent_usd: f64,
    /// `true` while the worker takes no job because today's cap is spent.
    pub over: bool,
}

/// Where the worker's spend gauges read from; `None` while no budget is attached.
pub type BudgetSource = Arc<dyn Fn() -> Option<BudgetStatus> + Send + Sync>;

/// One model call's token counts, as the Messages API reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    /// Input tokens charged at the full rate.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Input tokens read from the prompt cache.
    pub cache_read_input_tokens: u64,
    /// Input tokens written to the prompt cache.
    pub cache_creation_input_tokens: u64,
}

/// What the hosted worker measures (`GET /metrics` on its side port).
///
/// ```text
/// pk_worker_jobs_claimed_total{kind}                   counter
/// pk_worker_jobs_done_total{kind}                      counter
/// pk_worker_jobs_failed_total{kind,retry}              counter (retry="true" = handed back to the queue)
/// pk_worker_job_duration_seconds{kind}                 histogram
/// pk_ai_spend_usd_today                                gauge, from the budget at scrape time
/// pk_ai_budget_usd                                     gauge: PK_AI_DAILY_USD
/// pk_ai_paused                                         gauge: 1 while today's cap is spent
/// pk_ai_calls_total{model}                             counter
/// pk_ai_tokens_total{model,direction}                  counter: input | output | cache_read | cache_write
/// pk_ai_spend_usd_total{model,task}                    counter (USD, this process's lifetime)
/// pk_worker_heartbeat_age_seconds                      gauge: seconds since the last heartbeat (NaN before the first)
/// pk_worker_build_info{version,sha}                    gauge = 1
/// ```
pub struct WorkerMetrics {
    registry: Registry,
    /// Jobs claimed, by kind.
    pub jobs_claimed: IntCounterVec,
    /// Jobs finished with a result, by kind.
    pub jobs_done: IntCounterVec,
    /// Jobs that failed; `retry="true"` went back to the queue.
    pub jobs_failed: IntCounterVec,
    /// Job wall time by kind.
    pub job_duration: HistogramVec,
    /// Model calls by model.
    pub ai_calls: IntCounterVec,
    /// Tokens by model and direction.
    pub ai_tokens: IntCounterVec,
    /// USD charged since this process started, by model and task.
    pub ai_spend_total: CounterVec,
    /// `1`, labelled with the version and the git sha of this image.
    pub build_info: IntGaugeVec,
    budget: Arc<RwLock<Option<BudgetSource>>>,
    heartbeat_at: Arc<Mutex<Option<Instant>>>,
}

impl std::fmt::Debug for WorkerMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerMetrics")
            .field("metrics", &self.registry.gather().len())
            .finish_non_exhaustive()
    }
}

impl WorkerMetrics {
    /// The worker's registry; `version` and `sha` label `pk_worker_build_info`.
    pub fn new(version: &str, sha: &str) -> ObsResult<Self> {
        let registry = Registry::new();
        let jobs_claimed = IntCounterVec::new(
            Opts::new("pk_worker_jobs_claimed_total", "Jobs claimed, by kind"),
            &["kind"],
        )?;
        let jobs_done = IntCounterVec::new(
            Opts::new("pk_worker_jobs_done_total", "Jobs finished with a result, by kind"),
            &["kind"],
        )?;
        let jobs_failed = IntCounterVec::new(
            Opts::new(
                "pk_worker_jobs_failed_total",
                "Jobs that failed (retry=true went back to the queue)",
            ),
            &["kind", "retry"],
        )?;
        let job_duration = HistogramVec::new(
            HistogramOpts::new("pk_worker_job_duration_seconds", "Job wall time by kind").buckets(JOB_BUCKETS.to_vec()),
            &["kind"],
        )?;
        let ai_calls = IntCounterVec::new(Opts::new("pk_ai_calls_total", "Model calls by model"), &["model"])?;
        let ai_tokens = IntCounterVec::new(
            Opts::new("pk_ai_tokens_total", "Tokens by model and direction"),
            &["model", "direction"],
        )?;
        let ai_spend_total = CounterVec::new(
            Opts::new("pk_ai_spend_usd_total", "USD charged since this process started"),
            &["model", "task"],
        )?;
        for c in [&jobs_claimed, &jobs_done, &jobs_failed, &ai_calls, &ai_tokens] {
            registry.register(Box::new(c.clone()))?;
        }
        registry.register(Box::new(job_duration.clone()))?;
        registry.register(Box::new(ai_spend_total.clone()))?;
        let build_info = build_info(
            &registry,
            "pk_worker_build_info",
            "1, labelled with the version and git sha of this image",
            version,
            sha,
        )?;

        let budget: Arc<RwLock<Option<BudgetSource>>> = Arc::new(RwLock::new(None));
        let status = {
            let budget = Arc::clone(&budget);
            move || shared(&budget).and_then(|src| src()).unwrap_or_default()
        };
        let s = status.clone();
        registry.register(Box::new(CallbackGauge::new(
            "pk_ai_spend_usd_today",
            "USD spent today (UTC) per engine/budget.py",
            &[],
            move || vec![(Vec::new(), s().spent_usd)],
        )?))?;
        let s = status.clone();
        registry.register(Box::new(CallbackGauge::new(
            "pk_ai_budget_usd",
            "Today's cap, PK_AI_DAILY_USD",
            &[],
            move || vec![(Vec::new(), s().daily_usd)],
        )?))?;
        registry.register(Box::new(CallbackGauge::new(
            "pk_ai_paused",
            "1 while the worker takes no job because today's cap is spent",
            &[],
            move || vec![(Vec::new(), if status().over { 1.0 } else { 0.0 })],
        )?))?;

        let heartbeat_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let hb = Arc::clone(&heartbeat_at);
        registry.register(Box::new(CallbackGauge::new(
            "pk_worker_heartbeat_age_seconds",
            "Seconds since the loop last touched its heartbeat",
            &[],
            move || {
                let at = *hb.lock().unwrap_or_else(PoisonError::into_inner);
                vec![(Vec::new(), at.map_or(f64::NAN, |t| t.elapsed().as_secs_f64()))]
            },
        )?))?;

        Ok(Self {
            registry,
            jobs_claimed,
            jobs_done,
            jobs_failed,
            job_duration,
            ai_calls,
            ai_tokens,
            ai_spend_total,
            build_info,
            budget,
            heartbeat_at,
        })
    }

    /// The underlying registry.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The text exposition.
    pub fn render(&self) -> ObsResult<String> {
        render(&self.registry)
    }

    /// The loop touched its heartbeat.
    pub fn heartbeat(&self) {
        *self.heartbeat_at.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    }

    /// Time since the last heartbeat; `None` before the first (what `/healthz` answers 503 on).
    pub fn heartbeat_age(&self) -> Option<std::time::Duration> {
        self.heartbeat_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .map(|t| t.elapsed())
    }

    /// Read the spend gauges from this source at scrape time.
    pub fn attach_budget(&self, f: impl Fn() -> Option<BudgetStatus> + Send + Sync + 'static) {
        *self.budget.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(f));
    }

    /// One model call charged: calls, tokens by direction and USD per model and task (`Budget.listeners`).
    pub fn on_charge(&self, model: Option<&str>, usage: &Usage, usd: f64, task: Option<&str>) {
        let m = model_label(model);
        self.ai_calls.with_label_values(&[&m]).inc();
        for (direction, n) in [
            ("input", usage.input_tokens),
            ("output", usage.output_tokens),
            ("cache_read", usage.cache_read_input_tokens),
            ("cache_write", usage.cache_creation_input_tokens),
        ] {
            if n != 0 {
                self.ai_tokens.with_label_values(&[m.as_str(), direction]).inc_by(n);
            }
        }
        self.ai_spend_total
            .with_label_values(&[&m, &task_label(task)])
            .inc_by(usd.max(0.0));
    }

    /// Start timing a job: counted as claimed now; `done` or `failed` observes its duration.
    pub fn job(&self, kind: Option<&str>) -> JobTimer<'_> {
        let kind = kind_label(kind);
        self.jobs_claimed.with_label_values(&[&kind]).inc();
        JobTimer {
            metrics: self,
            kind,
            t0: Instant::now(),
        }
    }
}

/// `worker/metrics.py` `JobTimer`: claimed at creation, the duration observed once, at `done` or `failed`. Dropping
/// a timer without either (a panic unwinding through a job) leaves the claim counted and nothing else.
#[must_use = "a job is counted done or failed by calling one of those"]
#[derive(Debug)]
pub struct JobTimer<'a> {
    metrics: &'a WorkerMetrics,
    kind: String,
    t0: Instant,
}

impl JobTimer<'_> {
    /// The kind label this job is counted under.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    fn close(&self) {
        self.metrics
            .job_duration
            .with_label_values(&[&self.kind])
            .observe(self.t0.elapsed().as_secs_f64());
    }

    /// The job finished with a result.
    pub fn done(self) {
        self.close();
        self.metrics.jobs_done.with_label_values(&[&self.kind]).inc();
    }

    /// The job failed; `retry` says it went back to the queue.
    pub fn failed(self, retry: bool) {
        self.close();
        self.metrics
            .jobs_failed
            .with_label_values(&[self.kind.as_str(), if retry { "true" } else { "false" }])
            .inc();
    }
}
