//! `riff` (`creative/write.py` `riff_ideas`, Write's folded "Ideas from the AI"): up to three draft lines in his
//! voice, grounded in his own related bits quoted verbatim. The voice profile is the first system block and what
//! he kept and passed over the second; the task line third; his idea, his riff and the bits are the user turn.
//! Always `draft: true`, never added for him.

use std::sync::Arc;

use pk_core::py;
use pk_creative::notebook::Index;
use pk_creative::write::{DRAFT_LABEL, RIFF_SCHEMA, RIFF_SYSTEM, riff_grounds, riff_ideas, riff_user};
use pk_llm::ChatMessage;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Ctx, Done, Runner};
use crate::error::{JobError, provider_why};

/// The task in `engine/config.hosted.toml`.
pub const TASK: &str = "creative_riff";
/// His riff lines read at most.
const MAX_LINES: usize = 20;

/// The `riff` input as the Python reads it: `lines` may be absent or null (`inp.get("lines") or []`) and only its
/// strings count (`pk_wire::worker::RiffInput` is the API's stricter shape of the same thing).
#[derive(Debug, Deserialize)]
pub struct RiffJob {
    /// His idea.
    pub idea: String,
    /// His lines so far.
    #[serde(default)]
    pub lines: Option<Vec<Value>>,
}

/// The `riff` kind.
#[derive(Debug)]
pub struct Riff;

impl Runner for Riff {
    type Input = RiffJob;

    fn bad_input(e: &serde_json::Error) -> JobError {
        JobError::Value(format!("bad riff input: {e}"))
    }

    async fn run(ctx: Ctx<'_>, input: RiffJob) -> Result<Done, JobError> {
        let corpus = Arc::clone(ctx.corpus()?);
        let idea = py::strip(&input.idea).to_string();
        let lines: Vec<String> = input
            .lines
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
            .map(|x| py::strip(x).to_string())
            .filter(|x| !x.is_empty())
            .take(MAX_LINES)
            .collect();
        let (i, l) = (idea.clone(), lines.clone());
        // the index and the ranking walk every bit he has: CPU work, off the async threads
        let grounds = tokio::task::spawn_blocking(move || riff_grounds(&corpus, &Index::new(&corpus), &i, &l))
            .await
            .map_err(|e| JobError::Value(format!("riff grounds: {e}")))?;
        let mut out = Map::new();
        out.insert("idea".into(), json!(idea));
        out.insert("draft".into(), json!(true));
        out.insert("label".into(), json!(DRAFT_LABEL));
        if grounds.is_empty() && lines.is_empty() {
            out.insert("status".into(), json!("no_grounding"));
            out.insert("ideas".into(), json!([]));
            out.insert("grounded_in".into(), json!([]));
            out.insert(
                "why".into(),
                json!("Nothing in your notebook or your riff to build on yet. Write a line first."),
            );
            return Ok(Done::result(Value::Object(out)));
        }
        let user = riff_user(&ctx.names.whose_idea(), &idea, &lines, &grounds);
        let msgs = ctx.voice.messages(TASK, RIFF_SYSTEM, [ChatMessage::user(user)]);
        match ctx
            .router
            .complete(TASK, &msgs, Some(&RIFF_SCHEMA), Some("creative"), None)
            .await
        {
            Ok(c) => {
                let ideas = riff_ideas(c.data.as_ref(), &grounds);
                out.insert("status".into(), json!(if ideas.is_empty() { "none" } else { "ok" }));
                out.insert("ideas".into(), Value::Array(ideas));
                out.insert("grounded_in".into(), json!(grounds));
                out.insert("provider".into(), json!(c.provider));
                out.insert("model".into(), json!(c.model));
            }
            Err(e) => {
                out.insert("status".into(), json!("provider_error"));
                out.insert("ideas".into(), json!([]));
                out.insert("grounded_in".into(), json!(grounds));
                out.insert("why".into(), json!(provider_why(&e)));
            }
        }
        Ok(Done::result(Value::Object(out)))
    }
}
