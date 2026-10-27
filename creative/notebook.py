"""Simple Punchable (docs/SIMPLE.md): his own material answers first, in his words and in plain language.

    related(c, text)       "From your notebook": his bits about an idea, verbatim, dated, proven on stage first
    stage_words(c, jid)    one line of stage history in words ("Killed at The Corner Room, Jun 2015")
    joke_map(c, jid)       a joke in the middle and its connections, grouped, each with a plain-words reason
    build(c, board)        the set builder: runtime from his tapes (estimates marked), weak transitions, best next

Copy rule (SIMPLE.md principle 6): nothing here returns "edges", "inferred", "proposed", "vs_room", "confidence",
"pipeline" or "units" in a field the app shows (`why`, `words`, `stage_line`). Ratios never reach a `why`.
Every `quote` is an exact substring of `quote_node`'s text (his doc, or the tape transcript); the app adds the "…".
"""
from __future__ import annotations

import math
import re
from collections import Counter

from . import connections as CN
from . import story as S
from .data import RATING_ORDER, Corpus, venue_short
from .util import mmss

MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
RATING_WORD = {"killed": "Killed", "worked": "Worked", "meh": "Got some laughs", "died": "Went quiet"}
PROVEN = ("killed", "worked")
KEEP_SHORT = {"ai", "tv", "dj", "pc", "uk", "la", "ny", "gf", "bf", "ex", "go", "uk", "us"}
STOP = set("""the and that this with you your for are was but not what when they them then there their have has had
just like its it's i'm don't can't about from all one get got know out who how why would could should will into
been were did does doing very really because also more some any him her his she he our we us my me too than
only even ever every said says say going gonna want make made thing things people time good bad yeah okay
lol im dont cant ill ive youre theyre thats whats wanna gotta kinda sorta well oh uh um guys man way see look""".split())
STOP -= {"us"}
# words that pass the filters but never name a subject (kept out of every "Same subject: ..." the app shows)
VAGUE = set("""let lets till until example instantly actually probably basically literally whatever someone something
anything everything nothing maybe still first last never always back new old day days year years guy guys thing
kind sort lot lots stuff place part point fact case yes yeah nope tried try trying told tell asked ask went goes
came put took take saw seen looked feel felt thought guess wanted need needed wait hey hello two three four five six
seven eight nine ten twenty thirty forty fifty hundred thousand million""".split())
_TOK = re.compile(r"[a-z0-9][a-z0-9']*")
_SENT = re.compile(r"[^.!?\n]+[.!?]*")
QUOTE_MAX = 200
BAND = 120  # a set builder target of 12:00 means "10-12 min": the low end is 2 minutes under (1 under 8 min)


# ============================================================== words
def _norm(w: str) -> str:
    w = w.strip("'")
    if w.endswith("'s"):
        w = w[:-2]
    if len(w) > 4 and w.endswith("ies"):
        return w[:-3] + "y"
    if len(w) > 4 and re.search(r"(ss|x|ch|sh)es$", w):
        return w[:-2]
    if len(w) > 3 and w.endswith("s") and not w.endswith("ss"):
        return w[:-1]
    return w


def tokens(text: str) -> list[str]:
    out = []
    for w in _TOK.findall((text or "").lower()):
        w = _norm(w)
        if (len(w) >= 3 or w in KEEP_SHORT) and w not in STOP:
            out.append(w)
    return out


def short_title(t: str | None) -> str:
    return (t or "").split(" (")[0].strip()


SHORT_NAME_MAX = 18


def short_name_of(title: str | None, custom: str | None = None) -> str:
    """The name on a paper set list: his own (`short_name` on the joke) when he set one, else the title cut at
    its first parenthesis and at a word break inside 18 characters ("Haunted dishwasher saga" -> "Haunted dishwasher")."""
    if isinstance(custom, str) and custom.strip():
        return custom.strip()
    t = short_title(title) or (title or "").strip()
    if len(t) <= SHORT_NAME_MAX:
        return t
    cut = t[:SHORT_NAME_MAX + 1]
    sp = cut.rfind(" ")
    out = (cut[:sp] if sp >= 4 else t[:SHORT_NAME_MAX]).rstrip(" ,.;:!?-")
    return out or t[:SHORT_NAME_MAX]


