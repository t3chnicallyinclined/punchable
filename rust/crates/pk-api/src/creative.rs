//! The creative layer the hosted Python adds to a few reads (`creative_api.Creative`): the notebook
//! (`pk_creative::corpus::Corpus`) over the API's own files and decisions, and the creative stores replayed from the
//! `creative_event` rows. The hosted pod copies `srcfile` rows to its disk and builds the notebook from them
//! (`build_corpus`: the bundle's graph, its id map, `kb/v0/inferred.json`, `kb/v0/insights.json`, every set's
//! `analysis.json` and `meta.json`, never the laughs) with the database's decisions; here the same rows are read by
//! key and kept per path while their hash stays the same ([`SrcTexts`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use pk_creative::Items;
use pk_creative::corpus::{Corpus, CorpusError, Parts, Vectors};
use pk_store::{Store, StoreError, from_db, vars};
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;

use crate::tape::srcfile_key;

/// `srcfile` path -> `sha256`, for the rows a generation may read the text of (under `kb/`, `vectors/` and every
/// set's `analysis.json` / `meta.json` / `clips.json`).
pub type SrcShas = BTreeMap<String, String>;

/// Whether a scanned `srcfile` path is one the creative layer reads.
pub fn wanted(path: &str) -> bool {
    path.starts_with("kb/")
        || path.starts_with("vectors/")
        || path
            .strip_prefix("corpus/sets/")
            .and_then(|p| {
                p.strip_suffix("/analysis.json")
                    .or_else(|| p.strip_suffix("/meta.json"))
                    .or_else(|| p.strip_suffix("/clips.json"))
            })
            .is_some_and(|d| !d.contains('/'))
}

/// How many `srcfile` texts one query reads (the graph alone can be megabytes).
const READ_CHUNK: usize = 8;

/// The texts fetched so far, by `srcfile` path: `(sha256, text)`. Shared by every generation; a path's entry is
/// replaced when its hash moves, so it holds one copy of each file the notebook reads (what the Python pod keeps on
/// its disk).
#[derive(Debug, Default)]
pub struct SrcTexts(Mutex<HashMap<String, (String, Arc<str>)>>);

impl SrcTexts {
    /// The texts of `want` (path, sha256): each from the cache when its hash matches, else read by key, a few rows per
    /// query so the response held at once stays small (the hosted Python reads one row at a time for the same
    /// reason). A row gone since the generation listed it is left out, as a file the pod no longer has. With no
    /// database, only a miss is an error.
    pub async fn get(
        &self,
        store: Option<&Store>,
        want: &[(String, String)],
    ) -> Result<BTreeMap<String, Arc<str>>, StoreError> {
        let mut out = BTreeMap::new();
        let mut missing: Vec<&str> = Vec::new();
        {
            let cache = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            for (p, sha) in want {
                match cache.get(p) {
                    Some((h, t)) if h == sha => {
                        out.insert(p.clone(), Arc::clone(t));
                    }
                    _ => missing.push(p),
                }
            }
        }
        if missing.is_empty() {
            return Ok(out);
        }
        let store = store.ok_or_else(|| StoreError::mapping("the database is not connected"))?;
        for chunk in missing.chunks(READ_CHUNK) {
            let keys: Vec<String> = chunk.iter().map(|p| srcfile_key(p)).collect();
            let rows = store
                .last(
                    "LET $ids = array::map($ks, |$k| type::record('srcfile', $k));\
                     SELECT path, sha256, text FROM $ids;",
                    vars([("ks", pk_store::to_db(&json!(keys)))]),
                )
                .await?;
            let mut cache = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            for r in pk_store::store::rows_of(rows).into_iter().map(from_db) {
                let (Some(p), Some(h), Some(t)) = (
                    r.get("path").and_then(Value::as_str),
                    r.get("sha256").and_then(Value::as_str),
                    r.get("text").and_then(Value::as_str),
                ) else {
                    continue;
                };
                let t: Arc<str> = Arc::from(t);
                cache.insert(p.to_string(), (h.to_string(), Arc::clone(&t)));
                out.insert(p.to_string(), t);
            }
        }
        Ok(out)
    }
}

