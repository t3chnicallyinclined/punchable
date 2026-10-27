"""Punchable API, hosted (the server, infra/k8s/30-api.yaml). The same code as api-dev/server.py, configured from env:

    PORT=8080  PUBLIC_URL  OIDC_ISSUER  OIDC_AUDIENCE  OIDC_JWKS_URL  PK_AUTH (default 1 here)
    SURREAL_URL=ws://surrealdb:8000/rpc  SURREAL_NS  SURREAL_DB  SURREAL_API_USER  SURREAL_API_PASS (database EDITOR)
    PK_DATA=/tmp/punchable   (the only writable place: an emptyDir)

The database is the source of truth here. db/push_remote.py (infra/ship.sh dbload) puts the graph, decisions and the
text files the API reads (transcripts, analyses, clips, creative logs, vectors as JSON) into SurrealDB; at start, and
whenever a new bundle is loaded, this process writes those files under PK_DATA and serves from them. Decisions and
creative events are read from the database (append-only rows); the files under PK_DATA are a cache of them.

The HTTP server answers at once: /api/health is 200 when SurrealDB answers (`loaded: false` until the first
dbload) and 503 when it does not, so readiness reflects the database. No ollama/claude here: drafts and ask are
queued for the home worker (worker/), and search is BM25 (hybrid falls back to full text without a query vector).
"""
from __future__ import annotations

import json
import os
import shutil
import sys
import threading
import time
from http.server import ThreadingHTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
DESK = HERE.parent
sys.path.insert(0, str(DESK / "api-dev"))
sys.path.insert(0, str(DESK / "db"))
os.environ.setdefault("PK_AUTH", "1")
os.environ.setdefault("PUNCH_API_QUIET", "1")

import server as SV  # noqa: E402
import auth as AUTH  # noqa: E402
import sdb  # noqa: E402

DATA = Path(os.environ.get("PK_DATA", "/tmp/punchable"))
COMEDY = DATA / "comedy"
VEC = DATA / "vectors"
STATE = {"surreal": False, "loaded": False, "error": "starting", "bundle": None, "since": time.time()}


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, file=sys.stderr, flush=True)


class _NoEmbedder:
    def embed(self, *a, **k):
        raise RuntimeError("no embedding model in the hosted image")


def materialize(r: sdb.Rpc, bundle: dict):
    """srcfile rows -> files under DATA (one row at a time, so memory stays flat). Vector parts -> .f32 caches."""
    import numpy as np
    rows = r.last("SELECT path, sha256 FROM srcfile;")
    tmp = DATA / "incoming"
    shutil.rmtree(tmp, ignore_errors=True)
    for x in rows:
        p = x["path"]
        if p.startswith("/") or ".." in Path(p).parts:
            raise RuntimeError(f"bad srcfile path {p!r}")
        text = r.last("SELECT VALUE text FROM srcfile WHERE path = $p LIMIT 1;", {"p": p})[0]
        out = tmp / (p if p.startswith("vectors/") else "comedy/" + p)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(text, encoding="utf-8")
    # vectors/<cache>.partNNN.json -> vectors/<cache> + ids (creative.data.Vectors reads the f32 caches)
    vdir = tmp / "vectors"
    if vdir.is_dir():
        caches: dict[str, tuple[str, list]] = {}
        for p in sorted(vdir.glob("*.part*.json")):
            d = json.loads(p.read_text(encoding="utf-8"))
            ids_file, acc = caches.setdefault(d["cache"], (d["ids_file"], []))
            acc += d["ids"]
            with open(vdir / d["cache"], "ab") as f:
                np.asarray(d["rows"], dtype=np.float32).tofile(f)
            p.unlink()
        for cache, (ids_file, acc) in caches.items():
            (vdir / ids_file).write_text(json.dumps(acc), encoding="utf-8")
    # swap in: comedy/ (keeping the API-written decision and creative caches) and vectors/
    for sub in ("comedy", "vectors"):
        new, cur = tmp / sub, DATA / sub
        if not new.exists():
            new.mkdir(parents=True)
        if sub == "comedy":
            for keep in [cur / "kb" / "v0" / "decisions.jsonl"] + sorted((cur / "kb" / "creative").glob("*.json*")) \
                    if cur.exists() else []:
                if keep.is_file() and not (new / keep.relative_to(cur)).exists():
                    (new / keep.relative_to(cur)).parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(keep, new / keep.relative_to(cur))
        old = DATA / f"old-{sub}"
        shutil.rmtree(old, ignore_errors=True)
        if cur.exists():
            os.replace(cur, old)
        os.replace(new, cur)
        shutil.rmtree(old, ignore_errors=True)
    shutil.rmtree(tmp, ignore_errors=True)
    (COMEDY / "kb" / "creative").mkdir(parents=True, exist_ok=True)
    log(f"materialized {len(rows)} files from bundle {bundle.get('sha', '')[:12]}")


def hosted_kb_class():
    cls, SK = SV.surreal_kb_class()
    from kbmap import row_to_decision

    class HostedKB(cls):
        """The database is canonical here: decisions are read from it (the /tmp jsonl is only a cache)."""

        def _read_file_decisions(self):
            return [row_to_decision(x) for x in self.rpc.last("SELECT * FROM decision ORDER BY seq;")], 0

        def _write_line(self, d: dict):
            super()._write_line(d)

    return HostedKB


