"""His voice profile as data: one document in the database, the single source of truth for every writing prompt.

    GET  /api/profile/voice                 {kind: "voice", text, updated_at, by, chars, previous: {text, updated_at, by} | null}
    PUT  /api/profile/voice {text}          save it (by him, from Settings > Voice profile). Legal-checked like every
                                            ingest (gsync/rules.py content_is_legal): a hit is 422 and nothing is kept.
    GET  /api/worker/profile/voice          the worker's copy, fetched per job (worker token only): {kind, text, updated_at, by}
    PUT  /api/worker/profile/voice {text, actor}   the one-off loader (tools/profile/put_voice.py) with the worker's
                                            credentials; `actor` must be import:<what> (it uses import:profile); the
                                            same legal check, the same 422.

Store `profile` (creative/store.py: comedy/kb/creative/profile.jsonl + snapshot; on SurrealDB also creative_event
rows, reconciled like every creative store). Two ops on the document `profile:voice`:
    put {kind, text}            his text (Settings, or the loader). The document keeps its previous version
                                (`previous`), so a save is undone by saving the previous text again (the app's Undo).
                                A put never touches `learned`.
    learn {kept[], dropped[]}   the learned section (creative/learned.py): appended by the API itself when he keeps
                                one of the app's drafts (learn_kept: POST /api/ideas with `draft: true` or a riff
                                draft's exact text, or the Brainstorm decision with a punch-up draft's text) and
                                when he dismisses one (POST /api/profile/voice/learn {dropped: [text | {text, bit}]},
                                app token only, learn_dropped: the same legal rule, 422). Never by a PUT. Capped and
                                merged by creative.learned.merge; never touches `text` or `previous`.
Both GETs return `learned` ({kept, dropped, updated_at}: the lines, with the bit id and the date); the worker's GET
also returns `laugh_lines` (computed from the corpus each time: creative.learned.laugh_lines, [] without a laugh map).
The text is never a file in the repo or in an image, and never logged: the worker (creative/voice.py) logs its length,
a short sha and `updated_at`. Provenance: `by` is the owner id for his own saves, `import:profile` for the loader.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DESK = HERE.parent
if str(DESK) not in sys.path:
    sys.path.insert(0, str(DESK))
if str(DESK / "tools" / "explorer") not in sys.path:
    sys.path.insert(0, str(DESK / "tools" / "explorer"))  # kb.py (DecisionError), as server.py loads it

import creative_api as CR  # noqa: E402
import kb as K  # noqa: E402
from creative import learned as L  # noqa: E402
from creative.data import by_kind  # noqa: E402
from creative.store import Store  # noqa: E402

STORE = "profile"
VOICE_ID = "profile:voice"
MAX_CHARS = 60_000
ROUTE = re.compile(r"^/api/(worker/)?profile/voice$")
LEARN_ROUTE = re.compile(r"^/api/profile/voice/learn$")   # POST {dropped: [...]}: he dismissed a draft (app token only)
LEGAL_MSG = "That reads like court material; Punchable won't keep it as your voice profile. Nothing was saved."


def profile_apply(items: dict, ev: dict) -> None:
    """Ops: put {kind, text} (the previous version rides along for Undo; `learned` is kept as it was) and
    learn {kept[], dropped[]} (merged into `learned` by creative.learned.merge; `text` and `previous` as they were)."""
    op, i, d = ev["op"], ev["id"], ev.get("data") or {}
    if i != VOICE_ID:
        raise ValueError(f"profiles live on {VOICE_ID}")
    cur = items.get(i)
    if op == "put":
        if not isinstance(d.get("text"), str):
            raise ValueError("text must be a string")
        items[i] = {"id": i, "kind": str(d.get("kind") or "voice"), "text": d["text"], "updated_at": ev["ts"],
                    "by": ev.get("by"),
                    "previous": ({"text": cur["text"], "updated_at": cur["updated_at"], "by": cur.get("by")}
                                 if cur and cur.get("text") is not None else None),
                    "learned": (cur or {}).get("learned") or L.empty()}
    elif op == "learn":
        if not isinstance(d.get("kept", []), list) or not isinstance(d.get("dropped", []), list):
            raise ValueError("kept and dropped must be lists")
        if not (d.get("kept") or d.get("dropped")):
            raise ValueError("a learn event names at least one line")
        base = cur or {"id": i, "kind": "voice", "text": None, "updated_at": None, "by": None, "previous": None}
        items[i] = {**base, "learned": L.merge(base.get("learned"), d.get("kept"), d.get("dropped"), ts=ev["ts"])}
    else:
        raise ValueError(f"unknown op {op}")


CR.APPLY.setdefault(STORE, profile_apply)   # LineStore (the SurrealDB mirror) looks the apply function up by name


def store(cr, kb) -> Store:
    """The profile store for this backend (the file, or the file + creative_event rows on SurrealDB)."""
    dl = cr.reconcile(kb, STORE)
    if dl is None:
        return Store(cr.dir, STORE, profile_apply)
    m = cr._mirror(kb)

    def writer(name, seq, line, ev, m=m):
        if name in cr.conflicts:
            raise CR.HttpError(409, cr.conflicts[name] + "; writes are refused until they agree")
        m.create(name, seq, line, ev)
        CR._append_file(cr.dir / f"{name}.jsonl", line)
    return CR.LineStore(cr.dir, STORE, dl, writer)


def view(doc: dict | None, worker: bool = False, laughs: list | None = None) -> dict:
    if not doc or doc.get("text") is None:
        out = {"kind": "voice", "text": None, "updated_at": None, "by": None, "chars": 0}
    else:
        out = {"kind": doc["kind"], "text": doc["text"], "updated_at": doc["updated_at"], "by": doc.get("by"),
               "chars": len(doc["text"])}
    out["by_kind"] = by_kind(out["by"])  # T16: owner | import | ai | null, so the app stops testing the actor prefix
    out["learned"] = (doc or {}).get("learned") or L.empty()
    if worker:
        out["laugh_lines"] = laughs or []
    else:
        prev = (doc or {}).get("previous")
        out["previous"] = {**prev, "by_kind": by_kind(prev.get("by"))} if isinstance(prev, dict) else prev
    return out


def laugh_lines(cr, kb) -> list[dict]:
    """creative.learned.laugh_lines over this backend's corpus; [] when the corpus cannot be built here (a test
    handler without a knowledge base) or has no laugh map. Never fails a GET."""
    try:
        return L.laugh_lines(cr.corpus(kb))
    except Exception:  # noqa: BLE001  no corpus on this handler: the worker drafts without laugh lines
        return []


def learn_kept(cr, kb, text: str, bit: str | None, by: str, draft: bool = False) -> dict | None:
    """He kept one of the app's drafts: `text` is a line he just saved as his own (POST /api/ideas, or the Brainstorm
    unit with source "draft"). Kept when EITHER the app flagged it (`draft` = `draft: true` on POST /api/ideas) OR it
    is the exact text of a finished riff or punch-up job's draft (creative.learned.drafted over the job store). One
    `learn` event: that line kept (marked `edited` when the flag is set but no draft matches: his edit of a draft),
    and, when a job matched, its other drafts passed over (minus any he kept or dismissed before). His own typed
    line, unflagged: nothing is written, None. Never raises: the caller's write is already done."""
    try:
        with cr.lock:
            jobs = cr.stores(kb)["jobs"].items
            kind, others = L.drafted(jobs, text)
            if kind is None and not draft:
                return None
            st = store(cr, kb)
            learned = (st.get(VOICE_ID) or {}).get("learned") or {}
            # a sibling he kept or dismissed earlier is already a signal of its own: only lines he never acted on
            seen = {L._norm(e["text"]) for k in ("kept", "dropped") for e in learned.get(k) or []}
            others = [t for t in others if L._norm(t) not in seen]
            kept = {"text": text, "bit": bit, **({"edited": True} if kind is None else {})}
            doc = st.append("learn", VOICE_ID, {"kept": [kept], "dropped": [{"text": t, "bit": bit} for t in others],
                                                "from": kind or "draft_flag"}, by=by)
        return L.counts(doc.get("learned"))
    except Exception:  # noqa: BLE001  the learned section is a nicety: his save stands either way
        return None


