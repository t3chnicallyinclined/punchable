"""Working sets: a storyboard promoted to the set he performs night after night.

    python -m creative ws promote|show|version|lock|unlock|align|history|try|punchup|perfected|demo

Store: comedy/kb/creative/workingsets.jsonl (append-only events) + workingsets.json (snapshot).
Locks (bit or line) live on the working set and hold across versions: a locked bit gets no
notes and no punch-ups; a locked line is never offered to the model and never targeted.
"""
from __future__ import annotations

import os
import math

from . import story
from .data import Corpus, creative_dir, venue_short
from .store import Store
from .textdiff import phrases, present, toks
from .util import cv, mean, mmss, r2, sha8, slug, trend

OWNER_ID = os.environ.get("PK_OWNER_ID", "owner")  # the actor id on your own edits (`by`); PK_OWNER_ID

PUNCH_SCHEMA = {"type": "object", "properties": {"drafts": {"type": "array", "items": {
    "type": "object", "properties": {"line_no": {"type": "integer"}, "alt": {"type": "string"},
                                     "move": {"type": "string"}},
    "required": ["line_no", "alt", "move"], "additionalProperties": False}}},
    "required": ["drafts"], "additionalProperties": False}
DRAFT_LABEL = story.DRAFT_LABEL
LOW_N = 3  # fewer nights than this on either side = low n
VR_CLIP = 3.0


# ============================================================== store
def _apply(items: dict, ev: dict) -> None:
    op, i, d = ev["op"], ev["id"], ev.get("data") or {}
    if op == "promote":
        if i in items:
            raise ValueError(f"working set {i} exists")
        items[i] = {"id": i, "title": d["title"], "storyboard_id": d.get("storyboard_id"), "by": ev.get("by"),
                    "source": "his" if ev.get("by") == OWNER_ID else "inferred",
                    "created_at": ev["ts"], "updated_at": ev["ts"], "current": 1, "locks": {}, "deleted": False,
                    "versions": [{"v": 1, "created_at": ev["ts"], "note": d.get("note") or "v1: promoted from the storyboard",
                                  "target_s": int(d["target_s"]), "bits": list(d["bits"]), "by": ev.get("by")}]}
        return
    w = items.get(i)
    if w is None or w.get("deleted"):
        raise ValueError(f"no working set {i}")
    cur = w["versions"][-1]
    if op == "new_version":
        if not d.get("note"):
            raise ValueError("a new version needs a note saying what changed")
        v = {"v": cur["v"] + 1, "created_at": ev["ts"], "note": d["note"],
             "target_s": int(d.get("target_s") or cur["target_s"]),
             "bits": list(cur["bits"] if d.get("bits") is None else d["bits"]),  # [] = he emptied the set
             "by": ev.get("by")}
        w["versions"].append(v)
        w["current"] = v["v"]
    elif op in ("lock_bit", "unlock_bit"):
        lk = w["locks"].setdefault(d["joke_id"], {"bit": False, "lines": []})
        lk["bit"] = op == "lock_bit"
    elif op == "lock_line":
        lk = w["locks"].setdefault(d["joke_id"], {"bit": False, "lines": []})
        if not any(x["text"] == d["text"] for x in lk["lines"]):
            lk["lines"].append({"text": d["text"], "node": d.get("node"), "ts": ev["ts"]})
    elif op == "unlock_line":
        lk = w["locks"].get(d["joke_id"]) or {"lines": []}
        lk["lines"] = [x for x in lk["lines"] if x["text"] != d["text"]]
    elif op == "try_add":
        # his own try-tonight note (the AI's notes are computed by try_tonight, never stored)
        text = (d.get("text") or "").strip()
        if not text or len(text) > 500:
            raise ValueError("a try note needs its text (up to 500 characters)")
        notes = w.setdefault("try_notes", [])
        if any(n["id"] == d.get("note_id") for n in notes):
            raise ValueError(f"try note {d.get('note_id')} exists")
        notes.append({"id": d["note_id"], "joke_id": d.get("joke_id"), "text": text, "done": False,
                      "created_at": ev["ts"], "done_at": None, "by": ev.get("by")})
    elif op == "try_done":
        # a tick on one of his notes, or on one of the AI's (keyed "<joke_id>|<kind>")
        nid, done = d.get("note_id"), bool(d.get("done"))
        if not isinstance(nid, str) or not nid:
            raise ValueError("note_id is required")
        mine = next((n for n in w.get("try_notes", []) if n["id"] == nid), None)
        if mine is not None:
            mine["done"], mine["done_at"] = done, ev["ts"] if done else None
        else:
            w.setdefault("try_done", {})[nid] = done
    elif op == "update":
        if "title" in d:
            w["title"] = d["title"]
    elif op == "delete":
        w["deleted"] = True
    else:
        raise ValueError(f"unknown op {op}")
    w["updated_at"] = ev["ts"]


