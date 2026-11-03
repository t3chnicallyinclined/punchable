//! The job queue's write side (`creative_api.Creative.enqueue`, `enqueue_followup`, `_queued`, `job_brief`): a
//! request that wants a model's work queues a job for the hosted worker and answers 202 with where to poll, or 200
//! with the finished job of the same key. The queue is the `jobs` creative store: rows, never process memory, so the
//! Python's worker routes hand out what this service queued and the other way round.

use axum::http::StatusCode;
use pk_creative::Items;
use serde_json::{Map, Value, json};
use sha1::{Digest as _, Sha1};

use super::{Creative, Pre, Written};
use crate::error::ApiResult;

/// `WAITING_NOTE`.
pub fn waiting_note(w: &str) -> Value {
    match w {
        "transcriber" => json!("Transcription happens on your phone for now; this file is waiting."),
        _ => Value::Null,
    }
}

/// `job_brief(j)`: the job without its input and result.
pub fn brief(j: &Map<String, Value>) -> Value {
    let mut out = Map::new();
    for k in ["id", "kind", "status", "created_at", "done_at", "attempts", "error"] {
        out.insert(k.into(), j.get(k).cloned().unwrap_or(Value::Null));
    }
    if let Some(w) = j.get("waiting_for").filter(|w| pk_core::py::truthy(Some(w))) {
        out.insert("waiting_for".into(), w.clone());
        out.insert("note".into(), w.as_str().map_or(Value::Null, waiting_note));
    }
    Value::Object(out)
}

/// `_queued(j)`: the 202 body: the job, where to poll, and what a kind no worker runs is waiting for.
pub fn queued(j: &Map<String, Value>) -> Value {
    let id = j.get("id").map(pk_core::py::str).unwrap_or_default();
    let mut out = json!({"status": "queued", "job": brief(j), "poll": format!("/api/jobs/{id}")});
    if let Some(w) = j.get("waiting_for").filter(|w| pk_core::py::truthy(Some(w))) {
        out["waiting_for"] = w.clone();
        out["note"] = w.as_str().map_or(Value::Null, waiting_note);
    }
    out
}

/// The twelve hex of `sha1(seed)` a job id ends with.
fn stamp(seed: &str) -> String {
    hex::encode(Sha1::digest(seed.as_bytes()))[..12].to_string()
}

fn obj(v: &Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

fn status_of(j: &Map<String, Value>) -> &str {
    j.get("status").and_then(Value::as_str).unwrap_or("")
}

/// `Creative.enqueue(st, kind, key, inp, fresh, by)`: `(200, the cached result)` when a job of this key is done and
/// `fresh` is not asked, `(202, the queued job)` when one is queued or claimed, else a new job (202). Never runs the
/// job here. The check runs over the items as they are at the append (again after a lost race).
pub async fn enqueue(
    cr: &Creative,
    kind: &str,
    key: &str,
    input: Value,
    fresh: bool,
    by: &str,
) -> ApiResult<(StatusCode, Value)> {
    let (_, items0) = cr.lines_and_items("jobs").await?;
    let jid = format!(
        "job:{kind}-{}",
        stamp(&format!("{key}{}{}", pk_core::time::now_iso(), items0.len()))
    );
    let pre = |items: &Items| -> ApiResult<Pre> {
        let live: Vec<&Map<String, Value>> = items
            .values()
            .filter_map(Value::as_object)
            .filter(|j| j.get("key").and_then(Value::as_str) == Some(key))
            .collect();
        if !fresh {
            // `max(done, key=done_at or "")`: the first of the newest on ties
            let mut done: Option<(&Map<String, Value>, String)> = None;
            for j in live.iter().filter(|j| status_of(j) == "done") {
                let at = j
                    .get("done_at")
                    .filter(|t| pk_core::py::truthy(Some(t)))
                    .map(pk_core::py::str)
                    .unwrap_or_default();
                if done.as_ref().is_none_or(|(_, b)| at > *b) {
                    done = Some((j, at));
                }
            }
            if let Some((j, _)) = done {
                let mut out = Map::new();
                out.insert("cached".into(), json!(true));
                out.insert("job".into(), brief(j));
                if let Some(r) = j.get("result").and_then(Value::as_object) {
                    out.extend(r.clone());
                }
                return Ok(Pre::Done(StatusCode::OK, Value::Object(out)));
            }
        }
        if let Some(p) = live.iter().find(|j| matches!(status_of(j), "queued" | "claimed")) {
            return Ok(Pre::Done(StatusCode::ACCEPTED, queued(p)));
        }
        Ok(Pre::Go)
    };
    match cr
        .append_if(
            "jobs",
            "enqueue",
            &jid,
            json!({"kind": kind, "key": key, "input": input}),
            by,
            pre,
        )
        .await?
    {
        Written::Landed(item) => Ok((StatusCode::ACCEPTED, queued(&obj(&item)))),
        Written::Skipped(status, v) => Ok((status, v)),
    }
}

/// `Creative.enqueue_followup(st, kind, by)`: a follow-up job (embed, memolink) after new material, one QUEUED per
/// kind at a time; `None` when one is already waiting (checked again after a lost race).
pub async fn enqueue_followup(cr: &Creative, kind: &str, by: &str) -> ApiResult<Option<Value>> {
    let (_, items0) = cr.lines_and_items("jobs").await?;
    let n = items0.len();
    let jid = format!(
        "job:{kind}-{}",
        stamp(&format!("{kind}{}{n}", pk_core::time::now_iso()))
    );
    let pre = |items: &Items| -> ApiResult<Pre> {
        let waiting = items
            .values()
            .filter_map(Value::as_object)
            .any(|j| j.get("kind").and_then(Value::as_str) == Some(kind) && status_of(j) == "queued");
        Ok(if waiting {
            Pre::Done(StatusCode::OK, Value::Null)
        } else {
            Pre::Go
        })
    };
    match cr
        .append_if(
            "jobs",
            "enqueue",
            &jid,
            json!({"kind": kind, "key": format!("{kind}|{n}"), "input": {}}),
            by,
            pre,
        )
        .await?
    {
        Written::Landed(item) => Ok(Some(item)),
        Written::Skipped(..) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_brief_carries_the_wait_only_when_there_is_one() {
        let j = obj(
            &json!({"id": "job:x", "kind": "set_text", "status": "queued", "created_at": "t", "attempts": 0,
                             "waiting_for": null, "input": {"big": true}}),
        );
        assert_eq!(
            brief(&j),
            json!({"id": "job:x", "kind": "set_text", "status": "queued", "created_at": "t", "done_at": null,
                   "attempts": 0, "error": null})
        );
        let q = queued(&j);
        assert_eq!(q["poll"], "/api/jobs/job:x");
        assert!(q.get("waiting_for").is_none());
        let w = obj(&json!({"id": "job:y", "kind": "process_set", "status": "queued", "waiting_for": "transcriber"}));
        let q = queued(&w);
        assert_eq!(q["waiting_for"], "transcriber");
        assert_eq!(q["note"], waiting_note("transcriber"));
        assert_eq!(brief(&w)["note"], waiting_note("transcriber"));
    }

    #[test]
    fn the_job_id_is_sha1_of_key_time_and_count() {
        assert_eq!(stamp("abc"), "a9993e364706");
    }
}
