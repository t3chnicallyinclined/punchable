//! Who is calling: the bearer check every app route goes through (`server.py` `authorize`), the principal it hands a
//! handler, and the rule that says which actor a write is by (`server.py` `actor`, used by the writes at T10).
//!
//! With auth on, every route needs a valid Keycloak access token except `/api/health` and `/api/live` (which read
//! one when it is there). A `worker` token may use only `/api/worker/*`, and only it may. With auth off (a loopback
//! dev server; `config` refuses anything else) every request is the comic.

pub mod actor;
pub mod verifier;

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};

use crate::error::ApiError;
use crate::state::AppState;
pub use verifier::{AuthError, Jwks, Verifier, bearer};

/// The claims of the token a request was made with.
#[derive(Debug, Clone, PartialEq)]
pub struct Principal {
    /// The verified claims.
    pub claims: Map<String, Value>,
}

impl Principal {
    /// The token carries the `worker` realm role.
    pub fn is_worker(&self) -> bool {
        verifier::roles(&self.claims).contains(&verifier::WORKER_ROLE)
    }
}

/// What the auth layer found, in the request's extensions: `None` = auth off, or a route that does not need a token
/// and got none (or a bad one).
#[derive(Debug, Clone, Default)]
pub struct Caller(pub Option<Principal>);

/// Paths that answer without a token.
const OPEN: [&str; 2] = ["/api/health", "/api/live"];

/// The header value as the Python's `http.server` reads it (latin-1, so any byte is a character).
fn header_text(v: &axum::http::HeaderValue) -> String {
    v.as_bytes().iter().map(|b| char::from(*b)).collect()
}

/// The middleware: the five refusals of `server.py` `authorize`, in its order.
pub async fn authorize(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    let Some(v) = st.verifier() else {
        req.extensions_mut().insert(Caller(None));
        return next.run(req).await;
    };
    let path = req.uri().path().to_string();
    let raw = req.headers().get(header::AUTHORIZATION).map(header_text);
    let tok = bearer(raw.as_deref()).map(str::to_string);
    if OPEN.contains(&path.as_str()) {
        let p = match tok {
            Some(t) => v.verify(&t).await.ok().map(|claims| Principal { claims }),
            None => None,
        };
        req.extensions_mut().insert(Caller(p));
        return next.run(req).await;
    }
    let Some(tok) = tok else {
        return ApiError::unauthorized("sign in required").into_response();
    };
    let p = match v.verify(&tok).await {
        Ok(claims) => Principal { claims },
        Err(e) => return ApiError::unauthorized(e.to_string()).into_response(),
    };
    let worker_route = path.starts_with("/api/worker/");
    if p.is_worker() != worker_route {
        let msg = if worker_route {
            "this route is for the home worker only"
        } else {
            "a worker token cannot use the app's routes"
        };
        return ApiError::forbidden(msg).into_response();
    }
    req.extensions_mut().insert(Caller(Some(p)));
    next.run(req).await
}
