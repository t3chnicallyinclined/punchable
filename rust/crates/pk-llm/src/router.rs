//! `providers.py` `Router`: a task name → the providers in its chain → the model, then the task's `fallbacks`
//! models once each after a refusal or a cut answer; one retry on an answer that is not the JSON the schema asks
//! for (the bad answer and a correction appended to the turns); every call, success or failure, appended to the runs
//! log with its tokens, its cost and its cache numbers, never a key.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use pk_core::py;

use crate::backend::{AnthropicBackend, Called, ChatMessage};
use crate::budget::Budget;
use crate::client::Client;
use crate::config::{Config, LlmConfig, TaskCfg};
use crate::error::LlmError;
use crate::schema::{cut, parse_and_validate, schema_instructions};

/// A finished call: the text, the parsed data when a schema was given, and the accounting.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Completion {
    /// The raw text.
    pub text: String,
    /// The parsed, schema-validated JSON when a schema was given.
    pub data: Option<Value>,
    /// The task name.
    pub task: String,
    /// The provider that answered.
    pub provider: String,
    /// The model that answered.
    pub model: String,
    /// Input tokens.
    pub tokens_in: u64,
    /// Output tokens.
    pub tokens_out: u64,
    /// Wall time of the answering call.
    pub ms: u64,
    /// The runs-log id of the answering call.
    pub call_id: String,
    /// The hash of the prompt (messages and schema).
    pub prompt_sha: String,
    /// `(provider, error kind)` for every attempt before the answer.
    pub attempts: Vec<(String, String)>,
}

/// One line of the runs log (`runs.jsonl`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallRecord {
    /// UTC, seconds.
    pub ts: String,
    /// 12 hex chars.
    pub call_id: String,
    /// The caller's run id.
    pub run_id: Option<String>,
    /// The task.
    pub task: String,
    /// The provider.
    pub provider: String,
    /// The model.
    pub model: String,
    /// The prompt hash.
    pub prompt_sha: String,
    /// The caller's few-shot hash.
    pub fewshot_sha: Option<String>,
    /// Input tokens (0 on failure).
    pub tokens_in: u64,
    /// Output tokens (0 on failure).
    pub tokens_out: u64,
    /// Wall time.
    pub ms: u64,
    /// Answered and valid.
    pub ok: bool,
    /// What the budget charged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usd: Option<f64>,
    /// Cache reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    /// Cache writes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<u64>,
    /// `Kind: words` on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The task router over the configured backends.
#[derive(Debug)]
pub struct Router {
    cfg: Config,
    backends: BTreeMap<String, AnthropicBackend>,
    log_path: Option<PathBuf>,
    calls: Mutex<Vec<CallRecord>>,
}

