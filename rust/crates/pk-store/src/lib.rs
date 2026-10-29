//! `SurrealDB` through the official `surrealdb` crate, for Punchable's hosted services: the row mapping that
//! `db/kbmap.py` defines, the schema (`db/schema.surql` and `db/schema_extra.surql`, compiled in), the
//! decision-append transaction of `api-dev/surreal_kb.py`, the `creative_event` mirror of `api-dev/creative_api.py`,
//! and one `LIVE` subscription over the tables the app watches.
//!
//! **Crate version.** The server is `SurrealDB` 3.3.4. The crate is `surrealdb` 3.3.5 (workspace `Cargo.toml`): the
//! 3.x line is stable (3.0.0 shipped in 2025; 3.3.5 is the latest stable on 2026-10-29), it speaks the 3.x RPC and
//! checks the server against `>=3.0.0-alpha.1, <4.0.0` on connect, and its value model keeps `NONE` and `NULL` apart
//! (`Value::None` vs `Value::Null`). The last 2.x (2.7.0) speaks the 2.x protocol and refuses a 3.x server; 4.0.0 is
//! a beta. So "latest stable 2.x, else a 3.x prerelease" resolved to the stable 3.3.5. Only `protocol-ws` and `rustls`
//! are enabled: the services never embed a database.
//!
//! **Rules carried over from `db/sdb.py`.** Every statement that is not OK is an error, never a swallowed row. A
//! write whose last statement returns no rows is [`StoreError::EmptyWrite`]: a denied `CREATE`/`UPDATE` comes back as
//! `OK []` (trap T5) and must never look like a save. Change feeds are checked, never assumed: `DEFINE TABLE
//! OVERWRITE` without the clause drops one silently (trap T6). `NONE` (absent) and `NULL` stay distinct through
//! [`value::to_db`] and [`value::from_db`], which is why the crate has its own conversion and does not use the SDK's
//! `into_json_value` (that one folds `NONE` into `null`). Typed values go over the wire, so a string that looks like a
//! record id or a date stays a string.
//!
//! The JSON shapes the rows map to and from are `pk_kb`'s: [`pk_kb::Rec`] for a node or an edge, [`pk_kb::Decision`]
//! for a line of the log. The proof is `tests/scratch.rs`, against a scratch `surreal start memory`.

pub mod config;
pub mod creative;
pub mod error;
pub mod ids;
pub mod kbmap;
pub mod live;
pub mod schema;
pub mod store;
pub mod value;

pub use config::{Auth, Config, secrets};
pub use creative::{Reconcile, Reconciled, line_hash, same_line};
pub use error::{Result, StoreError};
pub use kbmap::{CreativeEvent, DecisionRow, EdgeKind, EdgeRow, Field, MetaState, NodeRow, SrcFile};
pub use live::{Change, Changes, LIVE_TABLES};
pub use schema::{CHANGEFEED_TABLES, SCHEMA, SCHEMA_EXTRA, schema_shas};
pub use store::{Appended, MIN_VERSION, Snapshot, Store, Vars, vars};
pub use value::{from_db, to_db};
