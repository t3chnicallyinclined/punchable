"""Joke connections: ranked, capped groups by creative intent for one joke.

    python -m creative connections joke:airport-bit [--cap 5] [--json]

Groups: next, tags, same_world, callbacks, same_shape, contrasts. Each item carries a score
(0-1), a one-line why, verbatim evidence and its source (inferred / proposed / his).
Also: the stage trend across sets, the best night, and "what would make it land harder".
"""
from __future__ import annotations


import numpy as np

from .data import Corpus, venue_short
from .util import first_line, mean, median, r2, trend

GROUPS = [
    ("next", "What could come next"),
    ("tags", "Lines that could tag it"),
    ("same_world", "Same world (persona / premise)"),
    ("callbacks", "Callbacks"),
    ("same_shape", "Same shape"),
    ("contrasts", "Same topic, other angle"),
]
SRC_RANK = {"his": 2, "inferred": 1, "proposed": 0}
STAGE_W = {"working": 1.0, "testing": 1.0, "to_try": 0.95, "idea": 0.9, "retired": 0.8}


def _best_src(a, b):
    return a if SRC_RANK.get(a, 0) >= SRC_RANK.get(b, 0) else b


def _nights(c: Corpus, rows):
    return ", ".join(venue_short(r["venue"], r["date"]) for r in rows)


class _Group:
    def __init__(self):
        self.items = {}

    def add(self, key, score, why, source, evidence, basis, **extra):
        it = self.items.get(key)
        if it is None:
            it = {"id": key, "score": 0.0, "why": [], "source": source, "basis": [], "evidence": []}
            it.update(extra)
            self.items[key] = it
        it["score"] = max(it["score"], score) + (0.05 if it["basis"] and basis not in it["basis"] else 0)
        if why and why not in it["why"]:
            it["why"].append(why)
        it["source"] = _best_src(it["source"], source)
        if basis not in it["basis"]:
            it["basis"].append(basis)
        for e in evidence:
            if e and e not in it["evidence"]:
                it["evidence"].append(e)

    def ranked(self, c: Corpus, cap: int):
        out = []
        for it in self.items.values():
            it = dict(it)
            it["score"] = round(min(1.0, it["score"]), 3)
            it["why"] = "; ".join(it["why"])[:240]
            it["evidence"] = it["evidence"][:2]
            jid = it.get("joke_id")
            if jid:
                it["title"] = c.title(jid)
                it["stage"] = c.stage(jid)[0]
            out.append(it)
        out.sort(key=lambda x: (-x["score"], -SRC_RANK.get(x["source"], 0), x["id"]))
        return out[:cap]


def _label_names(c: Corpus, ls):
    return [c.labels[l]["name"] for l in ls]


def _label_ev(c: Corpus, jid: str, label: str):
    w_e = c.joke_labels.get(jid, {}).get(label)
    return c.edge_evidence(w_e[1], 1) if w_e else []


def _idf_overlap(c: Corpus, a: str, b: str, kinds, min_w=0.55):
    la, lb = c.labels_of(a, kinds, min_w), c.labels_of(b, kinds, min_w)
    shared = sorted(set(la) & set(lb), key=lambda l: (-c.idf.get(l, 1.0), l))
    if not la or not lb:
        return 0.0, []
    num = sum(c.idf.get(l, 1.0) for l in shared)
    den = sum(c.idf.get(l, 1.0) for l in set(la) | set(lb))
    return (num / den if den else 0.0), shared


def _candidates(c: Corpus, jid: str):
    return [k for k in c.jokes if k != jid]


