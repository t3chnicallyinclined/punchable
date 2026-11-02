//! The data the read routes answer from: the replayed state the database holds (`surreal_kb._fetch`), the AI layer
//! (`analysis` rows), the few text files the reads need (`srcfile` rows: `corpus/sets/*/meta.json`,
//! `kb/v0/learning.json`; the hosted Python writes the same rows to disk and reads them back), and the per-view index
//! built over it. One [`Data`] per `(meta:state.gen, meta:state.files_rev)`; it is immutable once built, shared
//! behind an `Arc`, and replaced whole when either changes.

use std::collections::BTreeMap;
use std::sync::Arc;

use pk_kb::View;
use pk_store::{Store, StoreError, from_db, vars};
use serde_json::{Map, Value, json};
use tokio::sync::OnceCell;

use crate::error::ApiError;
use crate::index::{Ai, AppIndex, SetMetas, visible};

/// The srcfile path of the learning file (`kb/v0/learning.json` under the data folder).
pub const LEARNING: &str = "kb/v0/learning.json";

/// What one request reads first: is there data, and which generation (`SELECT ... FROM ONLY meta:state`).
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// `meta:state.gen`.
    pub generation: Option<i64>,
    /// A reload is running.
    pub loading: bool,
    /// `meta:state.files_rev` (bumped when the worker posts files).
    pub files_rev: Value,
    /// A bundle has been loaded (`load_info.bundle`).
    pub has_bundle: bool,
}

impl State {
    /// The identity a [`Data`] is built for.
    pub fn key(&self) -> (Option<i64>, Value) {
        (self.generation, self.files_rev.clone())
    }
}

/// Read `meta:state` (one round trip). A database that was never loaded has no `meta` table: no bundle.
pub async fn state(store: &Store) -> Result<State, StoreError> {
    let v = from_db(
        store
            .last(
                "SELECT gen, loading, files_rev, load_info.bundle AS bundle FROM ONLY meta:state;",
                vars([]),
            )
            .await?,
    );
    Ok(State {
        generation: v.get("gen").and_then(Value::as_i64),
        loading: v.get("loading") == Some(&Value::Bool(true)),
        files_rev: v.get("files_rev").cloned().unwrap_or(Value::Null),
        has_bundle: v.get("bundle").is_some_and(|b| !b.is_null()),
    })
}

/// One generation of the data.
#[derive(Debug)]
pub struct Data {
    /// The state it was built for.
    pub state: State,
    /// `meta:state.load_info`.
    pub load_info: Value,
    /// `meta:state.orphans`.
    pub orphans: i64,
    /// The AI layer.
    pub ai: Ai,
    /// Working per joke, from the database.
    pub working: BTreeMap<String, (bool, Vec<String>)>,
    /// `corpus/sets/<dir>/meta.json` by folder.
    pub metas: SetMetas,
    /// `kb/v0/learning.json` as stored, when there is one.
    pub learning: Option<String>,
    /// How long the database read took, seconds.
    pub fetch_s: f64,
    visible: Arc<AppIndex>,
    /// The whole view when it differs from the visible one (some material is hidden); its index is built on the
    /// first `?include_hidden=1`.
    full_view: Option<View>,
    full: OnceCell<Arc<AppIndex>>,
}

impl Data {
    /// The index a request reads: the visible view, or everything on `?include_hidden=1`.
    pub async fn index(self: &Arc<Self>, include_hidden: bool) -> Result<Arc<AppIndex>, ApiError> {
        if !include_hidden || self.full_view.is_none() {
            return Ok(self.visible());
        }
        let me = Arc::clone(self);
        self.full
            .get_or_try_init(|| async move {
                tokio::task::spawn_blocking(move || {
                    let v = me.full_view.clone().unwrap_or_else(|| me.visible.view.clone());
                    Arc::new(AppIndex::build(v, &me.ai, &me.working, &me.metas))
                })
                .await
                // a panic in the build is a bug: a 500 (as the Python's exception), and nothing cached, so the
                // next request tries again
                .map_err(|e| ApiError::internal("JoinError", e))
            })
            .await
            .cloned()
    }

    /// The index of the visible view (built with the data).
    pub fn visible(&self) -> Arc<AppIndex> {
        Arc::clone(&self.visible)
    }

