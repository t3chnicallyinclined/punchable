//! The ingest routes of the home worker and gsync (`api-dev/ingest_api.py`), hosted: every import lands in the
//! database, nothing stays local.
//!
//! ```text
//! POST /api/worker/ideas            {text, title?, from?, reason?, actor}      a new idea (+ auto-attach), by actor
//! POST /api/worker/decision         {target, action, id?, after?, reason?, actor}
//! POST /api/worker/undo             {indexes, actor}
//! GET  /api/worker/jokes/<id>                                                 the joke page (what gsync reads)
//! GET  /api/worker/files[?prefix=]                                            [{path, sha256, bytes}], files_rev
//! GET  /api/worker/files/content?path=                                        {path, sha256, text}
//! POST /api/worker/files            {files: [{path, text}], remove?: [path], actor}
//! POST /api/worker/events           {store: "merges", op: attach|detach, id, data, actor: ai:*}
//! GET  /api/worker/state                                                      what the worker checks first
//! ```
//!
//! Every write names its actor (`import:gsync`, `import:memo`, `import:youtube`, `import:upload`, or `ai:<what>`);
//! the comic's own actions never come through here. Court and legal material is refused (422) and nothing of it is
//! stored or echoed. A file is a `srcfile` row (the database is the truth) and `meta:state.files_rev` moves; if the
//! file is a graph input (a set's analysis, meta, laughs, transcript, `inferred.json`, ...), the graph tables are
//! re-synced at once ([`pk_store::Store::sync_graph`], `db/load.py` `sync`: writes only what changed). There is no
//! cache file on this service: the hosted Python's pod copy of the rows has no counterpart here.

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use axum::Extension;
use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::HeaderMap;
use axum::response::Response;
use indexmap::IndexMap;
use pk_core::py;
use pk_kb::overlay::{Doc, OverlayFiles, set_id_of};
use pk_store::kbmap::SrcFile;
use pk_store::{Store, SyncInput, Vars, from_db, to_db, vars};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::srcpaths::{MAX_FILE_BYTES, clean, is_set_input, worker_may_write};
use super::{Creative, Kb, Pre, ProbeError, Written, actor, body_json, detached, http, request_within, store_err};
use crate::auth::Caller;
use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::state::AppState;
use crate::tape::srcfile_key;

/// `MAX_FILES_BODY`: the largest body `POST /api/worker/files` takes.
pub const MAX_FILES_BODY: usize = 24 * 1024 * 1024;

/// The writes to `srcfile` (`_lock`): one file post at a time in this process.
static FILES_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// `^at:[A-Za-z0-9]{6,40}$` under `re.match`.
static EVENT_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\Aat:[A-Za-z0-9]{6,40}\n?\z").unwrap_or_else(|e| unreachable!("constant regex: {e}"))
});

fn store_of(st: &AppState) -> ApiResult<std::sync::Arc<Store>> {
    st.store_handle().ok_or_else(super::not_connected)
}

/// `import_actor(h, b)`: the actor of a worker write: required, and never the comic's own id.
fn import_actor(st: &AppState, caller: &Caller, b: &Map<String, Value>) -> ApiResult<String> {
    let a = actor(st, caller, b)?;
    if b.get("actor").and_then(Value::as_str) != Some(a.as_str()) {
        return Err(ApiError::bad_request(
            "a worker write must name its actor: import:<source> or ai:<what>",
        ));
    }
    Ok(a)
}

fn is_legal(st: &AppState, v: Option<&Value>) -> bool {
    v.and_then(Value::as_str).is_some_and(|t| st.legal().is_legal(t))
}

fn refused(msg: &str) -> ApiError {
    http(422, msg)
}

/// `files_rev(kb)`: `meta:state.files_rev`, or null.
async fn files_rev(store: &Store) -> ApiResult<Value> {
    let v = store
        .last("SELECT files_rev FROM ONLY meta:state;", Vars::default())
        .await
        .map_err(store_err)?;
    Ok(match from_db(v) {
        Value::Object(o) => o.get("files_rev").cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    })
}

