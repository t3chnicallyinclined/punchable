"""Ingest routes for the home worker: every import lands in the database (online-first, 2026-10-27).

The comic: "when we do the ingestion everything should be saved into db, nothing should stay local, this is a full
online application". The PC is a compute worker: it takes a job, downloads what it needs into a temporary workspace,
transcribes and analyses, and posts the results HERE; it keeps nothing as the truth. These routes are the only way
its results get in. Hosted, `authorize()` lets only a worker token reach /api/worker/*; on a loopback dev API with
auth off they are open, as the rest of the API is.

    POST /api/worker/ideas            {text, title?, from?, reason?, actor}      a new idea (+ auto-attach), by actor
    POST /api/worker/decision         {target, action, id?, after?, reason?, actor}
    POST /api/worker/undo             {indexes, actor}
    GET  /api/worker/jokes/<id>                                                 the joke page (what gsync reads)
    GET  /api/worker/files[?prefix=]                                            [{path, sha256, bytes}], files_rev
    GET  /api/worker/files/content?path=                                        {path, sha256, text}
    POST /api/worker/files            {files: [{path, text}], remove?: [path], actor}
    POST /api/worker/events           {store: "merges", op: attach|detach, id, data, actor: ai:*}
    GET  /api/worker/state                                                      what the worker checks first

Every write names its actor (server.ACTOR_RE): `import:gsync`, `import:memo`, `import:youtube`, `import:upload`,
or `ai:<what>`; the comic's own actions never come through here. Court and legal material: every text that arrives
is checked with the same legal rules gsync uses (gsync/rules.py, from the private legal_rules.json); a hit is
refused (422) and nothing of it is stored or echoed. Paths: db/srcpaths.WORKER_PATTERNS only (never audio).

Where files go:
- hosted (`Handler.files_mode = "db"`, set by api/entry.py): a `srcfile` row first (the database is the truth),
  then the API's own cache file under PK_DATA; `meta:state.files_rev` moves so other API pods refresh their cache.
- local dev (`"disk"`): the data folder itself (the files are the truth until the cutover), atomically.
Then, if the file is a graph input (a set's analysis/meta/laughs/transcript, inferred.json, ...), the database's
graph tables are re-synced at once (db/load.py sync: writes only what changed).
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import re
import sys
import threading
from pathlib import Path
from urllib.parse import unquote

HERE = Path(__file__).resolve().parent
DESK = HERE.parent
if str(DESK / "db") not in sys.path:
    sys.path.insert(0, str(DESK / "db"))
import srcpaths as SP  # noqa: E402

import kb as K  # noqa: E402

MAX_FILES_BODY = 24 * 1024 * 1024
POST_PATHS = {"/api/worker/files", "/api/worker/ideas", "/api/worker/decision", "/api/worker/undo", "/api/worker/events"}
_rules = {"mod": None}
_lock = threading.RLock()


def legal_rules():
    """gsync/rules.py loaded from its file (not the gsync package, whose __init__ pulls in Drive code)."""
    if _rules["mod"] is None:
        spec = importlib.util.spec_from_file_location("pk_legal_rules", DESK / "gsync" / "rules.py")
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        _rules["mod"] = mod
    return _rules["mod"]


def is_legal(text: str) -> bool:
    return legal_rules().content_is_legal(text or "")


def sha256(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


class Refused(K.DecisionError):
    pass


def import_actor(h, b: dict) -> str:
    """The actor of a worker write: required, and never the comic's own id (these routes are for imports and
    machine judgments; his own taps go through the app's routes)."""
    a = h.actor(b)
    if not isinstance(b.get("actor"), str) or a != b.get("actor"):
        raise K.DecisionError("a worker write must name its actor: import:<source> or ai:<what>", 400)
    return a


# ---------------------------------------------------------------------------------------------------------- files
def comedy_dir(kb) -> Path:
    return Path(kb.corpus).parent


def data_root(kb) -> Path:
    """Hosted: PK_DATA (comedy/ and vectors/ under it). Local: the data folder's parent is not ours; vectors/ files
    go to <data folder>/db/punchable/cache/vectors."""
    return Path(os.environ["PK_DATA"]) if os.environ.get("PK_DATA") else comedy_dir(kb).parent


