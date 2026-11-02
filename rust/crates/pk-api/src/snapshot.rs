//! The data the read routes answer from: the replayed state the database holds (`surreal_kb._fetch`), the AI layer
//! (`analysis` rows), the few text files the reads need (`srcfile` rows: `corpus/sets/*/meta.json`,
//! `kb/v0/learning.json`; the hosted Python writes the same rows to disk and reads them back), and the per-view index
//! built over it. One [`Data`] per `(meta:state.gen, meta:state.files_rev)`; it is immutable once built, shared
//! behind an `Arc`, and replaced whole when either changes.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use pk_kb::Decision;
use pk_store::{DecisionRow, EdgeRow, NodeRow, Snapshot, Store, StoreError, from_db, vars};
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
    /// `load_info.bundle` itself (a new bundle = read everything again).
    pub bundle: Value,
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
        bundle: v.get("bundle").cloned().unwrap_or(Value::Null),
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
    pub ai: Arc<Ai>,
    /// Working per joke, from the database.
    pub working: BTreeMap<String, (bool, Vec<String>)>,
    /// `corpus/sets/<dir>/meta.json` by folder.
    pub metas: Arc<SetMetas>,
    /// The rows this generation was built from (the next one patches them).
    snap: Arc<Snapshot>,
    /// `kb/v0/learning.json` as stored, when there is one.
    pub learning: Option<String>,
    /// How long the database read took, seconds.
    pub fetch_s: f64,
    visible: Arc<AppIndex>,
    /// Some material is hidden, so the whole view differs from the visible one: it and its index are built on the
    /// first `?include_hidden=1` only.
    has_hidden: bool,
    full: OnceCell<Arc<AppIndex>>,
    /// `PK_OWNER_ID`, for the lazy whole view.
    owner: String,
}

