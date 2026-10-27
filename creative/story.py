"""Storyboards: chain jokes into a longer story or set.

    python -m creative story list|show|create|add|remove|move|update|health|next|bridge|seed|export

Store: comedy/kb/creative/storyboards.jsonl (append-only events) + storyboards.json (snapshot).
"""
from __future__ import annotations

import os
import math
import re
from collections import defaultdict

from .data import Corpus, creative_dir, venue_short
from .store import Store
from .util import mean, mmss, r2, sha8, slug, suffix_words, prefix_words

COMIC = os.environ.get("PK_COMIC_NAME", "the comic")  # how the AI prompts name you; PK_COMIC_NAME
COMIC_FIRST = os.environ.get("PK_COMIC_FIRST", COMIC)

OWNER_ID = os.environ.get("PK_OWNER_ID", "owner")  # the actor id on your own edits (`by`); PK_OWNER_ID

ROOMS_MIN = 2
CLOSER_STRONG = 1.2   # mean vs_room for a strong closer
LOW_ENERGY = 0.8      # mean vs_room under this = a low bit
TARGET_TOL = 0.10     # runtime within +-10% of target is on target
HUB_DF = 150          # a topic on more jokes than this is a hub, not a link


# ============================================================== store
def _apply(items: dict, ev: dict) -> None:
    op, i, d = ev["op"], ev["id"], ev.get("data") or {}
    if op == "create":
        if i in items:
            raise ValueError(f"storyboard {i} exists")
        items[i] = {"id": i, "title": d.get("title") or "Untitled", "joke_ids": list(d.get("joke_ids") or []),
                    "target_s": int(d.get("target_s") or 900), "notes": d.get("notes") or "",
                    "source": "his" if ev.get("by") == OWNER_ID else "inferred", "by": ev.get("by"),
                    "seed": d.get("seed"), "created_at": ev["ts"], "updated_at": ev["ts"], "deleted": False}
        return
    b = items.get(i)
    if b is None or b.get("deleted"):
        raise ValueError(f"no storyboard {i}")
    ids = b["joke_ids"]
    if op == "update":
        for k in ("title", "target_s", "notes"):
            if k in d:
                b[k] = int(d[k]) if k == "target_s" else d[k]
    elif op == "set_jokes":
        b["joke_ids"] = list(d["joke_ids"])
    elif op == "add":
        j = d["joke_id"]
        if j in ids:
            raise ValueError(f"{j} already on the board")
        at = d.get("at")
        ids.insert(len(ids) if at is None else max(0, min(int(at), len(ids))), j)
    elif op == "remove":
        if d["joke_id"] not in ids:
            raise ValueError(f"{d['joke_id']} not on the board")
        ids.remove(d["joke_id"])
    elif op == "move":
        f, t = int(d["from"]), int(d["to"])
        if not (0 <= f < len(ids) and 0 <= t < len(ids)):
            raise ValueError("move out of range")
        ids.insert(t, ids.pop(f))
    elif op == "delete":
        b["deleted"] = True
    else:
        raise ValueError(f"unknown op {op}")
    if ev.get("by") == OWNER_ID:
        b["source"] = "his"
    b["updated_at"] = ev["ts"]


def board_store(directory=None) -> Store:
    return Store(directory or creative_dir(), "storyboards", _apply)


def create(store: Store, title: str, joke_ids=(), target_s: int = 900, notes: str = "", by: str = OWNER_ID,
           ts: str | None = None, id_: str | None = None, seed: dict | None = None) -> dict:
    id_ = id_ or f"sb:{slug(title, 32)}-{sha8([title, ts or '', len(store.items)])}"
    return store.append("create", id_, {"title": title, "joke_ids": list(joke_ids), "target_s": target_s,
                                        "notes": notes, "seed": seed}, by=by, ts=ts)


def update(store, id_, by=OWNER_ID, ts=None, **fields):
    return store.append("update", id_, {k: v for k, v in fields.items() if v is not None}, by=by, ts=ts)


def add(store, id_, joke_id, at=None, by=OWNER_ID, ts=None):
    return store.append("add", id_, {"joke_id": joke_id, "at": at}, by=by, ts=ts)


