//! The graph tables brought up to date with the files, writing only what changed: `db/load.py` `sync`, the part of
//! the loader the API runs after the home worker posts a set's pipeline output (`surreal_kb.sync_now`).
//!
//! The target is computed by [`pk_kb::overlay::build_overlay`] (the AI layer over `graph.json`) and the replay of the
//! decision rows the database already has ([`pk_kb::View::replayed`]); records that differ from the database are
//! rewritten, those that are gone are deleted, and the side tables (`laugh_run`, `laugh`, `analysis`) are replaced.
//!
//! Not here, on purpose: the bulk `load` (the dbload pod's job), the schema (`apply_schema`: the dbload pod applies
//! `schema.surql`, pk-api applies `schema_extra.surql`), and the decision lines another writer appended to a
//! `decisions.jsonl` (the decisions are the database's rows, so there are none to append). A sync that would need
//! the full load (the data was never loaded; the records' order changed) is refused with a 503 instead of doing it.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::time::Instant;

use pk_kb::overlay::{Overlaid, OverlayFiles, build_overlay_owned, set_id_of};
use pk_kb::{Rec, View};
use serde_json::{Value as Json, json};
use surrealdb::types::{Array, Number, Object, RecordId, Value};

use crate::error::{Result, StoreError};
use crate::ids::{EDGE_TABLES, NODE_TABLES, parse_node_rid, rid_str};
use crate::kbmap::{DecisionRow, EdgeRow, NodeRow, laugh_to_row};
use crate::store::{Store, rows_of, vars};
use crate::value::{from_db, to_db};

/// What a sync reads: the parsed files and the paths `load_info` names them by.
#[derive(Debug)]
pub struct SyncInput<'a> {
    /// `graph.json`, the bundle's. OWNED: the sync moves its records and frees what it has used as it goes.
    pub graph: Json,
    /// The AI documents and set metas.
    pub files: OverlayFiles,
    /// Each set folder's parsed `laughs_auto.json` (objects only, in folder-name order); consumed by the side tables.
    pub laughs: Vec<(String, Json)>,
    /// `(path, bytes)` of every input read, for `load_info.sources_sig`.
    pub sources: Vec<(String, usize)>,
    /// The `graph`, `kb`, `decisions`, `corpus` and `id_map` strings `load_info` carries.
    pub paths: BTreeMap<&'static str, Json>,
    /// The actor id on the comic's own edits (the replay marks them).
    pub owner: &'a str,
}

/// What `load_info` is told about a sync, once the graph itself has been given up.
#[derive(Debug)]
struct InfoCtx {
    /// `version`, `generated_at` and `runs` of the graph.
    meta: Json,
    /// `(path, bytes)` of every input read.
    sources: Vec<(String, usize)>,
    /// The paths `load_info` carries.
    paths: BTreeMap<&'static str, Json>,
}

/// What a sync did.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncOutcome {
    /// Always `sync`.
    pub mode: &'static str,
    /// Nodes and edges rewritten, records removed, decisions appended.
    pub changed: Json,
    /// Seconds it took.
    pub seconds: f64,
}

/// How many laughs one `INSERT` of the side-table transaction carries.
const SIDE_BATCH: usize = 400;
/// How long a sync may hold `lease:sync` without finishing before another process may take it (seconds).
const SYNC_LEASE_S: u32 = 30;
/// How often a running sync renews its lease (seconds): a third of the lease, so one missed beat is survived.
const SYNC_RENEW_S: u64 = 10;

/// `load_info` keys another writer owns and a sync carries over (`FOREIGN_INFO`).
const FOREIGN_INFO: [&str; 1] = ["bundle"];

/// A row position as the database's integer.
fn idx(i: usize) -> i64 {
    i64::try_from(i).unwrap_or(i64::MAX)
}

/// A count as `f64` (a record count is far below `u32::MAX`).
fn small(i: usize) -> f64 {
    f64::from(u32::try_from(i).unwrap_or(u32::MAX))
}

/// `int(max(ords)) + 1`: the next free whole `ord`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "ords are small non-negative counts or fractions between them"
)]
fn next_ord(max: f64) -> i64 {
    max.trunc() as i64 + 1
}

