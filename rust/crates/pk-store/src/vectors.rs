//! The vector sweeper's rows (`api-dev/search_api.py` `pending`, `sweep`): the texts with no vector yet, or a
//! vector made from an older text, and the writes that store a batch's vectors with the sha256 of the text each was
//! made from (`embedded_sha`), so an edited text is embedded again and an unchanged one never is. Units keep no
//! sha: they are embedded at the load and never edited.

use surrealdb::types::{Array, Number, Object, RecordId, Value};

use crate::error::Result;
use crate::kbmap::line_hash;
use crate::store::{Store, rows_of, vars};
use crate::value::as_str;

/// The tables the sweeper fills, in the Python's order.
pub const VECTOR_TABLES: [&str; 3] = ["joke", "performance", "unit"];

/// A row to embed: its record and its text.
pub type PendingRow = (RecordId, String);

fn pending_rows(v: Value) -> Vec<PendingRow> {
    rows_of(v)
        .into_iter()
        .filter_map(|r| match r {
            Value::Object(o) => match (o.get("id"), o.get("text").and_then(as_str)) {
                (Some(Value::RecordId(id)), Some(t)) if !t.trim().is_empty() => Some((id.clone(), t.to_string())),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

impl Store {
    /// The rows whose text has no vector, or a vector of an older text, per table (`pending`): `jokes` are the
    /// typed ideas (jokes with no unit: only those carry their text in `summary`), which the caller takes from the
    /// view; every moment with text; every unit without a vector.
    pub async fn vectors_pending(&self, jokes: Vec<RecordId>) -> Result<Vec<(&'static str, Vec<PendingRow>)>> {
        let mut out = Vec::with_capacity(3);
        let jokes = if jokes.is_empty() {
            Vec::new()
        } else {
            let ids = Value::Array(Array::from(jokes.into_iter().map(Value::RecordId).collect::<Vec<_>>()));
            pending_rows(
                self.last(
                    "SELECT id, summary AS text FROM joke WHERE id IN $ids AND summary != NONE \
                     AND (embedded_sha = NONE OR embedded_sha != crypto::sha256(summary));",
                    vars([("ids", ids)]),
                )
                .await?,
            )
        };
        out.push(("joke", jokes));
        out.push((
            "performance",
            pending_rows(
                self.last(
                    "SELECT id, text FROM performance WHERE text != NONE AND string::len(text) > 0 \
                     AND (embedded_sha = NONE OR embedded_sha != crypto::sha256(text));",
                    Object::new(),
                )
                .await?,
            ),
        ));
        out.push((
            "unit",
            pending_rows(
                self.last(
                    "SELECT id, text FROM unit WHERE embedding = NONE AND text != NONE AND string::len(text) > 0;",
                    Object::new(),
                )
                .await?,
            ),
        ));
        Ok(out)
    }

    /// Store a batch's vectors (`sweep`'s `FOR $x IN $rows`): `embedding` on every row, and `embedded_sha` of the
    /// text on `joke` and `performance` rows.
    pub async fn vectors_set(&self, table: &str, rows: &[(RecordId, String, Vec<f32>)]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let with_sha = table != "unit";
        let updates: Vec<Value> = rows
            .iter()
            .map(|(id, text, vec)| {
                let mut o = Object::new();
                o.insert("id", Value::RecordId(id.clone()));
                o.insert(
                    "v",
                    Value::Array(Array::from(
                        vec.iter()
                            .map(|x| Value::Number(Number::Float(f64::from(*x))))
                            .collect::<Vec<_>>(),
                    )),
                );
                if with_sha {
                    o.insert("s", Value::String(line_hash(text)));
                }
                Value::Object(o)
            })
            .collect();
        let sql = if with_sha {
            "FOR $x IN $rows { LET $id = $x.id; UPDATE $id SET embedding = $x.v, embedded_sha = $x.s; };"
        } else {
            "FOR $x IN $rows { LET $id = $x.id; UPDATE $id SET embedding = $x.v; };"
        };
        self.query(sql, vars([("rows", Value::Array(Array::from(updates)))]))
            .await?;
        Ok(())
    }
}