def ws_store(directory=None) -> Store:
    return Store(directory or creative_dir(), "workingsets", _apply)


def current(ws: dict) -> dict:
    return ws["versions"][-1]


def promote(c: Corpus, store: Store, board: dict, target_s: int | None = None, by: str = OWNER_ID,
            ts: str | None = None, id_: str | None = None, bit_targets: dict | None = None) -> dict:
    bits = []
    for j in board["joke_ids"]:
        d, basis = c.duration(j)
        t = (bit_targets or {}).get(j) or int(round(d / 5.0) * 5)
        bits.append({"joke_id": j, "title": c.title(j), "target_s": t, "target_basis": basis})
    id_ = id_ or f"ws:{slug(board['title'], 28)}-{sha8([board['id'], ts or ''])}"
    return store.append("promote", id_, {"title": board["title"], "storyboard_id": board["id"], "bits": bits,
                                         "target_s": target_s or board.get("target_s") or sum(b["target_s"] for b in bits),
                                         "note": f"v1: promoted from storyboard '{board['title']}'"}, by=by, ts=ts)


def new_version(store, ws_id, note, bits=None, target_s=None, by=OWNER_ID, ts=None):
    return store.append("new_version", ws_id, {"note": note, "bits": bits, "target_s": target_s}, by=by, ts=ts)


def lock_bit(store, ws_id, joke_id, by=OWNER_ID, ts=None):
    return store.append("lock_bit", ws_id, {"joke_id": joke_id}, by=by, ts=ts)


def unlock_bit(store, ws_id, joke_id, by=OWNER_ID, ts=None):
    return store.append("unlock_bit", ws_id, {"joke_id": joke_id}, by=by, ts=ts)


def lock_line(c: Corpus, store, ws_id, joke_id, text, node, by=OWNER_ID, ts=None):
    """Lock one line. It must be verbatim in the cited node (a set segment or a unit)."""
    if not c.verify({"node": node, "quote": text}):
        raise ValueError(f"line is not verbatim in {node}")
    return store.append("lock_line", ws_id, {"joke_id": joke_id, "text": text, "node": node}, by=by, ts=ts)


def unlock_line(store, ws_id, joke_id, text, by=OWNER_ID, ts=None):
    return store.append("unlock_line", ws_id, {"joke_id": joke_id, "text": text}, by=by, ts=ts)


def add_try(store, ws_id, joke_id, text, by=OWNER_ID, ts=None) -> tuple[dict, str]:
    """His own try-tonight note. Returns (working set, note id)."""
    from .util import now_iso
    ts = ts or now_iso()
    nid = f"try:{sha8([ws_id, joke_id, text, ts, len(store.items.get(ws_id, {}).get('try_notes', []))])}"
    return store.append("try_add", ws_id, {"note_id": nid, "joke_id": joke_id, "text": text}, by=by, ts=ts), nid


def tick_try(store, ws_id, note_id, done, by=OWNER_ID, ts=None):
    return store.append("try_done", ws_id, {"note_id": note_id, "done": bool(done)}, by=by, ts=ts)


def ai_note_id(n: dict) -> str:
    """The stable id of a computed try-tonight note (the app keys its ticks the same way)."""
    return f"{n['joke_id']}|{n['kind']}"


