//! The connection and what the services do with it: checked statement batches (every statement OK, or an error that
//! names the statement), guarded writes (rows, or [`StoreError::EmptyWrite`]), the decision-append transaction of
//! `api-dev/surreal_kb.py` `_append`, and the snapshot its `_fetch` builds the view from.

use std::collections::BTreeMap;
use std::time::Duration;

use pk_core::text::head;
use pk_kb::model::truthy_str;
use pk_kb::{Decision, Rec, View};
use serde_json::{Value as Json, json};
use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use surrealdb::opt::auth;
use surrealdb::types::{Array, Number, Object, Value};

use crate::config::{Auth, Config};
use crate::error::{Result, StoreError};
use crate::ids::{EDGE_TABLES, NODE_TABLES, rid_str};
use crate::kbmap::{DecisionRow, EdgeRow, MetaState, NodeRow, working_of};
use crate::value::{as_f64, as_i64, to_db};

/// The oldest server the services accept: HNSW lives in RAM and is rebuilt on restart; the identical-vector fix.
pub const MIN_VERSION: (u64, u64, u64) = (3, 3, 1);
/// Waits between retries of a "can be retried" write conflict (`db/sdb.py` `RETRY_WAITS`), in milliseconds.
const RETRY_WAITS: [u64; 4] = [200, 500, 1000, 2000];
/// Rows per `INSERT`.
const BATCH: usize = 500;

/// Bound variables of a statement batch: `$name` for each key.
pub type Vars = Object;

/// Bound variables from pairs.
pub fn vars<'a>(pairs: impl IntoIterator<Item = (&'a str, Value)>) -> Vars {
    let mut o = Object::new();
    for (k, v) in pairs {
        o.insert(k, v);
    }
    o
}

/// One connection to `SurrealDB`, signed in, on one namespace and database. Cheap to clone (the official client is
/// a handle).
#[derive(Debug, Clone)]
pub struct Store {
    db: Surreal<Any>,
    cfg: Config,
}

/// What [`Store::append_decision`] read back in the same transaction that created the row.
#[derive(Debug, Clone, PartialEq)]
pub struct Appended {
    /// The decision's `seq`.
    pub seq: i64,
    /// The touched node or edge as a KB dict, after the event applied the decision; `None` when it is gone or the
    /// decision touched nothing.
    pub record: Option<Rec>,
    /// Every joke that is working or has hit sets, after the write: `joke id -> (working, set ids)`.
    pub working: BTreeMap<String, (bool, Vec<String>)>,
    /// `meta:state.gen` after the write.
    pub generation: Option<i64>,
    /// `meta:state.orphans` after the write.
    pub orphans: i64,
    /// The decision found no target in the database (`orphans` grew).
    pub orphaned: bool,
}

/// The replayed state as the database holds it (`fn::apply_decision` already applied every decision): what
/// [`pk_kb::View`] needs, in `ord` order.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// `meta:state.gen`.
    pub generation: Option<i64>,
    /// A reload is running.
    pub loading: bool,
    /// Decisions whose target was gone.
    pub orphans: i64,
    /// `meta:state.load_info` (`graph_meta`, `graph`, `counts`, ...).
    pub load_info: Json,
    /// Every node as a KB dict, in `ord` order.
    pub nodes: Vec<Rec>,
    /// Every edge as a KB dict, in `ord` order.
    pub edges: Vec<Rec>,
    /// Every decision line, in `seq` order.
    pub decisions: Vec<Decision>,
    /// `joke id -> (working, set ids)` for every joke.
    pub working: BTreeMap<String, (bool, Vec<String>)>,
}

impl Snapshot {
    /// The nodes and edges as a `graph.json` object, with `version`, `generated_at` and `runs` from
    /// `load_info.graph_meta`.
    pub fn graph(&self) -> Json {
        let meta = self.load_info.get("graph_meta").cloned().unwrap_or(Json::Null);
        json!({
            "version": meta.get("version").cloned().unwrap_or(Json::Null),
            "generated_at": meta.get("generated_at").cloned().unwrap_or(Json::Null),
            "runs": meta.get("runs").cloned().unwrap_or(Json::Null),
            "nodes": self.nodes,
            "edges": self.edges,
        })
    }

    /// The view over this state: no replay (the database did it), merges resolved and indexed like
    /// `surreal_kb.SnapshotView`; the decisions and `undone` are kept for the read models.
    pub fn view(&self, report: Vec<String>, owner_id: &str) -> View {
        let mut v = View::new(&self.graph(), Vec::new(), report, owner_id);
        v.undone = self.decisions.iter().filter_map(Decision::undo_of).collect();
        v.decisions.clone_from(&self.decisions);
        v
    }
}

