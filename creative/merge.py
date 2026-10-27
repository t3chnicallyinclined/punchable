"""Same-idea groups, merges and auto-attach (the comic, 2026-10-26: "we should be able to consolidate jokes/ideas too
meaning they merge into one unit/doc, no reason to always have 1000's of google docs").

    python -m creative merge groups [--examples 10] [--json]
    python -m creative merge for <joke_id>
    python -m creative merge match "<text>"

Pure functions over a Corpus. Nothing here writes: the API (api-dev/merge_api.py) turns a plan into decision lines
and store events, and gsync does the Drive side. See CREATIVE.md section 6.
"""
from __future__ import annotations

import math
import re
from collections import Counter, defaultdict

import numpy as np

from .data import Corpus

# ---------------------------------------------------------------------------------------------------------- tuning
UNIT_CAND = 0.72        # unit-unit cosine that makes a pair worth scoring at all
TOK = re.compile(r"[a-z0-9]+(?:'[a-z]+)?")
STOP = frozenset("""a an the and or but so to of in on at for with is it's its i i'm im you your he she they we me my
this that was were be been are am do did does have has had just like if then there what when how who why not no
yeah um uh oh ok okay all one get got go going gonna know about from as by up out can can't don't would could
should will his her him them their our us it them than too very really also more some any into over about
said says say thing things people time good bad want make made see now well even ever every back still
dont cant im its thats whats""".split())
TITLE_STOP = STOP | frozenset("bit joke tag tags idea story version part new old note notes set list roast".split())
STAGE_RANK = {"working": 4, "testing": 3, "to_try": 2, "idea": 1, "retired": 0}


def _norm(t: str | None) -> str:
    return (t or "").lower().replace("’", "'").replace("‘", "'").replace("�", "'")


def _toks(t: str | None) -> list[str]:
    return TOK.findall(_norm(t))


def _grams(ts: list[str], k: int) -> set[tuple]:
    return {tuple(ts[i:i + k]) for i in range(len(ts) - k + 1)}


def _content(ts) -> set[str]:
    return {w for w in ts if w not in STOP and len(w) > 2}


