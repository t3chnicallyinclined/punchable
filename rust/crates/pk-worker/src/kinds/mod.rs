//! The job kinds the hosted worker runs (`worker/__main__.py` `HOSTED_KINDS` + `HOSTED_EMBED_KINDS`), each a
//! [`Runner`] with its typed input, dispatched by one exhaustive `match`: a kind added to [`Kind`] does not compile
//! until it is handled. All eleven are ported (`docs/RUST_WORKSTREAM.md` T8); the kinds that read his notebook mirror
//! it first, and all but `ask` build the `creative/data.py` `Corpus` from it (`ask` reads the v0 graph instead).

pub mod ask;
pub mod ask_chat;
pub mod bridge;
pub mod embed;
pub mod facts;
pub mod memolink;
pub mod punchup;
pub mod rant;
pub mod rant_draft;
pub mod rant_line;
pub mod riff;
pub mod set_text;

use std::collections::BTreeMap;
use std::sync::Arc;

use pk_creative::corpus::Corpus;
use pk_embed::Embedder;
use pk_llm::Router;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::api::ApiClient;
use crate::error::JobError;
use crate::legal::Legal;
use crate::voice::{Names, Voice};

pub use facts::Pages;

/// Every kind a hosted worker may be asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A bridge line between two bits.
    Bridge,
    /// Punch-up drafts for a bit.
    Punchup,
    /// Checkable facts for an idea.
    Facts,
    /// Riff lines for an idea.
    Riff,
    /// One turn of an Ask conversation.
    AskChat,
    /// A set transcribed on the phone.
    SetText,
    /// Vectors for new material.
    Embed,
    /// Memo-to-bit linking.
    Memolink,
    /// The older one-shot Ask.
    Ask,
    /// Film it's "Make it a rant" on one version.
    Rant,
    /// Film it's "Pick a hook" on one version.
    Hook,
    /// The Rant tab's "Write a rant": a rant drafted from a topic and the sources he ticked (Rust only, R7).
    RantDraft,
    /// The Rant tab's script: three versions of one line (Rust only, R7).
    RantLine,
}

/// The kinds that need audio; a hosted worker never asks for them and hands one back if given it.
pub const TRANSCRIBER_KINDS: [&str; 2] = ["process_set", "import_youtube"];

impl Kind {
    /// `HOSTED_KINDS`, in the Python's order.
    pub const HOSTED: [Kind; 8] = [
        Kind::Bridge,
        Kind::Punchup,
        Kind::Facts,
        Kind::Riff,
        Kind::AskChat,
        Kind::SetText,
        Kind::Rant,
        Kind::Hook,
    ];
    /// `HOSTED_EMBED_KINDS`: only with the bge model in the image.
    pub const HOSTED_EMBED: [Kind; 3] = [Kind::Embed, Kind::Memolink, Kind::Ask];
    /// Kinds only this worker has (the Rant tab, `docs/design/SPEC-2026-11-06-rant-tab.md` R7: new server work is
    /// written once, in Rust). Claimed after the Python's kinds, so the differential's claim bodies are the Python's
    /// with these appended.
    pub const RUST_ONLY: [Kind; 2] = [Kind::RantDraft, Kind::RantLine];

