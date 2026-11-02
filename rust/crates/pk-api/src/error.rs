//! The API's errors as the app reads them: `{"error": msg}` with `Cache-Control: no-store`, and on a 401 exactly the
//! `WWW-Authenticate` challenge the Python sends (`api-dev/server.py` `err`, `deny`). The texts are part of the
//! contract: the app shows some of them.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

/// The challenge on every 401 (`server.py` `deny`).
pub const WWW_AUTHENTICATE: &str = r#"Bearer realm="punchable", error="invalid_token""#;

/// An error answered as `{"error": msg}` with its status.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{status}: {msg}")]
pub struct ApiError {
    /// The HTTP status.
    pub status: StatusCode,
    /// The message the app may show.
    pub msg: String,
}

impl ApiError {
    /// An error with any status.
    pub fn new(status: StatusCode, msg: impl Into<String>) -> Self {
        Self {
            status,
            msg: msg.into(),
        }
    }

    /// 400.
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }

    /// 401 (the response carries the challenge).
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, msg)
    }

    /// 403.
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, msg)
    }

    /// 404.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, msg)
    }

    /// 503.
    pub fn unavailable(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, msg)
    }

    /// A fault of ours: a fixed-shape 500 (`"<Kind>: <message>"`, the Python's `f"{type(e).__name__}: {e}"`). Every
    /// 500 goes to the crash reporter (through the scrubber) when it is answered.
    pub fn internal(kind: &str, msg: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{kind}: {msg}"))
    }
}

impl From<pk_core::py::PyError> for ApiError {
    /// A `ValueError` / `TypeError` raised while reading the request answers 400 with its text (`server.py`
    /// `except ValueError as e: return self.err(400, str(e))`).
    fn from(e: pk_core::py::PyError) -> Self {
        Self::bad_request(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() && self.status != StatusCode::SERVICE_UNAVAILABLE {
            pk_obs::crash::report(&self, None, Some(self.status.as_u16()), &[]);
        }
        let mut r = json_response(self.status, &json!({ "error": self.msg }));
        if self.status == StatusCode::UNAUTHORIZED {
            r.headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static(WWW_AUTHENTICATE));
        }
        r
    }
}

/// A JSON answer the way `server.py` `send_json` writes one: UTF-8, `no-store`.
pub fn json_response(status: StatusCode, body: &Value) -> Response {
    // Serialising a `Value` cannot fail (string keys only); an empty body would be the only fallback.
    json_bytes(status, serde_json::to_vec(body).unwrap_or_default().into())
}

/// A JSON body already serialised (a cached answer), with the same headers as [`json_response`].
pub fn json_bytes(status: StatusCode, bytes: axum::body::Bytes) -> Response {
    let mut r = (status, bytes).into_response();
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// 200 with a JSON body.
pub fn ok(body: &Value) -> Response {
    json_response(StatusCode::OK, body)
}

/// The result type of a handler.
pub type ApiResult<T = Response> = Result<T, ApiError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_401_carries_the_challenge_and_no_store() {
        let r = ApiError::unauthorized("sign in required").into_response();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(r.headers()[header::WWW_AUTHENTICATE], WWW_AUTHENTICATE);
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
        let r = ApiError::forbidden("x").into_response();
        assert!(r.headers().get(header::WWW_AUTHENTICATE).is_none());
    }
}
