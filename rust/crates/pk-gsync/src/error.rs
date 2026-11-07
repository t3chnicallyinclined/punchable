//! The errors of a run, named the way the Python names them: the run record stores `"{type}: {message}"` and the
//! messages are part of the contract (the owner reads them in `status` and the log).

use pk_core::py;
use serde_json::Value;

/// A Google API answer of 300 or more (`googleapiclient.errors.HttpError`).
#[derive(Debug, Clone)]
pub struct HttpError {
    /// The HTTP status.
    pub status: u16,
    /// The request URL (with its query string).
    pub uri: String,
    /// The reason: the body's `error.message`, else the HTTP reason phrase.
    pub reason: String,
    /// `error.detail` / `details` / `errors` / `message` of the body, or an empty string.
    pub details: Value,
}

impl HttpError {
    /// What `HttpError(resp, content, uri)` makes of an answer (`_get_reason`).
    pub fn new(status: u16, phrase: &str, content: &[u8], uri: &str) -> Self {
        let mut reason = phrase.to_string();
        let mut details = Value::String(String::new());
        if let Ok(text) = std::str::from_utf8(content) {
            match serde_json::from_str::<Value>(text) {
                Ok(Value::Object(m)) => {
                    if let Some(msg) = m.get("error").and_then(|e| e.get("message")) {
                        reason = py::str(msg);
                        let err = &m["error"];
                        let kw = ["detail", "details", "errors", "message"]
                            .into_iter()
                            .find(|k| err.get(*k).is_some());
                        if let Some(k) = kw {
                            details = err[k].clone();
                        }
                    }
                }
                Ok(Value::Array(a)) if !a.is_empty() => {
                    if let Some(msg) = a[0].get("error").and_then(|e| e.get("message")) {
                        reason = py::str(msg);
                        if let Some(d) = a[0]["error"].get("details") {
                            details = d.clone();
                        }
                    }
                }
                Ok(_) | Err(_) => details = Value::String(text.to_string()),
            }
        }
        Self {
            status,
            uri: uri.to_string(),
            reason: py::strip(&reason).to_string(),
            details,
        }
    }

    /// `error_details[0].get("reason")` as `GoogleDrive._x` reads it; anything else is `""`.
    pub fn reason_code(&self) -> String {
        match &self.details {
            Value::Array(a) if !a.is_empty() => a[0]
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            _ => String::new(),
        }
    }
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if py::truthy(Some(&self.details)) {
            write!(
                f,
                "<HttpError {} when requesting {} returned \"{}\". Details: \"{}\">",
                self.status,
                self.uri,
                self.reason,
                py::str(&self.details)
            )
        } else {
            write!(
                f,
                "<HttpError {} when requesting {} returned \"{}\">",
                self.status, self.uri, self.reason
            )
        }
    }
}

/// Everything that can go wrong in a run.
#[derive(Debug, thiserror::Error)]
pub enum GsError {
    /// `WriteBlocked`: a write gsync is not allowed to make.
    #[error("{0}")]
    WriteBlocked(String),
    /// `ApiDown`: Punchable did not answer.
    #[error("{0}")]
    ApiDown(String),
    /// `ApiError`: Punchable answered with an error status.
    #[error("{status}: {msg}")]
    Api {
        /// The status.
        status: u16,
        /// The API's `error` word, else the body.
        msg: String,
    },
    /// `AuthMissing`: a sign-in step is missing; the message says which.
    #[error("{0}")]
    AuthMissing(String),
    /// Google refused or failed a Drive call.
    #[error("{0}")]
    Http(Box<HttpError>),
    /// `RefreshError`: Google refused the refresh token.
    #[error("{0}")]
    Refresh(String),
    /// `SystemExit(message)`: stop the program, message to stderr, exit 1.
    #[error("{0}")]
    Exit(String),
    /// Any other Python exception: its class name and its text.
    #[error("{msg}")]
    Other {
        /// The Python class name (`OSError`, `ValueError`, `URLError`, ...).
        name: String,
        /// `str(exception)`.
        msg: String,
    },
}

impl GsError {
    /// An exception of the given Python class.
    pub fn other(name: &str, msg: impl Into<String>) -> Self {
        Self::Other {
            name: name.to_string(),
            msg: msg.into(),
        }
    }

    /// The Python class name.
    pub fn type_name(&self) -> &str {
        match self {
            Self::WriteBlocked(_) => "WriteBlocked",
            Self::ApiDown(_) => "ApiDown",
            Self::Api { .. } => "ApiError",
            Self::AuthMissing(_) => "AuthMissing",
            Self::Http(_) => "HttpError",
            Self::Refresh(_) => "RefreshError",
            Self::Exit(_) => "SystemExit",
            Self::Other { name, .. } => name,
        }
    }

    /// `f"{type(e).__name__}: {e}"`, which is what the run record keeps.
    pub fn describe(&self) -> String {
        format!("{}: {}", self.type_name(), self)
    }
}

impl From<std::io::Error> for GsError {
    fn from(e: std::io::Error) -> Self {
        let name = match e.kind() {
            std::io::ErrorKind::NotFound => "FileNotFoundError",
            std::io::ErrorKind::PermissionDenied => "PermissionError",
            std::io::ErrorKind::AlreadyExists => "FileExistsError",
            _ => "OSError",
        };
        Self::other(name, e.to_string())
    }
}

impl From<serde_json::Error> for GsError {
    fn from(e: serde_json::Error) -> Self {
        Self::other("JSONDecodeError", e.to_string())
    }
}

/// A result in a run.
pub type Result<T> = std::result::Result<T, GsError>;