MAX_DISMISS = 10
LINE_MAX = 2000   # a riff line on the way in (creative_api.riff) is at most this long


def clean_dropped(body: dict) -> list[dict]:
    """POST /api/profile/voice/learn {dropped: [text | {text, bit?}], bit?}: 1-10 lines, each up to 2000 characters."""
    if not isinstance(body, dict):
        raise CR.HttpError(400, "body must be an object")
    xs = body.get("dropped")
    if not isinstance(xs, list) or not xs or len(xs) > MAX_DISMISS:
        raise CR.HttpError(400, f"dropped must be a list of 1-{MAX_DISMISS} lines")
    bit0 = body.get("bit") if isinstance(body.get("bit"), str) and CR.ID_RE.match(body["bit"]) else None
    out = []
    for x in xs:
        if isinstance(x, str):
            x = {"text": x}
        if not isinstance(x, dict) or not isinstance(x.get("text"), str) or not x["text"].strip():
            raise CR.HttpError(400, "each dropped line needs its text")
        if len(x["text"]) > LINE_MAX:
            raise CR.HttpError(413, f"a line is at most {LINE_MAX:,} characters")
        bit = x.get("bit") if isinstance(x.get("bit"), str) and CR.ID_RE.match(x["bit"]) else bit0
        out.append({"text": x["text"].strip(), "bit": bit})
    return out