fn full_load(why: &str) -> StoreError {
    StoreError::refused(
        503,
        format!("the graph tables need a full load first ({why}): run dbload"),
    )
}

/// `[ord, KB record]` of a database row, by KB id.
type Existing = (f64, Rec);

impl Store {
    /// `db_records`: the database's nodes and edges as KB dicts with their `ord`, and its decision rows.
    async fn db_records(
        &self,
    ) -> Result<(
        BTreeMap<String, Existing>,
        BTreeMap<String, Existing>,
        Vec<pk_kb::Decision>,
    )> {
        let sql = format!(
            "SELECT * OMIT embedding FROM {};SELECT * FROM {};SELECT * FROM decision ORDER BY seq;",
            NODE_TABLES.join(", "),
            EDGE_TABLES.join(", ")
        );
        let mut res = self.query(&sql, Object::new()).await?;
        let decs = rows_of(res.pop().unwrap_or(Value::None))
            .into_iter()
            .map(|r| DecisionRow::from_record(r).map(DecisionRow::into_decision))
            .collect::<Result<Vec<_>>>()?;
        let mut edges = BTreeMap::new();
        for r in rows_of(res.pop().unwrap_or(Value::None)) {
            let row = EdgeRow::from_record(r)?;
            let ord = row.ord;
            let e = row.into_edge();
            let id = e.get("id").and_then(Json::as_str).unwrap_or("").to_string();
            edges.insert(id, (ord, e));
        }
        let mut nodes = BTreeMap::new();
        for r in rows_of(res.pop().unwrap_or(Value::None)) {
            let row = NodeRow::from_record(r)?;
            let (ord, id) = (row.ord, rid_str(&row.id));
            nodes.insert(id, (ord, row.into_node()));
        }
        Ok((nodes, edges, decs))
    }

    /// `write_side_tables`: `laugh_run` + `laugh` (every set folder with a `laughs_auto.json`) and `analysis` (the AI
    /// index) are replaced. Returns how many laughs were written. ONE transaction (the Python's are separate
    /// statements): a crash or a refused row halfway leaves the tables as they were, never empty.
    async fn write_side_tables(&self, laughs: Vec<(String, Json)>, index: &pk_kb::overlay::Index) -> Result<usize> {
        let mut sql =
            String::from("BEGIN; DELETE analysis RETURN NONE; DELETE laugh RETURN NONE; DELETE laugh_run RETURN NONE;");
        let mut bound = Object::new();
        let mut nl = 0;
        for (n, (dir, mut doc)) in laughs.into_iter().enumerate() {
            let (Some(sid), Some(obj)) = (set_id_of(&dir), doc.as_object_mut()) else {
                continue;
            };
            let set_rid = parse_node_rid(&sid)?;
            let laughs_v = obj.remove("laughs");
            let kept: serde_json::Map<String, Json> = std::mem::take(obj);
            let mut content = Object::new();
            content.insert("set", Value::RecordId(set_rid.clone()));
            content.insert("set_dir", Value::String(dir.clone()));
            content.insert("doc", to_db(&Json::Object(kept)));
            bound.insert(format!("rk{n}"), Value::String(crate::ids::key_str(&set_rid.key)));
            bound.insert(format!("rc{n}"), Value::Object(content));
            let _ = write!(
                sql,
                "CREATE type::record('laugh_run', $rk{n}) CONTENT $rc{n} RETURN id;"
            );
            let mut rows: Vec<Value> = laughs_v
                .as_ref()
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
                .enumerate()
                .filter_map(|(i, l)| l.as_object().map(|l| laugh_to_row(l, &set_rid, idx(i))))
                .collect();
            drop(laughs_v);
            nl += rows.len();
            let mut c = 0;
            while !rows.is_empty() {
                let chunk: Vec<Value> = rows.drain(..SIDE_BATCH.min(rows.len())).collect();
                bound.insert(format!("lr{n}_{c}"), Value::Array(Array::from(chunk)));
                let _ = write!(sql, "INSERT INTO laugh $lr{n}_{c} RETURN id;");
                c += 1;
            }
        }
        // the AI index (set analyses and the inferred run), so the API needs no AI files at all
        for (i, (sid, meta)) in index.analysis.iter().enumerate() {
            let set_rid = parse_node_rid(sid)?;
            let rows: Vec<Json> = index
                .rows
                .get(sid)
                .into_iter()
                .flatten()
                .map(|(row, pid)| json!({"row": row, "perf": pid}))
                .collect();
            let mut c = Object::new();
            c.insert("set", Value::RecordId(set_rid.clone()));
            c.insert("ord", Value::Number(Number::Int(idx(i))));
            c.insert("meta", to_db(meta));
            c.insert("rows", to_db(&Json::Array(rows)));
            bound.insert(format!("ak{i}"), Value::String(crate::ids::key_str(&set_rid.key)));
            bound.insert(format!("ac{i}"), Value::Object(c));
            let _ = write!(sql, "CREATE type::record('analysis', $ak{i}) CONTENT $ac{i} RETURN id;");
        }
        sql.push_str("COMMIT;");
        self.query(&sql, bound).await?;
        Ok(nl)
    }

