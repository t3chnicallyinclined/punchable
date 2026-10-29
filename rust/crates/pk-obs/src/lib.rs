//! Observability for Punchable's services: logs, metrics and crash reports, with the one rule that matters here:
//! nothing the comic wrote leaves through any of them.
//!
//! - [`log`]: the `tracing` subscriber (`RUST_LOG`, stderr, compact).
//! - [`metrics`]: the `prometheus` registries of the API ([`ApiMetrics`]) and the worker ([`WorkerMetrics`]) with
//!   the exact metric names, labels, types and buckets of `api-dev/metrics.py` and `worker/metrics.py`; gauges read
//!   at scrape time are [`CallbackGauge`] collectors.
//! - [`labels`]: [`route_pattern`] and the other functions that make a label value safe (a route pattern, a status
//!   class, a job kind, a model id), and the job-queue statistics.
//! - [`scrub`]: the crash-report scrubber at the JSON level (`api-dev/scrub.py`), and [`crash`]: `sentry` with the
//!   scrubber as `before_send` / `before_breadcrumb` (`api-dev/crash.py`).
//!
//! The tests compare against goldens the Python writes (`rust/tools/gen_obs_goldens.py`) and prove by property that
//! no sensitive key survives the scrubber.

pub mod crash;
pub mod error;
pub mod labels;
pub mod log;
pub mod metrics;
pub mod scrub;

pub use crash::{before_breadcrumb, before_send, init_sentry, init_sentry_with, release};
pub use error::{ObsError, ObsResult};
pub use labels::{JobStats, job_stats, kind_label, model_label, route_pattern, status_class};
pub use log::init_tracing;
pub use metrics::{
    ApiMetrics, BudgetStatus, CONTENT_TYPE, CallbackGauge, JobTimer, Series, Usage, WorkerMetrics, render,
};
pub use scrub::{REDACTED, SENSITIVE_KEYS, scrub_event, scrub_text, scrub_value, scrub_value_by, sensitive_key};

/// The first 12 characters of a git sha for a label or a release; empty = `dev` (as the images set `PK_BUILD_SHA`).
pub fn sha12(sha: &str) -> String {
    let s = if sha.is_empty() { "dev" } else { sha };
    s.chars().take(12).collect()
}