def month_year(date: str | None) -> str:
    """'2015-06-26' -> 'Jun 2015'; '2015' -> '2015'."""
    d = date or ""
    if len(d) >= 7 and d[5:7].isdigit() and 1 <= int(d[5:7]) <= 12:
        return f"{MONTHS[int(d[5:7]) - 1]} {d[:4]}"
    return d[:4]


# ============================================================== stage history in words
def stage_words(c: Corpus, jid: str) -> tuple[str | None, bool, int]:
    """(one line, proven, nights). 'Killed at The Corner Room, Jun 2015' (+ ' · 3 nights'). None when never on tape."""
    ps = c.perfs.get(jid) or []
    if not ps:
        return None, False, 0
    best = max(ps, key=lambda p: (RATING_ORDER.get(p.get("ai_rating") or "died", 0), p.get("vs_room") or 0, p["date"]))
    name = venue_short(best.get("venue"), None).strip()
    where = "on stage" if not name or name.startswith("Unknown") else f"at {name}"
    line = f"{RATING_WORD.get(best.get('ai_rating') or 'died', 'On stage')} {where}, {month_year(best['date'])}"
    if len(ps) > 1:
        line += f" · {len(ps)} nights"
    return line, any((p.get("ai_rating") in PROVEN) for p in ps), len(ps)


# ============================================================== verbatim lines
def _texts(c: Corpus, jid: str) -> list[tuple[str, str, str | None]]:
    """(node, text, date) for each of his written versions of the joke, lead first. His own idea (added in the app,
    no doc yet) is its summary: those words are his."""
    out = []
    lead = c.lead_unit(jid)
    units = ([lead] if lead else []) + sorted((u for u in c.joke_units.get(jid, []) if u != lead),
                                              key=lambda u: (c.nodes[u].get("date") or "", u), reverse=True)
    for u in units:
        n = c.nodes.get(u) or {}
        if n.get("text"):
            out.append((u, n["text"], n.get("date")))
    j = c.jokes.get(jid) or {}
    if not out and jid in c.his_nodes and j.get("summary"):
        out.append((jid, j["summary"], (j.get("date") or j.get("first_seen") or j.get("ts") or "")[:10] or None))
    return out


def _sentences(text: str):
    for m in _SENT.finditer(text):
        s = m.group(0).strip()
        if len(s.split()) >= 3:
            yield s


def _clip(sentence: str, terms: set[str]) -> tuple[str, bool, bool]:
    """Cut a long sentence to <= QUOTE_MAX chars around the first matching word, at word boundaries (verbatim)."""
    if len(sentence) <= QUOTE_MAX:
        return sentence, False, False
    low = sentence.lower()
    at = min([low.find(t) for t in terms if low.find(t) >= 0] or [0])
    start = max(0, at - 60)
    if start:
        sp = sentence.find(" ", start)
        start = sp + 1 if 0 <= sp < at else start
    end = min(len(sentence), start + QUOTE_MAX)
    if end < len(sentence):
        sp = sentence.rfind(" ", start, end)
        end = sp if sp > start else end
    return sentence[start:end].strip(), start > 0, end < len(sentence)


def best_line(c: Corpus, jid: str, terms: set[str] | None = None) -> dict | None:
    """The line to show for a bit: with an idea's words, the sentence that shares the most of them; else the reveal
    sentence; else the punch line from his best night on tape; else his first full sentence. Always verbatim."""
    terms = terms or set()
    reveal = ((c.jokes.get(jid) or {}).get("anatomy") or {}).get("reveal") or ""
    best, best_sc = None, -1.0
    for node, text, date in _texts(c, jid):
        for k, s in enumerate(_sentences(text)):
            ws = set(tokens(s))
            sc = len(ws & terms) * 2.0
            if reveal and reveal.lower() in s.lower():
                sc += 1.5
            sc -= 0.01 * k  # earlier wins a tie
            if node == (c.lead_unit(jid) or node):
                sc += 0.1
            if sc > best_sc:
                best, best_sc = (node, s, date), sc
    if best is not None and (best_sc >= 1.0 or not c.perfs.get(jid)):
        node, s, date = best
        q, cut_l, cut_r = _clip(s, terms)
        return {"quote": q, "quote_node": node, "date": date, "cut_start": cut_l, "cut_end": cut_r, "from": "doc"}
    # a taped bit with no matching written line: its punch line on his best night (the transcript, verbatim)
    ps = sorted(c.perfs.get(jid) or [], key=lambda p: (RATING_ORDER.get(p.get("ai_rating") or "died", 0), p["date"]))
    for p in reversed(ps):
        for pl in reversed(p.get("punchlines") or []):
            t = (pl.get("text") or "").strip()
            seg = pl.get("seg_id")
            if t and seg and t in (c.text_of(seg) or ""):
                q, cut_l, cut_r = _clip(t, terms)
                return {"quote": q, "quote_node": seg, "date": p["date"], "cut_start": cut_l, "cut_end": cut_r,
                        "from": "stage"}
    if best is not None:
        node, s, date = best
        q, cut_l, cut_r = _clip(s, terms)
        return {"quote": q, "quote_node": node, "date": date, "cut_start": cut_l, "cut_end": cut_r, "from": "doc"}
    return None


