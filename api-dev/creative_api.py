"""Creative routes for the Punchable API (docs/API_CONTRACT.md), backed by creative/.

Every response body is what creative/ returns (creative/CREATIVE.md), served as-is. This module adds only:
- **Stores**: the comic's writes (storyboards, working sets, venue pins) and the draft job queue are creative/store.py
  stores: an append-only `<name>.jsonl` plus a snapshot. On the SurrealDB backend each event is ALSO a
  `creative_event` row (db/schema_extra.surql), written FIRST (a unique (store, seq) index makes a double append
  impossible), then the same line is appended to the jsonl. LIVE SELECT on `creative_event` pushes it to the app
  exactly like a decision. A reconcile step keeps file and table equal: whichever is a strict prefix of the other is
  extended (the CLI wrote a line locally; the hosted pod rebuilt its /tmp files from the database). If they diverge,
  writes are refused (409) until resolved; nothing is merged silently.
- **Reads per backend**: `?backend=files` replays the jsonl and builds the corpus with decisions.jsonl; SurrealDB
  replays `creative_event` and builds the corpus with the decisions from the database. db/tests/test_creative.py
  diffs the two.
- **Drafts never block**: a bridge or punch-up returns the cached draft (a finished job with the same key), or
  `202` with a queued job for the home worker (worker/; claude_code runs only on the comic's PC).
"""
from __future__ import annotations

import copy
import hashlib
import json
import os
import re
import sys
import threading
import time
from pathlib import Path
from urllib.parse import unquote

HERE = Path(__file__).resolve().parent
DESK = HERE.parent
if str(DESK) not in sys.path:
    sys.path.insert(0, str(DESK))

from creative import connections as C  # noqa: E402
from creative import notebook as N  # noqa: E402
from creative import story as S  # noqa: E402
from creative import today as T  # noqa: E402
from creative import venues as V  # noqa: E402
from creative import workingset as W  # noqa: E402
from creative.data import Corpus, Vectors, set_key_from_dir  # noqa: E402
from creative.store import Store  # noqa: E402
from creative.util import mmss, now_iso  # noqa: E402

OWNER_ID = os.environ.get("PK_OWNER_ID", "owner")  # the actor id on your own edits (`by`); PK_OWNER_ID

JOB_KINDS = ("bridge", "punchup", "ask", "process_set", "import_youtube", "facts", "riff", "ask_chat", "embed", "memolink")
# Jobs that download or crunch tapes: only the home worker runs them, never this process (not even --jobs inproc).
WORKER_ONLY = {"process_set", "import_youtube", "embed", "memolink"}
STALE_CLAIM_S = 15 * 60          # a claimed job with no result after this is handed out again
MAX_ATTEMPTS = 3
ID_RE = re.compile(r"^[A-Za-z0-9:_@.+\-]{1,300}$")
YT_RE = re.compile(r"^https?://(?:www\.|m\.|music\.)?(?:youtube\.com/(?:watch\?(?:[^#]*&)?v=|shorts/|live/|embed/)|youtu\.be/)"
                   r"([A-Za-z0-9_-]{11})(?:[?&#/].*)?$")
AI_NOTE_RE = re.compile(r"^joke:[^|]{1,280}\|(put_back|try_without|dropped|over_time|slow_start)$")


class HttpError(Exception):
    def __init__(self, status: int, msg: str):
        super().__init__(msg)
        self.status = status


# ====================================================================== stores
def venue_apply(items: dict, ev: dict) -> None:
    """the comic moves or renames a map pin. Ops: pin {lat, lon} · rename {name} · reset."""
    op, i, d = ev["op"], ev["id"], ev.get("data") or {}
    v = items.setdefault(i, {"venue_id": i, "lat": None, "lon": None, "name": None, "by": None, "updated_at": None})
    if op == "pin":
        lat, lon = d.get("lat"), d.get("lon")
        if not all(isinstance(x, (int, float)) and not isinstance(x, bool) for x in (lat, lon)):
            raise ValueError("lat and lon must be numbers")
        if not (-90 <= lat <= 90 and -180 <= lon <= 180):
            raise ValueError("lat/lon out of range")
        v["lat"], v["lon"] = float(lat), float(lon)
    elif op == "rename":
        name = (d.get("name") or "").strip()
        if not name or len(name) > 120:
            raise ValueError("name must be 1-120 characters")
        v["name"] = name
    elif op == "reset":
        v.update(lat=None, lon=None, name=None)
    else:
        raise ValueError(f"unknown op {op}")
    v["by"], v["updated_at"] = ev.get("by"), ev["ts"]


def job_apply(items: dict, ev: dict) -> None:
    """The draft/job queue. Ops: enqueue {kind, key, input} · claim {worker} · done {result} · fail {error, retry}."""
    op, i, d = ev["op"], ev["id"], ev.get("data") or {}
    if op == "enqueue":
        if i in items:
            raise ValueError(f"job {i} exists")
        if d.get("kind") not in JOB_KINDS:
            raise ValueError(f"job kind must be one of {JOB_KINDS}")
        items[i] = {"id": i, "kind": d["kind"], "key": d.get("key"), "input": d.get("input") or {}, "status": "queued",
                    "created_at": ev["ts"], "by": ev.get("by"), "attempts": 0, "claimed_at": None, "worker": None,
                    "done_at": None, "result": None, "error": None}
        return
    j = items.get(i)
    if j is None:
        raise ValueError(f"no job {i}")
    if op == "claim":
        if j["status"] not in ("queued", "claimed"):
            raise ValueError(f"job {i} is {j['status']}")
        j.update(status="claimed", claimed_at=ev["ts"], worker=d.get("worker"), attempts=j["attempts"] + 1)
    elif op == "done":
        if j["status"] != "claimed":
            raise ValueError(f"job {i} is {j['status']}, not claimed")
        j.update(status="done", done_at=ev["ts"], result=d.get("result"), error=None)
    elif op == "fail":
        if j["status"] not in ("claimed", "queued"):
            raise ValueError(f"job {i} is {j['status']}")
        retry = bool(d.get("retry")) and j["attempts"] < MAX_ATTEMPTS
        j.update(status="queued" if retry else "failed", error=str(d.get("error") or "failed")[:2000],
                 done_at=None if retry else ev["ts"])
    else:
        raise ValueError(f"unknown op {op}")


WORKER_ONLINE_S = 120          # the home worker polls /api/worker/claim every few seconds; quiet this long = offline
GSYNC_STALE_S = 24 * 3600      # gsync --watch runs every minute; no run in a day = "not syncing"
GSYNC_JOKE_ORIGINS = ("gdocs", "app", "mixed")


