//! The dump: `SurrealDB`'s export streamed into a gzip file, and the checks a dump must pass before and after it
//! leaves the machine.
//!
//! Recordings the phone uploaded wait in the database for a transcriber (docs/adr/0014): bulky, transient and sent
//! again by the phone, so a backup carries their table DEFINITIONS (a restore needs the indexes) and none of their
//! rows. The export is therefore two `POST /export` requests: every other table in full, then the three upload tables
//! with `records: false` and the other kinds switched off, whose second `OPTION IMPORT;` is dropped (an import refuses
//! to see it twice, seen on 3.3.4).

use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::time::Duration;

use base64::Engine;
use flate2::write::GzEncoder;
use flate2::{Compression, GzBuilder};
use reqwest::blocking::{Client, Response};
use reqwest::{StatusCode, Url};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::{BackupError, Result, fail};
use crate::gzread::PyGzip;
use crate::settings::Vars;
use crate::{CHUNK, s3};

/// Tables whose rows are left out of a backup (their definitions stay).
pub const EXCLUDED_TABLES: [&str; 3] = ["upload", "upload_part", "upload_stage"];

/// What a good dump looks like.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpInfo {
    /// The size of the gzip file.
    pub bytes_gz: u64,
    /// The size of the `SurrealQL` inside.
    pub bytes_sql: u64,
    /// `DEFINE TABLE` statements counted at a line start.
    pub tables: u64,
    /// SHA-256 of the gzip file, hex.
    pub sha256: String,
}

/// Where and as whom to export.
#[derive(Debug, Clone)]
pub struct ExportConfig {
    url: Url,
    user: String,
    pass: String,
    ns: String,
    db: String,
    database_level: bool,
    client: Client,
}

impl ExportConfig {
    /// From `SURREAL_URL`, `SURREAL_USER` / `SURREAL_PASS`, `SURREAL_NS` / `SURREAL_DB`, `SURREAL_AUTH_LEVEL`, checked in
    /// the order the Python checks them.
    pub fn from_vars(v: &Vars, extra_roots_pem: &[Vec<u8>]) -> Result<Self> {
        let raw = v.env("SURREAL_URL", Some("http://surrealdb:8000"))?;
        let url = Url::parse(&raw)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
        let Some(url) = url else {
            return fail("SURREAL_URL must be http(s)://host:port");
        };
        let user = v.env("SURREAL_USER", None)?;
        let pass = v.env("SURREAL_PASS", None)?;
        let ns = v.env("SURREAL_NS", Some("punchable"))?;
        let db = v.env("SURREAL_DB", Some("main"))?;
        let database_level = match v.get_or("SURREAL_AUTH_LEVEL", "database") {
            "database" => true,
            "root" => false,
            _ => return fail("SURREAL_AUTH_LEVEL must be database or root"),
        };
        Ok(Self {
            url,
            user,
            pass,
            ns,
            db,
            database_level,
            client: s3::http_client(extra_roots_pem)?,
        })
    }

    /// The namespace.
    pub fn ns(&self) -> &str {
        &self.ns
    }

    /// The database.
    pub fn db(&self) -> &str {
        &self.db
    }

    /// `scheme://host:port` plus the URL's path without a trailing slash, as the Python builds it.
    fn base(&self) -> String {
        format!(
            "{}{}",
            self.url.origin().ascii_serialization(),
            self.url.path().trim_end_matches('/')
        )
    }

    fn post(&self, path: &str, content_type: &str, accept: &str, body: Vec<u8>) -> Result<Response> {
        let auth = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", self.user, self.pass));
        let mut rb = self
            .client
            .post(format!("{}{path}", self.base()))
            .timeout(Duration::from_mins(25))
            .header("Authorization", format!("Basic {auth}"))
            .header("Surreal-NS", &self.ns)
            .header("Surreal-DB", &self.db)
            .header("Accept", accept)
            .header("Content-Type", content_type);
        if self.database_level {
            // SurrealDB 3.x checks Basic credentials against a DATABASE user only when these two headers name it;
            // without them it tries root and answers 401.
            rb = rb
                .header("Surreal-Auth-NS", &self.ns)
                .header("Surreal-Auth-DB", &self.db);
        }
        let r = rb
            .body(body)
            .send()
            .map_err(|e| BackupError(format!("export: {}", chain(&e))))?;
        if r.status() != StatusCode::OK {
            let status = r.status().as_u16();
            let mut text = Vec::new();
            let _ = r.take(300).read_to_end(&mut text);
            return fail(format!(
                "{}: HTTP {status} {}",
                path.rsplit('/').next().unwrap_or(path),
                String::from_utf8_lossy(&text).trim()
            ));
        }
        Ok(r)
    }

