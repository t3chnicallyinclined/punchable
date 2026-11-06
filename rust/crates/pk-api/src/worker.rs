//! The hosted worker as the API sees it (`creative_api.worker_status`, `check_claim`, `Creative.claim`): its last
//! claim is one row, `worker:seen`, in the database, written by whichever server answered `POST /api/worker/claim`
//! and read by both for the worker's line (Tonight, Settings > Connections, Ask), so the two servers agree on it
//! while both serve (T10 group 5). It was the Python process's memory before, and `/api/tonight/screen` here had to
//! ask the Python for it.
//!
//! Also the lease one background task takes over the database ([`Lease`]): the vector sweeper runs in both
//! processes during T10, and the lease, not the process, says which one embeds.

use std::time::{SystemTime, UNIX_EPOCH};

use pk_creative::reducers::jobs::{JOB_KINDS, MAX_ATTEMPTS, TRANSCRIBER_KINDS, kinds_repr};
use pk_store::{Store, StoreError, from_db, to_db, vars};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::error::{ApiError, ApiResult};

/// `WORKER_ONLINE_S`: quiet this long = offline.
pub const WORKER_ONLINE_S: i64 = 120;
/// `STALE_CLAIM_S`: a claimed job with no result after this is handed out again.
pub const STALE_CLAIM_S: f64 = 15.0 * 60.0;
/// The record the last claim is kept in.
pub const SEEN_ID: &str = "worker:seen";

/// What a claim reports (`check_claim`'s `seen`), as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Seen {
    /// `time.time()` of the claim.
    pub t: f64,
    /// The same instant, ISO.
    pub at: String,
    /// The worker's name (40 characters at most).
    pub name: String,
    /// It said it runs on the server.
    pub hosted: bool,
    /// The models it writes with, when it said.
    pub model: Option<String>,
    /// Why it takes nothing (over its budget), when it said.
    pub paused: Option<String>,
}

/// `time.time()`.
pub fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

/// `_ts(s)`: `datetime.fromisoformat(s).timestamp()`, 0 for none or unreadable. A stamp without an offset is read
/// as UTC (the Python reads it as the pod's local time, which is UTC on the server; every service writes offsets).
pub fn iso_ts(s: Option<&str>) -> f64 {
    let Some(s) = s else {
        return 0.0;
    };
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return t.timestamp() as f64 + f64::from(t.timestamp_subsec_micros()) / 1e6;
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f"))
        .map_or(0.0, |t| t.and_utc().timestamp() as f64)
}

fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `check_claim(body)`: the kinds the worker may take (`None` = any) and what it reports. A hosted worker asking for
/// a transcriber kind is refused outright (400); a `kinds` that is not a list of job kinds is the 400 naming them.
pub fn check_claim(b: &Map<String, Value>) -> ApiResult<(Option<Vec<String>>, Seen)> {
    let kinds = match b.get("kinds").filter(|k| pk_core::py::truthy(Some(k))) {
        None => None,
        Some(Value::Array(xs)) => {
            let mut out = Vec::with_capacity(xs.len());
            for x in xs {
                match x.as_str().filter(|k| JOB_KINDS.contains(k)) {
                    Some(k) => out.push(k.to_string()),
                    None => {
                        return Err(ApiError::bad_request(format!(
                            "kinds must be a subset of {}",
                            kinds_repr()
                        )));
                    }
                }
            }
            Some(out)
        }
        Some(_) => {
            return Err(ApiError::bad_request(format!(
                "kinds must be a subset of {}",
                kinds_repr()
            )));
        }
    };
    let hosted = pk_core::py::truthy(b.get("hosted"));
    if hosted
        && kinds
            .as_ref()
            .is_none_or(|ks| ks.iter().any(|k| TRANSCRIBER_KINDS.contains(&k.as_str())))
    {
        let mut t = TRANSCRIBER_KINDS.to_vec();
        t.sort_unstable();
        return Err(ApiError::bad_request(format!(
            "a hosted worker must list its kinds and never a transcriber kind ([{}]): transcription happens on the phone",
            t.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(", ")
        )));
    }
    let text = |k: &str, n: usize| {
        b.get(k)
            .filter(|v| pk_core::py::truthy(Some(v)))
            .map(|v| cut(&pk_core::py::str(v), n))
    };
    let seen = Seen {
        t: unix_now(),
        at: pk_core::time::now_iso(),
        name: text("worker", 40).unwrap_or_else(|| "home".to_string()),
        hosted,
        model: text("model", 120),
        paused: text("paused", 300),
    };
    Ok((kinds, seen))
}