    /// The wire name.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Bridge => "bridge",
            Kind::Punchup => "punchup",
            Kind::Facts => "facts",
            Kind::Riff => "riff",
            Kind::AskChat => "ask_chat",
            Kind::SetText => "set_text",
            Kind::Embed => "embed",
            Kind::Memolink => "memolink",
            Kind::Ask => "ask",
            Kind::Rant => "rant",
            Kind::Hook => "hook",
            Kind::RantDraft => "rant_draft",
            Kind::RantLine => "rant_line",
        }
    }

    /// The kind with that wire name.
    pub fn parse(s: &str) -> Option<Kind> {
        Self::HOSTED
            .into_iter()
            .chain(Self::HOSTED_EMBED)
            .chain(Self::RUST_ONLY)
            .find(|k| k.name() == s)
    }

    /// Whether this worker can run it yet.
    pub fn ported(self) -> bool {
        match self {
            Kind::AskChat
            | Kind::Facts
            | Kind::SetText
            | Kind::Riff
            | Kind::Punchup
            | Kind::Bridge
            | Kind::Embed
            | Kind::Memolink
            | Kind::Ask
            | Kind::Rant
            | Kind::Hook
            | Kind::RantDraft
            | Kind::RantLine => true,
        }
    }

    /// Whether it reads the `creative/data.py` `Corpus` (the v1 graph): every notebook kind but `ask`, whose own
    /// corpus is the v0 graph (`engine/ask.py`), so a notebook without `kb/v1/graph.json` still answers an Ask.
    pub fn builds_corpus(self) -> bool {
        self.reads_notebook() && self != Kind::Ask
    }

    /// Whether it reads his notebook (the worker mirrors it and builds the `Corpus` first, as the Python's
    /// `run_job` refreshes its workspace).
    pub fn reads_notebook(self) -> bool {
        match self {
            Kind::Riff | Kind::Punchup | Kind::Bridge | Kind::Embed | Kind::Memolink | Kind::Ask => true,
            Kind::AskChat
            | Kind::Facts
            | Kind::SetText
            | Kind::Rant
            | Kind::Hook
            | Kind::RantDraft
            | Kind::RantLine => false,
        }
    }

    /// `online.ACTOR`: who the files and ideas a job posts are by.
    pub fn actor(self) -> &'static str {
        match self {
            Kind::Bridge => "ai:bridge",
            Kind::Punchup => "ai:punchup",
            Kind::Facts => "ai:facts",
            Kind::Riff => "ai:riff",
            Kind::AskChat => "ai:ask_chat",
            Kind::SetText => "ai:set_text",
            Kind::Embed => "ai:embed",
            Kind::Memolink => "ai:memolink",
            Kind::Ask => "ai:ask",
            Kind::Rant => "ai:rant",
            Kind::Hook => "ai:hook",
            Kind::RantDraft => "ai:rant_draft",
            Kind::RantLine => "ai:rant_line",
        }
    }

    /// The kinds the claim asks for: the hosted kinds (and the embedding kinds with a model), the ported ones only,
    /// then the Rust-only kinds.
    pub fn claimable(with_bge: bool) -> Vec<Kind> {
        let embed: &[Kind] = if with_bge { &Self::HOSTED_EMBED } else { &[] };
        Self::HOSTED
            .iter()
            .chain(embed)
            .chain(&Self::RUST_ONLY)
            .copied()
            .filter(|k| k.ported())
            .collect()
    }
}

/// The bge-small model behind the embedding kinds, loaded on the first job that needs it (the Python's `Embedder`
/// loads on first use too): a worker that never embeds never holds the model, and a model file that will not load
/// fails only the embed / memolink job that asked, never the worker. A failed load is not kept: the next job tries
/// again. Cheap to clone (one shared cell).
#[derive(Clone)]
pub struct Model(Arc<ModelCell>);

struct ModelCell {
    dir: Option<std::path::PathBuf>,
    cell: tokio::sync::OnceCell<Arc<dyn Embedder>>,
}

impl Model {
    /// The model in `dir`, loaded when a job first asks (one intra-op thread: the pod has one CPU).
    pub fn lazy(dir: std::path::PathBuf) -> Self {
        Self(Arc::new(ModelCell {
            dir: Some(dir),
            cell: tokio::sync::OnceCell::new(),
        }))
    }

    /// A model already loaded (tests: the real model shared across scenarios, or a hash embedder).
    pub fn loaded(e: Arc<dyn Embedder>) -> Self {
        Self(Arc::new(ModelCell {
            dir: None,
            cell: tokio::sync::OnceCell::new_with(Some(e)),
        }))
    }

    /// Whether it is in memory now.
    pub fn is_loaded(&self) -> bool {
        self.0.cell.initialized()
    }