    /// `make_info`: what `load_info` says about this load (counts read from the tables).
    async fn make_info(
        &self,
        ctx: &InfoCtx,
        li: &Json,
        merged_index: &pk_kb::overlay::Index,
        laughs: usize,
        n_decisions: usize,
    ) -> Result<Json> {
        let edge_counts: Vec<String> = EDGE_TABLES
            .iter()
            .map(|t| format!("count(SELECT id FROM {t})"))
            .collect();
        let sql = format!(
            "RETURN {{ joke: count(SELECT id FROM joke), unit: count(SELECT id FROM unit), label: count(SELECT id FROM label), \
             set: count(SELECT id FROM set), performance: count(SELECT id FROM performance), list: count(SELECT id FROM list), \
             laugh: count(SELECT id FROM laugh), decision: count(SELECT id FROM decision), analysis: count(SELECT id FROM analysis), \
             edges: math::sum([{}]), working: count(SELECT id FROM joke WHERE working = true), \
             vectors: count(SELECT id FROM unit WHERE embedding != NONE), hidden_units: count(SELECT id FROM unit WHERE hidden = true) }};",
            edge_counts.join(", ")
        );
        let counts = from_db(self.last(&sql, Object::new()).await?);
        let st = self
            .last("SELECT orphans, gen FROM ONLY meta:state;", Object::new())
            .await?;
        let orphans = match &st {
            Value::Object(o) => o.get("orphans").map_or(Json::Null, |v| from_db(v.clone())),
            _ => Json::Null,
        };
        let p = |k: &str| ctx.paths.get(k).cloned().unwrap_or(Json::Null);
        let g = &ctx.meta;
        let sig: Vec<Json> = ctx.sources.iter().map(|(path, n)| json!([path, 0, n])).collect();
        let mut info = json!({
            "graph": p("graph"),
            "graph_version": li.get("graph_version").cloned().unwrap_or_else(|| json!("?")),
            "graph_reason": li.get("graph_reason").cloned().unwrap_or_else(|| json!("")),
            "graph_meta": {"version": g.get("version"), "generated_at": g.get("generated_at"), "runs": g.get("runs")},
            "kb": p("kb"), "decisions": p("decisions"), "corpus": p("corpus"), "fixtures": Json::Null, "id_map": p("id_map"),
            "id_map_log": merged_index.id_map, "inferred": merged_index.inferred,
            "bad_lines": 0, "decision_lines": n_decisions, "orphans": orphans,
            "loaded_at": pk_core::time::now_iso(), "mode": "sync",
            "counts": counts, "missing_vectors_embedded": 0, "laughs_loaded": laughs,
            "sources_sig": sig,
        });
        // a load or sync rewrites load_info; the keys it does not make (the bundle record) are carried over
        if let Some(o) = info.as_object_mut() {
            for k in FOREIGN_INFO {
                if let Some(v) = li.get(k)
                    && !o.contains_key(k)
                {
                    o.insert(k.to_string(), v.clone());
                }
            }
        }
        Ok(info)
    }

