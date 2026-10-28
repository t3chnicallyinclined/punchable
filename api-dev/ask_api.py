"""Ask routes: a conversation with his own notebook (docs/API_CONTRACT.md "Ask"), backed by creative/askchat.py.

    GET   /api/ask/chats                 his past conversations, newest first
    POST  /api/ask/chats {question}      a new conversation with its first question
    GET   /api/ask/chats/<id>            one conversation (finished answers are gated and stored as they arrive)
    POST  /api/ask/chats/<id> {question} a follow-up in that conversation
    PATCH /api/ask/chats/<id> {title?, deleted?}   rename, or hide it from the list (the log keeps it)

One turn: this process retrieves his sources (creative.askchat.retrieve over the API's own corpus) and builds the
prompt; the model call is a job (`ask_chat`) for the home worker, so claude runs only on his PC; when the job is done
this process runs the gate (creative.askchat.finish) against ITS corpus text: a quote that is not word for word in the
source it cites is dropped, a source that was not retrieved is cut, and with nothing left the answer is "I couldn't
find that in your notebook." Nothing retrieved = that answer at once, no job.

Storage: the `chats` store (creative/store.py: comedy/kb/creative/chats.jsonl + snapshot; on SurrealDB each event is
also a creative_event row, like merges). His questions are his; answers are stored with `by: ai:ask`.
"""
from __future__ import annotations

import os
import re
import sys
import time
from pathlib import Path
from urllib.parse import unquote

HERE = Path(__file__).resolve().parent
DESK = HERE.parent
if str(DESK) not in sys.path:
    sys.path.insert(0, str(DESK))

import creative_api as CR  # noqa: E402
from creative import askchat as AC  # noqa: E402
from creative.store import Store  # noqa: E402
from creative.util import now_iso, sha8  # noqa: E402

OWNER_ID = os.environ.get("PK_OWNER_ID", "owner")  # the actor id on your own edits (`by`); PK_OWNER_ID

STORE = "chats"
MAX_Q = 1000
MAX_TURNS = 60
ID_RE = re.compile(r"^chat:[a-f0-9]{8,16}$")


def chat_apply(items: dict, ev: dict) -> None:
    """Ops: new {title} · ask {turn, q, job, sources} · answer {turn, result} · fail {turn, error} · patch {title?, deleted?}."""
    op, i, d = ev["op"], ev["id"], ev.get("data") or {}
    if op == "new":
        if i in items:
            raise ValueError(f"chat {i} exists")
        items[i] = {"id": i, "title": str(d.get("title") or "")[:120], "created_at": ev["ts"], "updated_at": ev["ts"],
                    "deleted": False, "turns": []}
        return
    ch = items.get(i)
    if ch is None:
        raise ValueError(f"no chat {i}")
    if op == "ask":
        q = d.get("q")
        if not isinstance(q, str) or not q.strip():
            raise ValueError("q is required")
        if any(t["id"] == d.get("turn") for t in ch["turns"]):
            raise ValueError(f"turn {d.get('turn')} exists")
        ch["turns"].append({"id": d["turn"], "q": q, "status": "thinking", "asked_at": ev["ts"], "job": d.get("job"),
                            "sources": list(d.get("sources") or []), "result": None, "error": None})
    elif op in ("answer", "fail"):
        t = next((t for t in ch["turns"] if t["id"] == d.get("turn")), None)
        if t is None:
            raise ValueError(f"no turn {d.get('turn')}")
        if t["status"] != "thinking":
            raise ValueError(f"turn {t['id']} is {t['status']}")
        if op == "answer":
            t.update(status="done", result=d.get("result"), done_at=ev["ts"])
        else:
            t.update(status="failed", error=str(d.get("error") or "failed")[:500], done_at=ev["ts"])
    elif op == "patch":
        if "title" in d:
            title = str(d["title"] or "").strip()
            if not title or len(title) > 120:
                raise ValueError("title must be 1-120 characters")
            ch["title"] = title
        if "deleted" in d:
            ch["deleted"] = bool(d["deleted"])
    else:
        raise ValueError(f"unknown op {op}")
    ch["updated_at"] = ev["ts"]


