//! The usage meter's half in the worker (SPEC 2026-11-17 §13): every model call the budget charged goes into an
//! outbox when its job ends, and a task of its own posts the outbox to `POST /api/worker/usage` every
//! [`FLUSH_EVERY`], off the job's path. pk-api prices and keeps them (`usage_call` rows), so the record survives a
//! pod restart, which the spend ledger file (`ai_spend.json` on the pod's emptyDir) does not.
//!
//! Counts and names only: the call id, when, the job id and kind, the router task, the model and four token counts.
//! Never a prompt or an answer.
//!
//! A post that fails keeps its calls for the next turn (an older pk-api without the route answers 404: the calls wait
//! for the new one, which is why pk-api ships first). The outbox holds at most [`CAP`] calls; past that the oldest go
//! and are counted (`dropped`), so a long outage costs memory once, never without bound.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use pk_llm::router::CallRecord;
use pk_wire::worker::usage::{MAX_BATCH, UsageCall, UsagePost};
use reqwest::Method;
use tokio_util::sync::CancellationToken;

use crate::api::{ApiClient, ApiError};

/// How often the outbox is posted.
pub const FLUSH_EVERY: Duration = Duration::from_secs(20);
/// The most calls the outbox holds.
pub const CAP: usize = 5000;
/// The last post's own limit when the worker stops.
pub const LAST_POST: Duration = Duration::from_secs(5);
/// The route.
pub const PATH: &str = "/api/worker/usage";

/// What the meter keeps of one call record: only a call the budget charged (it cost money: an answer, a refusal or
/// a cut answer, `billed`); `None` for one that failed before an answer.
pub fn call_of(rec: &CallRecord) -> Option<UsageCall> {
    let b = rec.billed?;
    b.usd?;
    Some(UsageCall {
        call_id: rec.call_id.clone(),
        at: rec.ts.clone(),
        job: rec.tag.as_ref().map(|t| t.job.clone()),
        kind: rec.tag.as_ref().map(|t| t.kind.clone()),
        task: rec.task.clone(),
        model: rec.model.clone(),
        tokens_in: b.tokens_in,
        tokens_out: b.tokens_out,
        cache_read: b.cache_read,
        cache_write: b.cache_write,
    })
}

/// The calls waiting to be posted.
#[derive(Debug, Default)]
pub struct Outbox {
    q: Mutex<VecDeque<UsageCall>>,
    dropped: AtomicU64,
    told_missing: AtomicBool,
}