def write_decisions_cache(kb):
    lines, _ = kb._read_file_decisions()
    p = kb.decisions_path
    p.parent.mkdir(parents=True, exist_ok=True)
    tmp = p.with_suffix(".tmp")
    tmp.write_text("".join(json.dumps(d, ensure_ascii=False, separators=(",", ":")) + "\n" for d in lines), encoding="utf-8")
    os.replace(tmp, p)


def boot(ns: str, db: str):
    """Until the data is there: check SurrealDB every 5 s. Then build the KB, and keep watching for new bundles."""
    kb = None
    while True:
        try:
            r = sdb.Rpc(None, ns=ns, db=db)
            try:
                sdb.check_version(None)
                STATE["surreal"] = True
                info = {}
                if "meta" in (r.last("INFO FOR DB;") or {}).get("tables", {}):
                    st = r.last("SELECT load_info FROM ONLY meta:state;")
                    info = (sdb.strip_none(st) or {}).get("load_info") or {} if isinstance(st, dict) else {}
                bundle = info.get("bundle")
                STATE["no_bundle"] = not bundle
                if not bundle:
                    STATE.update(loaded=False, error="the data is not loaded yet (infra/ship.sh dbload)")
                elif bundle.get("sha") != (STATE["bundle"] or {}).get("sha"):
                    log("new bundle", bundle.get("sha", "")[:12])
                    lock = kb.lock if kb else threading.RLock()
                    with lock:
                        materialize(r, bundle)
                        if kb is None:
                            kb = start_kb(ns, db, bundle)
                        else:
                            write_decisions_cache(kb)
                            kb._sview = None
                            # a load may have rebuilt the creative rows in a new order: re-read them from the DB
                            for n in ("storyboards", "workingsets", "venues"):
                                for f in (COMEDY / "kb" / "creative").glob(f"{n}.json*"):
                                    f.unlink()
                    STATE.update(loaded=True, error=None, bundle=bundle)
            finally:
                r.close()
        except Exception as e:  # noqa: BLE001
            STATE.update(surreal=False if "connect" in str(e).lower() or isinstance(e, OSError) else STATE["surreal"],
                         error=f"{type(e).__name__}: {str(e)[:200]}")
            log("boot:", STATE["error"])
        time.sleep(5 if not STATE["loaded"] else 15)


def start_kb(ns: str, db: str, bundle: dict):
    cls = hosted_kb_class()
    graph = COMEDY / bundle["graph"]
    id_map = COMEDY / bundle["id_map"] if bundle.get("id_map") else None
    kbdir = COMEDY / "kb" / "v0"
    kb = cls(kbdir, COMEDY / "corpus", DATA / "tapes", DATA / "inbox", None, graph=graph, id_map=id_map)
    kb.init_surreal(ns, db, None, live=True, watch=False)
    kb.creative_graph = graph  # load_info.graph is the dbload pod's path; creative/ reads the copy here
    kb.embedder = _NoEmbedder()
    write_decisions_cache(kb)
    import creative_api as CR
    SV.Handler.creative = CR.Creative(COMEDY / "kb" / "creative", VEC if VEC.is_dir() else None, geocode=False,
                                      jobs_mode="worker")
    SV.Handler.primary = "surreal"
    SV.Handler.jobs_on_upload = True
    t = time.time()
    kb.app().summaries()
    SV.Handler.kbs = {"surreal": kb}
    log(f"serving {len(kb.view().jokes)} jokes, {len(kb.view().decisions)} decisions (warm {time.time() - t:.1f}s)")
    return kb


def boot_status() -> dict:
    # ready (the readiness probe, /api/health?ready=1): serving data, or nothing to serve yet (no dbload has ever run).
    # A pod that is still materializing a bundle is NOT ready, so a rollout keeps the old pod until the new one serves.
    ready = STATE["loaded"] or (STATE["surreal"] and bool(STATE.get("no_bundle")))
    return {"ok": STATE["loaded"], "surreal": STATE["surreal"], "loaded": STATE["loaded"], "ready": ready,
            "error": STATE["error"]}


def main():
    port = int(os.environ.get("PORT", "8080"))
    ns, db = os.environ.get("SURREAL_NS", "punchable"), os.environ.get("SURREAL_DB", "main")
    DATA.mkdir(parents=True, exist_ok=True)
    verifier = AUTH.from_env()
    if verifier is None and os.environ.get("PK_ALLOW_NO_AUTH") != "1":
        raise SystemExit("the hosted API runs with PK_AUTH=1 (set PK_ALLOW_NO_AUTH=1 only for a local measurement)")
    SV.Handler.verifier = verifier
    SV.Handler.origins = tuple(x for x in [os.environ.get("PUBLIC_URL", "").rstrip("/")] if x)
    SV.Handler.kbs = {}
    SV.Handler.boot = staticmethod(boot_status)
    threading.Thread(target=boot, args=(ns, db), daemon=True, name="boot").start()
    host = os.environ.get("HOST", "0.0.0.0")
    srv = ThreadingHTTPServer((host, port), SV.Handler)
    srv.daemon_threads = True
    log(f"punchable api on {host}:{port}, auth {'on' if verifier else 'OFF'}, data {DATA}")
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