impl Store {
    /// Connect, sign in and select the namespace and database.
    pub async fn connect(cfg: Config) -> Result<Self> {
        let db = surrealdb::engine::any::connect(cfg.endpoint()).await?;
        match &cfg.auth {
            Auth::Root { user, pass } => {
                db.signin(auth::Root {
                    username: user.clone(),
                    password: pass.clone(),
                })
                .await?;
            }
            Auth::Database {
                user,
                pass,
                ns,
                db: dbn,
            } => {
                db.signin(auth::Database {
                    namespace: ns.clone(),
                    database: dbn.clone(),
                    username: user.clone(),
                    password: pass.clone(),
                })
                .await?;
            }
        }
        let store = Self { db, cfg };
        store.define_strict_database().await?;
        Ok(store)
    }

    /// The official client, for what this crate does not wrap.
    pub fn db(&self) -> &Surreal<Any> {
        &self.db
    }

    /// How this store was opened.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// The server's version.
    pub async fn version(&self) -> Result<(u64, u64, u64)> {
        let v = self.db.version().await?;
        Ok((v.major, v.minor, v.patch))
    }

    /// The server's version, refused below [`MIN_VERSION`].
    pub async fn check_version(&self) -> Result<(u64, u64, u64)> {
        let v = self.version().await?;
        if v < MIN_VERSION {
            return Err(StoreError::Config(format!(
                "SurrealDB {}.{}.{} is older than {}.{}.{}: the vector index needs >= 3.3.1",
                v.0, v.1, v.2, MIN_VERSION.0, MIN_VERSION.1, MIN_VERSION.2
            )));
        }
        Ok(v)
    }

