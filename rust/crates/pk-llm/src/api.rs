//! The Messages API on the wire, typed: the request the backend builds (`Request`), the response object
//! (`Response`, the API's `Message`), its `Usage`, and the stream events with the accumulator that folds them into
//! the final response the way the Python SDK's `get_final_message()` does. There is no official Rust SDK: these
//! are serde models of the documented JSON, covering what this project sends and reads (text, thinking, structured
//! output; no tools).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::LlmError;

/// A turn's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// The comic's material and the retry's correction.
    User,
    /// A previous answer (the retry echoes the bad one).
    Assistant,
}

/// One turn; the content is a string (this project never sends content blocks in turns).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Who speaks.
    pub role: Role,
    /// The words.
    pub content: String,
}

/// `{"type": "ephemeral"}`: a prompt-cache breakpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheControl {
    /// Always `ephemeral`.
    #[serde(rename = "type")]
    pub kind: CacheKind,
}

/// The one cache kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheKind {
    /// The default five-minute cache.
    Ephemeral,
}

impl CacheControl {
    /// The breakpoint the backend puts on system blocks.
    pub fn ephemeral() -> Self {
        Self {
            kind: CacheKind::Ephemeral,
        }
    }
}

/// A system prompt block: text, with a cache breakpoint or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemBlock {
    /// Always `text`.
    #[serde(rename = "type")]
    pub kind: TextKind,
    /// The words.
    pub text: String,
    /// The breakpoint, when this block ends a cached prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// The one block kind a system prompt has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextKind {
    /// Plain text.
    Text,
}

impl SystemBlock {
    /// A block without a breakpoint.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            kind: TextKind::Text,
            text: text.into(),
            cache_control: None,
        }
    }
}

/// `thinking: {"type": "adaptive"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thinking {
    /// The mode.
    #[serde(rename = "type")]
    pub kind: ThinkingKind,
}

/// The thinking modes this project sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingKind {
    /// The model decides when and how much to think.
    Adaptive,
}

impl Thinking {
    /// Adaptive thinking.
    pub fn adaptive() -> Self {
        Self {
            kind: ThinkingKind::Adaptive,
        }
    }
}

/// `output_config.effort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    /// Quick.
    Low,
    /// The default on the hosted config's default task.
    Medium,
    /// The API's default.
    High,
    /// Between high and max.
    #[serde(rename = "xhigh")]
    XHigh,
    /// Correctness over cost.
    Max,
}

/// The values `[tasks.x] effort` may take, in the Python's order (the config error names them).
pub const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

impl std::str::FromStr for Effort {
    type Err = LlmError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "low" => Ok(Effort::Low),
            "medium" => Ok(Effort::Medium),
            "high" => Ok(Effort::High),
            "xhigh" => Ok(Effort::XHigh),
            "max" => Ok(Effort::Max),
            other => Err(LlmError::Config(format!(
                "effort must be one of ({}), not {}",
                EFFORTS
                    .iter()
                    .map(|e| pk_core::py::quote(e))
                    .collect::<Vec<_>>()
                    .join(", "),
                pk_core::py::quote(other)
            ))),
        }
    }
}

/// `output_config.format`: structured output against a (transformed) JSON Schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputFormat {
    /// Always `json_schema`.
    #[serde(rename = "type")]
    pub kind: FormatKind,
    /// The schema after `transform_schema`.
    pub schema: Value,
}

/// The one output format kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatKind {
    /// A JSON Schema.
    JsonSchema,
}

/// `output_config`: effort and/or format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OutputConfig {
    /// Thinking depth and token spend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    /// Structured output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<OutputFormat>,
}

/// `POST /v1/messages`, as this project sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// The model id.
    pub model: String,
    /// The output ceiling; hitting it is a `CutShort`.
    pub max_tokens: u32,
    /// The turns.
    pub messages: Vec<Message>,
    /// The system blocks, breakpoints on the first, the last and (with three) the middle one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Vec<SystemBlock>>,
    /// Adaptive thinking on Opus and Sonnet; omitted on Haiku and when a task says `thinking = "none"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<Thinking>,
    /// Effort and structured output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<OutputConfig>,
    /// `true` when the answer streams (the SDK adds it to the body; so does this client).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

