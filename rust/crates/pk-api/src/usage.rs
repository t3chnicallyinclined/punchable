//! The usage meter (SPEC 2026-11-17 §13, phase 1): how much the account uses Punchable and what it costs.
//!
//! Three tables, pk-api's own (`db/schema_usage.surql`): `usage_call` (one row per model call the worker's budget
//! charged, posted by the worker to `POST /api/worker/usage` and priced HERE from the one price table,
//! `pk_core::prices`), `usage_media` (one row per transcript or recording received: seconds and bytes) and
//! `usage_day` (the daily rollup: the activity counters and the day's AI totals).
//!
//! Off the request path (owner rule: observability costs nothing on the hot path): a request only adds to a counter
//! in memory ([`Meter::note`], one short lock); [`run`] writes the counters once a minute and recomputes the AI totals
//! of the days that moved. A media row is written by a task the route spawns after it answered. Prometheus reads the
//! last rollup's aggregates, never per account and never from the database at scrape time.
//!
//! Privacy: counts, kinds, model names and money only. Never a prompt, an answer, a title or a line of his material.
//! The one pointer kept is the job id (`job:<kind>-<12 hex of a hash>`, `writes/jobs.rs`): it names the queue's row,
//! which holds that job's input and result in his own database, so a cost can be traced to the job that spent it. It
//! is never in the report, the export, a metric or a log line.
//!
//! Single tenant until the per-user slice (SPEC §6): the account is `PK_OWNER_ID`. Phase 2 keys the same rows by the
//! account of the token's `sub`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use chrono::{Datelike, NaiveDate};
use pk_store::{Store, StoreError, from_db, to_db, vars};
use pk_wire::worker::usage::{MAX_BATCH, UsageCall};
use serde_json::{Map, Value, json};

use crate::error::ApiError;
use crate::state::AppState;

/// How often [`run`] flushes, when `PK_USAGE_FLUSH_S` is not set.
pub const DEFAULT_EVERY: Duration = Duration::from_secs(60);
/// How often the storage sample (rows and bytes) is taken.
pub const STORAGE_EVERY: Duration = Duration::from_secs(3600);
/// The longest range `GET /api/admin/usage` answers.
pub const MAX_RANGE_DAYS: i64 = 400;
/// The lead's assumption for "what would he have to charge": a 70% gross margin on the AI cost alone (hosting,
/// Apple's cut and taxes not counted). Labelled as an assumption wherever it is shown.
pub const MARGIN: f64 = 0.70;

/// The activity of one day, as counted in memory and added to `usage_day`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// App requests answered (2xx/3xx, not health, not the worker's, not the meter's own page).
    pub requests: u64,
    /// Of those, writes.
    pub writes: u64,
    /// Bits and versions saved, jots.
    pub bits: u64,
    /// Redline rounds sent.
    pub redlines: u64,
    /// AI jobs asked for.
    pub drafts: u64,
    /// `/api/live` sockets opened (the app coming forward).
    pub sessions: u64,
}

impl Counts {
    fn add(&mut self, o: &Self) {
        self.requests += o.requests;
        self.writes += o.writes;
        self.bits += o.bits;
        self.redlines += o.redlines;
        self.drafts += o.drafts;
        self.sessions += o.sessions;
    }
}

/// Routes that save a bit, a version or a jot (POST).
const BIT_ROUTES: [&str; 2] = ["/api/ideas", "/api/jokes/{id}/versions"];
/// Routes that send a Redline round (POST).
const REDLINE_ROUTES: [&str; 1] = ["/api/jokes/{id}/redline"];
/// Routes that ask the AI for something (POST): each queues a job the worker charges for.
const DRAFT_ROUTES: [&str; 12] = [
    "/api/riff",
    "/api/facts",
    "/api/ask",
    "/api/ask/chats",
    "/api/ask/chats/{id}",
    "/api/jokes/{id}/punchup",
    "/api/jokes/{id}/rant",
    "/api/jokes/{id}/hook",
    "/api/rant/draft",
    "/api/rant/line",
    "/api/workingsets/{id}/bits/{id}/punchup",
    "/api/jokes/{id}/redline",
];

/// Whether `path` is `tpl` with each `{id}` standing for exactly one segment.
fn matches(tpl: &str, path: &str) -> bool {
    let mut t = tpl.split('/');
    let mut p = path.split('/');
    loop {
        match (t.next(), p.next()) {
            (None, None) => return true,
            (Some("{id}"), Some(s)) if !s.is_empty() => {}
            (Some(a), Some(b)) if a == b => {}
            _ => return false,
        }
    }
}

