"""Your sets (docs/API_CONTRACT.md "Your sets"): many sets at once, each opened and worked on. Engine: creative/mysets.py.

A set is a working set: the same store (`workingsets`, creative_api's stores on both backends), so every working set he
already has is listed here, and every write is one of his actions in that append-only log.

    GET   /api/mysets                         his sets, newest first: name, length, run time, one-word health
    POST  /api/mysets                         new set: {title, target_s, joke_ids?} | {board_id, title?, target_s?}
                                              | {from_set, target_s, title?, joke_ids?}  (joke_ids from the trim)
    GET   /api/mysets/match?set=<recording>   the set a recording lines up with
    GET   /api/mysets/{id}                    the set screen (creative/mysets.view)
    GET   /api/mysets/{id}/trim?target_s=300  "make a 5 from my 10": a suggestion, nothing is written
    PATCH /api/mysets/{id}                    {joke_ids?, target_s?, note?} = a new version (the note says what changed,
                                              written for him when he leaves it out); {title} = rename; {delete: true}
    POST  /api/mysets/{id}/lock               {joke_id, unlock?}
    POST  /api/mysets/{id}/bridge             {from, to}: the bridge draft (cached, or 202 + a job for the home worker)
"""
from __future__ import annotations

import re
from urllib.parse import unquote

import creative_api as CR
from creative import mysets as M
from creative import workingset as W

HttpError = CR.HttpError

GET_ROUTES = [
    (re.compile(r"^/api/mysets$"), "list"),
    (re.compile(r"^/api/mysets/match$"), "match"),
    (re.compile(r"^/api/mysets/([^/]+)/trim$"), "trim"),
    (re.compile(r"^/api/mysets/([^/]+)$"), "one"),
]
POST_ROUTES = [
    (re.compile(r"^/api/mysets$"), "create"),
    (re.compile(r"^/api/mysets/([^/]+)/lock$"), "lock"),
    (re.compile(r"^/api/mysets/([^/]+)/bridge$"), "bridge"),
]
PATCH_ROUTES = [(re.compile(r"^/api/mysets/([^/]+)$"), "patch")]
TABLES = {"GET": GET_ROUTES, "POST": POST_ROUTES, "PATCH": PATCH_ROUTES}


def match(method: str, path: str):
    for rx, name in TABLES.get(method, []):
        m = rx.match(path)
        if m:
            return name, [unquote(x) for x in m.groups()]
    return None


def _title(v, required=True):
    if v is None and not required:
        return None
    if not isinstance(v, str) or not v.strip() or len(v) > 200:
        raise HttpError(400, "title is required (1-200 characters)")
    return v.strip()