def joke_connections(c: Corpus, jid: str, cap: int = 5) -> dict:
    if jid not in c.jokes:
        raise KeyError(f"unknown joke {jid}")
    g = {k: _Group() for k, _ in GROUPS}

    # ---------------------------------------------------------------- explicit edges
    for e in c.edges_by_node.get(jid, []):
        k, src, dst = e.get("kind"), e["src"], e["dst"]
        if k not in ("segue", "same_premise", "callback_to", "tag_of", "same_shape"):
            continue
        source, ev, conf = c.source_of(e), c.edge_evidence(e), float(e.get("confidence") or 0.5)
        reason = (e.get("reason") or "").strip()
        if k == "segue" and src == jid and dst in c.jokes:
            g["next"].add(dst, conf, reason, source, ev, "segue", joke_id=dst, edge=e["id"])
        elif k == "same_premise":
            other = dst if src == jid else src
            if other in c.jokes:
                g["same_world"].add(other, conf, reason, source, ev, "same_premise", joke_id=other, edge=e["id"])
        elif k == "callback_to":
            other = dst if src == jid else src
            if other in c.jokes:
                g["callbacks"].add(other, conf, reason, source, ev, "callback_to", joke_id=other, edge=e["id"],
                                   direction="this_calls_back" if src == jid else "calls_back_this")
        elif k == "same_shape":
            other = dst if src == jid else src
            if other in c.jokes:
                g["same_shape"].add(other, conf, reason, source, ev, "same_shape", joke_id=other, edge=e["id"])
        elif k == "tag_of" and dst == jid and src.startswith("unit:"):
            owner = c.unit_joke.get(src)
            q = ev[0]["quote"] if ev else None
            g["tags"].add(src, conf, reason, source, ev, "tag_of", unit_id=src, line=q,
                          joke_id=owner if owner and owner != jid else None, edge=e["id"])

    # ---------------------------------------------------------------- proven back-to-back
    for (a, b), rows in c.back_to_back.items():
        if a == jid and b in c.jokes:
            n = len(rows)
            ev = [c.seg_evidence(r["b_seg"]) for r in rows[-2:]]
            g["next"].add(b, min(1.0, 0.55 + 0.15 * n),
                          f"Done right after it {n}x on tape ({_nights(c, rows)})", "inferred", ev,
                          "back_to_back", joke_id=b, nights=n)

    # ---------------------------------------------------------------- tags by querying with the reveal
    # Spike 01: "could tag" works when the query is the reveal. Reveal vector vs other units' reveal
    # vectors (bge-small, cached); bge scores are compressed, so a hit must stand 2.5 sd above the mean.
    lead = c.lead_unit(jid)
    q = c.vectors.reveal(lead) if lead else None
    reveal = ((c.jokes[jid].get("anatomy") or {}).get("reveal") or "").strip()
    if q is not None:
        scored = []
        for u, owner in c.unit_joke.items():
            if owner == jid or c.nodes[u].get("kind") not in ("tag", "topper", "setup+punch", "callback"):
                continue
            v = c.vectors.reveal(u)
            if v is not None:
                scored.append((float(np.dot(q, v)), u))
        if len(scored) >= 10:
            arr = np.array([s for s, _ in scored])
            mu, sd = float(arr.mean()), float(arr.std()) or 1.0
            scored.sort(key=lambda x: (-x[0], x[1]))
            for s, u in scored[: cap * 2]:
                z = (s - mu) / sd
                if z < 2.5:
                    break
                un = c.nodes[u]
                rv = ((un.get("anatomy") or {}).get("reveal") or "").strip()
                line = rv if len(rv.split()) >= 3 and rv in (un.get("text") or "") else first_line(un.get("text") or "")
                ev = c.ev(u, line)
                if not ev:
                    continue
                owner = c.unit_joke.get(u)
                why = ("Lands on the same note as your reveal" + (f" '{reveal}'" if reveal else "")
                       + f" ({z:.1f} sd above typical); from '{c.title(owner)}'")
                score = min(1.0, 0.45 + 0.08 * (z - 2.5)) * STAGE_W.get(c.stage(owner)[0], 0.9)
                g["tags"].add(u, score, why, "proposed", [ev], "reveal_query", unit_id=u, line=line, joke_id=owner)

    # ---------------------------------------------------------------- same world: persona + specific topics
    my_personas = c.labels_of(jid, ("persona",), 0.5)
    my_topics = c.labels_of(jid, ("topic",), 0.55)
    if my_personas or my_topics:
        for o in _candidates(c, jid):
            sp = sorted(set(my_personas) & set(c.labels_of(o, ("persona",), 0.5)))
            ov_t, st = _idf_overlap(c, jid, o, ("topic",), 0.55)
            if not sp and ov_t < 0.34:
                continue
            score = (0.55 if sp else 0) + 0.45 * ov_t
            score *= STAGE_W.get(c.stage(o)[0], 0.9)
            parts = []
            if sp:
                parts.append("same persona: " + ", ".join(_label_names(c, sp)))
            if st:
                parts.append("shared topics: " + ", ".join(_label_names(c, st[:3])))
            ev = []
            for l in (sp + st)[:2]:
                ev += _label_ev(c, o, l)
            src = "inferred" if any(c.source_of(c.joke_labels[o][l][1]) != "proposed" for l in sp + st) else "proposed"
            g["same_world"].add(o, score, "; ".join(parts), src, ev[:2], "labels", joke_id=o,
                                personas=_label_names(c, sp), topics=_label_names(c, st[:3]))

    # same chunk (learn/insights Louvain over shared labels and links)
    for ch in (c.insights.get("chunks") or {}).get("chunks", []):
        members = [c.map_joke(x) for x in ch.get("jokes", [])]
        if jid in members:
            for o in members:
                if o and o != jid and o in c.jokes:
                    g["same_world"].add(o, 0.42 * STAGE_W.get(c.stage(o)[0], 0.9),
                                        f"same chunk in your Insights: '{ch.get('name')}'", "proposed", [],
                                        "chunk", joke_id=o)

    # ---------------------------------------------------------------- same shape: devices/structures + text
    for o in _candidates(c, jid):
        ov, shared = _idf_overlap(c, jid, o, ("device", "structure"), 0.55)
        if ov < 0.3 or len(shared) < 2:
            continue
        cs = c.cos(jid, o)
        score = 0.7 * ov + 0.3 * (cs if cs is not None else 0.5)
        score *= STAGE_W.get(c.stage(o)[0], 0.9)
        ev = []
        for l in shared[:2]:
            ev += _label_ev(c, o, l)
        g["same_shape"].add(o, score, "same moves: " + ", ".join(_label_names(c, shared[:3])), "proposed", ev[:2],
                            "labels", joke_id=o, shared=_label_names(c, shared[:3]))

    # ---------------------------------------------------------------- contrasts: same topic, opposite angle
    my_att = set(c.labels_of(jid, ("attitude",), 0.5))
    my_moves = set(c.devices(jid, 3))
    for o in _candidates(c, jid):
        ov_t, st = _idf_overlap(c, jid, o, ("topic",), 0.55)
        if not st or ov_t < 0.25:
            continue
        o_att = set(c.labels_of(o, ("attitude",), 0.5))
        o_moves = set(c.devices(o, 3))
        if my_moves & o_moves:
            continue  # same move = not another angle
        cs = c.cos(jid, o)
        att_flip = bool(my_att and o_att and not (my_att & o_att))
        score = ov_t * (1 - (cs if cs is not None else 0.6)) * 2.2 + (0.15 if att_flip else 0)
        score *= STAGE_W.get(c.stage(o)[0], 0.9)
        if score < 0.2:
            continue
        angle = (f"{', '.join(_label_names(c, sorted(o_att)))} instead of {', '.join(_label_names(c, sorted(my_att)))}"
                 if att_flip else
                 (f"built on {', '.join(_label_names(c, sorted(o_moves))[:2])}" if o_moves else "a different move"))
        ev = _label_ev(c, o, st[0])
        g["contrasts"].add(o, score, f"Also about {', '.join(_label_names(c, st[:2]))}, other angle: {angle}",
                           "proposed", ev, "labels", joke_id=o, topics=_label_names(c, st[:2]))

    groups = [{"key": k, "title": t, "items": g[k].ranked(c, cap)} for k, t in GROUPS]
    return {
        "joke": _joke_head(c, jid),
        "groups": groups,
        "stage_trend": stage_trend(c, jid),
        "land_harder": land_harder(c, jid),
        "notes": ["Scores are 0-1 within a group. Labels are machine proposals; laughs come from automatic "
                  "detection. Source 'his' = you confirmed it, 'inferred' = AI-accepted, 'proposed' = a suggestion."],
    }