def with_his_notes(ws: dict, notes: list[dict]) -> list[dict]:
    """try_tonight's notes with `id` and his `done` ticks, after his own notes (source "his", kind "his")."""
    ticks = ws.get("try_done") or {}
    his = [{"joke_id": n.get("joke_id"), "title": None, "kind": "his", "score": 1.0, "text": n["text"], "evidence": [],
            "source": "his", "low_n": False, "id": n["id"], "done": bool(n.get("done")), "created_at": n.get("created_at")}
           for n in ws.get("try_notes", [])]
    ai = [{**n, "id": ai_note_id(n), "done": bool(ticks.get(ai_note_id(n)))} for n in notes]
    return his + ai


def is_locked_line(ws: dict, joke_id: str, line: str) -> bool:
    for x in (ws.get("locks", {}).get(joke_id) or {}).get("lines", []):
        if present(x["text"], line, 0.5) or present(line, x["text"], 0.5):
            return True
    return False


def bit_locked(ws: dict, joke_id: str) -> bool:
    return bool((ws.get("locks", {}).get(joke_id) or {}).get("bit"))


# ============================================================== align one night
def _lcs(a: list, b: list) -> list:
    m = [[0] * (len(b) + 1) for _ in range(len(a) + 1)]
    for i in range(len(a) - 1, -1, -1):
        for j in range(len(b) - 1, -1, -1):
            m[i][j] = m[i + 1][j + 1] + 1 if a[i] == b[j] else max(m[i + 1][j], m[i][j + 1])
    out, i, j = [], 0, 0
    while i < len(a) and j < len(b):
        if a[i] == b[j]:
            out.append(a[i]); i += 1; j += 1
        elif m[i + 1][j] >= m[i][j + 1]:
            i += 1
        else:
            j += 1
    return out


def align(c: Corpus, ws: dict, set_key: str) -> dict:
    ver = current(ws)
    plan = [b["joke_id"] for b in ver["bits"]]
    s = c.sets[set_key]
    segs = [x for x in c.segments[set_key] if x.get("status") != "banter"]
    first_pos, pos = {}, 0
    for x in segs:
        pos += 1
        if x["joke"] and x["joke"] not in first_pos and x.get("status") in ("known", "new"):
            first_pos[x["joke"]] = pos
    performed = sorted((j for j in plan if j in first_pos), key=lambda j: first_pos[j])
    in_order = _lcs(plan, performed)
    perf_by = {p["joke_id"]: p for j in plan for p in c.perfs.get(j, []) if p["set"] == set_key}
    bits = []
    for i, b in enumerate(ver["bits"], 1):
        j = b["joke_id"]
        p = perf_by.get(j)
        row = {"joke_id": j, "title": c.title(j), "planned_pos": i, "target_s": b["target_s"],
               "locked": bit_locked(ws, j), "performed": p is not None}
        if p:
            row.update({"performed_pos": first_pos[j], "segments": p["segments"], "time_s": p["duration_s"],
                        "delta_s": round(p["duration_s"] - b["target_s"], 1),
                        "laugh_s": p["laugh_s"], "laugh_s_per_min": p["laugh_s_per_min"], "vs_room": p["vs_room"],
                        "ai_rating": p["ai_rating"], "max_bars": p["max_bars"],
                        "time_to_first_laugh_s": p["time_to_first_laugh_s"],
                        "script_diff": {k: (p["script_diff"] or {}).get(k) for k in ("on_script_pct", "dropped", "added")},
                        "script_diff_basis": f"vs written version {p['version_unit']}" if p["version_unit"] else None,
                        "in_order": j in in_order,
                        "evidence": [x for x in [c.seg_evidence(p["main_seg"])] if x]})
        bits.append(row)
    added = []
    for x in segs:
        if x["joke"] in plan and x.get("status") in ("known", "new"):
            continue
        added.append({"idx": x["idx"], "title": x.get("title"), "status": x.get("status"), "joke_id": x["joke"],
                      "time_s": round((x["end_ms"] - x["start_ms"]) / 1000, 1),
                      "laugh_s_per_min": x.get("laugh_s_per_min"), "vs_room": x.get("vs_room"),
                      "ai_rating": x.get("ai_rating")})
    done = [b for b in bits if b["performed"]]
    ls, ws_ = sum(perf_by[b["joke_id"]]["laugh_s"] for b in done), sum(perf_by[b["joke_id"]]["window_s"] for b in done)
    summ = s["summary"]
    return {
        "working_set": ws["id"], "version": ver["v"], "set": set_key, "date": s["date"], "venue": s["venue"],
        "venue_label": venue_short(s["venue"], s["date"]),
        "order": {"planned": plan, "performed": performed, "in_order": in_order,
                  "moved": [j for j in performed if j not in in_order],
                  "same_order": performed == [j for j in plan if j in performed]},
        "dropped": [j for j in plan if j not in first_pos],
        "added": added,
        "bits": bits,
        "totals": {"planned_target_s": sum(b["target_s"] for b in ver["bits"]),
                   "performed_planned_s": round(sum(b["time_s"] for b in done), 1),
                   "added_s": round(sum(a["time_s"] for a in added), 1),
                   "bits_performed": len(done), "bits_planned": len(plan),
                   "ws_laugh_s_per_min": round(ls / (ws_ / 60), 2) if ws_ else None,
                   "set_laugh_s_per_min": summ.get("laugh_s_per_min"), "room_heat": summ.get("room_heat"),
                   "material_s": summ.get("material_s")},
    }


