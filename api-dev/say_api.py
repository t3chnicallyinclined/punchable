"""Say it or ask it (docs/UX_2026-10.md): one input on every screen. The app does not ask him which it is.

    POST /api/say {text, kind?, context?}      -> {kind: "question"|"bit", intent, guess, overridden, ...}
    GET  /api/tonight/screen                   -> everything the Tonight tab shows, in one response

A question runs the existing ask path (ask_api: a new conversation, the model call is a job for his home PC) and
answers with the conversation (`chat`). A bit runs the existing POST /api/ideas flow (server.Handler.idea_result:
a joke node at stage idea, auto-attach to the bit it clearly belongs to) and answers with the idea (`idea`) and
how to undo it.

Provenance (the review rule): the text is his, the classification is the app's. The classifier's guess is an
`intents` store event by `ai:intent`; when the app sends `kind` (his "Not a question? Save as a bit" tap) that is
an `override` event by him, on top of the guess. The idea or question itself is written by him, as before.
Nothing here records the classifier's guess as his decision.

The classifier is deterministic and cheap (no model): a question mark; a leading wh-word, auxiliary or request
("build me", "give me"); "my material / my bits / my closer". "what if ..." without a question mark is a premise,
so a bit. Everything else is a bit: a lost punchline costs more than a misfiled question.
"""
from __future__ import annotations

import os
import re
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
DESK = HERE.parent
if str(DESK) not in sys.path:
    sys.path.insert(0, str(DESK))

import creative_api as CR  # noqa: E402
import kb as K  # noqa: E402
from creative import notebook as N  # noqa: E402
from creative.store import Store  # noqa: E402
from creative.util import now_iso, sha8  # noqa: E402

OWNER_ID = os.environ.get("PK_OWNER_ID", "owner")  # the actor id on your own edits (`by`); PK_OWNER_ID
AI_ACTOR = "ai:intent"
STORE = "intents"
KINDS = ("question", "bit")
MAX_TEXT = 4000

# ---------------------------------------------------------------------------------------------------- classifier
FILLERS = ("oh", "and", "so", "um", "uh", "hey", "ok", "okay", "also", "wait", "like", "yeah", "well", "hmm", "but")
QUESTION_STARTS = (
    "what", "whats", "what's", "when", "where", "which", "who", "whos", "who's", "why", "how", "hows", "how's",
    "do", "does", "did", "can", "could", "should", "would", "will", "is", "are", "was", "were", "have", "has", "had",
    "am", "any", "anything", "anyone", "got", "remind", "list", "show", "find", "pull", "give", "build", "make",
    "tell", "search", "look",
)
REQUEST_RE = re.compile(r"^(build|make|give|show|find|pull|put|get|tell|write|new|sort|run)\b.*\b(me|up|my|a|an|the)\b")
NEW_SET_RE = re.compile(r"^(new|a new|build|make)\b.*\b\d+\b")           # "new 7 for friday", "build me a tight 5"
MY_MATERIAL_RE = re.compile(r"\bmy (bits?|material|jokes?|sets?|closer|opener|notebook|stuff|best|strongest|"
                            r"weakest|last set|tight \d+|\d+)\b")
PREMISE_RE = re.compile(r"^(what if|imagine|picture this|you know (how|when|what)|so my|my (dad|mom|wife|husband|"
                        r"girlfriend|boyfriend|kid|kids|son|daughter|brother|sister|roommate|neighbou?r|landlord|"
                        r"boss|doctor|dentist|uber|therapist|cat|dog))\b")


def _words(text: str) -> list[str]:
    return re.findall(r"[a-z0-9']+", text.lower())


