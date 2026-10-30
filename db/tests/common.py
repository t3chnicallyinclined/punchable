"""Shared test plumbing: TEMP copies of the KB files, a TEMP SurrealDB namespace/database, API processes.

Never touches the real decisions.jsonl or the real database (punchable/main): every test copies the files it
needs into a temp folder and loads them into `punchable_test/<unique db>`, which is removed afterwards.
"""
from __future__ import annotations

import json
import math
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
DB_DIR = HERE.parent
DESK = DB_DIR.parent
sys.path.insert(0, str(DB_DIR))
import sdb  # noqa: E402

COMEDY = sdb.COMEDY
TEST_NS = "punchable_test"


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def graph_choice() -> dict:
    """The graph the real database was loaded from (so tests diff the same version the app shows)."""
    p = COMEDY / "db" / "punchable" / "graph_choice.json"
    if p.is_file():
        return json.loads(p.read_text(encoding="utf-8"))
    return {"graph": str(COMEDY / "kb" / "v0" / "graph.json"), "id_map": None}


class TempKB:
    """Temp copies: graph.json (+ id_map), decisions.jsonl, inferred.json, KB_REPORT.md, and the corpus set folders."""

    def __init__(self, tag: str, decisions: bool = True):
        self.root = Path(tempfile.mkdtemp(prefix=f"punch_{tag}_"))
        c = graph_choice()
        self.kb = self.root / "kb"
        self.kb.mkdir()
        shutil.copy(c["graph"], self.root / "graph.json")
        self.graph = self.root / "graph.json"
        self.id_map = None
        if c.get("id_map"):
            shutil.copy(c["id_map"], self.root / "id_map.json")
            self.id_map = self.root / "id_map.json"
        real = COMEDY / "kb" / "v0"
        for f in ("inferred.json", "KB_REPORT.md", "learning.json"):
            if (real / f).is_file():
                shutil.copy(real / f, self.kb / f)
        if decisions and (real / "decisions.jsonl").is_file():
            shutil.copy(real / "decisions.jsonl", self.kb / "decisions.jsonl")
        else:
            (self.kb / "decisions.jsonl").write_text("", encoding="utf-8")
        self.corpus = self.root / "corpus"
        for d in (COMEDY / "corpus" / "sets").iterdir():
            if d.is_dir():
                (self.corpus / "sets" / d.name).mkdir(parents=True)
                for f in ("transcript.json", "laughs_auto.json", "meta.json", "analysis.json"):
                    if (d / f).is_file():
                        shutil.copy(d / f, self.corpus / "sets" / d.name / f)
        self.inbox = self.root / "inbox"
        self.inbox.mkdir()
        # creative/ stores (boards, working sets, pins, jobs): temp copies, so no test writes the real ones
        self.creative = self.root / "creative"
        self.creative.mkdir()
        real_c = COMEDY / "kb" / "creative"
        for f in ("storyboards.jsonl", "workingsets.jsonl", "geocode_cache.json", "venue_addresses.json"):
            if (real_c / f).is_file():
                shutil.copy(real_c / f, self.creative / f)

    def cleanup(self):
        shutil.rmtree(self.root, ignore_errors=True)


_SCRATCH = None


def scratch_server() -> int:
    """One in-memory SurrealDB 3.3.4 per test process (same binary and credentials as the real one), so tests
    never write to the real on-disk store. Exported as SURREAL_PORT, which db/sdb.py, db/load.py and
    api-dev/server.py honour, so every child process talks to it too."""
    global _SCRATCH
    if _SCRATCH is None:
        import atexit
        port = free_port()
        proc = sdb.start_scratch(port, "memory")
        atexit.register(proc.terminate)
        os.environ["SURREAL_PORT"] = str(port)
        _SCRATCH = (proc, port)
    return _SCRATCH[1]


