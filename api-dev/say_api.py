"""Say it or ask it (docs/UX_2026-10.md): one input on every screen. The app does not ask him which it is.

    POST /api/say {text, kind?, context?, choice?} -> {kind: "question"|"bit"|"command", intent, guess, overridden, ...}
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
KINDS = ("question", "bit", "command")
MAX_TEXT = 4000
MAX_CANDIDATES = 5

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
    return re.findall(r"[a-z0-9']+", text.lower().replace("’", "'"))


# ---------------------------------------------------------------------------------------------------- commands
# "new 7 for friday" / "new 5" / "make a 10 for the club" / "build me a tight 5" -> a set list of that length;
# "add <bit> to tonight" / "put the kale bit in tonight's set" -> that bit onto tonight's board. The parse is the app's
# guess (ai:intent, like the other kinds); the board or the add is written by him once he confirms (one tap), or at
# once when the command is unambiguous (a number; one clear bit) and the screen shows Undo.
NUM_WORDS = {"one": 1, "two": 2, "three": 3, "four": 4, "five": 5, "six": 6, "seven": 7, "eight": 8, "nine": 9,
             "ten": 10, "twelve": 12, "fifteen": 15, "twenty": 20, "thirty": 30, "forty": 40, "forty-five": 45,
             "forty five": 45, "sixty": 60}
_NUM = r"(\d{1,3}|" + "|".join(sorted(NUM_WORDS, key=len, reverse=True)) + ")"
NEW_SET_CMD = re.compile(
    r"^(?:(?:make|build|start|create|give|set up|setup)(?: me)?|new|a new)\s+(?:(?:a|an|another)\s+)?"
    r"(?:(?:new|tight|quick|fresh|clean|solid|killer)\s+)*" + _NUM +
    r"(?:\s*(?:min|mins|minute|minutes|minuter|m))?(?:\s+(?:set|set list|setlist|list|one|spot))?"
    r"(?:\s+(?:for|at|on)\s+(.+?))?\s*$")
ADD_CMD = re.compile(
    r"^(?:add|put|stick|throw|drop|slot)\s+(?:in\s+)?(?:the\s+)?(.+?)\s+(?:in|into|on|onto|to)\s+"
    r"(?:tonight(?:'s|s)?(?:\s+(?:set\s+list|setlist|set|list))?|(?:the|my)\s+(?:set\s+list|setlist|set|list)(?:\s+(?:for\s+)?tonight)?)"
    r"\s*$")
QUERY_STOP = {"the", "a", "an", "my", "that", "this", "bit", "one", "joke", "thing", "about", "on", "of", "to", "and",
              "with", "it"}


def parse_command(text: str) -> dict | None:
    """The command in a line, or None. {op: new_set, minutes, title} or {op: add_to_tonight, query, words[]}."""
    t = " ".join((text or "").lower().replace("\u2019", "'").split())
    t = t.rstrip(".! ")
    if not t or t.endswith("?"):
        return None
    m = NEW_SET_CMD.match(t)
    if m:
        n = m.group(1)
        mins = int(n) if n.isdigit() else NUM_WORDS.get(n)
        if not mins or not 1 <= mins <= 240:
            return None
        target = (m.group(2) or "").strip(" .,!")
        target = re.sub(r"^(the|a|an)\s+", "", target)
        title = (target[:1].upper() + target[1:]) if target else time.strftime("%A")
        return {"op": "new_set", "minutes": mins, "title": f"{title[:40]} · {mins}"}
    m = ADD_CMD.match(t)
    if m:
        q = m.group(1).strip(" .,!'\"")
        ws = [w for w in _words(q) if w not in QUERY_STOP]
        return {"op": "add_to_tonight", "query": q, "words": ws}
    return None


def match_bits(c, words: list[str], query: str, cap: int = MAX_CANDIDATES) -> list[dict]:
    """Fuzzy match on title and short name (word-prefix overlap, a bonus for the whole query inside the name).
    Sorted best first; `sure` on the first one when it is the one clear hit."""
    if not words and not query:
        return []
    q = " ".join(_words(query))
    rows = []
    for jid, n in c.jokes.items():
        title = n.get("title") or ""
        short = N.short_name_of(title, n.get("short_name"))
        tw = set(_words(title)) | set(_words(short))
        if not tw:
            continue
        hit = 0
        for w in words:
            if any(x == w or (len(w) >= 4 and x.startswith(w)) or (len(x) >= 4 and w.startswith(x)) for x in tw):
                hit += 1
        score = hit / len(words) if words else 0.0
        low = " ".join(_words(title)) + " | " + " ".join(_words(short))
        if q and q in low:
            score += 0.3
        if score <= 0:
            continue
        rows.append({"joke_id": jid, "title": N.short_title(title), "short_name": short, "score": round(min(1.0, score), 3),
                     "nights": len(c.perfs.get(jid) or [])})
    rows.sort(key=lambda r: (-r["score"], -r["nights"], r["joke_id"]))
    rows = rows[:cap]
    if rows:
        top = rows[0]["score"]
        second = rows[1]["score"] if len(rows) > 1 else 0.0
        rows[0]["sure"] = top >= 0.75 and (top - second >= 0.25 or (top >= 1.0 and second < 1.0))
    return rows


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
    cmd = parse_command(low)
    if cmd:
        return "command", "command_" + cmd["op"]
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
            raise ValueError("kind must be question, bit or command")
        if not str(ev.get("by") or "").startswith("ai:"):
            raise ValueError("a classification is the app's judgment: by must be ai:*")
        items[i] = {"id": i, "text": d.get("text") or "", "guess": d["kind"], "rule": d.get("rule"), "kind": d["kind"],
                    "context": d.get("context") or None, "command": d.get("command"), "by": ev.get("by"), "at": ev["ts"],
                    "overridden": False, "override_by": None, "ref": None}
        return
    it = items.get(i)
    if it is None:
        raise ValueError(f"no intent {i}")
    if op == "override":
        if d.get("kind") not in KINDS:
            raise ValueError("kind must be question, bit or command")
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
        raise CR.HttpError(400, "kind must be question, bit or command")
    guess, rule = classify(text)
    cmd = parse_command(text) if (guess == "command" or want == "command") else None
    if want == "command" and not cmd:
        raise CR.HttpError(400, 'not a command I know (try "new 7 for friday" or "add <bit> to tonight")')
    by = h.actor(b)   # who is speaking: him. A named import or ai actor in the body is refused: this is his input
    if by != OWNER_ID:
        raise CR.HttpError(403, "say is the comic's own input")
    st = store(cr, kb)
    iid = "say:" + sha8({"t": text, "at": now_iso(), "n": st.n_events})
    st.append("classify", iid, {"text": text, "kind": guess, "rule": rule, "context": ctx, "command": cmd}, by=AI_ACTOR)
    kind = guess
    if want is not None and want != guess:
        st.append("override", iid, {"kind": want}, by=by)
        kind = want
    out = {"kind": kind, "intent": iid, "guess": guess, "rule": rule, "overridden": kind != guess, "text": text,
           "worker": AK.worker_status(cr)}
    if kind == "command":
        out["command"] = run_command(cr, kb, st, iid, cmd, b, by)
        return 200, out
    if kind == "question":
        chats = AK.store(cr, kb)
        cid = "chat:" + sha8({"q": text, "at": now_iso(), "n": len(chats.items)})
        title = text if len(text) <= 60 else text[:59].rstrip() + "…"
        jk = (ctx or {}).get("joke_id")   # asked from a bit's page: the conversation is about that bit
        jk = jk if isinstance(jk, str) and jk in cr.corpus(kb).jokes else None
        chats.append("new", cid, {"title": title, **({"joke": jk} if jk else {})}, by=by)
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


def _board_brief(b: dict) -> dict:
    return {"id": b["id"], "title": b["title"], "target_s": b.get("target_s"), "n": len(b.get("joke_ids") or [])}


def run_command(cr, kb, st: Store, iid: str, cmd: dict, b: dict, by: str) -> dict:
    """Execute (or ask about) a parsed command. Every write here is his (by = him): after his tap (`choice`, the bit
    he picked when the app asked), or at once when the command is unambiguous and the screen shows Undo. The parse
    itself stays the app's classify event."""
    from creative import story as S
    c, stores = cr.corpus(kb), cr.stores(kb)
    boards = stores["storyboards"]
    out = dict(cmd)
    if cmd["op"] == "new_set":
        rec = S.create(boards, cmd["title"], [], cmd["minutes"] * 60, "", by=by)
        bb = boards.items[rec["id"]]
        st.append("acted", iid, {"ref": bb["id"]}, by=by)
        out.update({"done": True, "board": _board_brief(bb),
                    "undo": {"PATCH": f"/api/boards/{bb['id']}", "body": {"delete": True}}})
        return out
    board = CR.tonight_board(c, stores)
    bid = board["id"] if board else None
    choice = b.get("choice")
    if choice is not None and (not isinstance(choice, str) or choice not in c.jokes):
        raise CR.HttpError(400, "choice must be one of your bits")
    cands = match_bits(c, cmd["words"], cmd["query"])
    out["candidates"] = cands
    out["board"] = {"id": board["id"], "title": board["title"], "target_s": board["target_s"],
                    "n": len(board["segments"])} if board else None
    jid = choice or (cands[0]["joke_id"] if cands and cands[0].get("sure") else None)
    if not jid:
        out["done"] = False
        out["ask"] = "which" if cands else "none"
        return out
    pick = next((x for x in cands if x["joke_id"] == jid), None) or {
        "joke_id": jid, "title": N.short_title(c.title(jid)),
        "short_name": N.short_name_of(c.title(jid), (c.jokes.get(jid) or {}).get("short_name")), "score": None}
    created = False
    if bid is None:
        rec = S.create(boards, f"{time.strftime('%A')} · tonight", [jid], 7 * 60, "", by=by)
        bid, created = rec["id"], True
    else:
        bb = boards.items[bid]
        if jid in (bb.get("joke_ids") or []):
            st.append("acted", iid, {"ref": bid, "joke": jid, "already": True}, by=by)
            out.update({"done": True, "already": True, "joke": pick, "board": _board_brief(bb),
                        "position": bb["joke_ids"].index(jid)})
            return out
        S.add(boards, bid, jid, None, by=by)
    bb = boards.items[bid]
    st.append("acted", iid, {"ref": bid, "joke": jid}, by=by)
    out.update({"done": True, "created": created, "joke": pick, "board": _board_brief(bb),
                "position": bb["joke_ids"].index(jid),
                "undo": {"PATCH": f"/api/boards/{bid}",
                         "body": {"delete": True} if created else {"remove": {"joke_id": jid}}}})
    return out


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