impl Router {
    /// A router over given backends (tests inject one on a mock server); `log_path` is the runs log, or none.
    pub fn new(cfg: Config, backends: Vec<AnthropicBackend>, log_path: Option<PathBuf>) -> Self {
        Self {
            cfg,
            backends: backends.into_iter().map(|b| (b.name.clone(), b)).collect(),
            log_path,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// The hosted router: an `AnthropicBackend` per `kind = "anthropic"` provider whose key `lc` holds (the binary
    /// filled it from the environment); other kinds are not ported and are skipped with a warning.
    pub fn hosted(cfg: Config, lc: &LlmConfig, log_path: Option<PathBuf>) -> Result<Self, LlmError> {
        let mut backends = Vec::new();
        for (name, pcfg) in cfg.providers() {
            if pcfg.kind != "anthropic" {
                tracing::warn!(provider = %name, kind = %pcfg.kind, "provider kind not ported; skipped");
                continue;
            }
            let Some(key) = lc.api_keys.get(name) else {
                continue;
            };
            let mut client = Client::new(key.clone())?;
            if let Some(url) = &lc.base_url {
                client = client.with_base_url(url.clone());
            }
            let budget = pcfg.budget.then(|| Budget::from_config(&lc.budget));
            backends.push(AnthropicBackend::new(name, pcfg.clone(), client, budget));
        }
        Ok(Self::new(cfg, backends, log_path))
    }

    /// The configuration.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// The calls this router made, in order.
    pub fn calls(&self) -> Vec<CallRecord> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// `[(provider, model)]` in preference order, available ones only.
    pub fn resolve(&self, task: &str) -> Result<Vec<(String, String)>, LlmError> {
        let tcfg = self.cfg.task(task)?;
        Ok(tcfg
            .providers
            .iter()
            .filter(|name| self.backends.contains_key(*name))
            .filter_map(|name| tcfg.model_on(name).map(|m| (name.clone(), m.to_string())))
            .collect())
    }

    /// `provider:model` of the first choice, or `none`.
    pub fn describe(&self, task: &str) -> String {
        match self.resolve(task).ok().and_then(|r| r.into_iter().next()) {
            Some((p, m)) => format!("{p}:{m}"),
            None => "none".to_string(),
        }
    }

    /// The model, then the task's `fallbacks` (task names whose model on this provider is tried once each after a
    /// refusal or a cut answer; `["default"]` = the default task's model). Duplicates and the model itself skipped.
    pub fn models_for(&self, tcfg: &TaskCfg, provider: &str, model: &str) -> Vec<String> {
        let mut out = vec![model.to_string()];
        for t in &tcfg.fallbacks {
            if let Some(fm) = self
                .cfg
                .task(t)
                .ok()
                .and_then(|c| c.model_on(provider).map(str::to_string))
                && !out.contains(&fm)
            {
                out.push(fm);
            }
        }
        out
    }

    /// Run a task over the messages; with a schema the answer is parsed and validated (one retry), and `data` is
    /// set. A refusal or a cut answer on every model reaches the caller as those plain words; the day's cap as
    /// `Budget` with no other provider tried.
    pub async fn complete(
        &self,
        task: &str,
        messages: &[ChatMessage],
        schema: Option<&Value>,
        run_id: Option<&str>,
        fewshot_sha: Option<&str>,
    ) -> Result<Completion, LlmError> {
        let tcfg = self.cfg.task(task)?;
        let prompt_sha = sha16(&json!({"messages": messages, "schema": schema}));
        let chain = self.resolve(task)?;
        if chain.is_empty() {
            return Err(LlmError::NoProvider { task: task.to_string() });
        }
        let mut attempts: Vec<(String, String)> = Vec::new();
        let mut last: Option<LlmError> = None;
        for (name, model) in chain {
            let b = &self.backends[&name];
            for m in self.models_for(&tcfg, &name, &model) {
                let ctx = Ctx {
                    task,
                    run_id,
                    fewshot_sha,
                    prompt_sha: &prompt_sha,
                    messages,
                    schema,
                    tcfg: &tcfg,
                };
                match self.attempt(b, &m, &ctx, &mut attempts).await {
                    Ok(c) => return Ok(c),
                    Err(e @ LlmError::Budget(_)) => return Err(e), // the day's cap: the caller keeps the job
                    Err(e) => {
                        let plain = e.is_plain();
                        last = Some(e);
                        if !plain {
                            break; // provider down, rate limited, bad JSON twice: the next provider, not the next model
                        }
                    }
                }
            }
        }
        match last {
            Some(e) if e.is_plain() => Err(e), // the plain words survive to the caller (and to the screen)
            other => Err(LlmError::Failed {
                task: task.to_string(),
                attempts,
                last: other.map(|e| e.to_string()).unwrap_or_default(),
            }),
        }
    }

    /// One model on one provider, with one retry on unparseable / invalid JSON.
    async fn attempt(
        &self,
        b: &AnthropicBackend,
        model: &str,
        ctx: &Ctx<'_>,
        attempts: &mut Vec<(String, String)>,
    ) -> Result<Completion, LlmError> {
        let (messages, schema, tcfg) = (ctx.messages, ctx.schema, ctx.tcfg);
        let mut msgs = messages.to_vec();
        let mut last: Option<LlmError> = None;
        for _ in 0..2 {
            let t0 = Instant::now();
            let call_id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
            let called: Called = match b.call(model, &msgs, schema, tcfg, ctx.task).await {
                Ok(c) => c,
                Err(e) => {
                    let ms = t0.elapsed().as_millis() as u64;
                    let words = format!("{}: {}", e.kind(), cut(&e.to_string(), 200));
                    self.log(ctx, &call_id, &b.name, model, 0, 0, ms, false, Some(words), None);
                    if !matches!(e, LlmError::Budget(_)) {
                        attempts.push((b.name.clone(), e.kind().to_string()));
                    }
                    return Err(e);
                }
            };
            let ms = t0.elapsed().as_millis() as u64;
            let (data, err) = match schema {
                Some(s) => match parse_and_validate(&called.text, s) {
                    Ok(d) => (Some(d), None),
                    Err(e) => (None, Some(e)),
                },
                None => (None, None),
            };
            self.log(
                ctx,
                &call_id,
                &b.name,
                model,
                called.tokens_in,
                called.tokens_out,
                ms,
                err.is_none(),
                err.as_ref().map(ToString::to_string),
                Some(&called),
            );
            let Some(err) = err else {
                return Ok(Completion {
                    text: called.text,
                    data,
                    task: ctx.task.to_string(),
                    provider: b.name.clone(),
                    model: model.to_string(),
                    tokens_in: called.tokens_in,
                    tokens_out: called.tokens_out,
                    ms,
                    call_id,
                    prompt_sha: ctx.prompt_sha.to_string(),
                    attempts: attempts.clone(),
                });
            };
            attempts.push((b.name.clone(), "invalid_json".to_string()));
            let correction = format!(
                "That was not valid: {err}. {}",
                schema_instructions(schema.expect("an invalid answer needs a schema"))
            );
            msgs = messages.to_vec();
            msgs.push(ChatMessage::assistant(cut(&called.text, 4000)));
            msgs.push(ChatMessage::user(correction));
            last = Some(err);
        }
        Err(last.expect("two attempts leave an error"))
    }

    #[allow(clippy::too_many_arguments)]
    fn log(
        &self,
        ctx: &Ctx<'_>,
        call_id: &str,
        provider: &str,
        model: &str,
        tokens_in: u64,
        tokens_out: u64,
        ms: u64,
        ok: bool,
        error: Option<String>,
        called: Option<&Called>,
    ) {
        let rec = CallRecord {
            ts: pk_core::time::now_iso(),
            call_id: call_id.to_string(),
            run_id: ctx.run_id.map(str::to_string),
            task: ctx.task.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            prompt_sha: ctx.prompt_sha.to_string(),
            fewshot_sha: ctx.fewshot_sha.map(str::to_string),
            tokens_in,
            tokens_out,
            ms,
            ok,
            usd: called.and_then(|c| c.usd),
            cache_read: called.map(|c| c.cache_read),
            cache_write: called.map(|c| c.cache_write),
            error,
        };
        if let Some(path) = &self.log_path
            && let Err(e) = append_line(path, &rec)
        {
            tracing::warn!("runs log {}: {e}", path.display());
        }
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(rec);
    }
}

/// What every call of one `complete` shares.
struct Ctx<'a> {
    task: &'a str,
    run_id: Option<&'a str>,
    fewshot_sha: Option<&'a str>,
    prompt_sha: &'a str,
    messages: &'a [ChatMessage],
    schema: Option<&'a Value>,
    tcfg: &'a TaskCfg,
}

fn append_line(path: &PathBuf, rec: &CallRecord) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new().append(true).create(true).open(path)?;
    writeln!(f, "{}", serde_json::to_string(rec).expect("a record serialises"))
}

/// `providers.py` `sha`: the first 16 hex of sha256 over `json.dumps(obj, sort_keys=True, ensure_ascii=False)`.
pub fn sha16(value: &Value) -> String {
    let text = py::dumps_sorted(value);
    hex::encode(Sha256::digest(text.as_bytes()))[..16].to_string()
}
