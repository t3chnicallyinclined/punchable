//! T9 only (deleted at T10): every request this service does not answer itself goes to the Python API unchanged,
//! streamed both ways, so an Ingress path moved here can never swallow a write (an Ingress prefix routes every
//! method: docs/ARCHITECTURE.md section 2). The Python authenticates, counts and answers those requests itself; this
//! layer adds nothing to them and counts nothing.

use std::time::Duration;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::StreamExt as _;

use crate::error::ApiError;
use crate::state::AppState;

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

/// The fallback: forward to `PK_PYTHON_URL`, or answer 404 the Python's way when none is configured.
pub async fn forward(State(st): State<AppState>, req: Request) -> Response {
    let Some(base) = st.config().python_url.clone() else {
        let mut r = ApiError::not_found("not found").into_response();
        r.extensions_mut().insert(Uncounted);
        return r;
    };
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