    /// Run a batch; every statement's result, in order. The first statement that is not OK is an error. A
    /// "Transaction write conflict ... can be retried" is retried up to four times when that is safe (one statement,
    /// or one `BEGIN ... COMMIT` block: a failed transaction leaves nothing behind).
    pub async fn query(&self, sql: &str, vars: Vars) -> Result<Vec<Value>> {
        let begins = sql
            .trim_start()
            .get(..5)
            .is_some_and(|h| h.eq_ignore_ascii_case("BEGIN"));
        let mut attempt = 0;
        loop {
            let mut resp = self.db.query(sql).bind(Value::Object(vars.clone())).await?;
            let n = resp.num_statements();
            let mut errors = resp.take_errors();
            if errors.is_empty() {
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    out.push(resp.take::<Value>(i)?);
                }
                return Ok(out);
            }
            let index = *errors.keys().min().expect("not empty");
            let msg = errors.remove(&index).expect("present").to_string();
            let atomic = n == 1 || begins;
            if attempt < RETRY_WAITS.len() && atomic && msg.contains("can be retried") {
                tokio::time::sleep(Duration::from_millis(RETRY_WAITS[attempt])).await;
                attempt += 1;
                continue;
            }
            return Err(StoreError::Statement {
                index,
                msg,
                sql: head(sql, 600),
            });
        }
    }

    /// The last statement's result of a batch.
    pub async fn last(&self, sql: &str, vars: Vars) -> Result<Value> {
        Ok(self.query(sql, vars).await?.pop().unwrap_or(Value::None))
    }

    /// A write whose last statement must return rows (at least one, or exactly `expect`). Use `RETURN AFTER` or
    /// `RETURN id` (never `RETURN NONE`) on guarded writes.
    pub async fn write(&self, sql: &str, vars: Vars, expect: Option<usize>) -> Result<Vec<Value>> {
        let res = self.last(sql, vars).await?;
        let rows: Vec<Value> = match res {
            Value::Array(a) => a.into_inner().into_iter().filter(|r| !r.is_nullish()).collect(),
            Value::None | Value::Null => Vec::new(),
            other => vec![other],
        };
        if rows.is_empty() {
            return Err(StoreError::EmptyWrite { sql: head(sql, 400) });
        }
        if let Some(n) = expect
            && rows.len() != n
        {
            return Err(StoreError::WrongCount {
                sql: head(sql, 400),
                rows: rows.len(),
                expected: n,
            });
        }
        Ok(rows)
    }

    /// `meta:state`, or `None` when the database was never loaded.
    pub async fn meta_state(&self) -> Result<Option<MetaState>> {
        MetaState::from_record(self.last("SELECT * FROM ONLY meta:state;", Object::new()).await?)
    }

    /// How many decision rows there are.
    pub async fn decision_count(&self) -> Result<i64> {
        let v = self
            .last("RETURN count(SELECT id FROM decision);", Object::new())
            .await?;
        as_i64(&v).ok_or_else(|| StoreError::mapping(format!("count is not a number: {v:?}")))
    }

    /// `INSERT` rows in batches: into `table`, or as relations when `table` is `None`. Every row must come back.
    pub async fn insert_rows(&self, table: Option<&str>, rows: Vec<Value>) -> Result<()> {
        for chunk in rows.chunks(BATCH) {
            let sql = match table {
                Some(tb) => format!("INSERT INTO {tb} $rows RETURN id;"),
                None => "INSERT RELATION $rows RETURN id;".to_string(),
            };
            self.write(
                &sql,
                vars([("rows", Value::Array(Array::from(chunk.to_vec())))]),
                Some(chunk.len()),
            )
            .await?;
        }
        Ok(())
    }

    /// Load a `graph.json` object into an empty database the way `db/load.py` does: nodes then edges with `ord` in
    /// file order (an entry without a string id, or that is not an object, is skipped), `meta:state.ord_next` set
    /// past them, `load_info.graph_meta` recorded. Returns `(nodes, edges)` written.
    pub async fn load_graph(&self, graph: &Json) -> Result<(usize, usize)> {
        let mut ord = 0i64;
        let mut by_table: BTreeMap<&'static str, Vec<Value>> = BTreeMap::new();
        for n in graph.get("nodes").and_then(Json::as_array).into_iter().flatten() {
            let Json::Object(o) = n else { continue };
            if truthy_str(o.get("id")).is_none() {
                continue;
            }
            let row = NodeRow::from_node(o, ord)?;
            ord += 1;
            by_table.entry(row.table()).or_default().push(row.content());
        }
        let n_nodes = usize::try_from(ord).unwrap_or_default();
        for tb in NODE_TABLES {
            if let Some(rows) = by_table.remove(tb) {
                self.insert_rows(Some(tb), rows).await?;
            }
        }
        let mut erows = Vec::new();
        for e in graph.get("edges").and_then(Json::as_array).into_iter().flatten() {
            let Json::Object(o) = e else { continue };
            if truthy_str(o.get("id")).is_none() {
                continue;
            }
            erows.push(EdgeRow::from_edge(o, ord)?.content());
            ord += 1;
        }
        let n_edges = erows.len();
        self.insert_rows(None, erows).await?;
        let meta = json!({
            "version": graph.get("version").cloned().unwrap_or(Json::Null),
            "generated_at": graph.get("generated_at").cloned().unwrap_or(Json::Null),
            "runs": graph.get("runs").cloned().unwrap_or(Json::Null),
        });
        self.write(
            "UPDATE meta:state MERGE { ord_next: $n, load_info: { graph_meta: $meta } } RETURN id;",
            vars([("n", Value::Number(Number::Int(ord))), ("meta", to_db(&meta))]),
            None,
        )
        .await?;
        Ok((n_nodes, n_edges))
    }

    /// Append one decision: the `decision` row is created and `fn::apply_decision` (the event) applies it in the same
    /// transaction, which also reads back the touched record, every working joke and `meta:state`, so a snapshot can
    /// be patched without a full re-read (`surreal_kb._append`). `seq` is the current row count; when `file_count`
    /// is given (the lines in `decisions.jsonl`) and differs, the write is refused with 409 before anything happens,
    /// and 503 while a reload is running.
    pub async fn append_decision(&self, d: &Decision, file_count: Option<usize>) -> Result<Appended> {
        let pre = self
            .query(
                "SELECT gen, loading, orphans FROM ONLY meta:state; RETURN count(SELECT id FROM decision);",
                Object::new(),
            )
            .await?;
        let st = MetaState::from_record(pre.first().cloned().unwrap_or(Value::None))?
            .ok_or_else(|| StoreError::refused(503, "the database has never been loaded (python db/load.py)"))?;
        if st.loading == Some(true) {
            return Err(StoreError::refused(
                503,
                "the database is reloading; try again in a few seconds",
            ));
        }
        let n_db = pre.get(1).and_then(as_i64).unwrap_or(0);
        if let Some(n) = file_count
            && i64::try_from(n).ok() != Some(n_db)
        {
            return Err(StoreError::refused(
                409,
                format!(
                    "SurrealDB has {n_db} decisions but decisions.jsonl has {n}; run python db/load.py to resync before writing"
                ),
            ));
        }
        let seq = n_db;
        let row = DecisionRow::from_decision(d, seq);
        let target = d.target();
        let tgt = if target == Some("edge") {
            "fn::edge_find($tid)"
        } else {
            "fn::rid($tid)"
        };
        let tid = if matches!(target, Some("edge" | "node")) {
            d.id().map_or(Value::None, |s| Value::String(s.to_string()))
        } else {
            Value::String("joke:none".to_string())
        };
        // results: CREATE, LET, target, working, meta (BEGIN and COMMIT stripped by `transaction`)
        let sql = format!(
            "BEGIN TRANSACTION;\
             CREATE decision CONTENT $row RETURN id;\
             LET $t = {tgt};\
             IF $t != NONE AND record::exists($t) {{ (SELECT * OMIT embedding FROM ONLY $t) }} ELSE {{ NONE }};\
             SELECT id, working, working_sets FROM joke WHERE working = true OR array::len(working_sets) > 0;\
             SELECT gen, orphans FROM ONLY meta:state;\
             COMMIT TRANSACTION;"
        );
        let res = transaction(self.query(&sql, vars([("row", row.content()), ("tid", tid)])).await?, 5)?;
        if matches!(&res[0], Value::None | Value::Null) || matches!(&res[0], Value::Array(a) if a.is_empty()) {
            return Err(StoreError::EmptyWrite {
                sql: format!("decision {seq} was not written"),
            });
        }
        let record = match (&res[2], target) {
            (Value::Object(_), Some("edge")) => Some(EdgeRow::from_record(res[2].clone())?.into_edge()),
            (Value::Object(_), Some("node")) => Some(NodeRow::from_record(res[2].clone())?.into_node()),
            _ => None,
        };
        let working = working_rows(&res[3]);
        let st2 = MetaState::from_record(res[4].clone())?.unwrap_or_default();
        let orphans = st2.orphans.unwrap_or(0);
        Ok(Appended {
            seq,
            record,
            working,
            generation: st2.generation,
            orphans,
            orphaned: orphans > st.orphans.unwrap_or(0),
        })
    }

    /// Everything the view is built from, in one batch (`surreal_kb._fetch`): nodes (without vectors) and edges
    /// sorted by `ord`, decisions by `seq`, the working flags, `meta:state`.
    pub async fn fetch_snapshot(&self) -> Result<Snapshot> {
        let sql = format!(
            "SELECT * OMIT embedding FROM {};\
             SELECT * FROM {};\
             SELECT * FROM decision ORDER BY seq;\
             SELECT gen, loading, orphans, load_info FROM ONLY meta:state;",
            NODE_TABLES.join(", "),
            EDGE_TABLES.join(", ")
        );
        let mut res = self.query(&sql, Object::new()).await?;
        let st = MetaState::from_record(res.pop().unwrap_or(Value::None))?.unwrap_or_default();
        let decisions = rows_of(res.pop().unwrap_or(Value::None))
            .into_iter()
            .map(|r| DecisionRow::from_record(r).map(DecisionRow::into_decision))
            .collect::<Result<Vec<_>>>()?;
        let mut edges = rows_of(res.pop().unwrap_or(Value::None))
            .into_iter()
            .map(EdgeRow::from_record)
            .collect::<Result<Vec<_>>>()?;
        edges.sort_by_key(|e| e.ord);
        let mut nodes = rows_of(res.pop().unwrap_or(Value::None))
            .into_iter()
            .map(NodeRow::from_record)
            .collect::<Result<Vec<_>>>()?;
        nodes.sort_by_key(|n| n.ord);
        let working = nodes
            .iter()
            .filter(|n| n.kind == pk_kb::NodeKind::Joke)
            .map(|n| (rid_str(&n.id), n.working()))
            .collect();
        Ok(Snapshot {
            generation: st.generation,
            loading: st.loading.unwrap_or(false),
            orphans: st.orphans.unwrap_or(0),
            load_info: st.load_info.unwrap_or_else(|| json!({})),
            nodes: nodes.into_iter().map(NodeRow::into_node).collect(),
            edges: edges.into_iter().map(EdgeRow::into_edge).collect(),
            decisions,
            working,
        })
    }
}

