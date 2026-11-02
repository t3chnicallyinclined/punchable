//! The worker's one client for the API (`worker/__main__.py` `Client`): JSON over HTTP to `/api/worker/*`, a
//! Keycloak client-credentials token as `pk-worker` cached until 30 seconds before it expires and dropped on a 401,
//! and the Python's error words (`"<METHOD> <path>: HTTP <code> <first 300 bytes>"`). The secret is never logged.

use std::fmt;
use std::time::{Duration, Instant};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::Mutex;

/// The `User-Agent` every request carries, as the Python's.
pub const USER_AGENT: &str = "punchable-worker/1";
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
const TOKEN_TIMEOUT: Duration = Duration::from_secs(30);
const ERROR_BODY_BYTES: usize = 300;

/// `urllib.parse.quote`'s safe set: letters, digits, `_.-~` and `/` stay, everything else is percent-encoded.
const QUOTE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b'/');

/// `urllib.parse.quote(s)`: a job id in a path (`job:riff-…` → `job%3Ariff-…`).
pub fn quote(s: &str) -> String {
    utf8_percent_encode(s, QUOTE).to_string()
}

/// A secret that prints as `[redacted]`.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    /// Wrap a secret value.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// Where the token comes from.
#[derive(Debug, Clone)]
pub struct Credentials {
    /// The realm's token endpoint.
    pub token_url: String,
    /// `pk-worker`.
    pub client_id: String,
    /// `PK_WORKER_SECRET`.
    pub secret: Secret,
}

/// A failed call to the API or to Keycloak.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// The API answered with an error status.
    #[error("{method} {path}: HTTP {status} {body}")]
    Status {
        /// `GET`, `POST`, …
        method: String,
        /// The path as called.
        path: String,
        /// The status code.
        status: u16,
        /// The first 300 bytes of the body.
        body: String,
    },
    /// Keycloak refused the client credentials.
    #[error("Keycloak refused the worker's client credentials (HTTP {0})")]
    Token(u16),
    /// No answer in time.
    #[error("{0}")]
    Timeout(String),
    /// The connection: refused, reset, DNS.
    #[error("{0}")]
    Transport(String),
    /// The body was not the JSON expected.
    #[error("{0}")]
    Decode(String),
}

impl ApiError {
    /// The Python exception class (`RuntimeError` for a status, `URLError` / `TimeoutError` for the network).
    pub fn py_type(&self) -> &'static str {
        match self {
            ApiError::Status { .. } | ApiError::Token(_) => "RuntimeError",
            ApiError::Timeout(_) => "TimeoutError",
            ApiError::Transport(_) => "URLError",
            ApiError::Decode(_) => "JSONDecodeError",
        }
    }

    /// An `OSError` in the Python: the job goes back to the queue.
    pub fn retry(&self) -> bool {
        matches!(self, ApiError::Timeout(_) | ApiError::Transport(_))
    }
}

/// The error with its causes (`reqwest`'s own words stop at "error sending request"; the cause says refused,
/// reset or DNS).
fn with_causes(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        let words = c.to_string();
        if !out.contains(&words) {
            out.push_str(": ");
            out.push_str(&words);
        }
        cause = c.source();
    }
    out
}

impl From<reqwest::Error> for ApiError {
    fn from(e: reqwest::Error) -> Self {
        let words = with_causes(&e);
        if e.is_timeout() {
            ApiError::Timeout(format!("timed out: {words}"))
        } else if e.is_decode() {
            ApiError::Decode(words)
        } else {
            ApiError::Transport(words)
        }
    }
}

#[derive(serde::Deserialize)]
struct TokenAnswer {
    access_token: String,
    expires_in: Option<f64>,
}

#[derive(Debug)]
struct Token {
    value: String,
    expires: Instant,
}

#[derive(Debug)]
struct Auth {
    creds: Credentials,
    token: Mutex<Option<Token>>,
}

/// The client. Cheap to share behind an `Arc`; concurrent callers share one token (the refresh is single-flight).
#[derive(Debug)]
pub struct ApiClient {
    http: reqwest::Client,
    base: String,
    auth: Option<Auth>,
}

