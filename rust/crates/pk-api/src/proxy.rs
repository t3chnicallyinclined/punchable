//! The fallback. With `PK_PYTHON_URL` set (T9 to T11): every request this service does not answer itself goes to the
//! Python API unchanged, streamed both ways, so an Ingress path moved here can never swallow a write (an Ingress prefix
//! routes every method: docs/ARCHITECTURE.md section 2). The Python authenticates, counts and answers those requests
//! itself; this layer adds nothing to them except one count and one log line (`pk_api_forwarded_total`, below).
//!
//! With no `PK_PYTHON_URL` (T11 round 3: pk-api stands alone) the fallback answers what the Python answers to a path
//! it does not know, in the Python's order: auth, the 503 while booting, `?backend=`, and for a POST the body
//! (`body_json`) before the 404; a method the Python has no handler for (`TRACE`, ...) is its 501 page. The auth and
//! CORS layers around that answer are [`standalone_layers`], applied at the router (a forward must not get them).

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::StreamExt as _;
use prometheus::{IntCounterVec, Opts};

use crate::error::ApiError;
use crate::state::AppState;

/// The most distinct route labels `pk_api_forwarded_total` keeps; later ones are counted under `other`, so a scanner
/// walking made-up paths cannot grow the series without bound.
const MAX_FORWARD_ROUTES: usize = 64;

/// `pk_api_forwarded_total{method,route,to}`: every request that reached the fallback instead of a route of this service.
/// `to="python"` is a request passed on to the Python (the number that must be zero before the cutover);
/// `to="none"` is one this service answered itself with the Python's answer for an unknown path (no `PK_PYTHON_URL`).
/// Registered on the service's own registry, beside the Python-shared families (their golden stays untouched).
#[derive(Debug)]
pub struct Forwards {
    counter: IntCounterVec,
    seen: Mutex<HashSet<String>>,
}

impl Forwards {
    /// The counter, registered on `registry` (a second registration of the same name is a programming fault).
    pub fn new(registry: &prometheus::Registry) -> Self {
        let counter = IntCounterVec::new(
            Opts::new(
                "pk_api_forwarded_total",
                "Requests that reached the fallback: to=python was passed to the Python API, to=none was answered here as unknown",
            ),
            &["method", "route", "to"],
        )
        .unwrap_or_else(|e| unreachable!("constant metric definition: {e}"));
        if let Err(e) = registry.register(Box::new(counter.clone())) {
            tracing::error!(error = %e, "the forward counter could not be registered");
        }
        Self {
            counter,
            seen: Mutex::new(HashSet::new()),
        }
    }

    /// Count one request (method and route pattern only: never an id, a query or a body).
    pub fn count(&self, method: &Method, path: &str, to: &str) -> String {
        let route = pk_obs::route_pattern(path);
        let label = {
            let mut seen = self.seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if seen.contains(&route) || (seen.len() < MAX_FORWARD_ROUTES && seen.insert(route.clone())) {
                route
            } else {
                "other".to_string()
            }
        };
        self.counter
            .with_label_values(&[pk_obs::labels::method_label(method.as_str()), &label, to])
            .inc();
        label
    }
}

/// Hop-by-hop headers (RFC 9110 section 7.6.1) never forwarded.
const HOP: [HeaderName; 7] = [
    header::CONNECTION,
    header::PROXY_AUTHENTICATE,
    header::PROXY_AUTHORIZATION,
    header::TE,
    header::TRAILER,
    header::TRANSFER_ENCODING,
    header::UPGRADE,
];

/// How long a request waits for one of the fallback's slots before it is answered 503.
const PERMIT_WAIT: Duration = Duration::from_secs(30);

fn copy_headers(from: &HeaderMap, to: &mut HeaderMap) {
    // headers the `Connection` header names are hop-by-hop too
    let named: Vec<String> = from
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|s| s.trim().to_ascii_lowercase())
        .collect();
    for (k, v) in from {
        if !HOP.contains(k) && k != header::HOST && k.as_str() != "keep-alive" && !named.iter().any(|n| n == k.as_str())
        {
            to.append(k.clone(), v.clone());
        }
    }
}