def local_path(kb, rel: str) -> Path:
    if rel.startswith("vectors/"):
        if os.environ.get("PK_DATA"):
            return Path(os.environ["PK_DATA"]) / rel
        return comedy_dir(kb) / "db" / "punchable" / "cache" / rel
    return comedy_dir(kb) / rel


def write_atomic(p: Path, text: str):
    p.parent.mkdir(parents=True, exist_ok=True)
    tmp = p.with_name(p.name + f".{os.getpid()}.tmp")
    with open(tmp, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, p)


def list_disk(kb, prefix: str = "") -> list[dict]:
    root = comedy_dir(kb)
    out = []
    for p in sorted(root.rglob("*")):
        if not p.is_file() or p.suffix.lower() not in SP.TEXT_SUFFIXES:
            continue
        rel = p.relative_to(root).as_posix()
        if not rel.startswith(prefix) or not SP.worker_may_write(rel):
            continue
        b = p.read_bytes()
        out.append({"path": rel, "sha256": hashlib.sha256(b).hexdigest(), "bytes": len(b)})
    return out


def files_rev(kb) -> int | None:
    if getattr(kb, "backend", "files") != "surreal":
        return None
    import sdb
    st = kb.rpc.last("SELECT files_rev FROM ONLY meta:state;")
    return sdb.strip_none(st).get("files_rev") if isinstance(st, dict) else None


def list_files(h, kb, prefix: str = "") -> list[dict]:
    if getattr(h, "files_mode", "disk") == "db":
        rows = kb.rpc.last("SELECT path, sha256, bytes FROM srcfile WHERE string::starts_with(path, $p) ORDER BY path;",
                           {"p": prefix})
        return [{"path": r["path"], "sha256": r["sha256"], "bytes": r["bytes"]} for r in rows]
    return list_disk(kb, prefix)


def file_content(h, kb, rel: str) -> dict | None:
    rel = SP.clean(rel)
    if not rel:
        raise K.DecisionError("bad path")
    if getattr(h, "files_mode", "disk") == "db":
        rows = kb.rpc.last("SELECT path, sha256, text FROM srcfile WHERE path = $p LIMIT 1;", {"p": rel})
        return rows[0] if rows else None
    if not SP.worker_may_write(rel):
        raise K.DecisionError("not a data file", 403)
    p = local_path(kb, rel)
    if not p.is_file():
        return None
    text = p.read_text(encoding="utf-8")
    return {"path": rel, "sha256": sha256(text), "text": text}


def current_sha(h, kb, rel: str) -> str | None:
    if getattr(h, "files_mode", "disk") == "db":
        rows = kb.rpc.last("SELECT VALUE sha256 FROM srcfile WHERE path = $p LIMIT 1;", {"p": rel})
        return rows[0] if rows else None
    p = local_path(kb, rel)
    return hashlib.sha256(p.read_bytes()).hexdigest() if p.is_file() else None