def remove(store, id_, joke_id, by=OWNER_ID, ts=None):
    return store.append("remove", id_, {"joke_id": joke_id}, by=by, ts=ts)


def move(store, id_, frm, to, by=OWNER_ID, ts=None):
    return store.append("move", id_, {"from": frm, "to": to}, by=by, ts=ts)


def set_jokes(store, id_, joke_ids, by=OWNER_ID, ts=None):
    return store.append("set_jokes", id_, {"joke_ids": list(joke_ids)}, by=by, ts=ts)


# ============================================================== transitions
def _direct_edges(c: Corpus, a: str, b: str) -> list[dict]:
    out = []
    for e in c.edges_by_node.get(a, []):
        k = e.get("kind")
        if k not in ("segue", "same_premise", "callback_to", "tag_of", "same_shape", "variant_of"):
            continue
        s, d = e["src"], e["dst"]
        s = c.unit_joke.get(s, s) if s.startswith("unit:") else s
        d = c.unit_joke.get(d, d) if d.startswith("unit:") else d
        if {s, d} == {a, b}:
            out.append(e)
    rank = {"segue": 0, "callback_to": 1, "same_premise": 2, "tag_of": 3, "same_shape": 4, "variant_of": 5}
    out.sort(key=lambda e: (rank.get(e["kind"], 9), -(e.get("confidence") or 0), e["id"]))
    return out


def shared_labels(c: Corpus, a: str, b: str, kinds, min_w=0.4) -> list[str]:
    la, lb = c.labels_of(a, kinds, min_w), c.labels_of(b, kinds, min_w)
    return sorted(set(la) & set(lb), key=lambda l: (-c.idf.get(l, 1.0), l))


def _subject_link(c: Corpus, a: str, b: str):
    sc, ws, near = same_subject(c, [a], b)
    return (sc, ws, near) if sc >= 0.35 else None


def transition(c: Corpus, a: str, b: str) -> dict:
    """proven = performed back-to-back on tape; link = an edge (or a shared persona) exists; jump = none."""
    rows = c.back_to_back.get((a, b), [])
    edges = _direct_edges(c, a, b)
    personas = shared_labels(c, a, b, ("persona",), 0.5)
    topics = shared_labels(c, a, b, ("topic",), 0.4)
    # a topic link: a shared topic both bits hold at >= 0.4, skipping hub topics (> 150 jokes, as Insights does)
    strong_topics = [l for l in topics if c.label_df.get(l, 0) <= HUB_DF]
    via, evidence = [], []
    for e in edges[:3]:
        rev_seg = e["kind"] == "segue" and e["src"] != a
        via.append({"kind": e["kind"], "edge": e["id"], "source": c.source_of(e), "why": e.get("reason"),
                    "reverse": rev_seg})
        evidence += c.edge_evidence(e, 1)
    for l in personas:
        via.append({"kind": "persona", "label_id": l, "name": c.labels[l]["name"], "source": "proposed"})
    if rows:
        label = "proven"
        why = f"Done back-to-back {len(rows)}x ({', '.join(venue_short(r['venue'], r['date']) for r in rows)})"
        evidence = [c.seg_evidence(r["b_seg"]) for r in rows[-2:]] + evidence
    elif edges:
        label = "link"
        e = edges[0]
        kind = e["kind"].replace("_", " ")
        if e["kind"] == "segue" and e["src"] != a:
            kind = "segue (you usually do it the other way round)"
        why = f"{kind}: {e.get('reason') or ''}".strip()
    elif personas or strong_topics:
        label = "link"
        if personas:
            why = f"persona link: {', '.join(c.labels[l]['name'] for l in personas)} (needs a bridge line)"
        else:
            why = f"topic link: {', '.join(c.labels[l]['name'] for l in strong_topics[:2])} (needs a bridge line)"
            for l in strong_topics[:2]:
                via.append({"kind": "topic", "label_id": l, "name": c.labels[l]["name"], "source": "proposed"})
        for l in (personas or strong_topics)[:1]:
            for j in (a, b):
                w_e = c.joke_labels[j].get(l)
                if w_e:
                    evidence += c.edge_evidence(w_e[1], 1)
    elif _subject_link(c, a, b):
        sc, ws, _ = _subject_link(c, a, b)
        label = "link"
        why = "same subject" + (f" ({', '.join(ws)})" if ws else "") + " (needs a bridge line)"
        via.append({"kind": "subject", "score": round(sc, 2), "words": ws, "source": "inferred"})
    else:
        label = "jump"
        why = ("topic jump" + (f" (only a weak shared topic: {', '.join(c.labels[l]['name'] for l in topics[:2])})"
                               if topics else " · no link yet"))
    rev = c.back_to_back.get((b, a), [])
    return {"from": a, "to": b, "label": label, "why": why[:240], "via": via,
            "needs_bridge": label == "jump" or (label == "link" and not edges),
            "shared_topics": [c.labels[l]["name"] for l in topics[:3]],
            "nights": [{"set": r["set"], "date": r["date"], "venue": r["venue"]} for r in rows],
            "reverse_order_nights": len(rev),
            "evidence": [x for x in evidence if x][:3]}


