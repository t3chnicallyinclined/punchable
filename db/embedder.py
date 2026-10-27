"""bge-small-en-v1.5 query and passage vectors (local, no downloads), for the API and db/load.py.

Two engines, the same model and pooling (CLS, L2-normalised, 384-d), so their vectors match each other and the
cached ones:
  1. Python: engine/embed.py (onnxruntime + tokenizers) over the ONNX export in <PK_CACHE_DIR>/models/
     bge-small-en-v1.5 or PK_BGE_DIR (tools/kb/fetch_bge.py). Used whenever that folder and onnxruntime exist.
  2. node: db/embed.mjs (transformers.js), the original sidecar; it needs the old spikes/ install.
2026-10-27: the spikes/ install was gone after the repo split ("embedding install missing"), so new notes got no
vectors and same-idea matching ran on words alone. Engine 1 removes that dependency.
"""
from __future__ import annotations

import json
import subprocess
import sys
import threading
from pathlib import Path

HERE = Path(__file__).resolve().parent
QUERY_PREFIX = "Represent this sentence for searching relevant passages: "


def python_engine():
    """engine.embed.Embedder on the local ONNX model, or None if it is not installed here."""
    try:
        sys.path.insert(0, str(HERE.parent))
        from engine.config import bge_dir_override
        d = bge_dir_override()
        if d is None:
            return None
        import onnxruntime  # noqa: F401
        import tokenizers  # noqa: F401
        from engine.embed import Embedder as E
        return E(d)
    except Exception:  # noqa: BLE001
        return None


class Embedder:
    def __init__(self, timeout: float = 120, prefer: str = "python"):
        self.proc = None
        self.lock = threading.Lock()
        self.n = 0
        self.timeout = timeout
        self.error: str | None = None
        self.py = python_engine() if prefer == "python" else None
        self.engine = "python" if self.py is not None else "node"

    def _start(self):
        self.proc = subprocess.Popen(["node", str(HERE / "embed.mjs")], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, text=True, encoding="utf-8", bufsize=1)
        first = self.proc.stdout.readline()
        if not first or not json.loads(first).get("ready"):
            self.error = "embedder did not start (no bge-small model: run python tools/kb/fetch_bge.py)"
            raise RuntimeError(self.error)

    def embed(self, texts: list[str], query: bool = False) -> list[list[float]]:
        with self.lock:
            if self.py is not None:
                xs = [(QUERY_PREFIX if query else "") + (t or " ") for t in texts]
                return [[float(x) for x in row] for row in self.py.embed(xs)]
            if self.proc is None or self.proc.poll() is not None:
                self._start()
            self.n += 1
            self.proc.stdin.write(json.dumps({"id": self.n, "texts": texts, "query": query}) + "\n")
            self.proc.stdin.flush()
            line = self.proc.stdout.readline()
            msg = json.loads(line)
            if msg.get("error"):
                raise RuntimeError(msg["error"])
            return msg["vecs"]

    def close(self):
        if self.proc and self.proc.poll() is None:
            self.proc.stdin.close()
            try:
                self.proc.wait(5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
