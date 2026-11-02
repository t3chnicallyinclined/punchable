//! `punchup` (`creative/workingset.py` `punch_up`): sharper alternatives for numbered lines of his latest taped
//! night, in his voice, never for a line he locked; the working set in the job carries his locks (the ones on the
//! server are the ones honoured). The voice profile first, the learned block second, the task line third.

use std::sync::Arc;

use pk_creative::punch::{PUNCH_SCHEMA, PUNCH_SYSTEM, bit_locked, punch_drafts, punch_user};
use pk_creative::write::DRAFT_LABEL;
use pk_llm::ChatMessage;
use pk_wire::worker::PunchupInput;
use serde_json::{Map, Value, json};

use super::{Ctx, Done, Runner};
use crate::error::{JobError, provider_why};

/// The task in `engine/config.hosted.toml`.
pub const TASK: &str = "creative_punchup";

/// The `punchup` kind.
#[derive(Debug)]
pub struct Punchup;

impl Runner for Punchup {
    type Input = PunchupInput;

    fn bad_input(e: &serde_json::Error) -> JobError {
        JobError::Value(format!("bad punchup input: {e}"))
    }

    async fn run(ctx: Ctx<'_>, input: PunchupInput) -> Result<Done, JobError> {
        let corpus = Arc::clone(ctx.corpus()?);
        let jid = input.joke_id;
        let mut out = Map::new();
        out.insert("joke_id".into(), json!(jid));
        out.insert("title".into(), json!(corpus.title(&jid)));
        out.insert("draft".into(), json!(true));
        out.insert("label".into(), json!(DRAFT_LABEL));
        out.insert("drafts".into(), json!([]));
        let mut finish = |status: &str, extra: &[(&str, Value)]| {
            out.insert("status".into(), json!(status));
            for (k, v) in extra {
                out.insert((*k).to_string(), v.clone());
            }
            Ok(Done::result(Value::Object(std::mem::take(&mut out))))
        };
        let ws = input.working_set;
        if bit_locked(&ws, &jid) {
            return finish(
                "locked",
                &[("why", json!("This bit is locked (perfected); no punch-ups."))],
            );
        }
        let (c, w, j) = (Arc::clone(&corpus), ws.clone(), jid.clone());
        // the latest night's phrases against every lock: CPU work, off the async threads
        let (user, basis) = tokio::task::spawn_blocking(move || punch_user(&c, &w, &j))
            .await
            .map_err(|e| JobError::Value(format!("punch-up lines: {e}")))?;
        let Some(based_on) = basis.based_on.clone() else {
            return finish("no_tape", &[("why", json!("No taped night to work from."))]);
        };
        let Some(user) = user else {
            return finish(
                "all_locked",
                &[
                    ("why", json!("Every line on the last night is locked.")),
                    ("blocked_locked", json!(basis.blocked)),
                ],
            );
        };
        let msgs = ctx.voice.messages(TASK, PUNCH_SYSTEM, [ChatMessage::user(user)]);
        match ctx
            .router
            .complete(TASK, &msgs, Some(&PUNCH_SCHEMA), Some("creative"), None)
            .await
        {
            Ok(r) => {
                let (drafts, blocked) = punch_drafts(r.data.as_ref(), &ws, &jid, &basis);
                finish(
                    "ok",
                    &[
                        ("drafts", Value::Array(drafts)),
                        ("blocked_locked", json!(blocked)),
                        ("based_on", json!(based_on)),
                        ("provider", json!(r.provider)),
                        ("model", json!(r.model)),
                        ("call_id", json!(r.call_id)),
                        ("task", json!(TASK)),
                    ],
                )
            }
            Err(e) => finish(
                "provider_error",
                &[
                    ("why", json!(provider_why(&e))),
                    ("blocked_locked", json!(basis.blocked)),
                ],
            ),
        }
    }
}