/// What one answered request counts as, or nothing.
pub fn classify(method: &str, path: &str, status: u16) -> Option<Counts> {
    if !(200..400).contains(&status) || matches!(method, "OPTIONS" | "HEAD") {
        return None;
    }
    let path = path.split_once('?').map_or(path, |(a, _)| a);
    if path == "/api/health"
        || path == "/api/live"
        || path == "/metrics"
        || path.starts_with("/api/worker/")
        || path.starts_with("/api/admin/")
    {
        return None;
    }
    let post = method == "POST";
    let any = |list: &[&str]| post && list.iter().any(|t| matches(t, path));
    Some(Counts {
        requests: 1,
        writes: u64::from(matches!(method, "POST" | "PUT" | "PATCH" | "DELETE")),
        bits: u64::from(any(&BIT_ROUTES)),
        redlines: u64::from(any(&REDLINE_ROUTES)),
        drafts: u64::from(any(&DRAFT_ROUTES)),
        sessions: 0,
    })
}

/// A job kind as the meter keeps it: one the queue knows, else `other` (a bounded label for Prometheus).
pub fn kind_of(kind: Option<&str>) -> &'static str {
    kind.and_then(|k| {
        pk_creative::reducers::jobs::JOB_KINDS
            .iter()
            .find(|j| **j == k)
            .copied()
    })
    .unwrap_or("other")
}

/// A model or task name as stored: at most 64 characters of `[A-Za-z0-9._-]`, else `other`.
fn name_of(s: &str) -> String {
    if !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        s.to_string()
    } else {
        "other".to_string()
    }
}

/// How far from today a posted call's day may be (a worker that was cut off posts late; never ahead of the clock).
const DAYS_BACK: i64 = 7;

/// When a posted call happened, as stored: its own time when it is an RFC 3339 instant within [`DAYS_BACK`] days
/// before `now` and less than a day ahead, written canonically (UTC, seconds); else the time it was received. The day
/// is that instant's UTC day, so a worker can never make a row land on a day of its choosing.
fn when_of(at: &str, now: chrono::DateTime<chrono::Utc>) -> (String, String) {
    let parsed = (at.len() <= 40)
        .then(|| chrono::DateTime::parse_from_rfc3339(at).ok())
        .flatten()
        .map(|t| t.with_timezone(&chrono::Utc));
    let t = match parsed {
        Some(t) if (now - t).num_days() <= DAYS_BACK && (t - now).num_days() < 1 => t,
        _ => now,
    };
    (
        t.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        t.format("%Y-%m-%d").to_string(),
    )
}

/// A `YYYY-MM-DD` query value.
fn parse_day(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn int(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// One stored call: the row `usage_call:<call_id>` gets, priced from the one table.
pub fn call_row(account: &str, c: &UsageCall, now: chrono::DateTime<chrono::Utc>) -> Result<(String, Value), String> {
    let id = c.call_id.as_str();
    if id.is_empty() || id.len() > 32 || !id.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("call_id must be 1-32 hex characters".into());
    }
    let model = name_of(&c.model);
    let usd = pk_core::prices::tokens_usd(&model, c.tokens_in, c.tokens_out, c.cache_write, c.cache_read);
    let (at, day) = when_of(&c.at, now);
    let mut row = Map::new();
    row.insert("account".into(), json!(account));
    row.insert("at".into(), json!(at));
    row.insert("day".into(), json!(day));
    if let Some(j) = c.job.as_deref().filter(|j| {
        !j.is_empty()
            && j.len() <= 64
            && j.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':'))
    }) {
        row.insert("job".into(), json!(j));
    }
    row.insert("kind".into(), json!(kind_of(c.kind.as_deref())));
    row.insert("task".into(), json!(name_of(&c.task)));
    row.insert("model".into(), json!(model));
    row.insert("tokens_in".into(), json!(int(c.tokens_in)));
    row.insert("tokens_out".into(), json!(int(c.tokens_out)));
    row.insert("cache_read".into(), json!(int(c.cache_read)));
    row.insert("cache_write".into(), json!(int(c.cache_write)));
    row.insert("usd".into(), json!(usd));
    row.insert("priced".into(), json!(pk_core::prices::PRICES_READ));
    Ok((id.to_ascii_lowercase(), Value::Object(row)))
}

