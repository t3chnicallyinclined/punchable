"""Desk explorer core: graph.json + decisions.jsonl replayed on top.

graph.json is only ever opened for reading. decisions.jsonl is only ever
opened in append mode, one JSON object per line, flushed and fsynced.
Standard library only. See docs/KB_SCHEMA.md for the contract.
"""
from __future__ import annotations

import copy
import hashlib
import json
import math
import os
import re
import threading
from datetime import datetime, timezone
from pathlib import Path

OWNER_ID = os.environ.get("PK_OWNER_ID", "owner")  # the actor id on your own edits (`by`); PK_OWNER_ID

EDGE_KINDS = ["part_of", "variant_of", "performed_as", "tag_of", "same_shape", "callback_to",
              "same_premise", "has_label", "segue", "custom"]
STAGES = ["idea", "to_try", "testing", "working", "retired"]
LABEL_KINDS = ["device", "structure", "form", "attitude", "theory", "topic", "persona", "custom"]
ACTIONS = ("confirm", "reject", "edit", "add", "delete")
CONFIRMED = {"confirmed", "edited"}
REJECTED = {"rejected", "ruled_out", "superseded"}
# How much a reviewed answer of this kind teaches the next run (review-queue weight).
KIND_WEIGHT = {"same_shape": 1.0, "callback_to": 1.0, "variant_of": 1.0, "same_premise": 0.95, "has_label": 0.95,
               "segue": 0.85, "performed_as": 0.85, "part_of": 0.8, "tag_of": 0.75, "custom": 0.9}
# joke `hidden`: a merge folds a joke into another one (api-dev/merge_api.py); undo sets it back.
# joke `short_name`: what he writes on a paper set list (docs/UX_2026-10.md), 1-24 characters or null = the default.
NODE_EDIT_FIELDS = {"joke": {"stage", "title", "summary", "hidden", "short_name"}, "label": {"name", "definition", "merged_into"},
                    # Punchable (API_CONTRACT "Requested by app"): his show date / venue / "audio is bad" on a set,
                    # and what a stretch of tape is (a bit, crowd work or banter) on a performance.
                    "set": {"show_date", "venue", "audio_unreliable"},
                    "performance": {"kind"}}
PERF_KINDS = ("bit", "crowd_work", "banter")
DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")


def now_iso():
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def edge_id(src: str, kind: str, dst: str) -> str:
    return "e:" + hashlib.sha1(f"{src}|{kind}|{dst}".encode("utf-8")).hexdigest()[:12]


def slugify(s: str) -> str:
    s = re.sub(r"[^a-z0-9]+", "-", s.lower()).strip("-")
    return s[:60] or "x"


def bucket(status) -> str:
    if status in CONFIRMED:
        return "confirmed"
    if status in REJECTED:
        return "rejected"
    return "proposed"


# Lines written before imports carried their own actor (2026-10-27) say `by: <owner>` although an importer wrote them.
# decisions.jsonl is append-only, so they are never rewritten; readers get the corrected actor from actor_of().
LEGACY_IMPORT_REASONS = (("[gdocs] voice memo", "import:memo"), ("[gdocs]", "import:gsync"))


def actor_of(d: dict) -> str | None:
    """Who made a decision line: `by`, except legacy importer lines stamped with the owner (see above)."""
    by = d.get("by")
    if by == OWNER_ID:
        r = d.get("reason") or ""
        for prefix, actor in LEGACY_IMPORT_REASONS:
            if r.startswith(prefix) or r.startswith("undo: " + prefix):
                return actor if not r.startswith("undo: ") else by  # his undo of an import is his own action
    return by


def public(d: dict) -> dict:
    """A record without the view's private (_underscore) fields."""
    return {k: v for k, v in d.items() if not k.startswith("_")}


def ekind(e: dict) -> str:
    return e.get("custom_kind") or e.get("kind") or "?"


class DecisionError(ValueError):
    def __init__(self, msg, status=400):
        super().__init__(msg)
        self.status = status


def parse_report(path: Path) -> list[str]:
    """Edge IDs listed under a 'first 50' heading in KB_REPORT.md, in order."""
    try:
        text = path.read_text(encoding="utf-8")
    except OSError:
        return []
    out, level = [], None
    for line in text.splitlines():
        m = re.match(r"^(#{1,6})\s+(.*)$", line)
        if m:
            if level is not None and len(m.group(1)) <= level:
                break
            if level is None and re.search(r"first\s*50", m.group(2), re.I):
                level = len(m.group(1))
            continue
        if level is not None:
            for x in re.findall(r"e:[0-9a-f]{12}", line):
                if x not in out:
                    out.append(x)
    return out