/// Keep `seen` as the worker's last claim (`UPSERT worker:seen`), for both servers.
pub async fn save_seen(store: &Store, seen: &Seen) -> Result<(), StoreError> {
    let row = serde_json::to_value(seen).map_err(|e| StoreError::mapping(e.to_string()))?;
    store
        .write(
            "UPSERT worker:seen CONTENT $w RETURN AFTER;",
            vars([("w", to_db(&row))]),
            Some(1),
        )
        .await?;
    Ok(())
}

/// The worker's last claim, from whichever server answered it; `None` when none ever did.
pub async fn load_seen(store: &Store) -> Result<Option<Seen>, StoreError> {
    let v = from_db(store.last("SELECT * OMIT id FROM ONLY worker:seen;", vars([])).await?);
    if v.is_null() || !v.is_object() {
        return Ok(None);
    }
    serde_json::from_value(v)
        .map(Some)
        .map_err(|e| StoreError::mapping(format!("worker:seen: {e}")))
}

/// `worker_status(cr)` for a hosted queue (`jobs_mode` = worker): the line the app shows, from the last claim.
/// `hosted_api` = this API runs with auth on (`PK_AUTH=1`), which names "the server" even before a claim.
pub fn status(seen: Option<&Seen>, hosted_api: bool, now: f64) -> Value {
    let online = seen.is_some_and(|w| now - w.t < WORKER_ONLINE_S as f64);
    let hosted = seen.is_some_and(|w| w.hosted);
    json!({
        "online": online,
        "last_seen": seen.map(|w| w.at.clone()),
        "name": seen.map(|w| w.name.clone()),
        "where": if hosted || hosted_api { "the server" } else { "a development PC" },
        "hosted": hosted,
        "model": seen.and_then(|w| w.model.clone()),
        "paused": if online { seen.and_then(|w| w.paused.clone()) } else { None },
        "online_window_s": WORKER_ONLINE_S,
    })
}

/// `Creative.claim`'s `claimable(j)`: queued, or claimed too long ago with attempts left, and of a kind asked for.
pub fn claimable(j: &Map<String, Value>, kinds: Option<&[String]>, now: f64) -> bool {
    let s = |k: &str| j.get(k).and_then(Value::as_str).unwrap_or("");
    // `if kinds and ...`: an empty list is any kind (`check_claim` already made it `None`)
    if let Some(ks) = kinds.filter(|ks| !ks.is_empty())
        && !ks.iter().any(|k| k == s("kind"))
    {
        return false;
    }
    match s("status") {
        "queued" => true,
        "claimed" if j.get("attempts").and_then(Value::as_i64).unwrap_or(0) < MAX_ATTEMPTS => {
            now - iso_ts(j.get("claimed_at").and_then(Value::as_str)) > STALE_CLAIM_S
        }
        _ => false,
    }
}

/// The job a claim takes: the oldest claimable one (`sorted(..., key=(created_at, id))[0]`), by id.
pub fn pick(items: &pk_creative::Items, kinds: Option<&[String]>, now: f64) -> Option<String> {
    items
        .values()
        .filter_map(Value::as_object)
        .filter(|j| claimable(j, kinds, now))
        .map(|j| {
            let s = |k: &str| j.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            (s("created_at"), s("id"))
        })
        .min()
        .map(|(_, id)| id)
}

/// `can_drop(j, body)`: a worker's "result" may end a QUEUED job only as a final failure of a kind no worker runs
/// (an upload waiting for a transcriber, a video import): `ok` false, `retry` false.
pub fn can_drop(j: &Map<String, Value>, b: &Map<String, Value>) -> bool {
    j.get("status").and_then(Value::as_str) == Some("queued")
        && j.get("kind")
            .and_then(Value::as_str)
            .is_some_and(|k| TRANSCRIBER_KINDS.contains(&k))
        && !pk_core::py::truthy(b.get("ok"))
        && !pk_core::py::truthy(b.get("retry"))
}

/// `len(json.dumps(res))`: the length in characters of the default dump, where every non-ASCII character is a
/// `\uXXXX` escape (two for one past the Basic Multilingual Plane).
pub fn dumps_ascii_len(v: &Value) -> usize {
    pk_core::py::dumps(v)
        .chars()
        .map(|c| match u32::from(c) {
            0..=0x7f => 1,
            0x80..=0xffff => 6,
            _ => 12,
        })
        .sum()
}

