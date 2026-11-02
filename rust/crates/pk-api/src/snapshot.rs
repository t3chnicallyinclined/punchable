//! The data the read routes answer from: the replayed state the database holds (`surreal_kb._fetch`), the AI layer
//! (`analysis` rows), the few text files the reads need (`srcfile` rows: `corpus/sets/*/meta.json`,
//! `kb/v0/learning.json`; the hosted Python writes the same rows to disk and reads them back), and the per-view index
//! built over it. One [`Data`] per `(meta:state.gen, meta:state.files_rev)`; it is immutable once built, shared
//! behind an `Arc`, and replaced whole when either changes.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

use pk_kb::{Decision, View};
use pk_store::{DecisionRow, EdgeRow, NodeRow, Snapshot, Store, StoreError, from_db, vars};
use serde_json::{Map, Value, json};
use tokio::sync::{OnceCell, Semaphore};

use crate::error::ApiError;
use crate::index::{Ai, AppIndex, SetMetas, hidden_ids, view_of};

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

/// The rows of one generation (`pk_store::Snapshot` without its state fields), each record behind an `Arc` so the
/// next generation shares every record a patch did not touch: a patch copies pointers, not records.
#[derive(Debug, Clone, Default)]
pub struct Rows {
    /// `meta:state.load_info`.
    pub load_info: Value,
    /// `meta:state.orphans`.
    pub orphans: i64,
    /// Every node as a KB dict, in `ord` order.
    pub nodes: Vec<Arc<pk_kb::Rec>>,
    /// Every edge as a KB dict, in `ord` order.
    pub edges: Vec<Arc<pk_kb::Rec>>,
    /// Every decision line, in `seq` order.
    pub decisions: Vec<Arc<Decision>>,
    /// `joke id -> (working, set ids)` for every joke.
    pub working: BTreeMap<String, (bool, Vec<String>)>,
}

