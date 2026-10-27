"""Voice memos from the iPhone -> Punchable notes (the comic, 2026-10-26: "we need a way to import all local voice notes
from iphone"). See GSYNC.md "Voice memos".

Two halves, so the 60-second Docs sync never waits on Whisper:

  MemoPull       (inside every gsync pass, the gsync venv)  Drive > Punchable > "Voice memos": list the audio files,
                 skip court material by metadata (rules.legal_reason) and duplicates by Drive's md5Checksum, download a
                 batch into comedy/notes/inbox (disk-guarded), then start the processor if it is not running.
                 Drive is only READ here: his memo files are never written, moved, renamed or deleted.
  MemoProcessor  (`python -m gsync.memos process`, a Python with faster-whisper; started by MemoPull, detached)
                 one memo at a time: sha256 dedupe, the recorded date from the file, ffmpeg -> 16 kHz wav + 32 kbps
                 Opus, faster-whisper large-v3 (engine/audio.py), the content legal net on the transcript, then
                 POST /api/ideas (which auto-attaches it to the bit it clearly belongs to, or leaves a new idea).
                 The wav and the downloaded copy are deleted afterwards (Drive keeps the original); the Opus copy
                 stays in tapes/processed/<date>_<sha8>/ for playback, the transcript in corpus/memos/<date>_<sha8>/.

State: comedy/kb/gsync/memos.json (both halves write it under a lock file; the API reads it for the Notes list).
Resumable: every step is recorded before the next one, a crashed memo goes back to the queue, and a memo already
filed (its decision line exists) is never filed twice.
"""
from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

from . import rules
from .config import FOLDER_MIME, IDEA_MAX, MEMO_FOLDER_NAME, Config
from .punchable import GDOCS, ApiDown, Punchable
from .store import now_iso, write_atomic

DESK = Path(__file__).resolve().parent.parent
AUDIO_EXT = {".m4a", ".mp3", ".wav", ".aac", ".caf", ".mp4", ".opus", ".ogg", ".flac", ".amr", ".3gp", ".webm", ".mov"}
WAITING = ("listed", "downloaded", "transcribing")
NOT_COUNTED = ("skipped_legal", "duplicate")
MAX_TRIES = 3
PROC_ALIVE_S = 20 * 60


def _ext(name: str) -> str:
    return Path(name or "").suffix.lower()


def is_audio(meta: dict) -> bool:
    mt = meta.get("mimeType") or ""
    return mt.startswith("audio/") or mt in ("video/mp4", "video/quicktime", "video/3gpp") or _ext(meta.get("name")) in AUDIO_EXT


# ---------------------------------------------------------------------------------------------------------- the store
@contextlib.contextmanager
def _locked(path: Path, timeout: float = 60.0):
    """A cross-process lock file next to memos.json (gsync and the processor both write it). Stale after 2 minutes."""
    lock = path.with_name(path.name + ".lock")
    lock.parent.mkdir(parents=True, exist_ok=True)
    t0 = time.monotonic()
    while True:
        try:
            fd = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            os.write(fd, str(os.getpid()).encode())
            os.close(fd)
            break
        except FileExistsError:
            try:
                if time.time() - lock.stat().st_mtime > 120:
                    lock.unlink(missing_ok=True)
                    continue
            except FileNotFoundError:
                continue
            if time.monotonic() - t0 > timeout:
                raise TimeoutError(f"memos.json is locked ({lock})")
            time.sleep(0.05)
    try:
        yield
    finally:
        lock.unlink(missing_ok=True)