def gsync_status(d: Path, hosted: bool) -> dict:
    """Settings > Connections > Google Docs, read from gsync's own files (gsync/GSYNC.md, comedy/kb/gsync/).
    Read-only, counts only: no titles, no text, no doc IDs, nothing about the legal exclusions."""
    out = {"status": "not_set_up", "last_sync": None, "last_ok": None, "docs_linked": 0, "joke_docs": 0,
           "boards": 0, "conflicts": 0, "errors_last_run": 0, "last_mode": None, "where": "your PC"}
    mp, rp = d / "map.json", d / "runs.jsonl"
    if not mp.is_file() and not rp.is_file():
        out["status"] = "not_reported" if hosted else "not_set_up"
        return out
    try:
        m = json.loads(mp.read_text(encoding="utf-8")) if mp.is_file() else {}
    except (OSError, ValueError):
        m = {}
    docs = [e for e in (m.get("docs") or {}).values() if isinstance(e, dict) and not e.get("hidden")]
    out["docs_linked"] = len(docs)
    out["joke_docs"] = sum(1 for e in docs if e.get("origin") in GSYNC_JOKE_ORIGINS)
    out["boards"] = len(m.get("boards") or {})
    runs = [r for r in _jsonl_tail(rp) if not r.get("dry_run")]
    cp = d / "conflicts.jsonl"
    if cp.is_file():
        with cp.open("rb") as f:
            out["conflicts"] = sum(1 for line in f if line.strip())
    if not runs:
        out["status"] = "not_synced_yet"
        return out
    last = runs[-1]
    ok = [r for r in runs if not r.get("errors") and r.get("mode") != "api_down"]
    out["last_sync"] = last.get("ts_end") or last.get("ts")
    out["last_ok"] = (ok[-1].get("ts_end") or ok[-1].get("ts")) if ok else None
    out["last_mode"] = last.get("mode")
    out["errors_last_run"] = len(last.get("errors") or [])
    if last.get("mode") == "api_down":
        out["status"] = "api_down"
    elif out["errors_last_run"]:
        out["status"] = "error"
    elif _age_s(out["last_sync"]) > GSYNC_STALE_S:
        out["status"] = "stale"
    else:
        out["status"] = "connected"
    return out


def _jsonl_tail(p: Path, max_bytes: int = 256 * 1024) -> list[dict]:
    """The last runs only: runs.jsonl grows by one line a minute under --watch."""
    if not p.is_file():
        return []
    with p.open("rb") as f:
        f.seek(0, os.SEEK_END)
        n = f.tell()
        f.seek(max(0, n - max_bytes))
        raw = f.read().decode("utf-8", "replace")
    lines = raw.splitlines()[1:] if n > max_bytes else raw.splitlines()
    out = []
    for line in lines:
        try:
            r = json.loads(line)
        except ValueError:
            continue  # a torn last line while gsync is writing
        if isinstance(r, dict):
            out.append(r)
    return out


def _age_s(ts: str | None) -> float:
    if not ts:
        return float("inf")
    from datetime import datetime, timezone
    try:
        t = datetime.fromisoformat(ts.replace("Z", "+00:00"))
    except ValueError:
        return float("inf")
    if t.tzinfo is None:
        t = t.replace(tzinfo=timezone.utc)
    return time.time() - t.timestamp()


SETTINGS_DEFAULTS = {"provider": "hosted", "training": False, "upload_video": False, "cellular": False}
PROVIDERS = ("hosted", "chatgpt", "key", "custom")
DELETE_CONFIRM = "delete my material"


def check_settings(d: dict) -> dict:
    """The settings the comic can change (app Settings, UX_SPEC 4.13): provider + three switches. Unknown keys refused."""
    if not isinstance(d, dict) or not d:
        raise ValueError("send one or more of provider, training, upload_video, cellular")
    bad = sorted(set(d) - set(SETTINGS_DEFAULTS))
    if bad:
        raise ValueError(f"unknown setting(s): {', '.join(bad)}")
    if "provider" in d and d["provider"] not in PROVIDERS:
        raise ValueError(f"provider must be one of {', '.join(PROVIDERS)}")
    for k in ("training", "upload_video", "cellular"):
        if k in d and not isinstance(d[k], bool):
            raise ValueError(f"{k} must be true or false")
    return d


def account_apply(items: dict, ev: dict) -> None:
    """His account: settings (item `settings`) and deletion requests (item `deletion`). Ops: settings {...} ·
    request_deletion {by_sub, would_delete}. A request only records that he asked; an owner deletes by hand."""
    op, i, d = ev["op"], ev["id"], ev.get("data") or {}
    if op == "settings":
        if i != "settings":
            raise ValueError("settings live on item 'settings'")
        it = items.setdefault(i, {"id": i, **SETTINGS_DEFAULTS, "updated_at": None})
        it.update(check_settings(dict(d)))
        it["updated_at"] = ev["ts"]
    elif op == "request_deletion":
        if i != "deletion":
            raise ValueError("deletion requests live on item 'deletion'")
        it = items.setdefault(i, {"id": i, "requests": []})
        it["requests"].append({"at": ev["ts"], "by_sub": d.get("by_sub"), "would_delete": d.get("would_delete") or [],
                               "status": "requested"})
    else:
        raise ValueError(f"unknown op {op}")


APPLY = {"storyboards": S._apply, "workingsets": W._apply, "venues": venue_apply, "jobs": job_apply,
         "account": account_apply}
STORES = tuple(APPLY)


def _h(line: str) -> str:
    return hashlib.sha256(line.encode("utf-8")).hexdigest()


def file_lines(p: Path) -> list[str]:
    """The jsonl's event lines, exactly as written (a torn last line is skipped, as Store.events does)."""
    out = []
    try:
        with open(p, encoding="utf-8") as f:
            for line in f:
                line = line.rstrip("\n").rstrip("\r")
                if not line.strip():
                    continue
                try:
                    json.loads(line)
                except ValueError:
                    continue
                out.append(line)
    except FileNotFoundError:
        pass
    return out


def _append_file(p: Path, line: str) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    with open(p, "ab") as f:
        if f.tell() > 0:
            with open(p, "rb") as r:
                r.seek(-1, os.SEEK_END)
                if r.read(1) != b"\n":
                    f.write(b"\n")
        f.write(line.encode("utf-8") + b"\n")
        f.flush()
        os.fsync(f.fileno())


class LineStore(Store):
    """creative/store.Store replayed from a given list of lines (the database's), writing through `writer`."""

    def __init__(self, directory: Path, name: str, lines: list[str], writer):
        self._lines = lines
        self._writer = writer
        super().__init__(directory, name, APPLY[name])

    def events(self) -> list[dict]:
        return [json.loads(x) for x in self._lines]

    def append(self, op: str, id_: str, data: dict, by: str = OWNER_ID, ts: str | None = None) -> dict:
        ev = {"ts": ts or now_iso(), "op": op, "id": id_, "by": by, "data": data}
        probe = json.loads(json.dumps(self.items))  # validate on a copy: a bad op never reaches the log
        self._apply(probe, ev)
        line = json.dumps(ev, ensure_ascii=False)
        self._writer(self.name, len(self._lines), line, ev)
        self._lines.append(line)
        self.items = probe
        self.n_events += 1
        self.write_snapshot(ev["ts"])
        return self.items.get(id_)


class Mirror:
    """creative_event rows in SurrealDB (one connection, the SurrealDB KB's)."""

    def __init__(self, kb):
        self.kb = kb

    def lines(self, store: str) -> list[str]:
        rows = self.kb.rpc.last("SELECT seq, line FROM creative_event WHERE store = $s ORDER BY seq;", {"s": store})
        out = [r["line"] for r in rows]
        if [r["seq"] for r in rows] != list(range(len(rows))):
            raise HttpError(500, f"creative_event {store}: sequence has gaps")
        return out

    def create(self, store: str, seq: int, line: str, ev: dict | None = None) -> None:
        import sdb
        ev = ev if ev is not None else json.loads(line)
        sdb_row = {"store": store, "seq": seq, "line": line, "h": _h(line), "op": ev.get("op"), "item": ev.get("id"),
                   "at": ev.get("ts")}
        self.kb.rpc.write("CREATE creative_event CONTENT $row RETURN id;", {"row": sdb_row}, expect=1)