def align_all(c: Corpus, ws: dict) -> list[dict]:
    return [align(c, ws, k) for k in c.set_keys()]


# ============================================================== history + wording A/B
def _night_texts(c: Corpus, p: dict) -> list[tuple[str, str]]:
    return [(sid, c.text_of(sid) or "") for sid in p["seg_ids"]]


def wording_effects(c: Corpus, jid: str, cap: int = 5) -> dict:
    """Phrases said on some nights and not others, grouped by the exact nights they were said,
    with the bit's laugh_s and vs_room on nights with vs without. Correlation, not proof."""
    ps = c.perfs.get(jid, [])
    if len(ps) < 2:
        return {"effects": [], "changes": [], "note": "needs 2+ nights"}
    texts = [" ".join(t for _, t in _night_texts(c, p)) for p in ps]
    groups = {}
    for i, p in enumerate(ps):
        punch = [x["text"] for x in p["punchlines"] if (x.get("bars") or 0) >= 1]
        for sid, t in _night_texts(c, p):
            for ph in phrases(t):
                vec = tuple(k == i or present(ph, texts[k]) for k in range(len(ps)))
                if all(vec) or not any(vec):
                    continue
                g = groups.setdefault(vec, {"phrases": [], "seen": set()})
                key = " ".join(toks(ph))
                if key in g["seen"]:
                    continue
                g["seen"].add(key)
                is_punch = any(present(x, ph, 0.7) or present(ph, x, 0.7) for x in punch)
                g["phrases"].append((is_punch, len(ph.split()), ph, sid))
    effects = []
    for vec, g in groups.items():
        w = [p for p, v in zip(ps, vec) if v]
        wo = [p for p, v in zip(ps, vec) if not v]
        d_ls = mean([p["laugh_s"] for p in w]) - mean([p["laugh_s"] for p in wo])
        # vs_room is clipped at 3.0: a near-silent room (e.g. 0.76 laugh-s/min) inflates it to 7x
        vr_w = mean([min(VR_CLIP, p["vs_room"]) for p in w if p["vs_room"] is not None])
        vr_wo = mean([min(VR_CLIP, p["vs_room"]) for p in wo if p["vs_room"] is not None])
        d_vr = (vr_w - vr_wo) if vr_w is not None and vr_wo is not None else None
        g["phrases"].sort(key=lambda x: (-x[0], -x[1], x[2]))
        reps = [{"node": sid, "quote": ph, "punchline": bool(pn)} for pn, _, ph, sid in g["phrases"][:3]]
        reps = [r for r in reps if c.verify(r)]
        if not reps:
            continue
        low = min(len(w), len(wo)) < LOW_N
        sign = "+" if d_ls >= 0 else "-"
        effects.append({
            "nights_with": [venue_short(p["venue"], p["date"]) for p in w],
            "nights_without": [venue_short(p["venue"], p["date"]) for p in wo],
            "n_with": len(w), "n_without": len(wo), "n_phrases": len(g["phrases"]),
            "delta_laugh_s": round(d_ls, 1), "delta_vs_room": r2(d_vr),
            "statement": (f"{sign}{abs(d_ls):.1f} laugh-s over {len(w)} night{'s' if len(w) != 1 else ''} with it vs "
                          f"{len(wo)} without" + (f" (vs room {d_vr:+.2f})" if d_vr is not None else "")
                          + (" · low n, a hunch not a finding" if low else "")),
            "low_n": low, "examples": reps, "said_in_last": bool(vec[-1]),
            "weight": round(min(1.0, abs(d_vr or 0)) * min(1.0, math.sqrt(min(len(w), len(wo)) / LOW_N)), 3),
            "_vec": vec, "_rank": (abs(d_vr or 0) * math.sqrt(min(len(w), len(wo))) + (0.05 if reps[0]["punchline"] else 0),
                                   len(g["phrases"])),
        })
    effects.sort(key=lambda e: (-e["_rank"][0], -e["_rank"][1], e["examples"][0]["quote"]))
    kept, seen = [], set()
    for e in effects:  # a group and its mirror (said on exactly the other nights) tell one story: keep one
        vec = e.pop("_vec")
        e.pop("_rank")
        if tuple(not x for x in vec) in seen:
            continue
        seen.add(vec)
        kept.append(e)
    effects = kept
    # night-to-night diffs (what changed between consecutive nights)
    changes = []
    for k in range(1, len(ps)):
        a, b = ps[k - 1], ps[k]
        new = [(ph, sid) for sid, t in _night_texts(c, b) for ph in phrases(t) if not present(ph, texts[k - 1])]
        gone = [(ph, sid) for sid, t in _night_texts(c, a) for ph in phrases(t) if not present(ph, texts[k])]
        changes.append({"from": venue_short(a["venue"], a["date"]), "to": venue_short(b["venue"], b["date"]),
                        "from_date": a["date"], "to_date": b["date"],
                        "added": [{"node": sid, "quote": ph} for ph, sid in new[:3]],
                        "dropped": [{"node": sid, "quote": ph} for ph, sid in gone[:3]],
                        "n_added": len(new), "n_dropped": len(gone),
                        "delta_laugh_s": round(b["laugh_s"] - a["laugh_s"], 1),
                        "delta_vs_room": r2((b["vs_room"] or 0) - (a["vs_room"] or 0))})
    return {"effects": effects[:cap], "changes": changes,
            "note": "Wording vs laughs across nights is correlation inside different rooms; a line said only on one "
                    "night carries that whole night's room with it."}


