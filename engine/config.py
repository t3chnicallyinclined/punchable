"""Engine configuration: engine/config.toml plus overrides.

Config(...) resolves {comedy} and {engine} in paths. `root=` redirects every
path that is written (kb, inbox, processed, out corpus) under one directory, which
is how tests and the end-to-end check run against temp copies.
"""
from __future__ import annotations

import copy
import os
import tomllib
from pathlib import Path

ENGINE_DIR = Path(__file__).resolve().parent
DEFAULT_TOML = ENGINE_DIR / "config.toml"


class Config:
    def __init__(self, path: str | Path | None = None, overrides: dict | None = None, root: str | Path | None = None):
        path = Path(path or os.environ.get("ENGINE_CONFIG") or DEFAULT_TOML)
        with open(path, "rb") as f:
            self.data = tomllib.load(f)
        if overrides:
            _deep_update(self.data, overrides)
        raw = dict(self.data.get("paths", {}))
        if root:
            root = Path(root)
            raw.update({"kb_dir": str(root / "kb"), "inbox_dir": str(root / "inbox"),
                        "processed_dir": str(root / "processed"), "out_corpus_dir": str(root / "corpus")})
            if overrides and "paths" in overrides:
                raw.update(overrides["paths"])
        comedy = os.path.expanduser(os.environ.get("COMEDY_DIR") or raw.get("comedy_dir", ""))
        self.paths: dict[str, Path] = {}
        for k, v in raw.items():
            v = str(v).replace("{comedy}", comedy).replace("{engine}", str(ENGINE_DIR))
            self.paths[k] = Path(os.path.normpath(os.path.expanduser(v)))

    # convenience -------------------------------------------------------------
    def p(self, key: str) -> Path:
        return self.paths[key]

    @property
    def inbox(self) -> dict:
        return self.data.get("inbox", {})

    @property
    def providers(self) -> dict:
        return self.data.get("providers", {})

    def task(self, name: str) -> dict:
        tasks = self.data.get("tasks", {})
        t = copy.deepcopy(tasks.get("default", {}))
        _deep_update(t, copy.deepcopy(tasks.get(name, {})))
        return t

    @property
    def runs_log(self) -> Path:
        return self.p("kb_dir") / "runs.jsonl"


def _deep_update(dst: dict, src: dict):
    for k, v in src.items():
        if isinstance(v, dict) and isinstance(dst.get(k), dict):
            _deep_update(dst[k], v)
        else:
            dst[k] = v
    return dst
