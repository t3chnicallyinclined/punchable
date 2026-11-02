//! `ask` (`engine/ask.py` `ask`, run by `worker/execute.py`): the one-shot Ask over his whole notebook. Its own
//! corpus over `kb/v0/graph.json` and the decision log ([`pk_creative::oneshot`]), full text through `SQLite` FTS5 +
//! bm25 ([`pk_fts`]), bge vectors fused by reciprocal rank, the `ask_plan` and `ask_answer` model calls, every
//! citation through the one verbatim gate.
//!
//! The work off the async threads: the corpus and the FTS table are built, the documents and the queries embedded
//! and the ranking run in `spawn_blocking`. A document's vector is made once per text and kept for the worker's
//! life ([`AskVectors`]; the Python keeps them under its disposable workspace), so only the first Ask after a start
//! embeds the whole notebook. No voice profile reaches these two calls (the Python's `ask` builds its own router
//! and never reads one).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use pk_creative::corpus::{CorpusError, round_to};
use pk_creative::oneshot::{
    ANSWER_SCHEMA, AskCorpus, Kb, NOTHING, PLAN_SCHEMA, Plan, Vecs, answer_messages, plan_messages, retrieve, verify,
};
use pk_embed::Embedder;
use pk_llm::ChatMessage;
use pk_wire::worker::AskInput;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::{Ctx, Done, Runner};
use crate::error::{JobError, llm_message};

/// The planner's task in `engine/config.hosted.toml`.
pub const PLAN_TASK: &str = "ask_plan";
/// The answer's task.
pub const ANSWER_TASK: &str = "ask_answer";
/// The graph the one-shot Ask reads (`kbstore.KB`: `<kb_dir>/graph.json`, `kb_dir = {comedy}/kb/v0`).
pub const GRAPH: &str = "kb/v0/graph.json";

/// Document id -> (sha256 of the text embedded, its vector).
type Rows = HashMap<String, (String, Arc<[f32]>)>;

/// The document vectors made so far, by document id and the hash of the text embedded: kept across jobs, pruned to
/// the current notebook on every Ask (an edited or deleted source does not stay in memory).
#[derive(Debug, Default)]
pub struct AskVectors {
    rows: Mutex<Rows>,
}