    /// `db/load.py` `sync`: bring the graph tables up to date with `input`, writing only what changed. `vectors` is
    /// asked for (once) only when a unit has to be rewritten, and gives `unit id -> vector` for the units that
    /// have one (`load_vectors`).
    pub async fn sync_graph(
        &self,
        input: SyncInput<'_>,
        vectors: impl AsyncFnOnce() -> Result<HashMap<String, Vec<f64>>>,
    ) -> Result<SyncOutcome> {
        let me = format!(
            "sync-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        );
        if !self.lease_acquire("sync", &me, SYNC_LEASE_S).await? {
            return Err(StoreError::refused(
                503,
                "another sync is running; this file is saved and the next post syncs it",
            ));
        }
        // the lease is renewed while the sync runs, so it can be short: a process that dies mid-sync loses it within
        // `SYNC_LEASE_S`, and the recovery (every supervise round) clears the flag it left
        let renew = {
            let (store, me) = (self.clone(), me.clone());
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(SYNC_RENEW_S)).await;
                    if let Err(e) = store.lease_acquire("sync", &me, SYNC_LEASE_S).await {
                        tracing::warn!(error = %e, "lease:sync was not renewed");
                    }
                }
            })
        };
        let out = self.sync_locked(input, vectors).await;
        renew.abort();
        // the lease is given back whatever happened; a process that dies keeps it until it lapses, and the next
        // start clears `loading` when nobody holds it (`sync_recover`)
        if let Err(e) = self.lease_release("sync", &me).await {
            tracing::warn!(error = %e, "lease:sync was not released; it lapses in a few minutes");
        }
        // a failed sync is owed: the next post syncs again even when its own files changed nothing
        let flag = if out.is_ok() { "false" } else { "true" };
        if let Err(e) = self
            .query(
                &format!("UPDATE meta:state SET sync_owed = {flag} RETURN NONE;"),
                Object::new(),
            )
            .await
        {
            tracing::warn!(error = %e, "meta:state.sync_owed was not written");
        }
        out
    }

    /// Whether a sync failed and has not been made good since (`meta:state.sync_owed`): the caller syncs again at its
    /// next chance, whatever its own files say.
    pub async fn sync_owed(&self) -> Result<bool> {
        let v = self
            .last("SELECT VALUE sync_owed FROM ONLY meta:state;", Object::new())
            .await?;
        Ok(matches!(v, Value::Bool(true)))
    }

    /// At start: a process killed in the middle of a sync leaves `meta:state.loading` true (with `loading_by = 'sync'`; a dbload sets `loading` alone and its flag is never touched here), and every write answers 503
    /// while it is. When nobody holds a live `lease:sync` the flag is cleared (and the sync owed). Returns whether it
    /// was cleared.
    pub async fn sync_recover(&self) -> Result<bool> {
        let v = self
            .last(
                "IF (SELECT VALUE loading FROM ONLY meta:state) = true \
                    AND (SELECT VALUE loading_by FROM ONLY meta:state) = 'sync' \
                    AND (!record::exists(lease:sync) OR lease:sync.owner = NONE OR lease:sync.until = NONE \
                         OR lease:sync.until < time::now()) \
                 { UPDATE meta:state SET loading = false, loading_by = NONE, sync_owed = true, gen += 1 RETURN VALUE id } ELSE { [] };",
                Object::new(),
            )
            .await?;
        Ok(!rows_of(v).is_empty())
    }

    async fn sync_locked(
        &self,
        input: SyncInput<'_>,
        vectors: impl AsyncFnOnce() -> Result<HashMap<String, Vec<f64>>>,
    ) -> Result<SyncOutcome> {
        let t0 = Instant::now();
        let st = self.last("SELECT * FROM ONLY meta:state;", Object::new()).await?;
        let li = match st {
            Value::Object(o) => o.get("load_info").map_or(Json::Null, |v| from_db(v.clone())),
            _ => Json::Null,
        };
        if !li.get("graph").is_some_and(|g| pk_core::py::truthy(Some(g))) {
            return Err(full_load("never loaded"));
        }
        let SyncInput {
            graph,
            files,
            laughs,
            sources,
            paths,
            owner,
        } = input;
        // the graph's records are MOVED into the overlay and from there into the replay: one copy of the graph is alive at
        // a time, and the files and the parsed graph are given up as soon as they have been used
        let Overlaid {
            meta,
            nodes,
            edges,
            index,
        } = build_overlay_owned(graph, &files).map_err(StoreError::mapping)?;
        drop(files);
        let (mut nodes_db, mut edges_db, decs) = self.db_records().await?;
        let n_db = decs.len();
        let mut want = View::replayed(&meta, nodes, edges, decs, Vec::new(), owner);
        let ctx = InfoCtx { meta, sources, paths };

        // ord: keep every existing record's ord; slot new ones between their neighbours (ord is a number). Order
        // matters within nodes and within edges (each is sorted on its own), not across them.
        let max_db = nodes_db
            .values()
            .chain(edges_db.values())
            .map(|v| v.0)
            .fold(f64::NEG_INFINITY, f64::max);
        let mut ords: HashMap<(char, String), f64> = HashMap::new();
        for (kind, keys, dbmap) in [
            ('n', want.nodes.keys().cloned().collect::<Vec<_>>(), &nodes_db),
            ('e', want.edges.keys().cloned().collect::<Vec<_>>(), &edges_db),
        ] {
            let mut last: Option<f64> = None;
            let mut pending: Vec<String> = Vec::new();
            for k in keys {
                if let Some((o, _)) = dbmap.get(&k) {
                    let o = *o;
                    if last.is_some_and(|l| o <= l) {
                        return Err(full_load("record order changed"));
                    }
                    let lo = last.unwrap_or(o - 1.0);
                    let n = small(pending.len());
                    for (i, pk) in pending.drain(..).enumerate() {
                        ords.insert((kind, pk), lo + (o - lo) * (small(i) + 1.0) / (n + 1.0));
                    }
                    last = Some(o);
                    ords.insert((kind, k), o);
                } else {
                    pending.push(k);
                }
            }
            let top = last.unwrap_or(-1.0).max(max_db);
            for (i, pk) in pending.into_iter().enumerate() {
                ords.insert((kind, pk), top + small(i) + 1.0);
            }
        }

        self.write(
            "UPDATE meta:state SET loading = true, loading_by = 'sync', gen += 1 RETURN id;",
            Object::new(),
            None,
        )
        .await?;
        let done = self
            .sync_writes(
                &ctx,
                laughs,
                &li,
                &index,
                &mut want,
                &mut nodes_db,
                &mut edges_db,
                &ords,
                n_db,
                vectors,
            )
            .await;
        // `finally`: the flag comes down whatever happened above
        let down = self
            .query(
                "UPDATE meta:state SET loading = false, loading_by = NONE, gen += 1 RETURN NONE;",
                Object::new(),
            )
            .await;
        let changed = done?;
        down?;
        Ok(SyncOutcome {
            mode: "sync",
            changed,
            seconds: t0.elapsed().as_secs_f64(),
        })
    }

    #[expect(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "the Python's one block, kept whole"
    )]
    async fn sync_writes(
        &self,
        ctx: &InfoCtx,
        laughs: Vec<(String, Json)>,
        li: &Json,
        index: &pk_kb::overlay::Index,
        want: &mut View,
        nodes_db: &mut BTreeMap<String, Existing>,
        edges_db: &mut BTreeMap<String, Existing>,
        ords: &HashMap<(char, String), f64>,
        n_db: usize,
        vectors: impl AsyncFnOnce() -> Result<HashMap<String, Vec<f64>>>,
    ) -> Result<Json> {
        let (mut n_nodes, mut n_edges, mut removed) = (0u64, 0u64, 0u64);
        let mut vec: Option<HashMap<String, Vec<f64>>> = None;
        let mut vectors = Some(vectors);
        let gone_nodes: Vec<String> = nodes_db
            .keys()
            .filter(|k| !want.nodes.contains_key(*k))
            .cloned()
            .collect();
        for nid in gone_nodes {
            nodes_db.remove(&nid);
            self.write(
                "DELETE $id RETURN BEFORE;",
                vars([("id", Value::RecordId(parse_node_rid(&nid)?))]),
                None,
            )
            .await?;
            removed += 1;
        }
        let gone_edges: Vec<Rec> = edges_db
            .iter()
            .filter(|(k, _)| !want.edges.contains_key(*k))
            .map(|(_, (_, e))| e.clone())
            .collect();
        // a record is given up as soon as it has been compared or written
        for (nid, n) in std::mem::take(&mut want.nodes) {
            if let Some((_, cur)) = nodes_db.remove(&nid)
                && pk_core::py::eq_map(&cur, &n)
            {
                continue;
            }
            let row = NodeRow::from_node(&n, ords[&('n', nid.clone())])?;
            let mut content = match row.content() {
                Value::Object(o) => o,
                other => return Err(StoreError::mapping(format!("node row is not an object: {other:?}"))),
            };
            if row.table() == "unit" {
                if vec.is_none()
                    && let Some(f) = vectors.take()
                {
                    vec = Some(f().await?);
                }
                if let Some(v) = vec.as_ref().and_then(|m| m.get(&nid)) {
                    content.insert(
                        "embedding",
                        Value::Array(Array::from(
                            v.iter().map(|x| Value::Number(Number::Float(*x))).collect::<Vec<_>>(),
                        )),
                    );
                }
            }
            let is_joke = row.table() == "joke";
            if is_joke {
                // working / working_sets are DB-only (fn::working): CONTENT replaces the record and a field DEFAULT
                // does not fill an existing one, so set them and recompute right after
                content.insert("working", Value::Bool(false));
                content.insert("working_sets", Value::Array(Array::new()));
            }
            content.remove("id");
            self.write(
                "UPSERT $id CONTENT $row RETURN id;",
                vars([("id", Value::RecordId(row.id.clone())), ("row", Value::Object(content))]),
                None,
            )
            .await?;
            if is_joke {
                self.query(
                    "fn::recompute_working($id);",
                    vars([("id", Value::RecordId(row.id.clone()))]),
                )
                .await?;
            }
            n_nodes += 1;
        }
        let edge_rid = |e: &Rec| -> Result<RecordId> {
            let kind = e.get("kind").and_then(Json::as_str).unwrap_or("");
            let id = e.get("id").and_then(Json::as_str).unwrap_or("");
            Ok(RecordId::new(
                crate::ids::edge_table(kind)?,
                id.strip_prefix("e:").unwrap_or(id),
            ))
        };
        for e in gone_edges {
            // an edge whose node was deleted above went with it (SurrealDB removes a record's edges): already gone is
            // fine here, where the Python's `write` answers EmptyWrite (a 500) for the removal of a set's analysis
            let id = e.get("id").and_then(Json::as_str).unwrap_or("").to_string();
            edges_db.remove(&id);
            self.query(
                "DELETE $id RETURN NONE;",
                vars([("id", Value::RecordId(edge_rid(&e)?))]),
            )
            .await?;
            removed += 1;
        }
        for (eid, e) in std::mem::take(&mut want.edges) {
            let cur = edges_db.remove(&eid);
            if let Some((_, c)) = &cur
                && pk_core::py::eq_map(c, &e)
            {
                continue;
            }
            if let Some((_, c)) = &cur {
                self.query("DELETE $id RETURN NONE;", vars([("id", Value::RecordId(edge_rid(c)?))]))
                    .await?;
            }
            let row = EdgeRow::from_edge(&e, ords[&('e', eid.clone())])?.content();
            self.write("INSERT RELATION [$row] RETURN id;", vars([("row", row)]), Some(1))
                .await?;
            n_edges += 1;
        }
        let nl = self.write_side_tables(laughs, index).await?;
        let mx = ords.values().copied().fold(-1.0_f64, f64::max);
        self.write(
            "UPDATE meta:state SET ord_next = $n RETURN id;",
            vars([("n", Value::Number(Number::Int(next_ord(mx))))]),
            None,
        )
        .await?;
        let info = self.make_info(ctx, li, index, nl, n_db).await?;
        let mut info = info;
        let changed = json!({"nodes": n_nodes, "edges": n_edges, "removed": removed, "decisions": 0});
        if let Some(o) = info.as_object_mut() {
            o.insert("sync_changed".into(), changed.clone());
        }
        self.write(
            "UPDATE meta:state SET load_info = $info RETURN id;",
            vars([("info", to_db(&info))]),
            None,
        )
        .await?;
        Ok(changed)
    }
}
