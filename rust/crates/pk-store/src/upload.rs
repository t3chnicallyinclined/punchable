//! Uploads staged for a transcriber (`db/schema_extra.surql` `upload`, `upload_part`; `api-dev/upload_store.py` is
//! the other writer of the same rows). The bytes are `upload_part` rows of at most [`PART`] bytes keyed by a private
//! token; the `upload:<file name>` row names the token and carries the meta.json, and is made LAST, so a half-sent
//! file is never listed. A name is reserved by that CREATE (the record id is unique): a refused one tries the next
//! suffix, as the Python's exclusive file create does.

use std::time::{SystemTime, UNIX_EPOCH};

use surrealdb::types::{Bytes, Number, Object, Value};

use crate::error::{Result, StoreError};
use crate::store::{Store, rows_of, vars};
use crate::value::{as_i64, as_str, from_db, to_db};

/// The largest part, in bytes (`upload_store.PART`).
pub const PART: usize = 1024 * 1024;
/// A part with no `upload` row this long after it was written is an orphan (the pod died mid-body).
pub const ORPHAN_S: f64 = 3600.0;

/// One upload as the inbox lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadRow {
    /// The file name (the record's key).
    pub name: String,
    /// The private key of its parts.
    pub token: String,
    /// Its size in bytes.
    pub bytes: i64,
    /// How many parts it has.
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
        token: o.get("token").and_then(as_str).unwrap_or_default().to_string(),
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
    /// One part of an upload, and the staging row that counts it (one transaction: a part is never there uncounted).
    pub async fn upload_put_part(&self, token: &str, seq: i64, data: Vec<u8>) -> Result<()> {
        self.query(
            "BEGIN; CREATE upload_part CONTENT {token: $k, seq: $s, data: $d} RETURN id; \
             UPSERT type::record('upload_stage', $k) SET t = $t, n = (n ?? 0) + 1 RETURN id; COMMIT;",
            vars([
                ("k", Value::String(token.to_string())),
                ("s", Value::Number(Number::Int(seq))),
                ("t", float(now())),
                ("d", Value::Bytes(Bytes::from(data))),
            ]),
        )
        .await?;
        Ok(())
    }

    /// The parts of a token and its staging row (an upload cut short, a failed commit).
    ///
    /// One transaction, so a write conflict is retried as a whole (`query` retries a `BEGIN ... COMMIT` block); two
    /// statements apart, the first could not be retried and a concurrent upload's sweep made it a 500.
    pub async fn upload_drop_parts(&self, token: &str) -> Result<()> {
        retry_conflict(|| async {
            self.query(
                "BEGIN; DELETE upload_part WHERE token = $k RETURN NONE; \
                 DELETE type::record('upload_stage', $k) RETURN NONE; COMMIT;",
                vars([("k", Value::String(token.to_string()))]),
            )
            .await
            .map(|_| ())
        })
        .await
    }

    /// Parts of an upload that never committed (or whose row was deleted first): found by their STAGING row, whose
    /// newest-part time is older than [`ORPHAN_S`] (a long upload keeps its row fresh with every part), never by
    /// scanning the megabyte rows. A token that did commit only loses its staging row.
    pub async fn upload_sweep_orphans(&self) -> Result<()> {
        self.query(
            "LET $stale = (SELECT VALUE record::id(id) FROM upload_stage WHERE t < $old); \
             LET $kept = (SELECT VALUE token FROM upload WHERE token IN $stale); \
             FOR $k IN $stale { IF $k NOT IN $kept { DELETE upload_part WHERE token = $k RETURN NONE; }; }; \
             DELETE upload_stage WHERE t < $old RETURN NONE;",
            vars([("old", float(now() - ORPHAN_S))]),
        )
        .await?;
        Ok(())
    }

    /// Make the `upload:<name>` row (the commit): the next free suffix when the name is taken (`<stem>-2.<ext>`).
    /// Refused when the staging row does not count exactly `parts` parts, so a listed upload always has every part.
    /// The name used.
    pub async fn upload_commit(
        &self,
        name: &str,
        token: &str,
        meta: &serde_json::Value,
        bytes: i64,
        parts: i64,
    ) -> Result<String> {
        let counted = self
            .last(
                "SELECT VALUE n FROM ONLY type::record('upload_stage', $k);",
                vars([("k", Value::String(token.to_string()))]),
            )
            .await?;
        if as_i64(&counted) != Some(parts) {
            return Err(StoreError::mapping(format!(
                "upload {name}: {parts} parts expected, {counted:?} written"
            )));
        }
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
                        ("k", Value::String(token.to_string())),
                        ("m", to_db(meta)),
                        ("b", Value::Number(Number::Int(bytes))),
                        ("p", Value::Number(Number::Int(parts))),
                        ("t", float(now())),
                    ]),
                    Some(1),
                )
                .await;
            match r {
                Ok(_) => {
                    self.query(
                        "DELETE type::record('upload_stage', $k) RETURN NONE;",
                        vars([("k", Value::String(token.to_string()))]),
                    )
                    .await?;
                    return Ok(use_name);
                }
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

    /// One part's bytes; `None` when it is missing.
    pub async fn upload_part(&self, token: &str, seq: i64) -> Result<Option<Vec<u8>>> {
        let v = self
            .last(
                "SELECT VALUE data FROM upload_part WHERE token = $k AND seq = $s LIMIT 1;",
                vars([
                    ("k", Value::String(token.to_string())),
                    ("s", Value::Number(Number::Int(seq))),
                ]),
            )
            .await?;
        Ok(rows_of(v).into_iter().find_map(|d| match d {
            Value::Bytes(b) => Some(b.to_vec()),
            _ => None,
        }))
    }

    /// An upload and its parts: the row first (in the same transaction a stale staging row marks the token for the
    /// sweep), then the bytes, so a crash between the two leaves orphan parts the sweep removes and never an inbox row
    /// with no bytes.
    ///
    /// A write conflict with a concurrent upload's sweep is retried a few times, a little longer each, after the
    /// store's own retries of one transaction ([`retry_conflict`]), so concurrent uploads never answer 500.
    pub async fn upload_delete(&self, name: &str) -> Result<()> {
        if let Some(row) = self.upload_get(name).await? {
            retry_conflict(|| async {
                self.query(
                    "BEGIN; DELETE type::record('upload', $n) RETURN NONE; \
                     UPSERT type::record('upload_stage', $k) SET t = 0.0, n = 0 RETURN id; COMMIT;",
                    vars([
                        ("k", Value::String(row.token.clone())),
                        ("n", Value::String(name.to_string())),
                    ]),
                )
                .await
                .map(|_| ())
            })
            .await?;
            self.upload_drop_parts(&row.token).await?;
        }
        Ok(())
    }
}

