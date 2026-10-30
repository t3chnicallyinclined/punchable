//! The worker's file routes (`api-dev/ingest_api.py`): the text files the database holds (`db/srcpaths.py`
//! `WORKER_PATTERNS`: a set's analysis, meta, laughs, transcript, clips; a memo's transcript and meta; gsync's
//! state; the kb's inferred / insights / learning; a few creative caches; `vectors/extra_*.json`). Paths are
//! relative to the data folder, forward slashes, never audio.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `GET /api/worker/files[?prefix=]`: every file the worker may read, sorted by path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FilesListing {
    /// The files under the prefix.
    pub files: Vec<FileEntry>,
    /// `meta:state.files_rev` on `SurrealDB` (moves on every write); `null` on the files backend.
    pub files_rev: Option<i64>,
    /// `"db"` or `"disk"`.
    pub mode: String,
}

/// One file in the listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FileEntry {
    /// Relative path.
    pub path: String,
    /// SHA-256 of the bytes, hex.
    pub sha256: String,
    /// Size in bytes.
    pub bytes: u64,
}

/// `GET /api/worker/files/content?path=`: the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FileContent {
    /// Relative path.
    pub path: String,
    /// SHA-256 of the text's UTF-8 bytes, hex.
    pub sha256: String,
    /// The text, whole.
    pub text: String,
}

/// `POST /api/worker/files`: files to write (up to 12 MiB a batch, 8 MiB a file) and paths to remove, by an actor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FilesPost {
    /// The files, each checked (pattern, size, JSON parses, sha matches, legal rules) before any is written.
    pub files: Vec<FilePut>,
    /// Paths to delete; left out when there are none (the worker never sends the key).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove: Vec<String>,
    /// `import:<source>` or `ai:<what>`.
    pub actor: String,
}

/// One file to write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FilePut {
    /// Relative path.
    pub path: String,
    /// The text, whole.
    pub text: String,
    /// SHA-256 of the text; the API refuses a mismatch. gsync sends none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// The answer to a files post.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FilesPostResponse {
    /// Always `true`.
    pub ok: bool,
    /// Paths whose content changed.
    pub written: Vec<String>,
    /// Paths that existed and were deleted.
    pub removed: Vec<String>,
    /// Files whose sha already matched.
    pub unchanged: u64,
    /// The graph re-sync's report when a set input changed on `SurrealDB`; `null` otherwise.
    pub synced: Option<Value>,
    /// `meta:state.files_rev` after the write; `null` on the files backend.
    pub files_rev: Option<i64>,
    /// The actor echoed back.
    pub actor: String,
}
