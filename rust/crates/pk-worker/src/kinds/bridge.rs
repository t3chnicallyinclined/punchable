//! `bridge` (`creative/story.py` `bridge_draft`): one spoken line between two of his bits, grounded only in the
//! links between them (a shared persona or topic, a link edge); with none, nothing is drafted ("a bridge would be
//! invented"). His profile first, the learned block second, the task line third.

use std::sync::Arc;

use pk_creative::story::{BRIDGE_SCHEMA, BRIDGE_SYSTEM, Story, bridge_finish};
use pk_creative::write::DRAFT_LABEL;
use pk_llm::ChatMessage;
use pk_wire::worker::BridgeInput;
use serde_json::{Map, Value, json};

use super::{Ctx, Done, Runner};
use crate::error::{JobError, provider_why};

/// The task in `engine/config.hosted.toml`.
pub const TASK: &str = "creative_bridge";

/// The `bridge` kind.
#[derive(Debug)]
pub struct Bridge;

impl Runner for Bridge {
    type Input = BridgeInput;

    fn bad_input(e: &serde_json::Error) -> JobError {
        JobError::Value(format!("bad bridge input: {e}"))
    }

    async fn run(ctx: Ctx<'_>, input: BridgeInput) -> Result<Done, JobError> {
        let corpus = Arc::clone(ctx.corpus()?);
        let he_could_say = ctx.names.he_could_say();
        let (a, b) = (input.from, input.to);
        let (aa, bb) = (a.clone(), b.clone());
        // the transition walks every bit's cosines for the subject band: CPU work, off the async threads
        let (label, links, user, titles) = tokio::task::spawn_blocking(move || {
            let st = Story::new(&corpus);
            let t = st.transition(&aa, &bb);
            let links = st.bridge_links(&aa, &bb);
            let user = (!links.is_empty()).then(|| st.bridge_user(&he_could_say, &aa, &bb, &links));
            (t["label"].clone(), links, user, (corpus.title(&aa), corpus.title(&bb)))
        })
        .await
        .map_err(|e| JobError::Value(format!("bridge links: {e}")))?;
        let mut out = Map::new();
        out.insert("from".into(), json!(a));
        out.insert("to".into(), json!(b));
        out.insert("from_title".into(), json!(titles.0));
        out.insert("to_title".into(), json!(titles.1));
        out.insert("transition".into(), label);
        out.insert("draft".into(), json!(true));
        out.insert("label".into(), json!(DRAFT_LABEL));
        let Some(user) = user else {
            out.insert("status".into(), json!("no_grounding"));
            out.insert("text".into(), Value::Null);
            out.insert("grounded_in".into(), json!([]));
            out.insert(
                "why".into(),
                json!(
                    "No shared persona, topic or link between these two; a bridge would be invented. Add a link or \
                     pick a different neighbour."
                ),
            );
            return Ok(Done::result(Value::Object(out)));
        };
        let msgs = ctx.voice.messages(TASK, BRIDGE_SYSTEM, [ChatMessage::user(user)]);
        match ctx
            .router
            .complete(TASK, &msgs, Some(&BRIDGE_SCHEMA), Some("creative"), None)
            .await
        {
            Ok(r) => {
                let (text, grounded) = bridge_finish(r.data.as_ref(), &links);
                let all: Vec<Value> = links.iter().map(|x| x["id"].clone()).collect();
                out.insert("status".into(), json!("ok"));
                out.insert("text".into(), json!(text));
                out.insert("grounded_in".into(), Value::Array(grounded));
                out.insert("all_links".into(), Value::Array(all));
                out.insert("provider".into(), json!(r.provider));
                out.insert("model".into(), json!(r.model));
                out.insert("call_id".into(), json!(r.call_id));
                out.insert("task".into(), json!(TASK));
            }
            Err(e) => {
                out.insert("status".into(), json!("provider_error"));
                out.insert("text".into(), Value::Null);
                out.insert("grounded_in".into(), Value::Array(links));
                out.insert("why".into(), json!(provider_why(&e)));
            }
        }
        Ok(Done::result(Value::Object(out)))
    }
}