    /// `rev` as `/api/health` and `/api/live` say it (`db:<gen>`).
    pub fn rev(&self) -> String {
        format!(
            "db:{}",
            self.state
                .generation
                .map_or_else(|| "None".to_string(), |g| g.to_string())
        )
    }
}

fn rows(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a,
        Value::Null => Vec::new(),
        other => vec![other],
    }
}

/// Read everything for `st` and build the index (the CPU half in `spawn_blocking`).
pub async fn load(store: &Store, st: State, owner_id: &str) -> Result<Data, StoreError> {
    let t = std::time::Instant::now();
    let snap = store.fetch_snapshot().await?;
    let mut res = store
        .query(
            "SELECT set, meta, rows FROM analysis ORDER BY ord;\
             SELECT path, text FROM srcfile WHERE path = $learning \
               OR (string::starts_with(path, 'corpus/sets/') AND string::ends_with(path, '/meta.json'));",
            vars([("learning", pk_store::to_db(&json!(LEARNING)))]),
        )
        .await?
        .into_iter()
        .map(from_db);
    let analysis = rows(res.next().unwrap_or(Value::Null));
    let files = rows(res.next().unwrap_or(Value::Null));
    let fetch_s = t.elapsed().as_secs_f64();

    let mut ai = Ai {
        inferred: snap.load_info.get("inferred").cloned().unwrap_or(Value::Null),
        ..Ai::default()
    };
    for a in analysis {
        let Some(sid) = a.get("set").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        ai.analysis
            .insert(sid.clone(), a.get("meta").cloned().unwrap_or_else(|| json!({})));
        let pairs = a
            .get("rows")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|x| {
                (
                    x.get("row").cloned().unwrap_or(Value::Null),
                    x.get("perf").and_then(Value::as_str).map(str::to_string),
                )
            })
            .collect();
        ai.rows.insert(sid, pairs);
    }
    let mut metas = SetMetas::new();
    let mut learning = None;
    for f in files {
        let (Some(path), Some(text)) = (
            f.get("path").and_then(Value::as_str),
            f.get("text").and_then(Value::as_str),
        ) else {
            continue;
        };
        if path == LEARNING {
            learning = Some(text.to_string());
        } else if let Some(dir) = path
            .strip_prefix("corpus/sets/")
            .and_then(|p| p.strip_suffix("/meta.json"))
            .filter(|d| !d.contains('/'))
            && let Ok(Value::Object(o)) = serde_json::from_str::<Value>(text)
        {
            metas.insert(dir.to_string(), o);
        }
    }
    let owner = owner_id.to_string();
    let built = tokio::task::spawn_blocking(move || {
        let full = snap.view(Vec::new(), &owner);
        let graph = snap.graph();
        let vis = visible(&full, &graph, &owner);
        let (vis_view, full_view) = match vis {
            Some(v) => (v, Some(full)),
            None => (full, None),
        };
        let idx = AppIndex::build(vis_view, &ai, &snap.working, &metas);
        Data {
            state: st,
            load_info: snap.load_info,
            orphans: snap.orphans,
            ai,
            working: snap.working,
            metas,
            learning,
            fetch_s,
            visible: Arc::new(idx),
            full_view,
            full: OnceCell::new(),
        }
    })
    .await
    .map_err(|e| StoreError::mapping(format!("building the index failed: {e}")))?;
    Ok(built)
}

/// `ai_status` for `/api/health`.
pub fn ai_status(d: &Data) -> Value {
    let an = &d.ai.analysis;
    let source = |m: &Value| m.get("source").cloned().unwrap_or(Value::Null);
    let by_set: Map<String, Value> = an
        .iter()
        .map(|(k, m)| {
            (
                k.clone(),
                json!({"source": source(m), "run_id": m.get("run_id").cloned().unwrap_or(Value::Null)}),
            )
        })
        .collect();
    json!({
        "inferred": d.ai.inferred,
        "analysis": {
            "sets": an.len(),
            "real": an.values().filter(|m| source(m) == "real").count(),
            "fixture": an.values().filter(|m| source(m) == "fixture").count(),
            "by_set": by_set,
        },
    })
}