def _joke_head(c: Corpus, jid: str) -> dict:
    n = c.jokes[jid]
    st, src = c.stage(jid)
    dur, basis = c.duration(jid)
    return {"id": jid, "title": n.get("title"), "stage": st, "stage_source": src, "summary": n.get("summary"),
            "devices": _label_names(c, c.devices(jid)), "duration_s": dur, "duration_basis": basis,
            "rooms": len({p["venue"] for p in c.perfs.get(jid, [])}), "nights": len(c.perfs.get(jid, []))}


def stage_trend(c: Corpus, jid: str) -> dict:
    ps = c.perfs.get(jid, [])
    nights = [{"date": p["date"], "set": p["set"], "venue": p["venue"], "label": venue_short(p["venue"], p["date"]),
               "laugh_s_per_min": p["laugh_s_per_min"], "vs_room": p["vs_room"], "ai_rating": p["ai_rating"],
               "laugh_s": p["laugh_s"], "duration_s": p["duration_s"],
               "time_to_first_laugh_s": p["time_to_first_laugh_s"], "seg_ids": p["seg_ids"]} for p in ps]
    t = trend([(p["date"], p["laugh_s_per_min"]) for p in ps])
    tv = trend([(p["date"], p["vs_room"]) for p in ps])
    best = None
    if ps:
        b = max(ps, key=lambda p: (p["laugh_s_per_min"] or 0, p["date"]))
        best = {"date": b["date"], "set": b["set"], "venue": b["venue"], "label": venue_short(b["venue"], b["date"]),
                "laugh_s_per_min": b["laugh_s_per_min"], "vs_room": b["vs_room"], "ai_rating": b["ai_rating"],
                "basis": "highest laugh-s/min (same rule as inferred.json best_set)"}
        br = max(ps, key=lambda p: (p["vs_room"] or 0, p["date"]))
        if br is not b:
            best["best_vs_room"] = {"date": br["date"], "label": venue_short(br["venue"], br["date"]), "vs_room": br["vs_room"]}
    return {"nights": nights, "trend": t["trend"], "trend_rel_change": t["rel_change"],
            "vs_room_trend": tv["trend"], "vs_room_rel_change": tv["rel_change"],
            "avg_laugh_s_per_min": r2(mean([p["laugh_s_per_min"] for p in ps])),
            "avg_vs_room": r2(mean([p["vs_room"] for p in ps])), "best_night": best,
            "low_n": len(ps) < 3,
            "note": "Raw laugh-s/min moves with the room; read it against vs_room (1.0 = the room's own rate)."}


