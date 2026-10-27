"""Punchable: writes go through the local API (POST /api/ideas, /api/decision, /api/undo) so they land as normal decision
lines (by: owner) in decisions.jsonl and SurrealDB. Reads that the API has no endpoint for use the documented files,
read-only: decisions.jsonl, kb/v1/graph.json, kb/creative/storyboards.json, kb/creative/merges.json."""
from __future__ import annotations

import json
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
    def __init__(self, cfg: Config):
        self.cfg = cfg
        self.calls = 0

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
            r = self._req("POST", "/api/ideas", {"text": text, "title": title or "", "from": frm, "reason": reason})
            return {"id": r["id"], "indexes": r["indexes"]}
        t = (title or " ".join(text.split()[:7]))[:120] or "New line"
        r = self._req("POST", "/api/decision", {"target": "node", "action": "add", "reason": reason,
                                                "after": {"type": "joke", "title": t, "stage": "idea", "summary": text}})
        return {"id": r["decisions"][0]["id"], "indexes": r["indexes"]}

    def edit_joke(self, jid: str, after: dict, reason: str) -> dict | None:
        try:
            r = self._req("POST", "/api/decision", {"target": "node", "action": "edit", "id": jid, "after": after,
                                                    "reason": reason})
        except ApiError as e:
            if e.status == 400 and "nothing changed" in str(e):
                return None
            raise
        return {"indexes": r["indexes"]}

    def undo(self, indexes: list[int]) -> dict:
        return self._req("POST", "/api/undo", {"indexes": indexes})

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