def put_files(h, kb, body: dict) -> dict:
    """Validate everything first (actor, whitelist, size, text, legal), then write: database row, then the cache file."""
    actor = import_actor(h, body)
    files, remove = body.get("files") or [], body.get("remove") or []
    if not isinstance(files, list) or not isinstance(remove, list) or len(files) + len(remove) > 500:
        raise K.DecisionError("files and remove must be lists (up to 500 entries)")
    clean = []
    for f in files:
        if not isinstance(f, dict) or not isinstance(f.get("text"), str):
            raise K.DecisionError("each file needs {path, text}")
        rel = SP.clean(f.get("path"))
        if not rel or not SP.worker_may_write(rel):
            raise K.DecisionError(f"not a file the worker may write: {str(f.get('path'))[:200]!r}", 403)
        b = f["text"].encode("utf-8")
        if len(b) > SP.MAX_FILE_BYTES:
            raise K.DecisionError(f"{rel}: over {SP.MAX_FILE_BYTES // 1_000_000} MB", 413)
        if rel.endswith(".json"):
            try:
                json.loads(f["text"])
            except ValueError:
                raise K.DecisionError(f"{rel}: not JSON")
        if f.get("sha256") and f["sha256"] != hashlib.sha256(b).hexdigest():
            raise K.DecisionError(f"{rel}: sha256 does not match the text")
        if is_legal(f["text"]):
            raise Refused(f"{rel}: refused by the legal rules (court material is never stored); nothing was kept", 422)
        clean.append((rel, f["text"]))
    gone = []
    for x in remove:
        rel = SP.clean(x)
        if not rel or not SP.worker_may_write(rel):
            raise K.DecisionError(f"not a file the worker may remove: {str(x)[:200]!r}", 403)
        gone.append(rel)
    db = getattr(h, "files_mode", "disk") == "db"
    written, unchanged = [], 0
    with _lock:
        have = {rel: current_sha(h, kb, rel) for rel in [r for r, _ in clean] + gone}
        for rel, text in clean:
            sha = sha256(text)
            if have.get(rel) == sha:
                unchanged += 1
                continue
            if db:
                key = hashlib.sha1(rel.encode()).hexdigest()
                kb.rpc.write("UPSERT type::record('srcfile', $k) CONTENT $c RETURN id;",
                             {"k": key, "c": {"path": rel, "sha256": sha, "bytes": len(text.encode("utf-8")), "text": text}},
                             expect=1)
            write_atomic(local_path(kb, rel), text)
            written.append(rel)
        removed = []
        for rel in gone:
            if not have.get(rel):
                continue
            if db:
                kb.rpc.query("DELETE srcfile WHERE path = $p RETURN NONE;", {"p": rel})
            local_path(kb, rel).unlink(missing_ok=True)
            removed.append(rel)
        if db and (written or removed):
            kb.rpc.write("UPDATE meta:state SET files_rev = (files_rev ?? 0) + 1 RETURN id;")
    synced = None
    if any(SP.is_set_input(r) for r in written + removed) and hasattr(kb, "sync_now"):
        synced = kb.sync_now("worker files: " + ", ".join(sorted({r.split("/")[0] + "/" + r.split("/")[1] for r in written + removed})))
    return {"ok": True, "written": written, "removed": removed, "unchanged": unchanged, "synced": synced,
            "files_rev": files_rev(kb), "actor": actor}


# ---------------------------------------------------------------------------------------------------------- events
EVENT_OPS = {"merges": {"attach", "detach"}}


def post_event(h, kb, body: dict) -> dict:
    actor = import_actor(h, body)
    if not actor.startswith("ai:"):
        raise K.DecisionError("events from the worker are machine judgments: actor must be ai:<what>", 400)
    store, op, eid, data = body.get("store"), body.get("op"), body.get("id"), body.get("data") or {}
    if store not in EVENT_OPS or op not in EVENT_OPS[store]:
        raise K.DecisionError(f"store/op must be one of {EVENT_OPS}")
    if not isinstance(eid, str) or not re.match(r"^at:[A-Za-z0-9]{6,40}$", eid) or not isinstance(data, dict):
        raise K.DecisionError("id must be at:<hex> and data an object")
    if op == "attach" and is_legal(str(data.get("text") or "")):
        raise Refused("refused by the legal rules; nothing was kept", 422)
    import merge_api as MG
    cr = h.creative
    if cr is None:
        raise K.DecisionError("creative stores are off on this API", 503)
    with cr.lock:
        st = MG.store(cr, kb)
        cur = st.items.get(eid)
        if op == "attach" and cur and cur.get("status") == "attached":
            return {"ok": True, "id": eid, "already": True}
        if op == "detach" and (not cur or cur.get("status") != "attached"):
            return {"ok": True, "id": eid, "already": True}
        try:
            rec = st.append(op, eid, data, by=actor)
        except (ValueError, KeyError, TypeError) as e:
            raise K.DecisionError(f"{op} refused: {e}")
    return {"ok": True, "id": eid, "record": MG.brief(rec)}