def bit_history(c: Corpus, jid: str) -> dict:
    ps = c.perfs.get(jid, [])
    t = trend([(p["date"], p["laugh_s_per_min"]) for p in ps])
    best = max(ps, key=lambda p: (p["laugh_s_per_min"] or 0, p["date"])) if ps else None
    k = cv([p["laugh_s_per_min"] for p in ps])
    return {
        "joke_id": jid, "title": c.title(jid), "n_nights": len(ps),
        "nights": [{"date": p["date"], "set": p["set"], "venue": venue_short(p["venue"], p["date"]),
                    "time_s": p["duration_s"], "laugh_s": p["laugh_s"], "laugh_s_per_min": p["laugh_s_per_min"],
                    "vs_room": p["vs_room"], "ai_rating": p["ai_rating"]} for p in ps],
        "best_night": ({"date": best["date"], "venue": venue_short(best["venue"], best["date"]),
                        "laugh_s_per_min": best["laugh_s_per_min"], "vs_room": best["vs_room"]} if best else None),
        "trend": t["trend"], "trend_rel_change": t["rel_change"],
        "vs_room_trend": trend([(p["date"], p["vs_room"]) for p in ps])["trend"],
        "cv_laugh_s_per_min": r2(k), "stability": r2(max(0.0, 1 - k)) if k is not None else None,
        "low_n": len(ps) < LOW_N,
        "wording": wording_effects(c, jid),
    }