/// The response's token counts; the budget prices them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Usage {
    /// Uncached input tokens.
    pub input_tokens: u64,
    /// Output tokens (final after `message_delta`).
    pub output_tokens: u64,
    /// Tokens written to the prompt cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
    /// Tokens served from the prompt cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
    /// Whatever else the API reports (`service_tier`, `cache_creation`, ...), kept as is.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Usage {
    /// `cache_creation_input_tokens` or 0.
    pub fn cache_write(&self) -> u64 {
        self.cache_creation_input_tokens.unwrap_or(0)
    }

    /// `cache_read_input_tokens` or 0.
    pub fn cache_read(&self) -> u64 {
        self.cache_read_input_tokens.unwrap_or(0)
    }
}

/// `stop_details`, present when `stop_reason` is `refusal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopDetails {
    /// `refusal`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The refusal category (an open set).
    #[serde(default)]
    pub category: Option<String>,
    /// The API's explanation, when it gives one.
    #[serde(default)]
    pub explanation: Option<String>,
}

/// A content block of the answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// The words.
    Text {
        /// The text so far.
        text: String,
    },
    /// The (summarised or empty) thinking.
    Thinking {
        /// The thinking text so far.
        thinking: String,
        /// The signature that lets the block be replayed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    /// Anything else (tool use, server tool results): kept, not read.
    #[serde(other)]
    Other,
}

/// The API's `Message` object: the final answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// `msg_...`.
    pub id: String,
    /// Always `message`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Always `assistant`.
    pub role: String,
    /// The model that answered.
    pub model: String,
    /// The blocks.
    pub content: Vec<ContentBlock>,
    /// `end_turn`, `max_tokens`, `refusal`, ... (an open set).
    #[serde(default)]
    pub stop_reason: Option<String>,
    /// The custom stop sequence hit, if any.
    #[serde(default)]
    pub stop_sequence: Option<String>,
    /// Set when the stop reason is `refusal`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_details: Option<StopDetails>,
    /// The token counts.
    pub usage: Usage,
}

impl Response {
    /// Every text block joined, the way `AnthropicBackend.call` reads the answer.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// A delta inside a `content_block_delta` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Delta {
    /// More text.
    TextDelta {
        /// The piece.
        text: String,
    },
    /// More thinking.
    ThinkingDelta {
        /// The piece.
        thinking: String,
    },
    /// The thinking block's signature.
    SignatureDelta {
        /// The signature.
        signature: String,
    },
    /// Anything else (`input_json_delta`, `citations_delta`): not read.
    #[serde(other)]
    Other,
}

/// The `delta` of a `message_delta` event.
#[derive(Debug, Clone, Deserialize)]
pub struct MessageDelta {
    /// The final stop reason.
    #[serde(default)]
    pub stop_reason: Option<String>,
    /// The stop sequence hit.
    #[serde(default)]
    pub stop_sequence: Option<String>,
    /// The refusal details.
    #[serde(default)]
    pub stop_details: Option<StopDetails>,
}

/// The `usage` of a `message_delta` event: cumulative output tokens, the input counts when the API restates them.
#[derive(Debug, Clone, Deserialize)]
pub struct DeltaUsage {
    /// Output tokens so far (final on the last delta).
    pub output_tokens: u64,
    /// Restated input tokens.
    #[serde(default)]
    pub input_tokens: Option<u64>,
    /// Restated cache writes.
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    /// Restated cache reads.
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
}

/// The body of an `error` event.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiError {
    /// `overloaded_error`, `api_error`, ...
    #[serde(rename = "type")]
    pub kind: String,
    /// The API's words.
    pub message: String,
}

