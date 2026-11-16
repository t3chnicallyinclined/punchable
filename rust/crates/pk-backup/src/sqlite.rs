//! The second file: Bugsink's SQLite (crash reports), copied with sqlite3's online backup API, never a raw file copy (a
//! WAL-mode database is three files and a plain copy of a live one is not a consistent snapshot).

use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::gzread::PyGzip;
use flate2::{Compression, GzBuilder};
use rusqlite::backup::{Backup, StepResult};
use rusqlite::{Connection, OpenFlags};

use crate::CHUNK;
use crate::dump::hash_file;
use crate::error::{BackupError, Result, fail};

/// The first 16 bytes of every SQLite file.
pub const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// The sizes a good copy reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteInfo {
    /// The gzip file's size.
    pub bytes_gz: u64,
    /// The database copy's size.
    pub bytes_db: u64,
    /// Tables in the copy.
    pub tables: u64,
    /// SHA-256 of the gzip file, hex.
    pub sha256: String,
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn side(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// A copy of a WAL-mode database brings its -wal/-shm side files when opened; remove all three (and a journal).
fn rm_sqlite(path: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let _ = std::fs::remove_file(side(path, suffix));
    }
}

fn sql_err(e: &rusqlite::Error) -> String {
    let kind = match e {
        rusqlite::Error::SqliteFailure(..) => "OperationalError",
        _ => "Error",
    };
    format!("{kind}: {e}")
}

/// Online backup of `src` into a temp file next to `dest`, then gzip it. Returns the copy's size in bytes.
pub fn backup_to_gzip(src: &Path, dest: &Path) -> Result<u64> {
    if !src.is_file() {
        return fail(format!("BUGSINK_DB_PATH {}: no such file", src.display()));
    }
    let tmp = side(dest, ".db");
    let result = (|| -> std::result::Result<u64, String> {
        let uri = format!("file:{}?mode=ro", src.display());
        let s = Connection::open_with_flags(
            uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| sql_err(&e))?;
        s.busy_timeout(Duration::from_secs(60)).map_err(|e| sql_err(&e))?;
        {
            let mut d = Connection::open(&tmp).map_err(|e| sql_err(&e))?;
            let b = Backup::new(&s, &mut d).map_err(|e| sql_err(&e))?;
            // pages at a time: a writer can get in between steps; wait a quarter second only when it did
            loop {
                match b.step(1024).map_err(|e| sql_err(&e))? {
                    StepResult::Done => break,
                    StepResult::Busy | StepResult::Locked => std::thread::sleep(Duration::from_millis(250)),
                    _ => {}
                }
            }
        }
        drop(s);
        let n = std::fs::metadata(&tmp).map_err(|e| format!("OSError: {e}"))?.len();
        let mut f = File::open(&tmp).map_err(|e| format!("OSError: {e}"))?;
        let raw = File::create(dest).map_err(|e| format!("OSError: {e}"))?;
        let mut gz = GzBuilder::new().mtime(0).write(raw, Compression::new(6));
        std::io::copy(&mut f, &mut gz).map_err(|e| format!("OSError: {e}"))?;
        gz.finish()
            .and_then(|mut r| r.flush())
            .map_err(|e| format!("OSError: {e}"))?;
        Ok(n)
    })();
    rm_sqlite(&tmp);
    result.map_err(|e| BackupError(format!("sqlite backup: {e}")))
}