/// `online_first(kb)`: `meta:state.online_first`, set at the cutover.
async fn online_first(store: &Store) -> ApiResult<bool> {
    let v = store
        .last("SELECT online_first FROM ONLY meta:state;", Vars::default())
        .await
        .map_err(store_err)?;
    Ok(match from_db(v) {
        Value::Object(o) => py::truthy(o.get("online_first")),
        _ => false,
    })
}

// ------------------------------------------------------------------------------------------------------ the reads

/// `GET /api/worker/files[?prefix=]`.
pub async fn list(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let store = store_of(&st)?;
    let prefix = q.get("prefix").unwrap_or("").to_string();
    let rows = store
        .last(
            "SELECT path, sha256, bytes FROM srcfile WHERE string::starts_with(path, $p) ORDER BY path;",
            vars([("p", to_db(&json!(prefix)))]),
        )
        .await
        .map_err(store_err)?;
    let files: Vec<Value> = pk_store::store::rows_of(rows)
        .into_iter()
        .map(from_db)
        .map(|r| json!({"path": r["path"], "sha256": r["sha256"], "bytes": r["bytes"]}))
        .collect();
    Ok(ok(
        &json!({"files": files, "files_rev": files_rev(&store).await?, "mode": "db"}),
    ))
}

/// `GET /api/worker/files/content?path=`.
pub async fn content(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let Some(rel) = clean(q.get("path").unwrap_or("")) else {
        return Err(ApiError::bad_request("bad path"));
    };
    let store = store_of(&st)?;
    let rows = store
        .last(
            "SELECT path, sha256, text FROM srcfile WHERE path = $p LIMIT 1;",
            vars([("p", to_db(&json!(rel)))]),
        )
        .await
        .map_err(store_err)?;
    match pk_store::store::rows_of(rows).into_iter().map(from_db).next() {
        Some(r) => Ok(ok(&r)),
        None => Err(ApiError::not_found("no such file")),
    }
}

/// `GET /api/worker/state`: what the worker checks first.
pub async fn state(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let store = store_of(&st)?;
    let fp = hex::encode(Sha256::digest(st.legal().canonical().as_bytes()));
    Ok(ok(&json!({
        "decisions": store.decision_count().await.map_err(store_err)?,
        "files_rev": files_rev(&store).await?,
        "files_mode": "db",
        "backend": "surreal",
        "online_first": online_first(&store).await?,
        "legal_rules": &fp[..12],
    })))
}

// ------------------------------------------------------------------------------------------------------ the files

/// `POST /api/worker/files {files: [{path, text, sha256?}], remove?: [path], actor}`.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "the Python's `endswith('.json')` is case sensitive"
)]
pub async fn put(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let r = request_within(st, raw.as_deref(), &headers, body, MAX_FILES_BODY).await?;
    let (st, b) = (r.st, r.b);
    let by = import_actor(&st, &caller, &b)?;
    let (files, remove) = (as_list(b.get("files")), as_list(b.get("remove")));
    let (Some(files), Some(remove)) = (files, remove) else {
        return Err(ApiError::bad_request(
            "files and remove must be lists (up to 500 entries)",
        ));
    };
    if files.len() + remove.len() > 500 {
        return Err(ApiError::bad_request(
            "files and remove must be lists (up to 500 entries)",
        ));
    }
    let mut clean_files: Vec<(String, String)> = Vec::new();
    for f in files {
        let (Some(fo), Some(text)) = (f.as_object(), f.get("text").and_then(Value::as_str)) else {
            return Err(ApiError::bad_request("each file needs {path, text}"));
        };
        let shown =
            |v: Option<&Value>| py::quote(&py::str(v.unwrap_or(&Value::Null)).chars().take(200).collect::<String>());
        let rel = fo.get("path").and_then(Value::as_str).and_then(clean);
        let Some(rel) = rel.filter(|r| worker_may_write(r)) else {
            return Err(http(
                403,
                format!("not a file the worker may write: {}", shown(fo.get("path"))),
            ));
        };
        if text.len() > MAX_FILE_BYTES {
            return Err(http(413, format!("{rel}: over {} MB", MAX_FILE_BYTES / 1_000_000)));
        }
        if rel.ends_with(".json") && serde_json::from_str::<Value>(text).is_err() {
            return Err(ApiError::bad_request(format!("{rel}: not JSON")));
        }
        if py::truthy(fo.get("sha256")) && fo.get("sha256").and_then(Value::as_str) != Some(&pk_store::line_hash(text))
        {
            return Err(ApiError::bad_request(format!("{rel}: sha256 does not match the text")));
        }
        if st.legal().is_legal(text) {
            return Err(refused(&format!(
                "{rel}: refused by the legal rules (court material is never stored); nothing was kept"
            )));
        }
        clean_files.push((rel, text.to_string()));
    }
    let mut gone: Vec<String> = Vec::new();
    for x in remove {
        let rel = x.as_str().and_then(clean);
        let Some(rel) = rel.filter(|r| worker_may_write(r)) else {
            return Err(http(
                403,
                format!(
                    "not a file the worker may remove: {}",
                    py::quote(&py::str(x).chars().take(200).collect::<String>())
                ),
            ));
        };
        gone.push(rel);
    }
    let out = detached(async move { put_files(&st, &by, clean_files, gone).await }).await?;
    Ok(ok(&out))
}

