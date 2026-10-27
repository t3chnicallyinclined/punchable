"""Ask: a conversation with his own notebook (the "ask" pattern, multi-turn).

    sources(c)                       every quotable thing he has: his written bits, his own ideas, notes, and every
                                     set moment (a transcript segment with its show, date, room and laughs)
    retrieve(c, question, history)   the sources for one turn: his-words search (BM25), a room named in the question
                                     pulls that room's sets, a year narrows, "killed / best" ranks by laughs, and the
                                     sources the last answer cited are carried so "which of those killed?" works
    prompt(c, question, history, ids)  the messages for the model (ids = the retrieved sources, numbered S1..Sn)
    finish(c, ids, data)             the gate: every quote must be word for word in the source it cites (else the
                                     quote is dropped), every [S#] must be a retrieved source (else it is cut),
                                     "quoted" prose that is in no source is cut, and with nothing left the answer is
                                     "I couldn't find that in your notebook." — never a guess
    run_job(inp, complete)           the home worker's half: one model call, nothing else (the API retrieves and gates)

The answer only ever comes from sources that were retrieved and shown to the model. Metadata lines (dates, rooms,
"Killed · 6 laughs") are computed from his tapes; quotes come only from a source's TEXT.
Copy rule (docs/SIMPLE.md): chips and refusals are plain words; no ratios, ids or "units" reach the screen.
"""
from __future__ import annotations

import os
import math
import re
from collections import Counter

from .data import RATING_ORDER, Corpus, venue_short
from .notebook import RATING_WORD, _texts, month_year, short_title, stage_words, tokens
from .util import mmss

COMIC = os.environ.get("PK_COMIC_NAME", "the comic")  # how the AI prompts name you; PK_COMIC_NAME
COMIC_FIRST = os.environ.get("PK_COMIC_FIRST", COMIC)

REFUSAL = "I couldn't find that in your notebook."
TASK = "ask_chat"
MAX_SOURCES = 24
PER_JOKE = 3
SNIPPET = 1100
HISTORY_TURNS = 4
QUOTE_MIN = 3
SCHEMA = {"type": "object", "properties": {
    "found": {"type": "boolean"},
    "answer": {"type": "string"},
    "citations": {"type": "array", "items": {"type": "object", "properties": {
        "source": {"type": "string"}, "quote": {"type": "string"}},
        "required": ["source", "quote"], "additionalProperties": False}}},
    "required": ["found", "answer", "citations"], "additionalProperties": False}
# words of the question that say what kind of answer he wants, not what it is about
QSTOP = set("""written write wrote writing bit bits joke jokes material notebook killed kill kills killing worked work
works died die bombed bomb best worst strongest weakest funniest laugh laughs laughed stage performed perform set sets
show shows night nights ever which what where when who how why did does have has any all tell list find give show
those these them that this there their one ones said say talk talked mention mentioned stuff""".split())
RATING_WANT = {"killed": "killed", "kill": "killed", "kills": "killed", "best": "killed", "strongest": "killed",
               "funniest": "killed", "hit": "killed", "hits": "killed", "worked": "worked", "work": "worked",
               "laugh": "killed", "laughs": "killed", "died": "died", "bombed": "died", "bomb": "died",
               "worst": "died", "weakest": "died", "quiet": "died", "flopped": "died"}
YEAR_RE = re.compile(r"\b(19[89]\d|20[0-4]\d)\b")
MARK_RE = re.compile(r"\[\s*(S\d+(?:\s*[,;]\s*S\d+)*)\s*\]", re.I)
_TYPO = str.maketrans({"‘": "'", "’": "'", "‚": "'", "‛": "'", "′": "'", "“": '"',
                       "”": '"', "„": '"', "″": '"', "–": "-", "—": "-", "−": "-",
                       " ": " "})


# ============================================================== verbatim (engine/verify.py's rule, kept here because
# the hosted API image ships creative/ and not engine/)
def _norm_map(s: str):
    out, idx, prev_sp = [], [], False
    for i, ch in enumerate(s):
        if ch == "…":
            out += "..."
            idx += [i, i, i]
            prev_sp = False
            continue
        c = ch.translate(_TYPO)
        if c.isspace():
            if not prev_sp:
                out.append(" ")
                idx.append(i)
            prev_sp = True
        else:
            out.append(c)
            idx.append(i)
            prev_sp = False
    return "".join(out), idx