impl SrcTexts {
    /// The texts of `want` read by key and NOT kept (the vector parts: megabytes of JSON floats that are parsed once
    /// and dropped). A row gone since the generation listed it is left out.
    pub async fn read_uncached(
        store: Option<&Store>,
        want: &[(String, String)],
    ) -> Result<Vec<(String, String, Arc<str>)>, StoreError> {
        let store = store.ok_or_else(|| StoreError::mapping("the database is not connected"))?;
        let mut out = Vec::new();
        for chunk in want.chunks(READ_CHUNK) {
            let keys: Vec<String> = chunk.iter().map(|(p, _)| srcfile_key(p)).collect();
            let rows = store
                .last(
                    "LET $ids = array::map($ks, |$k| type::record('srcfile', $k));\
                     SELECT path, sha256, text FROM $ids;",
                    vars([("ks", pk_store::to_db(&json!(keys)))]),
                )
                .await?;
            for r in pk_store::store::rows_of(rows).into_iter().map(from_db) {
                if let (Some(p), Some(h), Some(t)) = (
                    r.get("path").and_then(Value::as_str),
                    r.get("sha256").and_then(Value::as_str),
                    r.get("text").and_then(Value::as_str),
                ) {
                    out.push((p.to_string(), h.to_string(), Arc::from(t)));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }
}

/// The notebook's vectors (`Creative.vectors()`: rebuilt when the vector files change), keyed by the exact
/// `vectors/` rows (path and hash) they were parsed from. A generation asks with its own rows and gets vectors parsed
/// from exactly those (never an older set); parsing happens once per change, under a lock, so two generations never
/// parse the same rows twice.
#[derive(Debug, Default)]
pub struct VectorCache(AsyncMutex<Option<ParsedVectors>>);

/// The `vectors/` rows (path, sha256) a set of vectors was parsed from, and the vectors.
type ParsedVectors = (Vec<(String, String)>, Arc<Vectors>);

impl VectorCache {
    /// The vectors of the `vectors/` rows in `files`.
    pub async fn get(&self, store: Option<&Store>, files: &SrcShas) -> Result<Arc<Vectors>, StoreError> {
        let want: Vec<(String, String)> = files
            .iter()
            .filter(|(p, _)| p.starts_with("vectors/"))
            .map(|(p, h)| (p.clone(), h.clone()))
            .collect();
        let mut slot = self.0.lock().await;
        if let Some((k, v)) = &*slot
            && *k == want
        {
            return Ok(Arc::clone(v));
        }
        if want.is_empty() {
            let v = Arc::new(Vectors::default());
            *slot = Some((want, Arc::clone(&v)));
            return Ok(v);
        }
        let rows = SrcTexts::read_uncached(store, &want).await?;
        // the rows read are exactly the ones listed (a part rewritten or removed since the listing is not)
        let exact = rows.len() == want.len() && rows.iter().zip(&want).all(|((p, h, _), (wp, wh))| p == wp && h == wh);
        let texts: Vec<(String, Arc<str>)> = rows.into_iter().map(|(p, _, t)| (p, t)).collect();
        // parsing megabytes of JSON floats is CPU work: off the async threads
        let vectors = tokio::task::spawn_blocking(move || Arc::new(Vectors::from_files(&texts)))
            .await
            .map_err(|e| StoreError::mapping(format!("parsing the vectors: {e}")))?;
        if exact {
            *slot = Some((want, Arc::clone(&vectors)));
        } else {
            // the newest rows, as the Python reads its disk now; not kept under a listing they do not match, so the
            // next generation (whose listing names them) reads again
            tracing::info!("vectors: the rows moved since the listing; parsed what is there, not cached");
        }
        Ok(vectors)
    }
}

/// The parent folder of a `srcfile` path (`kb/v0/graph.json` -> `kb/v0`).
fn parent(p: &str) -> &str {
    p.rsplit_once('/').map_or("", |(d, _)| d)
}

/// What `Creative._inputs` + `build_corpus` read, as `(srcfile path, key)` pairs (the key is the path `Parts::from_files` reads it under):
/// the bundle's graph (`load_info.bundle.graph`), its id map (`bundle.id_map`, else `id_map.json` next to the graph
/// when there is one), `kb/v0/inferred.json`, `kb/v0/insights.json`, and every set's `analysis.json` and
/// `meta.json`. Never the laughs (`build_corpus` passes none).
pub fn corpus_inputs(bundle: &Value, files: &SrcShas) -> Result<Vec<(String, String)>, CorpusError> {
    let graph = bundle
        .get("graph")
        .and_then(Value::as_str)
        .ok_or_else(|| CorpusError::Missing("load_info.bundle.graph".into()))?;
    let mut out = vec![(graph.to_string(), "kb/v1/graph.json".to_string())];
    let id_map = if let Some(p) = bundle.get("id_map").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(p.to_string())
    } else {
        let p = format!("{}/id_map.json", parent(graph));
        files.contains_key(&p).then_some(p)
    };
    if let Some(p) = id_map {
        out.push((p, "kb/v1/id_map.json".into()));
    }
    for p in ["kb/v0/inferred.json", "kb/v0/insights.json"] {
        out.push((p.into(), p.into()));
    }
    for p in files.keys() {
        if p.starts_with("corpus/sets/") && wanted(p) && (p.ends_with("/analysis.json") || p.ends_with("/meta.json")) {
            out.push((p.clone(), p.clone()));
        }
    }
    Ok(out)
}

/// `build_corpus` over the fetched texts (by `srcfile` path), the database's decision lines and the vectors the
/// Python's corpus carries (`Creative.vectors()`, from the `vectors/` rows: the story layer's subject links and the
/// connections' tag query read them).
pub fn build_corpus(
    inputs: &[(String, String)],
    texts: &BTreeMap<String, Arc<str>>,
    decisions: Vec<Value>,
    vectors: Arc<Vectors>,
    owner: &str,
) -> Result<Corpus, CorpusError> {
    let files: BTreeMap<String, Arc<str>> = inputs
        .iter()
        .filter_map(|(src, key)| texts.get(src).map(|t| (key.clone(), Arc::clone(t))))
        .collect();
    let mut parts = Parts::from_files(&files, decisions, vectors, owner)?;
    // `Corpus(..., laughs)` is not passed by the API's `build_corpus`
    parts.laughs.clear();
    // `self.insights = insights or {}`: an empty file reads as none
    if !pk_core::py::truthy(Some(&parts.insights)) {
        parts.insights = Value::Null;
    }
    Ok(Corpus::new(parts))
}

/// One generation's notebook with the story layer's caches (`_subject_idx`, `_cos_band`, `_joke_vec`, which the
/// Python keeps on its corpus object), built and dropped together.
pub type GenCorpus = pk_creative::story::StoryCorpus;

/// One creative store's items as last replayed from its `creative_event` lines. Creative writes do not move the
/// generation, so the lines are read on every request (as the Python's `Creative.stores` does) and replayed again
/// only when they differ from the last replay's: the items handed out are always those of the lines just read.
#[derive(Debug)]
pub struct Replayed {
    name: &'static str,
    last: Mutex<Option<(Vec<String>, Arc<Items>)>>,
}

impl Replayed {
    /// For the store `name` (a key of `pk_creative::reducers::for_store`).
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            last: Mutex::new(None),
        }
    }

