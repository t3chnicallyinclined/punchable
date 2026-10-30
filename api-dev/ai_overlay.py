"""The AI layer: inferred.json and the per-set analysis.json files, laid over graph.json.

Precedence (docs/KB_SCHEMA.md, "AI-first display rule"):  the comic's decisions > inferred > proposed.
  graph.json (proposed)  ->  inferred.json  ->  analysis.json rows  ->  decisions.jsonl replayed by kb.View

Nothing here writes anything. Files are re-read whenever their mtime/size changes, so a new
inferred.json or analysis.json is picked up on the next request with no restart and no code change.
Until the infer pipeline writes the real files, the fixtures in api-dev/fixtures/ stand in for them
(per set: a real analysis.json always wins over the fixture for that set).

The overlay is tolerant on purpose (the infer pipeline is being written in parallel):
  inferred.json   {jokes: [ {id|joke_id, stage?, stage_confidence?, stage_reason?, performances?: [...]} ] | {id: {...}},
                   edges: [ {id, status?, confidence?, reason?, ...} or full edges with src/dst/kind ],
                   nodes?: [ full nodes ]}
  analysis.json   exactly the "Set analysis" schema.
"""
from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path

RATINGS = ("killed", "worked", "meh", "died")
NOT_COUNTED = ("crowd_work", "banter")


def edge_id(src: str, kind: str, dst: str) -> str:
    return "e:" + hashlib.sha1(f"{src}|{kind}|{dst}".encode("utf-8")).hexdigest()[:12]


def set_dir_of(sid: str | None) -> str | None:
    if not sid:
        return None
    sid = sid[4:] if sid.startswith("set:") else sid
    m = re.match(r"^(\d{4})-?(\d{2})-?(\d{2})_([A-Za-z0-9_-]+)$", sid)
    return f"{m.group(1)}{m.group(2)}{m.group(3)}_{m.group(4)}" if m else None


def set_id_of(d: str) -> str | None:
    m = re.match(r"^(\d{4})(\d{2})(\d{2})_([A-Za-z0-9_-]+)$", d or "")
    return f"set:{m.group(1)}-{m.group(2)}-{m.group(3)}_{m.group(4)}" if m else None


class AIFiles:
    """Finds and caches inferred.json and every analysis.json (real first, then fixture).

    id_map: kb/v1/id_map.json (v0 id -> v1 id). The infer files may still carry v0 ids; they are resolved
    through it when the overlay is built (see remap_ai_ids)."""

    def __init__(self, kbdir: Path, corpus: Path | None, fixtures: Path | None, id_map: Path | None = None):
        self.kbdir, self.corpus, self.fixtures = kbdir, corpus, fixtures
        self.id_map_path = id_map
        self._cache: dict[str, tuple] = {}

    def id_map(self) -> dict | None:
        if not self.id_map_path:
            return None
        doc, _ = self._load(self.id_map_path)
        return doc

    @staticmethod
    def _sig(p: Path):
        try:
            st = p.stat()
            return (st.st_mtime_ns, st.st_size)
        except OSError:
            return None

    def _load(self, p: Path):
        sig = self._sig(p)
        if sig is None:
            return None, None
        key = str(p)
        hit = self._cache.get(key)
        if hit and hit[0] == sig:
            return hit[1], sig
        try:
            doc = json.loads(p.read_text(encoding="utf-8"))
            if not isinstance(doc, dict):
                raise ValueError("not an object")
        except (ValueError, OSError):
            # probably mid-write: keep the last good copy
            return (hit[1], hit[0]) if hit else (None, None)
        self._cache[key] = (sig, doc)
        return doc, sig

    def inferred(self):
        """(doc, source, path, sig)."""
        for src, p in (("real", self.kbdir / "inferred.json"), ("fixture", self.fixtures / "inferred.json" if self.fixtures else None)):
            if p is None:
                continue
            doc, sig = self._load(p)
            if doc is not None:
                return doc, src, p, sig
        return None, None, None, None

    def analyses(self) -> dict[str, tuple]:
        """set dir -> (doc, source, path, sig). A real analysis.json beats the fixture for the same set."""
        out = {}
        roots = [("fixture", self.fixtures / "sets" if self.fixtures else None), ("real", self.corpus / "sets" if self.corpus else None)]
        for src, root in roots:
            if root is None or not root.is_dir():
                continue
            for d in root.iterdir():
                p = d / "analysis.json"
                if not d.is_dir() or not set_id_of(d.name):
                    continue
                doc, sig = self._load(p)
                if doc is not None and isinstance(doc.get("jokes"), list):
                    out[d.name] = (doc, src, p, sig)
        return out

    def signature(self):
        inf = self.inferred()
        an = self.analyses()
        return (str(inf[2]), inf[3], tuple(sorted((k, str(v[2]), v[3]) for k, v in an.items())),
                str(self.id_map_path), self._sig(self.id_map_path) if self.id_map_path else None)

    def set_meta(self, d: str) -> dict:
        if not self.corpus:
            return {}
        p = self.corpus / "sets" / d / "meta.json"
        doc, _ = self._load(p)
        return doc or {}