def _said(text: str, phrase: str) -> bool:
    from .textdiff import present
    return present(phrase, text)


def land_harder(c: Corpus, jid: str, cap: int = 4) -> list[dict]:
    """Rule-based notes from his quality notes and the laugh metrics. Each note cites its basis."""
    out = []
    n = c.jokes[jid]
    note = ((n.get("quality") or {}).get("notes") or "").strip()
    if note:
        out.append({"text": f"Your writing note: {note}", "basis": "quality note (catalog pass)",
                    "source": "proposed", "evidence": [c.ev(jid, note, field="quality.notes")]})
    ps = c.perfs.get(jid, [])
    if ps:
        best = max(ps, key=lambda p: (p["laugh_s_per_min"] or 0, p["date"]))
        others = [p for p in ps if p is not best]
        # 1. a punchline that hit on the best night but was not said on the weaker nights
        for pl in sorted(best["punchlines"], key=lambda x: (-(x.get("bars") or 0), x.get("at_ms") or 0)):
            missing = [p for p in others if not any(_said(c.text_of(s) or "", pl["text"]) for s in p["seg_ids"])]
            if others and missing and (pl.get("bars") or 0) >= 2:
                ev = c.ev(pl["seg_id"], pl["text"])
                if ev:
                    out.append({"text": f"Keep \"{pl['text']}\": a {pl['bars']}-bar laugh at "
                                        f"{venue_short(best['venue'], best['date'])}, and you didn't say it at "
                                        f"{', '.join(venue_short(p['venue'], p['date']) for p in missing)}.",
                                "basis": "best night's punchlines vs the other nights' transcripts",
                                "source": "inferred", "evidence": [ev]})
                    break
        # 2. slow first laugh
        tt = [p["time_to_first_laugh_s"] for p in ps if p["time_to_first_laugh_s"] is not None]
        if tt and median(tt) > 15:
            fast = min(ps, key=lambda p: (p["time_to_first_laugh_s"] if p["time_to_first_laugh_s"] is not None else 1e9))
            out.append({"text": f"The first laugh takes {median(tt):.0f}s on average "
                                f"(fastest {fast['time_to_first_laugh_s']:.0f}s at {venue_short(fast['venue'], fast['date'])}). "
                                f"Get to the first punch sooner.",
                        "basis": "time_to_first_laugh_s across nights", "source": "inferred",
                        "evidence": [c.seg_evidence(fast["main_seg"])]})
        # 3. long dead air inside the bit
        da = max(ps, key=lambda p: (p["dead_air_max_s"] or 0))
        if (da["dead_air_max_s"] or 0) >= 25:
            out.append({"text": f"{da['dead_air_max_s']:.0f}s without a laugh inside the bit at "
                                f"{venue_short(da['venue'], da['date'])}. Add a tag or cut that stretch.",
                        "basis": "dead_air_max_s", "source": "inferred", "evidence": [c.seg_evidence(da["main_seg"])]})
        # 4. below the room most nights
        below = [p for p in ps if p["vs_room"] is not None and p["vs_room"] < 0.9]
        if len(ps) >= 2 and len(below) * 2 > len(ps):
            out.append({"text": f"It plays below the room on {len(below)} of {len(ps)} nights "
                                f"(avg {mean([p['vs_room'] for p in ps]):.2f}x the room).",
                        "basis": "vs_room", "source": "inferred", "evidence": []})
    for o in out:
        o["evidence"] = [e for e in o["evidence"] if e]
    return out[:cap]