# ============================================================== health
def heat(energy) -> int | None:
    if energy is None:
        return None
    return 1 if energy < 0.5 else 2 if energy < LOW_ENERGY + 0.1 else 3 if energy < 1.3 else 4


def health(c: Corpus, board: dict) -> dict:
    ids = [j for j in board["joke_ids"] if j in c.jokes]
    bits, total = [], 0.0
    for j in ids:
        d, basis = c.duration(j)
        e, n = c.energy(j)
        total += d
        bits.append({"joke_id": j, "title": c.title(j), "stage": c.stage(j)[0], "duration_s": d,
                     "duration_basis": basis, "energy": e, "heat": heat(e), "nights": n,
                     "devices": [c.labels[l]["name"] for l in c.devices(j)]})
    trans = [transition(c, a, b) for a, b in zip(ids, ids[1:])]
    checks = []
    tgt = board.get("target_s") or 0
    # R1 runtime
    if not tgt:
        checks.append({"rule": "runtime", "status": "info", "text": f"{mmss(total)} total (no target)"})
    elif abs(total - tgt) <= TARGET_TOL * tgt:
        checks.append({"rule": "runtime", "status": "ok", "text": f"{mmss(total)} of {mmss(tgt)}: on target"})
    elif total < tgt:
        checks.append({"rule": "runtime", "status": "info", "text": f"{mmss(total)} of {mmss(tgt)}: {mmss(tgt - total)} to fill"})
    else:
        checks.append({"rule": "runtime", "status": "warn", "text": f"{mmss(total)} of {mmss(tgt)}: {mmss(total - tgt)} over"})
    # R2 unbridged jumps
    jumps = [t for t in trans if t["label"] == "jump"]
    if jumps:
        names = "; ".join(f"{c.title(t['from'])} -> {c.title(t['to'])}" for t in jumps)
        checks.append({"rule": "jumps", "status": "warn",
                       "text": f"{len(jumps)} topic jump{'s' if len(jumps) > 1 else ''} with no bridge: {names}",
                       "pairs": [[t["from"], t["to"]] for t in jumps]})
    else:
        nb = sum(1 for t in trans if t["needs_bridge"])
        checks.append({"rule": "jumps", "status": "ok", "text": "every transition has a link"
                       + (f" ({nb} persona/topic link{'s' if nb > 1 else ''} could use a bridge line)" if nb else "")})
    # R3 consecutive repeated devices
    reps = []
    for a, b in zip(ids, ids[1:]):
        shared = sorted(set(c.devices(a)) & set(c.devices(b)))
        if shared:
            reps.append({"pair": [a, b], "devices": [c.labels[l]["name"] for l in shared]})
    if reps:
        r = reps[0]
        checks.append({"rule": "repeated_devices", "status": "warn",
                       "text": f"\"{r['devices'][0]}\" twice in a row ({c.title(r['pair'][0])} -> {c.title(r['pair'][1])})"
                               + (f" and {len(reps) - 1} more" if len(reps) > 1 else ""), "repeats": reps})
    else:
        checks.append({"rule": "repeated_devices", "status": "ok", "text": "device variety: no move repeats back-to-back"})
    # R4 closer strength
    if ids:
        last = bits[-1]
        strong = last["energy"] is not None and (
            (last["energy"] >= CLOSER_STRONG and last["nights"] >= ROOMS_MIN) or
            any(p["ai_rating"] == "killed" for p in c.perfs.get(last["joke_id"], [])) and last["energy"] >= 1.0)
        if strong:
            checks.append({"rule": "closer", "status": "ok",
                           "text": f"ends on a big one: {last['title']} ({last['energy']:.2f}x the room, {last['nights']} night{'s' if last['nights'] != 1 else ''})"})
        else:
            cands = [b for b in bits[:-1] if b["energy"] is not None and b["energy"] >= CLOSER_STRONG]
            sug = max(cands, key=lambda b: (b["energy"], b["joke_id"])) if cands else None
            if last["energy"] is None:
                text = f"closer untested: {last['title']} has no tape"
            else:
                text = f"closer could be stronger: {last['title']} averages {last['energy']:.2f}x the room"
            if sug:
                text += f" (try {sug['title']}, {sug['energy']:.2f}x)"
            checks.append({"rule": "closer", "status": "warn", "text": text,
                           "suggest": sug["joke_id"] if sug else None})
    # R5 energy dip: two taped low bits in a row
    dips = [[a["joke_id"], b["joke_id"]] for a, b in zip(bits, bits[1:])
            if a["energy"] is not None and b["energy"] is not None and a["energy"] < LOW_ENERGY and b["energy"] < LOW_ENERGY]
    if dips:
        checks.append({"rule": "energy_dip", "status": "warn",
                       "text": f"energy dips: {c.title(dips[0][0])} and {c.title(dips[0][1])} both play under {LOW_ENERGY}x the room",
                       "pairs": dips})
    else:
        checks.append({"rule": "energy_dip", "status": "ok", "text": "no two low bits in a row"})
    untaped = [b["title"] for b in bits if b["energy"] is None]
    return {"board": board["id"], "title": board.get("title"), "runtime_s": round(total, 1), "runtime": mmss(total),
            "target_s": tgt, "bits": bits, "transitions": trans,
            "energy_curve": [{"joke_id": b["joke_id"], "energy": b["energy"], "heat": b["heat"]} for b in bits],
            "checks": checks, "untaped": untaped,
            "notes": "energy = mean vs_room over taped nights (1.0 = the room's own laugh rate); heat 1-4 for bars."}


