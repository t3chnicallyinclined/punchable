//! The schema, compiled in from `db/schema.surql` and `db/schema_extra.surql`, applied the way `db/load.py` applies
//! it: only when its sha256 changed since the last time (every `DEFINE INDEX ... OVERWRITE` rebuilds its index from
//! every row, even unchanged), and with the change feeds checked and repaired (trap T6).

use std::collections::BTreeMap;

use regex::Regex;
use sha2::{Digest, Sha256};
use surrealdb::types::{Object, Value};

use crate::config::Auth;
use crate::error::{Result, StoreError};
use crate::store::{Store, vars};
use crate::value::as_str;

/// `db/schema.surql`: the graph tables, `fn::apply_decision`, the events.
pub const SCHEMA: &str = include_str!("../../../../db/schema.surql");
/// `db/schema_extra.surql`: the tables added for hosting (`creative_event`, `srcfile`), the search indexes.
pub const SCHEMA_EXTRA: &str = include_str!("../../../../db/schema_extra.surql");

/// Tables that carry a change feed (the future phone sync pulls these). Checked at startup.
pub const CHANGEFEED_TABLES: [&str; 7] = [
    "decision",
    "joke",
    "performance",
    "set",
    "label",
    "list",
    "performed_as",
];

/// `(sha of schema.surql + schema_extra.surql, sha of schema_extra.surql alone)`, as `load.py._sha` computes them:
/// each text followed by a NUL byte.
pub fn schema_shas() -> (String, String) {
    (sha(&[SCHEMA, SCHEMA_EXTRA]), sha(&[SCHEMA_EXTRA]))
}

fn sha(texts: &[&str]) -> String {
    let mut h = Sha256::new();
    for t in texts {
        h.update(t.as_bytes());
        h.update(b"\0");
    }
    hex::encode(h.finalize())
}

pub(crate) fn check_ident(s: &str, what: &str) -> Result<()> {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(())
    } else {
        Err(StoreError::Config(format!("{what} '{s}' is not a plain identifier")))
    }
}

impl Store {
    /// With root credentials, define the namespace and the `STRICT` database before anything selects them
    /// (`db/load.py` does the same): selecting a database first would create it lax, and `IF NOT EXISTS` would then
    /// leave it lax for good, so an unknown table would be a silent create instead of an error. A database user
    /// cannot define, and does not need to. Idempotent.
    pub(crate) async fn define_strict_database(&self) -> Result<()> {
        let (ns, db) = (&self.config().ns, &self.config().db);
        check_ident(ns, "namespace")?;
        check_ident(db, "database")?;
        if matches!(self.config().auth, Auth::Root { .. }) {
            self.db()
                .query(format!(
                    "DEFINE NAMESPACE IF NOT EXISTS {ns}; USE NS {ns}; DEFINE DATABASE IF NOT EXISTS {db} STRICT;"
                ))
                .await?
                .check()?;
        }
        self.db().use_ns(ns.clone()).use_db(db.clone()).await?;
        Ok(())
    }

    /// `{table: definition}` for the tables whose change feed is missing. Empty means all present.
    pub async fn check_changefeeds(&self, tables: &[&str]) -> Result<BTreeMap<String, String>> {
        let info = self.last("INFO FOR DB;", Object::new()).await?;
        let defined = match info {
            Value::Object(mut o) => match o.remove("tables") {
                Some(Value::Object(t)) => t,
                _ => Object::new(),
            },
            _ => Object::new(),
        };
        let mut missing = BTreeMap::new();
        for t in tables {
            let d = defined
                .get(*t)
                .map(|v| as_str(v).map_or_else(|| v.clone().into_json_value().to_string(), str::to_string))
                .unwrap_or_default();
            if !d.to_uppercase().contains("CHANGEFEED") {
                missing.insert(
                    (*t).to_string(),
                    if d.is_empty() { "(table missing)".to_string() } else { d },
                );
            }
        }
        Ok(missing)
    }