/// `body.get(k) or []` as a list: `None` when it is something else.
fn as_list(v: Option<&Value>) -> Option<&[Value]> {
    match v {
        Some(Value::Array(a)) => Some(a.as_slice()),
        Some(x) if !py::truthy(Some(x)) => Some(&[]),
        None => Some(&[]),
        _ => None,
    }
}

/// The write half of `put_files`: the rows, the revision, the sync.
async fn put_files(st: &AppState, by: &str, files: Vec<(String, String)>, gone: Vec<String>) -> ApiResult<Value> {
    let store = store_of(st)?;
    let (mut written, mut removed, mut unchanged) = (Vec::<String>::new(), Vec::<String>::new(), 0u64);
    {
        let _lock = FILES_LOCK.lock().await;
        let mut have: HashMap<String, Option<String>> = HashMap::new();
        for rel in files.iter().map(|f| &f.0).chain(&gone) {
            let rows = store
                .last(
                    "SELECT VALUE sha256 FROM srcfile WHERE path = $p LIMIT 1;",
                    vars([("p", to_db(&json!(rel)))]),
                )
                .await
                .map_err(store_err)?;
            let sha = pk_store::store::rows_of(rows)
                .into_iter()
                .map(from_db)
                .next()
                .and_then(|v| v.as_str().map(str::to_string));
            have.insert(rel.clone(), sha);
        }
        for (rel, text) in &files {
            let row = SrcFile::new(rel, text);
            if have.get(rel).and_then(Option::as_deref) == Some(row.sha256.as_str()) {
                unchanged += 1;
                continue;
            }
            store
                .write(
                    "UPSERT type::record('srcfile', $k) CONTENT $c RETURN id;",
                    vars([("k", to_db(&json!(srcfile_key(rel)))), ("c", row.content())]),
                    Some(1),
                )
                .await
                .map_err(store_err)?;
            written.push(rel.clone());
        }
        for rel in &gone {
            if have.get(rel).and_then(Option::as_ref).is_none_or(String::is_empty) {
                continue;
            }
            store
                .query(
                    "DELETE srcfile WHERE path = $p RETURN NONE;",
                    vars([("p", to_db(&json!(rel)))]),
                )
                .await
                .map_err(store_err)?;
            removed.push(rel.clone());
        }
        if !written.is_empty() || !removed.is_empty() {
            store
                .write(
                    "UPDATE meta:state SET files_rev = (files_rev ?? 0) + 1 RETURN id;",
                    Vars::default(),
                    None,
                )
                .await
                .map_err(store_err)?;
        }
    }
    let synced = if written.iter().chain(&removed).any(|r| is_set_input(r)) {
        Some(sync_now(st).await?)
    } else {
        None
    };
    Ok(
        json!({"ok": true, "written": written, "removed": removed, "unchanged": unchanged,
              "synced": synced, "files_rev": files_rev(&store).await?, "actor": by}),
    )
}

// ------------------------------------------------------------------------------------------------------ the sync

