//! The reads that were still the Python's (T10 round 6, group D): `GET /api/map` (every bit and the links between them),
//! `GET /api/connections` (Settings > Connections: one plain status per outside source), `GET /api/ai/worker` (the
//! worker's line and nothing else) and `GET /api/backend` (what answers: the app reads `primary` at start).
//!
//! `/api/connections` reads gsync's own files, which are `srcfile` rows here (`kb/gsync/map.json`, `runs.jsonl`,
//! `conflicts.jsonl`): counts only, no titles, no text, no doc ids, nothing about the legal exclusions.

use axum::extract::{RawQuery, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse as _;
use axum::response::Response;
use pk_creative::connections::all_map;
use pk_store::{from_db, vars};
use serde_json::{Map, Value, json};

use super::creative::items_of;
use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::state::AppState;
use crate::tape::srcfile_key;

/// `MAP_CAP_MAX`.
const MAP_CAP_MAX: usize = 2000;
/// `GSYNC_STALE_S`: no run in a day is "not syncing".
const GSYNC_STALE_S: f64 = 24.0 * 3600.0;
/// `GSYNC_JOKE_ORIGINS`.
const JOKE_ORIGINS: [&str; 3] = ["gdocs", "app", "mixed"];
/// `_jsonl_tail`'s window.
const TAIL_BYTES: usize = 256 * 1024;

/// `GET /api/map?cap=400`: every bit as a node (the top `cap` by nights, laughs) and the links between them. The
/// same-idea groups are the merge judge's (`versions` links); no judge, no such links, and the map still draws.
pub async fn map_all(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let d = super::prepare_rows(&st, &q).await?;
    let cap = q.get("cap").unwrap_or("400");
    let bad = || ApiError::bad_request(format!("cap must be 1-{MAP_CAP_MAX}"));
    if cap.is_empty() || !cap.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let cap: usize = cap.parse().map_err(|_| bad())?;
    if !(1..=MAP_CAP_MAX).contains(&cap) {
        return Err(bad());
    }
    let c = d.corpus(&st).await?;
    let cr = crate::writes::Creative::new(st.clone());
    let groups: Vec<Value> = match cr.lines_and_items("merges").await {
        Ok((lines, items)) => match d.merge_groups(&st, lines.len(), &items).await {
            Ok(g) => g.list().to_vec(),
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    let rev = d.rev();
    let mut out = tokio::task::spawn_blocking(move || all_map(&c, cap, &groups))
        .await
        .map_err(|e| ApiError::internal("JoinError", e))?;
    out["rev"] = json!(rev);
    Ok(ok(&out))
}

/// `GET /api/ai/worker`: `worker_status(cr)` and nothing else.
pub async fn ai_worker(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    super::pick(&q)?;
    Ok(ok(&super::creative::worker_status(&st).await?))
}

/// The text of one `srcfile` row, or none.
async fn file_text(st: &AppState, path: &str) -> ApiResult<Option<String>> {
    let store = st
        .store_handle()
        .ok_or_else(|| ApiError::internal("ConnectionError", "the database is not connected"))?;
    let rows = store
        .last(
            "SELECT VALUE text FROM type::record('srcfile', $k);",
            vars([("k", pk_store::to_db(&json!(srcfile_key(path))))]),
        )
        .await
        .map_err(|e| ApiError::internal("StoreError", e))?;
    Ok(pk_store::store::rows_of(rows)
        .into_iter()
        .map(from_db)
        .find_map(|v| v.as_str().map(str::to_string)))
}

/// `_age_s(ts)`: seconds since an ISO time (a missing zone is UTC), infinite when it is not one.
fn age_s(ts: Option<&str>) -> f64 {
    let Some(ts) = ts.filter(|t| !t.is_empty()) else {
        return f64::INFINITY;
    };
    let t = ts.replace('Z', "+00:00");
    let parsed = chrono::DateTime::parse_from_rfc3339(&t)
        .map(|d| d.timestamp_millis() as f64 / 1000.0)
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(&t, "%Y-%m-%dT%H:%M:%S%.f")
                .map(|d| d.and_utc().timestamp_millis() as f64 / 1000.0)
        });
    match parsed {
        Ok(at) => crate::worker::unix_now() - at,
        Err(_) => f64::INFINITY,
    }
}

/// `_jsonl_tail(p)`: the last runs only (`runs.jsonl` grows by a line a minute): objects, a torn line skipped.
fn jsonl_tail(text: &str) -> Vec<Map<String, Value>> {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let raw = String::from_utf8_lossy(&bytes[n.saturating_sub(TAIL_BYTES)..]).into_owned();
    let lines: Vec<&str> = pk_core::py::splitlines(&raw);
    let lines = if n > TAIL_BYTES {
        &lines[1.min(lines.len())..]
    } else {
        &lines[..]
    };
    lines
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| v.as_object().cloned())
        .collect()
}

