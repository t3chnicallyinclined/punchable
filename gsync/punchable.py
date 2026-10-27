"""Punchable: writes go through the API (POST /api/ideas, /api/decision, /api/undo) so they land as normal decision
lines in decisions.jsonl and SurrealDB, each stamped with its actor: `import:gsync` for Google Docs, `import:memo`
for voice memos (never the comic's own id: these are imports, not his taps). Reads that the local API has no
endpoint for use the documented files, read-only: decisions.jsonl, kb/v1/graph.json, kb/creative/storyboards.json,
kb/creative/merges.json.

HostedPunchable (online-first, PK_ONLINE_FIRST=1): the same calls against the hosted API's worker routes
(/api/worker/ideas, /decision, /undo, /jokes/<id>, /decisions, /creative, /files) with the worker's Keycloak token;
nothing is read from local files, and gsync's own state (kb/gsync/*.json) is posted back as srcfile rows."""
from __future__ import annotations

import json
import os
import re
import urllib.error
import urllib.parse
import urllib.request

from .config import IDEA_MAX, Config

GDOCS = "[gdocs]"  # reason prefix on every decision gsync writes: decision lines have no `source` field


class ApiDown(RuntimeError):
    pass


class ApiError(RuntimeError):
    def __init__(self, status, msg):
        super().__init__(f"{status}: {msg}")
        self.status = status


