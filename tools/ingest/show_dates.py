"""Show date and venue from the video title, and meta.json for sets that have none.

YouTube's upload_date is when the video went up, not when the show happened. The show date is usually in the title
("Sam Sample @ The Corner Room 10/31/16") or the description ("Mabel's 7/13/24"). This writes, into every
corpus/sets/<set>/meta.json:
  show_date, show_date_source, show_date_inferred: true
  show_venue, show_venue_source, show_venue_inferred: true
Everything else in an existing meta.json is left as it is. For a set with no meta.json it builds one (and copies the
YouTube caption file and its [Laughter]/[Applause] markers next to it).

A title date without a year takes the upload year (the year before if that lands after the upload). Venue names are
normalised to one string per room, so "Corner Room" and "The Corner Room" count as the same room for Working.

Your rooms and rules are private, in $PUNCHABLE_HOME/venues.json (default ~/.punchable/venues.json), never in the repo:
  {"venues": [["<regex on title/description>", "<canonical room name>"], ...],      first match wins
   "new_meta": {"<field>": <value>, ...},           extra fields for a NEW meta.json (after venue_guess)
   "title_flags": [{"pattern": "<regex>", "key": "<meta field>", "value": <any>}]}  set when the title matches
Without the file every set gets show_venue null and the date rules still apply.

Usage: python tools/ingest/show_dates.py [--dry-run]      (COMEDY_DIR = your data folder)
"""
from __future__ import annotations

import datetime as dt
import json
import os
import re
import shutil
import sys
from collections import Counter
from pathlib import Path

COMEDY = Path(os.path.expanduser(os.environ.get("COMEDY_DIR", "~/punchable-data")))
RAW = COMEDY / "tapes" / "raw"
SETS = COMEDY / "corpus" / "sets"


def _local() -> dict:
    p = Path(os.environ.get("PUNCHABLE_VENUES") or
             Path(os.environ.get("PUNCHABLE_HOME", Path.home() / ".punchable")) / "venues.json")
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


_LOCAL = _local()
# (pattern on title/description, canonical venue). First match wins.
VENUES = [(p, n) for p, n in _LOCAL.get("venues", [])]
NEW_META = dict(_LOCAL.get("new_meta", {}))
TITLE_FLAGS = [(re.compile(f["pattern"], re.I), f["key"], f["value"]) for f in _LOCAL.get("title_flags", [])]
DATE = re.compile(r"(?<![\d/])(\d{1,2})[/-](\d{1,2})(?:[/-](\d{2,4}))?(?![\d/])")

# ---- YouTube captions (WebVTT): the newly added line of each rolling cue, and its bracketed markers --------------
TS = r"(\d+):(\d\d):(\d\d)\.(\d+)"
MARK = re.compile(r"\[(Laughter|Applause|Music|Laughs|Cheering|Music playing)\]", re.I)


def secs(h, m, s, ms):
    return int(h) * 3600 + int(m) * 60 + int(s) + int(ms) / 1000


def parse_vtt(path):
    """Return list of (start, end, new_line) using only the newly added last line of each rolling cue."""
    cues, lines = [], path.read_text(encoding="utf-8", errors="replace").splitlines()
    i = 0
    while i < len(lines):
        m = re.match(TS + r" --> " + TS, lines[i])
        if not m:
            i += 1
            continue
        g = m.groups()
        start, end = secs(*g[:4]), secs(*g[4:])
        body = []
        i += 1
        while i < len(lines) and lines[i].strip() != "" or (i < len(lines) and lines[i] == " "):
            body.append(lines[i])
            i += 1
        body = [b for b in body if b.strip()]
        if end - start < 0.05 or not body:  # 10 ms "freeze" cues repeat the previous text
            continue
        last = re.sub(r"<[^>]+>", "", body[-1]).strip()
        cues.append((start, end, last))
    return cues


def caption_events(cues):
    seen, ev = set(), []
    for start, end, text in cues:
        for m in MARK.finditer(text):
            key = (m.group(1).lower(), round(start, 2))
            if key not in seen:
                seen.add(key)
                ev.append({"marker": f"[{m.group(1).title()}]", "start": round(start, 2), "end": round(end, 2)})
    return ev