/// How many times a whole write is asked again after another writer's conflict, and the wait before each (milliseconds).
const CONFLICT_WAITS: [u64; 4] = [25, 75, 150, 300];

/// Run `f` again, a bounded number of times, while it fails with a write conflict (`SurrealKV`: "can be retried"). Any
/// other error, and the last conflict, are returned as they are.
async fn retry_conflict<F, Fut>(f: F) -> Result<()>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Err(e) if attempt < CONFLICT_WAITS.len() && is_conflict(&e) => {
                tokio::time::sleep(std::time::Duration::from_millis(CONFLICT_WAITS[attempt])).await;
                attempt += 1;
            }
            other => return other,
        }
    }
}

/// A transaction conflict between two writers (the engine's words differ by backend).
fn is_conflict(e: &StoreError) -> bool {
    let text = e.to_string().to_lowercase();
    text.contains("can be retried") || text.contains("conflict")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[tokio::test]
    async fn a_conflict_is_asked_again_a_few_times_and_nothing_else_is() {
        let n = AtomicUsize::new(0);
        let r = retry_conflict(|| async {
            if n.fetch_add(1, Ordering::SeqCst) < 2 {
                Err(StoreError::mapping("Transaction write conflict: can be retried"))
            } else {
                Ok(())
            }
        })
        .await;
        assert!(r.is_ok());
        assert_eq!(n.load(Ordering::SeqCst), 3);
        // never ends
        let n = AtomicUsize::new(0);
        let r = retry_conflict(|| async {
            n.fetch_add(1, Ordering::SeqCst);
            Err(StoreError::mapping("write conflict"))
        })
        .await;
        assert!(r.is_err());
        assert_eq!(n.load(Ordering::SeqCst), CONFLICT_WAITS.len() + 1);
        // another error is not retried
        let n = AtomicUsize::new(0);
        let r = retry_conflict(|| async {
            n.fetch_add(1, Ordering::SeqCst);
            Err(StoreError::mapping("permission denied"))
        })
        .await;
        assert!(r.is_err());
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }
}