def card(c: Corpus, jid: str, terms: set[str] | None = None, **extra) -> dict:
    """A bit as the app shows it anywhere: short title, his line, its date, one stage line."""
    n = c.jokes.get(jid) or {}
    line = best_line(c, jid, terms) or {}
    st_line, proven, nights = stage_words(c, jid)
    d, basis = c.duration(jid)
    out = {"joke_id": jid, "title": short_title(n.get("title")), "short_name": short_name_of(n.get("title"), n.get("short_name")),
           "stage": c.stage(jid)[0],
           "quote": line.get("quote"), "quote_node": line.get("quote_node"), "cut_start": line.get("cut_start", False),
           "cut_end": line.get("cut_end", False),
           "date": line.get("date") or (n.get("first_seen") or "")[:10] or None,
           "stage_line": st_line, "proven": proven, "nights": nights,
           "duration_s": d, "estimated": not nights}
    out.update(extra)
    return out


# ============================================================== "From your notebook"
def _index(c: Corpus):
    idx = c.__dict__.get("_nb_index")
    if idx is not None:
        return idx
    docs, title_terms, df = {}, {}, Counter()
    for j, n in c.jokes.items():
        t = tokens(n.get("title") or "")
        parts = [n.get("summary") or ""] + [txt for _, txt, _ in _texts(c, j)]
        tf = Counter(t * 2 + tokens(" ".join(parts)))
        docs[j] = tf
        title_terms[j] = set(t)
        df.update(set(tf))
    avg = (sum(sum(tf.values()) for tf in docs.values()) / max(1, len(docs))) or 1.0
    idx = (docs, title_terms, df, avg)
    c.__dict__["_nb_index"] = idx
    return idx


