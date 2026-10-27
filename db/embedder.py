"""Python handle on db/embed.mjs (bge-small-en-v1.5 via transformers.js; local, no downloads)."""
from __future__ import annotations

import json
import subprocess
import threading
from pathlib import Path

HERE = Path(__file__).resolve().parent


class Embedder:
    def __init__(self, timeout: float = 120):
        self.proc = None
        self.lock = threading.Lock()
        self.n = 0
        self.timeout = timeout
        self.error: str | None = None

    def _start(self):
        self.proc = subprocess.Popen(["node", str(HERE / "embed.mjs")], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, text=True, encoding="utf-8", bufsize=1)
        first = self.proc.stdout.readline()
        if not first or not json.loads(first).get("ready"):
            self.error = "embedder did not start"
            raise RuntimeError(self.error)

    def embed(self, texts: list[str], query: bool = False) -> list[list[float]]:
        with self.lock:
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
