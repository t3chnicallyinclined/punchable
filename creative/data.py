"""Read-only corpus view for the creative layer.

Loads KB v1 graph.json with the comic's decisions.jsonl replayed on top (v0 ids mapped through
kb/v1/id_map.json), the per-set analysis.json files, inferred.json and insights.json.
Nothing here writes. Tests build a Corpus from in-memory dicts.

Every quote this layer emits is checked by `Corpus.verify()` against the text of the node it
cites: a unit or performance node's `text`, a set segment's `transcript` (`seg:<set>#<idx>`),
or a named field of a joke node (`field: "quality.notes"`).
"""
from __future__ import annotations

import json
import math
import os
from collections import defaultdict
from pathlib import Path

import numpy as np

from .util import mean, median, prefix_words, r2

DEFAULT_COMEDY = os.path.expanduser("~/punchable-data")
DESK = Path(__file__).resolve().parent.parent
VECTORS_DIR = DESK / "tools" / "kb" / "cache"
WPM = 157.0  # his taped pace (learn/insights `chunks.his_pace_wpm`)
RATING_ORDER = {"killed": 3, "worked": 2, "meh": 1, "died": 0}
LINK_KINDS = ("segue", "same_premise", "callback_to", "tag_of", "same_shape", "variant_of")


def comedy_dir() -> Path:
    return Path(os.environ.get("COMEDY_DIR", DEFAULT_COMEDY))


def creative_dir() -> Path:
    return Path(os.environ.get("PUNCHABLE_CREATIVE_DIR", comedy_dir() / "kb" / "creative"))


def set_key_from_dir(d: str) -> str:
    """20240719_FakeSet0006 -> 2024-07-19_FakeSet0006 (the analysis.json `set` field)."""
    if len(d) > 9 and d[8] == "_" and d[:8].isdigit():
        return f"{d[:4]}-{d[4:6]}-{d[6:8]}_{d[9:]}"
    return d


def venue_short(venue: str | None, date: str | None) -> str:
    v = (venue or "").strip()
    low = v.lower()
    if "maybe " in low:
        name = v[low.index("maybe ") + 6:].split(")")[0].strip(" '\"") + "?"
    elif not v or low.startswith("unknown"):
        name = "Unknown room"
    else:
        name = v.split("(")[0].split(",")[0].strip()
        for tail in (" Comedy Club", " Poets Cafe", " Comedy"):
            if name.endswith(tail) and len(name) > len(tail) + 2:
                name = name[: -len(tail)]
        if name.lower().startswith("comix at "):
            name = name[9:]
    yy = f" '{date[2:4]}" if date and len(date) >= 4 else ""
    return name + yy


class Vectors:
    """bge-small vectors cached by tools/kb (read-only): `unit:<id>` and `reveal:<id>`."""

    def __init__(self, vec_dir: Path | None = None, table: dict | None = None):
        self._row, self._m, self.table = {}, None, table
        if table is None:
            vd = Path(vec_dir or VECTORS_DIR)
            mats, n = [], 0
            # v0 cache: unit:<id>, reveal:<id>; v1 cache: areveal:<id> (the v1 anatomy reveal)
            for ids_name, vec_name in (("vectors_ids.json", "vectors.f32"), ("vectors_v1_ids.json", "vectors_v1.f32")):
                ids_p, vec_p = vd / ids_name, vd / vec_name
                if ids_p.exists() and vec_p.exists():
                    ids = json.load(open(ids_p, encoding="utf-8"))
                    m = np.fromfile(vec_p, dtype=np.float32)
                    if m.size == len(ids) * 384:
                        mats.append(m.reshape(len(ids), 384))
                        for i, k in enumerate(ids):
                            self._row.setdefault(k, n + i)
                        n += len(ids)
            if mats:
                self._m = np.concatenate(mats)

    @property
    def available(self) -> bool:
        return self.table is not None or self._m is not None

    def unit(self, uid: str):
        return self.get(uid)

    def reveal(self, uid: str):
        """Reveal vector of a unit: the v1 anatomy reveal if cached, else the v0 reveal."""
        bare = uid[5:] if uid.startswith("unit:") else uid
        v = self.get(f"areveal:{bare}")
        return v if v is not None else self.get(f"reveal:{bare}")

    def get(self, key: str):
        if self.table is not None:
            v = self.table.get(key)
            return None if v is None else np.asarray(v, dtype=np.float32)
        i = self._row.get(key)
        return None if i is None else self._m[i]