class View:
    """The replayed state. Built from scratch whenever an input changes."""

    def __init__(self, graph: dict, decisions: list[dict], report: list[str], laughs_for):
        self.graph_meta = {k: graph.get(k) for k in ("version", "generated_at", "runs")}
        self.nodes: dict[str, dict] = {}
        for n in graph.get("nodes", []):
            if isinstance(n, dict) and n.get("id"):
                self.nodes[n["id"]] = dict(n)
        self.edges: dict[str, dict] = {}
        for e in graph.get("edges", []):
            if isinstance(e, dict) and e.get("id"):
                self.edges[e["id"]] = dict(e)
        self.decisions = decisions
        self.report = report
        self.laughs_for = laughs_for
        self.orphans = 0
        self.undone = {d["undo_of"] for d in decisions if isinstance(d.get("undo_of"), int)}
        for i, d in enumerate(decisions):
            self._apply(i, d)
        self._resolve_merges()
        self._index()

    # ---- replay ---------------------------------------------------------
    def _apply(self, i: int, d: dict):
        target, did, action = d.get("target"), d.get("id"), d.get("action")
        after = d.get("after") if isinstance(d.get("after"), dict) else {}
        before = d.get("before") if isinstance(d.get("before"), dict) else {}
        store = self.edges if target == "edge" else self.nodes if target == "node" else None
        if store is None or not did:
            return
        if action == "add":
            rec = dict(after)
            rec["id"] = did
            if target == "edge":
                rec.setdefault("status", "confirmed")
            rec["_by_tristan"] = True
            rec["_last"] = i
            store[did] = rec
        elif action == "delete":
            store.pop(did, None)
        elif action in ("confirm", "reject", "edit"):
            rec = store.get(did)
            if rec is None:
                # The pipeline dropped it on a re-run; the decision keeps it alive.
                if target == "edge" and before.get("src") and before.get("dst"):
                    rec = dict(before)
                    rec["id"] = did
                    store[did] = rec
                elif target == "node" and before.get("type"):
                    rec = dict(before)
                    rec["id"] = did
                    store[did] = rec
                else:
                    self.orphans += 1
                    return
            for k, v in after.items():
                if k != "id":
                    rec[k] = v
            rec["_last"] = i
            if target == "edge":
                if "undo_of" in d:
                    rec.pop("_edited", None)
                elif action == "edit":
                    rec["_edited"] = True

    def _resolve_merges(self):
        alias = {}
        for nid, n in self.nodes.items():
            if n.get("merged_into") and n["merged_into"] in self.nodes and n["merged_into"] != nid:
                alias[nid] = n["merged_into"]

        def resolve(x):
            seen = set()
            while x in alias and x not in seen:
                seen.add(x)
                x = alias[x]
            return x

        self.alias = {k: resolve(k) for k in alias}
        if not self.alias:
            return
        best: dict[tuple, str] = {}
        rank = {"confirmed": 2, "proposed": 1, "rejected": 0}
        for eid, e in list(self.edges.items()):
            s, t = self.alias.get(e.get("src"), e.get("src")), self.alias.get(e.get("dst"), e.get("dst"))
            if (s, t) != (e.get("src"), e.get("dst")):
                e["_merged_from"] = {"src": e.get("src"), "dst": e.get("dst")}
                e["src"], e["dst"] = s, t
            key = (s, ekind(e), t)
            if key in best:
                other = self.edges[best[key]]
                if rank[bucket(e.get("status"))] > rank[bucket(other.get("status"))]:
                    other["_dup_of"] = eid
                    best[key] = eid
                else:
                    e["_dup_of"] = best[key]
            else:
                best[key] = eid
        for eid in [k for k, e in self.edges.items() if e.get("_dup_of")]:
            del self.edges[eid]

    def _index(self):
        self.adj: dict[str, list[str]] = {}
        for eid, e in self.edges.items():
            for end in (e.get("src"), e.get("dst")):
                self.adj.setdefault(end, []).append(eid)
        self.jokes = [n for n in self.nodes.values() if n.get("type") == "joke"]
        self.labels = [n for n in self.nodes.values() if n.get("type") == "label" and not self.alias.get(n["id"])]
        # unit -> joke (via part_of that is not rejected)
        self.unit_joke: dict[str, str] = {}
        self.joke_units: dict[str, list[tuple[str, str]]] = {}
        for eid, e in self.edges.items():
            if e.get("kind") != "part_of":
                continue
            u, j = e.get("src"), e.get("dst")
            if self.ntype(u) == "joke" and self.ntype(j) == "unit":
                u, j = j, u
            if self.ntype(u) == "unit" and self.ntype(j) == "joke":
                self.joke_units.setdefault(j, []).append((u, eid))
                if bucket(e.get("status")) != "rejected":
                    self.unit_joke[u] = j
        self.degree = {nid: len(v) for nid, v in self.adj.items()}

    # ---- helpers --------------------------------------------------------
    def ntype(self, nid):
        n = self.nodes.get(nid)
        return n.get("type") if n else None

    def brief(self, nid: str) -> dict:
        n = self.nodes.get(nid)
        if not n:
            return {"id": nid, "type": "missing", "title": nid}
        t = n.get("type")
        b = {"id": nid, "type": t}
        if t == "joke":
            b.update(title=n.get("title") or nid, stage=n.get("stage"), summary=n.get("summary"))
        elif t == "label":
            b.update(title=n.get("name") or nid, kind=n.get("kind"), definition=n.get("definition"))
        elif t == "unit":
            b.update(title=(n.get("text") or "")[:140], kind=n.get("kind"), date=n.get("date"),
                     doc_title=n.get("doc_title"), joke=self.unit_joke.get(nid))
            if self.unit_joke.get(nid):
                b["joke_title"] = self.nodes.get(self.unit_joke[nid], {}).get("title")
        elif t == "performance":
            s = self.nodes.get(n.get("set"), {})
            b.update(title=(n.get("text") or "")[:140], set=n.get("set"), date=s.get("date"), venue=s.get("venue"),
                     start_ms=n.get("start_ms"), end_ms=n.get("end_ms"))
        elif t == "set":
            b.update(title=n.get("title") or nid, date=n.get("date"), venue=n.get("venue"))
        else:
            b["title"] = n.get("title") or n.get("name") or nid
        if n.get("_by_tristan"):
            b["yours"] = True
        return b

    def edge_view(self, e: dict, focus: str | None = None) -> dict:
        v = public(e)
        v["bucket"] = bucket(e.get("status"))
        v["display_kind"] = ekind(e)
        v["yours"] = bool(e.get("_by_tristan"))
        v["edited"] = bool(e.get("_edited"))
        v["src_brief"] = self.brief(e.get("src"))
        v["dst_brief"] = self.brief(e.get("dst"))
        if e.get("_merged_from"):
            v["merged_from"] = e["_merged_from"]
        if focus:
            other = e.get("dst") if e.get("src") == focus else e.get("src")
            v["other"] = self.brief(other)
            v["direction"] = "out" if e.get("src") == focus else "in"
        return v

    def joke_scope(self, jid: str) -> set[str]:
        return {jid} | {u for u, _ in self.joke_units.get(jid, [])}

    def joke_edges(self, jid: str) -> list[dict]:
        seen, out = set(), []
        for nid in self.joke_scope(jid):
            for eid in self.adj.get(nid, []):
                if eid not in seen:
                    seen.add(eid)
                    out.append(self.edges[eid])
        return out

    # ---- views ----------------------------------------------------------
    def joke_summaries(self) -> list[dict]:
        out = []
        for j in self.jokes:
            jid = j["id"]
            c = {"proposed": 0, "confirmed": 0, "rejected": 0}
            labels = {}
            for e in self.joke_edges(jid):
                b = bucket(e.get("status"))
                c[b] += 1
                if e.get("kind") == "has_label" and b != "rejected" and self.ntype(e.get("dst")) == "label":
                    lid = e["dst"]
                    if labels.get(lid) != "confirmed":
                        labels[lid] = b
            last = j.get("_last")   # the index of the last decision that touched it (an add, an edit, a confirm)
            touched = self.decisions[last].get("ts") if isinstance(last, int) and 0 <= last < len(self.decisions) else None
            out.append({"id": jid, "title": j.get("title") or jid, "stage": j.get("stage"),
                        "summary": j.get("summary"), "first_seen": j.get("first_seen"), "last_seen": j.get("last_seen"),
                        "touched": touched, "counts": c, "labels": [{"id": k, "name": self.nodes[k].get("name"), "bucket": v} for k, v in labels.items()],
                        "units": len(self.joke_units.get(jid, [])), "yours": bool(j.get("_by_tristan"))})
        return out

    def search(self, q: str) -> list[str]:
        words = [w for w in re.split(r"\s+", q.lower().strip()) if w]
        if not words:
            return [j["id"] for j in self.jokes]
        hits = []
        for j in self.jokes:
            jid = j["id"]
            parts = [j.get("title") or "", j.get("summary") or "", jid]
            for u, _ in self.joke_units.get(jid, []):
                parts.append(self.nodes.get(u, {}).get("text") or "")
            for e in self.joke_edges(jid):
                if e.get("kind") == "has_label" and bucket(e.get("status")) != "rejected":
                    ln = self.nodes.get(e.get("dst"), {})
                    parts.append(ln.get("name") or "")
                    parts.append(ln.get("kind") or "")
            hay = " ".join(parts).lower()
            if all(w in hay for w in words):
                hits.append(jid)
        return hits

    def joke_detail(self, jid: str) -> dict | None:
        j = self.nodes.get(jid)
        if not j or j.get("type") != "joke":
            return None
        units = []
        for uid, peid in self.joke_units.get(jid, []):
            u = self.nodes.get(uid, {})
            units.append({**public(u), "edge": self.edge_view(self.edges[peid], jid)})
        units.sort(key=lambda u: (u.get("date") or "", u.get("id")))
        groups: dict[str, list] = {}
        for u in units:
            groups.setdefault(u.get("date") or "undated", []).append(u)
        perfs, edges = [], []
        for e in self.joke_edges(jid):
            ev = self.edge_view(e, jid if jid in (e.get("src"), e.get("dst")) else None)
            if ev.get("other") is None:
                # an edge on one of the joke's units
                unit_end = e.get("src") if e.get("src") in self.joke_scope(jid) else e.get("dst")
                ev["via_unit"] = self.brief(unit_end)
                ev["other"] = self.brief(e.get("dst") if unit_end == e.get("src") else e.get("src"))
            if e.get("kind") == "part_of" and ev["other"].get("type") == "unit" and e.get("dst") == jid:
                continue  # shown in the units section
            if e.get("kind") == "part_of" and ev["other"].get("type") == "unit" and e.get("src") == jid:
                continue
            if e.get("kind") == "performed_as" and ev["other"].get("type") == "performance":
                p = self.nodes.get(ev["other"]["id"], {})
                s = self.nodes.get(p.get("set"), {})
                laughs = self.laughs_for(p.get("set"), p.get("start_ms"), p.get("end_ms"))
                perfs.append({"perf": public(p), "set": public(s) if s else {"id": p.get("set")}, "edge": ev, "laughs": laughs})
                continue
            edges.append(ev)
        perfs.sort(key=lambda x: (x["set"].get("date") or "", x["perf"].get("start_ms") or 0))
        edges.sort(key=lambda x: ({"proposed": 0, "confirmed": 1, "rejected": 2}[x["bucket"]], x["display_kind"], -(x.get("confidence") or 0)))
        return {"joke": public(j), "yours": bool(j.get("_by_tristan")),
                "unit_groups": [{"date": k, "units": v} for k, v in groups.items()],
                "performances": perfs, "edges": edges,
                "history": [dict(d, index=i, undone=i in self.undone) for i, d in enumerate(self.decisions)
                            if d.get("id") in self.joke_scope(jid) or d.get("id") in {e["id"] for e in self.joke_edges(jid)}][-30:]}

    def label_index(self) -> list[dict]:
        out = []
        for lab in sorted(self.labels, key=lambda n: (LABEL_KINDS.index(n.get("kind")) if n.get("kind") in LABEL_KINDS else 99, (n.get("name") or "").lower())):
            lid = lab["id"]
            carriers = []
            for eid in self.adj.get(lid, []):
                e = self.edges[eid]
                if e.get("kind") != "has_label" or e.get("dst") != lid:
                    continue
                src = e.get("src")
                via = None
                if self.ntype(src) == "unit":
                    via = src
                    src = self.unit_joke.get(src, src)
                carriers.append({"edge": eid, "bucket": bucket(e.get("status")), "confidence": e.get("confidence"),
                                 "joke": self.brief(src), "via_unit": via})
            merged = sorted(k for k, v in self.alias.items() if v == lid)
            out.append({**public(lab), "yours": bool(lab.get("_by_tristan")), "carriers": carriers, "merged_from": merged,
                        "counts": {b: sum(1 for c in carriers if c["bucket"] == b) for b in ("proposed", "confirmed", "rejected")}})
        return out

    def queue(self, status: str = "proposed", limit: int = 200) -> list[dict]:
        items = []
        maxdeg = max(self.degree.values(), default=1) or 1
        rep = {x: i for i, x in enumerate(self.report)}
        for eid, e in self.edges.items():
            if bucket(e.get("status")) != status:
                continue
            conf = e.get("confidence")
            conf = 0.5 if not isinstance(conf, (int, float)) else max(0.0, min(1.0, float(conf)))
            unc = 1 - abs(conf - 0.5) * 2
            deg = self.degree.get(e.get("src"), 0) + self.degree.get(e.get("dst"), 0)
            impact = math.log1p(deg) / math.log1p(2 * maxdeg)
            score = (0.55 * unc + 0.45 * impact) * KIND_WEIGHT.get(e.get("kind"), 0.9)
            why = []
            if eid in rep:
                why.append(f"KB report: confirm first (#{rep[eid] + 1})")
            why.append(f"{'uncertain' if unc >= 0.5 else 'fairly sure'} ({round(conf * 100)}%)")
            why.append(f"touches {deg} links")
            items.append((0 if eid in rep else 1, rep.get(eid, 0), -score, eid,
                          {**self.edge_view(e), "score": round(score, 3), "why": " · ".join(why),
                           "context": self._context(e)}))
        items.sort(key=lambda t: t[:4])
        return [t[4] for t in items[:limit]], len(items)

    def _context(self, e: dict) -> dict:
        """A first line of material for each end, so a pair can be judged without opening it."""
        ctx = {}
        for end in ("src", "dst"):
            nid = e.get(end)
            if self.ntype(nid) == "joke":
                us = sorted(self.joke_units.get(nid, []), key=lambda x: self.nodes.get(x[0], {}).get("date") or "")
                ctx[end] = [(self.nodes.get(u, {}).get("text") or "")[:220] for u, _ in us[:2]]
            elif self.ntype(nid) in ("unit", "performance"):
                ctx[end] = [(self.nodes.get(nid, {}).get("text") or "")[:300]]
            elif self.ntype(nid) == "label":
                ctx[end] = [self.nodes[nid].get("definition") or ""]
        return ctx

    def neighborhood(self, center: str, hops: int, kinds: set | None, types: set, show_rejected: bool, cap: int = 120):
        if center not in self.nodes:
            return None
        seen = {center: 0}
        frontier = [center]
        edges = {}
        for h in range(1, hops + 1):
            nxt = []
            for nid in frontier:
                for eid in self.adj.get(nid, []):
                    e = self.edges[eid]
                    if kinds and e.get("kind") not in kinds and ekind(e) not in kinds:
                        continue
                    if not show_rejected and bucket(e.get("status")) == "rejected":
                        continue
                    other = e["dst"] if e["src"] == nid else e["src"]
                    if self.ntype(other) not in types and other != center:
                        continue
                    if other not in seen:
                        if len(seen) >= cap:
                            continue
                        seen[other] = h
                        nxt.append(other)
                    edges[eid] = e
            frontier = nxt
        edges = {k: e for k, e in edges.items() if e["src"] in seen and e["dst"] in seen}
        nodes = [{**self.brief(n), "hop": h} for n, h in seen.items()]
        return {"center": center, "nodes": nodes, "truncated": len(seen) >= cap,
                "edges": [{"id": k, "src": e["src"], "dst": e["dst"], "kind": ekind(e), "bucket": bucket(e.get("status")),
                           "confidence": e.get("confidence")} for k, e in edges.items()]}

    def stats(self) -> dict:
        by_kind, by_label = {}, {}
        added_labels, added_edges, added_jokes, node_edits = [], [], [], []
        for i, d in enumerate(self.decisions):
            if i in self.undone or "undo_of" in d:
                continue
            a, t = d.get("action"), d.get("target")
            before = d.get("before") if isinstance(d.get("before"), dict) else {}
            after = d.get("after") if isinstance(d.get("after"), dict) else {}
            if t == "edge" and a in ("confirm", "reject", "edit"):
                k = before.get("custom_kind") or before.get("kind") or "?"
                row = by_kind.setdefault(k, {"confirm": 0, "reject": 0, "edit": 0})
                row[a] += 1
                if before.get("kind") == "has_label" and before.get("dst"):
                    lid = self.alias.get(before["dst"], before["dst"])
                    r = by_label.setdefault(lid, {"name": (self.nodes.get(lid) or {}).get("name") or lid,
                                                  "confirm": 0, "reject": 0, "edit": 0})
                    r[a] += 1
            elif t == "edge" and a == "add":
                added_edges.append({"index": i, "ts": d.get("ts"), "id": d.get("id"), "kind": ekind(after),
                                    "src": self.brief(after.get("src")), "dst": self.brief(after.get("dst")), "reason": d.get("reason")})
            elif t == "node" and a == "add":
                (added_labels if after.get("type") == "label" else added_jokes).append(
                    {"index": i, "ts": d.get("ts"), "id": d.get("id"), "name": after.get("name") or after.get("title"),
                     "kind": after.get("kind"), "definition": after.get("definition"), "reason": d.get("reason")})
            elif t == "node" and a == "edit":
                node_edits.append({"index": i, "ts": d.get("ts"), "id": d.get("id"), "after": after, "reason": d.get("reason"),
                                   "title": self.brief(d.get("id")).get("title")})
        for r in list(by_kind.values()) + list(by_label.values()):
            n = r["confirm"] + r["reject"]
            r["confirm_rate"] = round(r["confirm"] / n, 3) if n else None
        live = [d for i, d in enumerate(self.decisions) if i not in self.undone and "undo_of" not in d]
        return {"total_decisions": len(self.decisions), "effective": len(live), "undone": len(self.undone),
                "by_kind": by_kind, "by_label": by_label, "added_labels": added_labels, "added_edges": added_edges,
                "added_jokes": added_jokes, "node_edits": node_edits,
                "edge_status": {b: sum(1 for e in self.edges.values() if bucket(e.get("status")) == b) for b in ("proposed", "confirmed", "rejected")}}


