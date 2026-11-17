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
    let caller = Caller(Some(p));
    // every owner-only route, whether or not its handler checks too (a new one that forgets is still closed)
    if path.starts_with(ADMIN_PREFIX)
        && let Err(e) = owner_only(&st, &caller)
    {
        return e.into_response();
    }
    req.extensions_mut().insert(caller);
    next.run(req).await
}

/// The owner-only routes' prefix ([`owner_only`] runs on every path under it, in [`authorize`]).
pub const ADMIN_PREFIX: &str = "/api/admin/";

/// The words of the owner gate's refusals (the app maps them to its screen).
pub const NOT_OWNER: &str = "this page is the owner's only";
/// The owner gate's refusal when the server has no owner pinned (`PK_OWNER_SUB` unset): fail closed.
pub const OWNER_NOT_PINNED: &str = "the owner is not pinned on this server (PK_OWNER_SUB)";

/// The owner-only routes (`/api/admin/*`, SPEC 2026-11-17 §13): the token's `sub` must be the pinned `PK_OWNER_SUB`.
/// Stricter than every other app route, which takes any valid non-worker token as him (single tenant): these pages
/// will show every account's usage once testers exist, so they are pinned now. With auth off (a loopback dev server,
/// `config` refuses anything else) there is no token and the caller is the comic, as on every route. A worker token
/// never gets here (`authorize` keeps it to `/api/worker/*`), and is refused here too.
pub fn owner_only(st: &AppState, caller: &Caller) -> Result<(), ApiError> {
    if st.verifier().is_none() {
        return Ok(());
    }
    let Some(p) = &caller.0 else {
        return Err(ApiError::unauthorized("sign in required"));
    };
    if p.is_worker() {
        return Err(ApiError::forbidden("a worker token cannot use the app's routes"));
    }
    let Some(owner) = st.config().owner_sub.as_deref() else {
        return Err(ApiError::forbidden(OWNER_NOT_PINNED));
    };
    match p.claims.get("sub").and_then(Value::as_str) {
        Some(sub) if sub == owner => Ok(()),
        _ => Err(ApiError::forbidden(NOT_OWNER)),
    }
}