def remap_ai_ids(doc: dict, kind: str, idmap: dict | None, known: dict, log: dict) -> dict:
    """Resolve v0 ids in an inferred.json / analysis.json doc through id_map.json (jokes, units, edges).

    Returns the doc unchanged when nothing maps, else a remapped deep copy (the cached doc is never mutated).
    An id that is neither in the graph nor resolvable through the map is listed in log["unresolved"]."""
    if not idmap or not isinstance(doc, dict):
        return doc
    jm, um, em = idmap.get("jokes") or {}, idmap.get("units") or {}, idmap.get("edges") or {}
    changed = []

    def node(x):
        if not isinstance(x, str) or not x or x in known:
            return x
        y = jm.get(x) if x.startswith("joke:") else um.get(x) if x.startswith("unit:") else None
        if y and y in known:
            changed.append((x, y))
            return y
        if x.startswith(("joke:", "unit:")) and x not in log["unresolved"]:
            log["unresolved"].append(x)
        return x

    d = json.loads(json.dumps(doc))
    if kind == "inferred":
        js = d.get("jokes")
        items = js.values() if isinstance(js, dict) else (js or [])
        for j in items:
            if isinstance(j, dict):
                for k in ("id", "joke_id"):
                    if k in j:
                        j[k] = node(j[k])
        if isinstance(js, dict):
            d["jokes"] = {node(k): v for k, v in js.items()}
        for e in d.get("edges") or []:
            if isinstance(e, dict):
                s0, t0 = e.get("src"), e.get("dst")
                e["src"], e["dst"] = node(s0), node(t0)
                if e.get("id") in em and em[e["id"]] and (e["src"], e["dst"]) != (s0, t0):
                    e["id"] = em[e["id"]]
    else:
        for row in d.get("jokes") or []:
            if not isinstance(row, dict):
                continue
            for k in ("joke_id", "version_unit_id"):
                if row.get(k):
                    row[k] = node(row[k])
            for x in row.get("similar") or []:
                if isinstance(x, dict) and x.get("joke_id"):
                    x["joke_id"] = node(x["joke_id"])
        sm = d.get("summary")
        if isinstance(sm, dict):
            for k in ("best", "weakest"):
                if isinstance(sm.get(k), str):
                    sm[k] = node(sm[k])
    if not changed:
        return doc
    log["applied"] += len(changed)
    log["pairs"].extend(f"{a} -> {b}" for a, b in changed if f"{a} -> {b}" not in log["pairs"])
    return d


def _overlap(a0, a1, b0, b1):
    return max(0, min(a1, b1) - max(a0, b0))