/// One owner per background task: `lease:<kind>` (`pk_store::Store::lease_acquire`), taken for `ttl_s` seconds by
/// the first process to ask and renewed by its owner; another process gets it only once it lapsed.
#[derive(Debug, Clone)]
pub struct Lease {
    /// The task.
    pub kind: String,
    /// Who asks (`pk-api:<host>:<pid>`).
    pub me: String,
    /// How long one grant lasts.
    pub ttl_s: u32,
}

impl Lease {
    /// The embedder's lease, for this process.
    pub fn embedder(me: impl Into<String>) -> Self {
        Self {
            kind: "embedder".into(),
            me: me.into(),
            ttl_s: 90,
        }
    }

    /// Take or renew the lease: `true` when this process holds it now.
    pub async fn acquire(&self, store: &Store) -> Result<bool, StoreError> {
        store.lease_acquire(&self.kind, &self.me, self.ttl_s).await
    }

    /// Give it up (a sweeper that cannot run must not keep the other process from running).
    pub async fn release(&self, store: &Store) -> Result<(), StoreError> {
        store.lease_release(&self.kind, &self.me).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: &Value) -> Map<String, Value> {
        v.as_object().cloned().expect("an object")
    }

    #[test]
    fn a_claim_is_checked_the_way_the_python_checks_it() {
        let (kinds, seen) = check_claim(&obj(
            &json!({"kinds": ["riff", "ask_chat"], "worker": "server-1", "hosted": true,
                                                     "model": "claude-x", "paused": null}),
        ))
        .unwrap();
        assert_eq!(
            kinds.as_deref(),
            Some(&["riff".to_string(), "ask_chat".to_string()][..])
        );
        assert_eq!(
            (seen.name.as_str(), seen.hosted, seen.model.as_deref(), seen.paused),
            ("server-1", true, Some("claude-x"), None)
        );
        assert!(seen.at.ends_with("+00:00") && seen.t > 1.7e9);
        let (kinds, seen) = check_claim(&obj(&json!({}))).unwrap();
        assert!(kinds.is_none() && seen.name == "home" && !seen.hosted);
        assert!(
            check_claim(&obj(&json!({"kinds": []}))).unwrap().0.is_none(),
            "an empty list is any kind"
        );
        let e = check_claim(&obj(&json!({"kinds": ["riff", "zap"]}))).unwrap_err();
        assert_eq!(
            e.msg,
            "kinds must be a subset of ('bridge', 'punchup', 'ask', 'process_set', 'import_youtube', 'facts', 'riff', 'ask_chat', 'embed', 'memolink', 'set_text')"
        );
        assert!(check_claim(&obj(&json!({"kinds": "riff"}))).is_err());
        let e = check_claim(&obj(&json!({"hosted": true}))).unwrap_err();
        assert_eq!(
            e.msg,
            "a hosted worker must list its kinds and never a transcriber kind (['import_youtube', 'process_set']): transcription happens on the phone"
        );
        assert!(check_claim(&obj(&json!({"hosted": true, "kinds": ["riff", "process_set"]}))).is_err());
        let (_, seen) = check_claim(&obj(
            &json!({"worker": "x".repeat(50), "paused": "p".repeat(400), "model": 7}),
        ))
        .unwrap();
        assert_eq!(
            (
                seen.name.len(),
                seen.paused.as_ref().map(String::len),
                seen.model.as_deref()
            ),
            (40, Some(300), Some("7"))
        );
    }

    #[test]
    fn the_status_is_the_pythons_line() {
        let now = 1_000_000.0;
        let seen = Seen {
            t: now - 10.0,
            at: "2026-11-03T12:00:00+00:00".into(),
            name: "server-1".into(),
            hosted: true,
            model: Some("m".into()),
            paused: Some("over budget".into()),
        };
        let w = status(Some(&seen), true, now);
        assert_eq!(
            w,
            json!({"online": true, "last_seen": "2026-11-03T12:00:00+00:00", "name": "server-1", "where": "the server",
                   "hosted": true, "model": "m", "paused": "over budget", "online_window_s": 120})
        );
        let stale = Seen { t: now - 121.0, ..seen };
        let w = status(Some(&stale), false, now);
        assert_eq!(
            (w["online"].as_bool(), w["paused"].is_null(), w["last_seen"].as_str()),
            (Some(false), true, Some("2026-11-03T12:00:00+00:00"))
        );
        assert_eq!(
            status(None, false, now),
            json!({"online": false, "last_seen": null, "name": null, "where": "a development PC", "hosted": false,
                   "model": null, "paused": null, "online_window_s": 120})
        );
        assert_eq!(status(None, true, now)["where"], "the server");
    }

