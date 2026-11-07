//! The recordings nobody collected (T10 round 5, group D; docs/adr/0014): an upload whose job no transcriber has
//! claimed within `PK_UPLOAD_KEEP_S` (7 days) is deleted, and the job that waited for it ends `failed` with the
//! reason, so the app shows the recording is gone and the phone can send it again. A recording whose job was claimed
//! is left alone (somebody is working on it); one whose job is over (done, failed, dropped) or that has no job is a
//! stray and goes too. The bytes of a dropped job, of one that failed for good and of one that was done are deleted
//! when that happens (`writes::uploads::release_for_job`); this is the backstop for the ones nobody ever touches.
//!
//! One owner: a pass takes `lease:uploads` first, like the vector sweeper's, so two API processes never both run it.

use std::time::Duration;

use axum::http::StatusCode;
use pk_creative::Items;
use serde_json::{Map, Value, json};

use crate::error::ApiResult;
use crate::state::AppState;
use crate::worker::Lease;
use crate::writes::uploads::upload_name;
use crate::writes::{Creative, Plan, Written, not_connected, store_err};

/// The default `PK_UPLOAD_KEEP_S`: seven days.
pub const DEFAULT_KEEP: Duration = Duration::from_hours(168);
/// The default `PK_UPLOAD_SWEEP_S`: a pass every hour.
pub const DEFAULT_EVERY: Duration = Duration::from_hours(1);
/// Who the job's end is by.
const ACTOR: &str = "ai:expiry";
/// Why the job ends.
const REASON: &str = "the recording waited a week for a transcriber and was deleted; send it again";

/// The lease of this process's expiry pass.
pub fn lease(me: impl Into<String>, every: Duration) -> Lease {
    Lease {
        kind: "uploads".into(),
        me: me.into(),
        ttl_s: u32::try_from(every.as_secs().saturating_mul(2).saturating_add(30)).unwrap_or(u32::MAX),
    }
}

/// The `process_set` jobs that name the upload `name`, live ones first: a name is free again once its upload is
/// gone, so a job that ended long ago may carry the name of a recording that is waiting now.
fn jobs_of<'a>(items: &'a Items, name: &str) -> Vec<&'a Map<String, Value>> {
    let mut jobs: Vec<&Map<String, Value>> = items
        .values()
        .filter_map(Value::as_object)
        .filter(|j| {
            j.get("kind").and_then(Value::as_str) == Some("process_set") && upload_name(j).as_deref() == Some(name)
        })
        .collect();
    jobs.sort_by_key(|j| match j.get("status").and_then(Value::as_str) {
        Some("claimed") => 0,
        Some("queued") => 1,
        _ => 2,
    });
    jobs
}

/// One pass at time `now` (seconds since the epoch): how many recordings went.
pub async fn expire_once(st: &AppState, now: f64) -> ApiResult<usize> {
    let store = st.store_handle().ok_or_else(not_connected)?;
    let names = store
        .upload_names_before(now - st.config().upload_keep.as_secs_f64())
        .await
        .map_err(store_err)?;
    if names.is_empty() {
        return Ok(0);
    }
    let _cr = st.creative_lock().lock().await;
    let cr = Creative::new(st.clone());
    let items = cr.items("jobs").await?;
    let mut gone = 0;
    for name in names {
        let jobs = jobs_of(&items, &name);
        match jobs.first().and_then(|j| j.get("status").and_then(Value::as_str)) {
            // a transcriber has it
            Some("claimed") => continue,
            Some("queued") => {
                let id = jobs
                    .first()
                    .and_then(|j| j.get("id").and_then(Value::as_str))
                    .unwrap_or_default()
                    .to_string();
                let plan = |items: &Items| -> ApiResult<Plan> {
                    let still = items
                        .get(&id)
                        .and_then(Value::as_object)
                        .is_some_and(|j| j.get("status").and_then(Value::as_str) == Some("queued"));
                    Ok(if still {
                        Plan::Event {
                            op: "fail".into(),
                            id: id.clone(),
                            data: json!({"error": REASON, "retry": false}),
                        }
                    } else {
                        Plan::Done(StatusCode::OK, Value::Null)
                    })
                };
                // claimed in the meantime: leave it
                if let Written::Skipped(..) = cr.append_planned("jobs", ACTOR, None, plan).await? {
                    continue;
                }
            }
            _ => {}
        }
        store.upload_delete(&name).await.map_err(store_err)?;
        gone += 1;
    }
    Ok(gone)
}

/// A pass every `every` while this process holds the lease; until the pod drains.
pub async fn run(st: AppState, lease: Lease, every: Duration) {
    let mut drain = st.drain_watch();
    loop {
        tokio::select! {
            () = tokio::time::sleep(every) => {}
            () = crate::state::drained(&mut drain) => {
                if let Some(store) = st.store_handle()
                    && let Err(e) = lease.release(&store).await
                {
                    tracing::warn!("uploads: the lease could not be released on drain: {e}");
                }
                return;
            }
        }
        let Some(store) = st.store_handle() else { continue };
        match lease.acquire(&store).await {
            Ok(true) => match expire_once(&st, crate::worker::unix_now()).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(deleted = n, "uploads: expired recordings nobody collected"),
                Err(e) => tracing::warn!("uploads: the expiry pass failed: {}", e.msg),
            },
            Ok(false) => {}
            Err(e) => tracing::warn!("uploads: the expiry lease could not be taken: {e}"),
        }
    }
}
