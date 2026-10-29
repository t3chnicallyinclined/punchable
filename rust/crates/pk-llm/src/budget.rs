//! The daily spend cap of the hosted provider (`engine/budget.py`): one small JSON ledger keyed by UTC day,
//! `{"2026-10-27": {"usd": 0.41, "calls": 7, "tasks": {"creative_riff": 0.3}}}`, trimmed to 14 days; list prices
//! per million tokens; `check` before a call, `charge` after it from the response's usage. A model not in the
//! table is priced as Opus so a mistake can only over-count. Nothing here ever sees a key.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::api::Usage;
use crate::error::LlmError;

/// The cap when `PK_AI_DAILY_USD` is not set.
pub const DEFAULT_DAILY_USD: f64 = 2.00;

/// USD per million tokens: input, output, cache write (1.25x input), cache read.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Prices {
    /// Uncached input.
    pub input: f64,
    /// Output.
    pub output: f64,
    /// Cache write.
    pub cache_write: f64,
    /// Cache read.
    pub cache_read: f64,
}

const fn p(input: f64, output: f64, cache_write: f64, cache_read: f64) -> Prices {
    Prices {
        input,
        output,
        cache_write,
        cache_read,
    }
}

/// Model id prefix → prices (first-party list prices, cached 2026-10-06); the longest matching prefix wins.
pub const PRICES: &[(&str, Prices)] = &[
    ("claude-opus-5-5", p(4.00, 20.00, 5.00, 0.20)),
    ("claude-sonnet-5-5", p(2.00, 10.00, 2.50, 0.20)),
    ("claude-haiku-5-5", p(0.10, 0.50, 0.125, 0.01)),
    ("claude-opus-5", p(5.00, 25.00, 6.25, 0.50)),
    ("claude-sonnet-5", p(2.00, 10.00, 2.50, 0.20)),
    ("claude-haiku-4-5", p(1.00, 5.00, 1.25, 0.10)),
];

/// The price of a model not in the table: Opus.
pub const FALLBACK_PRICE: Prices = PRICES[0].1;

/// `budget.py` `price_of`: case-insensitive prefix match, the longest prefix wins, Opus when none matches.
pub fn price_of(model: &str) -> Prices {
    let m = model.to_lowercase();
    PRICES
        .iter()
        .filter(|(k, _)| m.starts_with(k))
        .max_by_key(|(k, _)| k.len())
        .map_or(FALLBACK_PRICE, |(_, v)| *v)
}

/// USD for one response from its usage numbers (missing fields count as 0).
pub fn usage_usd(model: &str, usage: &Usage) -> f64 {
    let pr = price_of(model);
    (usage.input_tokens as f64 * pr.input
        + usage.output_tokens as f64 * pr.output
        + usage.cache_write() as f64 * pr.cache_write
        + usage.cache_read() as f64 * pr.cache_read)
        / 1_000_000.0
}

/// Today as `YYYY-MM-DD` in UTC.
pub fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// One day of the ledger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Day {
    /// Spent today.
    pub usd: f64,
    /// Calls today.
    pub calls: u64,
    /// Spent per task.
    pub tasks: BTreeMap<String, f64>,
}

/// `Budget::status()`: what the worker reports.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    /// The cap.
    pub daily_usd: f64,
    /// Spent today, 4 decimals.
    pub spent_usd: f64,
    /// Calls today.
    pub calls: u64,
    /// Spent per task today, 4 decimals.
    pub tasks: BTreeMap<String, f64>,
    /// Nothing left today.
    pub over: bool,
}

/// The cap and its ledger file.
#[derive(Debug)]
pub struct Budget {
    daily_usd: f64,
    path: PathBuf,
    pinned_day: Option<String>,
    lock: Mutex<()>,
}

impl Budget {
    /// A cap over a ledger file.
    pub fn new(daily_usd: f64, path: impl Into<PathBuf>) -> Self {
        Self {
            daily_usd,
            path: path.into(),
            pinned_day: None,
            lock: Mutex::new(()),
        }
    }

    /// The worker's budget: `PK_AI_DAILY_USD` (default 2.00); the ledger at `PK_AI_SPEND_FILE`, else
    /// `<PK_CACHE_DIR | PK_WORKSPACE | ~/.punchable-worker>/ai_spend.json`.
    pub fn from_env() -> Self {
        let daily = std::env::var("PK_AI_DAILY_USD")
            .ok()
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(DEFAULT_DAILY_USD);
        let path = std::env::var("PK_AI_SPEND_FILE")
            .ok()
            .filter(|s| !s.is_empty())
            .map_or_else(
                || {
                    let base = std::env::var("PK_CACHE_DIR")
                        .or_else(|_| std::env::var("PK_WORKSPACE"))
                        .ok()
                        .filter(|s| !s.is_empty())
                        .map(PathBuf::from)
                        .or_else(|| std::env::home_dir().map(|h| h.join(".punchable-worker")))
                        .unwrap_or_else(|| PathBuf::from(".punchable-worker"));
                    base.join("ai_spend.json")
                },
                PathBuf::from,
            );
        Self::new(daily, path)
    }