/// Set on a response that came from the Python: the metrics of this service never count it (it is the Python's latency,
/// not ours), whether the fallback sent it or an owned route forwards while its feature is off.
#[derive(Debug, Clone, Copy)]
pub struct Uncounted;

/// The fallback: forward to `PK_PYTHON_URL`, or (none configured) answer the Python's answer for an unknown path.
/// Either way the request is counted and logged by route pattern.
pub async fn forward(State(st): State<AppState>, req: Request) -> Response {
    let Some(base) = st.config().python_url.clone() else {
        let route = st.forwards().count(req.method(), req.uri().path(), "none");
        tracing::debug!(method = %req.method(), route = %route, "unknown path answered here");
        return standalone(&st, req).await;
    };
    let route = st.forwards().count(req.method(), req.uri().path(), "python");
    tracing::warn!(method = %req.method(), route = %route, "request forwarded to the Python API");
    // the slot is held until the forwarded body has been sent (moved into the stream below)
    let Ok(Ok(permit)) = tokio::time::timeout(PERMIT_WAIT, st.proxy_permits().acquire_owned()).await else {
        return ApiError::unavailable("the API is busy; try again in a moment").into_response();
    };
    let (parts, body) = req.into_parts();
    let pq = parts.uri.path_and_query().map_or("/", |p| p.as_str());
    let url = format!("{base}{pq}");
    let mut headers = HeaderMap::new();
    copy_headers(&parts.headers, &mut headers);
    let upstream = st
        .http()
        .request(parts.method, url)
        .headers(headers)
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await;
    match upstream {
        Ok(r) => {
            let status = r.status();
            let mut headers = HeaderMap::new();
            copy_headers(r.headers(), &mut headers);
            let stream = r.bytes_stream().map(move |chunk| {
                let _held = &permit;
                chunk
            });
            let mut out = Response::new(Body::from_stream(stream));
            *out.status_mut() = status;
            *out.headers_mut() = headers;
            out.extensions_mut().insert(Uncounted);
            out
        }
        Err(e) => {
            tracing::warn!(error = %e, "the Python API did not answer");
            ApiError::new(StatusCode::BAD_GATEWAY, "the API is restarting; try again in a moment").into_response()
        }
    }
}

// ------------------------------------------------------------------------------------------------ with no Python

/// What the Python does for an unknown path (`do_GET` / `do_POST` / `do_PATCH` / `do_PUT` / `do_DELETE`, after
/// `authorize`): the 503 while booting, `?backend=`, a POST's body, then `404 not found`. Nothing is read from the
/// database. Reached with auth already done (the layers of [`standalone_layers`], or the caller's own).
async fn standalone(st: &AppState, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let answer: Result<(), ApiError> = async {
        let q = crate::query::Query::parse(parts.uri.query());
        st.require_loaded()?;
        crate::routes::pick(&q)?;
        // the paths the Python's creative handlers claim, on a method they do not take: their own refusals, in the
        // order the handler makes them (a DELETE never gets there: `do_DELETE` answers 404 at once)
        let special = if parts.method == Method::DELETE {
            None
        } else {
            handler_refusal(&parts.method, parts.uri.path())
        };
        if let Some(Refusal {
            body_first,
            status,
            msg,
        }) = special
        {
            if body_first {
                crate::writes::body_json(&parts.headers, body).await?;
            }
            return Err(ApiError::new(status, msg));
        }
        if parts.method == Method::POST {
            crate::writes::body_json(&parts.headers, body).await?;
        }
        Err(ApiError::not_found("not found"))
    }
    .await;
    let mut r = match answer {
        Err(e) => e.into_response(),
        Ok(()) => ApiError::not_found("not found").into_response(),
    };
    // never in the request metrics: the route label of a made-up path is not ours to keep (the forward counter is)
    r.extensions_mut().insert(Uncounted);
    r
}