class Punchable:
    def __init__(self, cfg: Config, actor: str = "import:gsync"):
        self.cfg = cfg
        self.calls = 0
        self.actor = actor

    # ---- http ---------------------------------------------------------------------------------------------------
    def _req(self, method: str, path: str, body: dict | None = None, timeout: float = 30):
        self.calls += 1
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(self.cfg.api + path, data=data, method=method,
                                     headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.loads(r.read().decode("utf-8"))
        except urllib.error.HTTPError as e:
            try:
                msg = json.loads(e.read().decode("utf-8")).get("error")
            except Exception:  # noqa: BLE001
                msg = str(e)
            raise ApiError(e.code, msg) from None
        except (urllib.error.URLError, ConnectionError, TimeoutError) as e:
            raise ApiDown(str(e)) from None

    def health(self) -> bool:
        try:
            return bool(self._req("GET", "/api/health", timeout=5).get("ok"))
        except (ApiDown, ApiError):
            return False

    # ---- writes -------------------------------------------------------------------------------------------------
    def add_idea(self, text: str, title: str | None, frm: dict, reason: str) -> dict:
        if len(text) <= IDEA_MAX:
            r = self._req("POST", "/api/ideas", {"text": text, "title": title or "", "from": frm, "reason": reason,
                                                 "actor": self.actor})
            return {"id": r["id"], "indexes": r["indexes"]}
        t = (title or " ".join(text.split()[:7]))[:120] or "New line"
        r = self._req("POST", "/api/decision", {"target": "node", "action": "add", "reason": reason, "actor": self.actor,
                                                "after": {"type": "joke", "title": t, "stage": "idea", "summary": text}})
        return {"id": r["decisions"][0]["id"], "indexes": r["indexes"]}

    def edit_joke(self, jid: str, after: dict, reason: str) -> dict | None:
        try:
            r = self._req("POST", "/api/decision", {"target": "node", "action": "edit", "id": jid, "after": after,
                                                    "reason": reason, "actor": self.actor})
        except ApiError as e:
            if e.status == 400 and "nothing changed" in str(e):
                return None
            raise
        return {"indexes": r["indexes"]}

    def undo(self, indexes: list[int]) -> dict:
        return self._req("POST", "/api/undo", {"indexes": indexes, "actor": self.actor})

    def push_state(self, paths: list | None = None, force_runs: bool = False) -> dict | None:
        """Local mode: gsync's state stays in the data folder (the files are the truth until the cutover)."""
        return None

    def pull_state(self) -> list[str]:
        return []

    # ---- reads --------------------------------------------------------------------------------------------------
    def joke(self, jid: str) -> dict | None:
        try:
            d = self._req("GET", "/api/jokes/" + urllib.parse.quote(jid, safe=":"))
        except ApiError as e:
            if e.status == 404:
                return None
            raise
        j = d.get("joke") or {}
        lead, lead_text = j.get("lead_unit"), None
        for v in d.get("versions") or []:
            for u in v.get("units") or []:
                if lead and u.get("id") == lead:
                    lead_text = u.get("text")
        return {"id": jid, "title": j.get("title"), "summary": j.get("summary") or "", "stage": j.get("stage"),
                "lead_text": lead_text, "yours": bool(d.get("yours")), "hidden": bool(j.get("hidden"))}

    def decisions(self) -> list[dict]:
        p = self.cfg.decisions
        if not p.exists():
            return []
        out = []
        for ln in p.read_text(encoding="utf-8").splitlines():
            try:
                out.append(json.loads(ln))
            except ValueError:
                out.append({})
        return out

    def app_jokes(self) -> list[str]:
        """Jokes the comic created in the app: live node adds of type joke, not written by gsync."""
        ds = self.decisions()
        undone = {d["undo_of"] for d in ds if isinstance(d.get("undo_of"), int)}
        alive: dict[str, bool] = {}
        for i, d in enumerate(ds):
            if d.get("target") != "node" or i in undone or "undo_of" in d:
                continue
            if d.get("action") == "add" and (d.get("after") or {}).get("type") == "joke" \
                    and not (d.get("reason") or "").startswith(GDOCS):
                alive[d["id"]] = True
            elif d.get("action") == "delete" and d.get("id") in alive:
                alive[d["id"]] = False
        return [k for k, v in alive.items() if v]

    def boards(self) -> dict:
        p = self.cfg.storyboards
        if not p.exists():
            return {}
        items = json.loads(p.read_text(encoding="utf-8")).get("items") or {}
        return items if isinstance(items, dict) else {b["id"]: b for b in items}

    def merges(self) -> dict:
        """the comic's merges (and AI attaches) from the API's merge store snapshot, read-only."""
        from .merge import merges_from
        return merges_from(self.cfg.merges)

    def doc_jokes(self) -> dict[str, list[str]]:
        """gdoc_id -> joke ids, from the KB graph's units and part_of edges (most units first)."""
        p = self.cfg.graph
        if not p.exists():
            return {}
        g = json.loads(p.read_text(encoding="utf-8"))
        udoc = {n["id"]: n.get("doc_id") for n in g.get("nodes", []) if n.get("type") == "unit" and n.get("doc_id")}
        count: dict[str, dict[str, int]] = {}
        for e in g.get("edges", []):
            if e.get("kind") == "part_of" and e.get("src") in udoc and e.get("status") not in ("rejected", "ruled_out"):
                c = count.setdefault(udoc[e["src"]], {})
                c[e["dst"]] = c.get(e["dst"], 0) + 1
        return {d: sorted(c, key=lambda j: (-c[j], j)) for d, c in count.items()}


# ====================================================================================================== online-first
ROUTES = {"/api/ideas": "/api/worker/ideas", "/api/decision": "/api/worker/decision", "/api/undo": "/api/worker/undo"}
STATE_FILES = ("map.json", "memos.json", "units.json", "edits.jsonl", "conflicts.jsonl")
RUNS_EVERY_S = 600   # runs.jsonl grows every minute: posted at most every 10 minutes, or when a run did something


class HostedPunchable(Punchable):
    """Punchable against the hosted API's worker routes (worker/__main__.Client: Keycloak client credentials)."""

    def __init__(self, cfg: Config, client, actor: str = "import:gsync"):
        super().__init__(cfg, actor)
        self.cl = client
        self._graph = (None, None)
        self._pushed: dict[str, str] = {}
        self._runs_at = 0.0

    def _req(self, method: str, path: str, body: dict | None = None, timeout: float = 30):
        self.calls += 1
        if path.startswith("/api/jokes/"):
            path = "/api/worker/jokes/" + path[len("/api/jokes/"):]
        path = ROUTES.get(path, path)
        try:
            return self.cl.call(method, path, body)
        except RuntimeError as e:
            m = re.search(r"HTTP (\d{3}) (.*)$", str(e), re.S)
            if m:
                msg = m.group(2)
                try:
                    msg = json.loads(msg).get("error") or msg
                except ValueError:
                    pass
                raise ApiError(int(m.group(1)), msg) from None
            raise ApiDown(str(e)) from None
        except (urllib.error.URLError, ConnectionError, TimeoutError, OSError) as e:
            raise ApiDown(str(e)) from None

    def health(self) -> bool:
        try:
            r = self.cl.call("GET", "/api/health")
            return bool(r.get("ok") or r.get("loaded"))
        except Exception:  # noqa: BLE001
            return False

    def decisions(self) -> list[dict]:
        out, after = [], 0
        while True:
            page = self._req("GET", f"/api/worker/decisions?after={after}")
            out += [x["decision"] for x in page["lines"]]
            after += len(page["lines"])
            if not page["lines"] or after >= page["total"]:
                return out

    def _snapshot(self, store: str) -> dict:
        """The store's replayed items, as the API replays them (gsync's venv has no creative/ dependencies)."""
        items = self._req("GET", f"/api/worker/creative?store={store}&snapshot=1").get("items") or {}
        return items if isinstance(items, dict) else {}

    def boards(self) -> dict:
        return self._snapshot("storyboards")

    def merges(self) -> dict:
        return self._snapshot("merges")

    def doc_jokes(self) -> dict[str, list[str]]:
        files = {f["path"]: f["sha256"] for f in self._req("GET", "/api/worker/files?prefix=kb/")["files"]}
        rel = "kb/v1/graph.json" if "kb/v1/graph.json" in files else "kb/v0/graph.json"
        if rel not in files:
            return {}
        if self._graph[0] != files[rel]:
            d = self._req("GET", "/api/worker/files/content?path=" + urllib.parse.quote(rel))
            self._graph = (files[rel], json.loads(d["text"]))
        g = self._graph[1]
        udoc = {n["id"]: n.get("doc_id") for n in g.get("nodes", []) if n.get("type") == "unit" and n.get("doc_id")}
        count: dict[str, dict[str, int]] = {}
        for e in g.get("edges", []):
            if e.get("kind") == "part_of" and e.get("src") in udoc and e.get("status") not in ("rejected", "ruled_out"):
                c = count.setdefault(udoc[e["src"]], {})
                c[e["dst"]] = c.get(e["dst"], 0) + 1
        return {d: sorted(c, key=lambda j: (-c[j], j)) for d, c in count.items()}

    def pull_state(self) -> list[str]:
        """A fresh PC: gsync's state files that are in the database but not here are downloaded (never overwritten:
        this PC is their only writer)."""
        from .store import write_atomic
        got = []
        for f in self._req("GET", "/api/worker/files?prefix=kb/gsync/")["files"]:
            p = self.cfg.comedy / f["path"]
            if not p.exists():
                d = self._req("GET", "/api/worker/files/content?path=" + urllib.parse.quote(f["path"]))
                write_atomic(p, d["text"])
                got.append(f["path"])
            self._pushed[f["path"]] = f["sha256"]
        return got

    def push_state(self, paths: list | None = None, force_runs: bool = False) -> dict | None:
        """gsync's own state (kb/gsync/*) -> the database, when it changed. `paths`: extra data-folder files to post
        too (a memo's transcript.json and meta.json)."""
        import hashlib
        import time
        rels = [f"kb/gsync/{n}" for n in STATE_FILES]
        if force_runs or time.time() - self._runs_at > RUNS_EVERY_S:
            rels.append("kb/gsync/runs.jsonl")
        rels += list(paths or [])
        files = []
        for rel in rels:
            p = self.cfg.comedy / rel
            if not p.is_file():
                continue
            text = p.read_text(encoding="utf-8")
            h = hashlib.sha256(text.encode("utf-8")).hexdigest()
            if self._pushed.get(rel) == h:
                continue
            files.append({"path": rel, "text": text, "sha256": h})
        if not files:
            return {"written": []}
        r = self._req("POST", "/api/worker/files", {"files": files, "actor": self.actor})
        for f in files:
            self._pushed[f["path"]] = f["sha256"]
        if "kb/gsync/runs.jsonl" in rels:
            self._runs_at = time.time()
        return r


def make(cfg: Config, actor: str = "import:gsync") -> Punchable:
    """The client for this PC's mode: hosted worker routes when online-first (PK_ONLINE_FIRST=1), else the local API."""
    if os.environ.get("PK_ONLINE_FIRST") == "1":
        import sys
        from pathlib import Path
        sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
        import worker.__main__ as W
        s = W.secrets()
        cl = W.Client(s.get("PK_API", W.DEFAULT_API), auth=True, token_url=s.get("PK_TOKEN_URL", W.DEFAULT_TOKEN_URL),
                      client_id=s.get("PK_WORKER_CLIENT_ID", "pk-worker"), secret=s.get("PK_WORKER_SECRET"))
        return HostedPunchable(cfg, cl, actor)
    return Punchable(cfg, actor)
