"""Which text files live in the database (`srcfile` rows) and may be written there, by whom.

One list for every writer, so the bundle (db/push_remote.py pack), the API's worker route (POST /api/worker/files)
and the home worker's upload step can never disagree about what is data:

  BUNDLE   files db/push_remote.py packs from the PC (the one-way load used until the online-first cutover)
  WORKER   files the home worker may post at runtime (pipeline output, voice-memo transcripts, gsync's own state)

Paths are relative to the data folder (COMEDY_DIR), forward slashes. Never audio or video: only .json/.jsonl/.md, and
the worker route also refuses anything outside these patterns, absolute paths and `..`.
decisions.jsonl and the creative logs are NOT files here: they are rows (`decision`, `creative_event`) written
through the API's own write paths, one event at a time.
"""
from __future__ import annotations

import re

SET_FILES = ("analysis.json", "meta.json", "laughs_auto.json", "transcript.json", "clips.json")
KB0_FILES = ("decisions.jsonl", "inferred.json", "insights.json", "learning.json", "KB_REPORT.md")
CREATIVE_LOGS = ("storyboards", "workingsets", "venues")
CREATIVE_FILES = ("geocode_cache.json", "venue_addresses.json", "merge_judge.json")
GSYNC_FILES = ("memos.json", "map.json", "units.json", "edits.jsonl", "conflicts.jsonl", "runs.jsonl")
MEMO_FILES = ("transcript.json", "meta.json")
TEXT_SUFFIXES = {".json", ".jsonl", ".md"}
NOT_MIRRORED = {"kb/v0/decisions.jsonl"} | {f"kb/creative/{n}.jsonl" for n in CREATIVE_LOGS}

_NAME = r"[A-Za-z0-9][A-Za-z0-9_.\-]{0,120}"
WORKER_PATTERNS = [re.compile(p) for p in (
    rf"^corpus/sets/{_NAME}/(?:{'|'.join(map(re.escape, SET_FILES))})$",
    rf"^corpus/memos/{_NAME}/(?:{'|'.join(map(re.escape, MEMO_FILES))})$",
    rf"^kb/gsync/(?:{'|'.join(map(re.escape, GSYNC_FILES))})$",
    r"^kb/v0/(?:inferred\.json|insights\.json|learning\.json|KB_REPORT\.md)$",
    r"^kb/creative/(?:merge_judge\.json|geocode_cache\.json|venue_addresses\.json|suggest_judge\.json)$",
    r"^vectors/extra_[a-z_]{1,40}\.json$",
)]
MAX_FILE_BYTES = 8 * 1024 * 1024


def clean(rel: str) -> str | None:
    """The canonical form of a relative path, or None if it is not a plain relative path."""
    if not isinstance(rel, str) or not rel or len(rel) > 300:
        return None
    rel = rel.replace("\\", "/")
    if rel.startswith("/") or re.match(r"^[A-Za-z]:", rel) or any(p in ("", ".", "..") for p in rel.split("/")):
        return None
    return rel


def worker_may_write(rel: str) -> bool:
    rel = clean(rel)
    return bool(rel) and any(p.match(rel) for p in WORKER_PATTERNS)


def is_set_input(rel: str) -> bool:
    """Files db/load.py's sync reads (a change means the graph tables must be re-synced)."""
    return rel.startswith("corpus/sets/") or rel.startswith("kb/v0/") or rel.startswith("kb/v1/")