/// One SSE event of a Messages stream.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// The response shell: id, model, the input usage, no content yet.
    MessageStart {
        /// The shell.
        message: Response,
    },
    /// A new block at `index`.
    ContentBlockStart {
        /// Its position.
        index: usize,
        /// The empty block.
        content_block: ContentBlock,
    },
    /// More of the block at `index`.
    ContentBlockDelta {
        /// Which block.
        index: usize,
        /// The piece.
        delta: Delta,
    },
    /// The block at `index` is complete.
    ContentBlockStop {
        /// Which block.
        index: usize,
    },
    /// The stop reason and the final output tokens.
    MessageDelta {
        /// Stop reason, sequence, details.
        delta: MessageDelta,
        /// Output tokens so far.
        usage: DeltaUsage,
    },
    /// The end.
    MessageStop,
    /// Keep-alive.
    Ping,
    /// The API failed mid-stream.
    Error {
        /// Why.
        error: ApiError,
    },
    /// An event this client does not know: ignored.
    #[serde(other)]
    Other,
}

/// Fold one event into the snapshot, as the SDK's `accumulate_event` does. The first event must be
/// `message_start`; an `error` event is the API's failure.
pub fn accumulate(snapshot: &mut Option<Response>, event: StreamEvent) -> Result<(), LlmError> {
    let Some(snap) = snapshot.as_mut() else {
        return match event {
            StreamEvent::MessageStart { message } => {
                *snapshot = Some(message);
                Ok(())
            }
            StreamEvent::Error { error } => Err(LlmError::Http {
                status: None,
                message: error_words(&error),
            }),
            StreamEvent::Ping | StreamEvent::Other => Ok(()),
            other => Err(LlmError::Http {
                status: None,
                message: format!(
                    "Unexpected event order, got {} before \"message_start\"",
                    event_name(&other)
                ),
            }),
        };
    };
    match event {
        StreamEvent::ContentBlockStart { content_block, .. } => snap.content.push(content_block),
        StreamEvent::ContentBlockDelta { index, delta } => {
            if let Some(block) = snap.content.get_mut(index) {
                match (block, delta) {
                    (ContentBlock::Text { text }, Delta::TextDelta { text: more }) => text.push_str(&more),
                    (ContentBlock::Thinking { thinking, .. }, Delta::ThinkingDelta { thinking: more }) => {
                        thinking.push_str(&more);
                    }
                    (ContentBlock::Thinking { signature, .. }, Delta::SignatureDelta { signature: sig }) => {
                        *signature = Some(sig);
                    }
                    _ => {}
                }
            }
        }
        StreamEvent::MessageStart { .. }
        | StreamEvent::ContentBlockStop { .. }
        | StreamEvent::MessageStop
        | StreamEvent::Ping
        | StreamEvent::Other => {}
        StreamEvent::MessageDelta { delta, usage } => {
            snap.stop_reason = delta.stop_reason;
            snap.stop_sequence = delta.stop_sequence;
            if delta.stop_details.is_some() {
                snap.stop_details = delta.stop_details;
            }
            snap.usage.output_tokens = usage.output_tokens;
            if let Some(n) = usage.input_tokens {
                snap.usage.input_tokens = n;
            }
            if usage.cache_creation_input_tokens.is_some() {
                snap.usage.cache_creation_input_tokens = usage.cache_creation_input_tokens;
            }
            if usage.cache_read_input_tokens.is_some() {
                snap.usage.cache_read_input_tokens = usage.cache_read_input_tokens;
            }
        }
        StreamEvent::Error { error } => {
            return Err(LlmError::Http {
                status: None,
                message: error_words(&error),
            });
        }
    }
    Ok(())
}

fn error_words(e: &ApiError) -> String {
    format!("{}: {}", e.kind, e.message)
}

fn event_name(e: &StreamEvent) -> &'static str {
    match e {
        StreamEvent::MessageStart { .. } => "message_start",
        StreamEvent::ContentBlockStart { .. } => "content_block_start",
        StreamEvent::ContentBlockDelta { .. } => "content_block_delta",
        StreamEvent::ContentBlockStop { .. } => "content_block_stop",
        StreamEvent::MessageDelta { .. } => "message_delta",
        StreamEvent::MessageStop => "message_stop",
        StreamEvent::Ping => "ping",
        StreamEvent::Error { .. } => "error",
        StreamEvent::Other => "unknown",
    }
}