def learn_dropped(cr, kb, body: dict, by: str) -> dict:
    """He dismissed a draft (Write's or Brainstorm's "Dismiss"): one `learn {dropped}` event, by him. The same legal
    rule as every stored text (422, nothing kept). Returns the counts; the lines themselves are in GET's `learned`."""
    lines = clean_dropped(body)
    CR.refuse_legal(*[x["text"] for x in lines])
    with cr.lock:
        doc = store(cr, kb).append("learn", VOICE_ID, {"dropped": lines, "from": "dismiss"}, by=by)
    return {"ok": True, "learned": L.counts(doc.get("learned"))}


def clean_text(body: dict) -> str:
    t = body.get("text") if isinstance(body, dict) else None
    if not isinstance(t, str) or not t.strip():
        raise CR.HttpError(400, "text is required")
    t = t.replace("\r\n", "\n").strip("\n") + "\n"
    if len(t) > MAX_CHARS:
        raise CR.HttpError(413, f"the profile is at most {MAX_CHARS:,} characters")
    return t


def put(cr, kb, body: dict, by: str) -> dict:
    """Validate (shape, size, legal), then one `put` event. The legal check runs BEFORE anything is written."""
    import ingest_api as IG
    text = clean_text(body)
    if IG.is_legal(text):
        raise CR.HttpError(422, LEGAL_MSG)
    st = store(cr, kb)
    return view(st.append("put", VOICE_ID, {"kind": "voice", "text": text}, by=by))


def load_into_voice(cr, kb) -> bool:
    """The API's own copy into creative.voice (local dev, PK_JOBS=inproc: the executor runs in this process)."""
    from creative import voice
    doc = store(cr, kb).get(VOICE_ID)
    voice.set_learned((doc or {}).get("learned"), laugh_lines(cr, kb))
    if doc and doc.get("text"):
        voice.set_profile(doc["text"], doc["updated_at"], doc.get("by"), source="store")
        return True
    voice.set_profile(None)
    return False


def handle(cr, h, method: str, path: str, q: dict, body_fn) -> bool:
    if LEARN_ROUTE.match(path):
        # the app's token only: server.Handler keeps a worker token on /api/worker/*, so this is always him
        try:
            if method != "POST":
                raise CR.HttpError(405, "POST")
            h.send_json(learn_dropped(cr, h.kb, body_fn(), h.actor({})))
        except (CR.HttpError, K.DecisionError) as e:
            h.err(e.status, str(e))
        return True
    m = ROUTE.match(path)
    if not m:
        return False
    worker = bool(m.group(1))
    kb = h.kb
    try:
        if method == "GET":
            h.send_json(view(store(cr, kb).get(VOICE_ID), worker=worker, laughs=laugh_lines(cr, kb) if worker else None))
            return True
        if method != "PUT":
            raise CR.HttpError(405, "GET or PUT")
        body = body_fn()
        if not isinstance(body, dict):
            raise CR.HttpError(400, "body must be an object")
        if worker:
            import ingest_api as IG
            by = IG.import_actor(h, body)   # import:<what> or ai:<what>, never the owner's id
        else:
            by = h.actor(body)              # the app's token: the owner; a worker token never reaches this path
        with cr.lock:
            out = put(cr, kb, body, by)
        h.send_json(out)
        return True
    except (CR.HttpError, K.DecisionError) as e:
        h.err(e.status, str(e))
        return True
