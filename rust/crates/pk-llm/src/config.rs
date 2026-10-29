//! The engine configuration (`engine/config.py` over `engine/config.hosted.toml`): the providers and the tasks,
//! each task deep-merged over `[tasks.default]`. Only the keys the Anthropic backend and the router read are typed;
//! `paths` and `inbox` belong to the Python pipeline and are not ported. The hosted file ships inside the crate.

use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr as _;

use serde::Deserialize;
use toml::Table;

use crate::api::Effort;
use crate::error::LlmError;

/// `engine/config.hosted.toml`, the only configuration that ships (the worker image sets `ENGINE_CONFIG` to it).
pub const HOSTED_TOML: &str = include_str!("../../../../engine/config.hosted.toml");

/// `[providers.<name>]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct ProviderCfg {
    /// `anthropic` is the only kind ported; others are skipped with a warning.
    pub kind: String,
    /// The environment variable that holds the key.
    pub api_key_env: String,
    /// The default `max_tokens` when a task sets none.
    pub max_tokens: u32,
    /// `adaptive` (the default) or anything else to omit the parameter.
    pub thinking: String,
    /// Stream every request (the default).
    pub stream: bool,
    /// Cache breakpoints on the system blocks (the default).
    pub cache: bool,
    /// Consult the daily budget (the default).
    pub budget: bool,
}

impl Default for ProviderCfg {
    fn default() -> Self {
        Self {
            kind: String::new(),
            api_key_env: "ANTHROPIC_API_KEY".into(),
            max_tokens: 16000,
            thinking: "adaptive".into(),
            stream: true,
            cache: true,
            budget: true,
        }
    }
}

/// `[tasks.<name>]` merged over `[tasks.default]`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct TaskCfg {
    /// Providers in preference order.
    pub providers: Vec<String>,
    /// Provider → model id.
    pub models: BTreeMap<String, String>,
    /// `output_config.effort`, validated against `EFFORTS`.
    pub effort: Option<String>,
    /// Overrides the provider's `max_tokens`.
    pub max_tokens: Option<u32>,
    /// Overrides the provider's `thinking`.
    pub thinking: Option<String>,
    /// Task names whose model on the same provider is tried once each after a refusal or a cut answer.
    pub fallbacks: Vec<String>,
    /// A batch task (the Python's `claude_code` gate; informational here).
    pub offline: bool,
}

impl TaskCfg {
    /// The effort as the API enum, or the Python's error text for an unknown value.
    pub fn effort(&self) -> Result<Option<Effort>, LlmError> {
        self.effort.as_deref().map(Effort::from_str).transpose()
    }

    /// The model on a provider, if the task names one.
    pub fn model_on(&self, provider: &str) -> Option<&str> {
        self.models.get(provider).map(String::as_str)
    }
}

/// The parsed TOML.
#[derive(Debug, Clone)]
pub struct Config {
    providers: BTreeMap<String, ProviderCfg>,
    tasks: BTreeMap<String, Table>,
}

impl Config {
    /// Parse a TOML text.
    pub fn parse(text: &str) -> Result<Self, LlmError> {
        let root: Table = text.parse().map_err(|e| LlmError::Config(format!("config: {e}")))?;
        let providers = match root.get("providers") {
            Some(toml::Value::Table(t)) => t
                .iter()
                .map(|(k, v)| {
                    v.clone()
                        .try_into::<ProviderCfg>()
                        .map(|p| (k.clone(), p))
                        .map_err(|e| LlmError::Config(format!("providers.{k}: {e}")))
                })
                .collect::<Result<_, _>>()?,
            None => BTreeMap::new(),
            Some(_) => return Err(LlmError::Config("providers must be a table".into())),
        };
        let tasks = match root.get("tasks") {
            Some(toml::Value::Table(t)) => t
                .iter()
                .map(|(k, v)| match v {
                    toml::Value::Table(tt) => Ok((k.clone(), tt.clone())),
                    _ => Err(LlmError::Config(format!("tasks.{k} must be a table"))),
                })
                .collect::<Result<_, _>>()?,
            None => BTreeMap::new(),
            Some(_) => return Err(LlmError::Config("tasks must be a table".into())),
        };
        Ok(Self { providers, tasks })
    }

    /// The hosted configuration compiled in.
    pub fn hosted() -> Self {
        Self::parse(HOSTED_TOML).expect("the hosted config parses")
    }

    /// Read a TOML file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, LlmError> {
        let text = std::fs::read_to_string(path.as_ref())
            .map_err(|e| LlmError::Config(format!("{}: {e}", path.as_ref().display())))?;
        Self::parse(&text)
    }

    /// The providers by name.
    pub fn providers(&self) -> &BTreeMap<String, ProviderCfg> {
        &self.providers
    }

    /// Every task name, `default` included, sorted.
    pub fn task_names(&self) -> Vec<String> {
        self.tasks.keys().cloned().collect()
    }

    /// `Config.task(name)`: `default` deep-updated with the task's own table (an unknown task is the default).
    pub fn task(&self, name: &str) -> Result<TaskCfg, LlmError> {
        let mut merged = self.tasks.get("default").cloned().unwrap_or_default();
        if let Some(own) = self.tasks.get(name) {
            deep_update(&mut merged, own);
        }
        toml::Value::Table(merged)
            .try_into()
            .map_err(|e| LlmError::Config(format!("tasks.{name}: {e}")))
    }
}

/// `config.py` `_deep_update`: tables merge, anything else replaces.
fn deep_update(dst: &mut Table, src: &Table) {
    for (k, v) in src {
        match (dst.get_mut(k), v) {
            (Some(toml::Value::Table(d)), toml::Value::Table(s)) => deep_update(d, s),
            _ => {
                dst.insert(k.clone(), v.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_tasks_merge_over_default() {
        let c = Config::hosted();
        let riff = c.task("creative_riff").unwrap();
        assert_eq!(riff.providers, vec!["anthropic"]);
        assert_eq!(riff.model_on("anthropic"), Some("claude-opus-5-5"));
        assert_eq!(riff.effort().unwrap(), Some(Effort::High));
        assert_eq!(riff.fallbacks, vec!["default"]);
        assert!(riff.offline);
        let d = c.task("default").unwrap();
        assert_eq!(d.model_on("anthropic"), Some("claude-sonnet-5-5"));
        assert!(d.fallbacks.is_empty() && !d.offline);
        assert_eq!(c.task("no_such_task").unwrap(), d);
        let p = &c.providers()["anthropic"];
        assert!(p.stream && p.cache && p.budget && p.max_tokens == 16000 && p.thinking == "adaptive");
    }

    #[test]
    fn a_bad_effort_is_the_pythons_words() {
        let c = Config::parse("[tasks.x]\neffort = \"ultra\"\n").unwrap();
        let e = c.task("x").unwrap().effort().unwrap_err().to_string();
        assert_eq!(
            e,
            "effort must be one of ('low', 'medium', 'high', 'xhigh', 'max'), not 'ultra'"
        );
    }
}