def history(c: Corpus, ws: dict) -> list[dict]:
    return [bit_history(c, b["joke_id"]) for b in current(ws)["bits"]]


# ============================================================== try tonight
def try_tonight(c: Corpus, ws: dict, cap: int = 6, hist: list | None = None) -> list[dict]:
    hist = hist or history(c, ws)
    last_key = c.set_keys()[-1] if c.sets else None
    last = align(c, ws, last_key) if last_key else None
    by_bit = {b["joke_id"]: b for b in (last or {}).get("bits", [])}
    notes = []
    for h in hist:
        j = h["joke_id"]
        if bit_locked(ws, j):
            continue  # a locked bit is perfected: no notes
        lb = by_bit.get(j) or {}
        title = h["title"]
        for e in h["wording"]["effects"]:
            ex = e["examples"][0]
            if is_locked_line(ws, j, ex["quote"]):
                continue
            weight = e["weight"]
            if e["delta_laugh_s"] > 0 and (e["delta_vs_room"] or 0) > 0 and not e["said_in_last"]:
                notes.append({"joke_id": j, "title": title, "kind": "put_back", "score": round(0.3 + 0.6 * weight, 3),
                              "text": f"{title}: put back \"{ex['quote']}\". {e['statement']}.",
                              "evidence": [ex], "source": "inferred", "low_n": e["low_n"]})
                break
            if e["delta_laugh_s"] < 0 and (e["delta_vs_room"] or 0) < 0 and e["said_in_last"]:
                notes.append({"joke_id": j, "title": title, "kind": "try_without", "score": round(0.25 + 0.6 * weight, 3),
                              "text": f"{title}: try it without \"{ex['quote']}\". {e['statement']}.",
                              "evidence": [ex], "source": "inferred", "low_n": e["low_n"]})
                break
        if last and not lb.get("performed"):
            notes.append({"joke_id": j, "title": title, "kind": "dropped", "score": 0.5,
                          "text": f"{title}: you skipped it last night ({last['venue_label']}); it's #{lb.get('planned_pos')} in v{last['version']}.",
                          "evidence": [], "source": "inferred", "low_n": False})
        elif lb.get("performed") and lb["target_s"] and lb["time_s"] > 1.25 * lb["target_s"]:
            notes.append({"joke_id": j, "title": title, "kind": "over_time",
                          "score": round(min(0.9, 0.3 + (lb["time_s"] / lb["target_s"] - 1) * 0.3), 3),
                          "text": f"{title}: ran {mmss(lb['time_s'])} last night vs a {mmss(lb['target_s'])} target. Trim the setup.",
                          "evidence": lb.get("evidence", []), "source": "inferred", "low_n": False})
        ttfl = lb.get("time_to_first_laugh_s")
        bestt = [p["time_to_first_laugh_s"] for p in c.perfs.get(j, []) if p["time_to_first_laugh_s"] is not None]
        if ttfl is not None and bestt and ttfl > 20 and ttfl > min(bestt) + 10:
            notes.append({"joke_id": j, "title": title, "kind": "slow_start", "score": 0.35,
                          "text": f"{title}: the first laugh came {ttfl:.0f}s in last night; your fastest night got one in {min(bestt):.0f}s.",
                          "evidence": lb.get("evidence", []), "source": "inferred", "low_n": len(bestt) < LOW_N})
    notes.sort(key=lambda n: (-n["score"], n["joke_id"], n["kind"]))
    return notes[:cap]


