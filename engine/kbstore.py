"""The knowledge base as the engine sees it.

- graph.json: read, merged, written atomically (tmp + os.replace).
- decisions.jsonl: the comic's. Only ever opened for READING here. Replayed on top
  of graph.json with the Desk explorer's own View (tools/explorer/kb.py), so the
  engine and the UI agree on what is confirmed, edited and rejected.
"""
from __future__ import annotations

import importlib.util
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

from .config import Config

OWNER_ID = os.environ.get("PK_OWNER_ID", "owner")  # the actor id on your own edits (`by`); PK_OWNER_ID

CONFIRMED = {"confirmed", "edited"}
REJECTED = {"rejected", "ruled_out", "superseded"}
AUTO_STATUSES = {"confirmed_auto", "auto_confirmed", "auto"}


def _load_module(name: str, path: Path):
    if name in sys.modules:
        return sys.modules[name]
    spec = importlib.util.spec_from_file_location(name, str(path))
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def explorer_kb(cfg: Config):
    return _load_module("wd_explorer_kb", cfg.p("tools_dir") / "explorer" / "kb.py")


def kb_vocab(cfg: Config):
    return _load_module("wd_kb_vocab", cfg.p("tools_dir") / "kb" / "kb_vocab.py")


def edge_id(src, kind, dst):
    import hashlib
    return "e:" + hashlib.sha1(f"{src}|{kind}|{dst}".encode("utf-8")).hexdigest()[:12]


def now_iso():
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


