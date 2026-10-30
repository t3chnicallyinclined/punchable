//! `providers.py` `AnthropicBackend`: the request per task (system blocks with cache breakpoints on the first, the
//! last and, with three, the middle one; adaptive thinking on Opus and Sonnet; `effort`; the transformed schema as
//! `output_config.format`; `max_tokens` from the task or the provider), the call through the client, the budget
//! checked before and charged after, a refusal or a cut answer raised as plain words.

use serde_json::Value;

use crate::api::{CacheControl, FormatKind, Message, OutputConfig, OutputFormat, Request, Role, SystemBlock, Thinking};
use crate::budget::Budget;
use crate::client::Client;
use crate::config::{ProviderCfg, TaskCfg};
use crate::error::LlmError;
use crate::schema::transform_schema;

/// A message as the callers hand them over: `role` is `system`, `user` or `assistant`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    /// `system`, `user` or `assistant`.
    pub role: String,
    /// The words.
    pub content: String,
}

impl ChatMessage {
    /// A message with a role.
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
        }
    }

    /// A system block.
    pub fn system(content: impl Into<String>) -> Self {
        Self::new("system", content)
    }

    /// The comic's turn.
    pub fn user(content: impl Into<String>) -> Self {
        Self::new("user", content)
    }

    /// A previous answer.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new("assistant", content)
    }
}

/// What one call returned: the text and the numbers the runs log records.
#[derive(Debug, Clone, PartialEq)]
pub struct Called {
    /// The text blocks joined.
    pub text: String,
    /// `usage.input_tokens`.
    pub tokens_in: u64,
    /// `usage.output_tokens`.
    pub tokens_out: u64,
    /// `usage.cache_read_input_tokens` or 0.
    pub cache_read: u64,
    /// `usage.cache_creation_input_tokens` or 0.
    pub cache_write: u64,
    /// What the budget charged, 6 decimals, when there is a budget.
    pub usd: Option<f64>,
}

/// Claude through the Messages API: the hosted worker's only provider.
#[derive(Debug)]
pub struct AnthropicBackend {
    /// The provider's name in the config (`anthropic`).
    pub name: String,
    pcfg: ProviderCfg,
    client: Client,
    budget: Option<Budget>,
}

impl AnthropicBackend {
    /// A backend over a client (tests point it at a mock server) with or without a budget.
    pub fn new(name: impl Into<String>, pcfg: ProviderCfg, client: Client, budget: Option<Budget>) -> Self {
        Self {
            name: name.into(),
            pcfg,
            client,
            budget,
        }
    }

    /// The provider's configuration.
    pub fn pcfg(&self) -> &ProviderCfg {
        &self.pcfg
    }

    /// The budget, when one guards this backend.
    pub fn budget(&self) -> Option<&Budget> {
        self.budget.as_ref()
    }

    /// `thinking: {type: adaptive}` goes on Opus and Sonnet; Haiku is sent without (it thinks adaptively by
    /// default); a task or the provider may say `thinking = "none"` to omit it anywhere.
    pub fn thinks(model: &str, task: &TaskCfg, pcfg: &ProviderCfg) -> bool {
        let mode = task.thinking.as_deref().unwrap_or(&pcfg.thinking);
        mode == "adaptive" && !model.to_lowercase().starts_with("claude-haiku")
    }

    /// The request body for one call, exactly what the Python hands the SDK (plus `stream` when streaming).
    pub fn build_request(
        &self,
        model: &str,
        messages: &[ChatMessage],
        schema: Option<&Value>,
        task: &TaskCfg,
    ) -> Result<Request, LlmError> {
        let mut system: Vec<SystemBlock> = messages
            .iter()
            .filter(|m| m.role == "system" && !m.content.is_empty())
            .map(|m| SystemBlock::text(m.content.clone()))
            .collect();
        let turns: Vec<Message> = messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| {
                let role = match m.role.as_str() {
                    "assistant" => Role::Assistant,
                    _ => Role::User,
                };
                Message {
                    role,
                    content: m.content.clone(),
                }
            })
            .collect();
        let max_tokens = task.max_tokens.filter(|n| *n > 0).unwrap_or(self.pcfg.max_tokens);
        if !system.is_empty() && self.pcfg.cache {
            // the first block is the stable prefix shared by every task (the voice profile); the last is the task's
            // own line: two breakpoints (of the four allowed), so a different task still reuses the first. With
            // three blocks the middle one is the learned section: its own breakpoint, so riff and punch-up share
            // the [profile + learned] prefix too
            let n = system.len();
            system[0].cache_control = Some(CacheControl::ephemeral());
            system[n - 1].cache_control = Some(CacheControl::ephemeral());
            if n >= 3 {
                system[1].cache_control = Some(CacheControl::ephemeral());
            }
        }
        let mut output_config = OutputConfig {
            effort: task.effort()?,
            format: None,
        };
        if let Some(s) = schema {
            output_config.format = Some(OutputFormat {
                kind: FormatKind::JsonSchema,
                schema: transform_schema(s)?,
            });
        }
        Ok(Request {
            model: model.to_string(),
            max_tokens,
            messages: turns,
            system: (!system.is_empty()).then_some(system),
            thinking: Self::thinks(model, task, &self.pcfg).then(Thinking::adaptive),
            output_config: (output_config.effort.is_some() || output_config.format.is_some()).then_some(output_config),
            stream: self.pcfg.stream.then_some(true),
        })
    }

    /// One call: the budget checked first, the request sent, the usage charged, then a refusal or a cut answer as
    /// plain words, else the text.
    pub async fn call(
        &self,
        model: &str,
        messages: &[ChatMessage],
        schema: Option<&Value>,
        task: &TaskCfg,
        task_name: &str,
    ) -> Result<Called, LlmError> {
        if let Some(b) = &self.budget {
            b.check()?; // the worker keeps the job queued and says why
        }
        let request = self.build_request(model, messages, schema, task)?;
        let response = self.client.send(&request).await?;
        let u = &response.usage;
        let usd = match &self.budget {
            Some(b) => Some(round6(b.charge(model, u, task_name)?)),
            None => None,
        };
        if response.stop_reason.as_deref() == Some("refusal") {
            let (category, explanation) = response
                .stop_details
                .as_ref()
                .map_or((None, None), |d| (d.category.clone(), d.explanation.clone()));
            return Err(LlmError::Refusal { category, explanation });
        }
        if response.stop_reason.as_deref() == Some("max_tokens") {
            return Err(LlmError::CutShort {
                max_tokens: request.max_tokens,
            });
        }
        Ok(Called {
            text: response.text(),
            tokens_in: u.input_tokens,
            tokens_out: u.output_tokens,
            cache_read: u.cache_read(),
            cache_write: u.cache_write(),
            usd,
        })
    }
}

/// Python's `round(usd, 6)`.
fn round6(x: f64) -> f64 {
    (x * 1_000_000.0).round() / 1_000_000.0
}