class Routes(CR.Routes):
    def _live(self, st) -> list[dict]:
        return [w for w in st["workingsets"].items.values() if not w.get("deleted")]

    def _target(self, v, required=True):
        t = CR._int(v, "target_s", 60, 4 * 3600)
        if t is None and required:
            raise HttpError(400, "target_s is required (seconds, 60 to 14400)")
        return t

    # ---------------------------------------------------------------- reads
    def list(self):
        c, st = self.c, self.st()
        ws = sorted(self._live(st), key=lambda w: (w["updated_at"], w["id"]), reverse=True)
        return 200, {"sets": [M.summary(c, w) for w in ws], "lengths": M.LENGTHS}

    def one(self, sid):
        c, st = self.c, self.st()
        return 200, M.view(c, CR._ws(st, sid), self._live(st))

    def trim(self, sid):
        c, st = self.c, self.st()
        t = self.arg("target_s")
        if not str(t or "").isdigit():
            raise HttpError(400, "target_s is required (seconds)")
        return 200, M.trim(c, CR._ws(st, sid), self._target(int(t)))

    def match(self):
        key = self.arg("set") or ""
        c, st = self.c, self.st()
        if key.startswith("set:"):
            key = key[4:]
        if key not in c.sets:
            raise HttpError(404, "no such recording")
        return 200, M.match_recording(c, self._live(st), key)

    # ---------------------------------------------------------------- writes (his actions)
    def create(self):
        c, st = self.c, self.st()
        store = st["workingsets"]
        b = self.b
        if b.get("board_id") is not None:
            board = CR._board(st, b["board_id"] if isinstance(b["board_id"], str) else "")
            ids = [j for j in board["joke_ids"] if j in c.jokes]
            title = _title(b.get("title"), False) or board["title"]
            target = self._target(b.get("target_s"), False) or board.get("target_s") or 600
            ws = M.create(c, store, title, target, ids, note=f"v1: made from the board '{board['title']}'",
                          storyboard_id=board["id"])
        elif b.get("from_set") is not None:
            src = CR._ws(st, b["from_set"] if isinstance(b["from_set"], str) else "")
            target = self._target(b.get("target_s"))
            sug = M.trim(c, src, target)
            ids = self._jokes_list(c, b["joke_ids"]) if "joke_ids" in b else [x["joke_id"] for x in sug["keep"]]
            ws = M.create(c, store, _title(b.get("title"), False) or sug["title"], target, ids,
                          note=f"v1: a {M.length_words(target)} from '{src['title']}'")
        else:
            target = self._target(b.get("target_s"))
            ids = self._jokes_list(c, b.get("joke_ids") or [])
            ws = M.create(c, store, _title(b.get("title")), target, ids, note="v1: a new set")
        return 201, M.view(c, ws, self._live(st))

    def patch(self, sid):
        c, st = self.c, self.st()
        store = st["workingsets"]
        ws = CR._ws(st, sid)
        b = self.b
        if b.get("delete") is True:
            store.append("delete", ws["id"], {})
            return 200, {"deleted": ws["id"]}
        changed = False
        if "title" in b:
            t = _title(b["title"])
            if t != ws["title"]:
                ws = store.append("update", ws["id"], {"title": t})
                changed = True
        if "joke_ids" in b or "target_s" in b:
            cur = W.current(ws)
            old_ids = [x["joke_id"] for x in cur["bits"]]
            ids = self._jokes_list(c, b["joke_ids"]) if "joke_ids" in b else old_ids
            target = self._target(b["target_s"]) if "target_s" in b else cur["target_s"]
            if ids != old_ids or target != cur["target_s"]:
                note = b.get("note")
                if note is not None and (not isinstance(note, str) or not note.strip() or len(note) > 2000):
                    raise HttpError(400, "note must be text")
                note = (note or "").strip() or M.change_note(c, old_ids, ids, cur["target_s"], target)
                old = {x["joke_id"]: x for x in cur["bits"]}
                bits = []
                for j in ids:
                    if j in old:
                        bits.append(old[j])
                    else:
                        d, basis = c.duration(j)
                        bits.append({"joke_id": j, "title": c.title(j), "target_s": int(round(d / 5.0) * 5), "target_basis": basis})
                ws = store.append("new_version", ws["id"], {"note": note, "bits": bits, "target_s": target})
                changed = True
        if not changed and not any(k in b for k in ("title", "joke_ids", "target_s")):
            raise HttpError(400, "nothing to change (title, joke_ids, target_s, delete)")
        return 200, M.view(c, st["workingsets"].get(ws["id"]), self._live(st))

    def lock(self, sid):
        c, st = self.c, self.st()
        ws = CR._ws(st, sid)
        jid = CR._joke(c, self.b.get("joke_id"))
        if jid not in M.ids_of(ws):
            raise HttpError(400, f"{jid} is not in this set")
        unlock = bool(self.b.get("unlock"))
        if W.bit_locked(ws, jid) != (not unlock):
            ws = (W.unlock_bit if unlock else W.lock_bit)(st["workingsets"], ws["id"], jid)
        return 200, M.view(c, ws, self._live(st))

    def bridge(self, sid):
        c, st = self.c, self.st()
        ws = CR._ws(st, sid)
        a, b = CR._joke(c, self.b.get("from")), CR._joke(c, self.b.get("to"))
        ids = M.ids_of(ws)
        if a not in ids or b not in ids:
            raise HttpError(400, "both bits must be in this set")
        # the same key as a board's bridge: one draft per pair, wherever he asked for it
        return self.cr.enqueue(st, "bridge", f"bridge|{a}|{b}", {"from": a, "to": b, "set": ws["id"]},
                               fresh=bool(self.b.get("fresh")), kb=self.kb)


def handle(cr, h, method: str, path: str, q: dict, body_fn) -> bool:
    """Serve a /api/mysets route. Returns False when the path is not one of ours."""
    m = match(method, path)
    if not m:
        return False
    name, args = m
    try:
        body = body_fn() if method in ("POST", "PATCH") else None
        with cr.lock if method != "GET" else CR._nolock():
            status, out = getattr(Routes(cr, h, h.kb, q, body), name)(*args)
        h.send_json(out, status)
    except HttpError as e:
        h.err(e.status, str(e))
    return True