impl ApiClient {
    /// A client on `base` (`http://punchable-api` in the cluster); `creds` none = auth off (the loopback dev API).
    pub fn new(base: &str, creds: Option<Credentials>) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(CALL_TIMEOUT)
            .build()?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            auth: creds.map(|creds| Auth {
                creds,
                token: Mutex::new(None),
            }),
        })
    }

    /// The base URL.
    pub fn base(&self) -> &str {
        &self.base
    }

    async fn bearer(&self) -> Result<Option<String>, ApiError> {
        let Some(auth) = &self.auth else {
            return Ok(None);
        };
        let mut slot = auth.token.lock().await;
        if let Some(t) = slot.as_ref()
            && Instant::now() + Duration::from_secs(30) < t.expires
        {
            return Ok(Some(t.value.clone()));
        }
        let form = [
            ("grant_type", "client_credentials"),
            ("client_id", auth.creds.client_id.as_str()),
            ("client_secret", auth.creds.secret.expose()),
        ];
        let resp = self
            .http
            .post(&auth.creds.token_url)
            .timeout(TOKEN_TIMEOUT)
            .form(&form)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(ApiError::Token(resp.status().as_u16()));
        }
        let a: TokenAnswer = resp.json().await?;
        // a lifetime that does not fit (negative, NaN, absurd) is read as a minute, never a panic
        let ttl = Duration::try_from_secs_f64(a.expires_in.unwrap_or(60.0)).unwrap_or(Duration::from_secs(60));
        let now = Instant::now();
        let value = a.access_token;
        *slot = Some(Token {
            value: value.clone(),
            expires: now.checked_add(ttl).unwrap_or(now),
        });
        Ok(Some(value))
    }

    async fn drop_token(&self) {
        if let Some(auth) = &self.auth {
            *auth.token.lock().await = None;
        }
    }

    /// One call. `None` on 204 or an empty body; the parsed JSON otherwise.
    pub async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&(impl Serialize + Sync)>,
    ) -> Result<Option<Value>, ApiError> {
        let mut req = self
            .http
            .request(method.clone(), format!("{}{path}", self.base))
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(token) = self.bearer().await? {
            req = req.bearer_auth(token);
        }
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status.is_client_error() || status.is_server_error() {
            if status == StatusCode::UNAUTHORIZED {
                self.drop_token().await;
            }
            let bytes = resp.bytes().await.unwrap_or_default();
            let cut = &bytes[..bytes.len().min(ERROR_BODY_BYTES)];
            return Err(ApiError::Status {
                method: method.to_string(),
                path: path.to_string(),
                status: status.as_u16(),
                body: String::from_utf8_lossy(cut).into_owned(),
            });
        }
        if status == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let bytes = resp.bytes().await?;
        if bytes.is_empty() {
            return Ok(None);
        }
        let v: Value = serde_json::from_slice(&bytes).map_err(|e| ApiError::Decode(e.to_string()))?;
        Ok((!v.is_null()).then_some(v))
    }

    /// [`ApiClient::call`] read into a type; `None` on 204.
    pub async fn call_as<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&(impl Serialize + Sync)>,
    ) -> Result<Option<T>, ApiError> {
        match self.call(method, path, body).await? {
            Some(v) => serde_json::from_value(v)
                .map(Some)
                .map_err(|e| ApiError::Decode(format!("{path}: {e}"))),
            None => Ok(None),
        }
    }

    /// A GET with no body.
    pub async fn get(&self, path: &str) -> Result<Option<Value>, ApiError> {
        self.call(Method::GET, path, None::<&Value>).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_is_urllib_quote() {
        assert_eq!(quote("job:riff-0a1b"), "job%3Ariff-0a1b");
        assert_eq!(quote("a b/c~d_e.f"), "a%20b/c~d_e.f");
        assert_eq!(quote("é"), "%C3%A9");
    }

    #[test]
    fn the_secret_never_prints() {
        let c = Credentials {
            token_url: "https://auth.example/t".into(),
            client_id: "pk-worker".into(),
            secret: Secret::new("s3cr3t"),
        };
        assert!(!format!("{c:?}").contains("s3cr3t"));
    }
}