/// `sync_now(why)`: re-sync the graph tables from the database's own `srcfile` rows now (after the home worker posted
/// pipeline output). The inputs are the bundle's graph and id map, `kb/v0/inferred.json`, every set's
/// `analysis.json`, `meta.json` and `laughs_auto.json`.
pub async fn sync_now(st: &AppState) -> ApiResult<Value> {
    let store = store_of(st)?;
    let ms = store.meta_state().await.map_err(store_err)?;
    let li = ms.and_then(|m| m.load_info).unwrap_or(Value::Null);
    let bundle = li.get("bundle").cloned().unwrap_or(Value::Null);
    let Some(graph_path) = bundle.get("graph").and_then(Value::as_str).map(str::to_string) else {
        return Err(http(
            503,
            "the graph tables need a full load first (never loaded): run dbload",
        ));
    };
    let rows = store
        .last(
            "SELECT path, sha256 FROM srcfile WHERE path = $graph OR path = $idmap OR string::starts_with(path, 'corpus/sets/') \
             OR path = 'kb/v0/inferred.json';",
            vars([
                ("graph", to_db(&json!(graph_path))),
                ("idmap", to_db(&bundle.get("id_map").cloned().filter(Value::is_string).unwrap_or_else(|| json!("")))),
            ]),
        )
        .await
        .map_err(store_err)?;
    let shas: BTreeMap<String, String> = pk_store::store::rows_of(rows)
        .into_iter()
        .map(from_db)
        .filter_map(|r| {
            Some((
                r.get("path")?.as_str()?.to_string(),
                r.get("sha256")?.as_str()?.to_string(),
            ))
        })
        .collect();
    let id_map_path = bundle
        .get("id_map")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut want: Vec<(String, String)> = Vec::new();
    for (p, h) in &shas {
        let take = *p == graph_path
            || Some(p) == id_map_path.as_ref()
            || p == "kb/v0/inferred.json"
            || p.strip_prefix("corpus/sets/").is_some_and(|r| {
                r.split_once('/')
                    .is_some_and(|(_, f)| matches!(f, "analysis.json" | "meta.json" | "laughs_auto.json"))
            });
        if take {
            want.push((p.clone(), h.clone()));
        }
    }
    // the texts are parsed once and dropped: the laughs are megabytes and are read on every sync
    let texts = crate::creative::SrcTexts::read_uncached(Some(&store), &want)
        .await
        .map_err(store_err)?;
    let sources: Vec<(String, usize)> = texts.iter().map(|(p, _, t)| (p.clone(), t.len())).collect();
    let by_path: BTreeMap<&str, &str> = texts.iter().map(|(p, _, t)| (p.as_str(), &**t)).collect();
    // `AIFiles._load`: a document that is not a JSON object is as good as absent
    let parsed = |p: &str| -> Option<Value> {
        by_path
            .get(p)
            .and_then(|t| serde_json::from_str::<Value>(t).ok())
            .filter(Value::is_object)
    };
    let graph: Value = by_path
        .get(graph_path.as_str())
        .and_then(|t| serde_json::from_str(t).ok())
        .ok_or_else(|| {
            http(
                503,
                "the graph tables need a full load first (graph.json unreadable): run dbload",
            )
        })?;
    let mut files = OverlayFiles {
        id_map: id_map_path.as_deref().and_then(parsed),
        inferred: parsed("kb/v0/inferred.json").map(|doc| Doc {
            doc,
            source: "real".into(),
            path: "kb/v0/inferred.json".into(),
        }),
        analyses: Vec::new(),
        set_meta: IndexMap::new(),
    };
    let mut laughs: Vec<(String, Value)> = Vec::new();
    let dirs: std::collections::BTreeSet<&str> = by_path
        .keys()
        .filter_map(|p| p.strip_prefix("corpus/sets/")?.split_once('/').map(|(d, _)| d))
        .collect();
    for d in dirs {
        if set_id_of(d).is_none() {
            continue;
        }
        let at = |f: &str| format!("corpus/sets/{d}/{f}");
        if let Some(doc) = parsed(&at("analysis.json")).filter(|v| v.get("jokes").is_some_and(Value::is_array)) {
            files.analyses.push((
                d.to_string(),
                Doc {
                    doc,
                    source: "real".into(),
                    path: at("analysis.json"),
                },
            ));
        }
        if let Some(meta) = parsed(&at("meta.json")) {
            files.set_meta.insert(d.to_string(), meta);
        }
        if let Some(l) = parsed(&at("laughs_auto.json")) {
            laughs.push((d.to_string(), l));
        }
    }
    let mut paths: BTreeMap<&'static str, Value> = BTreeMap::new();
    paths.insert("graph", json!(graph_path));
    paths.insert("kb", json!("kb/v0"));
    paths.insert("decisions", json!("kb/v0/decisions.jsonl"));
    paths.insert("corpus", json!("corpus"));
    paths.insert("id_map", id_map_path.as_ref().map_or(Value::Null, |p| json!(p)));
    let input = SyncInput {
        graph: &graph,
        files: &files,
        laughs: &laughs,
        sources,
        paths,
        owner: &st.config().owner_id,
    };
    let _kb = st.kb_lock().lock().await;
    // the hosted Python's `load_vectors` finds no vector cache folder in its pod, so a unit that changed is written
    // without a vector and the sweeper embeds it: the same here
    let out = store
        .sync_graph(&input, async || Ok(HashMap::new()))
        .await
        .map_err(store_err)?;
    Ok(json!({"mode": out.mode, "changed": out.changed, "s": (out.seconds * 100.0).round() / 100.0}))
}

