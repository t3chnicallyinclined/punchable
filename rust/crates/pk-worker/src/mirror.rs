//! The notebook the writing kinds read, mirrored from the API (`worker/online.py` `Workspace.refresh`), in memory
//! instead of a folder: `GET /api/worker/files` (path + sha256), then `files/content` for every file whose sha
//! moved since the last job (in path order), files gone from the listing dropped, then every decision line
//! (`decisions?after=`), then the five creative logs. The same requests in the same order as the Python, so the
//! differential compares them. The vectors are rebuilt only when a vector file changed; texts are shared (`Arc`)
//! between the mirror and a job's snapshot, never copied.

use std::collections::BTreeMap;
use std::sync::Arc;

use pk_creative::corpus::{VectorPart, Vectors};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::api::{ApiClient, ApiError, quote};
use crate::legal::clean;

/// The creative logs `pull_creative` reads.
pub const CREATIVE_STORES: [&str; 5] = ["storyboards", "workingsets", "venues", "merges", "suggestions"];
/// `LOCAL_OWNED`: gsync's own state. Once mirrored it is not fetched again nor dropped (the Python: "this PC is
/// its writer").
const LOCAL_OWNED: &str = "kb/gsync/";

#[derive(Debug, Deserialize)]
struct Listing {
    files: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct Content {
    text: String,
}

#[derive(Debug, Deserialize)]
struct Page {
    lines: Vec<Line>,
    total: usize,
}

#[derive(Debug, Deserialize)]
struct Line {
    decision: Value,
}

#[derive(Debug, Default)]
struct State {
    files: BTreeMap<String, (String, Arc<str>)>,
    vectors: Arc<Vectors>,
    vector_shas: BTreeMap<String, String>,
}

/// What one job sees.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Every mirrored file: path → text.
    pub files: BTreeMap<String, Arc<str>>,
    /// The decision lines, oldest first.
    pub decisions: Vec<Value>,
    /// Each creative log's lines (a store the API does not know is absent).
    pub creative: BTreeMap<String, Vec<String>>,
    /// The vectors.
    pub vectors: Arc<Vectors>,
    /// `online.refresh`: files fetched, files listed, decision lines.
    pub refresh: Value,
}

/// The mirror, kept across jobs.
#[derive(Debug, Default)]
pub struct Mirror {
    state: Mutex<State>,
}

fn is_vector_file(rel: &str) -> bool {
    rel.starts_with("vectors/")
}

impl Mirror {
    /// An empty mirror.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bring the mirror up to date and take a snapshot (one job at a time refreshes; the rest wait their turn).
    pub async fn refresh(&self, api: &ApiClient) -> Result<Snapshot, ApiError> {
        let mut st = self.state.lock().await;
        let listing: Listing = api
            .call_as(reqwest::Method::GET, "/api/worker/files", None::<&Value>)
            .await?
            .ok_or_else(|| ApiError::Decode("the files listing is empty".into()))?;
        // `len(listing)`: the paths as listed (before cleaning), each once
        let raw_count = listing
            .files
            .iter()
            .map(|e| e.path.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len();
        let mut listed: BTreeMap<String, String> = BTreeMap::new();
        for e in listing.files {
            if let Some(rel) = clean(&e.path) {
                listed.insert(rel, e.sha256);
            }
        }
        let mut fetched = 0usize;
        for (rel, sha) in &listed {
            let have = st.files.get(rel);
            if have.is_some_and(|(h, _)| h == sha) || (rel.starts_with(LOCAL_OWNED) && have.is_some()) {
                continue;
            }
            let path = format!("/api/worker/files/content?path={}", quote(rel));
            let c: Content = api
                .call_as(reqwest::Method::GET, &path, None::<&Value>)
                .await?
                .ok_or_else(|| ApiError::Decode(format!("{rel}: no content")))?;
            st.files.insert(rel.clone(), (sha.clone(), Arc::from(c.text)));
            fetched += 1;
        }
        st.files
            .retain(|rel, _| listed.contains_key(rel) || rel.starts_with(LOCAL_OWNED));
        let vector_shas: BTreeMap<String, String> = st
            .files
            .iter()
            .filter(|(p, _)| is_vector_file(p))
            .map(|(p, (sha, _))| (p.clone(), sha.clone()))
            .collect();
        if vector_shas != st.vector_shas {
            let texts: Vec<(String, Arc<str>)> = st
                .files
                .iter()
                .filter(|(p, _)| is_vector_file(p))
                .map(|(p, (_, t))| (p.clone(), Arc::clone(t)))
                .collect();
            // parsing a few MB of JSON floats is CPU work: off the async threads. A failed rebuild keeps the last
            // vectors and is tried again next job (the shas do not move)
            match tokio::task::spawn_blocking(move || Arc::new(build_vectors(&texts))).await {
                Ok(v) => {
                    st.vectors = v;
                    st.vector_shas = vector_shas;
                }
                Err(e) => tracing::warn!("rebuilding the vectors: {e}; the last ones stay"),
            }
        }
        let files: BTreeMap<String, Arc<str>> = st.files.iter().map(|(p, (_, t))| (p.clone(), Arc::clone(t))).collect();
        let vectors = Arc::clone(&st.vectors);
        // decisions and the creative logs touch no mirror state: another job may refresh meanwhile
        drop(st);
        let mut decisions: Vec<Value> = Vec::new();
        loop {
            let path = format!("/api/worker/decisions?after={}", decisions.len());
            let page: Page = api
                .call_as(reqwest::Method::GET, &path, None::<&Value>)
                .await?
                .ok_or_else(|| ApiError::Decode("the decisions page is empty".into()))?;
            let n = page.lines.len();
            decisions.extend(page.lines.into_iter().map(|l| l.decision));
            if n == 0 || decisions.len() >= page.total {
                break;
            }
        }
        let mut creative = BTreeMap::new();
        for store in CREATIVE_STORES {
            // an older API without that store answers 400: skipped, as the Python does
            match api.get(&format!("/api/worker/creative?store={store}")).await {
                Ok(Some(v)) => {
                    let lines = v
                        .get("lines")
                        .and_then(Value::as_array)
                        .map(|ls| ls.iter().filter_map(|l| l.as_str().map(str::to_string)).collect())
                        .unwrap_or_default();
                    creative.insert(store.to_string(), lines);
                }
                Ok(None) | Err(ApiError::Status { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(Snapshot {
            files,
            refresh: json!({"fetched": fetched, "files": raw_count, "decisions": decisions.len()}),
            decisions,
            creative,
            vectors,
        })
    }
}

/// `rebuild_vectors` + `Vectors`: the parts (non-`extra_` files under `vectors/` named `*.part*.json`) in path
/// order, the `extra_*.json` files after.
fn build_vectors(files: &[(String, Arc<str>)]) -> Vectors {
    let mut parts = Vec::new();
    let mut extras = Vec::new();
    for (p, t) in files {
        let name = p.rsplit('/').next().unwrap_or(p);
        if name.starts_with("extra_") {
            if let Ok(v) = serde_json::from_str::<Value>(t) {
                extras.push((name.to_string(), v));
            }
        } else if name.contains(".part")
            && std::path::Path::new(name).extension().is_some_and(|x| x == "json")
            && let Ok(part) = serde_json::from_str::<VectorPart>(t)
        {
            parts.push(part);
        }
    }
    Vectors::from_parts(&parts, &extras)
}
