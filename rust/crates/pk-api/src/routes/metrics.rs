//! `GET /metrics`: the Prometheus text, open (in-cluster only: the Ingress never forwards it, the network policy
//! admits the observability namespace), answered while booting too so `up` and `pk_build_info` exist at once.

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

/// The handler.
pub async fn metrics(State(st): State<AppState>) -> Response {
    match st.metrics().render() {
        Ok(text) => {
            let mut r = (StatusCode::OK, text).into_response();
            let h = r.headers_mut();
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static(pk_obs::CONTENT_TYPE));
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            r
        }
        Err(e) => crate::error::ApiError::internal("ObsError", e).into_response(),
    }
}
