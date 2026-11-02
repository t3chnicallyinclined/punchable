//! What a run measured: one [`Sample`] per HTTP call, rolled up per route group and per route into [`Row`]s with
//! nearest-rank percentiles (the same definition on every run, so Python and Rust numbers compare).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

use serde::Serialize;

/// One HTTP call: where it went, how long it took end to end (the body read included), and what went wrong.
#[derive(Debug, Clone)]
pub struct Sample {
    /// The route group (`bits`, `search`, `writes`, ...).
    pub group: &'static str,
    /// The route template.
    pub route: &'static str,
    /// Time from sending the request to the last byte of the answer.
    pub elapsed: Duration,
    /// Why the call failed (a status outside 2xx, a transport error); `None` when it succeeded.
    pub error: Option<String>,
}

/// One line of the report.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    /// The group or route.
    pub name: String,
    /// Calls made.
    pub n: usize,
    /// Calls that failed.
    pub errors: usize,
    /// Median, milliseconds.
    pub p50_ms: f64,
    /// 95th percentile, milliseconds.
    pub p95_ms: f64,
    /// Slowest, milliseconds.
    pub max_ms: f64,
    /// The first failure's text, to say what kind of failure it was.
    pub first_error: Option<String>,
}

/// The nearest-rank `p`th percentile of `sorted`, in milliseconds; 0 for no samples.
pub fn percentile_ms(sorted: &[Duration], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1].as_secs_f64() * 1000.0
}

fn row(name: &str, samples: &[&Sample]) -> Row {
    let mut times: Vec<Duration> = samples.iter().map(|s| s.elapsed).collect();
    times.sort_unstable();
    Row {
        name: name.to_owned(),
        n: samples.len(),
        errors: samples.iter().filter(|s| s.error.is_some()).count(),
        p50_ms: percentile_ms(&times, 50.0),
        p95_ms: percentile_ms(&times, 95.0),
        max_ms: times.last().map_or(0.0, |d| d.as_secs_f64() * 1000.0),
        first_error: samples.iter().find_map(|s| s.error.clone()),
    }
}

/// Rows keyed by `key`, sorted by name, plus a final `all` row over every sample.
pub fn rows_by(samples: &[Sample], key: impl Fn(&Sample) -> &'static str) -> Vec<Row> {
    let mut by: BTreeMap<&'static str, Vec<&Sample>> = BTreeMap::new();
    for s in samples {
        by.entry(key(s)).or_default().push(s);
    }
    let mut out: Vec<Row> = by.iter().map(|(k, v)| row(k, v)).collect();
    out.push(row("all", &samples.iter().collect::<Vec<_>>()));
    out
}

/// The rows as an aligned text table.
pub fn table(title: &str, rows: &[Row]) -> String {
    let width = rows.iter().map(|r| r.name.len()).max().unwrap_or(4).max(title.len());
    let mut out = format!(
        "{title:<width$}  {:>6}  {:>6}  {:>9}  {:>9}  {:>9}\n",
        "n", "errors", "p50 ms", "p95 ms", "max ms"
    );
    for r in rows {
        let _ = writeln!(
            out,
            "{:<width$}  {:>6}  {:>6}  {:>9.1}  {:>9.1}  {:>9.1}{}",
            r.name,
            r.n,
            r.errors,
            r.p50_ms,
            r.p95_ms,
            r.max_ms,
            r.first_error
                .as_deref()
                .map(|e| format!("  first error: {e}"))
                .unwrap_or_default()
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn nearest_rank_percentiles() {
        let t: Vec<Duration> = (1..=100).map(ms).collect();
        assert!((percentile_ms(&t, 50.0) - 50.0).abs() < 1e-9);
        assert!((percentile_ms(&t, 95.0) - 95.0).abs() < 1e-9);
        assert!((percentile_ms(&[ms(7)], 95.0) - 7.0).abs() < 1e-9);
        assert!(percentile_ms(&[], 50.0).abs() < 1e-9);
    }

    #[test]
    fn rows_count_errors_and_end_with_all() {
        let s = |group, ms_, error: Option<&str>| Sample {
            group,
            route: "r",
            elapsed: ms(ms_),
            error: error.map(str::to_owned),
        };
        let samples = vec![s("a", 10, None), s("a", 30, Some("HTTP 500")), s("b", 20, None)];
        let rows = rows_by(&samples, |s| s.group);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "all"]);
        assert_eq!((rows[0].n, rows[0].errors), (2, 1));
        assert_eq!(rows[0].first_error.as_deref(), Some("HTTP 500"));
        assert_eq!(rows[2].n, 3);
        assert!((rows[2].max_ms - 30.0).abs() < 1e-9);
    }
}