def verbatim(quote: str, source: str) -> str | None:
    """The exact source substring for `quote`, or None. Only typography may differ (curly quotes, dashes, runs of
    spaces, an ellipsis character); the returned text is always the source's own bytes. Case and words must match."""
    if not quote or not source:
        return None
    q = quote.strip()
    if q in source and len(q) >= QUOTE_MIN:
        return q
    q = q.strip("\"'“”‘’").strip()
    q = re.sub(r"^(\.\.\.|…)\s*|\s*(\.\.\.|…)$", "", q)
    if len(q) < QUOTE_MIN:
        return None
    if q in source:
        return q
    ns, idx = _norm_map(source)
    nq = _norm_map(q)[0].strip()
    k = ns.find(nq) if len(nq) >= QUOTE_MIN else -1
    if k < 0:
        return None
    return source[idx[k]:idx[k + len(nq) - 1] + 1]


# ============================================================== sources
def _moment_words(seg: dict) -> str:
    st = seg.get("status")
    if st == "banter":
        return "Talking to the room"
    if st == "crowd_work":
        return "Crowd work"
    n = (seg.get("laughs") or {}).get("count") or 0
    word = RATING_WORD.get(seg.get("ai_rating") or "", "On stage")
    return f"{word} · {n} laugh{'s' if n != 1 else ''}" if n else word


def sources(c: Corpus) -> dict[str, dict]:
    """Every quotable source, keyed by the node its quote is checked against (unit:…, joke:… for his own idea typed
    in the app, seg:<set>#<idx> for a moment on tape). Cached on the corpus."""
    got = c.__dict__.get("_ask_sources")
    if got is not None:
        return got
    out: dict[str, dict] = {}
    for jid, j in c.jokes.items():
        line, _, nights = stage_words(c, jid)
        for node, text, date in _texts(c, jid):
            n = c.nodes.get(node) or {}
            if node == jid:
                kind = "idea"
            elif n.get("type") == "unit" and (n.get("source") or "gdoc") != "gdoc":
                kind = "note"
            else:
                kind = "bit"
            out[node] = {"id": node, "kind": kind, "text": text, "title": short_title(j.get("title")) or "Untitled",
                         "date": (date or "")[:10] or None, "joke_id": jid, "stage_line": line, "nights": nights,
                         "set_id": None, "start_ms": None, "venue": None, "rating": None}
    for key, segs in c.segments.items():
        s = c.sets.get(key) or {}
        date = (c.metas.get(key) or {}).get("show_date") or s.get("date") or key[:10]
        room = venue_short(s.get("venue"), None)
        for seg in segs:
            t = (seg.get("transcript") or "").strip()
            if not t:
                continue
            jid = seg.get("joke") if seg.get("status") in ("known", "new") else None
            title = short_title(c.title(jid)) if jid and jid in c.jokes else (seg.get("title") or "On stage")
            out[seg["seg_id"]] = {
                "id": seg["seg_id"], "kind": "moment", "text": t, "title": title, "date": date[:10],
                "joke_id": jid if jid in c.jokes else None, "set_id": f"set:{key}", "start_ms": seg.get("start_ms"),
                "venue": room, "rating": seg.get("ai_rating") if seg.get("status") in ("known", "new") else None,
                "status": seg.get("status"), "words": _moment_words(seg),
                "laughs": (seg.get("laughs") or {}).get("count") or 0, "stage_line": None, "nights": None}
    c.__dict__["_ask_sources"] = out
    return out


def _index(c: Corpus):
    idx = c.__dict__.get("_ask_index")
    if idx is not None:
        return idx
    S = sources(c)
    tfs, df = {}, Counter()
    for sid, s in S.items():
        tf = Counter(tokens(s["title"]) * 2 + tokens(s["text"]))
        tfs[sid] = tf
        df.update(set(tf))
    avg = (sum(sum(t.values()) for t in tfs.values()) / max(1, len(tfs))) or 1.0
    idx = (tfs, df, avg)
    c.__dict__["_ask_index"] = idx
    return idx