class Creative:
    """Creative state for one API process (one creative folder), shared by both backends."""

    def __init__(self, creative_dir: Path, vec_dir: Path | None, geocode: bool = False, jobs_mode: str = "worker"):
        self.dir = Path(creative_dir)
        self.vec_dir = Path(vec_dir) if vec_dir else None
        self.geocode = geocode
        self.jobs_mode = jobs_mode
        self.lock = threading.RLock()
        self.conflicts: dict[str, str] = {}
        # Settings > Connections: gsync writes its state next to the creative folder (comedy/kb/gsync) on his PC
        self.gsync_dir = Path(os.environ.get("PK_GSYNC_DIR") or self.dir.parent / "gsync")
        self.worker_seen: dict | None = None   # the last /api/worker/claim (in memory: "online" is about now)
        self._corpus: dict[str, tuple] = {}
        self._vectors = None
        os.environ["PUNCHABLE_CREATIVE_DIR"] = str(self.dir)  # creative.venues / story read it

    # ---------------------------------------------------------------- stores
    def _mirror(self, kb) -> Mirror | None:
        return Mirror(kb) if getattr(kb, "backend", "files") == "surreal" else None

    def reconcile(self, kb, name: str) -> list[str] | None:
        """File vs database for one store. Returns the database lines (after extending the shorter side), or None
        with no database. A divergence is recorded in self.conflicts and refuses writes."""
        m = self._mirror(kb)
        if m is None:
            return None
        p = self.dir / f"{name}.jsonl"
        with self.lock:
            fl, dl = file_lines(p), m.lines(name)
            if fl == dl:
                self.conflicts.pop(name, None)
            elif fl[:len(dl)] == dl:            # another writer appended to the file: copy the tail into the DB
                for i in range(len(dl), len(fl)):
                    m.create(name, i, fl[i])
                dl = fl[:]
                self.conflicts.pop(name, None)
            elif dl[:len(fl)] == fl:            # the DB is ahead (hosted /tmp rebuilt, a write from elsewhere)
                for line in dl[len(fl):]:
                    _append_file(p, line)
                self.conflicts.pop(name, None)
            else:
                k = next(i for i in range(min(len(fl), len(dl))) if fl[i] != dl[i])
                self.conflicts[name] = (f"{name}: {p.name} and the database differ from event {k} "
                                        f"({len(fl)} lines in the file, {len(dl)} in the database)")
            return dl

    def stores(self, kb) -> dict[str, Store]:
        """Fresh stores for this request, replayed from this backend's source."""
        out = {}
        for name in STORES:
            dl = self.reconcile(kb, name)
            if dl is None:
                out[name] = Store(self.dir, name, APPLY[name])
            else:
                m = self._mirror(kb)

                def writer(store, seq, line, ev, m=m):
                    if store in self.conflicts:
                        raise HttpError(409, self.conflicts[store] + "; writes are refused until they agree")
                    m.create(store, seq, line, ev)                 # the database first (unique store+seq) ...
                    _append_file(self.dir / f"{store}.jsonl", line)  # ... then the same line in the jsonl
                out[name] = LineStore(self.dir, name, dl, writer)
        return out

    # ---------------------------------------------------------------- corpus
    def vectors(self) -> Vectors:
        if self._vectors is None:
            self._vectors = Vectors(self.vec_dir) if self.vec_dir else Vectors()
        return self._vectors

    def _inputs(self, kb) -> dict:
        corpus = Path(kb.corpus)
        graph = Path(getattr(kb, "creative_graph", None) or kb.graph_path)
        idm = getattr(kb.ai, "id_map_path", None)
        if not idm and (graph.parent / "id_map.json").is_file():
            idm = graph.parent / "id_map.json"
        return {"graph": graph, "id_map": Path(idm) if idm else None, "kbdir": Path(kb.kbdir), "corpus": corpus}

    def _sig(self, inp: dict) -> tuple:
        paths = [inp["graph"], inp["id_map"], inp["kbdir"] / "inferred.json", inp["kbdir"] / "insights.json"]
        sets = inp["corpus"] / "sets"
        if sets.is_dir():
            for d in sorted(os.listdir(sets)):
                paths += [sets / d / "analysis.json", sets / d / "meta.json"]
        out = []
        for p in paths:
            try:
                st = p.stat()
                out.append((str(p), st.st_mtime_ns, st.st_size))
            except (OSError, TypeError, AttributeError):
                out.append((str(p), None))
        return tuple(out)

    def corpus(self, kb) -> Corpus:
        """creative.Corpus over the API's own inputs, with this backend's decisions (DB rows or decisions.jsonl).
        Cached until the backend's rev or an input file changes."""
        view = kb.full_view()
        inp = self._inputs(kb)
        key = (getattr(kb, "rev", None), len(view.decisions), self._sig(inp))
        hit = self._corpus.get(kb.backend)
        if hit and hit[0] == key:
            return hit[1]
        with self.lock:
            hit = self._corpus.get(kb.backend)
            if hit and hit[0] == key:
                return hit[1]
            self._corpus.pop(kb.backend, None)  # drop the old one before building (memory)
            c = build_corpus(inp, copy.deepcopy(view.decisions), self.vectors())
            self._corpus[kb.backend] = (key, c)
            return c

    # ---------------------------------------------------------------- jobs
    def enqueue(self, st: dict, kind: str, key: str, inp: dict, fresh: bool = False, kb=None,
                by: str = OWNER_ID) -> tuple[int, dict]:
        """(200, cached result) or (202, the queued job). Never runs the job here. `by`: who asked (his tap, or the
        import that queued a follow-up job)."""
        jobs = st["jobs"]
        live = [j for j in jobs.items.values() if j.get("key") == key]
        if not fresh:
            done = [j for j in live if j["status"] == "done"]
            if done:
                j = max(done, key=lambda x: x["done_at"] or "")
                return 200, {"cached": True, "job": job_brief(j), **(j["result"] or {})}
        pend = [j for j in live if j["status"] in ("queued", "claimed")]
        if pend:
            return 202, {"status": "queued", "job": job_brief(pend[0]), "poll": f"/api/jobs/{pend[0]['id']}"}
        jid = f"job:{kind}-{hashlib.sha1((key + now_iso() + str(len(jobs.items))).encode()).hexdigest()[:12]}"
        jobs.append("enqueue", jid, {"kind": kind, "key": key, "input": inp}, by=by)
        if self.jobs_mode == "inproc" and kb is not None and kind not in WORKER_ONLY:
            threading.Thread(target=self._run_inproc, args=(kb, jid), daemon=True).start()
        j = jobs.get(jid)
        return 202, {"status": "queued", "job": job_brief(j), "poll": f"/api/jobs/{jid}"}

    def _run_inproc(self, kb, jid: str):
        """Local dev only (PK_JOBS=inproc): the home worker's executor in a background thread of this API."""
        from worker import execute
        with self.lock:
            st = self.stores(kb)
            st["jobs"].append("claim", jid, {"worker": "inproc"}, by="worker:inproc")
            job = st["jobs"].get(jid)
        try:
            res, err = execute.run(job, inbox=Path(kb.inbox)), None
        except Exception as e:  # noqa: BLE001
            res, err = None, f"{type(e).__name__}: {e}"
        with self.lock:
            st = self.stores(kb)
            if err is None:
                st["jobs"].append("done", jid, {"result": res}, by="worker:inproc")
            else:
                st["jobs"].append("fail", jid, {"error": err}, by="worker:inproc")

    def claim(self, st: dict, kinds, worker: str) -> dict | None:
        now = time.time()
        jobs = st["jobs"]

        def claimable(j):
            if kinds and j["kind"] not in kinds:
                return False
            if j["status"] == "queued":
                return True
            if j["status"] == "claimed" and j["attempts"] < MAX_ATTEMPTS:
                return now - _ts(j["claimed_at"]) > STALE_CLAIM_S
            return False

        cand = sorted((j for j in jobs.items.values() if claimable(j)), key=lambda j: (j["created_at"], j["id"]))
        if not cand:
            return None
        jobs.append("claim", cand[0]["id"], {"worker": worker}, by=f"worker:{worker}")
        return jobs.get(cand[0]["id"])


