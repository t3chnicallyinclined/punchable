//! The routes this service answers, one module per Python route group. Every app route starts the way `do_GET`
//! does: the data (or the 503 while booting), then `?backend=` (only `surreal` exists here) and
//! `?include_hidden=`, then the route.

pub mod cloud;
pub mod health;
pub mod jokes;
pub mod kb;
pub mod learned;
pub mod metrics;
pub mod search;
pub mod sets;
pub mod today;

use std::sync::Arc;

use pk_kb::View;

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

/// The data and the index (the view plus the Bits list rows) a request reads.
pub async fn prepare(st: &AppState, q: &Query) -> Result<(Arc<Data>, Arc<AppIndex>), ApiError> {
    let d = st.data().await?;
    pick(q)?;
    let idx = d.index(q.include_hidden()).await?;
    Ok((d, idx))
}

/// [`prepare`] with `work` (a read of the database, not of the generation: the fused search) started right after
/// this request's `meta:state` read and running while the data and the index are made ready. The refusals keep the
/// Python's order: the data (503) first, then `?backend=`, and no work is started for a request that is refused.
pub async fn prepare_and<T: Send + 'static>(
    st: &AppState,
    q: &Query,
    work: impl Future<Output = T> + Send + 'static,
) -> Result<(Arc<Data>, Arc<AppIndex>, tokio::task::JoinHandle<T>), ApiError> {
    if let Err(e) = pick(q) {
        st.data().await?;
        return Err(e);
    }
    let (d, work) = st.data_and(work).await?;
    let idx = d.index(q.include_hidden()).await?;
    Ok((d, idx, work))
}

/// The work [`prepare_and`] started, awaited.
pub async fn finish<T>(work: tokio::task::JoinHandle<T>) -> Result<T, ApiError> {
    work.await.map_err(|e| ApiError::internal("JoinError", e))
}

/// The data and the view only, for a route that reads no Bits list row.
pub async fn prepare_view(st: &AppState, q: &Query) -> Result<(Arc<Data>, Arc<View>), ApiError> {
    let d = st.data().await?;
    pick(q)?;
    let v = d.view(q.include_hidden()).await?;
    Ok((d, v))
}

/// The data alone, for a route that reads only the rows.
pub async fn prepare_rows(st: &AppState, q: &Query) -> Result<Arc<Data>, ApiError> {
    let d = st.data().await?;
    pick(q)?;
    Ok(d)
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