    /// Apply both schema files to the configured namespace and database, only when they changed since the last
    /// time (or `force`). A root connection defines the namespace and the `STRICT` database first; a database user
    /// cannot, and does not need to. Returns whether the schema was applied.
    pub async fn apply_schema(&self, force: bool) -> Result<bool> {
        self.define_strict_database().await?;
        let (sha, xsha) = schema_shas();
        if !force
            && self.stored_shas().await?.0.as_deref() == Some(sha.as_str())
            && self.check_changefeeds(&CHANGEFEED_TABLES).await?.is_empty()
        {
            return Ok(false);
        }
        self.query(SCHEMA, Object::new()).await?;
        self.apply_extra_schema(true).await?;
        self.query(
            "IF !record::exists(meta:state) { CREATE meta:state CONTENT { gen: 0, ord_next: 0, orphans: 0, loading: false, load_info: {} } };",
            Object::new(),
        )
        .await?;
        self.repair_changefeeds().await?;
        self.write(
            "UPDATE meta:state SET schema_sha = $s, extra_sha = $x RETURN id;",
            vars([("s", Value::String(sha)), ("x", Value::String(xsha))]),
            None,
        )
        .await?;
        Ok(true)
    }

    /// Apply `schema_extra.surql` alone (additive, idempotent: the API does this at startup), skipped when unchanged
    /// since the last time (`meta:state.extra_sha`). Returns whether it was applied.
    pub async fn apply_extra_schema(&self, force: bool) -> Result<bool> {
        let (_, xsha) = schema_shas();
        if !force && self.stored_shas().await?.1.as_deref() == Some(xsha.as_str()) {
            return Ok(false);
        }
        self.query(SCHEMA_EXTRA, Object::new()).await?;
        if !force && self.last("RETURN record::exists(meta:state);", Object::new()).await? == Value::Bool(true) {
            self.write(
                "UPDATE meta:state SET extra_sha = $x RETURN id;",
                vars([("x", Value::String(xsha))]),
                None,
            )
            .await?;
        }
        Ok(true)
    }

    async fn stored_shas(&self) -> Result<(Option<String>, Option<String>)> {
        let info = self.last("INFO FOR DB;", Object::new()).await?;
        let has_meta = match &info {
            Value::Object(o) => matches!(o.get("tables"), Some(Value::Object(t)) if t.contains_key("meta")),
            _ => false,
        };
        if !has_meta {
            return Ok((None, None));
        }
        let row = self
            .last("SELECT schema_sha, extra_sha FROM ONLY meta:state;", Object::new())
            .await?;
        let Value::Object(o) = row else {
            return Ok((None, None));
        };
        let get = |k: &str| o.get(k).and_then(as_str).map(str::to_string);
        Ok((get("schema_sha"), get("extra_sha")))
    }

    /// Trap T6: something re-defined a table without its `CHANGEFEED` clause. `IF NOT EXISTS` cannot restore it, so
    /// exactly those tables are re-defined from `schema.surql` with `OVERWRITE` (which carries the clause).
    async fn repair_changefeeds(&self) -> Result<()> {
        let missing = self.check_changefeeds(&CHANGEFEED_TABLES).await?;
        if missing.is_empty() {
            return Ok(());
        }
        for (t, was) in &missing {
            let re = Regex::new(&format!(r"(?ms)^DEFINE TABLE IF NOT EXISTS {t} [^;]*;")).expect("a fixed pattern");
            let Some(m) = re.find(SCHEMA) else {
                return Err(StoreError::Changefeed(missing.clone()));
            };
            tracing::warn!(table = %t, was = %was, "change feed was missing; re-defining the table with its feed");
            self.query(&m.as_str().replacen("IF NOT EXISTS", "OVERWRITE", 1), Object::new())
                .await?;
        }
        let still = self.check_changefeeds(&CHANGEFEED_TABLES).await?;
        if still.is_empty() {
            Ok(())
        } else {
            Err(StoreError::Changefeed(still))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shas_are_sha256_hex_and_differ() {
        let (a, b) = schema_shas();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_eq!(sha(&["x"]), hex::encode(Sha256::digest(b"x\0")));
    }

    #[test]
    fn every_changefeed_table_is_in_the_schema() {
        for t in CHANGEFEED_TABLES {
            let re = Regex::new(&format!(r"(?ms)^DEFINE TABLE IF NOT EXISTS {t} [^;]*;")).unwrap();
            let m = re
                .find(SCHEMA)
                .unwrap_or_else(|| panic!("{t} is not defined in schema.surql"));
            assert!(m.as_str().contains("CHANGEFEED"), "{t} is defined without a feed");
        }
    }
}