class Corpus:
    def __init__(self, graph: dict, analyses: dict | None = None, inferred: dict | None = None,
                 insights: dict | None = None, decisions: list | None = None, id_map: dict | None = None,
                 metas: dict | None = None, vectors: Vectors | None = None):
        self.id_map = id_map or {}
        self.inferred = inferred or {}
        self.insights = insights or {}
        self.decisions = decisions or []
        self.metas = metas or {}
        self.vectors = vectors or Vectors(table={})
        self._joke_vec: dict = {}
        self._build_graph(graph)
        self._build_sets(analyses or {})

    # ------------------------------------------------------------------ loading
    @classmethod
    def load(cls, comedy: Path | str | None = None) -> "Corpus":
        root = Path(comedy or comedy_dir())
        kb1, kb0 = root / "kb" / "v1", root / "kb" / "v0"
        graph = json.load(open(kb1 / "graph.json", encoding="utf-8"))
        id_map = json.load(open(kb1 / "id_map.json", encoding="utf-8")) if (kb1 / "id_map.json").exists() else {}

        def opt(p):
            return json.load(open(p, encoding="utf-8")) if p.exists() else None

        decisions = []
        dp = kb0 / "decisions.jsonl"
        if dp.exists():
            with open(dp, encoding="utf-8") as f:  # read-only: the comic's file
                for line in f:
                    line = line.strip()
                    if line:
                        try:
                            decisions.append(json.loads(line))
                        except json.JSONDecodeError:
                            pass
        analyses, metas = {}, {}
        sets = root / "corpus" / "sets"
        if sets.exists():
            for d in sorted(os.listdir(sets)):
                ap = sets / d / "analysis.json"
                if ap.exists():
                    a = json.load(open(ap, encoding="utf-8"))
                    key = a.get("set") or set_key_from_dir(d)
                    analyses[key] = a
                    m = opt(sets / d / "meta.json")
                    if m:
                        metas[key] = m
        return cls(graph, analyses, opt(kb0 / "inferred.json"), opt(kb0 / "insights.json"), decisions, id_map,
                   metas, Vectors())

    # ------------------------------------------------------------------ id mapping
    def map_joke(self, jid):
        if not jid:
            return None
        m = self.id_map.get("jokes", {})
        if jid in m:
            return m[jid]
        return jid

    def map_unit(self, uid):
        return self.id_map.get("units", {}).get(uid, uid)

    def map_edge(self, eid):
        return self.id_map.get("edges", {}).get(eid, eid)

    # ------------------------------------------------------------------ graph
    def _build_graph(self, graph: dict):
        self.nodes = {n["id"]: dict(n) for n in graph.get("nodes", [])}
        edges = {e["id"]: dict(e) for e in graph.get("edges", [])}
        self.his_nodes, self.his_stage, self.perf_overrides, self.his_lists = set(), set(), {}, {}
        # replay decisions (last one wins, in file order)
        for d in self.decisions:
            if d.get("by", "owner") != "owner" or d.get("auto"):
                continue
            tgt, act, after = d.get("target"), d.get("action"), d.get("after") or {}
            if tgt == "edge":
                eid = self.map_edge(d.get("id"))
                if act in ("reject", "delete"):
                    edges.pop(eid, None)
                elif act == "add" and after.get("src") and after.get("dst"):
                    e = dict(after)
                    e.setdefault("id", eid)
                    e["_his"] = True
                    edges[e["id"]] = e
                elif eid in edges:
                    edges[eid].update({k: v for k, v in after.items() if k != "id"})
                    edges[eid]["_his"] = True
            elif tgt == "node":
                nid = d.get("id", "")
                if nid.startswith("joke:"):
                    nid = self.map_joke(nid) or nid
                if nid.startswith("perf:"):
                    self.perf_overrides.setdefault(nid, {}).update(after)
                elif nid.startswith("list:"):
                    if act == "delete":
                        self.his_lists.pop(nid, None)
                    else:
                        self.his_lists.setdefault(nid, {}).update(after)
                elif nid in self.nodes:
                    if act == "delete":
                        self.nodes[nid]["hidden"] = True
                    else:
                        self.nodes[nid].update({k: v for k, v in after.items() if k != "id"})
                        self.his_nodes.add(nid)
                        if "stage" in after:
                            self.his_stage.add(nid)
                elif act == "add" and after.get("type") == "joke":
                    n = dict(after)
                    n["id"] = nid
                    self.nodes[nid] = n
                    self.his_nodes.add(nid)
        self.edges = list(edges.values())
        self.jokes = {k: n for k, n in self.nodes.items() if n.get("type") == "joke" and not n.get("hidden")}
        self.labels = {k: n for k, n in self.nodes.items() if n.get("type") == "label"}
        self.edges_by_node = defaultdict(list)
        self.unit_joke, self.joke_units = {}, defaultdict(list)
        self.joke_labels = defaultdict(dict)  # joke -> label -> (weight, edge)
        best_part = {}
        for e in self.edges:
            self.edges_by_node[e["src"]].append(e)
            self.edges_by_node[e["dst"]].append(e)
            k = e.get("kind")
            if k == "part_of" and e["src"].startswith("unit:") and e["dst"] in self.jokes:
                c = e.get("confidence") or 0
                if c > best_part.get(e["src"], (-1, None))[0]:
                    best_part[e["src"]] = (c, e["dst"])
            elif k == "has_label" and e["src"] in self.jokes and e["dst"] in self.labels:
                w = 1.0 if self.source_of(e) == "his" else float(e.get("confidence") or 0)
                if w > self.joke_labels[e["src"]].get(e["dst"], (-1, None))[0]:
                    self.joke_labels[e["src"]][e["dst"]] = (w, e)
        for u, (_, j) in sorted(best_part.items()):
            un = self.nodes.get(u)
            if un and not un.get("hidden"):
                self.unit_joke[u] = j
                self.joke_units[j].append(u)
        # label document frequency (labels held with weight >= 0.5)
        df = defaultdict(int)
        for j, ls in self.joke_labels.items():
            for l, (w, _) in ls.items():
                if w >= 0.5:
                    df[l] += 1
        self.label_df = dict(df)
        n = max(1, len(self.jokes))
        self.idf = {l: math.log(n / (1 + c)) + 1.0 for l, c in df.items()}
        # inferred.json overlay (v0 ids mapped)
        self.inferred_jokes = {}
        for j in self.inferred.get("jokes", []):
            jid = self.map_joke(j.get("id"))
            if jid:
                self.inferred_jokes[jid] = j

    @staticmethod
    def source_of(e: dict) -> str:
        """his = the comic confirmed/edited/added it; inferred = accepted by the AI; proposed = a suggestion."""
        if e.get("_his") or e.get("by") == "owner":
            return "his"
        if e.get("status") in ("inferred", "confirmed", "confirmed_auto"):
            return "inferred"
        return "proposed"

    def stage(self, jid: str) -> tuple[str | None, str]:
        n = self.jokes.get(jid) or {}
        if jid in self.his_stage:
            return n.get("stage"), "his"
        ij = self.inferred_jokes.get(jid)
        if ij and ij.get("stage"):
            return ij["stage"], "inferred"
        return n.get("stage"), "proposed"

    def title(self, nid: str) -> str:
        n = self.nodes.get(nid) or {}
        return n.get("title") or n.get("name") or nid

    def labels_of(self, jid: str, kinds=None, min_w: float = 0.5) -> dict:
        out = {}
        for l, (w, e) in self.joke_labels.get(jid, {}).items():
            if w >= min_w and (kinds is None or self.labels[l].get("kind") in kinds):
                out[l] = (w, e)
        return out

    def devices(self, jid: str, n: int = 3, min_w: float = 0.55) -> list[str]:
        ls = self.labels_of(jid, ("device", "structure"), min_w)
        return [l for l, _ in sorted(ls.items(), key=lambda kv: (-kv[1][0], kv[0]))[:n]]

    def lead_unit(self, jid: str):
        n = self.jokes.get(jid) or {}
        u = self.map_unit(n.get("lead_unit")) if n.get("lead_unit") else None
        if u in self.nodes and not self.nodes[u].get("hidden"):
            return u
        us = self.joke_units.get(jid) or []
        return max(us, key=lambda x: (len(self.nodes[x].get("text") or ""), x)) if us else None

    def joke_vec(self, jid: str):
        if jid in self._joke_vec:
            return self._joke_vec[jid]
        vs = [self.vectors.get(u) for u in self.joke_units.get(jid, [])]
        vs = [v for v in vs if v is not None]
        v = None
        if vs:
            v = np.mean(np.stack(vs), axis=0)
            nrm = float(np.linalg.norm(v))
            v = v / nrm if nrm else None
        self._joke_vec[jid] = v
        return v

    def cos(self, a, b) -> float | None:
        va, vb = self.joke_vec(a), self.joke_vec(b)
        if va is None or vb is None:
            return None
        return float(np.dot(va, vb))

    # ------------------------------------------------------------------ sets
    def _build_sets(self, analyses: dict):
        self.sets, self.segments, self.order = {}, {}, {}
        self.perfs = defaultdict(list)  # joke -> [perf record] in date order
        self.back_to_back = defaultdict(list)  # (a, b) -> [{set, date, venue, b_seg}]
        for key in sorted(analyses, key=lambda k: (analyses[k].get("set", k)[:10], k)):
            a = analyses[key]
            date = key[:10]
            gset = self.nodes.get(f"set:{key}") or {}
            meta = self.metas.get(key) or {}
            venue = gset.get("venue") or meta.get("venue_guess") or "Unknown"
            s = a.get("summary", {})
            self.sets[key] = {"set": key, "date": date, "venue": venue, "title": gset.get("title") or meta.get("title"),
                              "summary": s, "set_node": f"set:{key}"}
            segs = []
            for seg in a.get("jokes", []):
                seg = dict(seg)
                seg["joke"] = self.map_joke(seg.get("joke_id"))
                seg["seg_id"] = f"seg:{key}#{seg['idx']}"
                ov = self.perf_overrides.get(f"perf:{key}@{seg.get('start_ms')}")
                if ov:
                    if ov.get("kind") in ("crowd_work", "banter"):
                        seg["status"] = ov["kind"]
                    if ov.get("rating"):
                        seg["ai_rating"] = ov["rating"]
                        seg["rating_by"] = "his"
                segs.append(seg)
            self.segments[key] = segs
            # performances: every segment of one joke in this set, merged
            by_joke = defaultdict(list)
            for seg in segs:
                if seg["joke"] and seg.get("status") in ("known", "new"):
                    by_joke[seg["joke"]].append(seg)
            set_rate = s.get("laugh_s_per_min")
            for jid, ss in by_joke.items():
                self.perfs[jid].append(self._perf(key, date, venue, set_rate, s.get("room_heat"), ss))
            # running order without banter; crowd work breaks a back-to-back
            order = []
            for seg in segs:
                if seg.get("status") == "banter":
                    continue
                j = seg["joke"] if seg.get("status") in ("known", "new") else None
                if order and j and order[-1][0] == j:
                    continue
                order.append((j, seg))
            self.order[key] = order
            for (ja, sa), (jb, sb) in zip(order, order[1:]):
                if ja and jb and ja != jb:
                    self.back_to_back[(ja, jb)].append({"set": key, "date": date, "venue": venue,
                                                        "a_seg": sa["seg_id"], "b_seg": sb["seg_id"]})
        for jid in self.perfs:
            self.perfs[jid].sort(key=lambda p: (p["date"], p["set"]))

    @staticmethod
    def _perf(key, date, venue, set_rate, room_heat, ss):
        if len(ss) == 1:
            x = ss[0]
            lspm, vs_room = x.get("laugh_s_per_min"), x.get("vs_room")
            laugh_s = (x.get("laughs") or {}).get("laugh_s", 0.0)
            window_s = (x.get("laughs") or {}).get("window_s", 0.0)
        else:
            laugh_s = sum((x.get("laughs") or {}).get("laugh_s", 0.0) for x in ss)
            window_s = sum((x.get("laughs") or {}).get("window_s", 0.0) for x in ss)
            lspm = round(laugh_s / (window_s / 60), 2) if window_s else 0.0
            vs_room = round(lspm / set_rate, 2) if set_rate else None
        main = max(ss, key=lambda x: (x["end_ms"] - x["start_ms"], -x["idx"]))
        ttfl = next((x.get("time_to_first_laugh_s") for x in ss if x.get("time_to_first_laugh_s") is not None), None)
        return {
            "joke_id": ss[0]["joke"], "set": key, "date": date, "venue": venue,
            "segments": [x["idx"] for x in ss], "seg_ids": [x["seg_id"] for x in ss], "main_seg": main["seg_id"],
            "start_ms": ss[0]["start_ms"], "end_ms": ss[-1]["end_ms"],
            "duration_s": round(sum((x["end_ms"] - x["start_ms"]) / 1000 for x in ss), 1),
            "laugh_s": round(laugh_s, 2), "window_s": round(window_s, 2), "laugh_s_per_min": lspm, "vs_room": vs_room,
            "ai_rating": max((x.get("ai_rating") or "died" for x in ss), key=lambda r: RATING_ORDER.get(r, 0)),
            "max_bars": max(((x.get("laughs") or {}).get("max_bars") or 0) for x in ss),
            "time_to_first_laugh_s": ttfl,
            "dead_air_max_s": max((x.get("dead_air_max_s") or 0) for x in ss),
            "punchlines": [dict(p, seg_id=x["seg_id"]) for x in ss for p in (x.get("punchlines") or [])],
            "script_diff": main.get("script_diff"), "version_unit": main.get("version_unit_id"),
            "set_laugh_s_per_min": set_rate, "room_heat": room_heat,
        }

    def set_keys(self) -> list[str]:
        return sorted(self.sets, key=lambda k: (self.sets[k]["date"], k))

    def seg(self, seg_id: str) -> dict | None:
        key, _, idx = seg_id[4:].rpartition("#")
        for s in self.segments.get(key, []):
            if str(s["idx"]) == idx:
                return s
        return None

    # ------------------------------------------------------------------ joke-level numbers
    def duration(self, jid: str) -> tuple[float, str]:
        """(seconds, basis): median performed duration, else written words at his taped pace."""
        ds = [p["duration_s"] for p in self.perfs.get(jid, [])]
        if ds:
            return round(median(ds), 1), f"tape ({len(ds)} night{'s' if len(ds) != 1 else ''})"
        u = self.lead_unit(jid)
        words = len(((self.nodes.get(u) or {}).get("text") or "").split()) if u else 0
        if not words:
            return 30.0, "default 0:30 (no text)"
        return round(min(300.0, words / WPM * 60), 1), f"estimate from {words} written words at {int(WPM)} wpm"

    def energy(self, jid: str):
        """Room-relative laugh density: mean vs_room over his nights (None if never taped).
        vs_room is clipped at 3.0: a near-silent room (a casino room '17, 0.76 laugh-s/min) inflates it to 7x."""
        ps = self.perfs.get(jid, [])
        v = mean([min(3.0, p["vs_room"]) for p in ps if p["vs_room"] is not None])
        return (None if v is None else r2(v)), len(ps)

    # ------------------------------------------------------------------ text + verification
    def text_of(self, node: str, field: str | None = None) -> str | None:
        if node.startswith("seg:"):
            s = self.seg(node)
            return s.get("transcript") if s else None
        n = self.nodes.get(node)
        if n is None:
            return None
        if field:
            cur = n
            for part in field.split("."):
                cur = cur.get(part) if isinstance(cur, dict) else None
            return cur if isinstance(cur, str) else None
        return n.get("text") or (n.get("definition") if n.get("type") == "label" else None)

    def verify(self, ev: dict) -> bool:
        q = ev.get("quote")
        t = self.text_of(ev.get("node", ""), ev.get("field"))
        return bool(q) and t is not None and q in t

    def ev(self, node: str, quote: str, field: str | None = None, **extra) -> dict | None:
        """An evidence item, or None if the quote is not verbatim in the node."""
        e = {"node": node, "quote": quote}
        if field:
            e["field"] = field
        e.update(extra)
        return e if self.verify(e) else None

    def edge_evidence(self, e: dict, n: int = 2) -> list[dict]:
        out = []
        for x in e.get("evidence") or []:
            node = self.map_unit(x.get("node", ""))
            ev = self.ev(node, x.get("quote", ""))
            if ev:
                out.append(ev)
            if len(out) >= n:
                break
        return out

    def seg_evidence(self, seg_id: str, n_words: int = 10, at: str = "start") -> dict | None:
        t = self.text_of(seg_id) or ""
        q = prefix_words(t, n_words)
        return self.ev(seg_id, q) if q else None


_CACHE: dict = {}


def load_corpus(comedy=None) -> Corpus:
    key = str(comedy or comedy_dir())
    if key not in _CACHE:
        _CACHE[key] = Corpus.load(comedy)
    return _CACHE[key]