    /// Every table of the database (`INFO FOR DB`), sorted.
    pub fn tables(&self) -> Result<Vec<String>> {
        let mut r = self.post("/sql", "text/plain", "application/json", b"INFO FOR DB;".to_vec())?;
        let mut body = Vec::new();
        r.read_to_end(&mut body)
            .map_err(|e| BackupError(format!("export: {}: {e}", kind(&e))))?;
        let parsed: Option<Vec<String>> = serde_json::from_slice::<Value>(&body).ok().and_then(|v| {
            let mut names: Vec<String> = v
                .get(0)?
                .get("result")?
                .get("tables")?
                .as_object()?
                .keys()
                .cloned()
                .collect();
            names.sort();
            Some(names)
        });
        parsed.ok_or_else(|| BackupError("INFO FOR DB did not answer a table list".into()))
    }

    /// Stream the export for the namespace / database into a gzip file: every table's definition and rows except
    /// [`EXCLUDED_TABLES`]' rows, then those tables' definitions alone. Returns the uncompressed byte count.
    pub fn export_to_gzip(&self, dest: &Path) -> Result<u64> {
        let tables = self.tables()?;
        let kept: Vec<&String> = tables
            .iter()
            .filter(|t| !EXCLUDED_TABLES.contains(&t.as_str()))
            .collect();
        let left_out: Vec<&String> = tables
            .iter()
            .filter(|t| EXCLUDED_TABLES.contains(&t.as_str()))
            .collect();
        if kept.is_empty() {
            return fail("export: the database has no table to back up");
        }
        let mut parts = vec![json!({ "tables": kept })];
        if !left_out.is_empty() {
            parts.push(json!({
                "tables": left_out, "records": false, "users": false, "accesses": false, "params": false,
                "functions": false, "analyzers": false,
            }));
        }
        let raw = File::create(dest).map_err(|e| BackupError(format!("export: {}: {e}", kind(&e))))?;
        let mut gz = GzBuilder::new().mtime(0).write(raw, Compression::new(6));
        let mut n = 0u64;
        for (k, cfg) in parts.iter().enumerate() {
            let mut r = self.post(
                "/export",
                "application/json",
                "text/plain",
                cfg.to_string().into_bytes(),
            )?;
            let wrote = |e: std::io::Error| BackupError(format!("export: {}: {e}", kind(&e)));
            if k > 0 {
                // the definitions-only part is a few KB: read it whole and drop its `OPTION IMPORT;`
                let mut chunk = Vec::new();
                r.read_to_end(&mut chunk).map_err(wrote)?;
                let marker = b"OPTION IMPORT;\n";
                if let Some(i) = chunk.windows(marker.len()).position(|w| w == marker) {
                    chunk.drain(i..i + marker.len());
                }
                gz.write_all(&chunk).map_err(wrote)?;
                n += chunk.len() as u64;
                continue;
            }
            let mut buf = vec![0u8; CHUNK];
            loop {
                // a dropped chunked stream raises here
                let got = r.read(&mut buf).map_err(wrote)?;
                if got == 0 {
                    break;
                }
                gz.write_all(&buf[..got]).map_err(wrote)?;
                n += got as u64;
            }
        }
        finish(gz).map_err(|e| BackupError(format!("export: {}: {e}", kind(&e))))?;
        Ok(n)
    }
}

fn finish(gz: GzEncoder<File>) -> std::io::Result<()> {
    let mut f = gz.finish()?;
    f.flush()
}

fn chain(e: &reqwest::Error) -> String {
    let mut s = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(inner) = src {
        s.push_str(": ");
        s.push_str(&inner.to_string());
        src = inner.source();
    }
    s
}

fn kind(e: &std::io::Error) -> String {
    format!("{:?}", e.kind())
}

