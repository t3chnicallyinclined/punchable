//! CORS exactly as `server.py` answers it: a page served from this machine (`http(s)://127.0.0.1` or `localhost`, any
//! port) or from `PUBLIC_URL` may call the API. Every JSON answer echoes an allowed `Origin` with `Vary: Origin` and
//! the exposed range headers; a preflight (`OPTIONS`, any path) is a 204 with the fixed method and header lists,
//! whatever the origin.
//!
//! Written out rather than `tower_http::cors::CorsLayer`: that layer answers a preflight only when
//! `Access-Control-Request-Method` is present and varies its headers by request, and the Python answers every
//! `OPTIONS` the same way. The phone and the web app both depend on the exact lists (the upload headers).

use std::sync::LazyLock;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use regex::Regex;

use crate::state::AppState;

/// `Access-Control-Allow-Methods` on a preflight.
pub const ALLOW_METHODS: &str = "GET, POST, PATCH, DELETE, OPTIONS";
/// `Access-Control-Allow-Headers` on a preflight (the upload path's `X-*` headers included).
pub const ALLOW_HEADERS: &str = "Authorization, Content-Type, Range, X-Filename, X-Kind, X-Title, X-Duration-S, \
     X-Marks-Ms, X-Recorded-At, X-Venue, X-Lat, X-Lng, X-Show-Group, X-Align-Working-Set";
/// `Access-Control-Expose-Headers` on an answer to an allowed origin.
pub const EXPOSE_HEADERS: &str = "Content-Range, Accept-Ranges, Content-Length";

static LOOPBACK: LazyLock<Regex> = LazyLock::new(|| {
    // `re.match(r"^https?://(127\.0\.0\.1|localhost)(:\d+)?$", o)`: `$` also matches before one final newline
    Regex::new(r"\Ahttps?://(?:127\.0\.0\.1|localhost)(?::[0-9]+)?\n?\z")
        .unwrap_or_else(|e| unreachable!("constant regex: {e}"))
});

/// `origin_ok`: loopback on any port, or one of the configured origins exactly.
pub fn origin_ok(origin: &str, allowed: &[String]) -> bool {
    LOOPBACK.is_match(origin) || allowed.iter().any(|a| a == origin)
}

/// The CORS headers for `origin`, added to an answer when it is allowed (`server.py` `cors`).
pub fn add_cors(headers: &mut HeaderMap, origin: Option<&HeaderValue>, allowed: &[String]) {
    let Some(o) = origin else { return };
    let Ok(text) = o.to_str() else { return };
    if origin_ok(text, allowed) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, o.clone());
        headers.insert(header::VARY, HeaderValue::from_static("Origin"));
        headers.insert(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static(EXPOSE_HEADERS),
        );
    }
}

/// The preflight answer (`server.py` `do_OPTIONS`).
pub fn preflight(origin: Option<&HeaderValue>, allowed: &[String]) -> Response {
    let mut r = StatusCode::NO_CONTENT.into_response();
    let h = r.headers_mut();
    add_cors(h, origin, allowed);
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static(ALLOW_METHODS),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static(ALLOW_HEADERS),
    );
    h.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from_static("0"));
    r
}

/// The middleware over the routes this service answers itself: preflights here, CORS headers on every answer.
pub async fn cors(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let origin = req.headers().get(header::ORIGIN).cloned();
    let allowed = st.config().origins.as_slice();
    if req.method() == Method::OPTIONS {
        return preflight(origin.as_ref(), allowed);
    }
    let mut r = next.run(req).await;
    add_cors(r.headers_mut(), origin.as_ref(), allowed);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_and_the_public_url_are_allowed() {
        let pub_url = vec!["https://punchable.io".to_string()];
        assert!(origin_ok("http://localhost:8081", &pub_url));
        assert!(origin_ok("https://127.0.0.1", &pub_url));
        assert!(origin_ok("http://localhost\n", &pub_url));
        assert!(origin_ok("https://punchable.io", &pub_url));
        assert!(!origin_ok("https://punchable.io/", &pub_url));
        assert!(!origin_ok("http://localhost.evil.io", &pub_url));
        assert!(!origin_ok("http://127.0.0.1:80x", &pub_url));
        assert!(!origin_ok("", &pub_url));
    }
}