// ------------------------------------------------------------------------------------------------------ the writes

/// `POST /api/worker/ideas`: an idea from an import, then its follow-up jobs (`embed`, and `memolink` for a memo or an
/// upload).
pub async fn ideas(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let b = body_json(&headers, body).await?;
    if is_legal(&st, b.get("text")) || is_legal(&st, b.get("title")) {
        return Err(refused(
            "refused by the legal rules (court material is never stored); nothing was kept",
        ));
    }
    let by = import_actor(&st, &caller, &b)?;
    let hidden = q.include_hidden();
    let out = detached(async move {
        let out = super::ideas::idea_result(&st, hidden, &b, &by).await?;
        enqueue_followups(&st, &by).await;
        Ok(out)
    })
    .await?;
    Ok(ok(&out))
}

/// `enqueue_followups`: vectors for an imported idea (job `embed`, cheap), then the memo <-> bit linking (job
/// `memolink`: judge, triage, clear attaches). One pending job each. The idea is saved either way.
async fn enqueue_followups(st: &AppState, actor: &str) {
    if !actor.starts_with("import:") {
        return;
    }
    let _cr = st.creative_lock().lock().await;
    let cr = Creative::new(st.clone());
    let mut kinds = vec!["embed"];
    if matches!(actor, "import:memo" | "import:upload") {
        kinds.push("memolink");
    }
    for kind in kinds {
        if let Err(e) = super::jobs::enqueue_followup(&cr, kind, actor).await {
            tracing::warn!("followup jobs not queued: {}", e.msg);
            return;
        }
    }
}

/// `POST /api/worker/decision`.
pub async fn decision(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let b = body_json(&headers, body).await?;
    if st.legal().is_legal(
        &serde_json::to_string(b.get("after").filter(|v| py::truthy(Some(v))).unwrap_or(&json!({})))
            .unwrap_or_default(),
    ) {
        return Err(refused("refused by the legal rules; nothing was kept"));
    }
    if let Some(id) = b.get("id").filter(|v| !v.is_null())
        && !id.as_str().is_some_and(super::is_id)
    {
        return Err(ApiError::bad_request("bad id"));
    }
    let by = import_actor(&st, &caller, &b)?;
    let hidden = q.include_hidden();
    let out = detached(async move {
        let _kb = st.kb_lock().lock().await;
        let (idx, line) = Kb::new(st.clone(), hidden).decide(&Value::Object(b), &by).await?;
        Ok(json!({"ok": true, "indexes": [idx], "decisions": [Value::Object(line.0)]}))
    })
    .await?;
    Ok(ok(&out))
}