impl AskVectors {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many vectors are held.
    pub fn len(&self) -> usize {
        self.rows.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    /// Nothing held.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The rows for `items` (`(id, text)` in document order), embedding the missing ones in one call (length-sorted
    /// batches of eight, as the Python's `VectorIndex.ensure`). Blocking: run it on a blocking thread.
    pub fn ensure(&self, e: &dyn Embedder, items: &[(String, String)]) -> Result<(Vec<f32>, usize), JobError> {
        let key = |t: &str| hex::encode(Sha256::digest(t.as_bytes()));
        let keys: Vec<String> = items.iter().map(|(_, t)| key(t)).collect();
        let missing: Vec<usize> = {
            let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
            (0..items.len())
                .filter(|&i| rows.get(&items[i].0).is_none_or(|(k, _)| *k != keys[i]))
                .collect()
        };
        let fresh = if missing.is_empty() {
            Vec::new()
        } else {
            let texts: Vec<&str> = missing.iter().map(|&i| items[i].1.as_str()).collect();
            e.embed(&texts, false)?
        };
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        for (&i, v) in missing.iter().zip(fresh) {
            rows.insert(items[i].0.clone(), (keys[i].clone(), Arc::from(v)));
        }
        let mut m = Vec::with_capacity(items.len() * pk_embed::DIM);
        for (id, _) in items {
            match rows.get(id) {
                Some((_, v)) => m.extend_from_slice(v),
                None => return Err(JobError::Value(format!("no vector for {id}"))),
            }
        }
        let live: HashSet<&str> = items.iter().map(|(id, _)| id.as_str()).collect();
        rows.retain(|id, _| live.contains(id.as_str()));
        Ok((m, missing.len()))
    }
}

/// The `ask` kind.
#[derive(Debug)]
pub struct Ask;

fn msgs(pair: [(&'static str, String); 2]) -> Vec<ChatMessage> {
    pair.into_iter().map(|(r, c)| ChatMessage::new(r, c)).collect()
}

fn secs(t: Instant) -> Value {
    json!(round_to(t.elapsed().as_secs_f64(), 2))
}

impl Runner for Ask {
    type Input = AskInput;

    fn bad_input(e: &serde_json::Error) -> JobError {
        JobError::Value(format!("bad ask input: {e}"))
    }

    async fn run(ctx: Ctx<'_>, input: AskInput) -> Result<Done, JobError> {
        let t_all = Instant::now();
        let question = input.question;
        let files = ctx
            .files
            .ok_or_else(|| JobError::Value("the ask kind needs the notebook and none was mirrored".into()))?;
        let graph = files
            .get(GRAPH)
            .cloned()
            .ok_or_else(|| CorpusError::Missing(GRAPH.into()))?;
        let decisions: Vec<Value> = ctx.decisions.map(<[Value]>::to_vec).unwrap_or_default();
        let model = ctx
            .embedder
            .ok_or_else(|| JobError::Value("no bge-small model in this worker".into()))?
            .get()
            .await?;
        let (owner, cache) = (ctx.owner.to_string(), Arc::clone(ctx.ask_vectors));
        let tx_files: Vec<(String, Arc<str>)> = files
            .iter()
            .filter(|(p, _)| p.starts_with("corpus/sets/"))
            .map(|(p, t)| (p.clone(), Arc::clone(t)))
            .collect();

        // 0 the index: corpus, full text, document vectors
        let t0 = Instant::now();
        let e = Arc::clone(&model);
        let built = tokio::task::spawn_blocking(move || -> Result<_, JobError> {
            let g: Value = serde_json::from_str(&graph).map_err(|err| CorpusError::Json {
                path: GRAPH.into(),
                msg: err.to_string(),
            })?;
            let kb = Kb::new(&g, &decisions, &owner);
            let files: std::collections::BTreeMap<String, Arc<str>> = tx_files.into_iter().collect();
            let corpus = AskCorpus::build(&kb, &files)?;
            if corpus.docs.is_empty() {
                // numpy's np.stack over no rows: the Python's Corpus cannot be built either
                return Err(JobError::Value("need at least one array to stack".into()));
            }
            let rows = corpus.fts_rows();
            let fts = pk_fts::Index::build(rows.iter().map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str())))
                .map_err(|err| JobError::Value(format!("the full-text index: {err}")))?;
            let items: Vec<(String, String)> = corpus.docs.values().map(|d| (d.id.clone(), d.embed_text())).collect();
            let (m, embedded) = cache.ensure(e.as_ref(), &items)?;
            Ok((Arc::new((kb, corpus)), fts, m, embedded))
        })
        .await
        .map_err(|err| JobError::Value(format!("building the ask index: {err}")))??;
        let (kc, fts, m, embedded) = built;
        tracing::info!(
            docs = kc.1.docs.len(),
            embedded,
            ms = t0.elapsed().as_millis(),
            "ask index"
        );
        let mut timings = Map::new();
        timings.insert("index_s".into(), secs(t0));
        let run_id = chrono::Local::now().format("ask-%Y%m%d-%H%M%S").to_string();
        let n0 = ctx.router.calls().len();

        // 1 plan
        let t0 = Instant::now();
        let labels = kc.0.labels();
        let mut plan = Plan::new(&question);
        match ctx
            .router
            .complete(
                PLAN_TASK,
                &msgs(plan_messages(&question, &labels)),
                Some(&PLAN_SCHEMA),
                Some(&run_id),
                None,
            )
            .await
        {
            Ok(c) => plan.update(c.data.as_ref().unwrap_or(&Value::Null)),
            // planning is an aid: retrieval still runs on the raw question (the Python catches every exception)
            Err(err) => plan.failed(&llm_message(&err)),
        }
        plan.settle(&question, &labels);
        timings.insert("plan_s".into(), secs(t0));

        // 2 retrieve
        let t0 = Instant::now();
        let (kc2, plan2, q2) = (Arc::clone(&kc), plan.clone(), question.clone());
        let picked = tokio::task::spawn_blocking(move || -> Result<Vec<String>, JobError> {
            let mut texts: Vec<&str> = plan2.search_queries.iter().map(String::as_str).collect();
            texts.push(&q2);
            texts.extend(plan2.joke_titles.iter().map(String::as_str));
            let mut queries: HashMap<String, Vec<f32>> = HashMap::new();
            for t in texts {
                if !queries.contains_key(t) {
                    // one query per call, as `Embedder.query` runs them (a batch would pad them together)
                    let v = model.embed(&[t], true)?.pop().unwrap_or_default();
                    queries.insert(t.to_string(), v);
                }
            }
            let (kb, corpus) = (&kc2.0, &kc2.1);
            let search = |expr: &str| fts.search(expr, pk_creative::oneshot::K).unwrap_or_default();
            let v = Vecs {
                docs: &m,
                queries: &queries,
            };
            Ok(retrieve(corpus, kb, &plan2, &q2, &search, &v))
        })
        .await
        .map_err(|err| JobError::Value(format!("ask retrieval: {err}")))??;
        timings.insert("retrieve_s".into(), secs(t0));

        let (kb, corpus) = (&kc.0, &kc.1);
        let mut out = Map::new();
        out.insert("question".into(), json!(question));
        out.insert("plan".into(), plan.to_value());
        out.insert("run_id".into(), json!(run_id));
        out.insert("sources".into(), json!(picked));
        if picked.is_empty() {
            out.insert("found".into(), json!(false));
            out.insert("answer".into(), json!(NOTHING));
            out.insert("citations".into(), json!([]));
            out.insert("removed".into(), json!([]));
            out.insert("timings".into(), Value::Object(timings));
            return Ok(Done::result(Value::Object(out)));
        }

        // 3 answer
        let t0 = Instant::now();
        let am = answer_messages(corpus, kb, &question, &picked, plan.by_year, &ctx.names.raw);
        let c = ctx
            .router
            .complete(ANSWER_TASK, &msgs(am), Some(&ANSWER_SCHEMA), Some(&run_id), None)
            .await?;
        timings.insert("answer_s".into(), secs(t0));

        // 4 verify
        let checked = verify(corpus, kb, &picked, c.data.as_ref().unwrap_or(&Value::Null));
        let (mut tin, mut tout) = (0u64, 0u64);
        for rec in ctx.router.calls().iter().skip(n0) {
            if rec.run_id.as_deref() == Some(run_id.as_str()) {
                tin += rec.tokens_in;
                tout += rec.tokens_out;
            }
        }
        timings.insert("total_s".into(), secs(t_all));
        out.insert("found".into(), json!(checked.found));
        out.insert("answer".into(), json!(checked.answer));
        out.insert("not_found".into(), checked.not_found);
        out.insert("citations".into(), Value::Array(checked.citations));
        out.insert("removed".into(), Value::Array(checked.removed));
        out.insert("timings".into(), Value::Object(timings));
        out.insert("tokens_in".into(), json!(tin));
        out.insert("tokens_out".into(), json!(tout));
        out.insert("provider".into(), json!(format!("{}:{}", c.provider, c.model)));
        Ok(Done::result(Value::Object(out)))
    }
}
