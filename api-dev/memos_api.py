"""Voice memos from the iPhone (gsync/memos.py does the work on his PC; this only reads its state).

    GET /api/memos                 {available, progress: {line, total, transcribed, waiting, ...}, items: [Memo]}
    GET /api/memos/{id}            {memo: Memo + segments[{start, end, text}]}
    GET /api/memos/{id}/audio      the Opus copy, with byte ranges

State: <comedy>/kb/gsync/memos.json (written by gsync, read-only here); transcripts in corpus/memos/<date>_<sha8>/,
audio in tapes/processed/<date>_<sha8>/audio.opus. Court material is not listed (only counted), duplicates neither.
On the hosted API none of these files exist: `available: false` and an empty list.
"""
from __future__ import annotations

import json
import re
import time
from pathlib import Path
from urllib.parse import unquote

LIST_TEXT = 600
HIDDEN = ("skipped_legal", "duplicate")
PROC_ALIVE_S = 20 * 60


def comedy_dir(kb) -> Path:
    """The data folder: the parent of corpus/ (locally the same as tapes/'s parent; hosted, PK_DATA/comedy, where
    the srcfile rows are cached, while tapes/ is PK_DATA/tapes and empty)."""
    return Path(kb.corpus).parent


def load(comedy: Path) -> dict | None:
    p = comedy / "kb" / "gsync" / "memos.json"
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


def _inside(comedy: Path, rel: str | None) -> Path | None:
    if not rel:
        return None
    p = (comedy / rel).resolve()
    try:
        p.relative_to(comedy.resolve())
    except ValueError:
        return None
    return p if p.is_file() else None


def progress(m: dict) -> dict:
    items = list((m.get("items") or {}).values())
    c: dict[str, int] = {}
    for it in items:
        c[it.get("status")] = c.get(it.get("status"), 0) + 1
    total = sum(v for k, v in c.items() if k not in HIDDEN)
    done = c.get("done", 0)
    waiting = c.get("listed", 0) + c.get("downloaded", 0) + c.get("transcribing", 0)
    pr = m.get("processor") or {}
    working = bool(waiting) and time.time() - float(pr.get("ts") or 0) < PROC_ALIVE_S
    paused = m.get("paused")
    line = f"{done} of {total} voice memo{'s' if total != 1 else ''} transcribed" if total else "No voice memos yet"
    if paused and paused.get("reason") == "low_disk":
        line += f" · paused: the PC is low on disk space ({paused.get('free_mb')} MB free)"
    return {"line": line, "total": total, "transcribed": done, "waiting": waiting, "failed": c.get("failed", 0),
            "duplicates": c.get("duplicate", 0), "skipped_legal": c.get("skipped_legal", 0), "working": working,
            "paused": paused}


def text_of(comedy: Path, it: dict) -> tuple[str, list[dict]]:
    p = _inside(comedy, it.get("transcript"))
    if not p:
        return "", []
    try:
        segs = json.loads(p.read_text(encoding="utf-8")).get("segments") or []
    except (OSError, ValueError):
        return "", []
    text = re.sub(r"\s+", " ", " ".join((s.get("text") or "").strip() for s in segs)).strip()
    return text, [{"start": s.get("start"), "end": s.get("end"), "text": (s.get("text") or "").strip()} for s in segs]


def card(comedy: Path, it: dict, full: bool = False) -> dict:
    text, segs = text_of(comedy, it) if it.get("status") == "done" else ("", [])
    att = it.get("attach") or None
    out = {"id": it["id"], "title": it.get("title") or Path(it.get("name") or "Voice memo").stem,
           "status": it.get("status"), "date": it.get("date"), "recorded_at": it.get("recorded_at"),
           "date_source": it.get("date_source"), "duration_s": it.get("duration_s"),
           "text": text if full or len(text) <= LIST_TEXT else text[:LIST_TEXT].rsplit(" ", 1)[0] + "…",
           "text_cut": (not full) and len(text) > LIST_TEXT, "joke_id": it.get("joke_id"),
           "attached_to": {"joke_id": att.get("into"), "title": att.get("title")}
           if att and att.get("decision") == "attach" else None,
           "long": bool(it.get("long")), "no_speech": bool(it.get("no_speech")),
           "audio": f"/api/memos/{it['id']}/audio" if _inside(comedy, it.get("audio")) else None,
           # online-first keeps no audio: the reference is where the recording lives (his Drive, or his phone)
           "reference": reference(it),
           "error": it.get("error") if it.get("status") == "failed" else None}
    if full:
        out["segments"] = segs
    return out