def classify(text: str) -> tuple[str, str]:
    """(kind, rule): kind is question or bit; rule names the test that decided, for the record and the tests."""
    t = " ".join((text or "").split())
    if not t:
        return "bit", "empty"
    if t.rstrip().endswith("?"):
        return "question", "question_mark"
    ws = _words(t)
    while ws and ws[0] in FILLERS:
        ws = ws[1:]
    if not ws:
        return "bit", "only_fillers"
    low = " ".join(ws)
    if PREMISE_RE.match(low):
        return "bit", "premise"
    if ws[0] in QUESTION_STARTS and (ws[0] not in ("got", "find", "look", "any") or len(ws) >= 2):
        return "question", "leading_wh_or_aux"
    if REQUEST_RE.match(low) or NEW_SET_RE.match(low):
        return "question", "request"
    if MY_MATERIAL_RE.search(low):
        return "question", "my_material"
    return "bit", "default"


# ---------------------------------------------------------------------------------------------------- the store
def intent_apply(items: dict, ev: dict) -> None:
    """classify {text, kind, rule, context} by ai:intent · override {kind} by him · acted {kind, ref} by him."""
    op, i, d = ev["op"], ev["id"], ev.get("data") or {}
    if op == "classify":
        if i in items:
            raise ValueError(f"intent {i} exists")
        if d.get("kind") not in KINDS:
            raise ValueError("kind must be question or bit")
        if not str(ev.get("by") or "").startswith("ai:"):
            raise ValueError("a classification is the app's judgment: by must be ai:*")
        items[i] = {"id": i, "text": d.get("text") or "", "guess": d["kind"], "rule": d.get("rule"), "kind": d["kind"],
                    "context": d.get("context") or None, "by": ev.get("by"), "at": ev["ts"], "overridden": False,
                    "override_by": None, "ref": None}
        return
    it = items.get(i)
    if it is None:
        raise ValueError(f"no intent {i}")
    if op == "override":
        if d.get("kind") not in KINDS:
            raise ValueError("kind must be question or bit")
        if str(ev.get("by") or "").startswith("ai:"):
            raise ValueError("an override is his tap, never the app's")
        it["kind"], it["overridden"], it["override_by"] = d["kind"], True, ev.get("by")
    elif op == "acted":
        it["ref"] = d.get("ref")
    else:
        raise ValueError(f"unknown op {op}")


CR.APPLY.setdefault(STORE, intent_apply)   # LineStore (the SurrealDB mirror) looks the apply function up by name


def store(cr, kb) -> Store:
    dl = cr.reconcile(kb, STORE)
    if dl is None:
        return Store(cr.dir, STORE, intent_apply)
    m = cr._mirror(kb)

    def writer(name, seq, line, ev, m=m):
        if name in cr.conflicts:
            raise CR.HttpError(409, cr.conflicts[name] + "; writes are refused until they agree")
        m.create(name, seq, line, ev)
        CR._append_file(cr.dir / f"{name}.jsonl", line)
    return CR.LineStore(cr.dir, STORE, dl, writer)


# ---------------------------------------------------------------------------------------------------- the routes
def _text(b: dict) -> str:
    t = b.get("text")
    if not isinstance(t, str) or not t.strip() or len(t) > MAX_TEXT:
        raise CR.HttpError(400, f"text is required (at most {MAX_TEXT} characters)")
    return " ".join(t.split())


def _context(b: dict) -> dict | None:
    c = b.get("context")
    if c is None:
        return None
    if not isinstance(c, dict):
        raise CR.HttpError(400, "context must be an object")
    out = {k: c[k] for k in ("set_id", "joke_id", "board_id", "screen") if isinstance(c.get(k), str) and len(c[k]) <= 300}
    return out or None