# ============================================================== punch-up drafts
def punch_up(c: Corpus, ws: dict, jid: str, complete=None, n: int = 3) -> dict:
    base = {"joke_id": jid, "title": c.title(jid), "draft": True, "label": DRAFT_LABEL, "drafts": []}
    if bit_locked(ws, jid):
        return dict(base, status="locked", why="This bit is locked (perfected); no punch-ups.")
    ps = c.perfs.get(jid, [])
    if not ps:
        return dict(base, status="no_tape", why="No taped night to work from.")
    p = ps[-1]
    lines, blocked = [], 0
    for sid in p["seg_ids"]:
        for ph in phrases(c.text_of(sid) or ""):
            if is_locked_line(ws, jid, ph):
                blocked += 1
                continue
            lines.append((ph, sid))
    if not lines:
        return dict(base, status="all_locked", why="Every line on the last night is locked.", blocked_locked=blocked)
    lines = lines[:40]
    best = max(ps, key=lambda x: (x["laugh_s_per_min"] or 0, x["date"]))
    hits = [x["text"] for x in best["punchlines"] if (x.get("bars") or 0) >= 2][:4]
    devs = [c.labels[l]["name"] for l in c.devices(jid)]
    msg = (f"Bit: '{c.title(jid)}'. His moves in this bit: {', '.join(devs) or 'n/a'}.\n"
           f"Lines that got real laughs on his best night ({venue_short(best['venue'], best['date'])}):\n"
           + "\n".join(f"- {h}" for h in hits)
           + "\n\nLines from his latest night you MAY punch up (only these, by number):\n"
           + "\n".join(f"{i}. {ph}" for i, (ph, _) in enumerate(lines, 1))
           + f"\n\nGive up to {n} punch-up drafts: a sharper alternative for one numbered line each, in his voice, "
             "using his own moves. Keep the premise. `move` names the device you used.")
    complete = complete or story._creative_complete()
    try:
        r = complete("creative_punchup", [{"role": "system", "content": "You punch up one comedian's own lines in "
                                           "his voice. Drafts only; he decides."},
                                          {"role": "user", "content": msg}], PUNCH_SCHEMA)
    except Exception as ex:
        return dict(base, status="provider_error", why=f"{type(ex).__name__}: {str(ex)[:160]}", blocked_locked=blocked)
    out = []
    for d in (r.data or {}).get("drafts", [])[:n]:
        k = d.get("line_no")
        if not isinstance(k, int) or not (1 <= k <= len(lines)):
            continue
        ph, sid = lines[k - 1]
        if is_locked_line(ws, jid, ph):  # belt and braces: never target a locked line
            blocked += 1
            continue
        alt = (d.get("alt") or "").strip()
        if not alt or " ".join(toks(alt)) == " ".join(toks(ph)):
            continue
        out.append({"target_line": ph, "target_evidence": {"node": sid, "quote": ph}, "alt": alt,
                    "move": d.get("move"), "draft": True, "label": DRAFT_LABEL})
    return dict(base, status="ok", drafts=out, blocked_locked=blocked, based_on=p["set"],
                provider=getattr(r, "provider", None), model=getattr(r, "model", None),
                call_id=getattr(r, "call_id", None), task="creative_punchup")