CR.APPLY.setdefault(STORE, chat_apply)   # LineStore (the SurrealDB mirror) looks the apply function up by name


class HttpError(CR.HttpError):
    pass


def store(cr, kb) -> Store:
    dl = cr.reconcile(kb, STORE)
    if dl is None:
        return Store(cr.dir, STORE, chat_apply)
    m = cr._mirror(kb)

    def writer(name, seq, line, ev, m=m):
        if name in cr.conflicts:
            raise CR.HttpError(409, cr.conflicts[name] + "; writes are refused until they agree")
        m.create(name, seq, line, ev)
        CR._append_file(cr.dir / f"{name}.jsonl", line)
    return CR.LineStore(cr.dir, STORE, dl, writer)


def worker_status(cr) -> dict:
    """Is the writer there to answer? The hosted worker on the server (creative_api.worker_status: name, last
    heartbeat, models, a budget pause); local dev with in-process jobs always is."""
    return CR.worker_status(cr)


def _history(ch: dict) -> list[dict]:
    out = []
    for t in ch["turns"]:
        if t["status"] != "done":
            continue
        r = t.get("result") or {}
        out.append({"q": t["q"], "answer": r.get("answer"), "cited": [x["source"] for x in r.get("chips") or []]})
    return out


def turn_view(t: dict) -> dict:
    r = t.get("result") or {}
    out = {"id": t["id"], "q": t["q"], "status": t["status"], "asked_at": t["asked_at"], "done_at": t.get("done_at")}
    if t["status"] == "done":
        out.update(answer=r.get("answer"), found=bool(r.get("found")), chips=r.get("chips") or [],
                   dropped_n=sum(1 for x in r.get("dropped") or [] if x.get("quote")),
                   model=r.get("model"), provider=r.get("provider"))
    elif t["status"] == "failed":
        out["error"] = t.get("error")
    return out


def chat_view(ch: dict, cr) -> dict:
    return {"id": ch["id"], "title": ch["title"], "created_at": ch["created_at"], "updated_at": ch["updated_at"],
            "turns": [turn_view(t) for t in ch["turns"]], "worker": worker_status(cr)}


def _plain_error(err: str) -> str:
    e = (err or "").lower()
    if "rate limit" in e or "usage limit" in e or "ratelimited" in e:
        return "Claude is at its usage limit on your PC right now. Ask again in a bit."
    if "timeout" in e or "timed out" in e:
        return "Your PC took too long to answer. Ask again."
    return "Your PC couldn't answer that one. Ask again."


def settle(cr, kb, st: Store, ch: dict) -> dict:
    """Thinking turns whose job has finished: gate the answer (or record the failure) now."""
    pend = [t for t in ch["turns"] if t["status"] == "thinking" and t.get("job")]
    if not pend:
        return ch
    jobs = cr.stores(kb)["jobs"]
    c = None
    for t in pend:
        j = jobs.get(t["job"])
        if j is None:
            st.append("fail", ch["id"], {"turn": t["id"], "error": "The question was lost. Ask again."}, by="ai:ask")
        elif j["status"] == "done":
            c = c or cr.corpus(kb)
            res = j.get("result") or {}
            out = AC.finish(c, t["sources"], res.get("data"), res.get("provider"), res.get("model"))
            st.append("answer", ch["id"], {"turn": t["id"], "result": out}, by="ai:ask")
        elif j["status"] == "failed":
            st.append("fail", ch["id"], {"turn": t["id"], "error": _plain_error(j.get("error") or "")}, by="ai:ask")
    return st.get(ch["id"])