/// `gsync_status(d, hosted)`: Settings > Connections > Google Docs, from gsync's own files.
pub fn gsync_status(map: Option<&str>, runs: Option<&str>, conflicts: Option<&str>, hosted: bool) -> Value {
    let mut out = json!({"status": "not_set_up", "last_sync": null, "last_ok": null, "docs_linked": 0, "joke_docs": 0,
                         "boards": 0, "conflicts": 0, "errors_last_run": 0, "last_mode": null, "where": "your PC"});
    if map.is_none() && runs.is_none() {
        out["status"] = json!(if hosted { "not_reported" } else { "not_set_up" });
        return out;
    }
    let m: Value = map.and_then(|t| serde_json::from_str(t).ok()).unwrap_or(Value::Null);
    let m = if m.is_object() { m } else { json!({}) };
    let docs: Vec<&Map<String, Value>> = m
        .get("docs")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(Map::values)
        .filter_map(Value::as_object)
        .filter(|e| !pk_core::py::truthy(e.get("hidden")))
        .collect();
    out["docs_linked"] = json!(docs.len());
    out["joke_docs"] = json!(
        docs.iter()
            .filter(|e| e
                .get("origin")
                .and_then(Value::as_str)
                .is_some_and(|o| JOKE_ORIGINS.contains(&o)))
            .count()
    );
    out["boards"] = json!(m.get("boards").and_then(Value::as_object).map_or(0, Map::len));
    let all_runs = runs.map(jsonl_tail).unwrap_or_default();
    let runs: Vec<&Map<String, Value>> = all_runs
        .iter()
        .filter(|r| !pk_core::py::truthy(r.get("dry_run")))
        .collect();
    if let Some(c) = conflicts {
        out["conflicts"] = json!(c.lines().filter(|l| !l.trim().is_empty()).count());
    }
    let Some(last) = runs.last() else {
        out["status"] = json!("not_synced_yet");
        return out;
    };
    let when = |r: &Map<String, Value>| {
        ["ts_end", "ts"]
            .iter()
            .filter_map(|k| r.get(*k).filter(|v| pk_core::py::truthy(Some(v))))
            .find_map(|v| v.as_str().map(str::to_string))
    };
    let okruns: Vec<&&Map<String, Value>> = runs
        .iter()
        .filter(|r| !pk_core::py::truthy(r.get("errors")) && r.get("mode").and_then(Value::as_str) != Some("api_down"))
        .collect();
    let last_sync = when(last);
    out["last_sync"] = json!(last_sync);
    out["last_ok"] = json!(okruns.last().and_then(|r| when(r)));
    out["last_mode"] = last.get("mode").cloned().unwrap_or(Value::Null);
    let errs = last.get("errors").and_then(Value::as_array).map_or(0, Vec::len);
    out["errors_last_run"] = json!(errs);
    let age = age_s(last_sync.as_deref());
    out["status"] = json!(if last.get("mode").and_then(Value::as_str) == Some("api_down") {
        "api_down"
    } else if errs > 0 {
        "error"
    } else if age > GSYNC_STALE_S {
        "stale"
    } else {
        "connected"
    });
    out
}

/// `GET /api/connections`: Settings > Connections, one plain status per outside source. Read-only.
pub async fn connections(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let d = super::prepare_rows(&st, &q).await?;
    let view = d.view(false).await?;
    let youtube = view
        .nodes
        .iter()
        .filter(|(k, n)| {
            n.get("type").and_then(Value::as_str) == Some("set") && {
                let b = k.as_bytes();
                b.len() >= 12
                    && b[b.len() - 12] == b'_'
                    && b[b.len() - 11..]
                        .iter()
                        .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
            }
        })
        .count();
    let (map, runs, conflicts) = (
        file_text(&st, "kb/gsync/map.json").await?,
        file_text(&st, "kb/gsync/runs.jsonl").await?,
        file_text(&st, "kb/gsync/conflicts.jsonl").await?,
    );
    let hosted = st.config().auth.is_some();
    let jobs = items_of(&st, |s| &s.jobs).await?;
    let waiting = jobs
        .values()
        .filter_map(Value::as_object)
        .filter(|j| {
            j.get("status").and_then(Value::as_str) == Some("queued")
                && j.get("waiting_for").and_then(Value::as_str) == Some("transcriber")
        })
        .count();
    let settings = crate::setlists::settings(&*items_of(&st, |s| &s.account).await?);
    let worker = super::creative::worker_status(&st).await?;
    Ok(ok(&json!({
        "google_docs": gsync_status(map.as_deref(), runs.as_deref(), conflicts.as_deref(), hosted),
        "youtube": {"sets": youtube, "import": "link"},
        "ai": {"provider": settings["provider"], "worker": worker},
        "transcriber": {"where": "your phone", "waiting": waiting,
                        "note": if waiting > 0 { crate::writes::jobs::waiting_note("transcriber") } else { Value::Null }},
        "audio_storage": {"status": "not_set_up", "provider": "Cloudflare R2"},
    })))
}

