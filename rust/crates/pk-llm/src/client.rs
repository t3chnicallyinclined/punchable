//! The HTTP client for `POST /v1/messages`: the headers the SDK sends, the SDK's retry policy (408/409/429/5xx and
//! connection errors, twice, `retry-after` honoured up to 60 s, else exponential back-off from 0.5 s to 8 s), the
//! SSE stream folded into the final response, and the status → error mapping (429 is `RateLimited`).

use std::time::Duration;

use eventsource_stream::Eventsource;
use futures::StreamExt as _;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::api::{Request, Response, StreamEvent, accumulate};
use crate::error::LlmError;

/// The first-party API; `ANTHROPIC_BASE_URL` overrides it as it does for the SDK.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// The API version header every request carries.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// The SDK's default: two retries after the first attempt.
pub const DEFAULT_MAX_RETRIES: u32 = 2;
const INITIAL_RETRY_DELAY: f64 = 0.5;
const MAX_RETRY_DELAY: f64 = 8.0;
/// The SDK's default request timeout (per read here: a long draft streams for longer than ten minutes in total).
const READ_TIMEOUT: Duration = Duration::from_secs(600);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// A Messages API client. The key is never logged.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    max_retries: u32,
}

impl Client {
    /// A client on the first-party API (or `ANTHROPIC_BASE_URL`) with the SDK's defaults.
    pub fn new(api_key: impl Into<String>) -> Result<Self, LlmError> {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .user_agent(concat!("pk-llm/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let base_url = std::env::var("ANTHROPIC_BASE_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        Ok(Self {
            http,
            base_url,
            api_key: api_key.into(),
            max_retries: DEFAULT_MAX_RETRIES,
        })
    }

    /// Point the client elsewhere (tests: a `wiremock` server).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_string();
        self
    }

    /// How many times a retryable failure is retried (0 disables).
    #[must_use]
    pub fn with_max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// The base URL in use.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Send the request and return the final response: streamed and folded when `request.stream` is `true`, the
    /// plain JSON otherwise.
    pub async fn send(&self, request: &Request) -> Result<Response, LlmError> {
        let url = format!("{}/v1/messages", self.base_url);
        let mut attempt: u32 = 0;
        loop {
            let sent = self.http.post(&url).headers(self.headers()).json(request).send().await;
            let response = match sent {
                Ok(r) => r,
                Err(e) => {
                    if attempt < self.max_retries {
                        tracing::debug!(attempt, "connection error, retrying: {e}");
                        tokio::time::sleep(retry_delay(attempt, None)).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(e.into());
                }
            };
            let status = response.status();
            if !status.is_success() {
                let headers = response.headers().clone();
                let body = response.text().await.unwrap_or_default();
                if should_retry(status, &headers) && attempt < self.max_retries {
                    tracing::debug!(%status, attempt, "retrying");
                    tokio::time::sleep(retry_delay(attempt, Some(&headers))).await;
                    attempt += 1;
                    continue;
                }
                let message = format!("Error code: {} - {}", status.as_u16(), body);
                return Err(if status == StatusCode::TOO_MANY_REQUESTS {
                    LlmError::RateLimited(message)
                } else {
                    LlmError::Http {
                        status: Some(status.as_u16()),
                        message: body,
                    }
                });
            }
            return if request.stream == Some(true) {
                Self::fold_stream(response).await
            } else {
                Ok(response.json::<Response>().await?)
            };
        }
    }

    async fn fold_stream(response: reqwest::Response) -> Result<Response, LlmError> {
        let mut events = response.bytes_stream().eventsource();
        let mut snapshot: Option<Response> = None;
        while let Some(item) = events.next().await {
            let ev = item.map_err(|e| LlmError::Http {
                status: None,
                message: format!("bad event stream: {e}"),
            })?;
            let event: StreamEvent = serde_json::from_str(&ev.data).map_err(|e| LlmError::Http {
                status: None,
                message: format!("bad event {:?}: {e}", ev.event),
            })?;
            let done = matches!(event, StreamEvent::MessageStop);
            accumulate(&mut snapshot, event)?;
            if done {
                break;
            }
        }
        snapshot.ok_or_else(|| LlmError::Http {
            status: None,
            message: "the stream ended before message_start".into(),
        })
    }

    fn headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("anthropic-version", HeaderValue::from_static(ANTHROPIC_VERSION));
        h.insert("accept", HeaderValue::from_static("application/json"));
        if let Ok(v) = HeaderValue::from_str(&self.api_key) {
            h.insert("x-api-key", v);
        }
        h
    }
}

/// The SDK's `_calculate_retry_timeout` without its jitter: `retry-after-ms` or `retry-after` when within a
/// minute, else `min(0.5 * 2^n, 8)` seconds.
fn retry_delay(attempt: u32, headers: Option<&HeaderMap>) -> Duration {
    if let Some(h) = headers {
        let from_header = header_f64(h, "retry-after-ms")
            .map(|ms| ms / 1000.0)
            .or_else(|| header_f64(h, "retry-after"));
        if let Some(s) = from_header
            && s > 0.0
            && s <= 60.0
        {
            return Duration::from_secs_f64(s);
        }
    }
    let n = attempt.min(1000);
    let secs = (INITIAL_RETRY_DELAY * 2f64.powf(f64::from(n))).min(MAX_RETRY_DELAY);
    Duration::from_secs_f64(secs)
}

fn header_f64(h: &HeaderMap, name: &str) -> Option<f64> {
    h.get(name)?.to_str().ok()?.trim().parse::<f64>().ok()
}

/// The SDK's `_should_retry`: the `x-should-retry` header decides when present; else 408, 409, 429 and 5xx.
fn should_retry(status: StatusCode, headers: &HeaderMap) -> bool {
    match headers.get("x-should-retry").and_then(|v| v.to_str().ok()) {
        Some("true") => return true,
        Some("false") => return false,
        _ => {}
    }
    matches!(status.as_u16(), 408 | 409 | 429) || status.is_server_error()
}