class TempDB:
    def __init__(self, tag: str):
        scratch_server()
        self.ns = TEST_NS
        self.db = f"t_{tag}_{os.getpid()}_{int(time.time() * 1000) % 10_000_000}"

    def rpc(self) -> sdb.Rpc:
        return sdb.Rpc(None, self.ns, self.db)

    def load(self, t: TempKB) -> dict:
        p = subprocess.run([sys.executable, str(DB_DIR / "load.py"), "--graph", str(t.graph), "--kb", str(t.kb),
                            "--corpus", str(t.corpus), "--ns", self.ns, "--db", self.db]
                           + (["--id-map", str(t.id_map)] if t.id_map else []),
                           capture_output=True, text=True, encoding="utf-8")
        if p.returncode != 0:
            raise RuntimeError("load failed:\n" + p.stdout[-2000:] + p.stderr[-3000:])
        out = p.stdout
        return json.loads(out[out.index("{"):])

    def drop(self):
        r = sdb.Rpc(None, ns=None, db=None)
        try:
            r.query(f"USE NS {self.ns}; REMOVE DATABASE IF EXISTS {self.db};")
        finally:
            r.close()


class Api:
    def __init__(self, t: TempKB, backend: str, db: TempDB | None = None, extra: list | None = None,
                 env: dict | None = None):
        self.port = free_port()
        self.url = f"http://127.0.0.1:{self.port}"
        args = [sys.executable, str(DESK / "api-dev" / "server.py"), "--port", str(self.port), "--kb", str(t.kb),
                "--graph", str(t.graph), "--corpus", str(t.corpus), "--inbox", str(t.inbox), "--backend", backend,
                "--creative-dir", str(t.creative), "--jobs", "worker"]
        if t.id_map:
            args += ["--id-map", str(t.id_map)]
        if db:
            args += ["--ns", db.ns, "--db", db.db]
        self.log = open(t.root / f"api_{backend}_{self.port}.log", "w", encoding="utf-8")
        self.proc = subprocess.Popen(args + (extra or []), stdout=self.log, stderr=subprocess.STDOUT,
                                     env={**os.environ, "PUNCH_API_QUIET": "1", "PYTHONHASHSEED": "0", **(env or {})})
        for _ in range(240):
            if self.proc.poll() is not None:
                raise RuntimeError(f"api exited {self.proc.returncode}: {Path(self.log.name).read_text()[-2000:]}")
            try:
                urllib.request.urlopen(self.url + "/api/health", timeout=2).read()
                return
            except OSError:
                time.sleep(0.25)
        raise RuntimeError("api did not start")

    def get(self, path: str, raw: bool = False, headers: dict | None = None):
        req = urllib.request.Request(self.url + path, headers=headers or {})
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                body = r.read()
                return (r.status, body, dict(r.headers)) if raw else (r.status, json.loads(body))
        except urllib.error.HTTPError as e:
            body = e.read()
            return (e.code, body, dict(e.headers)) if raw else (e.code, json.loads(body or b"{}"))

    def post(self, path: str, body: dict, method: str = "POST", headers: dict | None = None):
        req = urllib.request.Request(self.url + path, data=json.dumps(body).encode(), method=method,
                                     headers={"Content-Type": "application/json", **(headers or {})})
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                return r.status, json.loads(r.read())
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read() or b"{}")

    def stop(self):
        self.proc.terminate()
        try:
            self.proc.wait(10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.log.close()


# ---------------------------------------------------------------- comparison
def diff(a, b, path="$", out=None, tol=1e-9, limit=20):
    """Structural diff; floats equal within tol (relative). Returns a list of 'path: a != b' strings."""
    out = [] if out is None else out
    if len(out) >= limit:
        return out
    if isinstance(a, bool) or isinstance(b, bool):
        if a is not b:
            out.append(f"{path}: {a!r} != {b!r}")
    elif isinstance(a, (int, float)) and isinstance(b, (int, float)):
        if not math.isclose(a, b, rel_tol=tol, abs_tol=tol):
            out.append(f"{path}: {a!r} != {b!r}")
    elif isinstance(a, dict) and isinstance(b, dict):
        for k in sorted(set(a) | set(b), key=str):
            if k not in a or k not in b:
                out.append(f"{path}.{k}: {'missing' if k not in a else 'present'} vs {'missing' if k not in b else 'present'}")
            else:
                diff(a[k], b[k], f"{path}.{k}", out, tol, limit)
    elif isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            out.append(f"{path}: list length {len(a)} != {len(b)}")
        for i, (x, y) in enumerate(zip(a, b)):
            diff(x, y, f"{path}[{i}]", out, tol, limit)
    elif a != b:
        out.append(f"{path}: {str(a)[:80]!r} != {str(b)[:80]!r}")
    return out