/// `GET /api/backend`: what answers. The Python reports its own processes (the vector sweeper's runs, the live
/// socket's clients, the file syncs); here the one database and what was loaded into it, in the same shape. The app
/// reads `primary` at start.
pub async fn backend(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let d = super::prepare_rows(&st, &q).await?;
    let info = &d.load_info;
    let get = |k: &str| info.get(k).cloned().unwrap_or(Value::Null);
    Ok(ok(&json!({
        "primary": "surreal", "this_request": "surreal",
        "backends": {"surreal": {
            "backend": "surreal", "gen": d.state.generation, "loading": d.state.loading,
            "graph": get("graph"), "graph_version": get("graph_version"), "loaded_at": get("loaded_at"),
            "counts": get("counts"), "id_map": get("id_map_log"),
            "load_mode": get("mode"), "last_sync_changed": get("sync_changed"),
            "snapshot_fetch_s": d.fetch_s,
        }},
    })))
}

/// `GET /api/export`: everything that is his as one JSON download (decisions, sets, ideas, the inbox, the creative
/// logs and settings). Audio is not in it (it is on his phone). Read from the database: the same file the Python made
/// from its pod's copies. With the uploads still the Python pod's folder, the inbox is its, so the request is too.
pub async fn export(State(st): State<AppState>, req: axum::extract::Request) -> ApiResult<Response> {
    if st.config().uploads.is_none() {
        return Ok(crate::proxy::forward(State(st), req).await);
    }
    let q = Query::parse(req.uri().query());
    let d = super::prepare_rows(&st, &q).await?;
    let view = d.view(false).await?;
    let sets: Value = serde_json::from_slice(&d.sets_body(false).await?)
        .ok()
        .and_then(|b: Value| b.get("sets").cloned())
        .unwrap_or_else(|| json!([]));
    let ideas: Vec<Value> = view
        .jokes
        .iter()
        .filter_map(|j| view.nodes.get(j))
        .filter(|j| pk_core::py::truthy(j.get("_by_tristan")))
        .map(|j| {
            let get = |k: &str| j.get(k).cloned().unwrap_or(Value::Null);
            json!({"id": get("id"), "title": get("title"), "stage": get("stage"), "summary": get("summary"),
                   "from": get("from"), "first_seen": get("first_seen")})
        })
        .collect();
    let store = st
        .store_handle()
        .ok_or_else(|| ApiError::internal("ConnectionError", "the database is not connected"))?;
    let inbox: Vec<Value> = store
        .upload_rows()
        .await
        .map_err(|e| ApiError::internal("StoreError", e))?
        .iter()
        .map(crate::writes::uploads::inbox_row)
        .collect();
    let cr = crate::writes::Creative::new(st.clone());
    let mut creative = Map::new();
    for n in ["storyboards", "workingsets", "venues", "account"] {
        let (lines, items) = cr.lines_and_items(n).await?;
        let events: Vec<Value> = lines.iter().filter_map(|l| serde_json::from_str(l).ok()).collect();
        creative.insert(n.to_string(), json!({"items": &*items, "events": events}));
    }
    let out = json!({"format": "punchable-export/1", "exported_at": pk_core::time::now_iso(),
                     "decisions": view.decisions.iter().map(|x| Value::Object(x.0.clone())).collect::<Vec<_>>(),
                     "sets": sets, "ideas": ideas, "inbox": inbox, "creative": creative});
    // `json.dumps(out, ensure_ascii=False, indent=1)`
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    serde::Serialize::serialize(&out, &mut serde_json::Serializer::with_formatter(&mut buf, fmt))
        .map_err(|e| ApiError::internal("TypeError", e))?;
    let mut r = (StatusCode::OK, buf).into_response();
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    let name = format!(
        "attachment; filename=\"punchable-export-{}.json\"",
        chrono::Local::now().format("%Y%m%d")
    );
    if let Ok(v) = HeaderValue::from_str(&name) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(r)
}

/// `GET /api/audio/<set>`: a recording. There is none on the hosted pod (`AppKB.audio_path` looks in `tapes/`, which the
/// pod does not have: the audio is on his phone), so the answer is the Python's "no audio" for a good id and its 400 for
/// a bad one.
pub async fn audio(
    State(st): State<AppState>,
    axum::extract::Path(set_id): axum::extract::Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    super::pick(&q)?;
    if !crate::writes::is_id(&set_id) {
        return Err(ApiError::bad_request("bad set id"));
    }
    Err(ApiError::not_found("no audio for that set"))
}
