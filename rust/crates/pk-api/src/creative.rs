//! The creative layer the hosted Python adds to a few reads (`creative_api.Creative`): the notebook
//! (`pk_creative::corpus::Corpus`) over the API's own files and decisions, and the creative stores replayed from the
//! `creative_event` rows. The hosted pod copies `srcfile` rows to its disk and builds the notebook from them
//! (`build_corpus`: the bundle's graph, its id map, `kb/v0/inferred.json`, `kb/v0/insights.json`, every set's
//! `analysis.json` and `meta.json`, never the laughs) with the database's decisions; here the same rows are read by
//! key and kept per path while their hash stays the same ([`SrcTexts`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use pk_creative::corpus::{Corpus, CorpusError, Parts, Vectors};
use pk_store::{Store, StoreError, from_db, vars};
use serde_json::{Value, json};

use crate::tape::srcfile_key;

/// `srcfile` path -> `sha256`, for the rows a generation may read the text of (under `kb/`, `vectors/` and every
/// set's `analysis.json` / `meta.json`).
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
            })
            .is_some_and(|d| !d.contains('/'))
}

/// The texts fetched so far, by `srcfile` path: `(sha256, text)`. Shared by every generation.
#[derive(Debug, Default)]
pub struct SrcTexts(Mutex<HashMap<String, (String, Arc<str>)>>);

impl SrcTexts {
    /// The texts of `want` (path, sha256): each from the cache when its hash matches, else read (one query for every
    /// miss, by key). A row gone since the generation listed it is left out, as a file the pod no longer has.
    pub async fn get(
        &self,
        store: &Store,
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
        let keys: Vec<String> = missing.iter().map(|p| srcfile_key(p)).collect();
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
        Ok(out)
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
        if p.starts_with("corpus/sets/") && wanted(p) {
            out.push((p.clone(), p.clone()));
        }
    }
    Ok(out)
}

/// `build_corpus` over the fetched texts (by `srcfile` path) and the database's decision lines.
pub fn build_corpus(
    inputs: &[(String, String)],
    texts: &BTreeMap<String, Arc<str>>,
    decisions: Vec<Value>,
    owner: &str,
) -> Result<Corpus, CorpusError> {
    let files: BTreeMap<String, Arc<str>> = inputs
        .iter()
        .filter_map(|(src, key)| texts.get(src).map(|t| (key.clone(), Arc::clone(t))))
        .collect();
    let mut parts = Parts::from_files(&files, decisions, Arc::new(Vectors::default()), owner)?;
    // `Corpus(..., laughs)` is not passed by the API's `build_corpus`
    parts.laughs.clear();
    // `self.insights = insights or {}`: an empty file reads as none
    if !pk_core::py::truthy(Some(&parts.insights)) {
        parts.insights = Value::Null;
    }
    Ok(Corpus::new(parts))
}

/// `merge.attached(store.items)` keys: the notes auto-attached under a bit, which the Desk does not list on their
/// own (`merge_api.desk_filter`). `None` when the store cannot be read or replayed: the Python then filters nothing.
pub async fn attached(store: &Store, owner: &str) -> Option<HashSet<String>> {
    let lines = store.creative_lines("merges").await.ok()?;
    let reducer = pk_creative::reducers::for_store("merges", owner)?;
    let st = pk_creative::store::Store::replay(reducer, &lines).ok()?;
    Some(
        pk_creative::merge::MergeState::from_items(st.items())
            .attached
            .into_keys()
            .collect(),
    )
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
        assert!(!wanted("corpus/sets/d/transcript.json"));
        assert!(!wanted("corpus/sets/d/x/meta.json"));
        assert!(!wanted("corpus/notes/a.json"));
    }
}