def _ts(s: str | None) -> float:
    from datetime import datetime
    try:
        return datetime.fromisoformat(s).timestamp()
    except (TypeError, ValueError):
        return 0.0


def _no_model(*a, **k):
    raise RuntimeError("no model is called for a locked bit")


def job_brief(j: dict) -> dict:
    return {k: j.get(k) for k in ("id", "kind", "status", "created_at", "done_at", "attempts", "error")}


def build_corpus(inp: dict, decisions: list, vectors: Vectors) -> Corpus:
    """creative.data.Corpus.load, but over explicit paths (the API's graph, kb folder and corpus) and the
    backend's decisions."""
    g = json.load(open(inp["graph"], encoding="utf-8"))
    idm = json.load(open(inp["id_map"], encoding="utf-8")) if inp["id_map"] and inp["id_map"].is_file() else {}

    def opt(p: Path):
        return json.load(open(p, encoding="utf-8")) if p.is_file() else None

    analyses, metas = {}, {}
    sets = inp["corpus"] / "sets"
    if sets.is_dir():
        for d in sorted(os.listdir(sets)):
            ap = sets / d / "analysis.json"
            if ap.is_file():
                a = json.load(open(ap, encoding="utf-8"))
                key = a.get("set") or set_key_from_dir(d)
                analyses[key] = a
                m = opt(sets / d / "meta.json")
                if m:
                    metas[key] = m
    return Corpus(g, analyses, opt(inp["kbdir"] / "inferred.json"), opt(inp["kbdir"] / "insights.json"), decisions, idm,
                  metas, vectors)


# ====================================================================== helpers
def _joke(c: Corpus, jid) -> str:
    if not isinstance(jid, str) or not ID_RE.match(jid):
        raise HttpError(400, "bad joke id")
    n = c.nodes.get(jid)
    if not n or n.get("type") != "joke":
        raise HttpError(404, f"no such joke {jid}")
    return jid


def _board(st, bid: str) -> dict:
    b = st["storyboards"].get(bid)
    if not b or b.get("deleted"):
        raise HttpError(404, "no such storyboard")
    return b


def _ws(st, wid: str) -> dict:
    w = st["workingsets"].get(wid)
    if not w or w.get("deleted"):
        raise HttpError(404, "no such working set")
    return w


def _int(v, name, lo=0, hi=6 * 3600):
    if v is None:
        return None
    if isinstance(v, bool) or not isinstance(v, (int, float)) or not (lo <= v <= hi):
        raise HttpError(400, f"{name} must be a number between {lo} and {hi}")
    return int(v)


def board_summary(c: Corpus, b: dict) -> dict:
    run = sum(c.duration(j)[0] for j in b["joke_ids"])
    return {"id": b["id"], "title": b["title"], "n": len(b["joke_ids"]), "runtime_s": round(run, 1), "runtime": mmss(run),
            "target_s": b["target_s"], "updated": b["updated_at"], "source": b["source"]}


def latest_ws(st) -> dict | None:
    ws = [w for w in st["workingsets"].items.values() if not w.get("deleted")]
    return max(ws, key=lambda w: (w["updated_at"], w["id"])) if ws else None


def ws_detail(c: Corpus, ws: dict) -> dict:
    hist = W.history(c, ws)
    notes = W.try_tonight(c, ws, hist=hist)
    perf = W.perfected(c, ws, hist=hist)
    hb = {h["joke_id"]: h for h in hist}
    ver = W.current(ws)
    nights = [a for a in W.align_all(c, ws) if a["totals"]["bits_performed"]]
    last = {b["joke_id"]: b for b in (nights[-1]["bits"] if nights else [])}
    bits = []
    for b in ver["bits"]:
        j = b["joke_id"]
        lk = ws.get("locks", {}).get(j) or {}
        h = hb.get(j) or {}
        lb = last.get(j) or {}
        bits.append({**b, "locked": bool(lk.get("bit")), "locked_lines": lk.get("lines", []),
                     "nights": h.get("n_nights", 0), "best_night": h.get("best_night"), "trend": h.get("trend"),
                     "stability": h.get("stability"), "low_n": h.get("low_n", True),
                     "last_night": {k: lb.get(k) for k in ("performed", "time_s", "delta_s", "laugh_s_per_min", "vs_room",
                                                           "ai_rating")} if lb else None,
                     "notes": [n for n in notes if n["joke_id"] == j]})
    tries = W.with_his_notes(ws, notes)
    for t in tries:
        if t["source"] == "his" and t.get("joke_id"):
            t["title"] = c.title(t["joke_id"])
    return {"working_set": ws, "version": ver, "status": W.status(c, ws, notes), "perfected": perf,
            "try_tonight": tries, "bits": bits}


def venue_overlay(built: dict, edits: dict) -> dict:
    """His pin moves and renames on top of the geocoder's inferred pins."""
    for f in built["geojson"]["features"]:
        e = edits.get(f["properties"]["venue_id"])
        if not e:
            continue
        if e.get("lat") is not None:
            f["geometry"] = {"type": "Point", "coordinates": [e["lon"], e["lat"]]}
            f["properties"]["status"] = "his"
            f["properties"]["ambiguous"] = False
        if e.get("name"):
            f["properties"]["name"] = e["name"]
            f["properties"]["label"] = e["name"]
        f["properties"]["edited_at"] = e.get("updated_at")
    for v in built["venues"]:
        e = edits.get(v["venue_id"])
        if not e:
            continue
        g = v.get("geo") or {}
        if e.get("lat") is not None:
            v["geo"] = {**g, "lat": e["lat"], "lon": e["lon"], "status": "his", "ambiguous": False}
        if e.get("name"):
            v["name"] = e["name"]
        v["edited"] = {k: e.get(k) for k in ("lat", "lon", "name", "updated_at")}
    moved = {k for k, e in edits.items() if e.get("lat") is not None}
    built["ambiguous"] = [a for a in built["ambiguous"] if a["venue_id"] not in moved]
    return built


def find_line_node(c: Corpus, jid: str, line: str) -> str | None:
    """The set segment (newest first) or unit where `line` is verbatim, so a line lock can be checked."""
    for p in reversed(c.perfs.get(jid, [])):
        for sid in p["seg_ids"]:
            if line in (c.text_of(sid) or ""):
                return sid
    u = c.lead_unit(jid)
    if u and line in ((c.nodes.get(u) or {}).get("text") or ""):
        return u
    return None