# ---------------------------------------------------------------------------------------------------------- routes
def handle(h, method: str, path: str, q: dict) -> bool:
    """Serve one of our routes; False if the path is not ours. `h` is server.Handler (h.kb already picked)."""
    if not path.startswith("/api/worker/"):
        return False
    kb = h.kb
    arg = lambda k, d=None: (q.get(k) or [d])[0]  # noqa: E731
    try:
        if method == "GET":
            m = re.match(r"^/api/worker/jokes/(.+)$", path)
            if m:
                h.joke(kb.app(), unquote(m.group(1)))
                return True
            if path == "/api/worker/files":
                h.send_json({"files": list_files(h, kb, arg("prefix", "") or ""), "files_rev": files_rev(kb),
                             "mode": getattr(h, "files_mode", "disk")})
                return True
            if path == "/api/worker/files/content":
                got = file_content(h, kb, arg("path") or "")
                if not got:
                    h.err(404, "no such file")
                else:
                    h.send_json(got)
                return True
            if path == "/api/worker/state":
                v = kb.full_view()
                h.send_json({"decisions": len(v.decisions), "files_rev": files_rev(kb),
                             "files_mode": getattr(h, "files_mode", "disk"), "backend": getattr(kb, "backend", "files"),
                             "online_first": online_first(kb), "legal_rules": legal_rules().fingerprint()[:12]})
                return True
            return False
        if method != "POST" or path not in POST_PATHS:
            return False  # not ours: the body is left unread for the route that owns it
        if path == "/api/worker/files":
            b = h.body_json(MAX_FILES_BODY)
            h.send_json(put_files(h, kb, b))
            return True
        b = h.body_json()
        if path == "/api/worker/ideas":
            if is_legal(b.get("text") or "") or is_legal(b.get("title") or ""):
                raise Refused("refused by the legal rules (court material is never stored); nothing was kept", 422)
            h.idea(b, by=import_actor(h, b))
            enqueue_followups(h, kb, import_actor(h, b))
            return True
        if path == "/api/worker/decision":
            if is_legal(json.dumps(b.get("after") or {}, ensure_ascii=False)):
                raise Refused("refused by the legal rules; nothing was kept", 422)
            if b.get("id") is not None and (not isinstance(b["id"], str) or not re.match(r"^[A-Za-z0-9:_@.+\-]{1,300}$", b["id"])):
                raise K.DecisionError("bad id")
            r = kb.decide(b, by=import_actor(h, b))
            h.send_json({"ok": True, "indexes": [r["index"]], "decisions": [r["decision"]]})
            return True
        if path == "/api/worker/undo":
            idxs = b.get("indexes")
            if not isinstance(idxs, list) or not idxs or not all(isinstance(i, int) for i in idxs):
                raise K.DecisionError("indexes must be a non-empty list of integers")
            by = import_actor(h, b)
            out = []
            with kb.lock:
                for i in sorted(idxs, reverse=True):
                    out.append(kb.undo(i, by=by))
            h.send_json({"ok": True, "indexes": [r["index"] for r in out], "decisions": [r["decision"] for r in out]})
            return True
        if path == "/api/worker/events":
            h.send_json(post_event(h, kb, b))
            return True
        return False
    except K.DecisionError as e:
        h.err(e.status, str(e))
        return True


def enqueue_followups(h, kb, actor: str):
    """After an imported idea: vectors for it (job `embed`, cheap), then the memo <-> bit linking (job `memolink`:
    judge, triage, clear attaches; cached, so a run with nothing new costs no model calls). One pending job each."""
    cr = h.creative
    if cr is None or not actor.startswith("import:"):
        return
    try:
        with cr.lock:
            st = cr.stores(kb)
            cr.enqueue(st, "embed", "embed", {}, fresh=True, by=actor)
            if actor in ("import:memo", "import:upload"):
                cr.enqueue(st, "memolink", "memolink", {}, fresh=True, by=actor)
    except Exception as e:  # noqa: BLE001  the idea is saved either way
        sys.stderr.write(f"followup jobs not queued: {type(e).__name__}: {e}\n")


def online_first(kb) -> bool:
    """meta:state.online_first: set at the cutover (db/push_remote.py cutover). After it, a bundle load is refused."""
    if getattr(kb, "backend", "files") != "surreal":
        return False
    import sdb
    st = kb.rpc.last("SELECT online_first FROM ONLY meta:state;")
    return bool(sdb.strip_none(st).get("online_first")) if isinstance(st, dict) else False