    /// The store's items now. `Err` is a text in the Python's `creative_error` style: no database, a read that
    /// failed, or lines its reducer refuses.
    pub async fn items(&self, store: Option<&Store>, owner: &str) -> Result<Arc<Items>, String> {
        let store = store.ok_or("ConnectionError: the database is not connected")?;
        let lines = store
            .creative_lines(self.name)
            .await
            .map_err(|e| format!("HttpError: {e}"))?;
        self.replay(lines, owner)
    }

    /// Keep `items` as the replay of `lines` (a write just appended the last line and has the replay in hand).
    pub fn store(&self, lines: Vec<String>, items: Arc<Items>) {
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = Some((lines, items));
    }

    /// The items of `lines`: the last replay's when they are the same lines, else a new replay (kept for the next
    /// reader). The replay runs on the caller's thread: the stores are small, append-only logs.
    pub fn replay(&self, lines: Vec<String>, owner: &str) -> Result<Arc<Items>, String> {
        if let Some((l, items)) = &*self.last.lock().unwrap_or_else(PoisonError::into_inner)
            && *l == lines
        {
            return Ok(Arc::clone(items));
        }
        let reducer =
            pk_creative::reducers::for_store(self.name, owner).ok_or_else(|| format!("KeyError: '{}'", self.name))?;
        let st = pk_creative::store::Store::replay(reducer, &lines).map_err(|e| format!("ValueError: {e}"))?;
        let items = Arc::new(st.items().clone());
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = Some((lines, Arc::clone(&items)));
        Ok(items)
    }
}

/// The creative stores the reads here replay, each with its last replay.
#[derive(Debug)]
pub struct Stores {
    /// His account (`settings`).
    pub account: Replayed,
    /// The set lists (boards).
    pub storyboards: Replayed,
    /// His sets (working sets).
    pub workingsets: Replayed,
    /// His answers to "not a joke?".
    pub suggestions: Replayed,
    /// The job queue.
    pub jobs: Replayed,
    /// His voice profile.
    pub profile: Replayed,
    /// His merges, the AI's attaches, the pairs he dismissed.
    pub merges: Replayed,
    /// The rooms he pinned (the worker reads them; the venue routes are the Python's until group 6).
    pub venues: Replayed,
    /// His conversations with his notebook (Ask).
    pub chats: Replayed,
    /// His rant topics (the Rant tab; Rust only).
    pub topics: Replayed,
    /// Where each line of a rant came from, per version (the Rant tab's script; Rust only).
    pub script: Replayed,
}

impl Default for Stores {
    fn default() -> Self {
        Self {
            account: Replayed::new("account"),
            storyboards: Replayed::new("storyboards"),
            workingsets: Replayed::new("workingsets"),
            suggestions: Replayed::new("suggestions"),
            jobs: Replayed::new("jobs"),
            profile: Replayed::new("profile"),
            merges: Replayed::new("merges"),
            venues: Replayed::new("venues"),
            chats: Replayed::new("chats"),
            topics: Replayed::new("topics"),
            script: Replayed::new("script"),
        }
    }
}

