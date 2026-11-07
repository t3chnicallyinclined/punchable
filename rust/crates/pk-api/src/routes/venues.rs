//! `GET /api/venues` and `GET /api/venues.geojson` (T10 round 5, group B; `creative_api.Routes.venues`, `geojson`,
//! `_venues`, `venue_overlay`): the rooms he has played with what worked there, and the map's pins. The pins come
//! from the geocoder's cache, which is the `srcfile` row `kb/creative/geocode_cache.json` (the hosted service never
//! sends a query: `PK_GEOCODE` is unset), with the street addresses he supplied (`kb/creative/venue_addresses.json`)
//! tried first; his own pin moves and renames (the `venues` store) go on top.

use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::response::Response;
use pk_creative::Items;
use pk_creative::venues::{Built, Geocoder, build};
use serde_json::{Map, Value};

use crate::creative::GenCorpus;
use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::snapshot::Data;
use crate::state::AppState;

/// The geocoder's cache file.
const GEOCODE_CACHE: &str = "kb/creative/geocode_cache.json";
/// The addresses he supplied.
const ADDRESSES: &str = "kb/creative/venue_addresses.json";

/// One `srcfile` text of this generation, `None` when the generation has no such file.
async fn src_text(st: &AppState, d: &Data, path: &str) -> ApiResult<Option<Arc<str>>> {
    let Some(sha) = d.files.get(path) else {
        return Ok(None);
    };
    let mut got = st
        .src_texts()
        .get(st.store_handle().as_deref(), &[(path.to_string(), sha.clone())])
        .await
        .map_err(|e| ApiError::internal("StoreError", e))?;
    Ok(got.remove(path))
}

/// `_venues()`: the rooms, built from the notebook and the cache (offline), with his edits on top.
pub(crate) async fn built(st: &AppState, d: &Data, c: Arc<GenCorpus>, edits: Arc<Items>) -> ApiResult<Built> {
    let cache = src_text(st, d, GEOCODE_CACHE).await?;
    let addresses = src_text(st, d, ADDRESSES).await?;
    // `str(geo.cache_path)`: where the pod keeps its copy of the file
    let cache_path = st
        .config()
        .data
        .join("comedy")
        .join("kb")
        .join("creative")
        .join("geocode_cache.json")
        .display()
        .to_string();
    tokio::task::spawn_blocking(move || {
        let geo = Geocoder::from_text(cache.as_deref());
        let addresses: Map<String, Value> = addresses
            .as_deref()
            .and_then(|t| serde_json::from_str::<Value>(t).ok())
            .and_then(|v| match v {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .unwrap_or_default();
        build(&c, &geo, &addresses, &cache_path, &pk_core::time::now_iso()).overlay(&edits)
    })
    .await
    .map_err(|e| ApiError::internal("JoinError", e))
}

async fn rooms(st: &AppState, raw: Option<&str>) -> ApiResult<Built> {
    let q = Query::parse(raw);
    let (d, c, items) = super::creative::corpus_and_items(st, &q, |s| &s.venues).await?;
    built(st, &d, c, items).await
}

/// `GET /api/venues`: the rooms, what was not placed, and where the pins came from.
pub async fn venues(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    Ok(ok(&rooms(&st, raw.as_deref()).await?.venues_body()))
}

/// `GET /api/venues.geojson`: the map's features.
pub async fn geojson(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    Ok(ok(&rooms(&st, raw.as_deref()).await?.geojson()))
}