# ====================================================================== routes
GET_ROUTES = [
    (re.compile(r"^/api/jokes/(.+)/connections$"), "connections"),
    (re.compile(r"^/api/boards$"), "boards"),
    (re.compile(r"^/api/boards/([^/]+)$"), "board"),
    (re.compile(r"^/api/workingsets$"), "workingsets"),
    (re.compile(r"^/api/workingsets/([^/]+)$"), "workingset"),
    (re.compile(r"^/api/workingsets/([^/]+)/nights$"), "nights"),
    (re.compile(r"^/api/workingsets/([^/]+)/bits/([^/]+)$"), "bit"),
    (re.compile(r"^/api/venues$"), "venues"),
    (re.compile(r"^/api/venues\.geojson$"), "geojson"),
    (re.compile(r"^/api/sets/([^/]+)/clips$"), "clips"),
    (re.compile(r"^/api/jobs/([^/]+)$"), "job"),
    (re.compile(r"^/api/settings$"), "settings"),
    (re.compile(r"^/api/connections$"), "connections_status"),
    # Simple Punchable (docs/SIMPLE.md): Write, the joke map, the set builder
    (re.compile(r"^/api/notebook/related$"), "notebook_related"),
    (re.compile(r"^/api/jokes/(.+)/map$"), "joke_map"),
    (re.compile(r"^/api/boards/([^/]+)/build$"), "board_build"),
]
POST_ROUTES = [
    (re.compile(r"^/api/boards$"), "board_create"),
    (re.compile(r"^/api/boards/([^/]+)/items$"), "board_item"),
    (re.compile(r"^/api/boards/([^/]+)/bridge$"), "bridge"),
    (re.compile(r"^/api/workingsets$"), "ws_create"),
    (re.compile(r"^/api/workingsets/([^/]+)/versions$"), "ws_version"),
    (re.compile(r"^/api/workingsets/([^/]+)/lock$"), "ws_lock"),
    (re.compile(r"^/api/workingsets/([^/]+)/bits/([^/]+)/punchup$"), "punchup"),
    (re.compile(r"^/api/ask$"), "ask"),
    (re.compile(r"^/api/jokes/(.+)/punchup$"), "joke_punchup"),
    (re.compile(r"^/api/workingsets/([^/]+)/try$"), "try_add"),
    (re.compile(r"^/api/import/youtube$"), "youtube"),
    (re.compile(r"^/api/facts$"), "facts"),
    (re.compile(r"^/api/riff$"), "riff"),
]
PATCH_ROUTES = [
    (re.compile(r"^/api/boards/([^/]+)$"), "board_patch"),
    (re.compile(r"^/api/venues/([^/]+)$"), "venue_patch"),
    (re.compile(r"^/api/settings$"), "settings_patch"),
    (re.compile(r"^/api/workingsets/([^/]+)/try/([^/]+)$"), "try_patch"),
]
WORKER_GET = [
    (re.compile(r"^/api/worker/decisions$"), "w_decisions"),
    (re.compile(r"^/api/worker/creative$"), "w_creative"),
    (re.compile(r"^/api/worker/jobs/([^/]+)/file$"), "w_file"),
]
WORKER_POST = [
    (re.compile(r"^/api/worker/claim$"), "w_claim"),
    (re.compile(r"^/api/worker/jobs/([^/]+)/result$"), "w_result"),
]
TABLES = {"GET": GET_ROUTES + WORKER_GET, "POST": POST_ROUTES + WORKER_POST, "PATCH": PATCH_ROUTES}


def match(method: str, path: str):
    for rx, name in TABLES.get(method, []):
        m = rx.match(path)
        if m:
            return name, [unquote(x) for x in m.groups()]
    return None


