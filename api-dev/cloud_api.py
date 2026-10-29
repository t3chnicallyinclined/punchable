"""The cloud: every bit placed by meaning (2026-10-29, owner: "an AI-visualized 3D cloud map").

    GET /api/map/cloud[?cap=1600]   {nodes: [{id, short_name, title, stage, nights, heat, touched, x, y, z, cluster}],
                                     clusters: [{id, label, size, cx, cy, cz}], counts, rev}

The old map (creative/notebook.py all_map, lib/Galaxy) places bits by their LINKS: two bits are near when a segue,
a tag, a shared premise or the merge judge connects them, and a typed jot with no link falls in an "Unlinked" patch.
This one places them by what they SAY. Each bit gets one vector: the mean of its versions' bge-small vectors
(unit.embedding, loaded with the data) or, for a typed idea with no version, its own (joke.embedding, filled by
search_api's sweeper). Those 384 dimensions are projected to three with PCA (the three directions of greatest
spread; the first is x, the second y, the third z) and each axis is scaled to [-1, 1] at the 2nd/98th percentile so
one outlier cannot flatten the rest. Clusters are k-means in the full 384-d space (cosine, since the vectors are
unit length), named by the words most over-represented in their titles. Nothing here is a verdict: a place to look.

numpy only (the API image has it); 1,600 bits project in well under a second; cached per data revision + vector
count for five minutes (the sweeper may add vectors between).
"""
from __future__ import annotations

import math
import re
import sys
import threading
import time
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
DESK = HERE.parent
for p in (DESK / "db", DESK / "tools" / "explorer", DESK):
    if str(p) not in sys.path:
        sys.path.insert(0, str(p))
import sdb  # noqa: E402

CAP_DEFAULT, CAP_MAX = 1600, 2000
CACHE_S = 300.0
STOP = frozenset("""a an the and or but so to of in on at for with is it's its i i'm im you your he she they we me my
our us them his her their this that these those was were be been being are am do does did have has had not no yes
if then than when where what who why how all any some one two about into over under out up down off from by as
just like get got go going went come came say said says make made thing things really very also too again more
most much many can could would should will shall may might must there here now never always every ever still
even way yeah ok okay oh uh um dont don't cant can't didnt didn't wont won't thats that's youre you're ive i've""".split())
TITLE_STOP = STOP | frozenset("bit joke jokes tag tags idea ideas story version part new old note notes set list roast".split())
_cache: dict = {}
_lock = threading.Lock()


# ----------------------------------------------------------------------------------------------- the maths
def project(M, dims: int = 3):
    """PCA to `dims` axes, each scaled to [-1, 1] at the 2nd/98th percentile (clamped). Signs are fixed so the
    largest-loading dimension of each axis is positive: the same data always lands the same way up."""
    import numpy as np
    X = np.asarray(M, dtype=np.float32)
    n = X.shape[0]
    if n == 0:
        return np.zeros((0, dims), np.float32)
    if n == 1:
        return np.zeros((1, dims), np.float32)
    Xc = X - X.mean(axis=0, keepdims=True)
    _, _, vt = np.linalg.svd(Xc, full_matrices=False)
    comps = vt[:dims]
    for i in range(comps.shape[0]):
        j = int(np.argmax(np.abs(comps[i])))
        if comps[i, j] < 0:
            comps[i] = -comps[i]
    P = Xc @ comps.T
    if P.shape[1] < dims:
        P = np.concatenate([P, np.zeros((n, dims - P.shape[1]), np.float32)], axis=1)
    out = np.zeros_like(P)
    for a in range(dims):
        col = P[:, a]
        lo, hi = np.percentile(col, 2), np.percentile(col, 98)
        if hi - lo < 1e-9:
            continue
        out[:, a] = np.clip((col - lo) / (hi - lo) * 2 - 1, -1, 1)
    return out.astype(np.float32)


def kmeans(M, k: int, iters: int = 25, seed: int = 7):
    """Spherical k-means (the rows are unit vectors; cosine = dot). k-means++ seeding with a fixed seed, so the
    same data gives the same clusters. Returns (labels, centres)."""
    import numpy as np
    X = np.asarray(M, dtype=np.float32)
    n = X.shape[0]
    k = max(1, min(k, n))
    rng = np.random.default_rng(seed)
    centres = [X[int(rng.integers(n))]]
    for _ in range(1, k):
        sims = np.max(np.stack([X @ c for c in centres]), axis=0)
        d2 = np.clip(1.0 - sims, 0, None) ** 2
        s = d2.sum()
        idx = int(rng.integers(n)) if s <= 0 else int(rng.choice(n, p=d2 / s))
        centres.append(X[idx])
    Cm = np.stack(centres)
    labels = np.zeros(n, dtype=np.int64)
    for _ in range(iters):
        new = np.argmax(X @ Cm.T, axis=1)
        if np.array_equal(new, labels) and _ > 0:
            break
        labels = new
        for c in range(k):
            m = X[labels == c]
            if len(m):
                v = m.mean(axis=0)
                nrm = float(np.linalg.norm(v)) or 1.0
                Cm[c] = v / nrm
    return labels, Cm


def pick_k(n: int) -> int:
    return int(max(1, min(14, max(6, round(math.sqrt(n / 2.0)))))) if n >= 12 else max(1, min(3, n))


def _words(title: str) -> list[str]:
    return [w for w in re.findall(r"[a-z][a-z']+", (title or "").lower()) if w not in TITLE_STOP and len(w) > 2]