def lexical(c: Corpus, text: str) -> list[tuple[str, float]]:
    """BM25 over his titles, summaries and written versions. A word on more than a quarter of the jokes says
    nothing about the idea and is skipped."""
    docs, title_terms, df, avg = _index(c)
    n = max(1, len(docs))
    lines = [x for x in (text or "").splitlines() if x.strip()]
    head = set(tokens(lines[0])) if len(lines) > 1 else set()
    q = [w for w in dict.fromkeys(tokens(text)) if 0 < df.get(w, 0) <= max(3, n // 4)]
    if not q:
        return []
    # the first line is the premise, the lines under it are examples: premise words count double
    weight = {w: (1.0 if not head or w in head else 0.5) for w in q}
    out = []
    for j, tf in docs.items():
        dl = sum(tf.values()) or 1
        s = 0.0
        for w in q:
            f = tf.get(w, 0)
            if f:
                idf = math.log(1 + (n - df[w] + 0.5) / (df[w] + 0.5))
                s += weight[w] * idf * f * 2.2 / (f + 1.2 * (0.25 + 0.75 * dl / avg))
                if w in title_terms[j]:
                    s += weight[w] * 0.5 * idf
        if s > 0:
            out.append((j, s))
    out.sort(key=lambda x: (-x[1], x[0]))
    return out


def related(c: Corpus, text: str, cap: int = 10, hybrid: list[str] | None = None, exclude=()) -> dict:
    """His bits about an idea. Ranked by fusing his-words search (always) with the database's fused text+vector
    search when it is there (`hybrid`, best first); then, among the five closest, the ones that worked on stage
    come first (proven before written). Each item quotes the sentence that shares the most words with the idea."""
    terms = set(tokens(text))
    lex = lexical(c, text)
    fused: dict[str, float] = {}
    for rank, (j, _) in enumerate(lex[:60]):
        fused[j] = fused.get(j, 0.0) + 1.0 / (60 + rank)
    for rank, j in enumerate((hybrid or [])[:12]):
        j = c.map_joke(j) or j
        if j in c.jokes:
            fused[j] = fused.get(j, 0.0) + 1.0 / (60 + rank)
    ex = set(exclude)
    ranked = [j for j, _ in sorted(fused.items(), key=lambda kv: (-kv[1], kv[0])) if j not in ex][:cap]
    items = []
    for r, j in enumerate(ranked):
        it = card(c, j, terms, rank=r)
        it["why"] = _related_why(c, j, terms)
        items.append(it)
    items.sort(key=lambda it: (0 if it["proven"] and it["rank"] < 5 else 1, it["rank"]))
    return {"query": text, "items": items, "total": len(items)}


def _related_why(c: Corpus, jid: str, terms: set[str]) -> str:
    docs = _index(c)[0]
    shared = [w for w in sorted(terms) if docs.get(jid, {}).get(w)]
    return ("Shares: " + ", ".join(shared[:3])) if shared else "Close to the idea"


# ============================================================== joke map
GROUPS = [("next", "Could come next"), ("tags", "Could tag it"), ("same_world", "Same world"),
          ("callbacks", "Callbacks"), ("versions", "Older versions")]


def subject_words(ws) -> list[str]:
    return [w for w in ws or [] if w not in STOP and w not in VAGUE and len(w) >= 3][:3]


def _subject(ws) -> str:
    good = subject_words(ws)
    return "Same subject" + (f": {', '.join(good)}" if good else "")


def _plain_best_next(why: str, last_title: str) -> str:
    """best_next's reasons, in his words: no ratios, no 'x the room'."""
    parts = []
    for p in (why or "").split("; "):
        p = p.strip()
        if p.startswith("same subject as"):
            m = re.search(r"\(([^)]*)\)", p)
            parts.append(_subject(m.group(1).split(", ") if m else []))
        elif p.startswith("done right after"):
            parts.append(f"You did it right after {short_title(last_title)} on stage")
        elif p.startswith("segue"):
            parts.append(f"Could follow {short_title(last_title)}")
        elif p.startswith("same_premise") or p.startswith("same premise"):
            parts.append("Same premise")
        elif p.startswith("callback"):
            parts.append("Calls back")
        elif p.startswith("same world as"):
            m = re.search(r"\(([^)]*)\)", p)
            parts.append("Same world" + (f": {m.group(1)}" if m else ""))
        elif p.startswith("strong on tape"):
            parts.append("Strong on stage")
    return "; ".join(dict.fromkeys(parts)) or "From your notebook"


def _conn_why(it: dict, group: str) -> str:
    basis, src = it.get("basis") or [], it.get("source")
    if "back_to_back" in basis:
        n = it.get("nights") or 0
        return "You did it right after this on stage" + (f", {n} times" if n > 1 else "")
    if group == "next":
        return "You linked these" if src == "his" else "Could lead into it"
    if group == "tags":
        return "You tagged it with this" if src == "his" else "Lands on the same note"
    if group == "callbacks":
        return "It calls back to this" if it.get("direction") == "calls_back_this" else "This calls back to it"
    if group == "same_world":
        names = (it.get("personas") or []) + (it.get("topics") or [])
        if "same_premise" in basis:
            return "You marked it the same premise" if src == "his" else "Same premise"
        if names:
            return "Same world: " + ", ".join(names[:2])
        if "chunk" in basis:
            return "Goes in the same chunk"
    return "Connected"


def joke_map(c: Corpus, jid: str, cap: int = 8) -> dict:
    """The joke page as a map. Same world puts SAME SUBJECT first (the comic 2026-10-26: pulling in his old jokes on
    the same premise is the point), then shared persona/topic; nothing is ranked up for being strong on tape alone."""
    conn = CN.joke_connections(c, jid, cap=cap * 2)
    by = {g["key"]: g["items"] for g in conn["groups"]}
    groups: dict[str, list[dict]] = {k: [] for k, _ in GROUPS}
    seen: dict[str, set] = {k: {jid} for k, _ in GROUPS}

    def put(g, oid, why, mine=False, **extra):
        key = extra.get("unit_id") or oid
        if not oid or key in seen[g] or len(groups[g]) >= cap or (oid not in c.jokes and g != "tags"):
            return
        seen[g].add(key)
        it = card(c, oid, None, why=why, mine=mine, **extra) if oid in c.jokes else {"joke_id": None, "why": why, "mine": mine, **extra}
        groups[g].append(it)

    # could come next: proven on stage and his own links first, then the board engine's picks (subject first)
    nexts = sorted(by.get("next", []), key=lambda x: (0 if "back_to_back" in (x.get("basis") or []) else
                                                      1 if x.get("source") == "his" else 2, -x["score"]))
    for it in nexts:
        put("next", it.get("joke_id"), _conn_why(it, "next"), mine=it.get("source") == "his",
            proven_pair="back_to_back" in (it.get("basis") or []))
    for x in S.best_next(c, {"joke_ids": [jid]}, cap=cap):
        put("next", x["joke_id"], _plain_best_next(x["why"], c.title(jid)), proven_pair=x.get("transition") == "proven")

    # could tag it: his tags first, then lines that land on the same note (the reveal query)
    for it in sorted(by.get("tags", []), key=lambda x: (0 if x.get("source") == "his" else 1, -x["score"])):
        u, line = it.get("unit_id"), it.get("line")
        if not u or not line or line not in (c.text_of(u) or ""):
            continue
        owner = it.get("joke_id") or c.unit_joke.get(u)
        if owner and owner in c.jokes:
            put("tags", owner, _conn_why(it, "tags"), mine=it.get("source") == "his", unit_id=u)
            if groups["tags"] and groups["tags"][-1].get("unit_id") == u:
                g = groups["tags"][-1]
                g.update({"quote": line, "quote_node": u, "cut_start": False, "cut_end": False,
                          "date": (c.nodes.get(u) or {}).get("date") or g.get("date")})

    # same world: same subject (rare shared words + similarity), then persona/topic, then same chunk
    subj = []
    for o in c.jokes:
        if o == jid:
            continue
        sc, ws, _ = S.same_subject(c, [jid], o)
        if sc >= 0.35:
            subj.append((sc, o, ws))
    subj.sort(key=lambda x: (-x[0], x[1]))
    for sc, o, ws in subj:
        put("same_world", o, _subject(ws), subject=round(sc, 3))
    for it in sorted(by.get("same_world", []), key=lambda x: (0 if x.get("source") == "his" else 1, -x["score"])):
        put("same_world", it.get("joke_id"), _conn_why(it, "same_world"), mine=it.get("source") == "his")

    for it in by.get("callbacks", []):
        put("callbacks", it.get("joke_id"), _conn_why(it, "callbacks"), mine=it.get("source") == "his")

    # older versions: his other written versions of this bit, newest first (the lead version is the center)
    lead = c.lead_unit(jid)
    for node, text, date in _texts(c, jid):
        if node == lead or len(groups["versions"]) >= cap:
            continue
        s = next(iter(_sentences(text)), text.strip())
        q, cl, cr = _clip(s, set())
        groups["versions"].append({"joke_id": jid, "unit_id": node, "title": (c.nodes.get(node) or {}).get("doc_title")
                                   or "Written version", "quote": q, "quote_node": node, "cut_start": cl, "cut_end": cr,
                                   "date": date, "why": "An older version", "stage_line": None, "proven": False})

    center = card(c, jid)
    center["text"] = (_texts(c, jid) or [(None, "", None)])[0][1]
    return {"joke": center, "groups": [{"key": k, "title": t, "items": groups[k]} for k, t in GROUPS]}


# ============================================================== all bits (the bird's-eye map)
ALL_MAP_KINDS = ("next", "tags", "same_world", "callbacks", "versions")
MAX_EDGES_PER_NODE = 6


def all_map(c: Corpus, cap: int = 400, same_idea: list | None = None) -> dict:
    """Every bit as a node (capped: the top `cap` by nights, then laughs, then recency) and the links between them,
    grouped like the per-joke map: next (his segues, done back to back), tags (a line of one bit tagging another),
    same_world (same premise, a shared persona, the same Insights chunk), callbacks, versions (the merge judge's
    same-idea groups). Names only: no bodies. Cheap enough to cache per corpus revision."""
    def heat(j):
        vs = [p["vs_room"] for p in c.perfs.get(j, []) if p.get("vs_room") is not None]
        return round(sum(vs) / len(vs), 2) if vs else None

    rows = []
    for j, n in c.jokes.items():
        nights = len({p["set"] for p in c.perfs.get(j, [])})
        rows.append((nights, heat(j) or 0.0, (n.get("last_seen") or "")[:10], j))
    rows.sort(key=lambda r: (-r[0], -r[1], r[2], r[3]))
    rows.sort(key=lambda r: (-r[0], -r[1], r[3]))
    rows = sorted(rows, key=lambda r: (-r[0], -r[1]))  # stable: ties keep (last_seen, id) order from above
    keep = [r[3] for r in rows[:cap]]
    ks = set(keep)
    edges: dict[tuple, dict] = {}

    def link(a, b, kind, w, why=None):
        if a == b or a not in ks or b not in ks:
            return
        k = (min(a, b), max(a, b), kind)
        e = edges.get(k)
        if e is None or w > e["w"]:
            edges[k] = {"a": k[0], "b": k[1], "kind": kind, "w": round(float(w), 3), "why": why}

    for e in c.edges:
        k, src, dst = e.get("kind"), e["src"], e["dst"]
        conf = float(e.get("confidence") or 0.5)
        his = c.source_of(e) == "his"
        if k == "segue":
            link(src, dst, "next", 1.0 if his else conf, "Your segue" if his else "Could lead in")
        elif k == "same_premise":
            link(src, dst, "same_world", 1.0 if his else conf, "Same premise")
        elif k == "callback_to":
            link(src, dst, "callbacks", 1.0 if his else conf, "Calls back")
        elif k == "tag_of" and src.startswith("unit:"):
            owner = c.unit_joke.get(src)
            if owner and owner != dst:
                link(owner, dst, "tags", 1.0 if his else conf, "A line of it tags this")
    for (a, b), nights in c.back_to_back.items():
        link(a, b, "next", min(1.0, 0.55 + 0.15 * len(nights)), f"Done back to back on stage, {len(nights)}x")
    # a shared persona: each bit links to at most 4 others carrying the same persona label (strongest first)
    by_persona: dict[str, list] = {}
    for j in keep:
        for l, (w, _e) in c.labels_of(j, ("persona",), 0.5).items():
            by_persona.setdefault(l, []).append((float(w), j))
    for l, members in by_persona.items():
        members.sort(key=lambda x: (-x[0], x[1]))
        name = (c.labels.get(l) or {}).get("name") or "persona"
        for i, (w, j) in enumerate(members[:40]):
            for w2, o in members[i + 1:i + 5]:
                link(j, o, "same_world", 0.4 + 0.3 * min(w, w2), f"Same world: {name}")
    for ch in (c.insights.get("chunks") or {}).get("chunks", []):
        ms = [m for m in (c.map_joke(x) for x in ch.get("jokes", [])) if m in ks][:12]
        for i, j in enumerate(ms):
            for o in ms[i + 1:i + 4]:
                link(j, o, "same_world", 0.35, f"Same chunk: {ch.get('name')}")
    for g in same_idea or []:
        ms = [m["joke_id"] for m in g.get("members", [])]
        for i, j in enumerate(ms):
            for o in ms[i + 1:]:
                link(j, o, "versions", 0.9, "Looks like the same idea")
    # cap: at most MAX_EDGES_PER_NODE links per bit (strongest kept), then nodes * 4 overall
    deg: dict[str, int] = {}
    out_edges = []
    for e in sorted(edges.values(), key=lambda x: (-x["w"], x["a"], x["b"], x["kind"])):
        if deg.get(e["a"], 0) >= MAX_EDGES_PER_NODE or deg.get(e["b"], 0) >= MAX_EDGES_PER_NODE:
            continue
        deg[e["a"]] = deg.get(e["a"], 0) + 1
        deg[e["b"]] = deg.get(e["b"], 0) + 1
        out_edges.append(e)
        if len(out_edges) >= len(keep) * 4:
            break
    nodes = []
    for nights, h, last, j in rows[:cap]:
        n = c.jokes[j]
        nodes.append({"id": j, "short_name": short_name_of(n.get("title"), n.get("short_name")),
                      "title": short_title(n.get("title")), "stage": c.stage(j)[0], "nights": nights,
                      "heat": h or None, "last_seen": last or None, "degree": deg.get(j, 0)})
    by_kind = {k: 0 for k in ALL_MAP_KINDS}
    for e in out_edges:
        by_kind[e["kind"]] += 1
    return {"cap": cap, "nodes": nodes, "edges": out_edges,
            "counts": {"jokes": len(c.jokes), "nodes": len(nodes), "edges": len(out_edges), "by_kind": by_kind},
            "groups": [{"key": k, "title": t} for k, t in GROUPS if k in ALL_MAP_KINDS]}


# ============================================================== set builder
def target_band(target_s: int) -> tuple[int, int]:
    t = int(target_s or 0)
    return max(60, t - (BAND if t >= 480 else 60)), t


def _transition_words(t: dict) -> tuple[str, bool]:
    """(words, weak). Weak = nothing links the two, or only a shared world with no line between them yet."""
    if t["label"] == "proven":
        return "Done back to back on stage", False
    via = t.get("via") or []
    if t["label"] == "link":
        kinds = [v.get("kind") for v in via]
        if "segue" in kinds:
            v = via[kinds.index("segue")]
            return ("Your segue" if v.get("source") == "his" else "Could lead in"), False
        if "callback_to" in kinds:
            return "Callback", False
        if "same_premise" in kinds:
            return "Same premise", False
        if "persona" in kinds or "topic" in kinds:
            names = [v.get("name") for v in via if v.get("kind") in ("persona", "topic") and v.get("name")]
            return "Same world" + (f": {', '.join(names[:2])}" if names else "") + ", needs a line between", True
        if "subject" in kinds:
            ws = next((v.get("words") for v in via if v.get("kind") == "subject"), None) or []
            return _subject(ws) + ", needs a line between", True
        return "Linked", bool(t.get("needs_bridge"))
    return "No link yet", True


def build(c: Corpus, board: dict, cap: int = 5) -> dict:
    """A board as a set: runtime summed from his tapes where known (estimates marked), the target band, every
    transition in words with the weak ones flagged, and the best next bits from his notebook."""
    h = S.health(c, board)
    lo, hi = target_band(board.get("target_s") or 0)
    steps = []
    for i, b in enumerate(h["bits"]):
        t = h["transitions"][i - 1] if i else None
        words, weak = _transition_words(t) if t else (None, False)
        steps.append(card(c, b["joke_id"], None, position=i,
                          transition_in=None if not t else {"words": words, "weak": weak, "kind": t["label"],
                                                            "from": t["from"], "to": t["to"]}))
    total = h["runtime_s"]
    if not hi:
        status, words = "none", f"{mmss(total)}"
    elif total < lo:
        status, words = "short", f"{mmss(lo - total)} to fill"
    elif total > hi:
        status, words = "over", f"{mmss(total - hi)} over"
    else:
        status, words = "on", "on target"
    ids = [s["joke_id"] for s in steps]
    last = c.title(ids[-1]) if ids else ""
    nxt = [card(c, x["joke_id"], None, why=_plain_best_next(x["why"], last), proven_pair=x.get("transition") == "proven")
           for x in S.best_next(c, board, cap=cap)]
    notes = []
    for ch in h["checks"]:
        if ch["rule"] == "closer" and ch["status"] == "warn" and ids:
            sug = ch.get("suggest")
            notes.append({"rule": "closer", "text": "The ending could be stronger"
                          + (f": try closing on {short_title(c.title(sug))}" if sug else ""), "suggest": sug})
        if ch["rule"] == "energy_dip" and ch["status"] == "warn":
            a, b = ch["pairs"][0]
            notes.append({"rule": "dip", "text": f"Two quiet bits in a row: {short_title(c.title(a))}, {short_title(c.title(b))}"})
    return {"board": {"id": board["id"], "title": board.get("title"), "target_s": hi, "target_lo_s": lo},
            "runtime_s": total, "runtime": mmss(total), "estimated_n": sum(1 for s in steps if s["estimated"]),
            "status": status, "status_words": words, "weak_n": sum(1 for s in steps if (s["transition_in"] or {}).get("weak")),
            "steps": steps, "next": nxt, "notes": notes}
