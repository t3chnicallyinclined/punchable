//! `ask_chat` (`creative/askchat.py` `run_job`): one model call over the messages the API already built from his
//! retrieved sources, his profile first as background (never a source). The API gates the answer word for word
//! when it reads the result back, so nothing is checked here.

use std::sync::LazyLock;

use pk_llm::ChatMessage;
use pk_wire::worker::AskChatInput;
use serde_json::{Value, json};

use super::{Ctx, Done, Runner};
use crate::error::JobError;

/// The task in `engine/config.hosted.toml`.
pub const TASK: &str = "ask_chat";

/// `askchat.SCHEMA`: the answer's shape; the job never supplies one.
pub static SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({"type": "object", "properties": {
        "found": {"type": "boolean"},
        "answer": {"type": "string"},
        "citations": {"type": "array", "items": {"type": "object", "properties": {
            "source": {"type": "string"}, "quote": {"type": "string"}},
            "required": ["source", "quote"], "additionalProperties": false}}},
        "required": ["found", "answer", "citations"], "additionalProperties": false})
});

const BAD: &str = "bad ask_chat input";

/// The `ask_chat` kind.
#[derive(Debug)]
pub struct AskChat;

impl Runner for AskChat {
    type Input = AskChatInput;

    fn bad_input(_: &serde_json::Error) -> JobError {
        JobError::Value(BAD.to_string())
    }

    async fn run(ctx: Ctx<'_>, input: AskChatInput) -> Result<Done, JobError> {
        let ok = !input.messages.is_empty()
            && input
                .messages
                .iter()
                .all(|m| matches!(m.role.as_str(), "system" | "user" | "assistant"));
        if !ok {
            return Err(JobError::Value(BAD.to_string()));
        }
        let mut msgs: Vec<ChatMessage> = Vec::with_capacity(input.messages.len() + 1);
        if let Some(block) = ctx.voice.system_block(TASK) {
            msgs.push(ChatMessage::system(block));
        }
        msgs.extend(input.messages.into_iter().map(|m| ChatMessage::new(m.role, m.content)));
        let c = ctx
            .router
            .complete(TASK, &msgs, Some(&SCHEMA), Some("creative"), None)
            .await?;
        let data = c.data.filter(|d| d.as_object().is_none_or(|m| !m.is_empty()));
        Ok(Done::result(json!({
            "status": "answered",
            "data": data.unwrap_or_else(|| json!({})),
            "provider": c.provider,
            "model": c.model,
        })))
    }
}
