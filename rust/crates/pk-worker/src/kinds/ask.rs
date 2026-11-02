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
use std::sync::atomic::{AtomicU64, Ordering};
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

/// Asks started by this process: the router key's suffix.
static RUNS: AtomicU64 = AtomicU64::new(0);

/// Document id -> (sha256 of the text embedded, its vector).
type Rows = HashMap<String, (String, Arc<[f32]>)>;

#[derive(Debug, Default)]
struct Held {
    rows: Rows,
    /// The file was read (or there is none to read).
    loaded: bool,
}

/// The document vectors made so far, by document id and the hash of the text embedded: kept across jobs, pruned to
/// the current notebook on every Ask (an edited or deleted source does not stay). With a file (`PK_WORKSPACE`'s
/// `ask_vectors.{json,f32}`, the pod's emptyDir, as the Python keeps its `engine_cache` in its workspace) they also
/// survive a restart of the container: only the first Ask after the pod is created embeds the whole notebook.
///
/// One lock is held for a whole [`AskVectors::ensure`], embedding included: two Asks never embed the same text twice,
/// and one never reads a row another is replacing (the bge session is one at a time anyway).
#[derive(Debug, Default)]
pub struct AskVectors {
    held: Mutex<Held>,
    file: Option<std::path::PathBuf>,
}

/// The file pair's index: ids and text hashes, row by row of the `.f32` file.
#[derive(serde::Serialize, serde::Deserialize)]
struct Index {
    dim: usize,
    ids: Vec<String>,
    shas: Vec<String>,
}

impl AskVectors {
    /// A cache in memory only.
    pub fn new() -> Self {
        Self::default()
    }

    /// A cache kept in `<base>.json` + `<base>.f32` too (read on the first Ask, rewritten when rows change).
    pub fn with_file(base: std::path::PathBuf) -> Self {
        Self {
            held: Mutex::default(),
            file: Some(base),
        }
    }

    /// How many vectors are held.
    pub fn len(&self) -> usize {
        self.held.lock().unwrap_or_else(PoisonError::into_inner).rows.len()
    }

