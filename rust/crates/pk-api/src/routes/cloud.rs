//! `GET /api/map/cloud[?cap=1600]` (`server.py` + `api-dev/cloud_api.py`): every bit placed by meaning. Served from
//! a one-entry cache per data revision and cap for five minutes; a miss reads the vectors (one query) and computes
//! in `spawn_blocking`, holding the cache lock so concurrent misses build once.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{RawQuery, State};
use axum::response::Response;
use pk_store::{from_db, vars};
use serde_json::{Value, json};

use crate::cloud::{self, Bit, CACHE_S, CAP_DEFAULT, CAP_MAX};
use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::state::AppState;

/// bge-small's width; a vector of any other length is left out (the Python's `np.stack` would raise on it).
const DIM: usize = 384;

fn vector(v: &Value) -> Option<Vec<f32>> {
    let a = v.as_array().filter(|a| a.len() == DIM)?;
    a.iter().map(|x| x.as_f64().map(|f| f as f32)).collect()
}

fn mean_unit(vs: &[&Vec<f32>]) -> Vec<f32> {
    let d = vs[0].len();
    let mut m = vec![0.0f32; d];
    for v in vs {
        for (a, b) in m.iter_mut().zip(v.iter()) {
            *a += b;
        }
    }
    for a in &mut m {
        *a /= vs.len() as f32;
    }
    let n = m.iter().map(|a| a * a).sum::<f32>().sqrt();
    let n = if n == 0.0 { 1.0 } else { n };
    m.into_iter().map(|a| a / n).collect()
}

/// The handler.
pub async fn map_cloud(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (d, idx) = super::prepare(&st, &q).await?;
    let cap_s = q.get("cap").map_or_else(|| CAP_DEFAULT.to_string(), str::to_string);
    let cap = cap_s
        .chars()
        .all(|c| c.is_ascii_digit())
        .then(|| cap_s.parse::<i64>().ok())
        .flatten()
        .filter(|c| (1..=CAP_MAX).contains(c))
        .ok_or_else(|| ApiError::bad_request(format!("cap must be 1-{CAP_MAX}")))?;
    let rev = d.rev();
    let hidden = q.include_hidden();
    let mut cache = st.cloud_cache().lock().await;
    if let Some((r, c, h, at, body)) = cache.as_ref()
        && *r == rev
        && *c == cap
        && *h == hidden
        && at.elapsed() < Duration::from_secs(CACHE_S)
    {
        return Ok(ok(body));
    }
    let store = st
        .store_handle()
        .ok_or_else(|| ApiError::unavailable(crate::state::NOT_LOADED))?;
    let res = tokio::time::timeout(
        Duration::from_secs(30),
        store.query(
            "SELECT id, embedding FROM unit WHERE embedding != NONE AND hidden != true;\
             SELECT id, embedding FROM joke WHERE embedding != NONE;",
            vars([]),
        ),
    )
    .await
    .map_err(|_| ApiError::unavailable("the database did not answer in time"))?
    .map_err(|e| ApiError::internal("StoreError", e))?;
    let mut res = res.into_iter().map(from_db);
    let units = res.next().unwrap_or(Value::Null);
    let jokes = res.next().unwrap_or(Value::Null);
    let built = tokio::task::spawn_blocking(move || {
        let rows = |v: &Value| -> Vec<(String, Vec<f32>)> {
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| Some((r.get("id")?.as_str()?.to_string(), vector(r.get("embedding")?)?)))
                .collect()
        };
        let uvec: HashMap<String, Vec<f32>> = rows(&units).into_iter().collect();
        let view = &idx.view;
        let mut vecs: HashMap<&str, Vec<f32>> = HashMap::new();
        for (jid, us) in &view.joke_units {
            let vs: Vec<&Vec<f32>> = us.iter().filter_map(|(u, _)| uvec.get(u)).collect();
            if !vs.is_empty() {
                vecs.insert(jid.as_str(), mean_unit(&vs));
            }
        }
        let jrows = rows(&jokes);
        for (jid, v) in &jrows {
            if let Some((k, _)) = view.nodes.get_key_value(jid.as_str())
                && !vecs.contains_key(k.as_str())
            {
                vecs.insert(k.as_str(), v.clone());
            }
        }
        let mut items: Vec<&pk_wire::api::JokeSummary> =
            idx.summaries.iter().map(|r| &r.summary).filter(|s| vecs.contains_key(s.id.as_str())).collect();
        let key = |s: &pk_wire::api::JokeSummary| {
            let t = s.touched.as_deref().filter(|t| !t.is_empty()).or(s.last_seen.as_deref()).unwrap_or("");
            (t.chars().take(19).collect::<String>(), s.nights)
        };
        // Python's sort(reverse=True) is stable: equal keys keep the index order
        items.sort_by_key(|s| std::cmp::Reverse(key(s)));
        items.truncate(usize::try_from(cap).unwrap_or(usize::MAX));
        let counts_jokes = idx.summaries.len();
        if items.is_empty() {
            return json!({"nodes": [], "clusters": [],
                          "counts": {"jokes": counts_jokes, "placed": 0, "with_vectors": vecs.len()}});
        }
        let bits: Vec<Bit<'_>> = items
            .iter()
            .map(|s| Bit {
                id: &s.id,
                short_name: if s.short_name.is_empty() {
                    if s.short_title.is_empty() { &s.title } else { &s.short_title }
                } else {
                    &s.short_name
                },
                title: &s.title,
                stage: s.stage.as_deref(),
                nights: s.nights,
                heat: s.avg_vs_room,
                touched: s.touched.as_deref(),
            })
            .collect();
        let m: Vec<Vec<f32>> = items.iter().map(|s| vecs[s.id.as_str()].clone()).collect();
        let (nodes, clusters) = cloud::build(&bits, &m);
        json!({"nodes": nodes, "clusters": clusters,
               "counts": {"jokes": counts_jokes, "placed": items.len(), "with_vectors": vecs.len(), "k": clusters.len()}})
    })
    .await
    .map_err(|e| ApiError::internal("JoinError", e))?;
    let mut body = built;
    body["rev"] = json!(rev);
    let body = Arc::new(body);
    *cache = Some((rev, cap, hidden, Instant::now(), Arc::clone(&body)));
    Ok(ok(&body))
}
