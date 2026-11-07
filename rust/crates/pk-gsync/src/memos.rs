//! Voice memos from the iPhone -> Punchable notes (`gsync/memos.py`), the Drive half only.
//!
//! Two halves, so the 60-second Docs sync never waits on Whisper:
//!
//! - **`MemoPull`** (here, inside every pass): Drive > Punchable > "Voice memos": list the audio files, skip court
//!   material by metadata and duplicates by Drive's md5Checksum, download a batch into `comedy/notes/inbox`
//!   (disk-guarded), then start the processor if it is not running. Drive is only READ: his memo files are never
//!   written, moved, renamed or deleted.
//! - **`MemoProcessor`** (`python -m gsync.memos process`, a Python with faster-whisper; started by this half,
//!   detached): ffmpeg and Whisper large-v3 have no Rust equivalent in this round, so the Python processor stays
//!   and runs exactly as before. It reads and writes the same `memos.json`, under the same lock file, which is why
//!   the store below speaks the Python's protocol byte for byte.
//!
//! State: `comedy/kb/gsync/memos.json`. Resumable: every step is recorded before the next one.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::LazyLock;
use std::time::{Duration, Instant, SystemTime};

use pk_core::{py, text::head};
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::config::{Config, FOLDER_MIME, MEMO_FOLDER_NAME};
use crate::error::{GsError, Result};
use crate::pyjson::{self, INDENT1_SORTED};
use crate::services::Services;
use crate::store::write_atomic;
use crate::sync::Sync;

const AUDIO_EXT: [&str; 13] = [
    ".m4a", ".mp3", ".wav", ".aac", ".caf", ".mp4", ".opus", ".ogg", ".flac", ".amr", ".3gp", ".webm", ".mov",
];
const NOT_COUNTED: [&str; 2] = ["skipped_legal", "duplicate"];
const MAX_TRIES: i64 = 3;

static STEM_CLEAN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^A-Za-z0-9 ._-]+").expect("regex"));

/// `Path(name).suffix` and `.stem`: the last dot after the first character of the final component.
fn split_name(name: &str) -> (String, String) {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    match last.rfind('.') {
        Some(i) if i > 0 && i + 1 < last.len() => (last[..i].to_string(), last[i..].to_string()),
        _ => (last.to_string(), String::new()),
    }
}

fn ext(name: &str) -> String {
    split_name(name).1.to_lowercase()
}

/// Is this Drive file an audio file?
pub fn is_audio(meta: &Value) -> bool {
    let mt = meta.get("mimeType").and_then(Value::as_str).unwrap_or_default();
    mt.starts_with("audio/")
        || matches!(mt, "video/mp4" | "video/quicktime" | "video/3gpp")
        || AUDIO_EXT.contains(&ext(meta.get("name").and_then(Value::as_str).unwrap_or_default()).as_str())
}

/// `memos.json` with the Python's cross-process lock file next to it.
pub struct MemoStore<'a> {
    path: std::path::PathBuf,
    svc: &'a Services,
}

impl<'a> MemoStore<'a> {
    /// The store at `path`.
    pub fn new(path: &Path, svc: &'a Services) -> Self {
        Self {
            path: path.to_path_buf(),
            svc,
        }
    }

    fn lock_path(&self) -> std::path::PathBuf {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.path.with_file_name(format!("{name}.lock"))
    }

    /// A cross-process lock file (gsync and the processor both write `memos.json`). Stale after 2 minutes.
    fn locked<R>(&self, f: impl FnOnce() -> Result<R>) -> Result<R> {
        let lock = self.lock_path();
        if let Some(dir) = lock.parent() {
            fs::create_dir_all(dir)?;
        }
        let t0 = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&lock) {
                Ok(mut fd) => {
                    let _ = fd.write_all(std::process::id().to_string().as_bytes());
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&lock)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok())
                        .is_some_and(|age| age > Duration::from_secs(120));
                    if stale {
                        let _ = fs::remove_file(&lock);
                        continue;
                    }
                    if t0.elapsed() > Duration::from_secs(60) {
                        return Err(GsError::other(
                            "TimeoutError",
                            format!("memos.json is locked ({})", lock.display()),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(e.into()),
            }
        }
        let out = f();
        let _ = fs::remove_file(&lock);
        out
    }