    /// Pin "today" (tests and the golden replay); the clock otherwise.
    #[must_use]
    pub fn with_day(mut self, day: impl Into<String>) -> Self {
        self.pinned_day = Some(day.into());
        self
    }

    /// Move the pinned day (the golden replay walks through days).
    pub fn set_day(&mut self, day: impl Into<String>) {
        self.pinned_day = Some(day.into());
    }

    /// The cap.
    pub fn daily_usd(&self) -> f64 {
        self.daily_usd
    }

    /// The ledger file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn today(&self) -> String {
        self.pinned_day.clone().unwrap_or_else(today)
    }

    fn read(&self) -> BTreeMap<String, Day> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn write(&self, ledger: &BTreeMap<String, Day>) -> Result<(), LlmError> {
        let io = |e: std::io::Error| LlmError::Config(format!("spend ledger {}: {e}", self.path.display()));
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let tmp = self.path.with_file_name(format!(
            "{}.tmp",
            self.path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("ai_spend.json")
        ));
        std::fs::write(&tmp, serde_json::to_string_pretty(ledger).expect("a ledger serialises")).map_err(io)?;
        std::fs::rename(&tmp, &self.path).map_err(io)
    }

    /// Spent today.
    pub fn spent_today(&self) -> f64 {
        self.read().get(&self.today()).map_or(0.0, |d| d.usd)
    }

    /// What is left today (negative when over).
    pub fn remaining(&self) -> f64 {
        self.daily_usd - self.spent_today()
    }

    /// Today's numbers.
    pub fn status(&self) -> Status {
        let day = self.read().remove(&self.today()).unwrap_or_default();
        Status {
            daily_usd: self.daily_usd,
            spent_usd: round4(day.usd),
            calls: day.calls,
            tasks: day.tasks.iter().map(|(k, v)| (k.clone(), round4(*v))).collect(),
            over: self.remaining() <= 0.0,
        }
    }

    /// The plain words the worker shows for a queued job.
    pub fn reason(&self) -> String {
        format!(
            "over today's AI budget (${:.2} of ${:.2}); queued jobs run again after midnight UTC or a higher PK_AI_DAILY_USD",
            self.spent_today(),
            self.daily_usd
        )
    }

    /// `LlmError::Budget` when the cap is spent (a cap of 0 means no spend allowed).
    pub fn check(&self) -> Result<(), LlmError> {
        if self.daily_usd <= 0.0 || self.remaining() <= 0.0 {
            return Err(LlmError::Budget(self.reason()));
        }
        Ok(())
    }

    /// Record one response's cost under today and the task; returns the USD. The ledger keeps 14 days.
    pub fn charge(&self, model: &str, usage: &Usage, task: &str) -> Result<f64, LlmError> {
        let usd = usage_usd(model, usage);
        let _guard = self.lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let today = self.today();
        let mut ledger = self.read();
        let day = ledger.entry(today.clone()).or_default();
        day.usd += usd;
        day.calls += 1;
        *day.tasks.entry(task.to_string()).or_insert(0.0) += usd;
        // the oldest days go first, today never
        let doomed: Vec<String> = ledger.keys().filter(|k| **k != today).cloned().collect();
        for k in doomed {
            if ledger.len() <= 14 {
                break;
            }
            ledger.remove(&k);
        }
        self.write(&ledger)?;
        Ok(usd)
    }
}

/// Python's `round(x, 4)` for the values a ledger holds.
fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_match_by_longest_prefix() {
        assert_eq!(price_of("claude-sonnet-5-5-20260101"), PRICES[1].1);
        assert_eq!(price_of("claude-sonnet-5"), PRICES[4].1);
        assert_eq!(price_of("CLAUDE-OPUS-5-5"), PRICES[0].1);
        assert_eq!(price_of("gpt-x"), FALLBACK_PRICE);
        let big = Usage {
            input_tokens: 1_000_000,
            ..Usage::default()
        };
        assert!((usage_usd("claude-opus-5-5", &big) - 4.0).abs() < 1e-12);
        assert!((usage_usd("claude-haiku-5-5", &big) - 0.1).abs() < 1e-12);
    }

    #[test]
    fn the_cap_is_checked_before_and_charged_after() {
        let dir = tempfile::tempdir().unwrap();
        let b = Budget::new(5.0, dir.path().join("spend.json")).with_day("2026-10-29");
        let big = Usage {
            input_tokens: 1_000_000,
            ..Usage::default()
        };
        b.check().unwrap();
        b.charge("claude-opus-5-5", &big, "label").unwrap(); // 4.00 of 5.00
        b.check().unwrap();
        b.charge("claude-opus-5-5", &big, "label").unwrap(); // 8.00: allowed, the cap is checked before
        assert!(b.status().over);
        let e = b.check().unwrap_err().to_string();
        assert!(
            e.starts_with("over today's AI budget ($8.00 of $5.00); queued jobs run again"),
            "{e}"
        );
        assert!(Budget::new(0.0, dir.path().join("zero.json")).check().is_err());
    }
}