impl Data {
    /// The index a request reads: the visible view, or everything on `?include_hidden=1`.
    pub async fn index(self: &Arc<Self>, include_hidden: bool) -> Result<Arc<AppIndex>, ApiError> {
        if !include_hidden || !self.has_hidden {
            return Ok(self.visible());
        }
        let me = Arc::clone(self);
        self.full
            .get_or_try_init(|| async move {
                tokio::task::spawn_blocking(move || {
                    let v = me.snap.view(Vec::new(), &me.owner);
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
    build(
        Arc::new(snap),
        Arc::new(ai),
        Arc::new(metas),
        learning,
        st,
        fetch_s,
        owner_id,
    )
    .await
}

/// The CPU half: the view, the visible view and the index, in `spawn_blocking`.
async fn build(
    snap: Arc<Snapshot>,
    ai: Arc<Ai>,
    metas: Arc<SetMetas>,
    learning: Option<String>,
    st: State,
    fetch_s: f64,
    owner_id: &str,
) -> Result<Data, StoreError> {
    let owner = owner_id.to_string();
    tokio::task::spawn_blocking(move || {
        let t = std::time::Instant::now();
        // one view per rebuild: the visible one (the whole one only on `?include_hidden=1`)
        let (vis_view, has_hidden) = match visible(&snap, &owner) {
            Some(v) => (v, true),
            None => (snap.view(Vec::new(), &owner), false),
        };
        let t_view = t.elapsed();
        let idx = AppIndex::build(vis_view, &ai, &snap.working, &metas);
        tracing::info!(
            fetch_ms = fetch_s * 1000.0,
            view_ms = t_view.as_secs_f64() * 1000.0,
            index_ms = t.elapsed().saturating_sub(t_view).as_secs_f64() * 1000.0,
            "snapshot built"
        );
        Data {
            state: st,
            load_info: snap.load_info.clone(),
            orphans: snap.orphans,
            ai,
            working: snap.working.clone(),
            metas,
            learning,
            fetch_s,
            visible: Arc::new(idx),
            has_hidden,
            full: OnceCell::new(),
            snap,
            owner,
        }
    })
    .await
    .map_err(|e| StoreError::mapping(format!("building the index failed: {e}")))
}

/// The next generation from the previous one when only decisions were appended since (each one bumps `gen` by
/// one; the files, the bundle and the AI layer are unchanged): read the new decision rows, the current state of
/// each record they touched, and the Working flags, and rebuild the view over the patched rows. This is the
/// Python's `surreal_kb._patch`, applied to every writer's lines. Anything else (a reload, new files, a gap) is
/// `Ok(None)`: read everything.
pub async fn patch(store: &Store, old: &Data, st: State, owner_id: &str) -> Result<Option<Data>, StoreError> {
    let (Some(g0), Some(g1)) = (old.state.generation, st.generation) else {
        return Ok(None);
    };
    if st.loading || st.files_rev != old.state.files_rev || st.bundle != old.state.bundle || g1 <= g0 || g1 - g0 > 50 {
        return Ok(None);
    }
    let t = std::time::Instant::now();
    let n0 = old.snap.decisions.len();
    let rows_v = store
        .last(
            "SELECT * FROM decision WHERE seq >= $n ORDER BY seq;",
            vars([("n", pk_store::to_db(&json!(n0)))]),
        )
        .await?;
    let new: Vec<Decision> = pk_store::store::rows_of(rows_v)
        .into_iter()
        .map(|r| DecisionRow::from_record(r).map(DecisionRow::into_decision))
        .collect::<Result<_, _>>()?;
    if i64::try_from(new.len()).ok() != Some(g1 - g0) {
        return Ok(None);
    }
    // Only the shapes a patch reproduces exactly; everything else is a full read:
    // - edit / confirm / reject of a record that exists: updated in place, `ord` unchanged;
    // - add of a record that does not exist yet, once in the window: `fn::next_ord` gives it the highest ord, so it
    //   goes last, and several new records go in decision order, which is ord order.
    // A delete (the database drops the record's relations too), an add over an existing record (delete + upsert), a
    // target touched twice in one window, or any other action is not patched.
    let mut targets: Vec<(bool, String)> = Vec::new();
    for d in &new {
        let (Some(tg @ ("edge" | "node")), Some(id)) = (d.target(), d.id()) else {
            continue;
        };
        let edge = tg == "edge";
        let list = if edge { &old.snap.edges } else { &old.snap.nodes };
        let exists = list.iter().any(|r| r.get("id").and_then(Value::as_str) == Some(id));
        let ok = match d.action() {
            Some("edit" | "confirm" | "reject") => exists,
            Some("add") => !exists,
            _ => false,
        };
        if !ok || targets.iter().any(|(e, i)| *e == edge && i == id) {
            return Ok(None);
        }
        targets.push((edge, id.to_string()));
    }
    let mut sql = String::new();
    let mut vs = Vec::new();
    for (i, (edge, id)) in targets.iter().enumerate() {
        let f = if *edge { "fn::edge_find" } else { "fn::rid" };
        let _ = write!(
            sql,
            "LET $t{i} = {f}($id{i}); IF $t{i} != NONE AND record::exists($t{i}) \
             {{ (SELECT * OMIT embedding FROM ONLY $t{i}) }} ELSE {{ NONE }};"
        );
        vs.push((format!("id{i}"), pk_store::to_db(&json!(id))));
    }
    sql.push_str(
        "SELECT id, working, working_sets FROM joke WHERE working = true OR array::len(working_sets) > 0;\
         SELECT gen, orphans FROM ONLY meta:state;",
    );
    let res = store
        .query(&sql, vars(vs.iter().map(|(k, v)| (k.as_str(), v.clone()))))
        .await?;
    // each target is two statements (LET, the IF); the last two are Working and meta
    if res.len() != targets.len() * 2 + 2 {
        return Ok(None);
    }
    let meta = from_db(res[res.len() - 1].clone());
    if meta.get("gen").and_then(Value::as_i64) != Some(g1) {
        return Ok(None); // another write landed meanwhile: read everything
    }
    let mut snap = (*old.snap).clone();
    for (i, (edge, id)) in targets.iter().enumerate() {
        let rec = &res[i * 2 + 1];
        let list = if *edge { &mut snap.edges } else { &mut snap.nodes };
        let pos = list
            .iter()
            .position(|r| r.get("id").and_then(Value::as_str) == Some(id.as_str()));
        let row = if !from_db(rec.clone()).is_object() {
            None
        } else if *edge {
            Some(EdgeRow::from_record(rec.clone())?.into_edge())
        } else {
            Some(NodeRow::from_record(rec.clone())?.into_node())
        };
        match (pos, row) {
            (Some(p), Some(r)) => list[p] = r,
            (None, Some(r)) => {
                if !*edge && r.get("type").and_then(Value::as_str) == Some("joke") {
                    snap.working.insert(id.clone(), (false, Vec::new())); // a full read keys every joke
                }
                let list = if *edge { &mut snap.edges } else { &mut snap.nodes };
                list.push(r);
            }
            // gone, or an orphan add: not a shape a patch reproduces
            _ => return Ok(None),
        }
    }
    for w in snap.working.values_mut() {
        *w = (false, Vec::new());
    }
    for r in pk_store::store::rows_of(res[res.len() - 2].clone()) {
        let r = from_db(r);
        if let Some(id) = r.get("id").and_then(Value::as_str) {
            let sets = r
                .get("working_sets")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            snap.working
                .insert(id.to_string(), (r.get("working") == Some(&Value::Bool(true)), sets));
        }
    }
    snap.decisions.extend(new);
    snap.generation = Some(g1);
    snap.orphans = meta.get("orphans").and_then(Value::as_i64).unwrap_or(snap.orphans);
    let fetch_s = t.elapsed().as_secs_f64();
    let learning = old.learning.clone();
    build(
        Arc::new(snap),
        Arc::clone(&old.ai),
        Arc::clone(&old.metas),
        learning,
        st,
        fetch_s,
        owner_id,
    )
    .await
    .map(Some)
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