impl Outbox {
    fn q(&self) -> std::sync::MutexGuard<'_, VecDeque<UsageCall>> {
        self.q.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Add a call (the oldest goes when the outbox is full).
    pub fn push(&self, c: UsageCall) {
        let mut q = self.q();
        if q.len() >= CAP {
            q.pop_front();
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        q.push_back(c);
    }

    /// Calls waiting.
    pub fn len(&self) -> usize {
        self.q().len()
    }

    /// Nothing waiting.
    pub fn is_empty(&self) -> bool {
        self.q().is_empty()
    }

    /// Calls dropped because the outbox was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// The calls waiting, oldest first (tests).
    pub fn snapshot(&self) -> Vec<UsageCall> {
        self.q().iter().cloned().collect()
    }

    fn take(&self, n: usize) -> Vec<UsageCall> {
        let mut q = self.q();
        let n = n.min(q.len());
        q.drain(..n).collect()
    }

    /// A batch that was not posted goes back in front, in order (still within the cap: the newest win).
    fn put_back(&self, batch: Vec<UsageCall>) {
        let mut q = self.q();
        for c in batch.into_iter().rev() {
            if q.len() >= CAP {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            q.push_front(c);
        }
    }

    /// Post everything waiting, a batch at a time; the first failure puts its batch back and ends the flush.
    pub async fn flush(&self, api: &ApiClient) -> Result<usize, ApiError> {
        let mut sent = 0;
        loop {
            let batch = self.take(MAX_BATCH);
            if batch.is_empty() {
                return Ok(sent);
            }
            let n = batch.len();
            let body = UsagePost { calls: batch };
            match api.call(Method::POST, PATH, Some(&body)).await {
                Ok(_) => {
                    sent += n;
                    self.told_missing.store(false, Ordering::Relaxed);
                }
                Err(e) => {
                    if matches!(e, ApiError::Status { status: 404, .. })
                        && !self.told_missing.swap(true, Ordering::Relaxed)
                    {
                        tracing::warn!("usage: the API has no {PATH} yet (an older pk-api); the calls wait for it");
                    }
                    self.put_back(body.calls);
                    return Err(e);
                }
            }
        }
    }
}

/// The flusher: every [`FLUSH_EVERY`] until stopped, then one last post within [`LAST_POST`].
pub async fn run(outbox: &Outbox, api: &ApiClient, stop: CancellationToken) {
    loop {
        let stopped = tokio::select! {
            biased;
            () = stop.cancelled() => true,
            () = tokio::time::sleep(FLUSH_EVERY) => false,
        };
        if stopped {
            break;
        }
        if outbox.is_empty() {
            continue;
        }
        match outbox.flush(api).await {
            Ok(n) => tracing::debug!(sent = n, "usage: posted"),
            Err(ApiError::Status { status: 404, .. }) => {}
            Err(e) => tracing::warn!(error = %e, waiting = outbox.len(), "usage: post failed; kept for the next turn"),
        }
    }
    if !outbox.is_empty() {
        match tokio::time::timeout(LAST_POST, outbox.flush(api)).await {
            Ok(Ok(n)) => tracing::info!(sent = n, "usage: posted before stopping"),
            Ok(Err(e)) => tracing::warn!(error = %e, lost = outbox.len(), "usage: the last post failed"),
            Err(_) => tracing::warn!(lost = outbox.len(), "usage: the last post timed out"),
        }
    }
    if outbox.dropped() > 0 {
        tracing::warn!(
            dropped = outbox.dropped(),
            "usage: calls dropped because the outbox was full"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, usd: Option<f64>, tag: Option<(&str, &str)>) -> CallRecord {
        CallRecord {
            ts: "2026-11-17T10:00:00Z".into(),
            call_id: id.into(),
            run_id: None,
            task: "creative_riff".into(),
            provider: "anthropic".into(),
            model: "claude-opus-5-5".into(),
            prompt_sha: "x".into(),
            fewshot_sha: None,
            tokens_in: 10,
            tokens_out: 5,
            ms: 1,
            ok: true,
            usd,
            cache_read: Some(3),
            cache_write: None,
            error: None,
            billed: Some(pk_llm::Spent {
                tokens_in: 10,
                tokens_out: 5,
                cache_read: 3,
                cache_write: 0,
                usd,
            }),
            tag: tag.map(|(job, kind)| pk_llm::CallTag {
                job: job.into(),
                kind: kind.into(),
            }),
        }
    }

    #[test]
    fn a_charged_call_is_kept_with_its_job_and_a_failed_one_is_not() {
        let c = call_of(&rec("aa", Some(0.1), Some(("j1", "riff")))).expect("charged");
        assert_eq!((c.job.as_deref(), c.kind.as_deref()), (Some("j1"), Some("riff")));
        assert_eq!((c.tokens_in, c.tokens_out, c.cache_read, c.cache_write), (10, 5, 3, 0));
        assert!(call_of(&rec("bb", None, None)).is_none());
        // untagged (made on a task the job spawned): still kept, with no job
        assert!(call_of(&rec("cc", Some(0.1), None)).is_some_and(|c| c.job.is_none()));
    }

    #[test]
    fn a_full_outbox_drops_the_oldest_and_counts_it() {
        let o = Outbox::default();
        for k in 0..CAP + 3 {
            o.push(call_of(&rec(&format!("{k:x}"), Some(0.0), None)).unwrap());
        }
        assert_eq!((o.len(), o.dropped()), (CAP, 3));
        assert_eq!(o.snapshot()[0].call_id, "3");
        let b = o.take(2);
        o.put_back(b);
        assert_eq!(o.snapshot()[0].call_id, "3");
        assert_eq!(o.len(), CAP);
    }
}
