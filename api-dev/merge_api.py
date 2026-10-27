"""Merge routes for the Punchable API (docs/API_CONTRACT.md "Merge"), backed by creative/merge.py.

    GET  /api/merge/candidates            every same-idea group + the counts
    GET  /api/merge/candidates?joke=<id>  that joke's group (if any) and its closest other jokes
    GET  /api/merge/history?joke=<id>     what was folded into a joke: merges (verbatim, dated) and attached notes
    GET  /api/merge/<merge_id>            one merge or attach record, with its Google Docs status
    POST /api/merge {main, others[], reason?, group?}    the comic taps Merge
    POST /api/merge/undo {merge_id}                       undo a merge (mg:) or detach an auto-attached note (at:)
    POST /api/merge/dismiss {jokes[]}                     "not the same": never suggested together again

What a merge is (both backends, through the normal decision path, so it is his action and can be undone):
- every unit of each other joke has its part_of edge re-pointed to the main (his words keep their dates and docs);
- every performance matched to another joke is re-pointed to the main (nights, laughs and Working follow);
- each other joke gets `hidden: true` (kb.py allows it on jokes for this);
- one `merge` event in comedy/kb/creative/merges.jsonl (creative/store.py; a creative_event row on SurrealDB) with the
  decision indexes and a snapshot of every original's words, verbatim and dated.
Undo = /api/undo of those indexes, newest first, plus an `undo` event. Nothing is deleted.

Auto-attach (POST /api/ideas, which gsync also uses for a new Google Doc): `after_idea` places the new idea when it is
clearly one existing bit (creative.merge.match_text). The attach is the AI's, not his: it is an `attach` event by
`ai:attach` in merges.jsonl, never a decision line. The idea stays a joke (his words, his add line) and is left off the
Desk list while attached; undo (detach) brings it back.

Google Docs: gsync (on his PC) reads merges.json and does the Drive side (gsync/merge.py). This module never calls
Drive.
"""
from __future__ import annotations

import os
import json
import re
import sys
import threading
from pathlib import Path

HERE = Path(__file__).resolve().parent
DESK = HERE.parent
if str(DESK) not in sys.path:
    sys.path.insert(0, str(DESK))

import creative_api as CR  # noqa: E402
import kb as K  # noqa: E402
from creative import merge as MG  # noqa: E402
from creative.store import Store  # noqa: E402
from creative.util import now_iso, sha8  # noqa: E402

OWNER_ID = os.environ.get("PK_OWNER_ID", "owner")  # the actor id on your own edits (`by`); PK_OWNER_ID

STORE = "merges"
CR.APPLY.setdefault(STORE, MG.store_apply)   # LineStore (SurrealDB mirror) looks the apply function up by name
JUDGE_FILE = "merge_judge.json"              # creative/merge.py judge cache (offline: python -m creative merge judge)
MAX_OTHERS = 20
ID_RE = re.compile(r"^[A-Za-z0-9:_@.+\-]{1,300}$")
TAG = "[merge]"                              # reason prefix on every decision line a merge writes

_lock = threading.RLock()
_cache: dict = {}


class HttpError(CR.HttpError):
    pass


# ---------------------------------------------------------------------------------------------------------- state
def store(cr, kb) -> Store:
    """The merges store for this backend (file, or file + creative_event rows on SurrealDB), like Creative.stores."""
    dl = cr.reconcile(kb, STORE)
    if dl is None:
        return Store(cr.dir, STORE, MG.store_apply)
    m = cr._mirror(kb)

    def writer(name, seq, line, ev, m=m):
        if name in cr.conflicts:
            raise CR.HttpError(409, cr.conflicts[name] + "; writes are refused until they agree")
        m.create(name, seq, line, ev)
        CR._append_file(cr.dir / f"{name}.jsonl", line)
    return CR.LineStore(cr.dir, STORE, dl, writer)


def judgements(cr) -> dict:
    p = Path(cr.dir) / JUDGE_FILE
    try:
        st = p.stat()
    except OSError:
        return {}
    key = ("judge", str(p), st.st_mtime_ns, st.st_size)
    with _lock:
        if _cache.get("judge_key") != key:
            try:
                _cache["judge"] = json.loads(p.read_text(encoding="utf-8")).get("pairs") or {}
            except ValueError:
                _cache["judge"] = {}
            _cache["judge_key"] = key
        return _cache["judge"]