class Routes:
    """One request. `h` is the server's Handler (send_json / err / kb / headers / rfile)."""

    def __init__(self, cr: Creative, h, kb, q: dict, body: dict | None):
        self.cr, self.h, self.kb, self.q, self.b = cr, h, kb, q, body or {}

    def arg(self, k, d=None):
        return self.q.get(k, [d])[0]

    @property
    def c(self) -> Corpus:
        return self.cr.corpus(self.kb)

    def st(self):
        return self.cr.stores(self.kb)

    # ---------------------------------------------------------------- reads
    def connections(self, jid):
        c = self.c
        cap = self.arg("cap", "5")
        if not str(cap).isdigit() or not 1 <= int(cap) <= 20:
            raise HttpError(400, "cap must be 1-20")
        return 200, C.joke_connections(c, _joke(c, jid), cap=int(cap))

    # ---------------------------------------------------------------- Simple Punchable
    def notebook_related(self):
        """GET /api/notebook/related?q=<idea>&cap=10: his own bits about an idea (creative/notebook.related). On the
        SurrealDB backend the fused text+vector search adds its ranking; the files backend uses his-words search."""
        text = (self.arg("q") or "").strip()
        if not text or len(text) > 4000:
            raise HttpError(400, "q is required (at most 4000 characters)")
        cap = self.arg("cap", "10")
        if not str(cap).isdigit() or not 1 <= int(cap) <= 30:
            raise HttpError(400, "cap must be 1-30")
        c = self.c
        hybrid = None
        hs = getattr(self.kb, "hybrid_search", None)
        if hs is not None:
            try:
                hybrid = hs(text[:500], 20, False)
            except Exception:  # noqa: BLE001  search is a bonus; his-words search still answers
                hybrid = None
        ex = [x for x in (self.q.get("exclude") or []) if isinstance(x, str)]
        return 200, N.related(c, text, cap=int(cap), hybrid=hybrid, exclude=ex)

    def joke_map(self, jid):
        c = self.c
        cap = self.arg("cap", "8")
        if not str(cap).isdigit() or not 1 <= int(cap) <= 20:
            raise HttpError(400, "cap must be 1-20")
        return 200, N.joke_map(c, _joke(c, jid), cap=int(cap))

    def board_build(self, bid):
        c, st = self.c, self.st()
        b = _board(st, bid)
        out = N.build(c, b)
        ws = [w for w in st["workingsets"].items.values() if not w.get("deleted") and w.get("storyboard_id") == bid]
        out["working_set"] = max(ws, key=lambda w: w["updated_at"])["id"] if ws else None
        return 200, out

    def facts(self):
        text = self.b.get("text")
        if not isinstance(text, str) or not text.strip() or len(text) > 2000:
            raise HttpError(400, "text is required (at most 2000 characters)")
        key = "facts|" + hashlib.sha1(" ".join(text.lower().split()).encode()).hexdigest()[:16]
        return self.cr.enqueue(self.st(), "facts", key, {"text": text.strip()}, fresh=bool(self.b.get("fresh")), kb=self.kb)

    def riff(self):
        idea = self.b.get("idea")
        lines = self.b.get("lines") or []
        if not isinstance(idea, str) or not idea.strip() or len(idea) > 2000:
            raise HttpError(400, "idea is required (at most 2000 characters)")
        if not isinstance(lines, list) or len(lines) > 40 or not all(isinstance(x, str) and len(x) <= 2000 for x in lines):
            raise HttpError(400, "lines must be a list of text (at most 40)")
        inp = {"idea": idea.strip(), "lines": [x.strip() for x in lines if x.strip()]}
        key = "riff|" + hashlib.sha1(json.dumps(inp, sort_keys=True).lower().encode()).hexdigest()[:16]
        return self.cr.enqueue(self.st(), "riff", key, inp, fresh=bool(self.b.get("fresh")), kb=self.kb)

    def boards(self):
        c, st = self.c, self.st()
        bs = [b for b in st["storyboards"].items.values() if not b.get("deleted")]
        bs.sort(key=lambda b: (b["updated_at"], b["id"]), reverse=True)
        return 200, {"boards": [board_summary(c, b) for b in bs]}

    def board(self, bid):
        st = self.st()
        return 200, S.board_view(self.c, _board(st, bid))

    def workingsets(self):
        st = self.st()
        ws = [w for w in st["workingsets"].items.values() if not w.get("deleted")]
        ws.sort(key=lambda w: (w["updated_at"], w["id"]), reverse=True)
        items = [{"id": w["id"], "title": w["title"], "version": w["current"],
                  "target_s": W.current(w)["target_s"], "n": len(W.current(w)["bits"]),
                  "storyboard_id": w.get("storyboard_id"), "updated": w["updated_at"],
                  "source": w.get("source")} for w in ws]
        # `workingsets` is the key; `working_sets` (creative/'s CLI spelling) is an alias of the same list
        return 200, {"workingsets": items, "working_sets": items}

    def workingset(self, wid):
        st = self.st()
        return 200, ws_detail(self.c, _ws(st, wid))

    def nights(self, wid):
        st = self.st()
        return 200, {"working_set": wid, "nights": W.align_all(self.c, _ws(st, wid))}

    def bit(self, wid, jid):
        st = self.st()
        ws = _ws(st, wid)
        c = self.c
        _joke(c, jid)
        lk = ws.get("locks", {}).get(jid) or {}
        return 200, {**W.bit_history(c, jid), "working_set": wid, "locked": bool(lk.get("bit")),
                     "locked_lines": lk.get("lines", [])}

    def _venues(self):
        c, st = self.c, self.st()
        geo = V.Geocoder(self.cr.dir / "geocode_cache.json")
        built = V.build(c, geo, offline=not self.cr.geocode)
        return venue_overlay(built, st["venues"].items)

    def venues(self):
        v = self._venues()
        return 200, {k: v[k] for k in ("generated_at", "venues", "ambiguous", "geocoder")}

    def geojson(self):
        return 200, self._venues()["geojson"]

    def clips(self, sid):
        v = self.kb.view()
        s = v.nodes.get(sid)
        if not s or s.get("type") != "set":
            raise HttpError(404, "no such set")
        d = self.kb.set_dir(sid)
        p = Path(self.kb.corpus) / "sets" / d / "clips.json" if d else None
        if not p or not p.is_file():
            return 200, {"set": sid, "clips": [], "source": None}
        return 200, json.loads(p.read_text(encoding="utf-8"))

    def job(self, jid):
        j = self.st()["jobs"].get(jid)
        if not j:
            raise HttpError(404, "no such job")
        return 200, {**job_brief(j), "result": j.get("result")}

    # ---------------------------------------------------------------- writes (his actions)
    def _jokes_list(self, c, xs) -> list[str]:
        if not isinstance(xs, list) or len(xs) > 200:
            raise HttpError(400, "joke_ids must be a list (at most 200)")
        out = [_joke(c, j) for j in xs]
        if len(set(out)) != len(out):
            raise HttpError(400, "joke_ids has duplicates")
        return out

    def board_create(self):
        c, st = self.c, self.st()
        title = self.b.get("title")
        if not isinstance(title, str) or not title.strip() or len(title) > 200:
            raise HttpError(400, "title is required (1-200 characters)")
        notes = self.b.get("notes") or ""
        if not isinstance(notes, str) or len(notes) > 20000:
            raise HttpError(400, "notes must be text")
        b = S.create(st["storyboards"], title.strip(), self._jokes_list(c, self.b.get("joke_ids") or []),
                     _int(self.b.get("target_s"), "target_s", 30) or 900, notes)
        return 201, S.board_view(c, b)

    def board_patch(self, bid):
        c, st = self.c, self.st()
        store = st["storyboards"]
        b = _board(st, bid)
        ops = []
        upd = {}
        if "title" in self.b:
            t = self.b["title"]
            if not isinstance(t, str) or not t.strip() or len(t) > 200:
                raise HttpError(400, "title must be 1-200 characters")
            upd["title"] = t.strip()
        if "notes" in self.b:
            if not isinstance(self.b["notes"], str) or len(self.b["notes"]) > 20000:
                raise HttpError(400, "notes must be text")
            upd["notes"] = self.b["notes"]
        if "target_s" in self.b:
            upd["target_s"] = _int(self.b["target_s"], "target_s", 30)
        if upd:
            ops.append(("update", upd))
        if "joke_ids" in self.b:  # reorder / replace in one go
            ops.append(("set_jokes", {"joke_ids": self._jokes_list(c, self.b["joke_ids"])}))
        if "add" in self.b:
            a = self.b["add"] if isinstance(self.b["add"], dict) else {"joke_id": self.b["add"]}
            ops.append(("add", {"joke_id": _joke(c, a.get("joke_id")), "at": _int(a.get("at"), "at", 0, 200)}))
        if "remove" in self.b:
            r = self.b["remove"]
            ops.append(("remove", {"joke_id": _joke(c, r.get("joke_id") if isinstance(r, dict) else r)}))
        if "move" in self.b:
            mv = self.b["move"] if isinstance(self.b["move"], dict) else {}
            ops.append(("move", {"from": _int(mv.get("from"), "move.from", 0, 200), "to": _int(mv.get("to"), "move.to", 0, 200)}))
        if not ops:
            raise HttpError(400, "nothing to change (title, notes, target_s, joke_ids, add, remove, move)")
        probe = json.loads(json.dumps(store.items))  # all or nothing: every op is checked before any is written
        for op, data in ops:
            try:
                S._apply(probe, {"ts": now_iso(), "op": op, "id": b["id"], "by": OWNER_ID, "data": data})
            except (ValueError, KeyError) as e:
                raise HttpError(400, str(e))
        for op, data in ops:
            store.append(op, b["id"], data)
        return 200, S.board_view(c, store.get(b["id"]))

    def board_item(self, bid):
        c, st = self.c, self.st()
        b = _board(st, bid)
        jid = _joke(c, self.b.get("joke_id"))
        after = self.b.get("after")
        at = None
        if isinstance(after, str):
            if after not in b["joke_ids"]:
                raise HttpError(400, f"{after} is not on the board")
            at = b["joke_ids"].index(after) + 1
        elif after is not None:
            at = _int(after, "after", 0, 200) + 1
        try:
            S.add(st["storyboards"], b["id"], jid, at)
        except ValueError as e:
            raise HttpError(409 if "already" in str(e) else 400, str(e))
        return 200, S.board_view(c, st["storyboards"].get(b["id"]))

    def ws_create(self):
        c, st = self.c, self.st()
        b = _board(st, self.b.get("board_id") if isinstance(self.b.get("board_id"), str) else "")
        if not b["joke_ids"]:
            raise HttpError(400, "the storyboard has no bits")
        ws = W.promote(c, st["workingsets"], b, target_s=_int(self.b.get("target_s"), "target_s", 30))
        return 201, ws_detail(c, ws)

    def ws_version(self, wid):
        c, st = self.c, self.st()
        ws = _ws(st, wid)
        note = self.b.get("note")
        if not isinstance(note, str) or not note.strip() or len(note) > 2000:
            raise HttpError(400, "a new version needs a note saying what changed")
        bits = None
        if "joke_ids" in self.b:
            ids = self._jokes_list(c, self.b["joke_ids"])
            if not ids:
                raise HttpError(400, "joke_ids is empty")
            old = {x["joke_id"]: x for x in W.current(ws)["bits"]}
            bits = []
            for j in ids:
                if j in old:
                    bits.append(old[j])
                else:
                    d, basis = c.duration(j)
                    bits.append({"joke_id": j, "title": c.title(j), "target_s": int(round(d / 5.0) * 5), "target_basis": basis})
        ws = W.new_version(st["workingsets"], ws["id"], note.strip(), bits=bits,
                           target_s=_int(self.b.get("target_s"), "target_s", 30))
        return 201, ws_detail(c, ws)

    def ws_lock(self, wid):
        c, st = self.c, self.st()
        ws = _ws(st, wid)
        jid = _joke(c, self.b.get("joke_id"))
        if jid not in {b["joke_id"] for b in W.current(ws)["bits"]} and jid not in ws.get("locks", {}):
            raise HttpError(400, f"{jid} is not in this working set")
        unlock = bool(self.b.get("unlock"))
        line = self.b.get("line")
        store = st["workingsets"]
        if "line" in self.b and not isinstance(line, str):
            raise HttpError(400, "line must be text (leave it out to lock the whole bit)")
        if line is None:
            ws = (W.unlock_bit if unlock else W.lock_bit)(store, ws["id"], jid)
        else:
            if not isinstance(line, str) or not line.strip() or len(line) > 2000:
                raise HttpError(400, "line must be text")
            if unlock:
                ws = W.unlock_line(store, ws["id"], jid, line)
            else:
                node = self.b.get("node") or find_line_node(c, jid, line)
                if not node:
                    raise HttpError(400, "that line is not verbatim in any taped night or written version of the bit")
                try:
                    ws = W.lock_line(c, store, ws["id"], jid, line, node)
                except ValueError as e:
                    raise HttpError(400, str(e))
        return 200, ws_detail(c, ws)

    def venue_patch(self, vid):
        c, st = self.c, self.st()
        if not any(v["venue_id"] == vid for v in V.venue_stats(c)):
            raise HttpError(404, "no such venue")
        store = st["venues"]
        ops = []
        if "lat" in self.b or "lon" in self.b:
            ops.append(("pin", {"lat": self.b.get("lat"), "lon": self.b.get("lon")}))
        if "name" in self.b:
            ops.append(("rename", {"name": self.b.get("name")}))
        if self.b.get("reset"):
            ops = [("reset", {})]
        if not ops:
            raise HttpError(400, "send lat+lon, name, or reset")
        probe = json.loads(json.dumps(store.items))
        for op, data in ops:
            try:
                venue_apply(probe, {"ts": now_iso(), "op": op, "id": vid, "by": OWNER_ID, "data": data})
            except ValueError as e:
                raise HttpError(400, str(e))
        for op, data in ops:
            store.append(op, vid, data)
        v = self._venues()
        return 200, {"venue": next(x for x in v["venues"] if x["venue_id"] == vid),
                     "feature": next(f for f in v["geojson"]["features"] if f["properties"]["venue_id"] == vid)}

    # ---------------------------------------------------------------- drafts: cached or queued, never blocking
    def bridge(self, bid):
        c, st = self.c, self.st()
        _board(st, bid)
        a, b = _joke(c, self.b.get("from")), _joke(c, self.b.get("to"))
        return self.cr.enqueue(st, "bridge", f"bridge|{a}|{b}", {"from": a, "to": b, "board": bid},
                               fresh=bool(self.b.get("fresh")), kb=self.kb)

    def punchup(self, wid, jid):
        c, st = self.c, self.st()
        ws = _ws(st, wid)
        _joke(c, jid)
        if W.bit_locked(ws, jid):  # the lock rule, answered here without queueing anything
            return 200, W.punch_up(c, ws, jid, complete=_no_model)
        locks = json.dumps(ws.get("locks", {}).get(jid) or {}, sort_keys=True)
        key = f"punchup|{wid}|{jid}|v{ws['current']}|{hashlib.sha1(locks.encode()).hexdigest()[:10]}"
        return self.cr.enqueue(st, "punchup", key, {"working_set": ws, "joke_id": jid}, fresh=bool(self.b.get("fresh")), kb=self.kb)

    def ask(self):
        qn = self.b.get("question")
        if not isinstance(qn, str) or not qn.strip() or len(qn) > 1000:
            raise HttpError(400, "question is required (at most 1000 characters)")
        st = self.st()
        key = "ask|" + hashlib.sha1(" ".join(qn.lower().split()).encode()).hexdigest()[:16]
        return self.cr.enqueue(st, "ask", key, {"question": qn.strip()}, fresh=bool(self.b.get("fresh")), kb=self.kb)

    def joke_punchup(self, jid):
        """Brainstorm drafts for any bit, not only a working-set bit. His locks on that bit in ANY working set hold:
        a locked bit answers `locked` with no job; locked lines never reach the model (W.punch_up drops them)."""
        c, st = self.c, self.st()
        _joke(c, jid)
        lk = {"bit": False, "lines": []}
        for w in sorted(st["workingsets"].items.values(), key=lambda w: w["id"]):
            x = (w.get("locks") or {}).get(jid) if not w.get("deleted") else None
            if x:
                lk["bit"] = lk["bit"] or bool(x.get("bit"))
                have = {y["text"] for y in lk["lines"]}
                lk["lines"] += [ln for ln in x.get("lines", []) if ln["text"] not in have]
        ws = {"id": None, "title": None, "current": 0, "locks": {jid: lk}}
        if lk["bit"] or not c.perfs.get(jid):  # locked, or no tape: answered here, no model, no job
            return 200, W.punch_up(c, ws, jid, complete=_no_model)
        key = f"punchup|joke|{jid}|{hashlib.sha1(json.dumps(lk, sort_keys=True).encode()).hexdigest()[:10]}"
        return self.cr.enqueue(st, "punchup", key, {"working_set": ws, "joke_id": jid}, fresh=bool(self.b.get("fresh")),
                               kb=self.kb)

    def try_add(self, wid):
        """His own try-tonight note {joke_id, text}. joke_id may be null (a note for the whole set)."""
        c, st = self.c, self.st()
        ws = _ws(st, wid)
        jid = self.b.get("joke_id")
        if jid is not None:
            _joke(c, jid)
            if jid not in {b["joke_id"] for b in W.current(ws)["bits"]}:
                raise HttpError(400, f"{jid} is not in this working set")
        text = self.b.get("text")
        if not isinstance(text, str) or not text.strip() or len(text) > 500:
            raise HttpError(400, "text is required (up to 500 characters)")
        ws, nid = W.add_try(st["workingsets"], ws["id"], jid, text.strip())
        note = next(n for n in ws["try_notes"] if n["id"] == nid)
        return 201, {"working_set": ws["id"], "note": {**note, "title": c.title(jid) if jid else None, "source": "his"}}

    def try_patch(self, wid, nid):
        """Tick (or untick) a try note: one of his (`try:...`) or one of the AI's (`<joke_id>|<kind>`)."""
        st = self.st()
        ws = _ws(st, wid)
        if not isinstance(self.b.get("done"), bool):
            raise HttpError(400, "done must be true or false")
        done = self.b["done"]
        mine = next((n for n in ws.get("try_notes", []) if n["id"] == nid), None)
        if mine is None and not AI_NOTE_RE.match(nid):
            raise HttpError(404, "no such try note")
        was = mine["done"] if mine else bool((ws.get("try_done") or {}).get(nid))
        if was != done:
            ws = W.tick_try(st["workingsets"], ws["id"], nid, done)
        mine = next((n for n in ws.get("try_notes", []) if n["id"] == nid), None)
        return 200, {"working_set": ws["id"], "changed": was != done,
                     "note": {**mine, "source": "his"} if mine else {"id": nid, "done": done, "source": "inferred"}}

    def youtube(self):
        """Import a set from a YouTube link: queued for the home worker (worker/execute.py runs
        tools/ingest/fetch_youtube.py, show_dates.py and tools/audio/run_all.py there). Nothing downloads here."""
        url = self.b.get("url")
        if not isinstance(url, str) or len(url) > 500:
            raise HttpError(400, "url is required")
        m = YT_RE.match(url.strip())
        if not m:
            raise HttpError(400, "that is not a YouTube video link (youtube.com/watch?v=..., youtu.be/..., /shorts/...)")
        vid = m.group(1)
        have = next((n["id"] for n in self.kb.view().nodes.values()
                     if n.get("type") == "set" and str(n["id"]).endswith("_" + vid)), None)
        if have:
            return 200, {"status": "exists", "set": have, "video_id": vid}
        return self.cr.enqueue(self.st(), "import_youtube", f"youtube|{vid}",
                               {"url": f"https://www.youtube.com/watch?v={vid}", "video_id": vid}, kb=self.kb)

    def settings(self, st=None):
        it = (st or self.st())["account"].get("settings") or {}
        return 200, {**SETTINGS_DEFAULTS, **{k: it[k] for k in SETTINGS_DEFAULTS if k in it},
                     "updated_at": it.get("updated_at"), "source": "his" if it else "default"}

    def connections_status(self):
        """Settings > Connections: one plain status per outside source. Read-only."""
        w = self.cr.worker_seen
        online = bool(w) and time.time() - w["t"] < WORKER_ONLINE_S
        nodes = self.kb.view().nodes
        yt = sum(1 for k, n in nodes.items() if n.get("type") == "set" and re.search(r"_[A-Za-z0-9_-]{11}$", k))
        hosted = os.environ.get("PK_AUTH") == "1"
        return 200, {
            "google_docs": gsync_status(self.cr.gsync_dir, hosted),
            "youtube": {"sets": yt, "import": "link"},
            "ai": {"provider": self.settings()[1]["provider"],
                   "worker": {"online": online, "last_seen": w["at"] if w else None, "name": w["name"] if w else None,
                              "online_window_s": WORKER_ONLINE_S}},
            "audio_storage": {"status": "not_set_up", "provider": "Cloudflare R2"},
        }

    def settings_patch(self):
        st = self.st()
        try:
            d = check_settings(dict(self.b))
        except ValueError as e:
            raise HttpError(400, str(e))
        cur = st["account"].get("settings") or SETTINGS_DEFAULTS
        if any(cur.get(k) != v for k, v in d.items()):
            st["account"].append("settings", "settings", d)
        return self.settings(st)

    # ---------------------------------------------------------------- the home worker
    def w_claim(self):
        kinds = self.b.get("kinds") or None
        if kinds is not None and (not isinstance(kinds, list) or not set(kinds) <= set(JOB_KINDS)):
            raise HttpError(400, f"kinds must be a subset of {JOB_KINDS}")
        name = str(self.b.get("worker") or "home")[:40]
        self.cr.worker_seen = {"t": time.time(), "at": now_iso(), "name": name}
        with self.cr.lock:
            j = self.cr.claim(self.st(), kinds, name)
        return (200, {"job": j}) if j else (204, None)

    def w_result(self, jid):
        st = self.st()
        j = st["jobs"].get(jid)
        if not j:
            raise HttpError(404, "no such job")
        if j["status"] != "claimed":
            raise HttpError(409, f"job is {j['status']}")
        res = self.b.get("result")
        if self.b.get("ok"):
            if not isinstance(res, dict):
                raise HttpError(400, "result must be an object")
            if len(json.dumps(res)) > 2_000_000:
                raise HttpError(413, "result too large")
            st["jobs"].append("done", jid, {"result": res}, by="worker")
            if j["kind"] == "process_set":
                f = self._inbox_file(j)
                if f:
                    f.unlink(missing_ok=True)
        else:
            st["jobs"].append("fail", jid, {"error": str(self.b.get("error") or "failed")[:2000],
                                            "retry": bool(self.b.get("retry"))}, by="worker")
        return 200, {"ok": True, "job": job_brief(st["jobs"].get(jid))}

    def _inbox_file(self, j) -> Path | None:
        name = (j.get("input") or {}).get("file") or ""
        if not re.match(r"^[A-Za-z0-9._\-]{1,200}$", name):
            return None
        p = Path(self.kb.inbox) / name
        return p if p.is_file() else None

    def w_file(self, jid):
        j = self.st()["jobs"].get(jid)
        if not j or j["kind"] != "process_set":
            raise HttpError(404, "no such upload job")
        p = self._inbox_file(j)
        if not p:
            raise HttpError(404, "the upload is gone (the pod restarted?)")
        return "file", p

    def w_decisions(self):
        v = self.kb.full_view()
        after = max(0, int(self.arg("after", "0")) if str(self.arg("after", "0")).isdigit() else 0)
        lines = []
        for i, d in enumerate(v.decisions[after:after + 2000], after):
            line = json.dumps(d, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
            lines.append({"index": i, "decision": d, "h": _h(line)})
        return 200, {"total": len(v.decisions), "after": after, "lines": lines}

    def w_creative(self):
        name = self.arg("store")
        if name not in ("storyboards", "workingsets", "venues", "merges", "suggestions"):
            raise HttpError(400, "store must be storyboards | workingsets | venues | merges | suggestions")
        if name == "merges":
            import merge_api  # noqa: F401  registers the store's apply function
        if name == "suggestions":
            import suggest_api  # noqa: F401
        dl = self.cr.reconcile(self.kb, name)
        lines = dl if dl is not None else file_lines(self.cr.dir / f"{name}.jsonl")
        out = {"store": name, "total": len(lines), "lines": lines}
        if self.arg("snapshot"):  # the replayed items, for readers without creative/ (gsync's venv has no numpy)
            if name == "merges":
                import merge_api as MGA
                out["items"] = MGA.store(self.cr, self.kb).items
            elif name == "suggestions":
                import suggest_api as SGA
                out["items"] = SGA.store(self.cr, self.kb).items
            else:
                out["items"] = self.cr.stores(self.kb)[name].items
        return 200, out


def handle(cr: Creative, h, method: str, path: str, q: dict, body_fn) -> bool:
    """Serve a creative or worker route. Returns False when the path is not one of ours."""
    m = match(method, path)
    if not m:
        return False
    name, args = m
    try:
        body = body_fn() if method in ("POST", "PATCH") else None
        with cr.lock if method != "GET" else _nolock():
            status, out = getattr(Routes(cr, h, h.kb, q, body), name)(*args)
        if status == "file":
            h.send_file(out)
        elif status == 204:
            h.send_response(204)
            h.send_header("Content-Length", "0")
            h.end_headers()
        else:
            h.send_json(out, status)
    except HttpError as e:
        h.err(e.status, str(e))
    return True


class _nolock:
    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False


def merged_today(old: dict, c: Corpus, ws: dict | None) -> dict:
    """GET /api/today: today.build (the contract) merged into the existing Today payload, so the current screen
    keeps working while the new one is built. `latest_set` is the existing SetCard plus today.build's set fields
    (the SetCard's own keys win where both have one: title, date, venue)."""
    t = T.build(c, ws)
    out = dict(old)
    for k, v in t.items():
        if k == "latest_set":
            out["latest_set"] = {**(v or {}), **(old.get("latest_set") or {})} if (v or old.get("latest_set")) else None
        else:
            out[k] = v
    return out