def _ref(rows: np.ndarray) -> np.ndarray:
    """Each row's own "usually this close" level: its k-th highest similarity, k = 1% of the units (at least 10).
    On the real corpus (2,200 units) that is the 99th percentile; on a small one it never lands on the few true
    matches themselves."""
    n = rows.shape[1]
    k = max(10, n // 100)
    if n <= k:
        return np.full(rows.shape[0], 0.99, dtype=np.float32)
    return -np.partition(-rows, k - 1, axis=1)[:, k - 1]


def _stem(w: str) -> str:
    for suf in ("ing", "ers", "er", "es", "ed", "s"):
        if len(w) > len(suf) + 3 and w.endswith(suf):
            return w[: -len(suf)]
    return w


class Index:
    """Everything pair scoring needs, built once per corpus (cached on it as `_merge_idx`)."""

    def __init__(self, c: Corpus):
        self.c = c
        self.jokes = sorted(c.jokes)
        # texts: every unit of a joke; a joke with no unit (an app or Google Docs idea) is its summary
        self.units: dict[str, list[str]] = {}
        self.text: dict[str, str] = {}
        for j in self.jokes:
            us = sorted(u for u in c.joke_units.get(j, []) if not c.nodes[u].get("hidden"))
            self.units[j] = us
        self.utoks = {}
        for j in self.jokes:
            for u in self.units[j]:
                self.utoks[u] = _toks(c.nodes[u].get("text"))
            if not self.units[j]:
                self.utoks[f"summary:{j}"] = _toks(c.jokes[j].get("summary"))
        self.parts = {j: (self.units[j] or [f"summary:{j}"]) for j in self.jokes}
        n = max(1, len(self.utoks))
        self.n_parts = n
        self.ucont = {u: _content(ts) for u, ts in self.utoks.items()}
        self.ug3 = {u: {g for g in _grams(ts, 3) if sum(w not in STOP for w in g) >= 2} for u, ts in self.utoks.items()}
        self.ug2 = {u: {g for g in _grams(ts, 2) if g[0] not in STOP and g[1] not in STOP} for u, ts in self.utoks.items()}
        self.df_w = Counter(w for s in self.ucont.values() for w in s)
        self.df_3 = Counter(g for s in self.ug3.values() for g in s)
        self.df_2 = Counter(g for s in self.ug2.values() for g in s)
        self.idf = {w: math.log(n / (1 + d)) for w, d in self.df_w.items()}
        # titles: the bit's name (the catalog's or his)
        self.ttoks = {j: {_stem(w) for w in _toks(c.jokes[j].get("title")) if w not in TITLE_STOP and len(w) > 2}
                      for j in self.jokes}
        self.df_t = Counter(w for s in self.ttoks.values() for w in s)
        # unit vectors (bge-small, read-only cache) with each unit's own similarity band
        self.vec_units = [u for j in self.jokes for u in self.units[j] if c.vectors.get(u) is not None]
        self.urow = {u: i for i, u in enumerate(self.vec_units)}
        self.ujoke = {u: j for j in self.jokes for u in self.units[j]}
        if self.vec_units:
            m = np.stack([np.asarray(c.vectors.get(u), dtype=np.float32) for u in self.vec_units])
            m /= np.maximum(np.linalg.norm(m, axis=1, keepdims=True), 1e-9)
            self.M = m
            s = m @ m.T
            jk = np.array([self.ujoke[u] for u in self.vec_units])
            s[jk[:, None] == jk[None, :]] = -1.0
            self.S = s
            self.p99 = _ref(s)
        else:
            self.M, self.S, self.p99 = None, None, None
        # edges the KB already holds between two jokes (same_premise), and his rejects of them
        self.premise: dict[tuple, dict] = {}
        for e in c.edges:
            if e.get("kind") == "same_premise":
                a, b = c.unit_joke.get(e["src"], e["src"]), c.unit_joke.get(e["dst"], e["dst"])
                if a != b and a in c.jokes and b in c.jokes:
                    self.premise[tuple(sorted((a, b)))] = e

    # ------------------------------------------------------------------ one pair
    def phrases(self, a: str, b: str) -> list[str]:
        """Distinctive word runs both jokes use (a 3-word run in at most 3 texts, or a 2-content-word run in at most
        3), verbatim lower-case. The same bit rewritten keeps its phrases; two bits on one topic rarely share them."""
        out = set()
        for u in self.parts[a]:
            for v in self.parts[b]:
                for g in self.ug3[u] & self.ug3[v]:
                    if self.df_3[g] <= 3:
                        out.add(" ".join(g))
                for g in self.ug2[u] & self.ug2[v]:
                    if self.df_2[g] <= 3:
                        out.add(" ".join(g))
        # a 2-gram inside a shared 3-gram is the same evidence once
        keep = [p for p in out if not any(p != q and p in q for q in out)]
        return sorted(keep)

    def rare_words(self, a: str, b: str, max_df: int = 4) -> list[str]:
        wa = set().union(*(self.ucont[u] for u in self.parts[a]))
        wb = set().union(*(self.ucont[u] for u in self.parts[b]))
        return sorted((w for w in wa & wb if self.df_w[w] <= max_df), key=lambda w: (self.df_w[w], w))

    def title_words(self, a: str, b: str, max_df: int = 12) -> list[str]:
        return sorted(w for w in self.ttoks[a] & self.ttoks[b] if self.df_t[w] <= max_df)

    def best_units(self, a: str, b: str):
        """(cosine, band, unit_a, unit_b) of the closest pair of units. band = how far above each unit's own 99th
        percentile the pair sits (the smaller of the two), so one generic unit can't look close to everything."""
        if self.S is None:
            return None
        ra = [self.urow[u] for u in self.parts[a] if u in self.urow]
        rb = [self.urow[u] for u in self.parts[b] if u in self.urow]
        if not ra or not rb:
            return None
        sub = self.S[np.ix_(ra, rb)]
        i, k = np.unravel_index(int(np.argmax(sub)), sub.shape)
        cs = float(sub[i, k])
        band = min(cs - float(self.p99[ra[i]]), cs - float(self.p99[rb[k]]))
        return cs, band, self.vec_units[ra[i]], self.vec_units[rb[k]]

    def containment(self, u: str, v: str) -> float:
        a, b = self.ucont.get(u, set()), self.ucont.get(v, set())
        small = min(len(a), len(b))
        return len(a & b) / small if small else 0.0

    def candidates(self) -> set[tuple]:
        pairs = set()
        if self.S is not None:
            ii, kk = np.where(np.triu(self.S, 1) >= UNIT_CAND)
            for i, k in zip(ii.tolist(), kk.tolist()):
                a, b = self.ujoke[self.vec_units[i]], self.ujoke[self.vec_units[k]]
                if a != b:
                    pairs.add(tuple(sorted((a, b))))
        pairs |= set(self.premise)
        by_t = defaultdict(list)
        for j, ws in self.ttoks.items():
            for w in ws:
                if self.df_t[w] <= 12:
                    by_t[w].append(j)
        for js in by_t.values():
            for x in range(len(js)):
                for y in range(x + 1, len(js)):
                    pairs.add(tuple(sorted((js[x], js[y]))))
        return pairs

    def features(self, a: str, b: str) -> dict:
        bu = self.best_units(a, b)
        f = {"a": a, "b": b, "cos": None, "band": None, "units": None, "contain": 0.0,
             "phrases": self.phrases(a, b), "rare": self.rare_words(a, b), "title": self.title_words(a, b),
             "premise": None, "n_a": len(self.units[a]), "n_b": len(self.units[b])}
        if bu:
            f["cos"], f["band"], ua, ub = round(bu[0], 3), round(bu[1], 3), bu[2], bu[3]
            f["units"] = [ua, ub]
            f["contain"] = round(self.containment(ua, ub), 2)
        e = self.premise.get(tuple(sorted((a, b))))
        if e is not None:
            f["premise"] = {"edge": e["id"], "confidence": e.get("confidence"), "source": self.c.source_of(e),
                            "reason": e.get("reason")}
        return f


def index(c: Corpus) -> Index:
    idx = getattr(c, "_merge_idx", None)
    if idx is None:
        idx = Index(c)
        c._merge_idx = idx
    return idx


# ---------------------------------------------------------------------------------------------------------- scoring
def pair_score(f: dict) -> float:
    """How likely two jokes are one bit, before any judge: 0..1. Text closeness against each unit's own band, shared
    distinctive phrases, shared rare words, a shared rare title word, and a same_premise edge the KB already holds."""
    s = 0.0
    if f.get("band") is not None:
        s += max(0.0, min(0.45, f["band"] * 3.0))
    s += 0.08 * min(5, len(f.get("phrases") or []))
    s += 0.04 * min(4, len(f.get("rare") or []))
    s += 0.12 * min(2, len(f.get("title") or []))
    p = f.get("premise")
    if p:
        s += 0.5 * float(p.get("confidence") or 0.5) if p.get("source") != "his" else 0.6
    return round(min(1.0, s), 3)


JUDGE_FLOOR = 0.25       # pairs below this are never even asked about
SAME_MIN_CONF = 0.80     # the judge's own confidence a "same" needs to count
JUDGE_BATCH = 15
JUDGE_SCHEMA = {
    "type": "object",
    "properties": {"pairs": {"type": "array", "items": {
        "type": "object",
        "properties": {"id": {"type": "string"}, "same": {"type": "boolean"}, "confidence": {"type": "number"},
                       "why": {"type": "string"}},
        "required": ["id", "same", "confidence", "why"], "additionalProperties": False}}},
    "required": ["pairs"], "additionalProperties": False}
JUDGE_SYSTEM = (
    "You sort one stand-up comedian's own notes. For each pair, decide if A and B are THE SAME JOKE: the same premise "
    "taken the same way, so a comic would keep them as one bit with versions. Same joke: a rewrite or later version, a "
    "one-line jot of the bit, a longer draft of it, the same story told twice, the same argument with the same angle. "
    "NOT the same joke: the same broad topic (dating, racism, my niece, Facebook, Uber) with a different premise or angle; "
    "two different jokes that happen to share a line; roasts of two different people; a set list or idea dump that "
    "mentions it; one being the other side of an argument. When unsure, answer same=false. confidence is how sure you "
    "are of your answer (0-1). why is one plain sentence a comic would understand, naming the shared premise or the "
    "difference.")


def _excerpt(c: Corpus, ix: Index, j: str, unit: str | None, n: int = 900) -> str:
    t = c.nodes[unit].get("text") if unit else None
    t = t or c.jokes[j].get("summary") or c.jokes[j].get("title") or ""
    t = " ".join(t.split())
    return t[:n] + ("..." if len(t) > n else "")


def judge_key(c: Corpus, ix: Index, f: dict) -> str:
    """Cache key: the two excerpts as shown to the judge (order-free), so an edit to either text asks again."""
    import hashlib
    ua, ub = f.get("units") or (None, None)
    xs = sorted([c.title(f["a"]) + "\n" + _excerpt(c, ix, f["a"], ua), c.title(f["b"]) + "\n" + _excerpt(c, ix, f["b"], ub)])
    return hashlib.sha1("\n--\n".join(xs).encode("utf-8")).hexdigest()[:16]


def judge(c: Corpus, feats: list[dict], cache: dict, complete, log=None) -> dict:
    """Ask the routed model (task `merge_judge`, offline) about every pair not in `cache`; returns the cache, updated.
    A provider error leaves those pairs unjudged (they stay separate)."""
    ix = index(c)
    todo = [f for f in feats if judge_key(c, ix, f) not in cache]
    for i in range(0, len(todo), JUDGE_BATCH):
        batch = todo[i:i + JUDGE_BATCH]
        lines = []
        for k, f in enumerate(batch):
            ua, ub = f.get("units") or (None, None)
            lines.append(f"PAIR p{k + 1}\nA ({c.title(f['a'])}): {_excerpt(c, ix, f['a'], ua)}\n"
                         f"B ({c.title(f['b'])}): {_excerpt(c, ix, f['b'], ub)}")
        msg = "Decide each pair.\n\n" + "\n\n".join(lines)
        try:
            r = complete("merge_judge", [{"role": "system", "content": JUDGE_SYSTEM}, {"role": "user", "content": msg}],
                         JUDGE_SCHEMA)
        except Exception as e:  # noqa: BLE001  never guess: unjudged pairs stay separate
            if log:
                log(f"judge batch {i // JUDGE_BATCH}: {type(e).__name__}: {str(e)[:120]}")
            continue
        got = {p.get("id"): p for p in (r.data or {}).get("pairs") or []}
        for k, f in enumerate(batch):
            p = got.get(f"p{k + 1}")
            if p is None:
                continue
            cache[judge_key(c, ix, f)] = {"same": bool(p.get("same")), "confidence": float(p.get("confidence") or 0),
                                          "why": str(p.get("why") or "")[:240], "model": getattr(r, "model", None),
                                          "a": f["a"], "b": f["b"]}
        if log:
            log(f"judged {min(i + JUDGE_BATCH, len(todo))}/{len(todo)}")
    return cache


def _full(c: Corpus, ix: Index, j: str, n: int = 2500) -> str:
    """Every written version of a joke, oldest first, for the second look."""
    us = sorted(ix.units.get(j, []), key=lambda u: (c.nodes[u].get("date") or "", u))
    parts = [f"[{c.nodes[u].get('date') or 'undated'}] " + " ".join((c.nodes[u].get("text") or "").split()) for u in us]
    t = " / ".join(parts) or " ".join((c.jokes[j].get("summary") or c.jokes[j].get("title") or "").split())
    return t[:n] + ("..." if len(t) > n else "")


SECOND_LO = 0.55
SECOND_BATCH = 5


def second_opinion(c: Corpus, cache: dict, complete, log=None) -> dict:
    """Borderline "same" answers (SECOND_LO <= confidence < SAME_MIN_CONF) are asked again, a few at a time, with every
    written version of both jokes. A pair counts if either look says same at >= SAME_MIN_CONF."""
    ix = index(c)
    todo = [(k, v) for k, v in sorted(cache.items()) if v.get("same") and SECOND_LO <= v.get("confidence", 0) < SAME_MIN_CONF
            and "second" not in v and v.get("a") in c.jokes and v.get("b") in c.jokes]
    for i in range(0, len(todo), SECOND_BATCH):
        batch = todo[i:i + SECOND_BATCH]
        lines = [f"PAIR p{n + 1}\nA ({c.title(v['a'])}): {_full(c, ix, v['a'])}\nB ({c.title(v['b'])}): {_full(c, ix, v['b'])}"
                 for n, (_, v) in enumerate(batch)]
        msg = ("Decide each pair. Each side shows every written version of that joke, oldest first, separated by ' / '. "
               "Look closely: would he keep these as ONE bit?\n\n" + "\n\n".join(lines))
        try:
            r = complete("merge_judge", [{"role": "system", "content": JUDGE_SYSTEM}, {"role": "user", "content": msg}],
                         JUDGE_SCHEMA)
        except Exception as e:  # noqa: BLE001
            if log:
                log(f"second look {i // SECOND_BATCH}: {type(e).__name__}: {str(e)[:120]}")
            continue
        got = {p.get("id"): p for p in (r.data or {}).get("pairs") or []}
        for n, (k, v) in enumerate(batch):
            p = got.get(f"p{n + 1}")
            if p is not None:
                v["second"] = {"same": bool(p.get("same")), "confidence": float(p.get("confidence") or 0),
                               "why": str(p.get("why") or "")[:240]}
        if log:
            log(f"second look {min(i + SECOND_BATCH, len(todo))}/{len(todo)}")
    return cache


def verdict(v: dict | None) -> dict | None:
    """The answer that counts for a pair: the first look if it is confident, else a confident second look."""
    if not v:
        return None
    if v.get("same") and v.get("confidence", 0) >= SAME_MIN_CONF:
        return v
    s2 = v.get("second")
    if s2 and s2.get("same") and s2.get("confidence", 0) >= SAME_MIN_CONF:
        return {**v, **s2}
    return None


# ---------------------------------------------------------------------------------------------------------- groups
def _dates(c: Corpus, ix: Index, j: str) -> list[str]:
    ds = {c.nodes[u].get("date") for u in ix.units.get(j, []) if c.nodes[u].get("date")}
    ds |= {p["date"] for p in c.perfs.get(j, [])}
    n = c.jokes.get(j) or {}
    if not ds:
        ds |= {d for d in (n.get("first_seen"), n.get("last_seen")) if d}
    return sorted(d[:10] for d in ds)


def _docs(c: Corpus, ix: Index, j: str) -> list[dict]:
    out = {}
    for u in ix.units.get(j, []):
        n = c.nodes[u]
        if n.get("doc_id"):
            d = out.setdefault(n["doc_id"], {"doc_id": n["doc_id"], "title": n.get("doc_title"), "date": n.get("date")})
            if n.get("date") and (not d["date"] or n["date"] < d["date"]):
                d["date"] = n["date"]
    return sorted(out.values(), key=lambda d: (d["date"] or "", d["doc_id"]))


def member(c: Corpus, ix: Index, j: str) -> dict:
    ds = _dates(c, ix, j)
    st, st_src = c.stage(j)
    return {"joke_id": j, "title": c.title(j), "stage": st, "stage_source": st_src,
            "first_date": ds[0] if ds else None, "last_date": ds[-1] if ds else None, "dates": ds,
            "nights": len(c.perfs.get(j, [])), "units": len(ix.units.get(j, [])), "docs": _docs(c, ix, j),
            "yours": j in c.his_nodes}


def main_key(m: dict):
    """Best version first: proven on stage (nights), then stage, then his own edits, then the most recent date."""
    recent = tuple(-ord(ch) for ch in (m["last_date"] or ""))
    return (-m["nights"], -STAGE_RANK.get(m["stage"] or "", 0), not m["yours"], recent, -m["units"], m["joke_id"])


def main_why(m: dict, others: list[dict]) -> str:
    bits = []
    if m["nights"] and m["nights"] >= max([o["nights"] for o in others] + [0]):
        bits.append(f"performed on {m['nights']} night{'s' if m['nights'] != 1 else ''} (the most)")
    if STAGE_RANK.get(m["stage"] or "", 0) > max([STAGE_RANK.get(o["stage"] or "", 0) for o in others] + [0]):
        bits.append(f"furthest along ({m['stage']})")
    if m["yours"] and not any(o["yours"] for o in others):
        bits.append("you have edited it")
    if m["last_date"] and all((o["last_date"] or "") <= m["last_date"] for o in others):
        bits.append(f"the most recent version ({m['last_date']})")
    return "; ".join(bits) or "the fullest version"


def accepted_pairs(c: Corpus, judgements: dict, dismissed=frozenset(), exclude=frozenset()) -> dict:
    """pair -> evidence for every pair counted as the same idea: the judge said same at >= SAME_MIN_CONF on a
    candidate pair, or the comic confirmed a same_premise edge. Pairs he dismissed never come back."""
    ix = index(c)
    out = {}
    for a, b in sorted(ix.candidates()):
        if frozenset((a, b)) in dismissed or a in exclude or b in exclude:
            continue
        f = ix.features(a, b)
        sc = pair_score(f)
        p = f.get("premise")
        his = bool(p and p.get("source") == "his")
        v = verdict(judgements.get(judge_key(c, ix, f))) if sc >= JUDGE_FLOOR else None
        if his or v:
            out[(a, b)] = {"score": sc, "feat": f, "verdict": v, "his": his}
    return out


def _reasons(c: Corpus, prs: list[dict]) -> list[str]:
    out = []
    for p in prs:
        v, f = p.get("verdict"), p["feat"]
        if p.get("his"):
            out.append(f"You linked {c.title(f['a'])} and {c.title(f['b'])} as the same premise.")
        if v and v.get("why"):
            out.append(v["why"])
        if f.get("phrases"):
            out.append("Same words in both: " + ", ".join(f"\"{x}\"" for x in f["phrases"][:3]))
    return list(dict.fromkeys(out))[:5]


def build_groups(c: Corpus, judgements: dict, dismissed=frozenset(), max_size: int = 6, exclude=frozenset()) -> list[dict]:
    from .util import sha8
    ix = index(c)
    acc = accepted_pairs(c, judgements, dismissed, exclude)
    nbr = defaultdict(set)
    for a, b in acc:
        nbr[a].add(b)
        nbr[b].add(a)
    left, groups = set(nbr), []
    while left:
        start = min(left)
        comp, todo = {start}, [start]
        while todo:
            x = todo.pop()
            for y in nbr[x] & left:
                if y not in comp:
                    comp.add(y)
                    todo.append(y)
        ms = {j: member(c, ix, j) for j in comp}
        main = min(comp, key=lambda j: main_key(ms[j]))
        # no chaining: a member must be judged the same idea as the main itself
        keep = sorted({main} | (nbr[main] & comp), key=lambda j: main_key(ms[j]))[:max_size]
        if len(keep) < 2:
            left.discard(main)
            continue
        left -= set(keep)
        prs = [dict(v, a=a, b=b) for (a, b), v in acc.items() if a in keep and b in keep]
        mem = [ms[j] for j in keep]
        conf = [p["verdict"]["confidence"] for p in prs if p.get("verdict")] + [1.0 for p in prs if p["his"] and not p.get("verdict")]
        groups.append({
            "id": "grp:" + sha8(keep), "main": main, "main_title": c.title(main),
            "main_why": main_why(ms[main], mem[1:]), "members": mem, "reasons": _reasons(c, prs),
            "confidence": round(min(conf), 2) if conf else 1.0,
            "pairs": [{"a": p["a"], "b": p["b"], "score": p["score"],
                       "judge": {k: p["verdict"][k] for k in ("same", "confidence", "why")} if p.get("verdict") else None,
                       "his": p["his"], "phrases": p["feat"]["phrases"][:5]} for p in prs],
            "source": "his" if prs and all(p["his"] for p in prs) else "inferred"})
    groups.sort(key=lambda g: (-len(g["members"]), -g["confidence"], g["id"]))
    return groups


def counts(c: Corpus, groups: list[dict]) -> dict:
    ix = index(c)
    docs_all = {c.nodes[u].get("doc_id") for j in ix.jokes for u in ix.units[j] if c.nodes[u].get("doc_id")}
    in_g = {m["joke_id"] for g in groups for m in g["members"]}
    docs_g = {d["doc_id"] for g in groups for m in g["members"] for d in m["docs"]}
    return {"jokes": len(ix.jokes), "docs": len(docs_all), "groups": len(groups), "jokes_in_groups": len(in_g),
            "docs_in_groups": len(docs_g), "jokes_after_merge": len(ix.jokes) - len(in_g) + len(groups),
            "docs_after_merge": len(docs_all) - len(docs_g) + len(groups)}


RULE = (f"Same idea = Claude judged the pair the same bit at >= {SAME_MIN_CONF} confidence (only pairs that are already "
        f"close: unit text, shared phrases, rare words, title words or a same_premise edge), or you linked them. Every "
        f"member is judged against the main itself, so ideas never chain together.")


def find_groups(c: Corpus, judgements: dict, dismissed=frozenset(), exclude=frozenset()) -> dict:
    gs = build_groups(c, judgements, dismissed, exclude=exclude)
    return {"counts": counts(c, gs), "groups": gs, "rule": RULE, "judged_pairs": len(judgements)}


def candidates_for(c: Corpus, jid: str, judgements: dict, dismissed=frozenset(), cap: int = 5,
                   groups: list | None = None) -> dict:
    """The group this joke is in (if any) plus its closest other jokes, each with the judge's verdict when there is
    one. Only `group` is a merge suggestion; `others` are for a manual merge."""
    ix = index(c)
    if jid not in c.jokes:
        return {"joke_id": jid, "group": None, "others": []}
    gs = build_groups(c, judgements, dismissed) if groups is None else groups
    g = next((g for g in gs if any(m["joke_id"] == jid for m in g["members"])), None)
    in_g = {m["joke_id"] for m in g["members"]} if g else {jid}
    rows = []
    for a, b in ix.candidates():
        if jid not in (a, b):
            continue
        o = b if a == jid else a
        if o in in_g or frozenset((a, b)) in dismissed:
            continue
        f = ix.features(a, b)
        sc = pair_score(f)
        if sc < 0.2:
            continue
        v0 = judgements.get(judge_key(c, ix, f))
        v = verdict(v0) or v0
        rows.append({"joke_id": o, "title": c.title(o), "stage": c.stage(o)[0], "score": sc,
                     "judge": {k: v[k] for k in ("same", "confidence", "why")} if v else None,
                     "phrases": f["phrases"][:3], "last_date": (_dates(c, ix, o) or [None])[-1]})
    rows.sort(key=lambda r: (not (r["judge"] or {}).get("same", False), -r["score"], r["joke_id"]))
    return {"joke_id": jid, "group": g, "others": rows[:cap]}


# ---------------------------------------------------------------------------------------------------------- attach
# A new jot attaches only when it is clearly one existing bit: close in meaning to one unit (against both the jot's and
# that unit's own similarity bands), with that bit's own words in it, and clearly closer than any other bit. Anything
# less stays a new idea. No model call: this runs inside POST /api/ideas.
ATTACH_BAND = 0.10       # cosine above both "usually this close" levels (0.06 attached 6 of 1,171 old jots, 3 disputed)
ATTACH_MARGIN = 0.04     # ahead of the next bit by this much cosine (when that bit also has word support)
ATTACH_MIN_WORDS = 4     # content words a jot needs before it can be placed at all


def _jot_support(ix: Index, ts: list[str], j: str) -> tuple[list[str], float]:
    """Shared distinctive phrases between a jot and joke j, and how much of the jot's content words the closest unit
    holds."""
    g3 = {g for g in _grams(ts, 3) if sum(w not in STOP for w in g) >= 2}
    g2 = {g for g in _grams(ts, 2) if g[0] not in STOP and g[1] not in STOP}
    cont = _content(ts)
    phr, best = set(), 0.0
    for u in ix.parts[j]:
        for g in g3 & ix.ug3[u]:
            if ix.df_3[g] <= 3:
                phr.add(" ".join(g))
        for g in g2 & ix.ug2[u]:
            if ix.df_2[g] <= 3:
                phr.add(" ".join(g))
        if cont:
            best = max(best, len(cont & ix.ucont[u]) / len(cont))
    keep = sorted(p for p in phr if not any(p != q and p in q for q in phr))
    return keep, round(best, 2)


def match_text(c: Corpus, text: str, vec=None, exclude=(), group_of: dict | None = None) -> dict:
    """Where a new jot belongs. Returns {decision: attach|ambiguous|new, joke_id?, title?, score, reasons[], top[]}.
    `vec` is the jot's bge-small vector (db/embedder.py); without it only an unmistakable word match can attach.
    `group_of` maps a joke to its same-idea group's main: two close jokes in one group are one bit, not a tie, and
    the jot goes to that group's main."""
    group_of = group_of or {}
    ix = index(c)
    ts = _toks(text)
    cont = _content(ts)
    out = {"decision": "new", "joke_id": None, "title": None, "reasons": [], "top": []}
    if len(cont) < ATTACH_MIN_WORDS:
        out["reasons"] = [f"too short to place ({len(cont)} content words)"]
        return out
    excl = set(exclude)
    cands = []
    if vec is not None and ix.M is not None:
        v = np.asarray(vec, dtype=np.float32)
        v = v / max(float(np.linalg.norm(v)), 1e-9)
        sims = ix.M @ v
        jot_p99 = float(_ref(sims[None, :])[0])
        best = {}
        for i in np.argsort(-sims)[:40].tolist():
            u = ix.vec_units[i]
            j = ix.ujoke[u]
            if j in excl or j not in c.jokes:
                continue
            cs = float(sims[i])
            band = min(cs - float(ix.p99[i]), cs - jot_p99)
            if j not in best or cs > best[j][0]:
                best[j] = (cs, band, u)
        for j, (cs, band, u) in best.items():
            phr, ct = _jot_support(ix, ts, j)
            cands.append({"joke_id": j, "cos": round(cs, 3), "band": round(band, 3), "unit": u, "phrases": phr,
                          "contain": ct})
    else:
        for j in ix.jokes:
            if j in excl:
                continue
            phr, ct = _jot_support(ix, ts, j)
            if phr or ct >= 0.6:
                cands.append({"joke_id": j, "cos": None, "band": None, "unit": None, "phrases": phr, "contain": ct})
    cands.sort(key=lambda x: (-(x["cos"] or 0), -len(x["phrases"]), -x["contain"], x["joke_id"]))
    out["top"] = [dict(x, title=c.title(x["joke_id"])) for x in cands[:3]]
    if not cands:
        return out
    words_ok = lambda x: len(x["phrases"]) >= 2 or (x["contain"] >= 0.6 and len(cont) >= ATTACH_MIN_WORDS)  # noqa: E731
    top = cands[0]
    home = group_of.get(top["joke_id"], top["joke_id"])
    others = [x for x in cands[1:] if group_of.get(x["joke_id"], x["joke_id"]) != home]
    if vec is not None:
        close = top["band"] is not None and top["band"] >= ATTACH_BAND
        rival = next((x for x in others if (x["cos"] or 0) > (top["cos"] or 0) - ATTACH_MARGIN and
                      (x["phrases"] or x["contain"] >= 0.4)), None)
    else:
        close = len(top["phrases"]) >= 3 and top["contain"] >= 0.6
        rival = next((x for x in others if len(x["phrases"]) >= 2), None)
    if close and words_ok(top) and rival is None:
        if home in c.jokes and home != top["joke_id"]:
            top = dict(top, joke_id=home)
        out.update(decision="attach", joke_id=top["joke_id"], title=c.title(top["joke_id"]), score=top["cos"],
                   reasons=[f"reads like your bit \"{c.title(top['joke_id'])}\""
                            + (f" (meaning {top['cos']:.2f}, above both texts' usual range)" if top["cos"] else ""),
                            ("same words: " + ", ".join(f"\"{p}\"" for p in top["phrases"][:3])) if top["phrases"]
                            else f"{int(top['contain'] * 100)}% of its words are in that bit"])
    elif close or words_ok(top):
        out.update(decision="ambiguous", reasons=[f"close to \"{c.title(top['joke_id'])}\" but not clearly the same bit"
                                                 + (f"; \"{c.title(rival['joke_id'])}\" is about as close" if rival else "")
                                                 + ", so it stays a new idea"])
    return out


# ---------------------------------------------------------------------------------------------------------- store
MERGE_OPS = ("merge", "undo", "attach", "detach", "dismiss")


def store_apply(items: dict, ev: dict) -> None:
    """merges.jsonl events (creative/store.py): merge / undo (his), attach (ai:attach) / detach (his), dismiss (his)."""
    op, i, d = ev["op"], ev["id"], ev.get("data") or {}
    if op == "merge":
        if i in items:
            raise ValueError(f"merge {i} exists")
        others = list(d.get("others") or [])
        if not d.get("main") or not others or d["main"] in others or len(set(others)) != len(others):
            raise ValueError("a merge needs a main and other jokes (not the main)")
        items[i] = {"id": i, "kind": "merge", "status": "merged", "main": d["main"], "others": others,
                    "indexes": list(d.get("indexes") or []), "snapshot": d.get("snapshot") or {},
                    "group": d.get("group"), "reasons": d.get("reasons") or [], "by": ev.get("by"),
                    "created_at": ev["ts"], "updated_at": ev["ts"]}
    elif op == "undo":
        m = items.get(i)
        if not m or m.get("kind") != "merge" or m["status"] != "merged":
            raise ValueError(f"no merge {i} to undo")
        m.update(status="undone", undo_indexes=list(d.get("indexes") or []), undone_at=ev["ts"], updated_at=ev["ts"])
    elif op == "attach":
        if i in items:
            raise ValueError(f"attach {i} exists")
        if not d.get("joke") or not d.get("into") or d["joke"] == d["into"]:
            raise ValueError("an attach needs the new joke and the joke it belongs to")
        items[i] = {"id": i, "kind": "attach", "status": "attached", "joke": d["joke"], "into": d["into"],
                    "text": d.get("text"), "date": d.get("date"), "score": d.get("score"),
                    "reasons": d.get("reasons") or [], "source": d.get("source"), "by": ev.get("by"),
                    "created_at": ev["ts"], "updated_at": ev["ts"]}
    elif op == "detach":
        m = items.get(i)
        if not m or m.get("kind") != "attach" or m["status"] != "attached":
            raise ValueError(f"no attached note {i}")
        m.update(status="detached", detached_at=ev["ts"], updated_at=ev["ts"])
    elif op == "dismiss":
        js = sorted(set(d.get("jokes") or []))
        if len(js) < 2:
            raise ValueError("dismiss needs two or more jokes")
        items[i] = {"id": i, "kind": "dismiss", "status": "dismissed", "jokes": js, "by": ev.get("by"),
                    "created_at": ev["ts"], "updated_at": ev["ts"]}
    else:
        raise ValueError(f"unknown op {op}")


def review_attaches(c: Corpus, items: dict, complete, log=None) -> list[dict]:
    """Offline second check of the AI's own attaches (run by `merge judge`): each live attach is shown to the judge
    as jot vs bit. Returns the ones the judge says are NOT the same bit (confidence >= 0.7), to detach as `ai:judge`.
    His own detaches are never touched; an unanswered attach stays."""
    out = []
    live = [m for m in items.values() if m.get("kind") == "attach" and m.get("status") == "attached"
            and not m.get("reviewed") and m.get("into") in c.jokes]
    ix = index(c)
    for i in range(0, len(live), SECOND_BATCH):
        batch = live[i:i + SECOND_BATCH]
        lines = [f"PAIR p{n + 1}\nA (new note): {' '.join((m.get('text') or '').split())[:1500]}\n"
                 f"B ({c.title(m['into'])}): {_full(c, ix, m['into'])}" for n, m in enumerate(batch)]
        try:
            r = complete("merge_judge", [{"role": "system", "content": JUDGE_SYSTEM},
                                         {"role": "user", "content": "Decide each pair.\n\n" + "\n\n".join(lines)}],
                         JUDGE_SCHEMA)
        except Exception as e:  # noqa: BLE001
            if log:
                log(f"attach review: {type(e).__name__}: {str(e)[:120]}")
            continue
        got = {p.get("id"): p for p in (r.data or {}).get("pairs") or []}
        for n, m in enumerate(batch):
            p = got.get(f"p{n + 1}")
            if p and not p.get("same") and float(p.get("confidence") or 0) >= 0.7:
                out.append({"id": m["id"], "why": str(p.get("why") or "")[:240]})
    return out


def dismissed_pairs(items: dict) -> frozenset:
    out = set()
    for m in items.values():
        if m.get("kind") == "dismiss" and m.get("status") == "dismissed":
            js = m["jokes"]
            out |= {frozenset((a, b)) for x, a in enumerate(js) for b in js[x + 1:]}
    return frozenset(out)


def attached(items: dict) -> dict:
    """new joke id -> its live attach record."""
    return {m["joke"]: m for m in items.values() if m.get("kind") == "attach" and m.get("status") == "attached"}


def history(items: dict, jid: str) -> dict:
    """Everything folded into a joke: his merges into it (each original's words, verbatim and dated, from the
    snapshot taken at merge time) and the notes auto-attached to it."""
    merges = [m for m in items.values() if m.get("kind") == "merge" and m.get("main") == jid]
    notes = [m for m in items.values() if m.get("kind") == "attach" and m.get("into") == jid and m["status"] == "attached"]
    merged_into = next((m for m in items.values() if m.get("kind") == "merge" and m["status"] == "merged"
                        and jid in m.get("others", [])), None)
    return {"joke_id": jid,
            "merges": sorted(merges, key=lambda m: m["created_at"]),
            "notes": sorted(({"id": m["id"], "joke_id": m["joke"], "text": m.get("text"), "date": m.get("date"),
                              "reasons": m.get("reasons"), "score": m.get("score"), "source": "inferred",
                              "attached_at": m["created_at"]} for m in notes), key=lambda n: (n["date"] or "", n["id"])),
            "merged_into": {"merge_id": merged_into["id"], "main": merged_into["main"]} if merged_into else None}


# ---------------------------------------------------------------------------------------------------------- CLI
def judge_candidates(c: Corpus) -> list[dict]:
    ix = index(c)
    fs = [ix.features(a, b) for a, b in sorted(ix.candidates())]
    fs = [f for f in fs if pair_score(f) >= JUDGE_FLOOR and ((f["cos"] or 0) >= UNIT_CAND or f["premise"])]
    return sorted(fs, key=lambda f: (-pair_score(f), f["a"], f["b"]))


def load_judgements(d) -> dict:
    import json
    from pathlib import Path
    p = Path(d) / "merge_judge.json"
    return (json.loads(p.read_text(encoding="utf-8")).get("pairs") or {}) if p.exists() else {}


def judge_all(c: Corpus, d, complete=None, log=print) -> dict:
    """Offline: ask the judge about every candidate pair not cached yet; the cache (merge_judge.json) is saved
    after every batch, so a stopped run resumes where it was."""
    from pathlib import Path

    from .util import write_json
    if complete is None:
        from .story import _creative_complete
        complete = _creative_complete()
    p = Path(d) / "merge_judge.json"
    cache = load_judgements(d)
    fs = judge_candidates(c)
    for i in range(0, len(fs), JUDGE_BATCH):
        judge(c, fs[i:i + JUDGE_BATCH], cache, complete, log)
        write_json(p, {"version": 1, "task": "merge_judge", "pairs": cache})
    second_opinion(c, cache, complete, log)
    write_json(p, {"version": 1, "task": "merge_judge", "pairs": cache})
    return {"candidates": len(fs), "judged": len(cache),
            "second_looks": sum(1 for v in cache.values() if "second" in v)}


def cli(c: Corpus, d, x: list[str], cap: int = 10) -> dict:
    from .store import Store
    sub = x[0] if x else "groups"
    st = Store(d, "merges", store_apply)
    j = load_judgements(d)
    if sub == "judge":
        out = judge_all(c, d)
        from .story import _creative_complete
        bad = review_attaches(c, st.items, _creative_complete(), print)
        for x in bad:  # the AI undoing its own placement: by ai:judge, never in decisions.jsonl
            st.append("detach", x["id"], {"why": x["why"]}, by="ai:judge")
        out["attaches_detached"] = [x["id"] for x in bad]
        return out
    if sub == "groups":
        g = find_groups(c, j, dismissed_pairs(st.items), exclude=set(attached(st.items)))
        g["examples"] = [{"main": x["main_title"], "why_main": x["main_why"], "confidence": x["confidence"],
                          "members": [f"{m['title']} ({m['first_date']}..{m['last_date']}, {m['nights']} nights)"
                                      for m in x["members"]], "reasons": x["reasons"][:2]} for x in g["groups"][:cap]]
        return g
    if sub == "for":
        return candidates_for(c, x[1], j, dismissed_pairs(st.items))
    if sub == "match":
        return match_text(c, " ".join(x[1:]))
    raise SystemExit(f"merge: unknown {sub} (groups | for <joke> | match <text> | judge)")