    /// The model, loading it off the async threads the first time.
    pub async fn get(&self) -> Result<Arc<dyn Embedder>, JobError> {
        let e = self
            .0
            .cell
            .get_or_try_init(|| async {
                let Some(dir) = self.0.dir.clone() else {
                    return Err(JobError::Value("no bge-small model in this worker".into()));
                };
                let t = std::time::Instant::now();
                let bge = tokio::task::spawn_blocking(move || pk_embed::Bge::load_with_threads(&dir, 1))
                    .await
                    .map_err(|e| JobError::Value(format!("loading the bge model: {e}")))??;
                tracing::info!(ms = t.elapsed().as_millis(), "bge-small loaded");
                Ok::<Arc<dyn Embedder>, JobError>(Arc::new(bge))
            })
            .await?;
        Ok(Arc::clone(e))
    }
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.cell.get() {
            Some(e) => f.write_str(e.name()),
            None => f.write_str("bge-small (not loaded)"),
        }
    }
}

/// A claimed job as the API hands it out; the input is typed per kind by its runner.
#[derive(Debug, Clone, Deserialize)]
pub struct Job {
    /// `job:<kind>-<hex>`.
    pub id: String,
    /// The kind's wire name.
    pub kind: String,
    /// The kind's input (`null` reads as `{}`).
    #[serde(default)]
    pub input: Value,
}

/// What a job can use: everything shared, borrowed.
#[derive(Debug, Clone, Copy)]
pub struct Ctx<'a> {
    /// The API.
    pub api: &'a ApiClient,
    /// The model router (one budget for the process).
    pub router: &'a Router,
    /// The voice profile this job runs with.
    pub voice: &'a Voice,
    /// How the prompts name him.
    pub names: &'a Names,
    /// The court-material net.
    pub legal: &'a Legal,
    /// The fact-check page fetcher.
    pub pages: &'a Pages,
    /// His notebook, for the kinds that read it.
    pub corpus: Option<&'a Arc<Corpus>>,
    /// The mirrored files the notebook was built from (path → text), for the kinds that read it.
    pub files: Option<&'a BTreeMap<String, Arc<str>>>,
    /// The bge-small model, when this worker has it.
    pub embedder: Option<&'a Model>,
    /// The creative logs mirrored with the notebook (store → lines), for the kinds that read it.
    pub creative: Option<&'a BTreeMap<String, Vec<String>>>,
    /// The decision lines mirrored with the notebook, oldest first, for the kinds that read it.
    pub decisions: Option<&'a Arc<[Value]>>,
    /// The one-shot Ask's document vectors, kept across jobs.
    pub ask_vectors: &'a Arc<ask::AskVectors>,
    /// `PK_OWNER_ID`: the actor of his own edits (the merges store's reducer needs it).
    pub owner: &'a str,
}

impl<'a> Ctx<'a> {
    /// The notebook; an error for a kind dispatched without one (a wiring bug, never his input).
    pub fn corpus(&self) -> Result<&'a Arc<Corpus>, JobError> {
        self.corpus
            .ok_or_else(|| JobError::Value("this kind needs the notebook and none was loaded".into()))
    }
}

/// A text file a job leaves for the API (`online.Workspace.push` posts it after the kind returns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutFile {
    /// Relative path (`corpus/sets/<set>/analysis.json`).
    pub path: String,
    /// The whole text.
    pub text: String,
}

/// A finished job: the result object and the files to post.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Done {
    /// The kind's result.
    pub result: Map<String, Value>,
    /// Files to post, legal-checked first.
    pub files: Vec<OutFile>,
}

impl Done {
    /// A result with no files.
    pub fn result(v: Value) -> Self {
        Self {
            result: match v {
                Value::Object(m) => m,
                _ => Map::new(),
            },
            files: Vec::new(),
        }
    }
}