class KB:
    def __init__(self, cfg: Config):
        self.cfg = cfg
        self.dir = cfg.p("kb_dir")
        self.graph_path = self.dir / "graph.json"
        self.decisions_path = self.dir / "decisions.jsonl"
        self.reload()

    def reload(self):
        with open(self.graph_path, "r", encoding="utf-8") as f:
            self.graph = json.load(f)
        self.decisions = self.read_decisions()
        ek = explorer_kb(self.cfg)
        self.view = ek.View(self.graph, self.decisions, [], lambda *a, **k: None)
        self._derive()

    def read_decisions(self) -> list[dict]:
        out = []
        try:
            with open(self.decisions_path, "r", encoding="utf-8") as f:  # read only, always
                for line in f:
                    line = line.strip()
                    if line:
                        try:
                            d = json.loads(line)
                            if isinstance(d, dict):
                                out.append(d)
                        except ValueError:
                            pass
        except FileNotFoundError:
            pass
        return out

    # ---- what the decisions say --------------------------------------------
    def _derive(self):
        v = self.view
        self.touched = {d.get("id") for d in self.decisions if d.get("id")}
        self.rejected_edges = {eid for eid, e in v.edges.items() if e.get("status") in REJECTED}
        self.rejected_edge_keys = {(e.get("src"), e.get("kind"), e.get("dst")) for eid, e in v.edges.items()
                                   if e.get("status") in REJECTED}
        self.rejected_nodes = set()
        for d in self.decisions:
            if d.get("action") == "delete":
                (self.rejected_edges if d.get("target") == "edge" else self.rejected_nodes).add(d.get("id"))
            if d.get("target") == "node" and d.get("action") == "reject":
                self.rejected_nodes.add(d.get("id"))
        # origins of rejected engine-proposed nodes (a re-titled proposal of the same span stays rejected)
        self.rejected_origins = set()
        gnodes = {n["id"]: n for n in self.graph.get("nodes", []) if isinstance(n, dict) and n.get("id")}
        for d in self.decisions:
            if d.get("id") in self.rejected_nodes:
                n = gnodes.get(d["id"]) or d.get("before") or {}
                if n.get("origin_span"):
                    self.rejected_origins.add(n["origin_span"])
        self.performed_jokes = {e["dst"] for e in v.edges.values() if e.get("kind") == "performed_as"
                                and e.get("status") not in REJECTED}

    def is_rejected_edge(self, e: dict) -> bool:
        return e["id"] in self.rejected_edges or (e.get("src"), e.get("kind"), e.get("dst")) in self.rejected_edge_keys \
            or e.get("src") in self.rejected_nodes or e.get("dst") in self.rejected_nodes

    def examples(self, kinds: set[str], dst_prefix: str | None = None, n_pos=8, n_neg=3):
        """the comic's own confirmed (and ruled-out) edges, for few-shot prompts.

        Only decisions by OWNER_ID count, the edge must still be in that state after
        replay, and anything auto-confirmed is never used."""
        pos, neg, seen = [], [], set()
        for d in reversed(self.decisions):
            if d.get("target") != "edge" or d.get("by") != OWNER_ID or d.get("auto"):
                continue
            eid = d.get("id")
            if eid in seen:
                continue
            seen.add(eid)
            e = self.view.edges.get(eid)
            if not e or e.get("kind") not in kinds or e.get("status") in AUTO_STATUSES:
                continue
            if dst_prefix and not str(e.get("dst", "")).startswith(dst_prefix):
                continue
            rec = {"id": eid, "src": e.get("src"), "kind": e.get("kind"), "dst": e.get("dst"),
                   "reason": d.get("reason") or e.get("reason"),
                   "evidence": [x.get("quote") for x in e.get("evidence", []) if x.get("quote")][:2],
                   "src_title": self.title(e.get("src")), "dst_title": self.title(e.get("dst"))}
            if e.get("status") in CONFIRMED and d.get("action") in ("confirm", "edit", "add"):
                if len(pos) < n_pos:
                    pos.append(rec)
            elif e.get("status") in REJECTED and d.get("action") == "reject":
                if len(neg) < n_neg:
                    neg.append(rec)
        return pos, neg

    def title(self, nid):
        n = self.view.nodes.get(nid) or {}
        return n.get("title") or n.get("name") or (n.get("text") or "")[:80] or nid

    # ---- labels -------------------------------------------------------------
    def label_vocab(self) -> dict[str, dict]:
        """device / structure / topic / custom labels that are live (not merged away)."""
        out = {}
        for n in self.view.labels:
            if n.get("kind") in ("device", "structure", "topic", "custom") and n["id"] not in self.rejected_nodes:
                out[n["id"]] = {"name": n.get("name"), "kind": n.get("kind"), "definition": n.get("definition", "")}
        return out

    # ---- merge --------------------------------------------------------------
    def merge_run(self, run: dict, nodes: list[dict], edges: list[dict], origin_sha: str) -> dict:
        """Merge one engine run into graph.json. Never touches decisions.jsonl.

        - earlier untouched proposals from the same recording are replaced;
        - anything the comic decided on is kept as it is in graph.json (his decision
          replays on top of it);
        - rejected / deleted ids (and rejected src-kind-dst triples) are never re-added.
        """
        self.reload()
        g = self.graph
        stats = {"nodes_added": 0, "edges_added": 0, "skipped_rejected": 0, "kept_decided": 0, "replaced_stale": 0}

        def mine(x):
            return isinstance(x, dict) and (x.get("origin") or {}).get("sha256") == origin_sha

        keep_edges = [e for e in g["edges"] if not (mine(e) and e["id"] not in self.touched)]
        pinned = {x for e in keep_edges if mine(e) for x in (e["src"], e["dst"])}  # ends of decided edges
        keep_nodes = [n for n in g["nodes"] if not (mine(n) and n["id"] not in self.touched and n["id"] not in pinned)]
        stats["replaced_stale"] = (len(g["nodes"]) - len(keep_nodes)) + (len(g["edges"]) - len(keep_edges))
        node_ids = {n["id"] for n in keep_nodes}
        edge_ids = {e["id"] for e in keep_edges}
        for n in nodes:
            if n["id"] in self.rejected_nodes or (n.get("origin_span") and n["origin_span"] in self.rejected_origins
                                                  and n.get("type") == "joke"):
                stats["skipped_rejected"] += 1
                continue
            if n["id"] in node_ids:
                if n["id"] in self.touched:
                    stats["kept_decided"] += 1
                    continue
                keep_nodes = [x for x in keep_nodes if x["id"] != n["id"]]
            keep_nodes.append(n)
            node_ids.add(n["id"])
            stats["nodes_added"] += 1
        for e in edges:
            if self.is_rejected_edge(e) or e["src"] not in node_ids or e["dst"] not in node_ids:
                stats["skipped_rejected"] += 1
                continue
            if e["id"] in edge_ids:
                if e["id"] in self.touched:
                    stats["kept_decided"] += 1
                    continue
                keep_edges = [x for x in keep_edges if x["id"] != e["id"]]
            keep_edges.append(e)
            edge_ids.add(e["id"])
            stats["edges_added"] += 1
        g["nodes"], g["edges"] = keep_nodes, keep_edges
        g.setdefault("runs", []).append(run)
        self.write_graph(g)
        self.reload()
        return stats

    def write_graph(self, g: dict):
        tmp = self.graph_path.with_suffix(".json.tmp")
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(g, f, ensure_ascii=False, indent=1)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, self.graph_path)