def parse_date(text: str, upload: str):
    up = dt.date(int(upload[:4]), int(upload[4:6]), int(upload[6:]))
    for m in DATE.finditer(text or ""):
        mo, d, y = int(m[1]), int(m[2]), m[3]
        if not (1 <= mo <= 12 and 1 <= d <= 31):
            continue
        if y:
            y = int(y) + (2000 if len(y) == 2 else 0)
            try:
                return dt.date(y, mo, d), m[0], False
            except ValueError:
                continue
        try:
            cand = dt.date(up.year, mo, d)
        except ValueError:
            continue
        if cand > up:
            cand = dt.date(up.year - 1, mo, d)
        return cand, m[0], True
    return None, None, None


def venue_of(text: str):
    for pat, name in VENUES:
        if re.search(pat, text or "", re.I):
            return name
    return None


def new_meta(d: Path, info: dict, tr: dict) -> dict:
    out = SETS / d.name
    meta = {"id": info["id"], "title": info["title"], "upload_date": info["upload_date"],
            "duration": info["duration"], "uploader": info["uploader"], "url": info["url"],
            "webpage_url": info.get("webpage_url"), "description": info.get("description"),
            "venue_guess": venue_of(info["title"]) or venue_of(info.get("description")),
            **NEW_META,
            "captions": info.get("captions_kind"), "captions_track": info.get("captions_track"),
            "asr": {k: tr[k] for k in ("engine", "model", "device", "compute_type", "audio_sec", "compute_sec", "rtf")}}
    vtt = d / "captions.en.vtt"
    if vtt.exists():
        shutil.copyfile(vtt, out / "captions.vtt")
        ev = caption_events(parse_vtt(vtt))
        (out / "caption_events.json").write_text(json.dumps(
            {"source": f"YouTube {info.get('captions_kind')} captions ({info.get('captions_track')})",
             "note": "Auto-caption markers are sparse hints, not ground truth.", "events": ev}, indent=1), encoding="utf-8")
        meta["caption_marker_counts"] = dict(Counter(e["marker"] for e in ev))
    return meta


def main():
    dry = "--dry-run" in sys.argv
    rows = []
    for d in sorted(p for p in RAW.iterdir() if (p / "info.json").exists()):
        info = json.loads((d / "info.json").read_text(encoding="utf-8"))
        mp = SETS / d.name / "meta.json"
        tp = SETS / d.name / "transcript.json"
        if not tp.exists():
            continue
        created = not mp.exists()
        meta = new_meta(d, info, json.loads(tp.read_text(encoding="utf-8"))) if created else json.loads(mp.read_text(encoding="utf-8"))
        date, src, no_year = None, None, None
        for field in ("title", "description"):
            date, src, no_year = parse_date(info.get(field) or "", info["upload_date"])
            if date:
                src_field = field
                break
        up = f"{info['upload_date'][:4]}-{info['upload_date'][4:6]}-{info['upload_date'][6:]}"
        if date:
            meta["show_date"] = date.isoformat()
            meta["show_date_source"] = f"{src_field} '{info.get(src_field)}' ({src!r}" + (", year from upload date)" if no_year else ")")
        else:
            meta["show_date"] = up
            meta["show_date_source"] = "upload date (no date in title or description)"
        meta["show_date_inferred"] = True
        v = venue_of(info["title"]) or venue_of(info.get("description"))
        meta["show_venue"] = v
        meta["show_venue_source"] = "title" if venue_of(info["title"]) else ("description" if v else "none: title names no room")
        meta["show_venue_inferred"] = True
        for rx, key, value in TITLE_FLAGS:
            if rx.search(info["title"]):
                meta[key] = value
        rows.append((d.name, up, meta["show_date"], meta["show_venue"], created))
        if not dry:
            mp.write_text(json.dumps(meta, indent=1, ensure_ascii=False), encoding="utf-8")
    for r in rows:
        flag = "  (show date differs from upload)" if r[1] != r[2] else ""
        print(f"{r[0]}: upload {r[1]} -> show {r[2]} @ {r[3]}{' [new meta.json]' if r[4] else ''}{flag}")


if __name__ == "__main__":
    main()
