//! Label discipline: the functions that turn a raw path, status, job kind, model id or task name into a label value
//! that is never the comic's material and never an id, and the job-queue statistics the API's scrape-time gauges
//! read. These are the Rust homes of `api-dev/metrics.py` (`route_pattern`, `status_class`, `job_stats`) and
//! `worker/metrics.py` (`kind_label`, `model_label`, the task label).

use std::collections::BTreeMap;
use std::sync::LazyLock;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use regex::Regex;
use serde_json::Value;

/// The HTTP methods the API counts by name; anything else is `OTHER`.
pub const METHODS: [&str; 6] = ["GET", "HEAD", "POST", "PATCH", "DELETE", "OPTIONS"];
/// Path segments past this many are cut to `...`.
pub const MAX_SEGMENTS: usize = 6;

/// A plain lower-case word (the Python `^[a-z][a-z_-]{0,31}$`; Python's `$` also accepts one trailing newline).
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z_-]{0,31}\n?$").expect("a fixed regex"));

/// `/api/joke/x:ab12?q=hi` → `/api/joke/{id}`. Only plain lower-case words survive as themselves; anything with a
/// digit, a colon, an upper-case letter, a dot or more than 32 characters is an id. Paths deeper than
/// [`MAX_SEGMENTS`] are cut. The query string and fragment are never looked at.
pub fn route_pattern(path: &str) -> String {
    let p = if path.is_empty() { "/" } else { path };
    let p = p.split_once('?').map_or(p, |(a, _)| a);
    let p = p.split_once('#').map_or(p, |(a, _)| a);
    let segs: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
    let mut out: Vec<&str> = segs
        .iter()
        .take(MAX_SEGMENTS)
        .map(|s| if WORD.is_match(s) { *s } else { "{id}" })
        .collect();
    if segs.len() > MAX_SEGMENTS {
        out.push("...");
    }
    if out.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", out.join("/"))
    }
}

/// The status label: the code itself when it is a real HTTP status (100..=599), else `unknown`.
pub fn status_class(code: i64) -> String {
    if (100..=599).contains(&code) {
        code.to_string()
    } else {
        "unknown".to_string()
    }
}

/// The method label: one of [`METHODS`] or `OTHER`.
pub fn method_label(method: &str) -> &'static str {
    METHODS.iter().copied().find(|m| *m == method).unwrap_or("OTHER")
}

/// A job kind as a label: alphanumerics and underscores up to 32 characters survive; `None`/empty is `unknown`;
/// anything else is `other`.
pub fn kind_label(kind: Option<&str>) -> String {
    let k = match kind {
        Some(k) if !k.is_empty() => k,
        _ => "unknown",
    };
    let body: String = k.chars().filter(|c| *c != '_').collect();
    if !body.is_empty() && body.chars().all(char::is_alphanumeric) && k.chars().count() <= 32 {
        k.to_string()
    } else {
        "other".to_string()
    }
}

/// A model id as a label (a closed set of a few names): trimmed, lower-cased, alphanumerics and `-._:` up to 64
/// characters; anything odd collapses to `unknown`.
pub fn model_label(model: Option<&str>) -> String {
    let m = model.unwrap_or("").trim().to_lowercase();
    let ok = !m.is_empty()
        && m.chars().count() <= 64
        && m.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '.' | '_' | ':'));
    if ok { m } else { "unknown".to_string() }
}

/// A task name as a label: as given up to 40 characters, `?` when missing, `other` when longer.
pub fn task_label(task: Option<&str>) -> String {
    let t = match task {
        Some(t) if !t.is_empty() => t,
        _ => "?",
    };
    if t.chars().count() <= 40 {
        t.to_string()
    } else {
        "other".to_string()
    }
}

/// What the API's scrape-time job gauges read: queued jobs by kind, how many wait for the transcriber (the phone)
/// and the age of the oldest of those.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JobStats {
    /// Queued jobs by kind.
    pub queued: BTreeMap<String, u64>,
    /// Queued jobs with `waiting_for == "transcriber"`.
    pub waiting: u64,
    /// Seconds since the oldest waiting job was created (0 when none, or when no date parses).
    pub oldest_waiting_s: f64,
}

/// `creative.util.now_iso()` is tz-aware ISO seconds; a naive timestamp counts as UTC; anything else is unknown (0).
fn ts(v: Option<&Value>) -> f64 {
    let s = match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => return 0.0,
        Some(other) => other.to_string(),
    };
    let s = s.replace('Z', "+00:00");
    if let Ok(d) = DateTime::parse_from_rfc3339(&s) {
        return d.timestamp() as f64 + f64::from(d.timestamp_subsec_micros()) / 1e6;
    }
    let naive = NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S%.f"))
        .or_else(|_| NaiveDate::parse_from_str(&s, "%Y-%m-%d").map(|d| d.and_hms_opt(0, 0, 0).expect("midnight")));
    naive.map_or(0.0, |n| n.and_utc().timestamp() as f64)
}

/// The statistics over the job store's items (`creative_api.job_apply` shape: `status`, `kind`, `waiting_for`,
/// `created_at`), with `now` as seconds since the epoch. Items that are not objects are skipped.
pub fn job_stats(jobs: &[Value], now: f64) -> JobStats {
    let mut st = JobStats::default();
    for j in jobs {
        let Value::Object(j) = j else { continue };
        if j.get("status").and_then(Value::as_str) != Some("queued") {
            continue;
        }
        let kind = match j.get("kind") {
            Some(Value::String(k)) if !k.is_empty() => k.clone(),
            Some(Value::Number(n)) if n.as_f64() != Some(0.0) => n.to_string(),
            Some(Value::Bool(true)) => "True".to_string(),
            _ => "unknown".to_string(),
        };
        *st.queued.entry(kind).or_insert(0) += 1;
        if j.get("waiting_for").and_then(Value::as_str) == Some("transcriber") {
            st.waiting += 1;
            let t = ts(j.get("created_at"));
            if t != 0.0 {
                st.oldest_waiting_s = st.oldest_waiting_s.max(now - t);
            }
        }
    }
    st
}

/// Seconds since the Unix epoch, now.
pub fn now_secs() -> f64 {
    Utc::now().timestamp_micros() as f64 / 1e6
}