fn base_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Read the whole gzip (CRC and length are checked by the reader at the end) and look at the `SurrealQL` inside. Fails
/// when the file is not a complete, plausible export. `min_bytes` defaults to `BACKUP_MIN_BYTES` (4096).
pub fn check_dump(path: &Path, min_bytes: Option<u64>, vars: &Vars) -> Result<DumpInfo> {
    let min_bytes = match min_bytes {
        Some(m) => m,
        None => u64::try_from(vars.int("BACKUP_MIN_BYTES", 4096, 1)?).unwrap_or(4096),
    };
    check_dump_chunked(path, min_bytes, CHUNK)
}

/// [`check_dump`] with the read size chosen (the tests make it tiny to put a `DEFINE TABLE` across a boundary).
pub fn check_dump_chunked(path: &Path, min_bytes: u64, chunk: usize) -> Result<DumpInfo> {
    const NEEDLE: &[u8] = b"\nDEFINE TABLE ";
    if !path.is_file() {
        return fail(format!("{}: no such file", path.display()));
    }
    let name = base_name(path);
    let incomplete = |e: &std::io::Error| BackupError(format!("{name}: not a complete gzip ({}: {e})", kind(e)));
    let (mut n, mut tables) = (0u64, 0u64);
    let (mut head, mut tail, mut carry): (Vec<u8>, Vec<u8>, Vec<u8>) = (Vec::new(), Vec::new(), Vec::new());
    let sha = hash_file(path).map_err(|e| incomplete(&e))?;
    let f = File::open(path).map_err(|e| incomplete(&e))?;
    let mut g = PyGzip::new(BufReader::new(f));
    let mut buf = vec![0u8; chunk.max(1)];
    loop {
        let got = g.read(&mut buf).map_err(|e| incomplete(&e))?;
        if got == 0 {
            break;
        }
        let part = &buf[..got];
        if head.len() < 4096 {
            head.extend_from_slice(part);
            head.truncate(4096);
        }
        // count line-start matches once, also across a chunk boundary
        let mut joined = std::mem::take(&mut carry);
        let carried = joined.len();
        joined.extend_from_slice(part);
        tables += count(&joined, NEEDLE, carried);
        let keep = joined.len().min(NEEDLE.len() - 1);
        carry = joined[joined.len() - keep..].to_vec();
        tail.extend_from_slice(part);
        if tail.len() > 256 {
            tail.drain(..tail.len() - 256);
        }
        n += got as u64;
    }
    if n < min_bytes {
        return fail(format!(
            "{name}: export is only {n} bytes (BACKUP_MIN_BYTES={min_bytes})"
        ));
    }
    if !contains(&head, b"OPTION IMPORT;") {
        return fail(format!(
            "{name}: does not start like a SurrealDB export (no OPTION IMPORT)"
        ));
    }
    if tables < 1 {
        return fail(format!("{name}: no DEFINE TABLE in the export"));
    }
    let trimmed = tail
        .iter()
        .rposition(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c));
    if trimmed.is_none_or(|i| tail[i] != b';') {
        return fail(format!("{name}: export does not end with a complete statement"));
    }
    Ok(DumpInfo {
        bytes_gz: std::fs::metadata(path).map_err(|e| incomplete(&e))?.len(),
        bytes_sql: n,
        tables,
        sha256: sha,
    })
}