def ask(cr, kb, st: Store, chat_id: str, question: str) -> dict:
    ch = st.get(chat_id)
    if len(ch["turns"]) >= MAX_TURNS:
        raise HttpError(400, f"a conversation holds at most {MAX_TURNS} questions; start a new one")
    if any(t["status"] == "thinking" for t in ch["turns"]):
        raise HttpError(409, "still thinking about the last question")
    c = cr.corpus(kb)
    hist = _history(ch)
    turn = f"t{len(ch['turns']) + 1}"
    ids = AC.retrieve(c, question, hist)
    if not ids:
        st.append("ask", chat_id, {"turn": turn, "q": question, "job": None, "sources": []}, by=OWNER_ID)
        st.append("answer", chat_id, {"turn": turn, "result": AC.refusal()}, by="ai:ask")
        return st.get(chat_id)
    msgs = AC.prompt(c, question, hist, ids)
    _, q = cr.enqueue(cr.stores(kb), "ask_chat", f"ask_chat|{chat_id}|{turn}", {"messages": msgs}, kb=kb)
    st.append("ask", chat_id, {"turn": turn, "q": question, "job": q["job"]["id"], "sources": ids}, by=OWNER_ID)
    return st.get(chat_id)


def _question(b: dict) -> str:
    q = b.get("question")
    if not isinstance(q, str) or not q.strip() or len(q) > MAX_Q:
        raise HttpError(400, f"question is required (at most {MAX_Q} characters)")
    return " ".join(q.split())


def handle(cr, h, method: str, path: str, q: dict, body_fn) -> bool:
    if not path.startswith("/api/ask/chats"):
        return False
    kb = h.kb
    try:
        m = re.match(r"^/api/ask/chats(?:/([^/]+))?$", path)
        if not m:
            raise HttpError(404, "no such route")
        cid = unquote(m.group(1)) if m.group(1) is not None else None
        if cid is not None and not ID_RE.match(cid):
            raise HttpError(404, "no such conversation")
        body = body_fn() if method in ("POST", "PATCH") else {}
        if not isinstance(body, dict):
            raise HttpError(400, "body must be an object")
        with cr.lock:
            st = store(cr, kb)
            if cid is None and method == "GET":
                out = []
                for ch in st.items.values():
                    if ch.get("deleted"):
                        continue
                    ch = settle(cr, kb, st, ch)
                    last = ch["turns"][-1] if ch["turns"] else None
                    out.append({"id": ch["id"], "title": ch["title"], "updated_at": ch["updated_at"],
                                "questions": len(ch["turns"]), "last_q": last["q"] if last else None,
                                "thinking": bool(last and last["status"] == "thinking")})
                out.sort(key=lambda x: (x["updated_at"], x["id"]), reverse=True)
                return _send(h, 200, {"chats": out, "worker": worker_status(cr)})
            if cid is None and method == "POST":
                question = _question(body)
                cid = "chat:" + sha8({"q": question, "at": now_iso(), "n": len(st.items)})
                title = question if len(question) <= 60 else question[:59].rstrip() + "…"
                st.append("new", cid, {"title": title}, by=OWNER_ID)
                ch = ask(cr, kb, st, cid, question)
                return _send(h, 201, chat_view(ch, cr))
            ch = st.get(cid)
            if ch is None or ch.get("deleted") and method != "PATCH":
                raise HttpError(404, "no such conversation")
            if method == "GET":
                return _send(h, 200, chat_view(settle(cr, kb, st, ch), cr))
            if method == "POST":
                ch = ask(cr, kb, st, cid, _question(body))
                return _send(h, 200, chat_view(ch, cr))
            if method == "PATCH":
                d = {k: body[k] for k in ("title", "deleted") if k in body}
                if not d:
                    raise HttpError(400, "nothing to change (title, deleted)")
                try:
                    st.append("patch", cid, d, by=OWNER_ID)
                except ValueError as e:
                    raise HttpError(400, str(e))
                return _send(h, 200, chat_view(st.get(cid), cr))
            raise HttpError(405, "method not allowed")
    except CR.HttpError as e:
        h.err(e.status, str(e))
    return True


def _send(h, status, obj) -> bool:
    h.send_json(obj, status)
    return True