impl Stores {
    /// The store by its `creative_event` name, for the writes (`None` for a store the reads here never replay).
    pub fn by_name(&self, name: &str) -> Option<&Replayed> {
        Some(match name {
            "account" => &self.account,
            "storyboards" => &self.storyboards,
            "workingsets" => &self.workingsets,
            "suggestions" => &self.suggestions,
            "jobs" => &self.jobs,
            "profile" => &self.profile,
            "merges" => &self.merges,
            "venues" => &self.venues,
            "chats" => &self.chats,
            "topics" => &self.topics,
            "script" => &self.script,
            _ => return None,
        })
    }
}

/// The merges store's lines and the notes attached in them, as last replayed. Creative writes do not move the
/// generation, so the lines are read on every request, as the Python reads them; the replay is redone only when
/// they changed.
#[derive(Debug, Default)]
pub struct Attached(Mutex<Option<AttachedLines>>);

/// The lines replayed and the attached notes they give.
type AttachedLines = (Vec<String>, Arc<HashSet<String>>);

impl Attached {
    /// `merge.attached(store.items)` keys: the notes auto-attached under a bit, which the Desk does not list on their
    /// own (`merge_api.desk_filter`). `None` with no database, or when the store cannot be read or replayed: the
    /// Python then filters nothing (its `except Exception: return items`).
    pub async fn get(&self, store: Option<&Store>, owner: &str) -> Option<Arc<HashSet<String>>> {
        let lines = match store?.creative_lines("merges").await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("desk_filter: reading the merges store: {e}; nothing filtered");
                return None;
            }
        };
        if let Some((l, att)) = &*self.0.lock().unwrap_or_else(PoisonError::into_inner)
            && *l == lines
        {
            return Some(Arc::clone(att));
        }
        let reducer = pk_creative::reducers::for_store("merges", owner)?;
        let att: Arc<HashSet<String>> = match pk_creative::store::Store::replay(reducer, &lines) {
            Ok(st) => Arc::new(
                pk_creative::merge::MergeState::from_items(st.items())
                    .attached
                    .into_keys()
                    .collect(),
            ),
            Err(e) => {
                tracing::warn!("desk_filter: replaying the merges store: {e}; nothing filtered");
                return None;
            }
        };
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some((lines, Arc::clone(&att)));
        Some(att)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(ps: &[&str]) -> SrcShas {
        ps.iter().map(|p| ((*p).to_string(), "h".to_string())).collect()
    }

    #[test]
    fn the_inputs_are_the_python_paths() {
        let f = files(&[
            "kb/v0/graph.json",
            "kb/v0/id_map.json",
            "corpus/sets/20240101_x/analysis.json",
            "corpus/sets/20240101_x/meta.json",
            "corpus/sets/20240101_x/laughs_auto.json",
            "corpus/sets/20240101_x/transcript.json",
            "corpus/sets/20240101_x/clips.json",
        ]);
        let got = corpus_inputs(&json!({"graph": "kb/v0/graph.json"}), &f).unwrap();
        let keys: Vec<&str> = got.iter().map(|(_, k)| k.as_str()).collect();
        assert_eq!(
            keys,
            [
                "kb/v1/graph.json",
                "kb/v1/id_map.json",
                "kb/v0/inferred.json",
                "kb/v0/insights.json",
                "corpus/sets/20240101_x/analysis.json",
                "corpus/sets/20240101_x/meta.json"
            ]
        );
        assert_eq!(got[1].0, "kb/v0/id_map.json");
        // the bundle's own id map wins; no id map anywhere is none
        let got = corpus_inputs(&json!({"graph": "kb/v1/graph.json", "id_map": "kb/v1/ids.json"}), &f).unwrap();
        assert_eq!(got[1], ("kb/v1/ids.json".into(), "kb/v1/id_map.json".into()));
        let got = corpus_inputs(&json!({"graph": "kb/v1/graph.json"}), &files(&[])).unwrap();
        assert_eq!(got.len(), 3);
        assert!(corpus_inputs(&json!({}), &f).is_err());
    }

    #[test]
    fn wanted_is_kb_vectors_and_the_set_files_the_notebook_reads() {
        assert!(wanted("kb/v0/graph.json"));
        assert!(wanted("vectors/vectors.f32.part000.json"));
        assert!(wanted("corpus/sets/d/analysis.json"));
        assert!(wanted("corpus/sets/d/meta.json"));
        assert!(wanted("corpus/sets/d/clips.json"));
        assert!(!wanted("corpus/sets/d/transcript.json"));
        assert!(!wanted("corpus/sets/d/x/meta.json"));
        assert!(!wanted("corpus/notes/a.json"));
    }
}