    #[test]
    fn the_oldest_claimable_job_is_picked() {
        let now = iso_ts(Some("2026-11-03T12:00:00+00:00"));
        let mut items = pk_creative::Items::new();
        let job = |id: &str, kind: &str, status: &str, created: &str, attempts: i64, claimed_at: Option<&str>| json!({"id": id, "kind": kind, "status": status, "created_at": created, "attempts": attempts, "claimed_at": claimed_at});
        items.insert(
            "job:b".into(),
            job("job:b", "riff", "queued", "2026-11-03T11:00:00+00:00", 0, None),
        );
        items.insert(
            "job:a".into(),
            job("job:a", "ask", "queued", "2026-11-03T11:00:00+00:00", 0, None),
        );
        items.insert(
            "job:c".into(),
            job("job:c", "riff", "queued", "2026-11-03T10:00:00+00:00", 0, None),
        );
        items.insert(
            "job:d".into(),
            job("job:d", "riff", "done", "2026-11-03T09:00:00+00:00", 1, None),
        );
        items.insert(
            "job:e".into(),
            job(
                "job:e",
                "riff",
                "claimed",
                "2026-11-03T08:00:00+00:00",
                1,
                Some("2026-11-03T11:59:00+00:00"),
            ),
        );
        items.insert(
            "job:f".into(),
            job(
                "job:f",
                "riff",
                "claimed",
                "2026-11-03T07:00:00+00:00",
                1,
                Some("2026-11-03T11:00:00+00:00"),
            ),
        );
        items.insert(
            "job:g".into(),
            job(
                "job:g",
                "riff",
                "claimed",
                "2026-11-03T06:00:00+00:00",
                3,
                Some("2026-11-03T10:00:00+00:00"),
            ),
        );
        // f: a stale claim with attempts left; g: no attempts left; e: claimed a minute ago
        assert_eq!(pick(&items, None, now).as_deref(), Some("job:f"));
        let riff = ["riff".to_string()];
        assert_eq!(pick(&items, Some(&riff), now).as_deref(), Some("job:f"));
        let ask = ["ask".to_string()];
        assert_eq!(pick(&items, Some(&ask), now).as_deref(), Some("job:a"));
        let none = ["facts".to_string()];
        assert_eq!(pick(&items, Some(&none), now), None);
        items.remove("job:f");
        assert_eq!(
            pick(&items, None, now).as_deref(),
            Some("job:c"),
            "then the oldest queued; ties by id"
        );
        items.remove("job:c");
        assert_eq!(pick(&items, None, now).as_deref(), Some("job:a"));
    }

    #[test]
    fn a_result_may_drop_only_a_queued_transcriber_job() {
        let q = obj(&json!({"status": "queued", "kind": "process_set"}));
        assert!(can_drop(&q, &obj(&json!({"ok": false}))));
        assert!(!can_drop(&q, &obj(&json!({"ok": false, "retry": true}))));
        assert!(!can_drop(&q, &obj(&json!({"ok": true}))));
        assert!(!can_drop(
            &obj(&json!({"status": "queued", "kind": "riff"})),
            &obj(&json!({}))
        ));
        assert!(!can_drop(
            &obj(&json!({"status": "claimed", "kind": "process_set"})),
            &obj(&json!({}))
        ));
    }

    #[test]
    fn the_dump_length_counts_ascii_escapes() {
        assert_eq!(dumps_ascii_len(&json!({"a": 1})), 8);
        assert_eq!(dumps_ascii_len(&json!("é")), 8, "\"\\u00e9\"");
        assert_eq!(dumps_ascii_len(&json!("😀")), 14, "two surrogate escapes");
        assert_eq!(dumps_ascii_len(&json!("a\n")), 5, "\"a\\n\": five characters");
    }

    #[test]
    fn iso_ts_reads_offsets_and_none() {
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(near(iso_ts(None), 0.0));
        assert!(near(iso_ts(Some("soon")), 0.0));
        assert!(near(iso_ts(Some("1970-01-01T00:01:00+00:00")), 60.0));
        assert!(near(iso_ts(Some("1970-01-01T00:01:00")), 60.0));
        assert!(near(iso_ts(Some("1970-01-01T01:01:00+01:00")), 60.0));
    }
}