def reference(it: dict) -> dict | None:
    if it.get("drive_id"):
        return {"kind": "drive", "url": f"https://drive.google.com/file/d/{it['drive_id']}/view",
                "name": it.get("name")}
    if it.get("name") or it.get("recorded_at"):
        return {"kind": "file", "name": it.get("name"), "recorded_at": it.get("recorded_at")}
    return None


def handle(h, kb, path: str, q: dict):
    comedy = comedy_dir(kb)
    m = load(comedy)
    if path == "/api/memos":
        if m is None:
            return h.send_json({"available": False, "progress": progress({}), "items": []})
        rows = [it for it in (m.get("items") or {}).values() if it.get("status") not in HIDDEN]
        order = {"transcribing": 0, "failed": 1, "downloaded": 2, "listed": 3, "done": 4}
        rows.sort(key=lambda it: (order.get(it.get("status"), 5), -(_ts(it.get("recorded_at")))))
        return h.send_json({"available": True, "progress": progress(m), "items": [card(comedy, it) for it in rows]})
    mm = re.match(r"^/api/memos/([^/]+)(/audio)?$", path)
    if not mm:
        return h.err(404, "no such memos route")
    key = unquote(mm.group(1))
    it = (m or {}).get("items", {}).get(key)
    if not it or it.get("status") in HIDDEN:
        return h.err(404, "no such voice memo")
    if not mm.group(2):
        return h.send_json({"memo": card(comedy, it, full=True)})
    p = _inside(comedy, it.get("audio"))
    if not p:
        return h.err(404, "no audio for that memo")
    return send_range(h, p, "audio/ogg")


def _ts(s: str | None) -> float:
    from datetime import datetime
    try:
        return datetime.fromisoformat((s or "").replace("Z", "+00:00")).timestamp()
    except ValueError:
        return 0.0


def send_range(h, p: Path, ctype: str):
    size = p.stat().st_size
    start, end, status = 0, size - 1, 200
    rng = h.headers.get("Range")
    if rng:
        mr = re.match(r"^bytes=(\d*)-(\d*)$", rng.strip())
        if mr and not (mr.group(1) == "" and mr.group(2) == ""):
            if mr.group(1) == "":
                start = max(0, size - int(mr.group(2)))
            else:
                start = int(mr.group(1))
                end = min(int(mr.group(2)), size - 1) if mr.group(2) else size - 1
        if not mr or (mr.group(1) == "" and mr.group(2) == "") or start >= size or start > end:
            h.send_response(416)
            h.cors()
            h.send_header("Content-Range", f"bytes */{size}")
            h.send_header("Content-Length", "0")
            h.end_headers()
            return None
        status = 206
    n = end - start + 1
    h.send_response(status)
    h.cors()
    h.send_header("Content-Type", ctype)
    h.send_header("Accept-Ranges", "bytes")
    h.send_header("Content-Length", str(n))
    h.send_header("Cache-Control", "private, max-age=3600")
    if status == 206:
        h.send_header("Content-Range", f"bytes {start}-{end}/{size}")
    h.end_headers()
    if h.command == "HEAD":
        return None
    with open(p, "rb") as f:
        f.seek(start)
        left = n
        while left > 0:
            chunk = f.read(min(256 * 1024, left))
            if not chunk:
                break
            h.wfile.write(chunk)
            left -= len(chunk)
    return None
