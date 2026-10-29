//! Punchable's knowledge base, with no IO.
//!
//! The graph (`graph.json`: nodes and edges as JSON objects) plus an append-only log of decisions, replayed in order
//! into a [`View`]: the state the API serves and the worker reads. A decision is one line: who did what to which node
//! or edge, with the before and after images, so every write is undoable by appending its inverse.
//!
//! This crate is the Rust home of `tools/explorer/kb.py`. It holds the replay ([`view`]), the read models
//! ([`read`]: briefs, summaries, search, a joke's detail, the label index) and the decision builder ([`build`]: the
//! validation every write goes through, and the inverse line an undo appends). The tests replay goldens generated
//! by the Python it replaces (`rust/tools/gen_kb_goldens.py`).
//!
//! Records stay JSON objects ([`Rec`]): the replay is a dict merge, and a fully typed struct would silently drop the
//! fields the pipeline writes. Typed accessors live on [`model`].
//!
//! Storage (`SurrealDB`, files) and HTTP live in other crates.

pub mod build;
pub mod decision;
pub mod error;
pub mod ids;
pub mod model;
pub mod read;
pub mod view;

pub use build::{Built, build, undo_line};
pub use decision::Decision;
pub use error::KbError;
pub use ids::{EdgeId, NodeId, NodeKind, edge_id, slugify};
pub use model::{Bucket, Rec, bucket, ekind, public};
pub use view::View;

/// The actor id written on the comic's own edits (`by`); `PK_OWNER_ID` in the Python, default `owner`.
pub const DEFAULT_OWNER_ID: &str = "owner";
