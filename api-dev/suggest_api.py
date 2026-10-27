""""Not a joke?" suggestions: voice memos the triage judge reads as a conversation or a work/personal note.

    GET  /api/suggestions/not-a-joke              {items: [{joke_id, title, text, kind, why, confidence, date}], count}
    POST /api/suggestions/not-a-joke/hide {joke_id}   his one tap: the idea is hidden (a decision line BY HIM, undoable)
    POST /api/suggestions/not-a-joke/keep {joke_id}   "it is a joke": never suggested again (a `keep` event, his)

Where they come from: creative/memolink.py `triage` (Claude, offline, run by the home worker) writes
kb/creative/suggest_judge.json {items: {memo key: {kind, confidence, why, joke_id, ...}}}. A memo counts when its
kind is conversation or note (or "unclear" with almost no words) at >= NOTJOKE_MIN (creative/memolink.flagged). Nothing is ever hidden or deleted automatically: the list only
suggests, and the hide is his own decision (`by` = his id), which /api/undo reverses.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
if str(HERE.parent) not in sys.path:
    sys.path.insert(0, str(HERE.parent))

import creative_api as CR  # noqa: E402
import kb as K  # noqa: E402
from creative.memolink import NOTJOKE_MIN, flagged  # noqa: E402
from creative.store import Store  # noqa: E402

STORE = "suggestions"
ID_RE = re.compile(r"^joke:[A-Za-z0-9:_@.+\-]{1,290}$")


def store_apply(items: dict, ev: dict) -> None:
    """keep {kind: not_a_joke} / unkeep: his answer to a suggestion, per joke."""
    op, i = ev["op"], ev["id"]
    if not ID_RE.match(i or ""):
        raise ValueError("id must be a joke id")
    if op == "keep":
        items[i] = {"joke_id": i, "kept": True, "kind": (ev.get("data") or {}).get("kind") or "not_a_joke",
                    "by": ev.get("by"), "at": ev["ts"]}
    elif op == "unkeep":
        items.pop(i, None)
    else:
        raise ValueError(f"unknown op {op}")


CR.APPLY.setdefault(STORE, store_apply)


def store(cr, kb) -> Store:
    dl = cr.reconcile(kb, STORE)
    if dl is None:
        return Store(cr.dir, STORE, store_apply)
    m = cr._mirror(kb)

    def writer(name, seq, line, ev, m=m):
        if name in cr.conflicts:
            raise CR.HttpError(409, cr.conflicts[name] + "; writes are refused until they agree")
        m.create(name, seq, line, ev)
        CR._append_file(cr.dir / f"{name}.jsonl", line)
    return CR.LineStore(cr.dir, STORE, dl, writer)


def judged(cr) -> dict:
    p = Path(cr.dir) / "suggest_judge.json"
    try:
        return json.loads(p.read_text(encoding="utf-8")).get("items") or {}
    except (OSError, ValueError):
        return {}


def items(cr, kb) -> list[dict]:
    v = kb.view()  # hidden jokes are not in it: once he hides one, it leaves the list
    kept = store(cr, kb).items
    out = []
    for memo, x in judged(cr).items():
        jid = x.get("joke_id")
        if not flagged(x):
            continue
        j = v.nodes.get(jid) if jid else None
        if not j or j.get("type") != "joke" or jid in kept:
            continue
        text = " ".join((j.get("summary") or "").split())
        out.append({"joke_id": jid, "memo": memo, "title": j.get("title"), "text": text[:400] + ("…" if len(text) > 400 else ""),
                    "kind": x["kind"], "why": x.get("why"), "confidence": x.get("confidence"),
                    "source": "inferred"})
    return sorted(out, key=lambda r: (-(r["confidence"] or 0), r["joke_id"]))


def handle(cr, h, method: str, path: str, q: dict, body_fn) -> bool:
    if not path.startswith("/api/suggestions/not-a-joke"):
        return False
    kb = h.kb
    try:
        if method == "GET" and path == "/api/suggestions/not-a-joke":
            xs = items(cr, kb)
            h.send_json({"items": xs, "count": len(xs), "rule": f"a voice memo Claude reads as a conversation or a "
                         f"note (confidence >= {NOTJOKE_MIN}); nothing is hidden until you tap Hide"})
            return True
        if method != "POST" or path not in ("/api/suggestions/not-a-joke/hide", "/api/suggestions/not-a-joke/keep"):
            raise CR.HttpError(404, "not found")
        b = body_fn()
        jid = b.get("joke_id")
        if not isinstance(jid, str) or not ID_RE.match(jid):
            raise CR.HttpError(400, "joke_id is required")
        if path.endswith("/keep"):
            with cr.lock:
                st = store(cr, kb)
                if jid not in st.items:
                    st.append("keep", jid, {"kind": "not_a_joke"}, by=h.actor({}))
            h.send_json({"ok": True, "joke_id": jid, "kept": True})
            return True
        r = kb.decide({"target": "node", "action": "edit", "id": jid, "after": {"hidden": True},
                       "reason": "not a joke (Punchable suggested it; you hid it)"}, by=h.actor({}))
        h.send_json({"ok": True, "joke_id": jid, "indexes": [r["index"]], "decisions": [r["decision"]],
                     "undo": {"POST": "/api/undo", "indexes": [r["index"]]}})
        return True
    except K.DecisionError as e:
        h.err(e.status, str(e))
    except CR.HttpError as e:
        h.err(e.status, str(e))
    return True