impl From<Snapshot> for Rows {
    fn from(s: Snapshot) -> Self {
        Self {
            load_info: s.load_info,
            orphans: s.orphans,
            nodes: s.nodes.into_iter().map(Arc::new).collect(),
            edges: s.edges.into_iter().map(Arc::new).collect(),
            decisions: s.decisions.into_iter().map(Arc::new).collect(),
            working: s.working,
        }
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
    /// `corpus/sets/<dir>/meta.json` by folder.
    pub metas: Arc<SetMetas>,
    /// The rows this generation was built from (the next one patches them).
    snap: Arc<Rows>,
    /// `kb/v0/learning.json` as stored, when there is one.
    pub learning: Option<String>,
    /// How long the database read took, seconds.
    pub fetch_s: f64,
    /// Indexes of the decisions a later line undid (`View.undone`, as a set).
    undone: HashSet<usize>,
    /// Some material is hidden, so the whole view differs from the visible one.
    has_hidden: bool,
    /// `[visible, whole]`: each built on the first request that needs it, never with the rows.
    lazy: [Lazy; 2],
    /// `PK_OWNER_ID`, for the lazy views.
    owner: String,
    /// The service's one-at-a-time bound on CPU builds (`AppState::cpu`).
    cpu: Arc<Semaphore>,
}

/// One view of a generation and its index, each built at most once, on first use, in `spawn_blocking`.
#[derive(Debug, Default)]
struct Lazy {
    view: OnceCell<Arc<View>>,
    index: OnceCell<Arc<AppIndex>>,
}

impl Data {
    /// A generation over these rows. Cheap (a pass over the nodes and the decisions): the views and their indexes
    /// are built by the first route that needs them ([`Data::view`], [`Data::index`]).
    #[expect(
        clippy::too_many_arguments,
        reason = "one generation's parts, named; called from load and patch only"
    )]
    pub fn new(
        snap: Arc<Rows>,
        ai: Arc<Ai>,
        metas: Arc<SetMetas>,
        learning: Option<String>,
        st: State,
        fetch_s: f64,
        owner_id: &str,
        cpu: Arc<Semaphore>,
    ) -> Self {
        Self {
            state: st,
            load_info: snap.load_info.clone(),
            orphans: snap.orphans,
            ai,
            metas,
            learning,
            fetch_s,
            undone: snap.decisions.iter().filter_map(|d| d.undo_of()).collect(),
            has_hidden: !hidden_ids(&snap).is_empty(),
            lazy: [Lazy::default(), Lazy::default()],
            owner: owner_id.to_string(),
            cpu,
            snap,
        }
    }

    fn slot(&self, include_hidden: bool) -> usize {
        // nothing hidden: the whole view is the visible one, built once
        usize::from(include_hidden && self.has_hidden)
    }

    /// The view a request reads: the visible one, or everything on `?include_hidden=1`.
    ///
    /// Built once per generation, one CPU build at a time across the service, in a task of its own so a caller that
    /// hangs up does not throw the work away (the next caller finds it built).
    pub async fn view(self: &Arc<Self>, include_hidden: bool) -> Result<Arc<View>, ApiError> {
        let slot = self.slot(include_hidden);
        if let Some(v) = self.lazy[slot].view.get() {
            return Ok(Arc::clone(v));
        }
        let me = Arc::clone(self);
        let task = tokio::spawn(async move {
            let it = Arc::clone(&me);
            me.lazy[slot]
                .view
                .get_or_try_init(|| it.cpu_build(move |d| view_of(&d.snap, &d.owner, slot == 1)))
                .await
                .cloned()
        });
        task.await.map_err(|e| ApiError::internal("JoinError", e))?
    }

    /// The view and its index (the Bits list rows): what `/api/meta`, `/api/pipeline` and the cloud read. Built like
    /// [`Data::view`].
    pub async fn index(self: &Arc<Self>, include_hidden: bool) -> Result<Arc<AppIndex>, ApiError> {
        let slot = self.slot(include_hidden);
        if let Some(i) = self.lazy[slot].index.get() {
            return Ok(Arc::clone(i));
        }
        let view = self.view(include_hidden).await?;
        let me = Arc::clone(self);
        let task = tokio::spawn(async move {
            let it = Arc::clone(&me);
            me.lazy[slot]
                .index
                .get_or_try_init(|| {
                    it.cpu_build(move |d| {
                        let t = std::time::Instant::now();
                        let idx = AppIndex::build(view, &d.ai, &d.snap.working, &d.metas);
                        tracing::debug!(ms = t.elapsed().as_secs_f64() * 1000.0, slot, "index built");
                        idx
                    })
                })
                .await
                .cloned()
        });
        task.await.map_err(|e| ApiError::internal("JoinError", e))?
    }

    /// Run `f` over this generation in `spawn_blocking`, holding the service's CPU permit.
    async fn cpu_build<T: Send + Sync + 'static>(
        self: Arc<Self>,
        f: impl FnOnce(&Self) -> T + Send + 'static,
    ) -> Result<Arc<T>, ApiError> {
        let _permit = Arc::clone(&self.cpu)
            .acquire_owned()
            .await
            .map_err(|e| ApiError::internal("AcquireError", e))?;
        tokio::task::spawn_blocking(move || Arc::new(f(&self)))
            .await
            // a panic in the build is a bug: a 500 (as the Python's exception), and nothing cached, so the next
            // request tries again
            .map_err(|e| ApiError::internal("JoinError", e))
    }

    /// Every decision line, in `seq` order (the same in both views).
    pub fn decisions(&self) -> &[Arc<Decision>] {
        &self.snap.decisions
    }

    /// Whether decision `i` was undone (`View.is_undone`).
    pub fn is_undone(&self, i: usize) -> bool {
        self.undone.contains(&i)
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
pub async fn load(store: &Store, st: State, owner_id: &str, cpu: Arc<Semaphore>) -> Result<Data, StoreError> {
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
    // the rows are parsed; what is left is a pass over them, off the runtime like every other CPU step here
    tokio::task::spawn_blocking(move || {
        Data::new(
            Arc::new(Rows::from(snap)),
            Arc::new(ai),
            Arc::new(metas),
            learning,
            st,
            fetch_s,
            &owner,
            cpu,
        )
    })
    .await
    .map_err(|e| StoreError::mapping(format!("building the data failed: {e}")))
}

/// The next generation from the previous one when only decisions were appended since (each one bumps `gen` by
/// one; the files, the bundle and the AI layer are unchanged): read the new decision rows, the current state of
/// each record they touched, and the Working flags, and rebuild the view over the patched rows. This is the
/// Python's `surreal_kb._patch`, applied to every writer's lines. Anything else (a reload, new files, a gap) is
/// `Ok(None)`: read everything.
pub async fn patch(store: &Store, old: Arc<Data>, st: State, owner_id: &str) -> Result<Option<Data>, StoreError> {
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
    // the touched records and Working as KB dicts (a few rows), then the copy of everything else off the runtime
    let mut rows = Vec::with_capacity(targets.len());
    for (i, (edge, _)) in targets.iter().enumerate() {
        let rec = &res[i * 2 + 1];
        rows.push(if !from_db(rec.clone()).is_object() {
            None
        } else if *edge {
            Some(EdgeRow::from_record(rec.clone())?.into_edge())
        } else {
            Some(NodeRow::from_record(rec.clone())?.into_node())
        });
    }
    let working: Vec<Value> = pk_store::store::rows_of(res[res.len() - 2].clone())
        .into_iter()
        .map(from_db)
        .collect();
    let fetch_s = t.elapsed().as_secs_f64();
    let owner = owner_id.to_string();
    let patch = Patch {
        new,
        targets,
        rows,
        working,
        orphans: meta.get("orphans").and_then(Value::as_i64),
    };
    tokio::task::spawn_blocking(move || patch.apply(&old, st, fetch_s, &owner))
        .await
        .map_err(|e| StoreError::mapping(format!("patching the data failed: {e}")))
}

/// What a patch read: the new decisions, the touched records (`None` = gone), Working, and the orphan count.
struct Patch {
    new: Vec<Decision>,
    targets: Vec<(bool, String)>,
    rows: Vec<Option<pk_kb::Rec>>,
    working: Vec<Value>,
    orphans: Option<i64>,
}

impl Patch {
    /// The CPU half: a copy of the previous rows (every node, edge and decision) with the patch applied.
    fn apply(self, old: &Data, st: State, fetch_s: f64, owner_id: &str) -> Option<Data> {
        // pointers to the previous generation's records; only the touched ones are replaced below
        let mut snap = (*old.snap).clone();
        for ((edge, id), row) in self.targets.iter().zip(self.rows) {
            let list = if *edge { &mut snap.edges } else { &mut snap.nodes };
            let pos = list
                .iter()
                .position(|r| r.get("id").and_then(Value::as_str) == Some(id.as_str()));
            match (pos, row) {
                (Some(p), Some(r)) => list[p] = Arc::new(r),
                (None, Some(r)) => {
                    if !*edge && r.get("type").and_then(Value::as_str) == Some("joke") {
                        snap.working.insert(id.clone(), (false, Vec::new())); // a full read keys every joke
                    }
                    list.push(Arc::new(r));
                }
                // gone, or an orphan add: not a shape a patch reproduces
                _ => return None,
            }
        }
        for w in snap.working.values_mut() {
            *w = (false, Vec::new());
        }
        for r in &self.working {
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
        snap.decisions.extend(self.new.into_iter().map(Arc::new));
        snap.orphans = self.orphans.unwrap_or(snap.orphans);
        Some(Data::new(
            Arc::new(snap),
            Arc::clone(&old.ai),
            Arc::clone(&old.metas),
            old.learning.clone(),
            st,
            fetch_s,
            owner_id,
            Arc::clone(&old.cpu),
        ))
    }
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