def _bm25(c: Corpus, text: str) -> dict[str, float]:
    tfs, df, avg = _index(c)
    n = max(1, len(tfs))
    q = [w for w in dict.fromkeys(tokens(text)) if w not in QSTOP and 0 < df.get(w, 0) <= max(3, n // 4)]
    out: dict[str, float] = {}
    for sid, tf in tfs.items():
        dl = sum(tf.values()) or 1
        s = 0.0
        for w in q:
            f = tf.get(w, 0)
            if f:
                idf = math.log(1 + (n - df[w] + 0.5) / (df[w] + 0.5))
                s += idf * f * 2.2 / (f + 1.2 * (0.25 + 0.75 * dl / avg))
        if s > 0:
            out[sid] = s
    return out


def rooms_named(c: Corpus, question: str) -> set[str]:
    """Set keys whose room is named in the question ("The Corner Room", "Mabel's", "Riverside", "AC")."""
    qw = set(re.findall(r"[a-z0-9']+", question.lower()))
    ql = " " + " ".join(re.findall(r"[a-z0-9']+", question.lower())) + " "
    out = set()
    for key, s in c.sets.items():
        name = venue_short(s.get("venue"), None).rstrip("?").lower()
        if not name or name.startswith("unknown"):
            continue
        words = [w for w in re.findall(r"[a-z0-9']+", name) if w not in ("the", "comedy", "club", "on")]
        if words and (" " + " ".join(re.findall(r"[a-z0-9']+", name)) + " " in ql or all(w in qw for w in words)):
            out.add(key)
        elif name == "seaside" and ({"ac"} & qw or "seaside" in ql):
            out.add(key)
    return out


def retrieve(c: Corpus, question: str, history: list[dict] | None = None, cap: int = MAX_SOURCES) -> list[str]:
    """The source ids for one turn, best first. `history` = earlier turns [{q, cited: [source ids]}], oldest first."""
    S = sources(c)
    history = (history or [])[-HISTORY_TURNS:]
    score: dict[str, float] = {}

    def add(sid, v):
        if sid in S:
            score[sid] = score.get(sid, 0.0) + v

    for sid, v in _bm25(c, question).items():
        add(sid, v)
    if history:  # a follow-up's subject is usually in the questions before it
        for sid, v in _bm25(c, " ".join(h.get("q") or "" for h in history)).items():
            add(sid, 0.4 * v)
    carried = [sid for sid in (history[-1].get("cited") or [])] if history else []
    carried_jokes = {S[x]["joke_id"] for x in carried if x in S and S[x].get("joke_id")}
    for sid in carried:
        add(sid, 6.0)
    qw = [w.strip("'") for w in re.findall(r"[a-z']+", question.lower())]
    want = next((RATING_WANT[w] for w in qw if w in RATING_WANT), None)
    rooms = rooms_named(c, question)
    for sid, s in S.items():
        if s["kind"] != "moment":
            continue
        if rooms and s["set_id"][4:] in rooms:
            add(sid, 5.0 + (0.5 if s.get("joke_id") else 0.0))
        if carried_jokes and s.get("joke_id") in carried_jokes and (want or rooms):
            add(sid, 4.0)  # "which of those killed?": their nights on tape
        if want and sid in score and s.get("rating"):
            r = RATING_ORDER.get(s["rating"], 0)
            add(sid, (r if want != "died" else 3 - r) * 0.8 + min(10, s.get("laughs") or 0) * 0.05)
    years = {int(y) for y in YEAR_RE.findall(question)}
    if years:
        score = {k: v for k, v in score.items() if (S[k].get("date") or "")[:4].isdigit()
                 and int(S[k]["date"][:4]) in years} or score
    if rooms:  # a question about a room is answered from that room's sets (and what was already on screen)
        score = {k: v for k, v in score.items() if (S[k]["kind"] == "moment" and S[k]["set_id"][4:] in rooms)
                 or k in carried} or score
    picked, per = [], Counter()
    for sid in sorted(score, key=lambda k: (-score[k], k)):
        s = S[sid]
        j = s.get("joke_id") or sid
        if per[(j, s["kind"] == "moment")] >= PER_JOKE and sid not in carried:
            continue
        per[(j, s["kind"] == "moment")] += 1
        picked.append(sid)
        if len(picked) >= cap:
            break
    return picked


# ============================================================== prompt
def _snippet(text: str, terms: set[str], n: int = SNIPPET) -> str:
    if len(text) <= n:
        return text
    low = text.lower()
    hits = sorted(p for t in terms for p in [low.find(t)] if p >= 0)
    at = max(0, (hits[0] if hits else 0) - n // 4)
    if at:
        sp = text.find(" ", at)
        at = sp + 1 if sp >= 0 else at
    end = min(len(text), at + n)
    sp = text.rfind(" ", at, end)
    end = sp if end < len(text) and sp > at else end
    return ("[...] " if at else "") + text[at:end] + (" [...]" if end < len(text) else "")


def header(s: dict) -> str:
    if s["kind"] == "moment":
        at = mmss((s.get("start_ms") or 0) / 1000)
        bit = f" · bit: {s['title']}" if s.get("joke_id") else f" · {s['title']}"
        return f"set moment · {s.get('venue') or 'a show'} · {s.get('date') or 'undated'} · at {at}{bit} · {s.get('words')}"
    kind = {"bit": "written bit", "idea": "his idea (typed in the app)", "note": "voice note"}[s["kind"]]
    st = f" · on stage: {s['stage_line']}" if s.get("stage_line") else " · not on tape"
    return f"{kind} · \"{s['title']}\" · {s.get('date') or 'undated'}{st}"


SYSTEM = ("You answer stand-up comic " + COMIC + "'s questions about his OWN material: his written bits, his ideas, "
          "his voice notes and the transcripts of his recorded sets (with the room, the date and the laughs). "
          "Use ONLY the numbered SOURCES in the latest message. Earlier answers in this conversation only tell you "
          "what 'those' or 'it' refers to; they are not evidence. Never use outside knowledge, never guess, never "
          "invent a line, a show or a laugh.")
RULES = ("Rules:\n"
         "- Answer in plain, short sentences, the way a friend who read his notebook would. No tables, no ratios. "
         "Say dates the way a person would (Jun 20, 2015).\n"
         "- Put the source after each claim like [S3]. Every claim needs one.\n"
         "- The line under each source header is computed from his tapes (room, date, how it went, laughs). You may "
         "state it, citing that source.\n"
         "- Quotes go ONLY in `citations`: copy a short line (under 25 words) character for character from that "
         "source's TEXT. Never quote the header line, never fix typos or transcription errors, never join two "
         "passages. Do not put quotation marks in `answer`.\n"
         "- One citation per source you used; `source` is its id, like S3.\n"
         "- If the sources do not answer the question, set found=false, leave `answer` empty and `citations` empty.")


def prompt(c: Corpus, question: str, history: list[dict] | None, ids: list[str]) -> list[dict]:
    S = sources(c)
    terms = {w for w in tokens(question) if w not in QSTOP}
    msgs = [{"role": "system", "content": SYSTEM}]
    for h in (history or [])[-HISTORY_TURNS:]:
        if h.get("q"):
            msgs.append({"role": "user", "content": h["q"]})
            msgs.append({"role": "assistant", "content": h.get("answer") or REFUSAL})
    blocks = []
    for i, sid in enumerate(ids, 1):
        s = S[sid]
        blocks.append(f"[S{i}] {header(s)}\nTEXT: {_snippet(s['text'], terms)}")
    msgs.append({"role": "user", "content": f"QUESTION: {question}\n\nSOURCES:\n\n" + "\n\n".join(blocks)
                 + "\n\n" + RULES})
    return msgs


# ============================================================== the gate
def chip(s: dict, n: int, quote: str | None) -> dict:
    """A tappable source as the app shows it."""
    if s["kind"] == "moment":
        at = mmss((s.get("start_ms") or 0) / 1000)
        href = f"/set/{s['set_id']}?at={int(s.get('start_ms') or 0)}"
        label = f"{s.get('venue') or 'On stage'} · {month_year(s.get('date'))} · {at}"
        sub = f"{s['title']} · {s.get('words')}"
    else:
        href = f"/joke/{s['joke_id']}"
        label = s["title"] + (f" · {month_year(s.get('date'))}" if s.get("date") else "")
        sub = s.get("stage_line") or {"bit": "Written", "idea": "Your idea", "note": "Voice note"}[s["kind"]]
    return {"n": n, "kind": s["kind"], "source": s["id"], "label": label, "sub": sub, "href": href,
            "joke_id": s.get("joke_id"), "set_id": s.get("set_id"), "start_ms": s.get("start_ms"),
            "date": s.get("date"), "venue": s.get("venue"), "quote": quote}


def finish(c: Corpus, ids: list[str], data: dict | None, provider: str | None = None, model: str | None = None) -> dict:
    """The model's answer through the gate. `ids` = the retrieved sources (S1 = ids[0])."""
    S = sources(c)
    data = data if isinstance(data, dict) else {}
    sid_of = {f"S{i}": x for i, x in enumerate(ids, 1) if x in S}
    dropped: list[dict] = []
    quotes: dict[str, str] = {}
    cited_order: list[str] = []
    for cit in data.get("citations") or []:
        if not isinstance(cit, dict):
            continue
        tag = str(cit.get("source") or "").strip().strip("[]").upper()
        sid = sid_of.get(tag)
        if sid is None:
            dropped.append({"source": tag, "quote": (cit.get("quote") or "")[:200], "why": "not a retrieved source"})
            continue
        q = (cit.get("quote") or "").strip()
        exact = verbatim(q, c.text_of(sid) or S[sid]["text"]) if q else None
        if q and exact is None:
            dropped.append({"source": tag, "quote": q[:200], "why": "not word for word in the source"})
        elif exact and sid not in quotes:
            quotes[sid] = exact
        if sid not in cited_order:
            cited_order.append(sid)
    answer = str(data.get("answer") or "").strip()
    # [S#] markers: retrieved ones become [1], [2] … (chip numbers); anything else is cut
    num: dict[str, int] = {}

    def mark(m):
        ns = []
        for tag in re.split(r"\s*[,;]\s*", m.group(1)):
            sid = sid_of.get(tag.upper())
            if sid is None:
                dropped.append({"source": tag, "quote": "", "why": "cited in the answer but not a retrieved source"})
                continue
            if sid not in num:
                num[sid] = len(num) + 1
            ns.append(f"[{num[sid]}]")
        return "".join(ns)
    answer = MARK_RE.sub(mark, answer)
    answer = re.sub(r"\[\s*S\d+[^\]]*\]", "", answer)  # a malformed marker ("[S3a]", "[S3, S4b]") is cut, not shown
    # "quoted" passages in the prose must be in a retrieved source, word for word
    texts = [c.text_of(x) or S[x]["text"] for x in sid_of.values()]

    def q_ok(m):
        q = m.group(2)
        if len(q.split()) < 4 or any(verbatim(q, t) for t in texts):
            return m.group(0)
        dropped.append({"source": None, "quote": q[:200], "why": "quoted in the answer, not word for word in any source"})
        return "[quote removed]"
    answer = re.sub(r'(")([^"\n]{3,600})(")', q_ok, answer)
    answer = re.sub("(“)([^“”\n]{3,600})(”)", q_ok, answer)
    for sid in cited_order:
        if sid not in num:
            num[sid] = len(num) + 1
    chips = [chip(S[sid], n, quotes.get(sid)) for sid, n in sorted(num.items(), key=lambda kv: kv[1])]
    found = bool(data.get("found")) and bool(chips) and bool(answer.strip())
    out = {"status": "done", "found": found, "dropped": dropped, "provider": provider, "model": model,
           "retrieved": len(ids)}
    if not found:
        out.update(answer=REFUSAL, chips=[])
    else:
        out.update(answer=re.sub(r"[ \t]{2,}", " ", answer).strip(), chips=chips)
    return out


def refusal(ids: list[str] | None = None) -> dict:
    """Nothing retrieved: answered here, no model call."""
    return {"status": "done", "found": False, "answer": REFUSAL, "chips": [], "dropped": [], "provider": None,
            "model": None, "retrieved": len(ids or [])}


def ask_local(c: Corpus, question: str, history: list[dict] | None = None, complete=None) -> dict:
    """One whole turn in one process (the CLI and the tests): retrieve → model → gate."""
    ids = retrieve(c, question, history)
    if not ids:
        return refusal()
    if complete is None:
        from .story import _creative_complete
        complete = _creative_complete()
    r = complete(TASK, prompt(c, question, history, ids), SCHEMA)
    out = finish(c, ids, getattr(r, "data", None), getattr(r, "provider", None), getattr(r, "model", None))
    out["sources"] = ids
    return out


# ============================================================== the home worker's half
def run_job(inp: dict, complete=None) -> dict:
    """worker/execute.py, kind `ask_chat`: one model call over the messages the API built. The schema is this
    module's, never the job's. The API gates the result (finish) when it reads it back."""
    msgs = inp.get("messages")
    if not isinstance(msgs, list) or not msgs or not all(isinstance(m, dict) and m.get("role") in
                                                          ("system", "user", "assistant") and isinstance(m.get("content"), str)
                                                          for m in msgs):
        raise ValueError("bad ask_chat input")
    if complete is None:
        from .story import _creative_complete
        complete = _creative_complete()
    r = complete(TASK, msgs, SCHEMA)
    return {"status": "answered", "data": getattr(r, "data", None) or {}, "provider": getattr(r, "provider", None),
            "model": getattr(r, "model", None)}