/// Read the whole gzip back (CRC / length), confirm the SQLite header and run `PRAGMA integrity_check` on the copy.
pub fn check_gz(path: &Path, min_bytes: u64) -> Result<SqliteInfo> {
    if !path.is_file() {
        return fail(format!("{}: no such file", path.display()));
    }
    let name = name_of(path);
    let sha = hash_file(path).map_err(|e| BackupError(format!("{name}: {e}")))?;
    let tmp = side(path, ".check");
    let outcome = (|| -> Result<(u64, u64)> {
        let incomplete =
            |e: &dyn std::fmt::Display| BackupError(format!("{name}: not a complete gzip of a database ({e})"));
        let (mut n, mut head) = (0u64, Vec::<u8>::new());
        {
            let f = File::open(path).map_err(|e| incomplete(&e))?;
            let mut g = PyGzip::new(BufReader::new(f));
            let mut out = File::create(&tmp).map_err(|e| incomplete(&e))?;
            let mut buf = vec![0u8; CHUNK];
            loop {
                let got = g.read(&mut buf).map_err(|e| incomplete(&e))?;
                if got == 0 {
                    break;
                }
                if head.len() < 16 {
                    head.extend_from_slice(&buf[..got]);
                    head.truncate(16);
                }
                out.write_all(&buf[..got]).map_err(|e| incomplete(&e))?;
                n += got as u64;
            }
        }
        if n < min_bytes {
            return fail(format!("{name}: database copy is only {n} bytes"));
        }
        if head != SQLITE_MAGIC {
            return fail(format!("{name}: not a SQLite file"));
        }
        let c = Connection::open_with_flags(&tmp, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(|e| incomplete(&e))?;
        let rows: Vec<String> = {
            let mut st = c.prepare("PRAGMA integrity_check").map_err(|e| incomplete(&e))?;
            st.query_map([], |r| r.get::<_, String>(0))
                .and_then(Iterator::collect)
                .map_err(|e| incomplete(&e))?
        };
        let tables: i64 = c
            .query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |r| {
                r.get(0)
            })
            .map_err(|e| incomplete(&e))?;
        let tables = u64::try_from(tables).unwrap_or(0);
        drop(c);
        if rows != ["ok"] {
            let joined: String = rows.join("; ").chars().take(200).collect();
            return fail(format!("{name}: integrity_check: {joined}"));
        }
        if tables < 1 {
            return fail(format!("{name}: no tables"));
        }
        Ok((n, tables))
    })();
    rm_sqlite(&tmp);
    let (bytes_db, tables) = outcome?;
    let bytes_gz = std::fs::metadata(path)
        .map_err(|e| BackupError(format!("{name}: {e}")))?
        .len();
    Ok(SqliteInfo {
        bytes_gz,
        bytes_db,
        tables,
        sha256: sha,
    })
}

#[cfg(test)]
mod tests {
    use flate2::read::MultiGzDecoder;

    use super::*;

    fn make(dir: &Path, rows: usize) -> (Connection, PathBuf) {
        let p = dir.join("db.sqlite3");
        let c = Connection::open(&p).unwrap();
        c.pragma_update(None, "journal_mode", "WAL").unwrap();
        c.execute("CREATE TABLE event (id INTEGER PRIMARY KEY, body TEXT)", [])
            .unwrap();
        for _ in 0..rows {
            c.execute("INSERT INTO event (body) VALUES (?1)", [&"x".repeat(100)])
                .unwrap();
        }
        (c, p)
    }

    #[test]
    fn a_consistent_copy_that_passes_integrity_check() {
        let d = tempfile::tempdir().unwrap();
        let (c, p) = make(d.path(), 200);
        let out = d.path().join("copy.sqlite3.gz");
        let n = backup_to_gzip(&p, &out).unwrap();
        let info = check_gz(&out, 1024).unwrap();
        assert_eq!((info.bytes_db, info.tables), (n, 1));
        assert!(info.bytes_gz < info.bytes_db && info.sha256.len() == 64);
        assert!(!side(&out, ".db").exists() && !side(&out, ".check").exists());
        drop(c);
    }

    #[test]
    fn works_while_a_writer_holds_the_database_open() {
        let d = tempfile::tempdir().unwrap();
        let (c, p) = make(d.path(), 200);
        c.execute_batch("BEGIN; INSERT INTO event (body) VALUES ('open transaction')")
            .unwrap();
        let out = d.path().join("copy.sqlite3.gz");
        backup_to_gzip(&p, &out).unwrap();
        let raw = d.path().join("copy.sqlite3");
        let mut bytes = Vec::new();
        MultiGzDecoder::new(File::open(&out).unwrap())
            .read_to_end(&mut bytes)
            .unwrap();
        std::fs::write(&raw, bytes).unwrap();
        let n: i64 = Connection::open(&raw)
            .unwrap()
            .query_row("SELECT count(*) FROM event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 200); // the copy is of the last commit
        c.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn rejects_a_truncated_or_foreign_file_or_a_missing_one() {
        let d = tempfile::tempdir().unwrap();
        let (c, p) = make(d.path(), 200);
        drop(c);
        let out = d.path().join("copy.sqlite3.gz");
        backup_to_gzip(&p, &out).unwrap();
        let data = std::fs::read(&out).unwrap();
        let cut = d.path().join("cut.sqlite3.gz");
        std::fs::write(&cut, &data[..data.len() / 2]).unwrap();
        assert!(check_gz(&cut, 1024).unwrap_err().0.contains("not a complete gzip"));
        let other = d.path().join("other.sqlite3.gz");
        let mut gz = GzBuilder::new().write(Vec::new(), Compression::new(6));
        gz.write_all(&[b"OPTION IMPORT;\n".as_slice(), &[b'x'; 5000]].concat())
            .unwrap();
        std::fs::write(&other, gz.finish().unwrap()).unwrap();
        assert!(check_gz(&other, 1024).unwrap_err().0.contains("not a SQLite file"));
        assert!(
            backup_to_gzip(&d.path().join("missing.sqlite3"), &out)
                .unwrap_err()
                .0
                .contains("no such file")
        );
    }
}