def groups(cr, c, st: Store) -> dict:
    """find_groups, cached per corpus + judge file + merges store; attached notes are not grouped."""
    j = judgements(cr)
    key = (id(c), _cache.get("judge_key"), st.n_events)
    with _lock:
        if _cache.get("groups_key") != key:
            att = set(MG.attached(st.items))
            _cache["groups"] = MG.find_groups(c, j, MG.dismissed_pairs(st.items), exclude=att)
            _cache["groups_key"] = key
        return _cache["groups"]


def drive_status(cr, mid: str) -> dict | None:
    """gsync's record of the Google Docs side of a merge (comedy/kb/gsync/map.json `merges`), read-only."""
    p = Path(cr.gsync_dir) / "map.json"
    try:
        m = json.loads(p.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    d = (m.get("merges") or {}).get(mid)
    if not d:
        return None
    return {k: d.get(k) for k in ("state", "doc_id", "doc_url", "moved", "left", "skipped_legal", "archive_folder",
                                  "applied_at", "undone_at", "error")}


# ---------------------------------------------------------------------------------------------------------- merge
def _snapshot(v: K.View, jid: str) -> dict:
    """Everything of one joke a merge must keep, verbatim: its units (text, date, doc) and, when the text is his own
    (an idea he typed or edited), its summary. Catalog summaries are AI-written and never copied as his words."""
    n = v.nodes[jid]
    units = []
    for u, eid in v.joke_units.get(jid, []):
        if K.bucket(v.edges[eid].get("status")) == "rejected":
            continue
        un = v.nodes.get(u) or {}
        units.append({"id": u, "date": un.get("date"), "doc_id": un.get("doc_id"), "doc_title": un.get("doc_title"),
                      "kind": un.get("kind"), "text": un.get("text") or ""})
    units.sort(key=lambda x: (x["date"] or "", x["id"]))
    his_summary = bool(n.get("_by_tristan")) or any(
        d.get("target") == "node" and d.get("id") == jid and "summary" in (d.get("after") or {})
        for d in v.decisions)
    perfs = []
    for eid in v.adj.get(jid, []):
        e = v.edges[eid]
        if e.get("kind") == "performed_as" and K.bucket(e.get("status")) != "rejected":
            p = e["src"] if e["dst"] == jid else e["dst"]
            pn = v.nodes.get(p) or {}
            perfs.append({"id": p, "set": pn.get("set"), "edge": eid})
    return {"title": n.get("title"), "stage": n.get("stage"), "first_seen": n.get("first_seen"),
            "last_seen": n.get("last_seen"), "summary": n.get("summary") if his_summary else None,
            "summary_is_his": his_summary, "added_at": n.get("first_seen") if n.get("_by_tristan") else None,
            "units": units, "performances": sorted(perfs, key=lambda x: x["id"]), "from": n.get("from")}


def plan(v: K.View, main: str, others: list[str], mid: str) -> list[dict]:
    """The decision requests a merge writes, validated against the view (kb.py builds each line)."""
    main_units = {u for u, eid in v.joke_units.get(main, []) if K.bucket(v.edges[eid].get("status")) != "rejected"}
    reqs = []
    for o in others:
        why = f"{TAG} {mid}: '{v.nodes[o].get('title')}' merged into '{v.nodes[main].get('title')}'"
        for u, eid in v.joke_units.get(o, []):
            e = v.edges[eid]
            if K.bucket(e.get("status")) == "rejected" or u in main_units:
                continue
            end = "dst" if e.get("dst") == o else "src"
            reqs.append({"target": "edge", "id": eid, "action": "edit", "after": {end: main}, "reason": why})
        for eid in v.adj.get(o, []):
            e = v.edges[eid]
            if e.get("kind") != "performed_as" or K.bucket(e.get("status")) == "rejected":
                continue
            end = "dst" if e.get("dst") == o else "src"
            other_end = e["src"] if end == "dst" else e["dst"]
            if any(v.edges[x].get("kind") == "performed_as" and {v.edges[x]["src"], v.edges[x]["dst"]} == {other_end, main}
                   for x in v.adj.get(main, [])):
                continue  # already matched to the main: leave it, the hidden joke's copy drops out with it
            reqs.append({"target": "edge", "id": eid, "action": "edit", "after": {end: main}, "reason": why})
        reqs.append({"target": "node", "id": o, "action": "edit", "after": {"hidden": True}, "reason": why})
    return reqs


def do_merge(cr, kb, body: dict) -> dict:
    main, others = body.get("main"), body.get("others")
    if not isinstance(main, str) or not ID_RE.match(main):
        raise HttpError(400, "main must be a joke id")
    if not isinstance(others, list) or not others or len(others) > MAX_OTHERS or \
            not all(isinstance(x, str) and ID_RE.match(x) for x in others):
        raise HttpError(400, f"others must be a list of 1-{MAX_OTHERS} joke ids")
    others = list(dict.fromkeys(others))
    if main in others:
        raise HttpError(400, "the main joke can't also be one of the others")
    reason = (body.get("reason") or "").strip()[:500]
    with kb.lock:
        v = kb.view()
        for j in [main] + others:
            if v.ntype(j) != "joke":
                raise HttpError(404, f"unknown joke {j} (or it was already merged)")
        st = store(cr, kb)
        mid = "mg:" + sha8([main, sorted(others), now_iso(), st.n_events])
        snapshot = {j: _snapshot(v, j) for j in [main] + others}
        built = []
        for req in plan(v, main, others, mid):
            d = kb._build(v, req)
            d["reason"] = req["reason"] + (f" ({reason})" if reason else "")
            built.append(d)
        idx = []
        try:
            for d in built:
                idx += kb.append_lines([d])
        except Exception:
            for i in reversed(idx):  # a half-written merge never stays half-written
                try:
                    kb.undo(i)
                except K.DecisionError:
                    pass
            raise
        rec = st.append("merge", mid, {"main": main, "others": others, "indexes": idx, "snapshot": snapshot,
                                       "group": body.get("group"), "reasons": body.get("reasons") or [],
                                       "reason": reason}, by=OWNER_ID)
    return {"ok": True, "merge_id": mid, "merge": brief(rec), "indexes": idx,
            "drive": {"state": "queued", "note": "gsync on your PC writes the merged Google Doc and moves the old docs "
                                                 "into 'Merged (archive)' on its next pass"}}


def do_undo(cr, kb, body: dict) -> dict:
    mid = body.get("merge_id")
    if not isinstance(mid, str) or not ID_RE.match(mid):
        raise HttpError(400, "merge_id is required")
    with kb.lock:
        st = store(cr, kb)
        m = st.items.get(mid)
        if not m:
            raise HttpError(404, "no such merge")
        if m["kind"] == "attach":
            if m["status"] != "attached":
                raise HttpError(409, f"that note is already {m['status']}")
            rec = st.append("detach", mid, {}, by=OWNER_ID)
            return {"ok": True, "merge_id": mid, "merge": brief(rec), "indexes": []}
        if m["kind"] != "merge" or m["status"] != "merged":
            raise HttpError(409, f"that merge is {m.get('status')}")
        done = []
        for i in sorted(m.get("indexes") or [], reverse=True):
            try:
                done.append(kb.undo(i)["index"])
            except K.DecisionError as e:
                if e.status not in (409,):  # already undone by hand: fine, anything else is not
                    raise
        rec = st.append("undo", mid, {"indexes": done}, by=OWNER_ID)
    return {"ok": True, "merge_id": mid, "merge": brief(rec), "indexes": done,
            "drive": {"state": "undo_queued", "note": "gsync moves the old docs back and releases the merged doc on its "
                                                      "next pass"}}


def do_dismiss(cr, kb, body: dict) -> dict:
    js = body.get("jokes")
    if not isinstance(js, list) or len(js) < 2 or len(js) > MAX_OTHERS + 1 or \
            not all(isinstance(x, str) and ID_RE.match(x) for x in js):
        raise HttpError(400, "jokes must be a list of 2 or more joke ids")
    js = sorted(set(js))
    did = "dm:" + sha8(js)
    with kb.lock:
        st = store(cr, kb)
        if did in st.items:
            return {"ok": True, "id": did, "already": True}
        st.append("dismiss", did, {"jokes": js, "reason": (body.get("reason") or "").strip()[:500]}, by=OWNER_ID)
    return {"ok": True, "id": did}


def brief(m: dict | None) -> dict | None:
    if not m:
        return None
    return {k: v for k, v in m.items() if k != "snapshot"}


# ---------------------------------------------------------------------------------------------------------- attach
_emb = {"e": None, "error": None}


def embed(text: str):
    """bge-small vector for a new jot (db/embedder.py, local node). None when the embedder can't run: then only an
    unmistakable word match can attach."""
    if _emb["error"]:
        return None
    try:
        if _emb["e"] is None:
            sys.path.insert(0, str(DESK / "db"))
            from embedder import Embedder
            _emb["e"] = Embedder(timeout=30)
        return _emb["e"].embed([text])[0]
    except Exception as e:  # noqa: BLE001
        _emb["error"] = f"{type(e).__name__}: {e}"
        return None


def after_idea(cr, kb, jid: str, text: str, source: str = "app") -> dict | None:
    """Called by POST /api/ideas after the idea's line is written. Never raises: a failure leaves a new idea."""
    if cr is None:
        return None
    try:
        hit = cr._corpus.get(kb.backend)  # the corpus as it was a moment ago is fine: the new idea has no units yet
        c = hit[1] if hit else cr.corpus(kb)
        r = MG.match_text(c, text, embed(text), exclude={jid})
        if r["decision"] != "attach":
            return {"decision": r["decision"], "reasons": r["reasons"]}
        st = store(cr, kb)
        aid = "at:" + sha8([jid, r["joke_id"]])
        rec = st.append("attach", aid, {"joke": jid, "into": r["joke_id"], "text": text, "date": now_iso()[:10],
                                        "score": r.get("score"), "reasons": r["reasons"], "source": source},
                        by="ai:attach")
        return {"decision": "attach", "id": aid, "into": r["joke_id"], "title": r["title"], "reasons": r["reasons"],
                "undo": {"POST": "/api/merge/undo", "merge_id": aid}, "record": brief(rec)}
    except Exception as e:  # noqa: BLE001  the idea is saved either way
        return {"decision": "new", "error": f"{type(e).__name__}: {e}"}


def desk_filter(cr, kb, items: list[dict], include_hidden: bool) -> list[dict]:
    """/api/jokes: an attached note shows under its joke, not as a separate idea (unless include_hidden)."""
    if cr is None or include_hidden:
        return items
    try:
        att = MG.attached(store(cr, kb).items)
    except Exception:  # noqa: BLE001
        return items
    return [x for x in items if x.get("id") not in att] if att else items


# ---------------------------------------------------------------------------------------------------------- routes
def handle(cr, h, method: str, path: str, q: dict, body_fn) -> bool:
    if not path.startswith("/api/merge"):
        return False
    kb = h.kb
    arg = lambda k: (q.get(k) or [None])[0]  # noqa: E731
    try:
        if method == "GET" and path == "/api/merge/candidates":
            c, st = cr.corpus(kb), store(cr, kb)
            jid = arg("joke")
            if jid:
                if not ID_RE.match(jid):
                    raise HttpError(400, "bad joke id")
                out = MG.candidates_for(c, jid, judgements(cr), MG.dismissed_pairs(st.items),
                                        groups=groups(cr, c, st)["groups"])
                out["attached_to"] = MG.attached(st.items).get(jid)
                out["history"] = MG.history(st.items, jid)
                return _send(h, 200, out)
            g = groups(cr, c, st)
            return _send(h, 200, {**g, "generated_at": now_iso()})
        if method == "GET" and path == "/api/merge/history":
            jid = arg("joke")
            if not jid or not ID_RE.match(jid):
                raise HttpError(400, "joke is required")
            st = store(cr, kb)
            out = MG.history(st.items, jid)
            for m in out["merges"]:
                m["drive"] = drive_status(cr, m["id"])
            return _send(h, 200, out)
        if method == "GET":
            m = re.match(r"^/api/merge/((?:mg|at|dm):[A-Za-z0-9]+)$", path)
            if m:
                rec = store(cr, kb).items.get(m.group(1))
                if not rec:
                    raise HttpError(404, "no such merge")
                return _send(h, 200, {**rec, "drive": drive_status(cr, rec["id"])})
        if method == "POST" and path == "/api/merge":
            return _send(h, 200, do_merge(cr, kb, body_fn()))
        if method == "POST" and path == "/api/merge/undo":
            return _send(h, 200, do_undo(cr, kb, body_fn()))
        if method == "POST" and path == "/api/merge/dismiss":
            return _send(h, 200, do_dismiss(cr, kb, body_fn()))
        raise HttpError(404, "not found")
    except CR.HttpError as e:
        h.err(e.status, str(e))
    return True


def _send(h, status, obj) -> bool:
    h.send_json(obj, status)
    return True