/// A refusal the Python's handlers make on a path they claim.
struct Refusal {
    /// The handler reads the body (`body_fn`) before it refuses.
    body_first: bool,
    status: StatusCode,
    msg: &'static str,
}

static ASK_PATH: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"\A/api/ask/chats(?:/([^/]+))?\n?\z").unwrap_or_else(|e| unreachable!("constant regex: {e}"))
});
static CHAT_ID: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"\Achat:[a-f0-9]{8,16}\n?\z").unwrap_or_else(|e| unreachable!("constant regex: {e}"))
});

/// What `profile_api.handle`, `say_api.handle` and `ask_api.handle` answer on a method they do not take (the paths
/// are theirs whatever the method; the methods this service answers never reach here). `HEAD` is a `GET` to the
/// Python (`do_HEAD` calls `do_GET`).
fn handler_refusal(method: &Method, path: &str) -> Option<Refusal> {
    let get = *method == Method::GET || *method == Method::HEAD;
    let refuse = |status: StatusCode, msg: &'static str| Refusal {
        body_first: false,
        status,
        msg,
    };
    let not_allowed = StatusCode::METHOD_NOT_ALLOWED;
    match path {
        "/api/profile/voice/learn" if *method != Method::POST => Some(refuse(not_allowed, "POST")),
        "/api/profile/voice" | "/api/worker/profile/voice" if !get && *method != Method::PUT => {
            Some(refuse(not_allowed, "GET or PUT"))
        }
        // `mysets_api` reads "match" as a set id on a PATCH (its GET route is the match): the set is not there
        "/api/mysets/match" if *method == Method::PATCH => Some(Refusal {
            body_first: true,
            status: StatusCode::NOT_FOUND,
            msg: "no such working set",
        }),
        // a bit's page under an id with a slash in it (`^/api/jokes/(.+)$`, `.../similar`): no such bit
        _ if get && path.strip_prefix("/api/jokes/").is_some_and(|rest| rest.contains('/')) => {
            Some(refuse(StatusCode::NOT_FOUND, "no such joke"))
        }
        // `memos_api.handle` takes every GET under the prefix; the routes it knows are answered before this
        _ if get && path.starts_with("/api/memos") => Some(refuse(StatusCode::NOT_FOUND, "no such memos route")),
        "/api/say" if *method != Method::POST => Some(refuse(not_allowed, "method not allowed")),
        "/api/tonight/screen" if !get => Some(refuse(not_allowed, "method not allowed")),
        _ if path.starts_with("/api/ask/chats") => {
            let Some(m) = ASK_PATH.captures(path) else {
                return Some(refuse(StatusCode::NOT_FOUND, "no such route"));
            };
            let cid = m.get(1).map(|c| {
                percent_encoding::percent_decode_str(c.as_str())
                    .decode_utf8_lossy()
                    .into_owned()
            });
            if cid.as_deref().is_some_and(|c| !CHAT_ID.is_match(c)) {
                return Some(refuse(StatusCode::NOT_FOUND, "no such conversation"));
            }
            // the list takes GET and POST (answered before this); anything else asks for a chat that is not there
            Some(Refusal {
                body_first: matches!(*method, Method::POST | Method::PATCH),
                status: StatusCode::NOT_FOUND,
                msg: "no such conversation",
            })
        }
        _ => None,
    }
}

/// The verbs `BaseHTTPRequestHandler` has a `do_*` for in `server.py`.
const PYTHON_METHODS: [&str; 7] = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];

