//! The `creative_event` mirror: every line of a creative store's jsonl (storyboards, working sets, venue pins, the
//! job queue) is also a row, written first, unique on `(store, seq)` so a double append is impossible, and the
//! reconcile rule of `api-dev/creative_api.py` `Creative.reconcile`: whichever of file and database is a strict
//! prefix of the other is extended; a divergence refuses writes until resolved, nothing is merged silently.
//!
//! Equality of two lines is equality of the parsed JSON (docs/RUST.md, wire-contract fact 3: the Rust side does not
//! reproduce `json.dumps`); two lines that are not both JSON compare as bytes.

use serde_json::Value as Json;
use surrealdb::types::{RecordId, Value};

use crate::error::{Result, StoreError};
pub use crate::kbmap::line_hash;
use crate::kbmap::{CreativeEvent, field_str};
use crate::store::{Store, rows_of, vars};
use crate::value::as_i64;

/// Whether two event lines are the same event: equal as parsed JSON, else equal as text.
pub fn same_line(a: &str, b: &str) -> bool {
    match (serde_json::from_str::<Json>(a), serde_json::from_str::<Json>(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// How file and database compared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reconcile {
    /// Line for line the same.
    Equal,
    /// Another writer appended to the file: its tail was copied into the database (`copied` rows).
    FileAhead {
        /// How many rows were created.
        copied: usize,
    },
    /// The database is ahead (a hosted pod rebuilt its files, a write from elsewhere): the caller appends `tail` to
    /// the file.
    DbAhead {
        /// The lines the file lacks, in order.
        tail: Vec<String>,
    },
    /// They differ from event `at`: writes to this store must be refused with `msg` (409) until resolved.
    Diverged {
        /// The first differing index.
        at: usize,
        /// Lines in the file.
        file: usize,
        /// Rows in the database.
        db: usize,
        /// The message the API answers with.
        msg: String,
    },
}

/// The database's lines after a reconcile, and what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciled {
    /// The store's lines as the database has them now, in `seq` order.
    pub lines: Vec<String>,
    /// What was done.
    pub outcome: Reconcile,
}

impl Store {
    /// A store's lines in `seq` order. The sequence must be `0..n` with no gap.
    pub async fn creative_lines(&self, store: &str) -> Result<Vec<String>> {
        let rows = rows_of(
            self.last(
                "SELECT seq, line FROM creative_event WHERE store = $s ORDER BY seq;",
                vars([("s", Value::String(store.to_string()))]),
            )
            .await?,
        );
        let mut out = Vec::with_capacity(rows.len());
        for (i, row) in rows.into_iter().enumerate() {
            let Value::Object(o) = row else {
                return Err(StoreError::mapping("creative_event row is not an object"));
            };
            if o.get("seq").and_then(as_i64) != i64::try_from(i).ok() {
                return Err(StoreError::Gap {
                    store: store.to_string(),
                });
            }
            let line = field_str(&o, "line").ok_or_else(|| StoreError::mapping("creative_event row has no line"))?;
            out.push(line.to_string());
        }
        Ok(out)
    }

    /// Create one row. A second row with the same `(store, seq)` is refused by the unique index: an error, never a
    /// silent duplicate.
    pub async fn create_creative_event(&self, ev: &CreativeEvent) -> Result<RecordId> {
        let rows = self
            .write(
                "CREATE creative_event CONTENT $row RETURN id;",
                vars([("row", ev.content())]),
                Some(1),
            )
            .await?;
        match rows.into_iter().next() {
            Some(Value::Object(mut o)) => match o.remove("id") {
                Some(Value::RecordId(r)) => Ok(r),
                other => Err(StoreError::mapping(format!(
                    "creative_event id is not a record id: {other:?}"
                ))),
            },
            other => Err(StoreError::mapping(format!("creative_event write returned {other:?}"))),
        }
    }

    /// File vs database for one store (`Creative.reconcile`): the file's tail is copied into the database when the
    /// database is a strict prefix of the file; the database's tail is returned for the file when the file is the
    /// prefix; a divergence is reported, not merged.
    pub async fn reconcile_creative(&self, store: &str, file_lines: &[String]) -> Result<Reconciled> {
        let db_lines = self.creative_lines(store).await?;
        let common = file_lines
            .iter()
            .zip(&db_lines)
            .take_while(|(f, d)| same_line(f, d))
            .count();
        let (fl, dl) = (file_lines.len(), db_lines.len());
        if common == fl && common == dl {
            return Ok(Reconciled {
                lines: db_lines,
                outcome: Reconcile::Equal,
            });
        }
        if common == dl {
            for (i, line) in file_lines.iter().enumerate().skip(dl) {
                let ev = CreativeEvent::new(store, i64::try_from(i).unwrap_or(i64::MAX), line)?;
                self.create_creative_event(&ev).await?;
            }
            return Ok(Reconciled {
                lines: file_lines.to_vec(),
                outcome: Reconcile::FileAhead { copied: fl - dl },
            });
        }
        if common == fl {
            return Ok(Reconciled {
                outcome: Reconcile::DbAhead {
                    tail: db_lines[fl..].to_vec(),
                },
                lines: db_lines,
            });
        }
        Ok(Reconciled {
            lines: db_lines,
            outcome: Reconcile::Diverged {
                at: common,
                file: fl,
                db: dl,
                msg: format!(
                    "{store}: {store}.jsonl and the database differ from event {common} ({fl} lines in the file, {dl} in the database)"
                ),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_compare_as_json() {
        assert!(same_line(r#"{"a":1,"b":2}"#, r#"{"b": 2, "a": 1}"#));
        assert!(!same_line(r#"{"a":1}"#, r#"{"a":2}"#));
        assert!(same_line("not json", "not json"));
        assert!(!same_line("not json", r#"{"a":1}"#));
    }
}