    /// Nothing held.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn paths(base: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        (base.with_extension("json"), base.with_extension("f32"))
    }

    /// The saved rows, or none (a missing, short or unreadable pair is ignored: the rows are made again).
    fn load(base: &std::path::Path) -> Rows {
        let (ji, jf) = Self::paths(base);
        let (Ok(index), Ok(raw)) = (std::fs::read(&ji), std::fs::read(&jf)) else {
            return Rows::new();
        };
        let Ok(ix) = serde_json::from_slice::<Index>(&index) else {
            return Rows::new();
        };
        if ix.dim != pk_embed::DIM || ix.ids.len() != ix.shas.len() || raw.len() != ix.ids.len() * ix.dim * 4 {
            return Rows::new();
        }
        let floats: Vec<f32> = raw
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        ix.ids
            .into_iter()
            .zip(ix.shas)
            .zip(floats.chunks_exact(ix.dim))
            .map(|((id, sha), v)| (id, (sha, Arc::from(v))))
            .collect()
    }

    /// Both files written next to themselves and renamed into place, the index last (it names what the rows are).
    fn save(base: &std::path::Path, rows: &Rows) -> std::io::Result<()> {
        let (ji, jf) = Self::paths(base);
        if let Some(dir) = base.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut ix = Index {
            dim: pk_embed::DIM,
            ids: Vec::with_capacity(rows.len()),
            shas: Vec::with_capacity(rows.len()),
        };
        let mut raw = Vec::with_capacity(rows.len() * pk_embed::DIM * 4);
        for (id, (sha, v)) in rows {
            ix.ids.push(id.clone());
            ix.shas.push(sha.clone());
            raw.extend(v.iter().flat_map(|x| x.to_le_bytes()));
        }
        let tmp = |p: &std::path::Path| p.with_extension("tmp");
        std::fs::write(tmp(&jf), &raw)?;
        std::fs::rename(tmp(&jf), &jf)?;
        std::fs::write(tmp(&ji), serde_json::to_vec(&ix).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp(&ji), &ji)
    }

    /// The rows for `items` (`(id, text)` in document order, flattened `n x 384`), embedding the missing ones in one
    /// call (length-sorted batches of eight, as the Python's `VectorIndex.ensure`), and how many were embedded.
    /// Blocking: run it on a blocking thread.
    pub fn ensure(&self, e: &dyn Embedder, items: &[(&str, &str)]) -> Result<(Vec<f32>, usize), JobError> {
        let keys: Vec<String> = items
            .iter()
            .map(|(_, t)| hex::encode(Sha256::digest(t.as_bytes())))
            .collect();
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if !held.loaded {
            if let Some(base) = &self.file {
                held.rows = Self::load(base);
            }
            held.loaded = true;
        }
        let missing: Vec<usize> = (0..items.len())
            .filter(|&i| held.rows.get(items[i].0).is_none_or(|(k, _)| *k != keys[i]))
            .collect();
        if !missing.is_empty() {
            let texts: Vec<&str> = missing.iter().map(|&i| items[i].1).collect();
            let fresh = e.embed(&texts, false)?;
            for (&i, v) in missing.iter().zip(fresh) {
                held.rows
                    .insert(items[i].0.to_string(), (keys[i].clone(), Arc::from(v)));
            }
        }
        let mut m = Vec::with_capacity(items.len() * pk_embed::DIM);
        for (id, _) in items {
            match held.rows.get(*id) {
                Some((_, v)) => m.extend_from_slice(v),
                None => return Err(JobError::Value(format!("no vector for {id}"))),
            }
        }
        let live: HashSet<&str> = items.iter().map(|(id, _)| *id).collect();
        let before = held.rows.len();
        held.rows.retain(|id, _| live.contains(id.as_str()));
        if let Some(base) = &self.file
            && (!missing.is_empty() || held.rows.len() != before)
            && let Err(err) = Self::save(base, &held.rows)
        {
            // the vectors are still right in memory: only the next restart re-embeds
            tracing::warn!("saving the ask vectors: {err}");
        }
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
        let decisions: Arc<[Value]> = ctx.decisions.map_or_else(|| Arc::from(Vec::new()), Arc::clone);
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
            let fts = pk_fts::Index::build(corpus.fts_rows())
                .map_err(|err| JobError::Value(format!("the full-text index: {err}")))?;
            let items: Vec<(&str, &str)> = corpus.docs.values().map(|d| (d.id.as_str(), d.embed_text())).collect();
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
        // the Python's run id (in the result); the router's key adds a per-process counter so two Asks started in the
        // same second never sum each other's tokens (the Python had a router per Ask)
        let run_id = chrono::Local::now().format("ask-%Y%m%d-%H%M%S").to_string();
        let run_key = format!("{run_id}#{}", RUNS.fetch_add(1, Ordering::Relaxed));

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
                Some(&run_key),
                None,
            )
            .await
        {
            Ok(c) => plan.update(c.data.as_ref().unwrap_or(&Value::Null)),
            // planning is an aid: retrieval still runs on the raw question (the Python catches every exception)
            Err(err) => plan.failed(&llm_message(&err)),
        }

        // 2 retrieve (the plan settles here too: one regex per label, off the async thread)
        let (kc2, q2) = (Arc::clone(&kc), question.clone());
        let (plan, picked, plan_s, retrieve_s) = tokio::task::spawn_blocking(move || -> Result<_, JobError> {
            plan.settle(&q2, &labels);
            let plan_s = secs(t0);
            let t0 = Instant::now();
            let plan2 = &plan;
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
            let picked = retrieve(corpus, kb, plan2, &q2, &search, &v);
            Ok((plan, picked, plan_s, secs(t0)))
        })
        .await
        .map_err(|err| JobError::Value(format!("ask retrieval: {err}")))??;
        timings.insert("plan_s".into(), plan_s);
        timings.insert("retrieve_s".into(), retrieve_s);

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
            .complete(ANSWER_TASK, &msgs(am), Some(&ANSWER_SCHEMA), Some(&run_key), None)
            .await?;
        timings.insert("answer_s".into(), secs(t0));

        // 4 verify: every quote through the gate against every picked source, off the async thread
        let data = c.data.clone().unwrap_or(Value::Null);
        let kc3 = Arc::clone(&kc);
        let checked = tokio::task::spawn_blocking(move || verify(&kc3.1, &kc3.0, &picked, &data))
            .await
            .map_err(|err| JobError::Value(format!("ask verify: {err}")))?;
        let (tin, tout) = ctx.router.tokens_for(&run_key);
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    /// The hash embedder, counting the texts it is asked for.
    #[derive(Default)]
    struct Counted(AtomicUsize);

    impl Embedder for Counted {
        fn embed(&self, texts: &[&str], query: bool) -> Result<Vec<Vec<f32>>, pk_embed::EmbedError> {
            self.0.fetch_add(texts.len(), Ordering::SeqCst);
            pk_embed::Hash.embed(texts, query)
        }
        fn name(&self) -> &'static str {
            "counted"
        }
    }

    fn rows(m: &[f32]) -> Vec<Vec<f32>> {
        m.chunks(pk_embed::DIM).map(<[f32]>::to_vec).collect()
    }

    #[test]
    fn a_text_is_embedded_once_and_an_edit_again() {
        let e = Counted::default();
        let c = AskVectors::new();
        let (m, n) = c.ensure(&e, &[("a", "the airport bins"), ("b", "my parrot")]).unwrap();
        assert_eq!((n, e.0.load(Ordering::SeqCst)), (2, 2));
        assert_eq!(rows(&m)[0], pk_embed::Hash::vector("the airport bins"));
        let (_, n) = c.ensure(&e, &[("a", "the airport bins"), ("b", "my parrot")]).unwrap();
        assert_eq!(n, 0, "nothing new");
        let (m, n) = c.ensure(&e, &[("b", "my parrot, edited")]).unwrap();
        assert_eq!(n, 1, "an edited text is embedded again");
        assert_eq!(rows(&m)[0], pk_embed::Hash::vector("my parrot, edited"));
        assert_eq!(c.len(), 1, "a source gone from the notebook is dropped");
    }

    #[test]
    fn the_vectors_survive_a_restart_through_the_file_and_a_bad_file_is_ignored() {
        let d = tempfile::tempdir().unwrap();
        let base = d.path().join("ws").join("ask_vectors");
        let items = [("a", "the airport bins"), ("b", "my parrot")];
        let e = Counted::default();
        let (first, _) = AskVectors::with_file(base.clone()).ensure(&e, &items).unwrap();
        let again = Counted::default();
        let (m, n) = AskVectors::with_file(base.clone()).ensure(&again, &items).unwrap();
        assert_eq!((n, again.0.load(Ordering::SeqCst)), (0, 0), "read back, not embedded");
        assert_eq!(m, first, "the same rows, bit for bit");
        std::fs::write(base.with_extension("f32"), b"short").unwrap();
        let (_, n) = AskVectors::with_file(base).ensure(&again, &items).unwrap();
        assert_eq!(n, 2, "a torn file is made again, never read as rows");
    }

    #[test]
    fn two_asks_at_once_embed_each_text_once_and_read_their_own_texts() {
        let e = Arc::new(Counted::default());
        let c = Arc::new(AskVectors::new());
        let old: Vec<(&str, &str)> = vec![("a", "the airport bins"), ("b", "my parrot")];
        let new: Vec<(&str, &str)> = vec![("a", "the airport bins, edited"), ("b", "my parrot")];
        let handles: Vec<_> = (0..8)
            .map(|k| {
                let (e, c, items) = (
                    Arc::clone(&e),
                    Arc::clone(&c),
                    if k % 2 == 0 { old.clone() } else { new.clone() },
                );
                std::thread::spawn(move || (k, c.ensure(e.as_ref(), &items).unwrap().0))
            })
            .collect();
        for h in handles {
            let (k, m) = h.join().unwrap();
            let want = if k % 2 == 0 {
                "the airport bins"
            } else {
                "the airport bins, edited"
            };
            assert_eq!(
                rows(&m)[0],
                pk_embed::Hash::vector(want),
                "each Ask reads the vector of its own text"
            );
        }
    }
}