/// `POST /api/worker/undo {indexes, actor}`.
pub async fn undo(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let b = body_json(&headers, body).await?;
    let idxs: Option<Vec<i64>> = b
        .get("indexes")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        // `isinstance(i, int)` holds for a bool, which is its integer
        .and_then(|a| {
            a.iter()
                .map(|i| match i {
                    Value::Bool(x) => Some(i64::from(*x)),
                    Value::Number(n) => n.as_i64(),
                    _ => None,
                })
                .collect()
        });
    let Some(mut idxs) = idxs else {
        return Err(ApiError::bad_request("indexes must be a non-empty list of integers"));
    };
    let by = import_actor(&st, &caller, &b)?;
    idxs.sort_unstable_by(|a, b| b.cmp(a));
    let hidden = q.include_hidden();
    let out = detached(async move {
        let (mut indexes, mut decisions) = (Vec::new(), Vec::new());
        let _kb = st.kb_lock().lock().await;
        let kb = Kb::new(st.clone(), hidden);
        for i in idxs {
            let (idx, line) = kb.undo(Some(i), &by).await?;
            indexes.push(idx);
            decisions.push(Value::Object(line.0));
        }
        Ok(json!({"ok": true, "indexes": indexes, "decisions": decisions}))
    })
    .await?;
    Ok(ok(&out))
}

/// `POST /api/worker/events {store: "merges", op: attach|detach, id, data, actor: ai:*}`: a machine judgment in the
/// merges store.
pub async fn events(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let b = body_json(&headers, body).await?;
    let by = import_actor(&st, &caller, &b)?;
    if !by.starts_with("ai:") {
        return Err(ApiError::bad_request(
            "events from the worker are machine judgments: actor must be ai:<what>",
        ));
    }
    let (store, op) = (super::str_of(&b, "store"), super::str_of(&b, "op"));
    if store != Some("merges") || !matches!(op, Some("attach" | "detach")) {
        return Err(ApiError::bad_request(
            "store/op must be one of {'merges': {'attach', 'detach'}}",
        ));
    }
    let op = op.unwrap_or_default().to_string();
    let eid = b.get("id").and_then(Value::as_str).filter(|i| EVENT_ID.is_match(i));
    let data = match b.get("data") {
        Some(Value::Object(d)) => Some(d.clone()),
        None | Some(Value::Null) => Some(Map::new()),
        Some(v) if !py::truthy(Some(v)) => Some(Map::new()),
        _ => None,
    };
    let (Some(eid), Some(data)) = (eid, data) else {
        return Err(ApiError::bad_request("id must be at:<hex> and data an object"));
    };
    let eid = eid.to_string();
    if op == "attach" {
        let text = match data.get("text") {
            Some(v) if py::truthy(Some(v)) => py::str(v),
            _ => String::new(),
        };
        if st.legal().is_legal(&text) {
            return Err(refused("refused by the legal rules; nothing was kept"));
        }
    }
    let out = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let already = |items: &pk_creative::Items| -> bool {
            let attached = items
                .get(&eid)
                .and_then(Value::as_object)
                .is_some_and(|c| c.get("status").and_then(Value::as_str) == Some("attached"));
            if op == "attach" { attached } else { !attached }
        };
        let done = json!({"ok": true, "id": eid, "already": true});
        if already(cr.items("merges").await?.as_ref()) {
            return Ok(done);
        }
        let data = Value::Object(data);
        let ops = [(op.clone(), eid.clone(), data.clone())];
        match cr.probe("merges", &ops, &by).await {
            Ok(()) => {}
            Err(ProbeError::Refused(m)) => return Err(ApiError::bad_request(format!("{op} refused: {m}"))),
            Err(ProbeError::Failed(e)) => return Err(e),
        }
        let pre = |items: &pk_creative::Items| -> ApiResult<Pre> {
            Ok(if already(items) {
                Pre::Done(axum::http::StatusCode::OK, done.clone())
            } else {
                Pre::Go
            })
        };
        Ok(match cr.append_if("merges", &op, &eid, data, &by, pre).await? {
            Written::Landed(rec) => json!({"ok": true, "id": eid, "record": super::merge::brief(&rec)}),
            Written::Skipped(_, v) => v,
        })
    })
    .await?;
    Ok(ok(&out))
}
