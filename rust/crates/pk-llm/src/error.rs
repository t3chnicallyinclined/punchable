//! The one error of the crate, with the plain words the comic reads copied from `engine/providers.py` (`Refusal`,
//! `CutShort`) and `engine/budget.py` (`BudgetExceeded`): the app shows these texts, so they must not change.

use crate::pyfmt;

/// Everything that can go wrong between a task and its answer.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// The model declined (`stop_reason` `refusal`); `category` and `explanation` are the API's `stop_details`.
    #[error("{}", refusal_words(category.as_deref(), explanation.as_deref()))]
    Refusal {
        /// The API's `stop_details.category`.
        category: Option<String>,
        /// The API's `stop_details.explanation`.
        explanation: Option<String>,
    },
    /// `stop_reason` `max_tokens`: the answer was cut off.
    #[error(
        "Claude ran out of room on this one (max_tokens {max_tokens}): the answer was cut short. Try a shorter idea."
    )]
    CutShort {
        /// The `max_tokens` the request carried.
        max_tokens: u32,
    },
    /// HTTP 429 after the client's retries.
    #[error("{0}")]
    RateLimited(String),
    /// Today's cap is spent: the caller keeps its job queued and says why (the text is `Budget::reason`).
    #[error("{0}")]
    Budget(String),
    /// No provider in the task's chain has a key.
    #[error("no available provider for task {}", pyfmt::quote(task))]
    NoProvider {
        /// The task name.
        task: String,
    },
    /// The answer was not the JSON the schema asks for (the message is the parser's or the validator's, cut to 200
    /// characters as the Python does).
    #[error("invalid JSON: {0}")]
    Schema(String),
    /// A schema `transform_schema` cannot express (the SDK raises the same way).
    #[error("{0}")]
    BadSchema(String),
    /// The wire: a connection error, a non-2xx status other than 429, a malformed stream.
    #[error("{}", http_words(*status, message))]
    Http {
        /// The HTTP status when a response came back.
        status: Option<u16>,
        /// The body or the transport's words.
        message: String,
    },
    /// The configuration: a bad TOML, an unknown effort, a missing model.
    #[error("{0}")]
    Config(String),
    /// Every provider in the chain failed for a reason that is not a refusal or a cut answer.
    #[error("task {} failed on all providers: {attempts:?}; last: {last}", pyfmt::quote(task))]
    Failed {
        /// The task name.
        task: String,
        /// `(provider, error kind)` per attempt, as the runs log records them.
        attempts: Vec<(String, String)>,
        /// The last error's words.
        last: String,
    },
}

impl LlmError {
    /// The Python exception class name the runs log and `Completion::attempts` record.
    pub fn kind(&self) -> &'static str {
        match self {
            LlmError::Refusal { .. } => "Refusal",
            LlmError::CutShort { .. } => "CutShort",
            LlmError::RateLimited(_) => "RateLimited",
            LlmError::Budget(_) => "BudgetExceeded",
            LlmError::NoProvider { .. } => "NoProvider",
            LlmError::Schema(_) => "invalid_json",
            LlmError::BadSchema(_) => "ValueError",
            LlmError::Http { status: Some(_), .. } => "APIStatusError",
            LlmError::Http { status: None, .. } => "APIConnectionError",
            LlmError::Config(_) | LlmError::Failed { .. } => "ProviderError",
        }
    }

    /// A refusal or a cut answer: the words survive to the caller instead of a generic failure.
    pub fn is_plain(&self) -> bool {
        matches!(self, LlmError::Refusal { .. } | LlmError::CutShort { .. })
    }
}

/// `providers.py` `Refusal.__init__`: the explanation when there is one (trimmed, no trailing dot), else the category
/// in parentheses, else nothing.
fn refusal_words(category: Option<&str>, explanation: Option<&str>) -> String {
    let why = explanation.unwrap_or("").trim().trim_end_matches('.');
    let tail = if why.is_empty() {
        category.map(|c| format!(" ({c})")).unwrap_or_default()
    } else {
        format!(": {why}")
    };
    format!("Claude declined this one{tail}. Try rewording.")
}

fn http_words(status: Option<u16>, message: &str) -> String {
    match status {
        Some(s) => format!("Error code: {s} - {message}"),
        None => format!("Connection error: {message}"),
    }
}

impl From<reqwest::Error> for LlmError {
    fn from(e: reqwest::Error) -> Self {
        LlmError::Http {
            status: e.status().map(|s| s.as_u16()),
            message: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_words_follow_the_python() {
        let r = |c: Option<&str>, e: Option<&str>| {
            LlmError::Refusal {
                category: c.map(String::from),
                explanation: e.map(String::from),
            }
            .to_string()
        };
        assert_eq!(r(None, None), "Claude declined this one. Try rewording.");
        assert_eq!(
            r(Some("cyber"), None),
            "Claude declined this one (cyber). Try rewording."
        );
        assert_eq!(
            r(Some("cyber"), Some(" why. ")),
            "Claude declined this one: why. Try rewording."
        );
        assert_eq!(
            r(Some("x"), Some("   ")),
            "Claude declined this one (x). Try rewording."
        );
    }
}
