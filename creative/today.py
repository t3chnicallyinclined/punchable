"""Today feed: one payload for the Today tab.

    python -m creative today [--ws <id>] [--date YYYY-MM-DD] [--json]

latest set summary · working set status · try-tonight notes · 3 serendipity picks (gems and
orphans, with a why) · new material from the latest set · minutes of material toward 5/15/30/60.
Serendipity picks rotate daily but are deterministic for a given date.
"""
from __future__ import annotations

import hashlib
from datetime import date as _date

from . import workingset as W
from .data import Corpus, venue_short
from .util import first_line, mmss, r1


def _rank(key: str, day: str) -> str:
    return hashlib.sha1(f"{day}|{key}".encode("utf-8")).hexdigest()


def latest_set(c: Corpus) -> dict | None:
    if not c.sets:
        return None
    k = c.set_keys()[-1]
    s = c.sets[k]
    sm = s["summary"]
    return {"set": k, "date": s["date"], "venue": s["venue"], "venue_label": venue_short(s["venue"], s["date"]),
            "title": s["title"], "material": mmss(sm.get("material_s")), "laugh_s_per_min": sm.get("laugh_s_per_min"),
            "room_heat": sm.get("room_heat"), "pct_laughing": sm.get("pct_laughing"),
            "jokes_counted": sm.get("jokes_counted"), "new_material": sm.get("new_material"),
            "best": sm.get("best") and {"joke_id": c.map_joke(sm["best"]), "title": c.title(c.map_joke(sm["best"]))},
            "weakest": sm.get("weakest") and {"joke_id": c.map_joke(sm["weakest"]), "title": c.title(c.map_joke(sm["weakest"]))}}


def new_material(c: Corpus) -> list[dict]:
    if not c.sets:
        return []
    k = c.set_keys()[-1]
    out = []
    for s in c.segments[k]:
        if s.get("status") != "new":
            continue
        out.append({"idx": s["idx"], "title": s.get("title"), "joke_id": s["joke"], "ai_rating": s.get("ai_rating"),
                    "laugh_s_per_min": s.get("laugh_s_per_min"), "vs_room": s.get("vs_room"),
                    "similar": [c.map_joke(x["joke_id"]) for x in (s.get("similar") or [])][:3],
                    "evidence": [e for e in [c.seg_evidence(s["seg_id"], 12)] if e]})
    return out


def serendipity(c: Corpus, day: str, n: int = 3) -> list[dict]:
    ins = c.insights
    gems = (ins.get("gems") or {}).get("gems", [])
    orphans = (ins.get("orphans") or {}).get("active_orphans", [])
    picks = []
    for g in sorted(gems, key=lambda g: _rank(g["joke"], day)):
        if len(picks) >= 2:
            break
        jid = c.map_joke(g["joke"])
        if jid not in c.jokes or jid == c.map_joke(g.get("like")) or c.perfs.get(jid):
            continue  # merged into its reference joke in v1, or on tape after all
        u = c.lead_unit(jid)
        line = first_line((c.nodes.get(u) or {}).get("text") or "") if u else ""
        picks.append({"kind": "gem", "joke_id": jid, "title": c.title(jid), "stage": c.stage(jid)[0],
                      "why": f"Never on tape, but built like {g.get('like_title')}: shares "
                             f"{', '.join((g.get('shared_shape') or [])[:3])}"
                             + (f" and {', '.join((g.get('shared_topics') or [])[:2])}" if g.get("shared_topics") else ""),
                      "score": g.get("score"), "source": "proposed",
                      "evidence": [e for e in [c.ev(u, line)] if e] if u and line else []})
    for o in sorted(orphans, key=lambda o: _rank(o, day)):
        if len(picks) >= n:
            break
        jid = c.map_joke(o)
        if jid not in c.jokes or any(p["joke_id"] == jid for p in picks):
            continue
        u = c.lead_unit(jid)
        if not u:
            continue
        line = first_line(c.nodes[u].get("text") or "")
        node = c.jokes[jid]
        picks.append({"kind": "orphan", "joke_id": jid, "title": c.title(jid), "stage": c.stage(jid)[0],
                      "why": f"An orphan: no link to anything else and never on tape (written {node.get('first_seen') or '?'}). "
                             "Give it a home on a storyboard or retire it.",
                      "score": None, "source": "inferred", "evidence": [e for e in [c.ev(u, line)] if e]})
    return picks[:n]


def minutes(c: Corpus) -> dict:
    """Minutes of material by stage (inferred stage first), measured from tape where possible."""
    by = {"working": 0.0, "testing": 0.0}
    measured = {"working": 0.0, "testing": 0.0}
    ids = {"working": [], "testing": []}
    for jid in c.jokes:
        st = c.stage(jid)[0]
        if st in by:
            d, basis = c.duration(jid)
            by[st] += d
            ids[st].append(jid)
            if basis.startswith("tape"):
                measured[st] += d
    proven = by["working"] / 60
    both = (by["working"] + by["testing"]) / 60
    return {"working_min": r1(proven), "testing_min": r1(by["testing"] / 60), "working_plus_testing_min": r1(both),
            "measured_share": r1(100 * (measured["working"] + measured["testing"]) / max(1, by["working"] + by["testing"])),
            "toward": [{"goal_min": g, "working_pct": min(100, round(100 * proven / g)),
                        "with_testing_pct": min(100, round(100 * both / g))} for g in (5, 15, 30, 60)],
            "n_working": len(ids["working"]), "n_testing": len(ids["testing"]),
            "basis": "stage from inferred.json (AI) unless you changed it; minutes = median taped duration, "
                     "else written words at 157 wpm"}


def build(c: Corpus, ws: dict | None = None, day: str | None = None) -> dict:
    day = day or _date.today().isoformat()
    out = {"date": day, "latest_set": latest_set(c), "working_set": None, "try_tonight": [],
           "serendipity": serendipity(c, day), "new_material": new_material(c), "minutes": minutes(c)}
    if ws:
        notes = W.try_tonight(c, ws)
        out["working_set"] = W.status(c, ws, notes)
        out["working_set"]["perfected"] = W.perfected(c, ws)["perfected"]
        out["try_tonight"] = notes[:3]
    tl = c.his_lists.get("list:tonight")
    if tl:
        out["his_tonight_list"] = [{"joke_id": c.map_joke(x["joke"]), "title": c.title(c.map_joke(x["joke"])),
                                    "done": x.get("done", False)} for x in tl.get("items", [])]
    return out