class MemoStore:
    """memos.json: {version, folder_id, items: {key: Item}, paused: {reason, free_mb, need_mb, at}|null,
    processor: {pid, at}}. Item: {id, source: drive|local, status: listed|downloaded|transcribing|done|failed|
    skipped_legal|duplicate, drive_id?, name, md5?, size, drive_created?, file?, sha256?, title?, recorded_at?, date?,
    date_source?, duration_s?, name_dir?, audio?, transcript?, chars?, joke_id?, attach?, long?, no_speech?, why?,
    dup_of?, error?, tries, updated_at}. A legal item keeps its id and reason only (no name, no text)."""

    def __init__(self, path: Path):
        self.path = path

    def load(self) -> dict:
        try:
            m = json.loads(self.path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            m = {}
        m.setdefault("version", 1)
        m.setdefault("items", {})
        m.setdefault("paused", None)
        m.setdefault("processor", {})
        return m

    def update(self, fn) -> dict:
        with _locked(self.path):
            m = self.load()
            out = fn(m)
            m["updated_at"] = now_iso()
            write_atomic(self.path, json.dumps(m, ensure_ascii=False, indent=1, sort_keys=True))
            return out if out is not None else m

    def set_item(self, key: str, **kw) -> dict:
        def f(m):
            it = m["items"].setdefault(key, {"id": key, "tries": 0})
            for k, v in kw.items():
                if v is None:
                    it.pop(k, None)
                else:
                    it[k] = v
            it["updated_at"] = now_iso()
            return dict(it)
        return self.update(f)


def progress(m: dict, now: float | None = None) -> dict:
    """The app's progress line: "23 of 140 voice memos transcribed"."""
    items = list(m.get("items", {}).values())
    c = {}
    for it in items:
        c[it.get("status")] = c.get(it.get("status"), 0) + 1
    total = sum(v for k, v in c.items() if k not in NOT_COUNTED)
    done = c.get("done", 0)
    waiting = sum(c.get(k, 0) for k in WAITING)
    pr = m.get("processor") or {}
    working = bool(waiting) and bool(pr.get("at")) and (now or time.time()) - float(pr.get("ts") or 0) < PROC_ALIVE_S
    paused = m.get("paused")
    line = f"{done} of {total} voice memo{'s' if total != 1 else ''} transcribed" if total else "No voice memos yet"
    if paused and paused.get("reason") == "low_disk":
        line += f" · paused: the PC is low on disk space ({paused.get('free_mb')} MB free)"
    return {"total": total, "transcribed": done, "waiting": waiting, "failed": c.get("failed", 0),
            "duplicates": c.get("duplicate", 0), "skipped_legal": c.get("skipped_legal", 0),
            "in_drive": c.get("listed", 0), "working": working, "paused": paused, "line": line,
            "folder_id": m.get("folder_id")}


def free_mb(p: Path) -> int:
    q = p
    while not q.exists() and q != q.parent:
        q = q.parent
    return int(shutil.disk_usage(q).free / 1e6)


# ---------------------------------------------------------------------------------------------------------- processor lock
class ProcLock:
    """Held by the running processor for its whole life; the OS releases it if the process dies."""

    def __init__(self, path: Path):
        self.path, self.f = path, None

    def acquire(self) -> bool:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        f = open(self.path, "a+b")
        try:
            if os.name == "nt":
                import msvcrt
                f.seek(0)
                msvcrt.locking(f.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            f.close()
            return False
        self.f = f
        return True

    def release(self) -> None:
        if not self.f:
            return
        try:
            if os.name == "nt":
                import msvcrt
                self.f.seek(0)
                msvcrt.locking(self.f.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                import fcntl
                fcntl.flock(self.f, fcntl.LOCK_UN)
        except OSError:
            pass
        self.f.close()
        self.f = None


def proc_lock_path(cfg: Config) -> Path:
    return cfg.state / "memos.processor.lock"


def processor_running(cfg: Config) -> bool:
    lk = ProcLock(proc_lock_path(cfg))
    if lk.acquire():
        lk.release()
        return False
    return True


def spawn_processor(cfg: Config) -> None:
    """Start `python -m gsync.memos process` detached, in a Python that has faster-whisper."""
    py = cfg.whisper_python if Path(cfg.whisper_python).is_file() else sys.executable
    log = cfg.comedy / "db" / "punchable" / "logs" / "memos.log"
    log.parent.mkdir(parents=True, exist_ok=True)
    flags = (0x00000200 | 0x08000000) if os.name == "nt" else 0  # CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
    env = {**os.environ, "PYTHONUTF8": "1", "COMEDY_DIR": str(cfg.comedy), "PUNCHABLE_API": cfg.api}
    with open(log, "ab") as lf:
        subprocess.Popen([py, "-m", "gsync.memos", "process"], cwd=str(DESK), stdin=subprocess.DEVNULL, stdout=lf,
                         stderr=subprocess.STDOUT, creationflags=flags, env=env, close_fds=True)


# ---------------------------------------------------------------------------------------------------------- Drive side
class MemoPull:
    """One pass of the Drive side, run by Sync.run() after the Docs work. `spawn` is injectable for tests."""

    def __init__(self, sync, spawn=None, running=None):
        self.s, self.cfg, self.drive, self.dry = sync, sync.cfg, sync.drive, sync.dry
        self.store = MemoStore(self.cfg.memos_state)
        self.spawn = spawn or spawn_processor
        self.running = running or processor_running

    def run(self) -> None:
        parent = self.s.m.get("folder_id")
        if not parent:  # narrow scopes: no Punchable folder to look in
            return
        folder = self.folder(parent)
        if not folder:
            return
        self.list_new(folder)
        if not self.dry:
            self.download_batch()
            m = self.store.load()
            if any(it.get("status") in ("downloaded", "transcribing") for it in m["items"].values()) \
                    and not self.running(self.cfg):
                self.spawn(self.cfg)
                self.s.act("memo_processor_started")

    def folder(self, parent: str) -> str | None:
        m = self.store.load()
        fid = m.get("folder_id")
        if fid:
            meta = self.drive.get(fid)
            if meta and not meta.get("trashed"):
                return fid
        found = self.drive.find_by_prop("punchable_kind", "voice_memos")
        if not found:  # one he made himself by hand
            found = next((f for f in self.drive.list_folder(parent) if f.get("mimeType") == FOLDER_MIME
                          and (f.get("name") or "").strip().lower() == MEMO_FOLDER_NAME.lower()), None)
        if not found:
            if self.dry:
                self.s.act("memo_folder_create")
                return None
            found = self.drive.create_folder(MEMO_FOLDER_NAME, parent, {"punchable_kind": "voice_memos"})
            self.s.act("memo_folder_create", doc=found["id"])
        self.store.update(lambda m: m.__setitem__("folder_id", found["id"]))
        return found["id"]

    def _walk(self, folder: str, depth: int = 0) -> list[dict]:
        out = []
        for f in self.drive.list_folder(folder):
            if f.get("mimeType") == FOLDER_MIME:
                if depth < 2:  # "Voice memos/2019" etc.
                    out += self._walk(f["id"], depth + 1)
            elif not f.get("trashed") and is_audio(f):
                out.append(f)
        return out

    def list_new(self, folder: str) -> None:
        files = self._walk(folder)
        m = self.store.load()
        known = m["items"]
        md5s = {it.get("md5"): k for k, it in known.items() if it.get("md5") and it.get("status") != "skipped_legal"}
        new = []
        for f in sorted(files, key=lambda x: (x.get("createdTime") or "", x.get("name") or "")):
            if f["id"] in known:
                continue
            why = rules.legal_reason(f, self.s.ancestors(f))
            if why:  # metadata only: never downloaded; only the id and the reason are kept
                new.append((f["id"], {"id": f["id"], "source": "drive", "drive_id": f["id"], "status": "skipped_legal",
                                      "why": why.split(":")[0], "tries": 0}))
                self.s.act("memo_skip_legal", doc=f["id"], why=why.split(":")[0])
                continue
            it = {"id": f["id"], "source": "drive", "drive_id": f["id"], "name": f.get("name"), "md5": f.get("md5Checksum"),
                  "size": int(f.get("size") or 0), "drive_created": f.get("createdTime"), "tries": 0}
            if it["md5"] and it["md5"] in md5s:
                it.update(status="duplicate", dup_of=md5s[it["md5"]])
                self.s.act("memo_duplicate", doc=f["id"])
            else:
                it["status"] = "listed"
                if it["md5"]:
                    md5s[it["md5"]] = f["id"]
                self.s.act("memo_new", doc=f["id"])
            new.append((f["id"], it))
        if new and not self.dry:
            def add(m):
                for k, it in new:
                    m["items"].setdefault(k, {**it, "updated_at": now_iso()})
            self.store.update(add)

    def download_batch(self) -> None:
        m = self.store.load()
        items = m["items"]
        staged = sum(int(it.get("size") or 0) for it in items.values() if it.get("status") in ("downloaded", "transcribing"))
        todo = sorted((it for it in items.values() if it.get("status") == "listed"),
                      key=lambda it: (it.get("drive_created") or "", it.get("name") or ""))
        inbox = self.cfg.notes_inbox
        inbox.mkdir(parents=True, exist_ok=True)
        stage_cap = self.cfg.memo_stage_mb * 1_000_000
        paused = None
        n = 0
        for it in todo:
            if n >= self.cfg.memo_batch:
                break
            size = int(it.get("size") or 0)
            if staged and staged + size > stage_cap:
                break  # the processor catches up first; the next pass downloads more
            free = free_mb(inbox)
            need = self.cfg.memo_reserve_mb + 4 * size // 1_000_000 + 1  # the file + its wav while transcribing
            if free < need:
                paused = {"reason": "low_disk", "free_mb": free, "need_mb": need, "at": now_iso()}
                self.s.act("memo_paused_low_disk", n=free)
                break
            stem = re.sub(r"[^A-Za-z0-9 ._-]+", "_", Path(it.get("name") or "memo").stem).strip()[:60] or "memo"
            fname = f"{it['id'][:12]}_{stem}{_ext(it.get('name')) or '.m4a'}"
            dest = inbox / fname
            part = dest.with_name(dest.name + ".part")
            # claim the file name first, so the processor never mistakes it for a file he dropped by hand
            self.store.set_item(it["id"], file=fname)
            try:
                got = self.drive.download(it["drive_id"], part)
                os.replace(part, dest)
            except Exception as e:  # noqa: BLE001  retried next pass, up to MAX_TRIES
                part.unlink(missing_ok=True)
                tries = int(it.get("tries") or 0) + 1
                self.store.set_item(it["id"], tries=tries, file=None, error=f"download: {type(e).__name__}: {e}"[:300],
                                    status="failed" if tries >= MAX_TRIES else "listed")
                self.s.rec["errors"].append({"memo": it["id"], "error": f"download: {type(e).__name__}: {e}"[:300]})
                continue
            self.store.set_item(it["id"], status="downloaded", file=fname, size=int(got or size), error=None)
            self.s.act("memo_download", doc=it["id"])
            staged += int(got or size)
            n += 1
        self.store.update(lambda m: m.__setitem__("paused", paused))


# ---------------------------------------------------------------------------------------------------------- the file
def _parse_dt(s: str | None) -> datetime | None:
    s = (s or "").strip()
    if not re.match(r"^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}", s):
        return None
    try:
        return datetime.fromisoformat(s.replace("Z", "+00:00"))
    except ValueError:
        m = re.match(r"^(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2}(?::\d{2})?)", s)
        return datetime.fromisoformat(f"{m.group(1)}T{m.group(2)}") if m else None


FILENAME_DT = re.compile(r"(20\d{2}|19\d{2})[-_.]?(\d{2})[-_.]?(\d{2})(?:[ _T-]?(\d{2})[-_.:]?(\d{2})[-_.:]?(\d{2}))?")


def recorded(tags: dict, name: str, drive_created: str | None, mtime: float | None) -> tuple[str | None, str | None, str]:
    """(recorded_at ISO, local date, source). Voice Memos keeps the recording time in the ©day atom (ffprobe's `date`
    tag); the container creation_time can be the export time, so it comes after it."""
    tags = {k.lower(): v for k, v in (tags or {}).items()}
    for key, src in (("date", "the memo's recording date (©day)"),
                     ("com.apple.quicktime.creationdate", "com.apple.quicktime.creationdate"),
                     ("creation_time", "the file's creation_time (can be when it was exported)")):
        d = _parse_dt(tags.get(key))
        if d:
            if d.tzinfo is None:
                d = d.astimezone()  # no zone in the file: this PC's
            local = d if key == "com.apple.quicktime.creationdate" else d.astimezone()
            return d.isoformat(), local.date().isoformat(), src
    m = FILENAME_DT.search(Path(name or "").stem)
    if m:
        try:
            d = datetime(int(m.group(1)), int(m.group(2)), int(m.group(3)), int(m.group(4) or 0), int(m.group(5) or 0),
                         int(m.group(6) or 0))
            return d.astimezone().isoformat(), d.date().isoformat(), "the file name"
        except ValueError:
            pass
    d = _parse_dt(drive_created)
    if d:
        return d.isoformat(), d.astimezone().date().isoformat(), "when it was saved to Google Drive"
    if mtime:
        d = datetime.fromtimestamp(mtime).astimezone()
        return d.isoformat(), d.date().isoformat(), "the file's modified time"
    return None, None, "unknown"


def probe(path: Path) -> dict:
    r = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration:format_tags", "-of", "json", str(path)],
                       capture_output=True, text=True, encoding="utf-8")
    try:
        fmt = json.loads(r.stdout or "{}").get("format", {})
    except ValueError:
        fmt = {}
    try:
        dur = float(fmt.get("duration"))
    except (TypeError, ValueError):
        dur = 0.0
    return {"duration_s": round(dur, 2), "tags": {k.lower(): v for k, v in (fmt.get("tags") or {}).items()}}


GENERIC = re.compile(r"^(new recording|recording|voice memo|audio|memo|untitled)\b|^\d|,", re.I)


def idea_title(title: str | None) -> str | None:
    """His own memo name ("airport bit tag") is the idea's title. Default names ("New Recording 12", a place like
    "Brooklyn, NY 3", "123 Main St", "20190314 210311") are not: the API then titles it from the first words."""
    t = (title or "").strip()
    if not t or GENERIC.search(t) or FILENAME_DT.fullmatch(t.replace(" ", "")):
        return None
    t = re.sub(r"\s+\d+$", "", t) if re.search(r"[A-Za-z]{3}", t) else t
    return t[:120] or None


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def transcript_text(tx: dict) -> str:
    return re.sub(r"\s+", " ", " ".join((s.get("text") or "").strip() for s in tx.get("segments") or [])).strip()


# ---------------------------------------------------------------------------------------------------------- processor
class MemoProcessor:
    def __init__(self, cfg: Config | None = None, pk: Punchable | None = None, transcriber=None, normalizer=None,
                 prober=None, log=print):
        self.cfg = cfg or Config()
        self.pk = pk or Punchable(self.cfg)
        self._tr, self._norm = transcriber, normalizer
        self.prober = prober or probe
        self.store = MemoStore(self.cfg.memos_state)
        self.log = log

    # whisper loads lazily, once per run, and is freed when the processor exits
    def transcriber(self):
        if self._tr is None:
            from engine.audio import Transcriber
            from engine.config import Config as EngineConfig
            self._tr = Transcriber(EngineConfig(), batch_size=4)  # batched on the GPU: memos are hours of audio
        return self._tr

    def normalizer(self):
        if self._norm is None:
            from engine.audio import normalize
            self._norm = normalize
        return self._norm

    def run(self, max_items: int | None = None) -> dict:
        lk = ProcLock(proc_lock_path(self.cfg))
        if not lk.acquire():
            self.log("memos: another processor is running")
            return {"status": "busy"}
        try:
            return self._run(max_items)
        finally:
            lk.release()

    def _run(self, max_items):
        def reset(m):
            for it in m["items"].values():
                if it.get("status") == "transcribing":  # a run that died mid-memo
                    it["status"] = "downloaded"
        self.store.update(reset)
        done = 0
        stats = {"done": 0, "failed": 0, "duplicate": 0, "skipped_legal": 0}
        while max_items is None or done < max_items:
            self.register_local()
            m = self.store.load()
            nxt = sorted((it for it in m["items"].values() if it.get("status") == "downloaded"),
                         key=lambda it: (it.get("drive_created") or "", it.get("name") or ""))
            if not nxt:
                break
            if not self.pk.health():
                self.log("memos: the Punchable API is not running; stopping (the next gsync pass starts me again)")
                stats["api_down"] = True
                break
            it = nxt[0]
            self.store.update(lambda m: m.__setitem__("processor", {"pid": os.getpid(), "at": now_iso(), "ts": time.time()}))
            r = self.process_one(it)
            if r == "low_disk":
                stats["paused"] = "low_disk"
                break
            stats[r] = stats.get(r, 0) + 1
            done += 1
        self.store.update(lambda m: m.__setitem__("processor", {}))
        self.log(f"memos: {json.dumps(stats)}; {progress(self.store.load())['line']}")
        return stats

    def register_local(self) -> None:
        """Audio files he dropped into comedy/notes/inbox himself (not gsync downloads) join the queue."""
        inbox = self.cfg.notes_inbox
        if not inbox.is_dir():
            return
        m = self.store.load()
        staged = {it.get("file") for it in m["items"].values() if it.get("file")}
        new = []
        for p in sorted(inbox.iterdir()):
            if not p.is_file() or p.name.startswith(".") or p.suffix.lower() not in AUDIO_EXT or p.name in staged:
                continue
            sha = sha256_file(p)
            key = "local:" + sha[:12] + hashlib.sha256(p.name.encode()).hexdigest()[:4]
            if key in m["items"] or key in {k for k, _ in new}:  # the same file dropped again: set aside, never deleted
                self._done_original({"source": "local"}, p, None)
                continue
            new.append((key, {"id": key, "source": "local", "name": p.name, "file": p.name, "size": p.stat().st_size,
                              "sha256": sha, "status": "downloaded", "tries": 0, "updated_at": now_iso()}))
        if new:
            self.store.update(lambda m: [m["items"].setdefault(k, v) for k, v in new])

    def _done_original(self, it: dict, path: Path, dest_dir: Path | None) -> None:
        """A Drive download is our copy (Drive keeps his original): deleted. A file he dropped by hand is his: moved
        next to the Opus copy, never deleted."""
        if not path.exists():
            return
        if it.get("source") == "drive":
            path.unlink()
            return
        dest_dir = dest_dir or (self.cfg.memo_audio / "_memo_duplicates")
        dest_dir.mkdir(parents=True, exist_ok=True)
        target, n = dest_dir / path.name, 1
        while target.exists():
            target = dest_dir / f"{path.stem}.dup{n}{path.suffix}"
            n += 1
        shutil.move(str(path), str(target))

    def _filed_already(self, key: str) -> str | None:
        """Crash guard: this memo's idea reached decisions.jsonl but not memos.json."""
        ds = self.pk.decisions()
        undone = {d["undo_of"] for d in ds if isinstance(d.get("undo_of"), int)}
        for i, d in enumerate(ds):
            if d.get("action") == "add" and i not in undone and f"(memo {key}," in (d.get("reason") or ""):
                return d.get("id")
        return None

    def file_idea(self, key: str, text: str, title: str | None, label: str, date: str | None) -> dict:
        jid = self._filed_already(key)
        if jid:
            return {"joke_id": jid, "attach": None}
        reason = f"{GDOCS} voice memo '{label}' (memo {key}, recorded {date or 'unknown'})"
        r = self.pk._req("POST", "/api/ideas", {"text": text, "title": title or "",
                                                "from": {"note": f"Voice memo '{label}', recorded {date or 'unknown'}"},
                                                "reason": reason})
        att = r.get("attach") or None
        if att:
            att = {k: att.get(k) for k in ("decision", "id", "into", "title") if att.get(k) is not None}
        return {"joke_id": r["id"], "attach": att}

    def process_one(self, it: dict) -> str:
        key = it["id"]
        path = self.cfg.notes_inbox / (it.get("file") or "")
        if not it.get("file") or not path.is_file():
            if it.get("source") == "drive":
                self.store.set_item(key, status="listed", file=None)  # gsync downloads it again
            else:
                self.store.set_item(key, status="failed", error="the file is no longer in notes/inbox")
            return "failed"
        self.store.set_item(key, status="transcribing")
        try:
            sha = it.get("sha256") or sha256_file(path)
            m = self.store.load()
            twin = next((k for k, x in m["items"].items() if k != key and x.get("sha256") == sha
                         and x.get("status") in ("done", "skipped_legal")), None)
            if twin:
                self._done_original(it, path, None)
                self.store.set_item(key, status="duplicate", dup_of=twin, sha256=sha, file=None)
                return "duplicate"
            pr = self.prober(path)
            dur = float(pr.get("duration_s") or 0)
            need = self.cfg.memo_reserve_mb // 2 + int(dur * 32_000 * 1.2 / 1e6) + 1
            free = free_mb(self.cfg.memo_audio)
            if free < need:
                self.store.update(lambda m: m.__setitem__("paused", {"reason": "low_disk", "free_mb": free,
                                                                     "need_mb": need, "at": now_iso()}))
                self.store.set_item(key, status="downloaded")
                return "low_disk"
            tags = pr.get("tags") or {}
            at, date, src = recorded(tags, it.get("name") or path.name, it.get("drive_created"), path.stat().st_mtime)
            label = (tags.get("title") or Path(it.get("name") or path.name).stem).strip()
            name = f"{date or 'undated'}_{sha[:8]}"
            adir, tdir = self.cfg.memo_audio / name, self.cfg.memo_corpus / name
            wav = adir / "audio16k.wav"
            self.normalizer()(path, wav)
            tx = self.transcriber()(wav, tdir)
            wav.unlink(missing_ok=True)  # derived and large: the Opus copy and Drive's original stay
            text = transcript_text(tx)
            if rules.content_is_legal(text):  # the second legal net, on the words: nothing of it is kept
                shutil.rmtree(tdir, ignore_errors=True)
                shutil.rmtree(adir, ignore_errors=True)
                if it.get("source") == "drive":
                    path.unlink(missing_ok=True)
                self.store.update(lambda m: m["items"].__setitem__(key, {
                    "id": key, "source": it.get("source"), "drive_id": it.get("drive_id"), "status": "skipped_legal",
                    "why": "content", "sha256": sha, "tries": 0, "updated_at": now_iso()}))
                return "skipped_legal"
            meta = {"kind": "voice_note", "source": "voice_memos", "title": label, "recorded_at": at, "date": date,
                    "date_source": src, "duration_s": dur, "sha256": sha, "drive_id": it.get("drive_id"),
                    "original_name": it.get("name"), "tags": {k: v for k, v in tags.items()
                                                              if k in ("date", "creation_time", "title", "encoder",
                                                                       "com.apple.quicktime.creationdate")}}
            tdir.mkdir(parents=True, exist_ok=True)
            (tdir / "meta.json").write_text(json.dumps(meta, ensure_ascii=False, indent=1), encoding="utf-8")
            filed = {"joke_id": None, "attach": None}
            flags = {}
            if not text:
                flags["no_speech"] = True
            elif len(text) > IDEA_MAX:
                flags["long"] = True  # a set-length recording: kept as a note with its audio, not one giant idea
            else:
                filed = self.file_idea(key, text, idea_title(label), label, date)
            self._done_original(it, path, adir)
            rel = lambda p: str(p.relative_to(self.cfg.comedy)).replace("\\", "/")  # noqa: E731
            self.store.set_item(key, status="done", sha256=sha, title=label, recorded_at=at, date=date, date_source=src,
                                duration_s=dur, name_dir=name,
                                audio=rel(adir / "audio.opus") if (adir / "audio.opus").exists() else None,
                                transcript=rel(tdir / "transcript.json"), chars=len(text), file=None, error=None,
                                joke_id=filed["joke_id"], attach=filed["attach"], **flags)
            self.log(f"memos: {label!r} ({dur:.0f}s, {date}) -> "
                     f"{(filed['attach'] or {}).get('title') or filed['joke_id'] or ('no speech' if not text else 'long note')}")
            return "done"
        except ApiDown:
            self.store.set_item(key, status="downloaded")
            raise
        except Exception as e:  # noqa: BLE001  one bad memo never stops the batch
            tries = int(it.get("tries") or 0) + 1
            self.store.set_item(key, status="failed" if tries >= MAX_TRIES else "downloaded", tries=tries,
                                error=f"{type(e).__name__}: {e}"[:400])
            self.log(f"memos: {key} failed ({tries}/{MAX_TRIES}): {type(e).__name__}: {e}")
            return "failed"


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="python -m gsync.memos")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("process", help="transcribe and file every downloaded voice memo (one at a time)")
    p.add_argument("--max", type=int, default=None)
    sub.add_parser("status", help="the progress line")
    a = ap.parse_args(argv)
    cfg = Config()
    if a.cmd == "status":
        print(json.dumps(progress(MemoStore(cfg.memos_state).load()), indent=1))
        return 0
    print(f"{datetime.now(timezone.utc).isoformat(timespec='seconds')} memos processor pid {os.getpid()}", flush=True)
    try:
        MemoProcessor(cfg).run(a.max)
    except ApiDown as e:
        print(f"memos: API down: {e}", flush=True)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