def label_clusters(titles_by_cluster: dict[int, list[str]], top: int = 3) -> dict[int, str]:
    """The two or three words most over-represented in a cluster's titles (rate in the cluster over rate overall,
    at least two occurrences), joined with " · ". A cluster with nothing distinctive is named by its commonest word;
    an empty one "—"."""
    total = Counter()
    per: dict[int, Counter] = {}
    for cid, titles in titles_by_cluster.items():
        c = Counter()
        for t in titles:
            c.update(set(_words(t)))
        per[cid] = c
        total.update(c)
    n_all = sum(len(t) for t in titles_by_cluster.values()) or 1
    out = {}
    for cid, c in per.items():
        n = len(titles_by_cluster[cid]) or 1
        scored = []
        for w, cnt in c.items():
            if cnt < 2 and n > 3:
                continue
            lift = (cnt / n) / (total[w] / n_all)
            scored.append((lift * math.log1p(cnt), w))
        scored.sort(reverse=True)
        words = [w for _, w in scored[:top]]
        if not words and c:
            words = [c.most_common(1)[0][0]]
        out[cid] = " · ".join(words) if words else "—"
    return out


# --------------------------------------------------------------------------------------------- the payload
def joke_vectors(r: sdb.Rpc, v) -> dict[str, list[float]]:
    """One vector per joke: the mean of its (non-hidden) units' vectors, else the joke's own. Jokes with neither are
    left out (a typed idea with no text)."""
    import numpy as np
    rows = r.last("SELECT id, embedding FROM unit WHERE embedding != NONE AND hidden != true;")
    uvec = {sdb.rid_str(x["id"]): np.asarray(x["embedding"], dtype=np.float32) for x in rows}
    out: dict[str, list[float]] = {}
    for jid, units in v.joke_units.items():
        vs = [uvec[u] for u, _ in units if u in uvec]
        if vs:
            m = np.mean(np.stack(vs), axis=0)
            nrm = float(np.linalg.norm(m)) or 1.0
            out[jid] = (m / nrm).tolist()
    rows = r.last("SELECT id, embedding FROM joke WHERE embedding != NONE;")
    for x in rows:
        jid = sdb.rid_str(x["id"])
        if jid not in out and jid in v.nodes:
            out[jid] = [float(t) for t in x["embedding"]]
    return out


def build(r: sdb.Rpc, a, cap: int = CAP_DEFAULT) -> dict:
    """The cloud for the app. `a` is the AppIndex (summaries() for names, stage, nights, heat, touched; .v the view)."""
    import numpy as np
    v = a.v
    vecs = joke_vectors(r, v)
    items = [x for x in a.summaries() if x["id"] in vecs]
    # the cap keeps the newest work: what he touched last, then nights
    items.sort(key=lambda x: ((x.get("touched") or x.get("last_seen") or "")[:19], x.get("nights") or 0), reverse=True)
    items = items[:cap]
    ids = [x["id"] for x in items]
    if not ids:
        return {"nodes": [], "clusters": [], "counts": {"jokes": len(a.summaries()), "placed": 0, "with_vectors": len(vecs)}}
    M = np.stack([np.asarray(vecs[j], dtype=np.float32) for j in ids])
    P = project(M, 3)
    labels, _ = kmeans(M, pick_k(len(ids)))
    titles: dict[int, list[str]] = {}
    for x, lab in zip(items, labels):
        titles.setdefault(int(lab), []).append(x.get("title") or "")
    names = label_clusters(titles)
    nodes = []
    for x, p, lab in zip(items, P, labels):
        nodes.append({"id": x["id"], "short_name": x.get("short_name") or x.get("short_title") or x["title"], "title": x["title"],
                      "stage": x.get("stage"), "nights": x.get("nights") or 0, "heat": x.get("avg_vs_room"), "touched": x.get("touched"),
                      "x": round(float(p[0]), 4), "y": round(float(p[1]), 4), "z": round(float(p[2]), 4), "cluster": int(lab)})
    clusters = []
    for cid in sorted(titles):
        pts = [n for n in nodes if n["cluster"] == cid]
        clusters.append({"id": cid, "label": names.get(cid, "—"), "size": len(pts),
                         "cx": round(sum(n["x"] for n in pts) / len(pts), 4), "cy": round(sum(n["y"] for n in pts) / len(pts), 4),
                         "cz": round(sum(n["z"] for n in pts) / len(pts), 4)})
    clusters.sort(key=lambda c: -c["size"])
    return {"nodes": nodes, "clusters": clusters,
            "counts": {"jokes": len(a.summaries()), "placed": len(nodes), "with_vectors": len(vecs), "k": len(clusters)}}


def cloud_payload(kb, a, cap: int) -> tuple[int, dict]:
    """GET /api/map/cloud. SurrealDB only (the vectors live there); the files backend answers 501 and the app shows
    the links map instead."""
    if getattr(kb, "backend", None) != "surreal" or not hasattr(kb, "rpc"):
        return 501, {"error": "the cloud needs the database's vectors; this backend has none"}
    rev = getattr(kb, "rev", None)
    with _lock:
        hit = _cache.get("v")
        if hit and hit[0] == (rev, cap) and time.time() - hit[2] < CACHE_S:
            return 200, hit[1]
    with kb.lock:
        out = build(kb.rpc, a, cap)
    out["rev"] = rev
    with _lock:
        _cache["v"] = ((rev, cap), out, time.time())
    return 200, out