/// Matches of `needle` in `hay` that end after the first `skip_end_before` bytes (those lie in the carried part and
/// were counted with the chunk before).
fn count(hay: &[u8], needle: &[u8], carried: usize) -> u64 {
    let mut c = 0;
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if &hay[i..i + needle.len()] == needle {
            if i + needle.len() > carried {
                c += 1;
            }
            i += needle.len();
        } else {
            i += 1;
        }
    }
    c
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// SHA-256 of a file, hex.
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut sha = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let got = f.read(&mut buf)?;
        if got == 0 {
            return Ok(hex::encode(sha.finalize()));
        }
        sha.update(&buf[..got]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "-- ------------------------------\n-- OPTION\n-- ------------------------------\n\nOPTION IMPORT;\n\n\
        DEFINE DATABASE OVERWRITE $session.db STRICT;\n\
        DEFINE TABLE joke TYPE NORMAL SCHEMAFULL CHANGEFEED 30d INCLUDE ORIGINAL PERMISSIONS NONE;\n\
        DEFINE TABLE unit TYPE NORMAL SCHEMAFULL PERMISSIONS NONE;\n";

    fn sql() -> Vec<u8> {
        let mut s = HEAD.as_bytes().to_vec();
        for i in 0..400 {
            s.extend_from_slice(format!("INSERT [ {{ id: unit:u{i}, text: 'line {i}' }} ];\n").as_bytes());
        }
        s
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut e = GzBuilder::new().mtime(0).write(Vec::new(), Compression::new(6));
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    fn write(dir: &Path, data: &[u8]) -> std::path::PathBuf {
        let p = dir.join("d.surql.gz");
        std::fs::write(&p, data).unwrap();
        p
    }

    #[test]
    fn accepts_a_good_export() {
        let d = tempfile::tempdir().unwrap();
        let s = sql();
        let info = check_dump_chunked(&write(d.path(), &gzip(&s)), 1024, CHUNK).unwrap();
        assert_eq!((info.tables, info.bytes_sql), (2, s.len() as u64));
        assert_eq!(info.sha256.len(), 64);
    }

    #[test]
    fn counts_tables_across_a_chunk_boundary() {
        let d = tempfile::tempdir().unwrap();
        let p = write(d.path(), &gzip(&sql()));
        for chunk in [1, 3, 7, 13, 16, 17, 4096] {
            assert_eq!(check_dump_chunked(&p, 1024, chunk).unwrap().tables, 2, "chunk {chunk}");
        }
    }

    #[test]
    fn rejects_a_truncated_gzip() {
        let d = tempfile::tempdir().unwrap();
        let raw = gzip(&sql());
        let e = check_dump_chunked(&write(d.path(), &raw[..raw.len() / 2]), 1024, CHUNK).unwrap_err();
        assert!(e.0.contains("not a complete gzip"), "{e}");
    }

    #[test]
    fn rejects_a_bad_crc() {
        let d = tempfile::tempdir().unwrap();
        let mut raw = gzip(&sql());
        let n = raw.len();
        raw[n - 8] ^= 0xFF; // the CRC32 field of the gzip trailer
        let e = check_dump_chunked(&write(d.path(), &raw), 1024, CHUNK).unwrap_err();
        assert!(e.0.contains("not a complete gzip"), "{e}");
    }

    #[test]
    fn rejects_a_tiny_export() {
        let d = tempfile::tempdir().unwrap();
        let e = check_dump_chunked(&write(d.path(), &gzip(&sql()[..300])), 1024, CHUNK).unwrap_err();
        assert!(e.0.contains("only"), "{e}");
    }

    #[test]
    fn rejects_an_export_without_tables() {
        let d = tempfile::tempdir().unwrap();
        let s = String::from_utf8(sql())
            .unwrap()
            .replace("DEFINE TABLE", "DEFINE FIELD");
        let e = check_dump_chunked(&write(d.path(), &gzip(s.as_bytes())), 1024, CHUNK).unwrap_err();
        assert!(e.0.contains("DEFINE TABLE"), "{e}");
    }

    #[test]
    fn rejects_a_cut_statement() {
        let d = tempfile::tempdir().unwrap();
        let s = sql();
        let e = check_dump_chunked(&write(d.path(), &gzip(&s[..s.len() - 20])), 1024, CHUNK).unwrap_err();
        assert!(e.0.contains("complete statement"), "{e}");
    }

    #[test]
    fn rejects_a_non_export() {
        let d = tempfile::tempdir().unwrap();
        let e = check_dump_chunked(&write(d.path(), &gzip(&b"hello;\n".repeat(2000))), 1024, CHUNK).unwrap_err();
        assert!(e.0.contains("OPTION IMPORT"), "{e}");
    }

    #[test]
    fn an_empty_file_is_read_as_an_empty_stream_like_pythons_gzip() {
        let d = tempfile::tempdir().unwrap();
        let e = check_dump_chunked(&write(d.path(), b""), 1024, CHUNK).unwrap_err();
        assert!(e.0.contains("export is only 0 bytes"), "{e}");
    }

    #[test]
    fn rejects_a_missing_file() {
        let e = check_dump_chunked(Path::new("no-such-file.gz"), 1024, CHUNK).unwrap_err();
        assert!(e.0.contains("no such file"), "{e}");
    }

    #[test]
    fn the_needle_counter_skips_what_the_carry_already_counted() {
        assert_eq!(count(b"x\nDEFINE TABLE a\nDEFINE TABLE b", b"\nDEFINE TABLE ", 0), 2);
        assert_eq!(count(b"x\nDEFINE TABLE a\nDEFINE TABLE b", b"\nDEFINE TABLE ", 15), 1);
    }
}
