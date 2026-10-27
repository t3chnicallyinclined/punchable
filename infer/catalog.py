"""His written catalog: jokes, units, labels (graph v0 + any v1 label batches)."""
import glob
import json
import math
import os
import re
from collections import defaultdict

import numpy as np

from .paths import GRAPH, KB_VECTORS, UNITS, V1_BATCHES


def slug(s):
    s = s.lower().replace("’", "'").replace("&", " and ")
    return re.sub(r"[^a-z0-9]+", "-", s).strip("-")


# Spellings the catalog pass may use for a persona label -> the label slug. Your own personas go in a private file,
# $PUNCHABLE_HOME/persona_synonyms.json ({"<spelling>": "<label slug>"}); it replaces these examples when present.
PERSONA_SYN = {"tech guy": "tech-guy", "computer tech": "tech-guy", "it guy": "tech-guy"}


def _local_personas() -> dict | None:
    p = os.path.join(os.environ.get("PUNCHABLE_HOME") or os.path.join(os.path.expanduser("~"), ".punchable"),
                     "persona_synonyms.json")
    try:
        with open(p, encoding="utf-8") as f:
            d = json.load(f)
        return {k.lower(): v for k, v in d.items() if not k.startswith("_")}
    except (OSError, ValueError):
        return None


if _local_personas() is not None:
    PERSONA_SYN = _local_personas()


class Catalog:
    def __init__(self):
        g = json.load(open(GRAPH, encoding="utf-8"))
        self.graph = g
        self.nodes = {n["id"]: n for n in g["nodes"]}
        self.jokes = {k: n for k, n in self.nodes.items() if n["type"] == "joke"}
        self.labels = {k: n for k, n in self.nodes.items() if n["type"] == "label"}
        self.units = {u["unit_id"]: u for u in json.load(open(UNITS, encoding="utf-8"))}
        self.unit_joke, self.joke_units = {}, defaultdict(list)
        self.joke_labels = defaultdict(dict)   # joke -> label -> (conf, reason)
        self.rel = defaultdict(list)           # joke -> [(other, kind, conf, reason)]
        for e in g["edges"]:
            if e["kind"] == "part_of" and e["dst"] in self.jokes:
                uid = e["src"].split(":", 1)[1]
                self.unit_joke[uid] = e["dst"]
                self.joke_units[e["dst"]].append(uid)
            elif e["kind"] == "has_label" and e["src"] in self.jokes:
                self.joke_labels[e["src"]][e["dst"]] = (e.get("confidence", 0.5), e.get("reason", ""))
            elif e["kind"] in ("same_shape", "same_premise", "variant_of", "tag_of", "callback_to") \
                    and e["src"] in self.jokes and e["dst"] in self.jokes:
                self.rel[e["src"]].append((e["dst"], e["kind"], e.get("confidence", 0.5), e.get("reason", "")))
                self.rel[e["dst"]].append((e["src"], e["kind"], e.get("confidence", 0.5), e.get("reason", "")))
        # label name lookup
        self.by_name = {}
        for lid, n in self.labels.items():
            self.by_name[(n["kind"], n["name"].lower())] = lid
            self.by_name[(n["kind"], lid.split(":", 2)[2].replace("_", " ").replace("-", " "))] = lid
        meta_p = os.path.join(KB_VECTORS, "build_meta.json")
        self.topic_merge = {}
        if os.path.exists(meta_p):
            for canon, alts in json.load(open(meta_p, encoding="utf-8")).get("topic_merges", {}).items():
                for a in alts:
                    self.topic_merge[a] = canon
        # v1 labels per unit (better catalog pass; use whatever batches exist)
        self.v1 = {}
        for f in sorted(glob.glob(os.path.join(V1_BATCHES, "labels_*.json"))):
            try:
                for u in json.load(open(f, encoding="utf-8")).get("units", []):
                    self.v1[u["unit_id"]] = u
            except (ValueError, KeyError):
                continue
        self._idf = None

    # ---- labels
    def label_id(self, kind, name):
        n = name.lower().strip()
        if kind == "persona":
            p = PERSONA_SYN.get(n)
            return f"label:persona:{p}" if p and f"label:persona:{p}" in self.labels else None
        if kind == "topic":
            n = self.topic_merge.get(n, n)
            lid = f"label:topic:{slug(n)}"
            return lid if lid in self.labels else None
        return self.by_name.get((kind, n)) or self.by_name.get((kind, n.replace("-", " ")))

    def unit_v1_labels(self, uid):
        """[(label_id, confidence, reason, evidence)] from the v1 pass for one unit."""
        out = []
        u = self.v1.get(uid)
        for l in (u or {}).get("labels") or []:
            lid = self.label_id(l.get("kind", ""), l.get("name", ""))
            if lid:
                out.append((lid, float(l.get("confidence", 0.5)), l.get("reason", ""), l.get("evidence", "")))
        return out

    def craft_labels(self, joke_id):
        """Craft profile of a joke: v0 has_label confidence, or the v1 support
        (sum of v1 confidences over its units / number of units), whichever is
        higher. Support-weighting keeps a 26-unit joke from becoming a hub."""
        if not hasattr(self, "_craft"):
            self._craft = {}
        if joke_id in self._craft:
            return self._craft[joke_id]
        out = {}
        for lid, (c, _r) in self.joke_labels.get(joke_id, {}).items():
            out[lid] = max(out.get(lid, 0), c)
        us = self.joke_units.get(joke_id, [])
        sup = {}
        for uid in us:
            for lid, c, _r, _e in self.unit_v1_labels(uid):
                sup[lid] = sup.get(lid, 0) + c
        for lid, t in sup.items():
            out[lid] = max(out.get(lid, 0), t / max(1, len(us)))
        self._craft[joke_id] = out
        return out

    def idf(self):
        if self._idf is None:
            df = defaultdict(int)
            for j in self.jokes:
                for lid in self.craft_labels(j):
                    df[lid] += 1
            N = max(1, len(self.jokes))
            self._idf = {l: math.log(N / (1 + d)) for l, d in df.items()}
        return self._idf

    # ---- vectors
    def joke_vectors(self, emb):
        if getattr(self, "_jv", None) is None:
            ids, rows = [], []
            for j in self.jokes:
                us = self.joke_units.get(j, [])
                if not us:
                    continue
                v = np.mean([emb.unit(u, self.units[u]["text"]) for u in us if u in self.units], axis=0)
                ids.append(j)
                rows.append(v / (np.linalg.norm(v) or 1))
            self._jv = (ids, np.stack(rows))
        return self._jv
