//! The nightly backup job in Rust (T11 round 7): `SurrealDB`'s export and Bugsink's SQLite to an S3-compatible bucket
//! (Cloudflare R2), a port **as-is** of `infra/backup/backup.py`, which stays until the T11 rollback window closes.
//!
//! The same commands (`run`, `list`, `fetch <key> <out.gz>`, `restore-check <file.gz>`, `export <out.gz>`), the same
//! settings (environment only; nothing secret is ever on argv or printed), the same object names
//! (`<prefix>daily/<ns>-<db>-<yyyymmddTHHMMSSZ>.surql.gz`, `<prefix>weekly/<same>`, `bugsink/daily|weekly/bugsink-db-...`),
//! the same checks before and after an upload, the same retention (the newest 14 daily and 8 weekly; only a key that
//! is exactly a backup name is ever a candidate) and the same exit status (0 ok, 1 on any failure with one redacted
//! line on stderr, 2 for a bad command line). The differential (`tests/differential.rs`) runs the Python and this
//! binary against one scratch `SurrealDB` and a local S3 stand-in and compares the objects, the exports and the
//! retention decisions.
//!
//! What differs, on purpose: error texts that quote a Python exception class (`BadGzipFile: ...`) quote the Rust one;
//! the HTTP timeouts are totals per request, where Python's are per socket read; and `SSL_CERT_FILE`, when set, adds
//! its certificates to the trusted roots (OpenSSL-linked Python reads the same variable on Linux).
//!
//! Library crates never read the environment (ADR 0010): [`Vars`] is built by the binary and handed in.

pub mod cli;
pub mod dump;
pub mod error;
pub mod redact;
pub mod retention;
pub mod s3;
pub mod settings;
pub mod sigv4;
pub mod sqlite;

pub use error::{BackupError, Result};
pub use settings::Vars;

/// 1 MiB, the Python's read size for every stream.
pub const CHUNK: usize = 1 << 20;