/// One kind: its input type and how it runs.
pub trait Runner {
    /// The input, as `pk-wire` types it.
    type Input: DeserializeOwned;
    /// The error for an input that does not parse (each kind words its own).
    fn bad_input(e: &serde_json::Error) -> JobError;
    /// Run it.
    fn run(ctx: Ctx<'_>, input: Self::Input) -> impl Future<Output = Result<Done, JobError>> + Send;
}

async fn run_typed<R: Runner>(ctx: Ctx<'_>, input: Value) -> Result<Done, JobError> {
    let input = if input.is_null() {
        Value::Object(Map::new())
    } else {
        input
    };
    let typed = serde_json::from_value::<R::Input>(input).map_err(|e| R::bad_input(&e))?;
    R::run(ctx, typed).await
}

/// Run one job of a kind this worker can run. A kind not ported yet is the caller's to hand back; reaching this
/// with one is a `ValueError`, as the Python's unknown kind.
pub async fn dispatch(kind: Kind, ctx: Ctx<'_>, input: Value) -> Result<Done, JobError> {
    match kind {
        Kind::AskChat => run_typed::<ask_chat::AskChat>(ctx, input).await,
        Kind::Facts => run_typed::<facts::Facts>(ctx, input).await,
        Kind::SetText => run_typed::<set_text::SetText>(ctx, input).await,
        Kind::Riff => run_typed::<riff::Riff>(ctx, input).await,
        Kind::Punchup => run_typed::<punchup::Punchup>(ctx, input).await,
        Kind::Bridge => run_typed::<bridge::Bridge>(ctx, input).await,
        Kind::Embed => run_typed::<embed::Embed>(ctx, input).await,
        Kind::Memolink => run_typed::<memolink::Memolink>(ctx, input).await,
        Kind::Ask => run_typed::<ask::Ask>(ctx, input).await,
        Kind::Rant => run_typed::<rant::Rant>(ctx, input).await,
        Kind::Hook => run_typed::<rant::Hook>(ctx, input).await,
        Kind::RantDraft => run_typed::<rant_draft::RantDraft>(ctx, input).await,
        Kind::RantLine => run_typed::<rant_line::RantLine>(ctx, input).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_model_that_will_not_load_fails_each_ask_and_is_never_kept() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("onnx")).unwrap();
        std::fs::write(d.path().join("onnx").join("model.onnx"), b"not an onnx model\n").unwrap();
        std::fs::write(d.path().join("tokenizer.json"), b"{}").unwrap();
        // building it reads nothing: a worker that never embeds never loads the model
        let m = Model::lazy(d.path().to_path_buf());
        assert!(!m.is_loaded());
        assert!(m.get().await.is_err());
        assert!(!m.is_loaded(), "a failed load is not kept");
        assert!(m.get().await.is_err(), "the next job tries again");
        let none = Model::lazy(d.path().join("missing"));
        assert!(none.get().await.is_err());
    }

    #[test]
    fn claimable_is_the_hosted_order_ported_only() {
        let names: Vec<&str> = Kind::claimable(false).into_iter().map(Kind::name).collect();
        assert_eq!(
            names,
            [
                "bridge",
                "punchup",
                "facts",
                "riff",
                "ask_chat",
                "set_text",
                "rant",
                "hook",
                "rant_draft",
                "rant_line"
            ]
        );
        let all: Vec<&str> = Kind::claimable(true).into_iter().map(Kind::name).collect();
        assert_eq!(
            all,
            [
                "bridge",
                "punchup",
                "facts",
                "riff",
                "ask_chat",
                "set_text",
                "rant",
                "hook",
                "embed",
                "memolink",
                "ask",
                "rant_draft",
                "rant_line"
            ],
            "the Python's eleven in its order, then the Rust-only kinds"
        );
        assert_eq!(Kind::parse("rant_draft"), Some(Kind::RantDraft));
        assert_eq!(Kind::parse("rant_line"), Some(Kind::RantLine));
        assert!(Kind::Ask.reads_notebook() && !Kind::Ask.builds_corpus() && Kind::Riff.builds_corpus());
        assert_eq!(Kind::parse("memolink"), Some(Kind::Memolink));
        assert_eq!(Kind::parse("process_set"), None);
        assert!(
            Kind::claimable(true)
                .iter()
                .all(|k| !TRANSCRIBER_KINDS.contains(&k.name()))
        );
    }
}