# ============================================================== best next
_WORD = re.compile(r"[a-z][a-z']{2,}")
_STOP = set("""the and that this with you your for are was but not what when they them then there their have has had
just like its it's i'm don't can't about from all one get got know out who how why would could should will into
been were did does doing very really because also more some any him her his she he our we us my me too than
only even ever every said says say going gonna want make made thing things people time good bad""".split())


def _subject_index(c: Corpus):
    """Per-joke content words (title + summary + unit text) with idf over all jokes; cached on the corpus."""
    idx = getattr(c, "_subject_idx", None)
    if idx is not None:
        return idx
    words = {}
    for j, n in c.jokes.items():
        lead = c.lead_unit(j)
        txt = " ".join([n.get("title") or "", n.get("title") or "", n.get("summary") or "",
                        (c.nodes[lead].get("text") or "") if lead else ""])
        words[j] = {w for w in _WORD.findall(txt.lower()) if w not in _STOP}
    df = defaultdict(int)
    for ws in words.values():
        for w in ws:
            df[w] += 1
    nj = max(1, len(words))
    # a subject word is shared by at least 2 jokes (so it can link anything) and is not common
    idf = {w: math.log(nj / (1 + d)) for w, d in df.items() if 2 <= d <= max(3, nj // 50)}
    c._subject_idx = (words, idf)
    return c._subject_idx


def _cos_band(c: Corpus, j: str) -> tuple[float, float]:
    cache = c.__dict__.setdefault("_cos_band", {})
    if j not in cache:
        cs = [x for x in (c.cos(j, o) for o in c.jokes if o != j) if x is not None]
        if len(cs) < 50:
            cache[j] = (1.0, 1.0)
        else:
            cs.sort()
            cache[j] = (cs[int(0.97 * (len(cs) - 1))], cs[int(0.997 * (len(cs) - 1))])
    return cache[j]


def same_subject(c: Corpus, ids: list[str], o: str) -> tuple[float, list[str], str | None]:
    """How strongly joke o shares the board's subject: rare shared words (title/summary/text) plus vector
    similarity when both have vectors. Returns (0..1, shared words, the board joke it is closest to)."""
    words, idf = _subject_index(c)
    ow = words.get(o, set())
    best, best_words, best_j = 0.0, [], None
    for j in ids:
        shared = sorted((w for w in ow & words.get(j, set()) if w in idf), key=lambda w: (-idf[w], w))
        lex = min(1.0, sum(idf[w] for w in shared[:6]) / 12.0) if len(shared) >= 2 else 0.0
        cs, (lo, hi) = c.cos(j, o), _cos_band(c, j)
        # bge cosines sit in a narrow band (0.5-0.8 for almost any two bits), so score against j's own
        # distribution: its 97th percentile -> 0, its 99.7th -> 1
        sem = (cs - lo) / (hi - lo) if cs is not None and hi > lo and cs > lo else 0.0
        sc = max(lex, sem)
        if sc > best:
            best, best_words, best_j = min(1.0, sc), shared[:3], j
    return best, best_words, best_j


def best_next(c: Corpus, board: dict, cap: int = 3) -> list[dict]:
    ids = [j for j in board["joke_ids"] if j in c.jokes]
    on = set(ids)
    pool = {}

    def bump(j, s, why, src):
        if j in on or j not in c.jokes or c.stage(j)[0] == "retired":
            return
        p = pool.setdefault(j, {"score": 0.0, "why": [], "source": "proposed"})
        p["score"] += s
        p["why"].append(why)
        if src == "his" or (src == "inferred" and p["source"] == "proposed"):
            p["source"] = src

    if ids:
        last = ids[-1]
        for (a, b), rows in c.back_to_back.items():
            if a == last:
                bump(b, 0.5 + 0.1 * len(rows), f"done right after {c.title(last)} {len(rows)}x on tape", "inferred")
        for e in c.edges_by_node.get(last, []):
            if e.get("kind") == "segue" and e["src"] == last:
                bump(e["dst"], 0.4 * (e.get("confidence") or 0.5), "segue: " + (e.get("reason") or "")[:90], c.source_of(e))
            elif e.get("kind") in ("same_premise", "callback_to"):
                o = e["dst"] if e["src"] == last else e["src"]
                bump(o, 0.25, f"{e['kind'].replace('_', ' ')} with {c.title(last)}", c.source_of(e))
        for o in c.jokes:
            sp = shared_labels(c, last, o, ("persona",), 0.5)
            if sp and o != last:
                bump(o, 0.2, f"same world as {c.title(last)} ({c.labels[sp[0]]['name']})", "proposed")
        recent = [c.energy(j)[0] for j in ids[-2:]]
        sagging = all(e is not None and e < 1.0 for e in recent)
    else:
        sagging = False
    # same subject as the board (the comic 2026-10-26: pulling his old jokes on the same premise in is the point,
    # so subject beats "strong on tape")
    topical = False
    if ids:
        for o in c.jokes:
            if o in on:
                continue
            sc, ws, near = same_subject(c, ids, o)
            if sc >= 0.35:
                topical = True
                bump(o, 0.6 * sc, f"same subject as {c.title(near)}" + (f" ({', '.join(ws)})" if ws else ""), "inferred")
    # performance energy: anything taped, scaled by how it plays against the room
    for o in c.perfs:
        e, n = c.energy(o)
        if e is None:
            continue
        if e >= CLOSER_STRONG and n >= ROOMS_MIN:
            bump(o, (0.1 if topical else 0.25) + (0.1 if sagging else 0), f"strong on tape ({e:.2f}x the room over {n} nights)"
                 + ("; lifts the energy" if sagging else ""), "inferred")
        elif o in pool:
            pool[o]["score"] += 0.05 * min(e, 2)
    out = []
    for j, p in pool.items():
        t = transition(c, ids[-1], j) if ids else None
        e, n = c.energy(j)
        out.append({"joke_id": j, "title": c.title(j), "stage": c.stage(j)[0], "score": round(min(1.0, p["score"]), 3),
                    "why": "; ".join(dict.fromkeys(p["why"]))[:200], "source": p["source"],
                    "transition": t["label"] if t else None, "energy": e, "nights": n,
                    "evidence": (t or {}).get("evidence", [])[:1]})
    out.sort(key=lambda x: (-x["score"], x["joke_id"]))
    return out[:cap]


# ============================================================== bridge drafts
BRIDGE_SCHEMA = {"type": "object", "properties": {"bridge": {"type": "string"},
                                                  "uses": {"type": "array", "items": {"type": "string"}}},
                 "required": ["bridge", "uses"], "additionalProperties": False}
DRAFT_LABEL = "DRAFT · AI · not your words yet"


def _creative_complete():
    from engine.config import Config
    from engine.providers import Router
    r = Router(Config(), log_path=creative_dir() / "runs.jsonl")
    return lambda task, messages, schema: r.complete(task, messages, schema, run_id="creative")


def _edge_text(c: Corpus, jid: str, end: bool) -> str:
    ps = c.perfs.get(jid, [])
    if ps:
        t = c.text_of(ps[-1]["seg_ids"][-1 if end else 0]) or ""
    else:
        u = c.lead_unit(jid)
        t = (c.nodes.get(u) or {}).get("text") or ""
    return suffix_words(t, 30) if end else prefix_words(t, 30)


def bridge_draft(c: Corpus, a: str, b: str, complete=None) -> dict:
    t = transition(c, a, b)
    links = []
    for l in shared_labels(c, a, b, ("persona",), 0.4) + shared_labels(c, a, b, ("topic",), 0.4):
        ev = []
        for j in (a, b):
            w_e = c.joke_labels[j].get(l)
            if w_e:
                ev += c.edge_evidence(w_e[1], 1)
        links.append({"id": f"L{len(links) + 1}", "kind": c.labels[l]["kind"], "label_id": l,
                      "name": c.labels[l]["name"], "evidence": ev[:2]})
    for e in _direct_edges(c, a, b)[:2]:
        links.append({"id": f"L{len(links) + 1}", "kind": e["kind"], "edge_id": e["id"], "name": e.get("reason"),
                      "evidence": c.edge_evidence(e, 2)})
    base = {"from": a, "to": b, "from_title": c.title(a), "to_title": c.title(b), "transition": t["label"],
            "draft": True, "label": DRAFT_LABEL}
    if not links:
        return dict(base, status="no_grounding", text=None, grounded_in=[],
                    why="No shared persona, topic or link between these two; a bridge would be invented. "
                        "Add a link or pick a different neighbour.")
    examples = []
    for e in c.edges:
        if e.get("kind") == "segue" and c.source_of(e) != "proposed":
            ev = c.edge_evidence(e, 2)
            if len(ev) == 2:
                examples.append(f"- '{ev[0]['quote']}' -> '{ev[1]['quote']}'")
        if len(examples) >= 3:
            break
    msg = (f"Write ONE short spoken bridge line (max 30 words) that {COMIC_FIRST} could say between two of his bits.\n"
           f"End of '{c.title(a)}': \"{_edge_text(c, a, True)}\"\n"
           f"Start of '{c.title(b)}': \"{_edge_text(c, b, False)}\"\n"
           f"Ground it ONLY in these links between the two bits (cite their ids in `uses`):\n"
           + "\n".join(f"{x['id']}: {x['kind']} = {x['name']}" for x in links)
           + ("\nHow his real transitions on tape sound:\n" + "\n".join(examples) if examples else "")
           + "\nFirst person, his conversational voice, no new premise, no punchline of its own.")
    complete = complete or _creative_complete()
    try:
        r = complete("creative_bridge", [{"role": "system", "content": "You draft transitions for one comedian's own "
                                          "set. You never invent facts about his life beyond the links given."},
                                         {"role": "user", "content": msg}], BRIDGE_SCHEMA)
    except Exception as ex:  # provider down: say so, never fake a draft
        return dict(base, status="provider_error", text=None, grounded_in=links, why=f"{type(ex).__name__}: {str(ex)[:160]}")
    data = r.data or {}
    known = {x["id"] for x in links}
    uses = [u for u in data.get("uses") or [] if u in known] or [links[0]["id"]]
    return dict(base, status="ok", text=(data.get("bridge") or "").strip(),
                grounded_in=[x for x in links if x["id"] in uses], all_links=[x["id"] for x in links],
                provider=getattr(r, "provider", None), model=getattr(r, "model", None),
                call_id=getattr(r, "call_id", None), task="creative_bridge")


# ============================================================== seed + export
def _order_chain(c: Corpus, ids: list[str]) -> list[str]:
    """Deterministic greedy chain: strongest taped bit closes; open on the best-connected bit;
    then always take the next bit with the best transition (proven > link > jump), energy breaks ties."""
    if len(ids) < 3:
        return sorted(ids)
    tw = {"proven": 3.0, "link": 2.0, "jump": 0.0}
    energy = {j: c.energy(j)[0] for j in ids}
    taped = [j for j in ids if energy[j] is not None]
    closer = max(taped, key=lambda j: (energy[j], j)) if taped else None
    rest = [j for j in ids if j != closer]

    def degree(j):
        return sum(tw[transition(c, j, o)["label"]] for o in rest if o != j)

    chain = [max(rest, key=lambda j: (degree(j), energy[j] or 0, j))]
    left = [j for j in rest if j != chain[0]]
    while left:
        cur = chain[-1]
        nxt = max(left, key=lambda j: (tw[transition(c, cur, j)["label"]], energy[j] or 0, j))
        chain.append(nxt)
        left.remove(nxt)
    return chain + ([closer] if closer else [])


def seed_from_insights(c: Corpus, store: Store, only_fits: bool = True, ts: str | None = None) -> list[dict]:
    out = []
    for ch in (c.insights.get("chunks") or {}).get("chunks", []):
        if only_fits and not ch.get("fits_5_15"):
            continue
        ids = [j for j in dict.fromkeys(c.map_joke(x) for x in ch.get("jokes", [])) if j in c.jokes]
        if len(ids) < 2:
            continue
        sid = f"sb:seed-{slug(ch.get('name') or 'chunk', 28)}-{sha8(ch.get('jokes'))}"
        if sid in store.items:
            b = store.items[sid]
            order = _order_chain(c, ids)
            # re-order an AI seed only while the comic has never touched it
            if b.get("source") != "his" and not b.get("deleted") and b["joke_ids"] != order:
                b = set_jokes(store, sid, order, by="ai:seed", ts=ts)
            out.append(b)
            continue
        mins = ch.get("minutes_est") or 5
        target = 300 if mins <= 6.5 else 600 if mins <= 11 else 900
        b = create(store, f"{ch.get('name')}", _order_chain(c, ids), target_s=target,
                   notes=f"Seeded from your Insights chunk '{ch.get('name')}' ({mins} min estimated; "
                         f"{ch.get('jokes_with_measured_minutes', 0)} bits measured on tape). Order is a draft.",
                   by="ai:seed", ts=ts, id_=sid, seed={"from": "insights.chunks", "chunk": ch.get("name"),
                                                       "minutes_est": mins})
        out.append(b)
    return out


def export_setlist(c: Corpus, board: dict) -> dict:
    lines, total = [], 0.0
    for i, j in enumerate(board["joke_ids"], 1):
        d, _ = c.duration(j)
        total += d
        lines.append(f"{i}. {c.title(j)} ({mmss(d)})")
    return {"board": board["id"], "title": board["title"], "lines": lines, "runtime": mmss(total),
            "text": f"{board['title']} · {mmss(total)}\n" + "\n".join(lines)}


def board_view(c: Corpus, board: dict) -> dict:
    """Everything the Storyboard screen needs in one payload."""
    h = health(c, board)
    return {"board": board, "health": h, "best_next": best_next(c, board), "setlist": export_setlist(c, board)}
