//! The routes this service answers, one module per Python route group. Every app route starts the way `do_GET`
//! does: the data (or the 503 while booting), then `?backend=` (only `surreal` exists here) and
//! `?include_hidden=`, then the route.

pub mod health;
pub mod kb;
pub mod learned;
pub mod metrics;

use std::sync::Arc;

use crate::error::ApiError;
use crate::index::AppIndex;
use crate::query::Query;
use crate::snapshot::Data;
use crate::state::AppState;

/// The only backend the hosted API has.
pub const BACKEND: &str = "surreal";

/// `server.py` `pick`: a `?backend=` other than this one is a 400 naming what there is.
pub fn pick(q: &Query) -> Result<(), ApiError> {
    match q.get("backend") {
        Some(b) if b != BACKEND => Err(ApiError::bad_request(format!(
            "backend {} is not available (have: {BACKEND})",
            pk_core::py::quote(b)
        ))),
        _ => Ok(()),
    }
}

/// The data and the index a request reads.
pub async fn prepare(st: &AppState, q: &Query) -> Result<(Arc<Data>, Arc<AppIndex>), ApiError> {
    let d = st.data().await?;
    pick(q)?;
    let idx = d.index(q.include_hidden()).await?;
    Ok((d, idx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_backend_is_named_in_the_refusal() {
        assert!(pick(&Query::parse(Some("backend=surreal"))).is_ok());
        assert!(pick(&Query::parse(Some("backend="))).is_ok());
        assert_eq!(
            pick(&Query::parse(Some("backend=files"))).unwrap_err().msg,
            "backend 'files' is not available (have: surreal)"
        );
    }
}