/// `BaseHTTPRequestHandler.send_error(501, "Unsupported method (%r)")`: the Python's HTML page, no CORS, the
/// connection closed. It comes before `authorize`, as in the Python (it never gets as far as a `do_*`).
pub async fn unsupported_method(req: Request, next: axum::middleware::Next) -> Response {
    if PYTHON_METHODS.contains(&req.method().as_str()) {
        return next.run(req).await;
    }
    let msg = format!("Unsupported method ({})", pk_core::py::quote(req.method().as_str()));
    let msg = msg.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let page = format!(
        "<!DOCTYPE HTML>
<html lang=\"en\">
    <head>
        <meta charset=\"utf-8\">
        <title>Error response</title>
    </head>
    <body>
        <h1>Error response</h1>
        <p>Error code: 501</p>
        <p>Message: {msg}.</p>
        <p>Error code explanation: 501 - Server does not support this operation.</p>
    </body>
</html>
"
    );
    let mut r = (StatusCode::NOT_IMPLEMENTED, page).into_response();
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html;charset=utf-8"),
    );
    h.insert(header::CONNECTION, HeaderValue::from_static("close"));
    r.extensions_mut().insert(Uncounted);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(m: &Method, p: &str) -> Option<(u16, &'static str, bool)> {
        handler_refusal(m, p).map(|r| (r.status.as_u16(), r.msg, r.body_first))
    }

    /// The handlers' refusals on a method they do not take (`profile_api`, `say_api`, `ask_api`).
    /// Red arm: dropping the `HEAD`-is-`GET` rule fails the `/api/say` case.
    #[test]
    fn the_pythons_handlers_refuse_a_method_they_do_not_take() {
        assert_eq!(
            said(&Method::PUT, "/api/profile/voice/learn"),
            Some((405, "POST", false))
        );
        assert_eq!(
            said(&Method::POST, "/api/profile/voice"),
            Some((405, "GET or PUT", false))
        );
        assert_eq!(
            said(&Method::PATCH, "/api/worker/profile/voice"),
            Some((405, "GET or PUT", false))
        );
        assert_eq!(said(&Method::GET, "/api/say"), Some((405, "method not allowed", false)));
        assert_eq!(
            said(&Method::HEAD, "/api/say"),
            Some((405, "method not allowed", false))
        );
        assert_eq!(
            said(&Method::POST, "/api/tonight/screen"),
            Some((405, "method not allowed", false))
        );
        assert_eq!(
            said(&Method::HEAD, "/api/tonight/screen"),
            None,
            "HEAD is a GET to the Python"
        );
        assert_eq!(
            said(&Method::GET, "/api/ask/chats/"),
            Some((404, "no such route", false))
        );
        assert_eq!(
            said(&Method::GET, "/api/ask/chats/a/b"),
            Some((404, "no such route", false))
        );
        assert_eq!(
            said(&Method::GET, "/api/ask/chatsX"),
            Some((404, "no such route", false))
        );
        assert_eq!(
            said(&Method::PUT, "/api/ask/chats/nope"),
            Some((404, "no such conversation", false))
        );
        assert_eq!(
            said(&Method::PATCH, "/api/ask/chats"),
            Some((404, "no such conversation", true))
        );
        assert_eq!(
            said(&Method::PUT, "/api/ask/chats"),
            Some((404, "no such conversation", false))
        );
        assert_eq!(said(&Method::PUT, "/api/labels"), None);
        assert_eq!(
            said(&Method::PATCH, "/api/mysets/match"),
            Some((404, "no such working set", true))
        );
        assert_eq!(said(&Method::GET, "/api/jokes/a/b"), Some((404, "no such joke", false)));
        assert_eq!(
            said(&Method::GET, "/api/jokes/a/b/similar"),
            Some((404, "no such joke", false))
        );
        assert_eq!(said(&Method::GET, "/api/jokes/a"), None);
        assert_eq!(
            said(&Method::GET, "/api/memos/x/y"),
            Some((404, "no such memos route", false))
        );
        assert_eq!(
            said(&Method::POST, "/api/memos/x/y"),
            None,
            "only a GET goes to the memos handler"
        );
        assert_eq!(said(&Method::POST, "/api/jokes/a/b"), None);
    }
}