def say(cr, h, kb, b: dict) -> tuple[int, dict]:
    import ask_api as AK
    text = _text(b)
    ctx = _context(b)
    want = b.get("kind")
    if want is not None and want not in KINDS:
        raise CR.HttpError(400, "kind must be question or bit")
    guess, rule = classify(text)
    by = h.actor(b)   # who is speaking: him. A named import or ai actor in the body is refused: this is his input
    if by != OWNER_ID:
        raise CR.HttpError(403, "say is the comic's own input")
    st = store(cr, kb)
    iid = "say:" + sha8({"t": text, "at": now_iso(), "n": st.n_events})
    st.append("classify", iid, {"text": text, "kind": guess, "rule": rule, "context": ctx}, by=AI_ACTOR)
    kind = guess
    if want is not None and want != guess:
        st.append("override", iid, {"kind": want}, by=by)
        kind = want
    out = {"kind": kind, "intent": iid, "guess": guess, "rule": rule, "overridden": kind != guess, "text": text,
           "worker": AK.worker_status(cr)}
    if kind == "question":
        chats = AK.store(cr, kb)
        cid = "chat:" + sha8({"q": text, "at": now_iso(), "n": len(chats.items)})
        title = text if len(text) <= 60 else text[:59].rstrip() + "…"
        chats.append("new", cid, {"title": title}, by=by)
        ch = AK.ask(cr, kb, chats, cid, text[:AK.MAX_Q])
        st.append("acted", iid, {"ref": cid}, by=by)
        out["chat"] = AK.chat_view(ch, cr)
        return 201, out
    reason = "said on " + (ctx.get("screen") or "the input bar") if ctx else "said on the input bar"
    idea = h.idea_result({"text": text, "reason": reason}, by=by)
    st.append("acted", iid, {"ref": idea["id"]}, by=by)
    att = idea.get("attach") or {}
    out["idea"] = idea
    out["saved_to"] = {"joke_id": att["into"], "title": att.get("title")} if att.get("decision") == "attach" else None
    out["undo"] = {"POST": "/api/undo", "indexes": idea["indexes"]}
    if att.get("decision") == "attach" and att.get("id"):
        out["undo"]["detach"] = {"POST": "/api/merge/undo", "merge_id": att["id"]}
    return 200, out


def tonight_screen(cr, h, kb) -> dict:
    """GET /api/tonight/screen: the whole Tonight tab from one call (the review rule: one request, no timers)."""
    import ask_api as AK
    import suggest_api as SG
    c, st = cr.corpus(kb), cr.stores(kb)
    board = CR.tonight_board(c, st)
    a = kb.app()
    latest = a.sets[-1] if a.sets else None
    latest_card = None
    if latest:
        sc = a.set_info(latest["id"]) if hasattr(a, "set_info") else {}
        latest_card = {"id": latest["id"], "title": latest.get("title"), "date": latest.get("date"),
                       "venue": latest.get("venue"), "reviewed": bool(sc.get("reviewed")) if sc else None}
    to_sort = len(SG.items(cr, kb))
    # suggested questions, from his own material (no model): the first gap, the closer, something never tried
    qs = []
    if board and board.get("first_gap"):
        g = board["first_gap"]
        qs.append(f"How do I get from {g['from_name']} to {g['to_name']}?")
    qs.append("What's my strongest closer right now?")
    never = [j for j, n in c.jokes.items() if not c.perfs.get(j)]
    if never:
        pick = max(never, key=lambda j: ((c.jokes[j].get("last_seen") or ""), j))
        qs.append(f"Anything I wrote about {N.short_name_of(c.title(pick), c.jokes[pick].get('short_name')).lower()} I never tried?")
    else:
        qs.append("What do I have that works in a quiet room?")
    return {"date": time.strftime("%Y-%m-%d"), "weekday": time.strftime("%A"), "board": board, "to_sort": to_sort,
            "latest_set": latest_card, "questions": qs[:3], "worker": AK.worker_status(cr),
            "counts": {"bits": len(c.jokes), "sets": len(getattr(c, "sets", {}) or {})}}


def handle(cr, h, method: str, path: str, q: dict, body_fn) -> bool:
    if path not in ("/api/say", "/api/tonight/screen"):
        return False
    kb = h.kb
    try:
        if path == "/api/tonight/screen":
            if method != "GET":
                raise CR.HttpError(405, "method not allowed")
            h.send_json(tonight_screen(cr, h, kb))
            return True
        if method != "POST":
            raise CR.HttpError(405, "method not allowed")
        b = body_fn()
        if not isinstance(b, dict):
            raise CR.HttpError(400, "body must be an object")
        with cr.lock:
            status, out = say(cr, h, kb, b)
        h.send_json(out, status)
    except K.DecisionError as e:
        h.err(e.status, str(e))
    except CR.HttpError as e:
        h.err(e.status, str(e))
    return True
