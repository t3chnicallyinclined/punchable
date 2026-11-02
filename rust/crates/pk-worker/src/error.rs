//! What can go wrong while a job runs, and how each failure reaches the API: the Python worker posts
//! `"<ExceptionType>: <message>"` (cut to 1,500 characters) with `retry` set for an IO error, a timeout or the day's
//! budget, and the budget's own words with no type in front (`worker/__main__.py` `run_one`). The type names are the
//! Python's, because the app and the job store show them.

use pk_core::py;
use pk_creative::corpus::CorpusError;
use pk_llm::LlmError;

use crate::api::ApiError;

/// The longest error text a result carries (`run_one` cuts at 1,500; the API cuts again at 2,000).
pub const MAX_ERROR_CHARS: usize = 1500;

/// A job that did not finish with a result.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    /// Bad input or a rule the kind enforces (`ValueError` in the Python).
    #[error("{0}")]
    Value(String),
    /// The model call: a refusal, a cut answer, the day's budget, every provider failing.
    #[error("{}", llm_message(.0))]
    Llm(#[from] LlmError),
    /// A call to the API.
    #[error(transparent)]
    Api(#[from] ApiError),
    /// His notebook could not be built from the mirror (a file missing or not JSON).
    #[error(transparent)]
    Corpus(#[from] CorpusError),
    /// The bge-small model refused (`RuntimeError` in the Python's embedder).
    #[error(transparent)]
    Embed(#[from] pk_embed::EmbedError),
    /// A creative log that does not replay (the Python's `Store` raises the reducer's `ValueError`).
    #[error("{}", .0.msg())]
    Creative(#[from] pk_creative::CreativeError),
}

impl JobError {
    /// The Python exception class this failure is reported as.
    pub fn py_type(&self) -> &'static str {
        match self {
            JobError::Value(_) | JobError::Creative(_) => "ValueError",
            JobError::Llm(e) => llm_type(e),
            JobError::Api(e) => e.py_type(),
            JobError::Corpus(CorpusError::Missing(_)) => "FileNotFoundError",
            JobError::Corpus(CorpusError::Json { .. }) => "JSONDecodeError",
            JobError::Embed(_) => "RuntimeError",
        }
    }

    /// The day's AI budget ran out: the job goes back to the queue with the reason and is never a crash report.
    pub fn is_budget(&self) -> bool {
        matches!(self, JobError::Llm(LlmError::Budget(_)))
    }

    /// Back to the queue rather than failed: the budget, an IO error, a timeout (`OSError`, `TimeoutError`).
    pub fn retry(&self) -> bool {
        // a missing notebook file is the Python's FileNotFoundError, an OSError: back to the queue
        self.is_budget()
            || matches!(self, JobError::Api(e) if e.retry())
            || matches!(self, JobError::Corpus(CorpusError::Missing(_)))
    }

    /// The `error` a failed result carries: the budget's words alone, else `"<Type>: <message>"`, cut.
    pub fn result_text(&self) -> String {
        let text = if self.is_budget() {
            self.to_string()
        } else {
            format!("{}: {self}", self.py_type())
        };
        text.chars().take(MAX_ERROR_CHARS).collect()
    }
}

/// The Python class name of an error the router lets out (`engine/providers.py`, `engine/budget.py`).
pub fn llm_type(e: &LlmError) -> &'static str {
    match e {
        LlmError::Refusal { .. } => "Refusal",
        LlmError::CutShort { .. } => "CutShort",
        LlmError::Budget(_) => "BudgetExceeded",
        LlmError::NoProvider { .. } => "NoProvider",
        LlmError::Failed { .. } | LlmError::Config(_) | LlmError::Schema(_) | LlmError::BadSchema(_) => "ProviderError",
        other => other.kind(),
    }
}

/// `str(e)` as the Python prints it. `Failed` lists its attempts as a Python list of tuples
/// (`[('anthropic', 'invalid_json')]`); `pk-llm`'s own `Display` prints Rust's `Debug` there.
pub fn llm_message(e: &LlmError) -> String {
    match e {
        LlmError::Failed { task, attempts, last } => {
            let list: Vec<String> = attempts
                .iter()
                .map(|(p, k)| format!("({}, {})", py::quote(p), py::quote(k)))
                .collect();
            format!(
                "task {} failed on all providers: [{}]; last: {last}",
                py::quote(task),
                list.join(", ")
            )
        }
        other => other.to_string(),
    }
}

/// `creative/util.py` `provider_why`: what a writing screen says when its model call fails. A refusal or a cut
/// answer is already plain words; anything else names the error, short.
pub fn provider_why(e: &LlmError) -> String {
    if e.is_plain() {
        e.to_string()
    } else {
        let msg: String = llm_message(e).chars().take(160).collect();
        format!("{}: {msg}", llm_type(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_lists_attempts_the_python_way() {
        let e = LlmError::Failed {
            task: "ask_chat".into(),
            attempts: vec![("anthropic".into(), "invalid_json".into())],
            last: "invalid JSON: x".into(),
        };
        assert_eq!(
            JobError::from(e).result_text(),
            "ProviderError: task 'ask_chat' failed on all providers: [('anthropic', 'invalid_json')]; last: invalid JSON: x"
        );
    }

    #[test]
    fn budget_is_a_retry_with_its_own_words() {
        let e = JobError::from(LlmError::Budget("over today's AI budget".into()));
        assert!(e.retry() && e.is_budget());
        assert_eq!(e.result_text(), "over today's AI budget");
        let v = JobError::Value("x".repeat(2000));
        assert!(!v.retry());
        assert_eq!(v.result_text().chars().count(), MAX_ERROR_CHARS);
    }

    #[test]
    fn provider_why_keeps_plain_words() {
        let r = LlmError::Refusal {
            category: None,
            explanation: None,
        };
        assert_eq!(provider_why(&r), "Claude declined this one. Try rewording.");
        assert_eq!(
            provider_why(&LlmError::NoProvider { task: "x".into() }),
            "NoProvider: no available provider for task 'x'"
        );
    }
}
