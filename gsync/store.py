"""comedy/kb/gsync/: map.json (atomic), runs/conflicts/edits .jsonl (append-only), units.json, mirror/."""
from __future__ import annotations

import json
import os
import tempfile
from datetime import datetime, timezone
from pathlib import Path


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def write_atomic(p: Path, text: str) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=p.parent, prefix=p.name + ".", suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, p)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)


def append_jsonl(p: Path, rec: dict) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    with open(p, "a", encoding="utf-8", newline="\n") as f:
        f.write(json.dumps(rec, ensure_ascii=False, separators=(",", ":")) + "\n")
        f.flush()
        os.fsync(f.fileno())


def read_jsonl(p: Path) -> list[dict]:
    if not p.exists():
        return []
    out = []
    for ln in p.read_text(encoding="utf-8").splitlines():
        try:
            out.append(json.loads(ln))
        except ValueError:
            pass  # a torn last line
    return out


class Store:
    """map.json shape:
    {version, changes_token, bootstrapped_at, folder_id, folders:{id:[parents]},
     docs:{gdoc_id:{origin: corpus|gdocs|app|board|ai_draft, joke_id?, jokes?, title, version, modified, doc_hash,
                    app_hash, file, hidden?, contains_drafts?, source?}},
     boards:{sb_id:{doc_id, hash, version}}, excluded:{gdoc_id: reason}, skipped:{gdoc_id: reason},
     pending:{gdoc_id: reason}}"""

    def __init__(self, root: Path):
        self.root = root
        self.map_path = root / "map.json"
        self.units_path = root / "units.json"
        self.mirror = root / "mirror"
        self.m = self._load()

    def _load(self) -> dict:
        if self.map_path.exists():
            return json.loads(self.map_path.read_text(encoding="utf-8"))
        return {"version": 1, "changes_token": None, "docs": {}, "boards": {}, "excluded": {}, "skipped": {},
                "pending": {}, "folders": {}}

    def save(self) -> None:
        write_atomic(self.map_path, json.dumps(self.m, ensure_ascii=False, indent=1, sort_keys=True))

    def log(self, name: str, rec: dict) -> None:
        append_jsonl(self.root / name, rec)

    def put_units(self, doc_id: str, units: list[dict]) -> None:
        all_u = json.loads(self.units_path.read_text(encoding="utf-8")) if self.units_path.exists() else []
        all_u = [u for u in all_u if u.get("doc_id") != doc_id] + units
        write_atomic(self.units_path, json.dumps(all_u, ensure_ascii=False, indent=1))

    def put_mirror(self, name: str, text: str) -> str:
        write_atomic(self.mirror / name, text)
        return str((self.mirror / name).relative_to(self.root.parent.parent)).replace("\\", "/")