/// The rows of a statement result: an array's items, a bare object as one row, nothing for `NONE`.
pub fn rows_of(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.into_inner(),
        Value::None | Value::Null => Vec::new(),
        other => vec![other],
    }
}

/// `joke id -> (working, set ids)` from `SELECT id, working, working_sets FROM joke` rows.
fn working_rows(v: &Value) -> BTreeMap<String, (bool, Vec<String>)> {
    let mut out = BTreeMap::new();
    if let Value::Array(a) = v {
        for row in a {
            if let Value::Object(o) = row
                && let Some(Value::RecordId(id)) = o.get("id")
            {
                out.insert(rid_str(id), working_of(o));
            }
        }
    }
    out
}

/// The `inner` statement results of a `BEGIN ... COMMIT` batch, whether or not the client counted `BEGIN` and
/// `COMMIT` as statements of their own.
fn transaction(mut res: Vec<Value>, inner: usize) -> Result<Vec<Value>> {
    if res.len() == inner + 2 {
        res.pop();
        res.remove(0);
    }
    if res.len() == inner {
        Ok(res)
    } else {
        Err(StoreError::mapping(format!(
            "transaction returned {} results, expected {inner}",
            res.len()
        )))
    }
}

/// `ord` as a float, for callers sorting partial selects (`ord` is `TYPE number`).
pub fn ord_of(v: &Value) -> f64 {
    match v {
        Value::Object(o) => o.get("ord").and_then(as_f64).unwrap_or(f64::MAX),
        _ => f64::MAX,
    }
}
