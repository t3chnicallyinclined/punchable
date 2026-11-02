//! The full-text leg of the one-shot Ask (`engine/ask.py` `Corpus`): an in-memory `SQLite` table
//! `fts5(id unindexed, title, text, tokenize='porter unicode61')`, filled in document order and searched with
//! `select id from f where f match ? order by bm25(f) limit ?`.
//!
//! Why `SQLite` and not a Rust index: the Ask ranks by `SQLite`'s own porter stemmer, unicode61 folding, FTS5 query
//! parser and `bm25()`, and the port's contract is the Python's order. `rusqlite` with `bundled` compiles `SQLite` from
//! its amalgamation (public domain; `libsqlite3-sys` is MIT) with FTS5 on, so the desk, CI and the image run one
//! engine with no system library. Pure: the database lives in memory and dies with the [`Index`]; no file, no
//! environment, no network. The only C in the workspace outside ONNX Runtime, behind this one crate
//! (`deny.toml` bans `rusqlite` everywhere else).
//!
//! [`Index`] holds a `rusqlite::Connection`: `Send`, not `Sync`. Build and search it on a blocking thread.

use rusqlite::{Connection, params};

/// What can go wrong: `SQLite`'s own words (the `rusqlite` error stays inside this crate).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FtsError {
    /// `SQLite` refused the statement (an FTS5 syntax error in a query lands here: the Python's
    /// `sqlite3.OperationalError`, which the Ask reads as no hits).
    #[error("sqlite: {0}")]
    Sqlite(String),
}

#[allow(clippy::needless_pass_by_value)] // a `map_err` adapter
fn sql(e: rusqlite::Error) -> FtsError {
    FtsError::Sqlite(e.to_string())
}

/// One table of `(id, title, text)` rows, searchable by an FTS5 query.
#[derive(Debug)]
pub struct Index {
    db: Connection,
    rows: usize,
}

impl Index {
    /// The table over `rows` (`(id, title, text)`), inserted in the given order (the rowid order `SQLite` breaks a
    /// bm25 tie by), in one transaction. Temporary storage is pinned to memory: nothing of his touches a disk.
    pub fn build<I, A, B, C>(rows: I) -> Result<Self, FtsError>
    where
        I: IntoIterator<Item = (A, B, C)>,
        A: AsRef<str>,
        B: AsRef<str>,
        C: AsRef<str>,
    {
        let mut db = Connection::open_in_memory().map_err(sql)?;
        db.execute_batch(
            "pragma temp_store = memory;
             create virtual table f using fts5(id unindexed, title, text, tokenize='porter unicode61')",
        )
        .map_err(sql)?;
        let mut n = 0usize;
        {
            let tx = db.transaction().map_err(sql)?;
            {
                let mut ins = tx.prepare("insert into f values (?,?,?)").map_err(sql)?;
                for (id, title, text) in rows {
                    ins.execute(params![id.as_ref(), title.as_ref(), text.as_ref()])
                        .map_err(sql)?;
                    n += 1;
                }
            }
            tx.commit().map_err(sql)?;
        }
        Ok(Self { db, rows: n })
    }

    /// How many rows.
    pub fn len(&self) -> usize {
        self.rows
    }

    /// No rows.
    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }

    /// The ids of the best `k` rows for an FTS5 query, best first (`order by bm25(f)`, lower is better).
    pub fn search(&self, expr: &str, k: usize) -> Result<Vec<String>, FtsError> {
        let mut st = self
            .db
            .prepare("select id from f where f match ? order by bm25(f) limit ?")
            .map_err(sql)?;
        let k = i64::try_from(k).unwrap_or(i64::MAX);
        st.query_map(params![expr, k], |r| r.get::<_, String>(0))
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)
    }
}

/// The `SQLite` this crate runs (`select sqlite_version()`), for the log and the parity note.
pub fn sqlite_version() -> &'static str {
    rusqlite::version()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx() -> Index {
        Index::build([
            ("a", "Airport", "Security wants my belt in a bin."),
            (
                "b",
                "Parrot",
                "My parrot only says legal disclaimers. Running, runs, ran.",
            ),
            ("c", "Gym", "At the gym I stare at the mirror."),
            ("d", "Bins", "bins bins bins and a belt"),
        ])
        .unwrap()
    }

    #[test]
    fn porter_stems_and_bm25_orders() {
        let i = idx();
        assert_eq!(i.len(), 4);
        // "bins" stems to "bin": both rows match, the one with more of it first
        assert_eq!(i.search("\"bins\"", 50).unwrap(), ["d", "a"]);
        assert_eq!(i.search("\"running\"", 50).unwrap(), ["b"], "porter: running -> run");
        assert_eq!(i.search("\"belt\" OR \"mirror\"", 50).unwrap().len(), 3);
        assert_eq!(i.search("\"bins\"", 1).unwrap(), ["d"], "the limit");
    }

    #[test]
    fn the_id_column_is_not_searched_and_titles_are() {
        let i = Index::build([("zebraid", "Parrot", "words"), ("y", "Gym", "zebra")]).unwrap();
        assert_eq!(i.search("\"zebraid\"", 50).unwrap(), Vec::<String>::new());
        assert_eq!(i.search("\"zebra\"", 50).unwrap(), ["y"]);
        let i = idx();
        assert_eq!(i.search("\"parrot\"", 50).unwrap(), ["b"]);
        assert_eq!(i.search("\"gym\"", 50).unwrap(), ["c"]);
    }

    #[test]
    fn a_bad_query_is_an_error_the_caller_reads_as_no_hits() {
        assert!(idx().search("\"unclosed", 50).is_err());
        assert!(Index::build(Vec::<(&str, &str, &str)>::new()).unwrap().is_empty());
        assert!(sqlite_version().starts_with('3'));
    }
}
