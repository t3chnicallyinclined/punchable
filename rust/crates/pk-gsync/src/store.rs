//! `comedy/kb/gsync/` (`gsync/store.py`): `map.json` (atomic), the append-only `runs` / `conflicts` / `edits` logs,
//! `units.json` and `mirror/`. The files are byte-for-byte what the Python wrote.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use pk_core::py;
use serde_json::{Map, Value, json};

use crate::error::Result;
use crate::pyjson::{self, COMPACT, INDENT1, INDENT1_SORTED};

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Write `text` to `p` through a temp file in the same folder and an atomic rename (`write_atomic`).
pub fn write_atomic(p: &Path, text: &str) -> Result<()> {
    let dir = p.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        "{name}.{}{}.tmp",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| -> std::io::Result<()> {
        let mut f = File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.flush()?;
        f.sync_all()?;
        fs::rename(&tmp, p)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(written?)
}

/// Append one compact JSON line to `p`.
pub fn append_jsonl(p: &Path, rec: &Value) -> Result<()> {
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(p)?;
    f.write_all((pyjson::dumps(rec, COMPACT) + "\n").as_bytes())?;
    f.flush()?;
    f.sync_all()?;
    Ok(())
}

/// The parsed lines of a `.jsonl` file; a torn line is skipped. Lines split the way `str.splitlines` splits.
pub fn read_jsonl(p: &Path) -> Vec<Value> {
    let Ok(text) = fs::read_to_string(p) else {
        return Vec::new();
    };
    py::splitlines(&text)
        .into_iter()
        .filter_map(|ln| serde_json::from_str::<Value>(ln).ok())
        .collect()
}

/// The sync map and the files beside it.
pub struct Store {
    /// `<comedy>/kb/gsync`.
    pub root: PathBuf,
    /// `map.json`'s parsed content.
    pub m: Value,
    /// Replace `map.json` on [`Store::save`] (off for a dry run is the caller's rule, not the store's).
    map_path: PathBuf,
    units_path: PathBuf,
    mirror: PathBuf,
}

impl Store {
    /// Open the folder, reading `map.json` when it exists.
    pub fn open(root: &Path) -> Result<Self> {
        let map_path = root.join("map.json");
        let m = if map_path.exists() {
            serde_json::from_str(&fs::read_to_string(&map_path)?)?
        } else {
            json!({"version": 1, "changes_token": null, "docs": {}, "boards": {}, "excluded": {}, "skipped": {},
                   "pending": {}, "folders": {}})
        };
        Ok(Self {
            root: root.to_path_buf(),
            m,
            units_path: root.join("units.json"),
            mirror: root.join("mirror"),
            map_path,
        })
    }

    /// Write `map.json`: indent 1, sorted keys.
    pub fn save(&self) -> Result<()> {
        write_atomic(&self.map_path, &pyjson::dumps(&self.m, INDENT1_SORTED))
    }

    /// Append a record to one of the logs (`runs.jsonl`, `conflicts.jsonl`, `edits.jsonl`).
    pub fn log(&self, name: &str, rec: &Value) -> Result<()> {
        append_jsonl(&self.root.join(name), rec)
    }

    /// Replace this doc's units in `units.json`.
    pub fn put_units(&self, doc_id: &str, units: Vec<Value>) -> Result<()> {
        let mut all: Vec<Value> = if self.units_path.exists() {
            match serde_json::from_str::<Value>(&fs::read_to_string(&self.units_path)?)? {
                Value::Array(a) => a,
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
        all.retain(|u| u.get("doc_id").and_then(Value::as_str) != Some(doc_id));
        all.extend(units);
        write_atomic(&self.units_path, &pyjson::dumps(&Value::Array(all), INDENT1))
    }

    /// Write a mirror file; returns its path relative to the data folder, with forward slashes.
    pub fn put_mirror(&self, name: &str, text: &str) -> Result<String> {
        write_atomic(&self.mirror.join(name), text)?;
        // relative to root.parent.parent, as the Python: <comedy>/kb/gsync -> <comedy>
        let base = self.root.parent().and_then(Path::parent).unwrap_or(&self.root);
        let rel = self.mirror.join(name);
        let rel = rel.strip_prefix(base).unwrap_or(&rel);
        Ok(rel.to_string_lossy().replace('\\', "/"))
    }

    /// `m[key]` as an object (the sync keeps these as maps).
    pub fn table(&mut self, key: &str) -> &mut Map<String, Value> {
        if !self.m[key].is_object() {
            self.m[key] = json!({});
        }
        self.m[key].as_object_mut().expect("just made an object")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_roundtrip_is_python_bytes_and_a_torn_line_is_skipped() {
        let d = tempfile::tempdir().expect("tmp");
        let root = d.path().join("comedy").join("kb").join("gsync");
        let mut s = Store::open(&root).expect("open");
        s.m["docs"]["d1"] = json!({"title": "é", "version": "3"});
        s.save().expect("save");
        let text = fs::read_to_string(root.join("map.json")).expect("read");
        assert!(
            text.starts_with("{\n \"boards\": {},\n \"changes_token\": null,"),
            "{text}"
        );
        assert!(text.contains("\n   \"title\": \"é\""), "{text}");
        let again = Store::open(&root).expect("reopen");
        assert_eq!(again.m, s.m);
        let rel = s.put_mirror("a.md", "x").expect("mirror");
        assert_eq!(rel, "kb/gsync/mirror/a.md");
        s.log("conflicts.jsonl", &json!({"a": 1})).expect("log");
        let mut f = OpenOptions::new()
            .append(true)
            .open(root.join("conflicts.jsonl"))
            .expect("open");
        f.write_all(b"{\"torn\"").expect("write");
        assert_eq!(read_jsonl(&root.join("conflicts.jsonl")), vec![json!({"a": 1})]);
    }
}