class KB:
    def __init__(self, graph: Path, decisions: Path, corpus: Path | None, report: Path | None):
        self.graph_path = Path(graph)
        self.decisions_path = Path(decisions)
        self.corpus = Path(corpus) if corpus else None
        self.report_path = Path(report) if report else None
        if self.decisions_path.resolve() == self.graph_path.resolve() or self.decisions_path.suffix != ".jsonl":
            raise SystemExit("refusing: the decisions file must be a separate .jsonl file, never graph.json")
        self.lock = threading.RLock()
        self._graph_sig = None
        self._graph = {"nodes": [], "edges": []}
        self.graph_state = "missing"
        self.graph_error = None
        self._view = None
        self._view_sig = None
        self._laughs_cache: dict[str, tuple] = {}
        self.bad_lines = 0

    # ---- inputs ---------------------------------------------------------
    @staticmethod
    def _sig(p: Path | None):
        try:
            st = p.stat()
            return (st.st_mtime_ns, st.st_size)
        except (OSError, AttributeError):
            return None

    def _load_graph(self):
        sig = self._sig(self.graph_path)
        if sig == self._graph_sig:
            return
        if sig is None:
            self._graph, self.graph_state, self.graph_error = {"nodes": [], "edges": []}, "missing", None
        else:
            try:
                with open(self.graph_path, "r", encoding="utf-8") as f:  # read-only, always
                    g = json.load(f)
                if not isinstance(g, dict):
                    raise ValueError("graph.json is not an object")
                self._graph, self.graph_state, self.graph_error = g, "ok", None
            except (ValueError, OSError) as e:
                # Probably being written right now: keep the last good copy.
                self.graph_state, self.graph_error = "unreadable", f"{type(e).__name__}: {e}"
                if self._view is not None:
                    self._graph_sig = None
                    return
        self._graph_sig = sig

    def _read_decisions(self) -> list[dict]:
        out, bad = [], 0
        try:
            with open(self.decisions_path, "r", encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        d = json.loads(line)
                        if isinstance(d, dict):
                            out.append(d)
                        else:
                            bad += 1
                    except ValueError:
                        bad += 1
        except FileNotFoundError:
            pass
        self.bad_lines = bad
        return out

    def laughs_for(self, set_id, start_ms, end_ms):
        if not self.corpus or not set_id or start_ms is None:
            return None
        sid = set_id[4:] if set_id.startswith("set:") else set_id
        if not re.match(r"^[A-Za-z0-9_-]+$", sid):
            return None
        p = self.corpus / "sets" / sid / "laughs_auto.json"
        sig = self._sig(p)
        if sig is None:
            return None
        hit = self._laughs_cache.get(sid)
        if not hit or hit[0] != sig:
            try:
                doc = json.loads(p.read_text(encoding="utf-8"))
            except (ValueError, OSError):
                return None
            hit = (sig, doc)
            self._laughs_cache[sid] = hit
        doc = hit[1]
        end = (end_ms if end_ms is not None else start_ms) + 3000  # laughs land just after the line
        ls = [l for l in doc.get("laughs", []) if isinstance(l, dict) and l.get("start_ms") is not None
              and start_ms <= l["start_ms"] <= end]
        return {"detector": doc.get("detector"), "run_id": doc.get("run_id"), "laughs": ls,
                "max_bars": max((l.get("bars") or 0 for l in ls), default=0), "hits": len(ls)}

    def view(self) -> View:
        with self.lock:
            self._load_graph()
            sig = (self._graph_sig, self._sig(self.decisions_path), self._sig(self.report_path), self.graph_state)
            if self._view is None or sig != self._view_sig:
                report = parse_report(self.report_path) if self.report_path else []
                self._view = View(self._graph, self._read_decisions(), report, self.laughs_for)
                self._view_sig = sig
            return self._view

    # ---- writing --------------------------------------------------------
    def _append(self, d: dict) -> int:
        line = json.dumps(d, ensure_ascii=False, separators=(",", ":"))
        assert "\n" not in line
        self.decisions_path.parent.mkdir(parents=True, exist_ok=True)
        with open(self.decisions_path, "ab") as f:  # append-only
            if f.tell() > 0:
                with open(self.decisions_path, "rb") as r:
                    r.seek(-1, os.SEEK_END)
                    if r.read(1) != b"\n":
                        f.write(b"\n")  # never glue onto a torn last line
            f.write(line.encode("utf-8") + b"\n")
            f.flush()
            os.fsync(f.fileno())
        self._view_sig = None
        return len(self.view().decisions) - 1

    def decide(self, req: dict, by: str = OWNER_ID) -> dict:
        """`by`: who made it. The comic's own action is OWNER_ID; imports are `import:<source>` and machine judgments
        `ai:<what>` (the API checks the actor; this only records it)."""
        with self.lock:
            v = self.view()
            d = self._build(v, req)
            d = {"ts": now_iso(), "target": d["target"], "id": d["id"], "action": d["action"],
                 "before": d.get("before"), "after": d.get("after"), "reason": (req.get("reason") or "").strip(),
                 "by": by, **({"undo_of": d["undo_of"]} if "undo_of" in d else {})}
            idx = self._append(d)
            return {"ok": True, "index": idx, "decision": d}

    def undo(self, index: int | None, by: str = OWNER_ID) -> dict:
        with self.lock:
            v = self.view()
            ds = v.decisions
            if index is None:
                cands = [i for i, d in enumerate(ds) if i not in v.undone and "undo_of" not in d]
                if not cands:
                    raise DecisionError("nothing to undo", 409)
                index = cands[-1]
            if not (0 <= index < len(ds)):
                raise DecisionError("no such decision", 404)
            d = ds[index]
            if index in v.undone:
                raise DecisionError("already undone", 409)
            if "undo_of" in d:
                raise DecisionError("that line is itself an undo", 409)
            t, did, a = d.get("target"), d.get("id"), d.get("action")
            store = v.edges if t == "edge" else v.nodes
            cur = store.get(did)
            before = d.get("before") if isinstance(d.get("before"), dict) else {}
            after = d.get("after") if isinstance(d.get("after"), dict) else {}
            if a == "add":
                inv = {"action": "delete", "before": public(cur) if cur else after, "after": None}
            elif a == "delete":
                inv = {"action": "add", "before": None, "after": before}
            else:
                inv = {"action": "edit", "before": public(cur) if cur else None,
                       "after": {k: before.get(k) for k in after}}
            reason = "undo" + (f": {d.get('reason')}" if d.get("reason") else "")
            out = {"ts": now_iso(), "target": t, "id": did, **inv, "reason": reason, "by": by, "undo_of": index}
            out = {k: out[k] for k in ("ts", "target", "id", "action", "before", "after", "reason", "by", "undo_of")}
            idx = self._append(out)
            return {"ok": True, "index": idx, "decision": out}

    # ---- validation -----------------------------------------------------
    def _build(self, v: View, req: dict) -> dict:
        target, action = req.get("target"), req.get("action")
        after = req.get("after") or {}
        if target not in ("edge", "node") or action not in ACTIONS:
            raise DecisionError("target must be edge|node and action confirm|reject|edit|add|delete")
        if not isinstance(after, dict):
            raise DecisionError("after must be an object")
        reason = req.get("reason")
        if reason is not None and (not isinstance(reason, str) or len(reason) > 4000):
            raise DecisionError("reason must be text under 4000 characters")

        def need_node(nid, types=None):
            n = v.nodes.get(nid)
            if not n or v.alias.get(nid):
                raise DecisionError(f"unknown node {nid!r}", 404)
            if types and n.get("type") not in types:
                raise DecisionError(f"{nid} is a {n.get('type')}, expected {'/'.join(types)}")
            return n

        def kind_fields(src):
            kind = src.get("kind")
            if kind not in EDGE_KINDS:
                raise DecisionError(f"unknown kind {kind!r}")
            out = {"kind": kind}
            if kind == "custom":
                ck = (src.get("custom_kind") or "").strip()
                if not ck or len(ck) > 60:
                    raise DecisionError("a custom connection needs a name (custom_kind), up to 60 characters")
                out["custom_kind"] = ck
            else:
                out["custom_kind"] = None
            return out

        if target == "edge":
            if action == "add":
                src, dst = after.get("src"), after.get("dst")
                need_node(src)
                need_node(dst)
                if src == dst:
                    raise DecisionError("a connection needs two different ends")
                kf = kind_fields(after)
                if kf["kind"] == "has_label" and v.ntype(dst) != "label":
                    raise DecisionError("has_label must point at a label")
                if not (req.get("reason") or "").strip():
                    raise DecisionError("say why (reason) when you add a connection")
                eid = edge_id(src, kf.get("custom_kind") or kf["kind"], dst)
                if eid in v.edges:
                    ex = v.edges[eid]
                    raise DecisionError(f"that connection already exists ({bucket(ex.get('status'))}); confirm or edit it instead", 409)
                rec = {"src": src, "dst": dst, **{k: x for k, x in kf.items() if x is not None}, "status": "confirmed",
                       "confidence": 1.0, "reason": req["reason"].strip(), "evidence": [], "source": OWNER_ID}
                return {"target": "edge", "id": eid, "action": "add", "before": None, "after": rec}
            eid = req.get("id")
            e = v.edges.get(eid)
            if not e:
                raise DecisionError(f"unknown edge {eid!r}", 404)
            snap = public(e)
            if action == "confirm":
                return {"target": "edge", "id": eid, "action": "confirm", "before": snap, "after": {"status": "confirmed"}}
            if action == "reject":
                return {"target": "edge", "id": eid, "action": "reject", "before": snap, "after": {"status": "rejected"}}
            if action == "delete":
                if not e.get("_by_tristan"):
                    raise DecisionError("only connections you added can be deleted; reject this one instead")
                return {"target": "edge", "id": eid, "action": "delete", "before": snap, "after": None}
            # edit
            ch = {}
            if "kind" in after:
                ch.update(kind_fields(after))
            for end in ("src", "dst"):
                if end in after and after[end] != e.get(end):
                    need_node(after[end])
                    ch[end] = after[end]
            s2, d2 = ch.get("src", e.get("src")), ch.get("dst", e.get("dst"))
            if s2 == d2:
                raise DecisionError("a connection needs two different ends")
            if ch.get("kind", e.get("kind")) == "has_label" and v.ntype(d2) != "label":
                raise DecisionError("has_label must point at a label")
            if not ch and not (req.get("reason") or "").strip():
                raise DecisionError("nothing changed")
            ch = {k: x for k, x in ch.items() if not (x is None and e.get(k) is None)}
            ch["status"] = "confirmed"
            return {"target": "edge", "id": eid, "action": "edit", "before": snap, "after": ch}

        # nodes
        if action == "add":
            t = after.get("type")
            if t == "label":
                name = (after.get("name") or "").strip()
                if not name or len(name) > 80:
                    raise DecisionError("a label needs a name (up to 80 characters)")
                kind = after.get("kind") or "custom"
                if kind not in LABEL_KINDS:
                    raise DecisionError(f"unknown label kind {kind!r}")
                nid = f"label:{kind}:{slugify(name)}"
                if nid in v.nodes:
                    raise DecisionError(f"label {nid} already exists; pick it instead", 409)
                rec = {"type": "label", "kind": kind, "name": name, "definition": (after.get("definition") or "").strip()}
                return {"target": "node", "id": nid, "action": "add", "before": None, "after": rec}
            if t == "joke":
                title = (after.get("title") or "").strip()
                if not title or len(title) > 120:
                    raise DecisionError("a joke or idea needs a title (up to 120 characters)")
                base = f"joke:{slugify(title)}"
                nid, k = base, 2
                while nid in v.nodes:
                    nid, k = f"{base}-{k}", k + 1
                stage = after.get("stage") or "idea"
                if stage not in STAGES:
                    raise DecisionError(f"unknown stage {stage!r}")
                rec = {"type": "joke", "title": title, "stage": stage, "summary": (after.get("summary") or "").strip(),
                       "first_seen": now_iso()[:10], "last_seen": now_iso()[:10]}
                return {"target": "node", "id": nid, "action": "add", "before": None, "after": rec}
            if t == "unit":
                # a line of his: a version written in the app (POST /api/jokes/<id>/versions), a kept draft
                # (Brainstorm "Use as mine"), or a typed idea folded into a bit by a merge (merge_api). The part_of edge
                # to its joke is the next decision; until then the unit stands alone.
                text = (after.get("text") or "").strip() if isinstance(after.get("text"), str) else ""
                if not text or len(text) > 4000:
                    raise DecisionError("a line needs its text (up to 4000 characters)")
                nid = after.get("id")
                if not (isinstance(nid, str) and re.match(r"^unit:[A-Za-z0-9._-]{1,80}$", nid)):
                    nid = "unit:" + hashlib.sha1(f"{text}|{now_iso()}|{len(v.nodes)}".encode("utf-8")).hexdigest()[:12]
                if nid in v.nodes:
                    raise DecisionError(f"{nid} already exists", 409)
                date = after.get("date") if isinstance(after.get("date"), str) and DATE_RE.match(after["date"]) else now_iso()[:10]
                rec = {"type": "unit", "text": text, "kind": str(after.get("kind") or "tag")[:40],
                       "source": str(after.get("source") or "app")[:40], "date": date}
                if isinstance(after.get("doc_title"), str) and after["doc_title"].strip():
                    rec["doc_title"] = after["doc_title"].strip()[:200]
                return {"target": "node", "id": nid, "action": "add", "before": None, "after": rec}
            raise DecisionError("you can add a label, a joke/idea or a line (unit)")
        nid = req.get("id")
        n = need_node(nid)
        snap = public(n)
        if action == "delete":
            if not n.get("_by_tristan"):
                raise DecisionError("only things you added can be deleted")
            return {"target": "node", "id": nid, "action": "delete", "before": snap, "after": None}
        if action in ("confirm", "reject"):
            return {"target": "node", "id": nid, "action": action, "before": snap,
                    "after": {"status": "confirmed" if action == "confirm" else "rejected"}}
        allowed = NODE_EDIT_FIELDS.get(n.get("type"), set())
        ch = {}
        for k, x in after.items():
            if k not in allowed:
                raise DecisionError(f"can't edit {k!r} on a {n.get('type')}")
            if k == "stage" and x not in STAGES:
                raise DecisionError(f"unknown stage {x!r}")
            if k == "merged_into" and x is not None:
                need_node(x, ("label",))
                if x == nid:
                    raise DecisionError("can't merge a label into itself")
            if k in ("name", "title") and (not isinstance(x, str) or not x.strip()):
                raise DecisionError(f"{k} can't be empty")
            if k == "show_date" and x is not None:
                if not isinstance(x, str) or not DATE_RE.match(x):
                    raise DecisionError("show_date must be YYYY-MM-DD (or null to clear it)")
                try:
                    datetime.strptime(x, "%Y-%m-%d")
                except ValueError:
                    raise DecisionError(f"show_date {x!r} is not a real date")
            if k == "venue" and (not isinstance(x, str) or not x.strip() or len(x) > 120):
                raise DecisionError("venue must be 1-120 characters")
            if k == "audio_unreliable" and not isinstance(x, bool):
                raise DecisionError("audio_unreliable must be true or false")
            if k == "hidden" and not isinstance(x, bool):
                raise DecisionError("hidden must be true or false")
            if k == "short_name" and x is not None and (not isinstance(x, str) or not x.strip() or len(x.strip()) > 24):
                raise DecisionError("short_name must be 1-24 characters (or null for the default)")
            if k == "kind" and x not in PERF_KINDS:
                raise DecisionError(f"kind must be one of {', '.join(PERF_KINDS)}")
            if x != n.get(k):
                ch[k] = x.strip() if isinstance(x, str) else x
        if not ch:
            raise DecisionError("nothing changed")
        return {"target": "node", "id": nid, "action": "edit", "before": snap, "after": ch}