    /// The parsed file with its defaults; a missing or unreadable file is an empty one.
    pub fn load(&self) -> Value {
        let mut m = fs::read_to_string(&self.path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        for (k, d) in [
            ("version", json!(1)),
            ("items", json!({})),
            ("paused", Value::Null),
            ("processor", json!({})),
        ] {
            if m.get(k).is_none() {
                m[k] = d;
            }
        }
        m
    }

    /// Change the file under the lock: load, `f`, stamp `updated_at`, write.
    pub fn update(&self, f: impl FnOnce(&mut Value)) -> Result<()> {
        self.locked(|| {
            let mut m = self.load();
            f(&mut m);
            m["updated_at"] = json!(self.svc.clock.now_iso());
            write_atomic(&self.path, &pyjson::dumps(&m, INDENT1_SORTED))
        })
    }

    /// Set fields on one item; a `None` value removes the key.
    pub fn set_item(&self, key: &str, kw: Vec<(&str, Option<Value>)>) -> Result<()> {
        self.update(|m| {
            if m["items"].get(key).is_none() {
                m["items"][key] = json!({"id": key, "tries": 0});
            }
            let it = &mut m["items"][key];
            for (k, v) in kw {
                match v {
                    None => {
                        if let Some(o) = it.as_object_mut() {
                            o.shift_remove(k);
                        }
                    }
                    Some(v) => it[k] = v,
                }
            }
            it["updated_at"] = json!(self.svc.clock.now_iso());
        })
    }
}

/// The app's progress line: "23 of 140 voice memos transcribed".
pub fn progress_line(m: &Value) -> String {
    let mut c: BTreeMap<String, i64> = BTreeMap::new();
    for it in m["items"].as_object().into_iter().flat_map(Map::values) {
        *c.entry(py::str(it.get("status").unwrap_or(&Value::Null))).or_insert(0) += 1;
    }
    let total: i64 = c
        .iter()
        .filter(|(k, _)| !NOT_COUNTED.contains(&k.as_str()))
        .map(|(_, v)| v)
        .sum();
    let done = c.get("done").copied().unwrap_or(0);
    let mut line = if total > 0 {
        format!(
            "{done} of {total} voice memo{} transcribed",
            if total == 1 { "" } else { "s" }
        )
    } else {
        "No voice memos yet".to_string()
    };
    let paused = &m["paused"];
    if py::truthy(Some(paused)) && paused["reason"] == "low_disk" {
        let _ = write!(
            line,
            " \u{b7} paused: the PC is low on disk space ({} MB free)",
            py::str(paused.get("free_mb").unwrap_or(&Value::Null))
        );
    }
    line
}

/// Whether the processor holds its lock (`processor_running`). The lock is a byte-range lock the Python takes with
/// `msvcrt.locking` / `fcntl.flock`; trying to take it ourselves and failing means someone holds it.
pub fn processor_running(cfg: &Config) -> bool {
    let path = cfg.state().join("memos.processor.lock");
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let Ok(f) = OpenOptions::new().read(true).append(true).create(true).open(&path) else {
        return true;
    };
    match f.try_lock() {
        Ok(()) => {
            let _ = f.unlock();
            false
        }
        Err(_) => true,
    }
}

fn is_listed_or_stuck(it: &Value) -> bool {
    matches!(it["status"].as_str(), Some("downloaded" | "transcribing"))
}

fn sort_key(it: &Value) -> (String, String) {
    (
        it.get("drive_created")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        it.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
    )
}

fn size_of(v: &Value) -> i64 {
    match v.get("size") {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// One pass of the Drive side, run by `Sync::run` after the Docs work.
pub fn pull_memos(s: &mut Sync<'_>) -> Result<()> {
    if !s.cfg.memos_enabled {
        return Ok(());
    }
    let Some(parent) = s.store.m["folder_id"]
        .as_str()
        .filter(|p| !p.is_empty())
        .map(str::to_string)
    else {
        return Ok(()); // narrow scopes: no Punchable folder to look in
    };
    let store = MemoStore::new(&s.cfg.memos_state(), s.svc);
    let Some(folder) = memo_folder(s, &store, &parent)? else {
        return Ok(());
    };
    list_new(s, &store, &folder)?;
    if !s.dry {
        download_batch(s, &store)?;
        let m = store.load();
        let waiting = m["items"]
            .as_object()
            .is_some_and(|o| o.values().any(is_listed_or_stuck));
        if waiting && !processor_running(s.cfg) {
            (s.svc.spawn)(s.cfg)?;
            s.act("memo_processor_started", &[]);
        }
    }
    Ok(())
}

fn memo_folder(s: &mut Sync<'_>, store: &MemoStore<'_>, parent: &str) -> Result<Option<String>> {
    let m = store.load();
    if let Some(fid) = m["folder_id"].as_str().filter(|f| !f.is_empty()) {
        let meta = s.drive.get(fid)?;
        if meta.is_some_and(|m| !py::truthy(m.get("trashed"))) {
            return Ok(Some(fid.to_string()));
        }
    }
    let mut found = s.drive.find_by_prop("punchable_kind", "voice_memos")?;
    if found.is_none() {
        // one he made himself by hand
        found = s.drive.list_folder(parent)?.into_iter().find(|f| {
            f["mimeType"] == FOLDER_MIME
                && py::strip(f.get("name").and_then(Value::as_str).unwrap_or_default()).to_lowercase()
                    == MEMO_FOLDER_NAME.to_lowercase()
        });
    }
    let found = if let Some(f) = found {
        f
    } else {
        if s.dry {
            s.act("memo_folder_create", &[]);
            return Ok(None);
        }
        let f = s.drive.create_folder(
            MEMO_FOLDER_NAME,
            Some(parent),
            &json!({"punchable_kind": "voice_memos"}),
        )?;
        s.act("memo_folder_create", &[("doc", f["id"].clone())]);
        f
    };
    let id = found["id"].clone();
    store.update(|m| m["folder_id"] = id.clone())?;
    Ok(id.as_str().map(str::to_string))
}

fn walk(s: &mut Sync<'_>, folder: &str, depth: u32) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for f in s.drive.list_folder(folder)? {
        if f["mimeType"] == FOLDER_MIME {
            if depth < 2 {
                // "Voice memos/2019" etc.
                out.extend(walk(s, f["id"].as_str().unwrap_or_default(), depth + 1)?);
            }
        } else if !py::truthy(f.get("trashed")) && is_audio(&f) {
            out.push(f);
        }
    }
    Ok(out)
}

fn list_new(s: &mut Sync<'_>, store: &MemoStore<'_>, folder: &str) -> Result<()> {
    let mut files = walk(s, folder, 0)?;
    let m = store.load();
    let known = m["items"].as_object().cloned().unwrap_or_default();
    let mut md5s: BTreeMap<String, String> = BTreeMap::new();
    for (k, it) in &known {
        if let Some(md5) = it.get("md5").and_then(Value::as_str).filter(|m| !m.is_empty())
            && it["status"] != "skipped_legal"
        {
            md5s.insert(md5.to_string(), k.clone());
        }
    }
    files.sort_by_key(|f| {
        (
            f.get("createdTime")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            f.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
        )
    });
    let mut new: Vec<(String, Value)> = Vec::new();
    for f in files {
        let id = py::str(&f["id"]);
        if known.contains_key(&id) {
            continue;
        }
        let anc = s.ancestors(&f)?;
        let excluded = Map::new();
        if let Some(why) = s.rules.legal_reason(&f, &anc, &excluded) {
            // metadata only: never downloaded; only the id and the reason are kept
            let why = why.split(':').next().unwrap_or_default().to_string();
            new.push((
                id.clone(),
                json!({"id": id, "source": "drive", "drive_id": id, "status": "skipped_legal", "why": why, "tries": 0}),
            ));
            s.act("memo_skip_legal", &[("doc", json!(id)), ("why", json!(why))]);
            continue;
        }
        let md5 = f.get("md5Checksum").cloned().unwrap_or(Value::Null);
        let mut it = json!({"id": id, "source": "drive", "drive_id": id, "name": f.get("name"), "md5": md5,
                            "size": size_of(&f), "drive_created": f.get("createdTime"), "tries": 0});
        let md5_text = md5.as_str().filter(|m| !m.is_empty());
        if let Some(twin) = md5_text.and_then(|m| md5s.get(m)).cloned() {
            it["status"] = json!("duplicate");
            it["dup_of"] = json!(twin);
            s.act("memo_duplicate", &[("doc", json!(id))]);
        } else {
            it["status"] = json!("listed");
            if let Some(m) = md5_text {
                md5s.insert(m.to_string(), id.clone());
            }
            s.act("memo_new", &[("doc", json!(id))]);
        }
        new.push((id, it));
    }
    if !new.is_empty() && !s.dry {
        store.update(|m| {
            for (k, it) in new {
                if m["items"].get(&k).is_none() {
                    let mut it = it;
                    it["updated_at"] = json!(s.svc.clock.now_iso());
                    m["items"][&k] = it;
                }
            }
        })?;
    }
    Ok(())
}

#[allow(clippy::many_single_char_names)] // the Python's own names
fn download_batch(s: &mut Sync<'_>, store: &MemoStore<'_>) -> Result<()> {
    let m = store.load();
    let items = m["items"].as_object().cloned().unwrap_or_default();
    let mut staged: i64 = items.values().filter(|it| is_listed_or_stuck(it)).map(size_of).sum();
    let mut todo: Vec<Value> = items.values().filter(|it| it["status"] == "listed").cloned().collect();
    todo.sort_by_key(sort_key);
    let inbox = s.cfg.notes_inbox();
    fs::create_dir_all(&inbox)?;
    let stage_cap = s.cfg.memo_stage_mb.cast_signed() * 1_000_000;
    let mut paused = Value::Null;
    let mut n = 0usize;
    for it in todo {
        if n >= s.cfg.memo_batch {
            break;
        }
        let size = size_of(&it);
        if staged != 0 && staged + size > stage_cap {
            break; // the processor catches up first; the next pass downloads more
        }
        let free = (s.svc.free_mb)(&inbox);
        let need = s.cfg.memo_reserve_mb.cast_signed() + 4 * size / 1_000_000 + 1;
        if free.cast_signed() < need {
            paused = json!({"reason": "low_disk", "free_mb": free, "need_mb": need, "at": s.svc.clock.now_iso()});
            s.act("memo_paused_low_disk", &[("n", json!(free))]);
            break;
        }
        let name = it
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .unwrap_or("memo");
        let stem = split_name(name).0;
        let cleaned = STEM_CLEAN.replace_all(&stem, "_");
        let stem = head(py::strip(&cleaned), 60);
        let stem = if stem.is_empty() { "memo".to_string() } else { stem };
        let id = py::str(&it["id"]);
        let e = ext(it.get("name").and_then(Value::as_str).unwrap_or_default());
        let fname = format!(
            "{}_{stem}{}",
            head(&id, 12),
            if e.is_empty() { ".m4a".to_string() } else { e }
        );
        let dest = inbox.join(&fname);
        let part = inbox.join(format!("{fname}.part"));
        // claim the file name first, so the processor never mistakes it for a file he dropped by hand
        store.set_item(&id, vec![("file", Some(json!(fname)))])?;
        let got = s
            .drive
            .download(it["drive_id"].as_str().unwrap_or_default(), &part)
            .and_then(|g| fs::rename(&part, &dest).map(|()| g).map_err(GsError::from));
        let got = match got {
            Ok(g) => g,
            Err(e @ GsError::Exit(_)) => return Err(e),
            Err(e) => {
                // retried next pass, up to MAX_TRIES
                let _ = fs::remove_file(&part);
                let tries = it.get("tries").and_then(Value::as_i64).unwrap_or(0) + 1;
                let msg = head(&format!("download: {}", e.describe()), 300);
                store.set_item(
                    &id,
                    vec![
                        ("tries", Some(json!(tries))),
                        ("file", None),
                        ("error", Some(json!(msg))),
                        (
                            "status",
                            Some(json!(if tries >= MAX_TRIES { "failed" } else { "listed" })),
                        ),
                    ],
                )?;
                s.error(json!({"memo": id, "error": msg}));
                continue;
            }
        };
        let kept = if got != 0 { got.cast_signed() } else { size };
        store.set_item(
            &id,
            vec![
                ("status", Some(json!("downloaded"))),
                ("file", Some(json!(fname))),
                ("size", Some(json!(kept))),
                ("error", None),
            ],
        )?;
        s.act("memo_download", &[("doc", json!(id))]);
        staged += kept;
        n += 1;
    }
    store.update(|m| m["paused"] = paused)
}