# ============================================================== perfected meter
def perfected(c: Corpus, ws: dict, hist: list | None = None) -> dict:
    hist = {h["joke_id"]: h for h in (hist or history(c, ws))}
    ver = current(ws)
    tot = sum(b["target_s"] for b in ver["bits"]) or 1
    locked_s, stab_w, stab_sum, rows = 0.0, 0.0, 0.0, []
    for b in ver["bits"]:
        j = b["joke_id"]
        lk = ws.get("locks", {}).get(j) or {}
        if lk.get("bit"):
            share = 1.0
        elif lk.get("lines"):
            ps = c.perfs.get(j, [])
            words = sum(len(toks(c.text_of(s) or "")) for s in ps[-1]["seg_ids"]) if ps else 0
            lw = sum(len(toks(x["text"])) for x in lk["lines"])
            share = min(1.0, lw / words) if words else 0.0
        else:
            share = 0.0
        locked_s += share * b["target_s"]
        h = hist.get(j) or {}
        st = h.get("stability")
        if st is not None:
            stab_w += b["target_s"]
            stab_sum += st * b["target_s"]
        rows.append({"joke_id": j, "title": b["title"], "locked_share": round(share, 2),
                     "cv_laugh_s_per_min": h.get("cv_laugh_s_per_min"), "stability": st,
                     "nights": h.get("n_nights", 0), "low_n": h.get("low_n", True)})
    locked = locked_s / tot
    stability = (stab_sum / stab_w) if stab_w else None
    score = 0.5 * locked + 0.5 * (stability or 0)
    return {"locked_share": round(locked, 2), "stability": r2(stability), "perfected": round(score, 2),
            "formula": "0.5 x locked share (by target time) + 0.5 x stability; stability = 1 - CV of laugh-s/min "
                       "across nights (sample sd / mean, floor 0), target-time weighted over bits with 2+ nights",
            "bits": rows}


# ============================================================== status line for Today
def status(c: Corpus, ws: dict, notes: list | None = None) -> dict:
    al = [a for a in align_all(c, ws) if a["totals"]["bits_performed"]]
    last = al[-1] if al else None
    prev = al[-2] if len(al) > 1 else None
    ver = current(ws)
    lr = last["totals"]["ws_laugh_s_per_min"] if last else None
    pr = prev["totals"]["ws_laugh_s_per_min"] if prev else None
    arrow = None if lr is None or pr is None else ("up" if lr > pr * 1.05 else "down" if lr < pr * 0.95 else "flat")
    notes = notes if notes is not None else try_tonight(c, ws)
    line = (f"{ws['title']} v{ver['v']} · {mmss(sum(b['target_s'] for b in ver['bits']))}"
            + (f" · last night {lr:.1f} laugh-s/min {'▲' if arrow == 'up' else '▼' if arrow == 'down' else '='}" if lr is not None else "")
            + f" · {min(3, len(notes))} things to try tonight")
    return {"working_set": ws["id"], "title": ws["title"], "version": ver["v"],
            "target_s": sum(b["target_s"] for b in ver["bits"]), "last_night": last and {
                "set": last["set"], "date": last["date"], "venue": last["venue_label"], "ws_laugh_s_per_min": lr,
                "bits_performed": last["totals"]["bits_performed"], "bits_planned": last["totals"]["bits_planned"]},
            "vs_previous": arrow, "line": line}


# ============================================================== demo
DEMO_BITS = ["joke:wedding-gig", "joke:smart-speaker", "joke:smart-fridge", "joke:gym-membership",
             "joke:parking-meter", "joke:airport-bit"]


def demo(c: Corpus, boards: Store, wss: Store, ts: str | None = None, with_drafts: bool = True, complete=None) -> dict:
    sid = "sb:demo-everyday-gripes-run"
    tgt = int(sum(round(c.duration(j)[0] / 5.0) * 5 for j in DEMO_BITS))
    b = boards.items.get(sid) or story.create(
        boards, "Everyday gripes run", DEMO_BITS, target_s=tgt, by="ai:demo", ts=ts, id_=sid,
        notes="Demo: his six repeated bits in his 2023-2025 running order.")
    wid = "ws:demo-everyday-gripes-run"
    ws = wss.items.get(wid) or promote(c, wss, b, by="ai:demo", ts=ts, id_=wid)
    hist = history(c, ws)
    notes = try_tonight(c, ws, hist=hist)
    out = {"working_set": ws, "alignments": align_all(c, ws), "history": hist, "try_tonight": notes,
           "perfected": perfected(c, ws, hist), "status": status(c, ws, notes)}
    if with_drafts:
        weakest = min((h for h in hist if h["n_nights"] and not bit_locked(ws, h["joke_id"])),
                      key=lambda h: (mean([n["vs_room"] for n in h["nights"]]) or 0, h["joke_id"]), default=None)
        if weakest:
            out["punch_up"] = punch_up(c, ws, weakest["joke_id"], complete=complete)
    return out
