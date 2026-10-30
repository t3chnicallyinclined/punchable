//! The job queue (`creative_api.job_apply`): enqueue, claim, done, fail. A queued transcriber job is honest about
//! waiting for the phone (`waiting_for: "transcriber"`), derived from its kind on every replay.

use serde_json::{Value, json};

use super::{item_mut, unknown_op};
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::pyish::{get_or_null, key, or, or_else};
use crate::store::{Items, Reducer};
use pk_core::py;
use pk_core::text::head;

/// Every job kind the queue accepts (`JOB_KINDS`), in the Python's order (it is printed in a refusal).
pub const JOB_KINDS: [&str; 11] = [
    "bridge",
    "punchup",
    "ask",
    "process_set",
    "import_youtube",
    "facts",
    "riff",
    "ask_chat",
    "embed",
    "memolink",
    "set_text",
];
/// Kinds that need the recording; no hosted worker runs them (`TRANSCRIBER_KINDS`).
pub const TRANSCRIBER_KINDS: [&str; 2] = ["process_set", "import_youtube"];
/// Claims a job gets before a retry is refused (`MAX_ATTEMPTS`).
pub const MAX_ATTEMPTS: i64 = 3;

/// Python's `repr(JOB_KINDS)`, the tuple as the refusal prints it.
fn kinds_repr() -> String {
    format!(
        "({})",
        JOB_KINDS
            .iter()
            .map(|k| format!("'{k}'"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn waiting_for(kind: &str) -> Value {
    if TRANSCRIBER_KINDS.contains(&kind) {
        json!("transcriber")
    } else {
        Value::Null
    }
}

/// The job-queue reducer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Jobs;

impl Reducer for Jobs {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, i, d) = (ev.op.as_str(), ev.id.as_str(), ev.data()?);
        if op == "enqueue" {
            if items.contains_key(i) {
                return Err(CreativeError::refused(format!("job {i} exists")));
            }
            let Some(kind) = d.get("kind").and_then(Value::as_str).filter(|k| JOB_KINDS.contains(k)) else {
                return Err(CreativeError::refused(format!(
                    "job kind must be one of {}",
                    kinds_repr()
                )));
            };
            items.insert(
                i.to_string(),
                json!({
                    "id": i, "kind": kind, "key": get_or_null(d, "key"), "input": or_else(d, "input", json!({})),
                    "status": "queued", "created_at": ev.ts, "by": ev.by, "attempts": 0, "claimed_at": null,
                    "worker": null, "done_at": null, "result": null, "error": null, "waiting_for": waiting_for(kind),
                }),
            );
            return Ok(());
        }
        let Some(j) = item_mut(items, i) else {
            return Err(CreativeError::refused(format!("no job {i}")));
        };
        let status = py::str(key(j, "status")?);
        match op {
            "claim" => {
                if !matches!(status.as_str(), "queued" | "claimed") {
                    return Err(CreativeError::refused(format!("job {i} is {status}")));
                }
                let attempts = key(j, "attempts")?.as_i64().unwrap_or(0);
                j.insert("status".into(), json!("claimed"));
                j.insert("claimed_at".into(), json!(ev.ts));
                j.insert("worker".into(), get_or_null(d, "worker"));
                j.insert("attempts".into(), json!(attempts + 1));
                j.insert("waiting_for".into(), Value::Null);
            }
            "done" => {
                if status != "claimed" {
                    return Err(CreativeError::refused(format!("job {i} is {status}, not claimed")));
                }
                j.insert("status".into(), json!("done"));
                j.insert("done_at".into(), json!(ev.ts));
                j.insert("result".into(), get_or_null(d, "result"));
                j.insert("error".into(), Value::Null);
                j.insert("waiting_for".into(), Value::Null);
            }
            "fail" => {
                if !matches!(status.as_str(), "claimed" | "queued") {
                    return Err(CreativeError::refused(format!("job {i} is {status}")));
                }
                let attempts = key(j, "attempts")?.as_i64().unwrap_or(0);
                let retry = py::truthy(d.get("retry")) && attempts < MAX_ATTEMPTS;
                let error = head(&py::str(or(d, "error").unwrap_or(&json!("failed"))), 2000);
                let kind = key(j, "kind")?.as_str().unwrap_or("").to_string();
                j.insert("status".into(), json!(if retry { "queued" } else { "failed" }));
                j.insert("error".into(), json!(error));
                j.insert("done_at".into(), if retry { Value::Null } else { json!(ev.ts) });
                j.insert(
                    "waiting_for".into(),
                    if retry { waiting_for(&kind) } else { Value::Null },
                );
            }
            _ => return Err(unknown_op(op)),
        }
        Ok(())
    }
}