def build_overlay(graph: dict, files: AIFiles) -> tuple[dict, dict]:
    """Return (merged graph, index). The merged graph is graph.json with the AI layer applied.

    index = {"rows": {set_id: [(row, perf_id)]}, "analysis": {set_id: meta}, "inferred": meta|None}
    """
    nodes = {n["id"]: n for n in graph.get("nodes", []) if isinstance(n, dict) and n.get("id")}
    edges = {e["id"]: e for e in graph.get("edges", []) if isinstance(e, dict) and e.get("id")}
    own_n, own_e = set(), set()  # copied (so the cached graph.json dicts are never mutated)

    def wn(nid):
        if nid not in own_n:
            nodes[nid] = dict(nodes[nid])
            own_n.add(nid)
        return nodes[nid]

    def we(eid):
        if eid not in own_e:
            edges[eid] = dict(edges[eid])
            own_e.add(eid)
        return edges[eid]

    def add_edge(e):
        edges[e["id"]] = e
        own_e.add(e["id"])

    index = {"rows": {}, "analysis": {}, "inferred": None}
    idmap = files.id_map() if hasattr(files, "id_map") else None
    maplog = {"applied": 0, "pairs": [], "unresolved": []}
    if idmap:
        index["id_map"] = maplog

    # ---- inferred.json ----------------------------------------------------
    doc, src, path, _ = files.inferred()
    doc = remap_ai_ids(doc, "inferred", idmap, nodes, maplog)
    if doc:
        run = doc.get("run_id") or "inferred"
        index["inferred"] = {"source": src, "path": str(path), "run_id": run, "generated_at": doc.get("generated_at"),
                             "notes": doc.get("notes")}
        alias: dict[str, str] = {}  # an inferred performance that is a graph.json performance under another start time

        def same_perf(n):
            best, bo = None, -1
            for p in nodes.values():
                if p.get("type") != "performance" or p.get("set") != n.get("set") or p.get("_ai_node"):
                    continue
                s0, s1 = n.get("start_ms") or 0, n.get("end_ms") or 0
                p0, p1 = p.get("start_ms") or 0, p.get("end_ms") or 0
                o = _overlap(s0, s1, p0, p1)
                if (abs(p0 - s0) <= 3000 or o >= 0.5 * max(1, min(s1 - s0, p1 - p0))) and o > bo:
                    best, bo = p, o
            return best

        SPAN = {"id", "type", "set", "start_ms", "end_ms", "text", "status", "source"}
        for n in doc.get("nodes") or []:
            if not isinstance(n, dict) or not n.get("id") or not n.get("type"):
                continue
            target = n["id"] if n["id"] in nodes else None
            if target is None and n["type"] == "performance":
                m = same_perf(n)
                if m:
                    alias[n["id"]] = target = m["id"]
            if target is None:
                nodes[n["id"]] = {k: x for k, x in n.items() if k != "status"} | {"_ai_node": True}
                own_n.add(n["id"])
                continue
            x = wn(target)
            for k, val in n.items():
                if k not in SPAN and val is not None:
                    x[k] = val
        js = doc.get("jokes") or []
        if isinstance(js, dict):
            js = [{"id": k, **v} for k, v in js.items() if isinstance(v, dict)]
        for j in js:
            if not isinstance(j, dict):
                continue
            jid = j.get("id") or j.get("joke_id")
            if not jid or (nodes.get(jid) or {}).get("type") != "joke":
                continue
            stage = j.get("stage") or j.get("inferred_stage")
            if stage in ("idea", "to_try", "testing", "working", "retired"):
                n = wn(jid)
                ai = {"stage": stage, "from_stage": n.get("stage"), "stage_confidence": j.get("stage_confidence", j.get("confidence")),
                      "stage_reason": j.get("stage_reason") or j.get("reason") or "", "run_id": run}
                n["stage"] = stage
                n["_ai"] = ai
            if isinstance(j.get("hit_sets"), int):
                wn(jid)["_ai_hit_sets"] = j["hit_sets"]
            for p in j.get("performances") or []:
                pid = p.get("perf") if isinstance(p, dict) else None
                pid = alias.get(pid, pid)
                r = p.get("ai_rating") or p.get("rating") if isinstance(p, dict) else None
                if pid and (nodes.get(pid) or {}).get("type") == "performance" and r in RATINGS:
                    pn = wn(pid)
                    pn["ai_rating"] = r
                    if p.get("rating_basis"):
                        pn["rating_basis"] = p["rating_basis"]
        for e in doc.get("edges") or []:
            if not isinstance(e, dict) or not e.get("id"):
                continue
            st = e.get("status") or "inferred"
            src, dst = alias.get(e.get("src"), e.get("src")), alias.get(e.get("dst"), e.get("dst"))
            eid = e["id"]
            if eid not in edges and (src, dst) != (e.get("src"), e.get("dst")) and e.get("kind"):
                eid = edge_id(src, e.get("custom_kind") or e["kind"], dst)
            if eid in edges:
                x = we(eid)
                for k in ("status", "confidence", "reason", "evidence", "source"):
                    if k in e and e[k] is not None:
                        x[k] = e[k]
                x["status"] = st
            elif src in nodes and dst in nodes and e.get("kind"):
                add_edge({**e, "id": eid, "src": src, "dst": dst, "status": st, "source": e.get("source") or f"auto:{run}"})
        # inferred > proposed: an inferred match for a performance retires the other proposed matches for it
        by_perf: dict[str, list[str]] = {}
        for x in edges.values():
            if x.get("kind") == "performed_as":
                by_perf.setdefault(x["src"] if str(x.get("src", "")).startswith("perf:") else x.get("dst"), []).append(x["id"])
        for ids in by_perf.values():
            if any(edges[i].get("status") == "inferred" for i in ids):
                for i in ids:
                    if edges[i].get("status") in (None, "proposed"):
                        we(i)["status"] = "superseded"

    # ---- analysis.json, per set ------------------------------------------
    perfs_by_set: dict[str, list[dict]] = {}
    for n in nodes.values():
        if n.get("type") == "performance":
            perfs_by_set.setdefault(n.get("set"), []).append(n)
    pa_by_perf: dict[str, list[str]] = {}
    for e in edges.values():
        if e.get("kind") == "performed_as":
            p = e["src"] if str(e.get("src", "")).startswith("perf:") else e.get("dst")
            pa_by_perf.setdefault(p, []).append(e["id"])

    for d, (doc, src, path, _) in sorted(files.analyses().items()):
        doc = remap_ai_ids(doc, "analysis", idmap, nodes, maplog)
        sid = set_id_of(d)
        if sid not in nodes:
            meta = files.set_meta(d)
            date = sid[4:14]
            nodes[sid] = {"id": sid, "type": "set", "title": meta.get("title") or d, "date": date,
                          "venue": meta.get("venue_guess") or "", "duration_s": meta.get("duration") or (doc.get("summary") or {}).get("duration_s") or 0,
                          "_ai_node": True}
            own_n.add(sid)
        run = doc.get("run_id") or "analysis"
        index["analysis"][sid] = {"source": src, "path": str(path), "run_id": run, "generated_at": doc.get("generated_at"),
                                  "summary": doc.get("summary") or {}, "set": d}
        used = set()
        rows = []
        for row in doc["jokes"]:
            if not isinstance(row, dict) or row.get("start_ms") is None:
                continue
            s0, s1 = int(row["start_ms"]), int(row.get("end_ms") or row["start_ms"])
            best, bo = None, 0
            for p in perfs_by_set.get(sid, []):
                if p["id"] in used:
                    continue
                p0, p1 = p.get("start_ms") or 0, p.get("end_ms") or 0
                o = _overlap(s0, s1, p0, p1)
                if abs(p0 - s0) <= 3000 or o >= 0.5 * max(1, min(s1 - s0, p1 - p0)):
                    o = max(o, 1)
                    if o > bo:
                        best, bo = p, o
            status = row.get("status") if row.get("status") in ("known", "new", "crowd_work", "banter") else ("known" if row.get("joke_id") else "new")
            if best:
                pid = best["id"]
                pn = wn(pid)
            else:
                pid = f"perf:{sid[4:]}@{s0}"
                if pid in nodes:
                    pn = wn(pid)
                else:
                    pn = {"id": pid, "type": "performance", "set": sid, "start_ms": s0, "end_ms": s1,
                          "text": row.get("transcript") or "", "kind": "bit", "name": row.get("title") or "", "_ai_node": True}
                    nodes[pid] = pn
                    own_n.add(pid)
                    perfs_by_set.setdefault(sid, []).append(pn)
            used.add(pid)
            if row.get("ai_rating") in RATINGS:
                pn["ai_rating"] = row["ai_rating"]
                pn["rating_basis"] = row.get("rating_basis") or ""
            if status in NOT_COUNTED:
                pn["kind"] = status
            elif pn.get("kind") in NOT_COUNTED:
                pn["kind"] = "bit"
            pn["_ai_kind"] = pn.get("kind")
            jid = row.get("joke_id")
            if status == "known" and jid and (nodes.get(jid) or {}).get("type") == "joke":
                eid = edge_id(pid, "performed_as", jid)
                for other in pa_by_perf.get(pid, []):
                    if other != eid and edges[other].get("status") in (None, "proposed", "inferred"):
                        we(other)["status"] = "superseded"
                pl = [x for x in (row.get("punchlines") or []) if isinstance(x, dict) and x.get("text")]
                if eid in edges:
                    x = we(eid)
                    x["status"] = "inferred"
                    x["confidence"] = row.get("match_confidence", x.get("confidence"))
                    if row.get("match_reason"):
                        x["reason"] = row["match_reason"]
                else:
                    add_edge({"id": eid, "src": pid, "dst": jid, "kind": "performed_as", "status": "inferred",
                              "confidence": row.get("match_confidence") or 0.5, "reason": row.get("match_reason") or "",
                              "evidence": [{"node": pid, "quote": pl[0]["text"], "start_ms": pl[0].get("at_ms")}] if pl else [],
                              "source": f"auto:{run}"})
                    pa_by_perf.setdefault(pid, []).append(eid)
            elif status == "new":
                for other in pa_by_perf.get(pid, []):
                    if edges[other].get("status") in (None, "proposed", "inferred"):
                        we(other)["status"] = "superseded"
            rows.append((row, pid))
        index["rows"][sid] = rows

    if maplog["unresolved"]:
        import sys
        sys.stderr.write(f"ai_overlay: {len(maplog['unresolved'])} id(s) in the infer files resolve neither in the graph nor "
                         f"through id_map.json: {', '.join(maplog['unresolved'][:20])}\n")
    merged = {k: graph.get(k) for k in ("version", "generated_at", "runs")}
    merged["nodes"] = list(nodes.values())
    merged["edges"] = list(edges.values())
    return merged, index
