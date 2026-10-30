"""Append-only event log + snapshot, the way the app stores storyboards and working sets.

<dir>/<name>.jsonl  one event per line, never rewritten:
    {"ts", "op", "id", "by", "data"}
<dir>/<name>.json   snapshot = every event replayed: {"version": 1, "updated_at", "events", "items": {id: obj}}

The snapshot is derived: delete it and `Store.rebuild()` replays the log. Writes are atomic
(tmp + rename). `by` is OWNER_ID for his own actions and "ai:<what>" for generated items, so
training data stays clean (KB_SCHEMA "AI-first display rule").
"""
from __future__ import annotations

import os
import json
from pathlib import Path
from typing import Callable

from .util import now_iso, write_json

OWNER_ID = os.environ.get("PK_OWNER_ID", "owner")  # the actor id on your own edits (`by`); PK_OWNER_ID


class Store:
    def __init__(self, directory: Path, name: str, apply: Callable[[dict, dict], None]):
        self.dir = Path(directory)
        self.name = name
        self.log = self.dir / f"{name}.jsonl"
        self.snap = self.dir / f"{name}.json"
        self._apply = apply
        self.items: dict[str, dict] = {}
        self.n_events = 0
        self.rebuild()

    def events(self) -> list[dict]:
        out = []
        if self.log.exists():
            with open(self.log, encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if line:
                        try:
                            out.append(json.loads(line))
                        except json.JSONDecodeError:
                            pass  # a torn last line from a crash is never fatal
        return out

    def rebuild(self) -> None:
        self.items, self.n_events = {}, 0
        for ev in self.events():
            self._apply(self.items, ev)
            self.n_events += 1

    def append(self, op: str, id_: str, data: dict, by: str = OWNER_ID, ts: str | None = None) -> dict:
        ev = {"ts": ts or now_iso(), "op": op, "id": id_, "by": by, "data": data}
        probe = json.loads(json.dumps(self.items))  # validate on a copy: a bad op never reaches the log
        self._apply(probe, ev)
        self.dir.mkdir(parents=True, exist_ok=True)
        with open(self.log, "a", encoding="utf-8") as f:
            f.write(json.dumps(ev, ensure_ascii=False) + "\n")
        self.items = probe
        self.n_events += 1
        self.write_snapshot(ev["ts"])
        return self.items.get(id_)

    def write_snapshot(self, ts: str | None = None) -> None:
        write_json(self.snap, {"version": 1, "updated_at": ts or now_iso(), "events": self.n_events,
                               "items": self.items})

    def get(self, id_: str) -> dict | None:
        return self.items.get(id_)