/// The meter of one process.
#[derive(Debug, Default)]
pub struct Meter {
    /// Counts not yet written, by day.
    pending: Mutex<BTreeMap<String, Counts>>,
    /// Days whose AI totals must be recomputed (a call for them landed).
    touched: Mutex<BTreeSet<String>>,
    /// `schema_usage.surql` applied by this process.
    schema: AtomicBool,
    /// When the last storage sample was taken.
    storage_at: Mutex<Option<Instant>>,
    /// What the last rollup left for Prometheus.
    gauges: RwLock<pk_obs::UsageGauges>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Meter {
    /// One answered request: added to today's counts in memory (nothing else on the request path).
    pub fn note(&self, method: &str, path: &str, status: u16) {
        if let Some(c) = classify(method, path, status) {
            lock(&self.pending).entry(pk_core::time::today()).or_default().add(&c);
        }
    }

    /// One `/api/live` socket opened by a signed-in client.
    pub fn session(&self) {
        lock(&self.pending).entry(pk_core::time::today()).or_default().sessions += 1;
    }

    /// The counts waiting to be written (tests).
    pub fn pending(&self) -> BTreeMap<String, Counts> {
        lock(&self.pending).clone()
    }

    /// What Prometheus reads.
    pub fn gauges(&self) -> pk_obs::UsageGauges {
        self.gauges.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Apply `schema_usage.surql` once per process (the first write needs the tables; STRICT refuses unknown ones).
    pub async fn ensure_schema(&self, store: &Store) -> Result<(), StoreError> {
        if self.schema.load(Ordering::SeqCst) {
            return Ok(());
        }
        store.apply_usage_schema().await?;
        self.schema.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Store a batch of calls from the worker: each validated, priced from the one table and upserted by its call
    /// id (a batch posted twice is stored once). `Err` with a 400 for a batch that is not one.
    pub async fn ingest(&self, store: &Store, account: &str, calls: &[UsageCall]) -> Result<usize, ApiError> {
        if calls.len() > MAX_BATCH {
            return Err(ApiError::bad_request(format!("at most {MAX_BATCH} calls per post")));
        }
        if calls.is_empty() {
            return Ok(0);
        }
        let now = chrono::Utc::now();
        let mut rows = Vec::with_capacity(calls.len());
        let mut days = BTreeSet::new();
        for (k, c) in calls.iter().enumerate() {
            let (id, row) = call_row(account, c, now).map_err(|e| ApiError::bad_request(format!("call {k}: {e}")))?;
            if let Some(d) = row.get("day").and_then(Value::as_str) {
                days.insert(d.to_string());
            }
            rows.push(json!({"id": id, "row": row}));
        }
        self.ensure_schema(store).await.map_err(crate::writes::store_err)?;
        store
            .query(
                "FOR $c IN $rows { UPSERT type::record('usage_call', $c.id) CONTENT $c.row RETURN NONE; };",
                vars([("rows", to_db(&Value::Array(rows)))]),
            )
            .await
            .map_err(crate::writes::store_err)?;
        lock(&self.touched).extend(days);
        Ok(calls.len())
    }

    /// Record one transcript or recording the server received, under `key` (the route's own dedupe key, so a retry
    /// is one row, and a repeat on a later day leaves it on the day it first came): its length and size, nothing it
    /// said. Called from a task the route spawned.
    pub async fn media(&self, store: &Store, account: &str, key: &str, kind: &str, seconds: f64, bytes: u64) {
        // the key may name his material (a transcript's digest, an upload's file name): only its hash is kept
        let id = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(key.as_bytes()));
        let now = pk_core::time::now_iso();
        let row = json!({"account": account, "at": now, "day": now.get(..10).unwrap_or_default(), "kind": kind,
                         "seconds": if seconds.is_finite() && seconds > 0.0 { seconds } else { 0.0 },
                         "bytes": int(bytes)});
        let r = async {
            self.ensure_schema(store).await?;
            store
                .query(
                    "LET $r = type::record('usage_media', $id); \
                     IF !record::exists($r) { CREATE $r CONTENT $row RETURN NONE; };",
                    vars([("id", to_db(&json!(&id[..32]))), ("row", to_db(&row))]),
                )
                .await
                .map(drop)
        };
        if let Err(e) = r.await {
            tracing::warn!(error = %e, kind, "usage: a media row was not written");
        }
    }

    /// Write the counts in memory (`+=`, so processes add up). On a failure they go back for the next flush.
    pub async fn flush(&self, store: &Store, account: &str) -> Result<(), StoreError> {
        let drained = std::mem::take(&mut *lock(&self.pending));
        if drained.is_empty() {
            return Ok(());
        }
        let r = async {
            self.ensure_schema(store).await?;
            let rows: Vec<Value> = drained
                .iter()
                .map(|(d, c)| {
                    json!({"day": d, "requests": int(c.requests), "writes": int(c.writes), "bits": int(c.bits),
                           "redlines": int(c.redlines), "drafts": int(c.drafts), "sessions": int(c.sessions)})
                })
                .collect();
            store
                .query(
                    "FOR $c IN $rows { UPSERT type::record('usage_day', [$a, $c.day]) SET account = $a, day = $c.day, \
                     requests += $c.requests, writes += $c.writes, bits += $c.bits, redlines += $c.redlines, \
                     drafts += $c.drafts, sessions += $c.sessions RETURN NONE; };",
                    vars([("a", to_db(&json!(account))), ("rows", to_db(&Value::Array(rows)))]),
                )
                .await
                .map(drop)
        };
        if let Err(e) = r.await {
            let mut p = lock(&self.pending);
            for (d, c) in drained {
                p.entry(d).or_default().add(&c);
            }
            return Err(e);
        }
        Ok(())
    }

    /// Recompute the AI totals of `day` from its calls (`=`, so running it twice is the same rollup).
    async fn roll_day(&self, store: &Store, account: &str, day: &str) -> Result<(), StoreError> {
        let rows = from_db(
            store
                .last(
                    "SELECT kind, model, count() AS calls, math::sum(usd) AS usd, math::sum(tokens_in) AS tokens_in, \
                     math::sum(tokens_out) AS tokens_out, math::sum(cache_read) AS cache_read, \
                     math::sum(cache_write) AS cache_write FROM usage_call WHERE account = $a AND day = $d \
                     GROUP BY kind, model;",
                    vars([("a", to_db(&json!(account))), ("d", to_db(&json!(day)))]),
                )
                .await?,
        );
        let rows: Vec<Value> = rows.as_array().cloned().unwrap_or_default();
        let usd: f64 = rows.iter().filter_map(|r| r["usd"].as_f64()).sum();
        let calls: i64 = rows.iter().filter_map(|r| r["calls"].as_i64()).sum();
        store
            .query(
                "UPSERT type::record('usage_day', [$a, $d]) SET account = $a, day = $d, ai = $ai, \
                 ai_usd = <float> $usd, ai_calls = $n, rolled = $now RETURN NONE;",
                vars([
                    ("a", to_db(&json!(account))),
                    ("d", to_db(&json!(day))),
                    ("ai", to_db(&Value::Array(rows))),
                    ("usd", to_db(&json!(usd))),
                    ("n", to_db(&json!(calls))),
                    ("now", to_db(&json!(pk_core::time::now_iso()))),
                ]),
            )
            .await
            .map(drop)
    }

    /// The rollup: the AI totals of today and of every day a call landed for since the last one, the storage sample
    /// when it is due, then the gauges.
    pub async fn rollup(&self, store: &Store, account: &str) -> Result<(), StoreError> {
        self.ensure_schema(store).await?;
        let today = pk_core::time::today();
        let mut days = std::mem::take(&mut *lock(&self.touched));
        days.insert(today.clone());
        for (k, d) in days.iter().enumerate() {
            if let Err(e) = self.roll_day(store, account, d).await {
                lock(&self.touched).extend(days.iter().skip(k).cloned());
                return Err(e);
            }
        }
        let due = lock(&self.storage_at).is_none_or(|t| t.elapsed() >= STORAGE_EVERY);
        if due {
            match self.sample_storage(store, account, &today).await {
                Ok(()) => *lock(&self.storage_at) = Some(Instant::now()),
                Err(e) => tracing::warn!(error = %e, "usage: the storage sample failed"),
            }
        }
        self.refresh_gauges(store, account, &today).await
    }

    /// Rows and bytes of the database (single tenant: all of it is the account's) and the recordings' bytes, on
    /// today's rollup row.
    async fn sample_storage(&self, store: &Store, account: &str, today: &str) -> Result<(), StoreError> {
        store
            .query(
                "UPSERT type::record('usage_day', [$a, $d]) SET account = $a, day = $d, storage = { \
                 jokes: count(SELECT id FROM joke), units: count(SELECT id FROM unit), \
                 sets: count(SELECT id FROM set), decisions: count(SELECT id FROM decision), \
                 creative_events: count(SELECT id FROM creative_event), \
                 file_rows: count(SELECT id FROM srcfile), \
                 file_bytes: math::sum(SELECT VALUE bytes FROM srcfile) ?? 0, \
                 recording_bytes: math::sum(SELECT VALUE bytes FROM upload) ?? 0, \
                 at: $now } RETURN NONE;",
                vars([
                    ("a", to_db(&json!(account))),
                    ("d", to_db(&json!(today))),
                    ("now", to_db(&json!(pk_core::time::now_iso()))),
                ]),
            )
            .await
            .map(drop)
    }

    async fn refresh_gauges(&self, store: &Store, account: &str, today: &str) -> Result<(), StoreError> {
        let month0 = format!("{}-01", &today[..7]);
        let rows = from_db(
            store
                .last(
                    "SELECT day, requests, ai_usd, ai FROM usage_day WHERE account = $a AND day >= $m AND day <= $t;",
                    vars([
                        ("a", to_db(&json!(account))),
                        ("m", to_db(&json!(month0))),
                        ("t", to_db(&json!(today))),
                    ]),
                )
                .await?,
        );
        let rows = rows.as_array().cloned().unwrap_or_default();
        let mut g = pk_obs::UsageGauges::default();
        for r in &rows {
            g.usd_month += r["ai_usd"].as_f64().unwrap_or(0.0);
            if r["requests"].as_i64().unwrap_or(0) > 0 {
                g.active_days_month += 1.0;
            }
            if r["day"].as_str() == Some(today) {
                g.requests_today = r["requests"].as_f64().unwrap_or(0.0);
                let mut usd: BTreeMap<String, f64> = BTreeMap::new();
                let mut calls: BTreeMap<String, f64> = BTreeMap::new();
                for a in r["ai"].as_array().into_iter().flatten() {
                    let k = kind_of(a["kind"].as_str()).to_string();
                    *usd.entry(k.clone()).or_default() += a["usd"].as_f64().unwrap_or(0.0);
                    *calls.entry(k).or_default() += a["calls"].as_f64().unwrap_or(0.0);
                }
                g.usd_today = usd.into_iter().collect();
                g.calls_today = calls.into_iter().collect();
            }
        }
        *self.gauges.write().unwrap_or_else(PoisonError::into_inner) = g;
        Ok(())
    }

    /// One turn of [`run`]: the counts, then the rollup.
    pub async fn tick(&self, store: &Store, account: &str) -> Result<(), StoreError> {
        self.flush(store, account).await?;
        self.rollup(store, account).await
    }

    /// `GET /api/admin/usage`: the days `from..=to` (default: this month so far), their totals, this month's
    /// projection, the price table and when it was read. Brings the numbers up to now first (the owner's page, not a
    /// hot path).
    pub async fn report(
        &self,
        store: &Store,
        account: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Value, ApiError> {
        let today_s = pk_core::time::today();
        let today = parse_day(&today_s).ok_or_else(|| ApiError::internal("ClockError", "today is not a day"))?;
        let month0 = today.with_day(1).unwrap_or(today);
        let bad = |w: &str| ApiError::bad_request(format!("{w} must be a day, YYYY-MM-DD"));
        let from = match from {
            Some(s) => parse_day(s).ok_or_else(|| bad("from"))?,
            None => month0,
        };
        let to = match to {
            Some(s) => parse_day(s).ok_or_else(|| bad("to"))?,
            None => today,
        };
        if to < from {
            return Err(ApiError::bad_request("to is before from"));
        }
        if (to - from).num_days() >= MAX_RANGE_DAYS {
            return Err(ApiError::bad_request(format!("at most {MAX_RANGE_DAYS} days at once")));
        }
        self.tick(store, account).await.map_err(crate::writes::store_err)?;
        let (f, t) = (from.format("%Y-%m-%d").to_string(), to.format("%Y-%m-%d").to_string());
        let range = |sql: &str, a: &str, f: &str, t: &str| {
            (
                sql.to_string(),
                vars([
                    ("a", to_db(&json!(a))),
                    ("f", to_db(&json!(f))),
                    ("t", to_db(&json!(t))),
                ]),
            )
        };
        let get = |q: (String, pk_store::Vars)| async move {
            store
                .last(&q.0, q.1)
                .await
                .map(from_db)
                .map_err(crate::writes::store_err)
        };
        let days = get(range(
            "SELECT day, requests, writes, bits, redlines, drafts, sessions, ai_usd, ai_calls, storage FROM usage_day \
             WHERE account = $a AND day >= $f AND day <= $t ORDER BY day;",
            account,
            &f,
            &t,
        ))
        .await?;
        let by = get(range(
            "SELECT kind, model, count() AS calls, math::sum(usd) AS usd, math::sum(tokens_in) AS tokens_in, \
             math::sum(tokens_out) AS tokens_out, math::sum(cache_read) AS cache_read, \
             math::sum(cache_write) AS cache_write FROM usage_call WHERE account = $a AND day >= $f AND day <= $t \
             GROUP BY kind, model;",
            account,
            &f,
            &t,
        ))
        .await?;
        let media = get(range(
            "SELECT kind, count() AS count, math::sum(seconds) AS seconds, math::sum(bytes) AS bytes FROM usage_media \
             WHERE account = $a AND day >= $f AND day <= $t GROUP BY kind;",
            account,
            &f,
            &t,
        ))
        .await?;
        let m0 = month0.format("%Y-%m-%d").to_string();
        let month_days = get(range(
            "SELECT day, requests, ai_usd FROM usage_day WHERE account = $a AND day >= $f AND day <= $t;",
            account,
            &m0,
            &today_s,
        ))
        .await?;
        Ok(build_report(&f, &t, today, &days, &by, &media, &month_days))
    }
}

/// The report's body from the rows the queries returned (pure, so the arithmetic is tested without a database).
pub fn build_report(
    from: &str,
    to: &str,
    today: NaiveDate,
    days: &Value,
    by: &Value,
    media: &Value,
    month_days: &Value,
) -> Value {
    let days = days.as_array().cloned().unwrap_or_default();
    let by = by.as_array().cloned().unwrap_or_default();
    let n = |v: &Value, k: &str| v[k].as_f64().unwrap_or(0.0);
    let i = |v: &Value, k: &str| v[k].as_i64().unwrap_or(0);
    let mut kinds: BTreeMap<String, (f64, i64)> = BTreeMap::new();
    let mut models: BTreeMap<String, (f64, i64)> = BTreeMap::new();
    let (mut tin, mut tout, mut cr, mut cw) = (0, 0, 0, 0);
    for r in &by {
        let k = kinds
            .entry(r["kind"].as_str().unwrap_or("other").to_string())
            .or_default();
        k.0 += n(r, "usd");
        k.1 += i(r, "calls");
        let m = models
            .entry(r["model"].as_str().unwrap_or("other").to_string())
            .or_default();
        m.0 += n(r, "usd");
        m.1 += i(r, "calls");
        tin += i(r, "tokens_in");
        tout += i(r, "tokens_out");
        cr += i(r, "cache_read");
        cw += i(r, "cache_write");
    }
    let ai_usd: f64 = kinds.values().map(|v| v.0).sum();
    let ai_calls: i64 = kinds.values().map(|v| v.1).sum();
    let mut by_kind: Vec<Value> = kinds
        .into_iter()
        .map(|(k, (usd, calls))| json!({"kind": k, "usd": usd, "calls": calls}))
        .collect();
    by_kind.sort_by(|a, b| n(b, "usd").total_cmp(&n(a, "usd")));
    let mut by_model: Vec<Value> = models
        .into_iter()
        .map(|(m, (usd, calls))| json!({"model": m, "usd": usd, "calls": calls}))
        .collect();
    by_model.sort_by(|a, b| n(b, "usd").total_cmp(&n(a, "usd")));
    let sum = |k: &str| days.iter().map(|d| i(d, k)).sum::<i64>();
    let active_days = days.iter().filter(|d| i(d, "requests") > 0).count();
    let storage = days
        .iter()
        .rev()
        .find_map(|d| d.get("storage").filter(|s| s.is_object()).cloned());
    let day_rows: Vec<Value> = days
        .iter()
        .map(|d| {
            json!({"day": d["day"], "ai_usd": n(d, "ai_usd"), "ai_calls": i(d, "ai_calls"),
                   "requests": i(d, "requests"), "writes": i(d, "writes"), "bits": i(d, "bits"),
                   "redlines": i(d, "redlines"), "drafts": i(d, "drafts"), "sessions": i(d, "sessions"),
                   "active": i(d, "requests") > 0})
        })
        .collect();

    // this month (UTC), whatever the range: so far, at this pace, per active day
    let md = month_days.as_array().cloned().unwrap_or_default();
    let month_usd: f64 = md.iter().map(|d| n(d, "ai_usd")).sum();
    let month_active = md.iter().filter(|d| i(d, "requests") > 0).count();
    let elapsed = f64::from(today.day());
    let in_month = f64::from(days_in_month(today));
    let projected = month_usd / elapsed * in_month;
    json!({
        "account": "you",
        "from": from,
        "to": to,
        "days": day_rows,
        "totals": {
            "ai_usd": ai_usd, "ai_calls": ai_calls,
            "tokens": {"input": tin, "output": tout, "cache_read": cr, "cache_write": cw},
            "by_kind": by_kind, "by_model": by_model,
            "active_days": active_days, "requests": sum("requests"), "writes": sum("writes"), "bits": sum("bits"),
            "redlines": sum("redlines"), "drafts": sum("drafts"), "sessions": sum("sessions"),
            "media": media.as_array().cloned().unwrap_or_default(),
        },
        "month": {
            "month": today.format("%Y-%m").to_string(),
            "ai_usd": month_usd,
            "days_elapsed": today.day(),
            "days_in_month": days_in_month(today),
            "projected_usd": projected,
            "active_days": month_active,
            "usd_per_active_day": if month_active > 0 { month_usd / month_active as f64 } else { 0.0 },
            "price_to_cover": {
                "margin": MARGIN,
                "usd": projected / (1.0 - MARGIN),
                "assumption": "the lead's: a 70% margin on the AI cost alone; hosting, Apple's cut and taxes are not counted",
            },
        },
        "storage": storage,
        "prices": prices_json(),
        "not_counted": "hosting (one shared server, a fixed cost), Apple and Google fees, transcription (on the iPhone, free)",
    })
}

fn days_in_month(d: NaiveDate) -> u32 {
    let (y, m) = (d.year(), d.month());
    let next = if m == 12 {
        NaiveDate::from_ymd_opt(y + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(y, m + 1, 1)
    };
    next.and_then(|n| n.pred_opt()).map_or(30, |l| l.day())
}

/// The price table the calls were priced at, and when and where it was read.
pub fn prices_json() -> Value {
    let row = |pfx: &str, p: &pk_core::prices::Prices| {
        json!({"model": pfx, "input": p.input, "output": p.output, "cache_write": p.cache_write,
               "cache_read": p.cache_read})
    };
    json!({
        "read": pk_core::prices::PRICES_READ,
        "source": pk_core::prices::PRICES_SOURCE,
        "per": "USD per million tokens",
        "models": pk_core::prices::PRICES.iter().map(|(k, p)| row(k, p)).collect::<Vec<_>>(),
        "long_prompts": pk_core::prices::LONG_PRICES.iter()
            .map(|(k, over, p)| { let mut r = row(k, p); r["over_tokens"] = json!(over); r })
            .collect::<Vec<_>>(),
    })
}

/// The days of a report as CSV (the export): one line per day, counts and money only.
pub fn csv(report: &Value) -> String {
    let mut out = String::from("day,ai_usd,ai_calls,requests,writes,bits,redlines,drafts,sessions,active\n");
    for d in report["days"].as_array().into_iter().flatten() {
        // writing to a String cannot fail
        let _ = writeln!(
            out,
            "{},{:.6},{},{},{},{},{},{},{},{}",
            d["day"].as_str().unwrap_or(""),
            d["ai_usd"].as_f64().unwrap_or(0.0),
            d["ai_calls"].as_i64().unwrap_or(0),
            d["requests"].as_i64().unwrap_or(0),
            d["writes"].as_i64().unwrap_or(0),
            d["bits"].as_i64().unwrap_or(0),
            d["redlines"].as_i64().unwrap_or(0),
            d["drafts"].as_i64().unwrap_or(0),
            d["sessions"].as_i64().unwrap_or(0),
            d["active"].as_bool().unwrap_or(false),
        );
    }
    out
}

/// The meter's loop: every `every`, the counts and the rollup. A failure is logged and the work stays for the next
/// turn (the counts go back in memory, the touched days stay touched).
pub async fn run(st: AppState, every: Duration) {
    let src = st.clone();
    st.metrics().set_usage_source(move || src.usage().gauges());
    let mut t = tokio::time::interval(every);
    t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        t.tick().await;
        let Some(store) = st.store_handle() else { continue };
        if let Err(e) = st.usage().tick(&store, &st.config().owner_id).await {
            tracing::warn!(error = %e, "usage: the flush or the rollup failed; kept for the next turn");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_counts_by_what_it_did() {
        let c = classify("POST", "/api/jokes/joke:abc/versions", 200).expect("counted");
        assert_eq!((c.requests, c.writes, c.bits, c.drafts), (1, 1, 1, 0));
        let c = classify("POST", "/api/jokes/abc/redline", 202).expect("counted");
        assert_eq!((c.redlines, c.drafts, c.bits), (1, 1, 0));
        // a pure-letter id is still one segment (route_pattern would keep it as a word)
        let c = classify("POST", "/api/jokes/abcdef/punchup", 202).expect("counted");
        assert_eq!(c.drafts, 1);
        let c = classify("GET", "/api/jokes?q=x", 200).expect("counted");
        assert_eq!((c.requests, c.writes), (1, 0));
        // never counted: refusals, probes, the worker, the meter's own page, preflights
        for (m, p, s) in [
            ("GET", "/api/jokes", 401),
            ("GET", "/api/health", 200),
            ("GET", "/api/health?ready=1", 200),
            ("POST", "/api/worker/claim", 200),
            ("GET", "/api/admin/usage", 200),
            ("OPTIONS", "/api/ideas", 204),
            ("GET", "/metrics", 200),
        ] {
            assert!(classify(m, p, s).is_none(), "{m} {p} {s}");
        }
        // one segment per {id}: a longer path is not the template
        assert!(!matches("/api/jokes/{id}/versions", "/api/jokes/a/b/versions"));
        assert!(!matches("/api/jokes/{id}/versions", "/api/jokes//versions"));
    }

    #[test]
    fn kinds_are_the_queues_or_other() {
        assert_eq!(kind_of(Some("riff")), "riff");
        assert_eq!(kind_of(Some("redline")), "redline");
        assert_eq!(kind_of(Some("drop table")), "other");
        assert_eq!(kind_of(None), "other");
    }

    fn call(model: &str, tin: u64, tout: u64, cw: u64, cr: u64) -> UsageCall {
        UsageCall {
            call_id: "a1b2c3d4e5f6".into(),
            at: "2026-11-17T10:00:00Z".into(),
            job: Some("j1".into()),
            kind: Some("riff".into()),
            task: "creative_riff".into(),
            model: model.into(),
            tokens_in: tin,
            tokens_out: tout,
            cache_read: cr,
            cache_write: cw,
        }
    }

    /// The price is the page's (read 2026-11-17), worked out here by hand, not through the table.
    #[test]
    fn a_call_is_priced_at_the_pages_numbers() {
        let (_, r) = call_row("owner", &call("claude-sonnet-5-5", 1000, 200, 0, 10_000), noon()).unwrap();
        let want = (1000.0 * 2.0 + 200.0 * 10.0 + 10_000.0 * 0.10) / 1e6;
        assert!(
            (r["usd"].as_f64().unwrap() - want).abs() < 1e-12,
            "{} vs {want}",
            r["usd"]
        );
        assert_eq!(r["priced"], "2026-11-17");
        assert_eq!(r["day"], "2026-11-17");
        assert_eq!(r["kind"], "riff");
        let (_, r) = call_row("owner", &call("claude-opus-5-5", 2000, 500, 3000, 0), noon()).unwrap();
        let want = (2000.0 * 4.0 + 500.0 * 20.0 + 3000.0 * 5.0) / 1e6;
        assert!((r["usd"].as_f64().unwrap() - want).abs() < 1e-12);
        let mut bad = call("claude-opus-5-5", 1, 1, 0, 0);
        bad.call_id = "x'; DELETE usage_call".into();
        assert!(call_row("owner", &bad, noon()).is_err());
    }

    fn noon() -> chrono::DateTime<chrono::Utc> {
        use chrono::TimeZone as _;
        chrono::Utc.with_ymd_and_hms(2026, 11, 17, 12, 0, 0).unwrap()
    }

    /// A posted time is kept only when it is a real instant near today, and is stored canonically; anything else is
    /// the time it was received (a worker cannot put a row on a day of its choosing, or store junk as `at`).
    #[test]
    fn a_calls_time_is_canonical_and_near_today() {
        let n = noon();
        assert_eq!(
            when_of("2026-11-17T10:00:00+02:00", n),
            ("2026-11-17T08:00:00Z".into(), "2026-11-17".into())
        );
        assert_eq!(when_of("2026-11-12T23:59:59Z", n).1, "2026-11-12");
        let received = ("2026-11-17T12:00:00Z".to_string(), "2026-11-17".to_string());
        for bad in [
            "9999-12-31T00:00:00Z",
            "2026-10-01T00:00:00Z",
            "2026-11-17",
            "02026-1-01T00:00:00Z",
            "garbage",
            &format!("2026-11-17T10:00:00Z{}", "x".repeat(100)),
        ] {
            assert_eq!(when_of(bad, n), received, "{bad}");
        }
    }

    /// One line a day, counts and money only, nothing between the lines.
    #[test]
    fn the_export_is_one_line_a_day() {
        let r = json!({"days": [
            {"day": "2026-11-01", "ai_usd": 0.5, "ai_calls": 3, "requests": 4, "writes": 1, "bits": 1, "redlines": 0,
             "drafts": 1, "sessions": 2, "active": true},
            {"day": "2026-11-02", "ai_usd": 0.0, "ai_calls": 0, "requests": 0, "writes": 0, "bits": 0, "redlines": 0,
             "drafts": 0, "sessions": 0, "active": false}]});
        assert_eq!(
            csv(&r),
            "day,ai_usd,ai_calls,requests,writes,bits,redlines,drafts,sessions,active\n\
             2026-11-01,0.500000,3,4,1,1,0,1,2,true\n\
             2026-11-02,0.000000,0,0,0,0,0,0,0,false\n"
        );
    }

    #[test]
    fn the_month_projects_at_this_pace() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        let md = json!([{"day": "2026-11-01", "requests": 3, "ai_usd": 0.5},
                        {"day": "2026-11-05", "requests": 0, "ai_usd": 0.0},
                        {"day": "2026-11-10", "requests": 9, "ai_usd": 1.5}]);
        let r = build_report(
            "2026-11-01",
            "2026-11-10",
            today,
            &json!([]),
            &json!([]),
            &json!([]),
            &md,
        );
        let m = &r["month"];
        assert_eq!(m["days_in_month"], 30);
        assert!((m["ai_usd"].as_f64().unwrap() - 2.0).abs() < 1e-12);
        assert!((m["projected_usd"].as_f64().unwrap() - 6.0).abs() < 1e-12);
        assert_eq!(m["active_days"], 2);
        assert!((m["usd_per_active_day"].as_f64().unwrap() - 1.0).abs() < 1e-12);
        assert!((m["price_to_cover"]["usd"].as_f64().unwrap() - 20.0).abs() < 1e-9);
        assert_eq!(days_in_month(NaiveDate::from_ymd_opt(2028, 2, 3).unwrap()), 29);
        assert_eq!(days_in_month(NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()), 31);
    }
}
