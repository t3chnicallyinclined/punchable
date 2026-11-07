//! The pointer rows of the recordings waiting for a transcriber (`db/schema_extra.surql` `upload`; docs/adr/0015).
//!
//! The bytes are NOT here: they are one object in the recordings bucket (R2), and the `upload:<file name>` row holds
//! only a pointer: the object key (in the `token` field, the name ADR 0014 gave it, so the schema and the Python's
//! rollback image are untouched), the size, the part count, the meta.json the upload carried and the time. The row is
//! made LAST, after the object is complete, so a half-sent file is never listed. A name is reserved by that CREATE (the
//! record id is unique): a refused one tries the next suffix, as the Python's exclusive file create does.
//!
//! `upload_part` and `upload_stage` (ADR 0014's database home) are no longer written; the tables stay defined until the
//! Python is deleted (a schema change inside the rollback window would break the rollback image).

use std::time::{SystemTime, UNIX_EPOCH};

use surrealdb::types::{Number, Object, Value};

use crate::error::Result;
use crate::store::{Store, rows_of, vars};
use crate::value::{as_i64, as_str, from_db, to_db};

/// One upload as the inbox lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadRow {
    /// The file name (the record's key).
    pub name: String,
    /// The recording's object key in the bucket (the row's `token` field).
    pub key: String,
    /// Its size in bytes.
    pub bytes: i64,
    /// How many parts it was sent in.
    pub parts: i64,
    /// The meta.json the upload carried.
    pub meta: serde_json::Map<String, serde_json::Value>,
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn float(f: f64) -> Value {
    Value::Number(Number::Float(f))
}

fn row_of(o: &Object) -> Option<UploadRow> {
    Some(UploadRow {
        name: o.get("name").and_then(as_str)?.to_string(),
        key: o.get("token").and_then(as_str).unwrap_or_default().to_string(),
        bytes: o.get("bytes").and_then(as_i64)?,
        parts: o.get("parts").and_then(as_i64).unwrap_or(0),
        meta: o
            .get("meta")
            .cloned()
            .map(from_db)
            .and_then(|m| m.as_object().cloned())
            .unwrap_or_default(),
    })
}

impl Store {
    /// Make the `upload:<name>` row (the commit): the next free suffix when the name is taken (`<stem>-2.<ext>`).
    /// `key` is the object that already holds the bytes. The name used.
    pub async fn upload_commit(
        &self,
        name: &str,
        key: &str,
        meta: &serde_json::Value,
        bytes: i64,
        parts: i64,
    ) -> Result<String> {
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) => (s, format!(".{e}")),
            None => (name, String::new()),
        };
        let mut use_name = name.to_string();
        let mut k = 2;
        loop {
            let r = self
                .write(
                    "CREATE type::record('upload', $n) CONTENT {name: $n, token: $k, meta: $m, bytes: $b, parts: $p, t: $t} \
                     RETURN id;",
                    vars([
                        ("n", Value::String(use_name.clone())),
                        ("k", Value::String(key.to_string())),
                        ("m", to_db(meta)),
                        ("b", Value::Number(Number::Int(bytes))),
                        ("p", Value::Number(Number::Int(parts))),
                        ("t", float(now())),
                    ]),
                    Some(1),
                )
                .await;
            match r {
                Ok(_) => return Ok(use_name),
                Err(e) if e.to_string().to_lowercase().contains("already exists") => {
                    use_name = format!("{stem}-{k}{ext}");
                    k += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Every upload, newest first.
    pub async fn upload_rows(&self) -> Result<Vec<UploadRow>> {
        let v = self
            .last(
                "SELECT name, token, bytes, parts, meta, t FROM upload ORDER BY t DESC, name DESC;",
                Object::new(),
            )
            .await?;
        Ok(rows_of(v)
            .into_iter()
            .filter_map(|r| if let Value::Object(o) = r { row_of(&o) } else { None })
            .collect())
    }

    /// The names of the uploads committed before `cutoff` (seconds since the epoch), oldest first.
    pub async fn upload_names_before(&self, cutoff: f64) -> Result<Vec<String>> {
        let v = self
            .last(
                "SELECT VALUE name FROM upload WHERE t < $c ORDER BY t;",
                vars([("c", float(cutoff))]),
            )
            .await?;
        Ok(rows_of(v)
            .into_iter()
            .filter_map(|r| as_str(&r).map(str::to_string))
            .collect())
    }

    /// The object key of every upload row: what the bucket sweep must not delete.
    pub async fn upload_keys(&self) -> Result<Vec<String>> {
        let v = self.last("SELECT VALUE token FROM upload;", Object::new()).await?;
        Ok(rows_of(v)
            .into_iter()
            .filter_map(|r| as_str(&r).map(str::to_string))
            .collect())
    }

    /// How many uploads wait.
    pub async fn upload_count(&self) -> Result<i64> {
        Ok(as_i64(&self.last("RETURN count(SELECT id FROM upload);", Object::new()).await?).unwrap_or(0))
    }

    /// One upload by its file name.
    pub async fn upload_get(&self, name: &str) -> Result<Option<UploadRow>> {
        let v = self
            .last(
                "SELECT name, token, bytes, parts, meta FROM ONLY type::record('upload', $n);",
                vars([("n", Value::String(name.to_string()))]),
            )
            .await?;
        Ok(if let Value::Object(o) = v { row_of(&o) } else { None })
    }

    /// Delete the pointer row of an upload and give it back (`None` when there was none), so the caller can delete the
    /// object it named. The row goes first and the bytes second: a crash between the two leaves an object with no row,
    /// which the bucket sweep and the lifecycle rule remove, and never a listed upload with no bytes.
    pub async fn upload_delete(&self, name: &str) -> Result<Option<UploadRow>> {
        let row = self.upload_get(name).await?;
        if row.is_some() {
            self.query(
                "DELETE type::record('upload', $n) RETURN NONE;",
                vars([("n", Value::String(name.to_string()))]),
            )
            .await?;
        }
        Ok(row)
    }
}
