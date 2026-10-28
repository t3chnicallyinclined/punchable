"""His voice profile as data: one document in the database, the single source of truth for every writing prompt.

    GET  /api/profile/voice                 {kind: "voice", text, updated_at, by, chars, previous: {text, updated_at, by} | null}
    PUT  /api/profile/voice {text}          save it (by him, from Settings > Voice profile). Legal-checked like every
                                            ingest (gsync/rules.py content_is_legal): a hit is 422 and nothing is kept.
    GET  /api/worker/profile/voice          the worker's copy, fetched per job (worker token only): {kind, text, updated_at, by}
    PUT  /api/worker/profile/voice {text, actor}   the one-off loader (tools/profile/put_voice.py) with the worker's
                                            credentials; `actor` must be import:<what> (it uses import:profile); the
                                            same legal check, the same 422.

Store `profile` (creative/store.py: comedy/kb/creative/profile.jsonl + snapshot; on SurrealDB also creative_event
rows, reconciled like every creative store). One op, `put {kind, text}`, on the document `profile:voice`. The document
keeps its previous version (`previous`), so a save is undone by saving the previous text again (the app's Undo).
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
from creative.store import Store  # noqa: E402

STORE = "profile"
VOICE_ID = "profile:voice"
MAX_CHARS = 60_000
ROUTE = re.compile(r"^/api/(worker/)?profile/voice$")
LEGAL_MSG = "That reads like court material; Punchable won't keep it as your voice profile. Nothing was saved."


def profile_apply(items: dict, ev: dict) -> None:
    """Ops: put {kind, text}. The previous version rides along for Undo."""
    op, i, d = ev["op"], ev["id"], ev.get("data") or {}
    if op != "put":
        raise ValueError(f"unknown op {op}")
    if i != VOICE_ID:
        raise ValueError(f"profiles live on {VOICE_ID}")
    if not isinstance(d.get("text"), str):
        raise ValueError("text must be a string")
    cur = items.get(i)
    items[i] = {"id": i, "kind": str(d.get("kind") or "voice"), "text": d["text"], "updated_at": ev["ts"],
                "by": ev.get("by"),
                "previous": ({"text": cur["text"], "updated_at": cur["updated_at"], "by": cur.get("by")} if cur else None)}


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


def view(doc: dict | None, worker: bool = False) -> dict:
    if not doc:
        out = {"kind": "voice", "text": None, "updated_at": None, "by": None, "chars": 0}
    else:
        out = {"kind": doc["kind"], "text": doc["text"], "updated_at": doc["updated_at"], "by": doc.get("by"),
               "chars": len(doc["text"])}
    if not worker:
        out["previous"] = (doc or {}).get("previous")
    return out


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
    if doc and doc.get("text"):
        voice.set_profile(doc["text"], doc["updated_at"], doc.get("by"), source="store")
        return True
    voice.clear()
    return False


def handle(cr, h, method: str, path: str, q: dict, body_fn) -> bool:
    m = ROUTE.match(path)
    if not m:
        return False
    worker = bool(m.group(1))
    kb = h.kb
    try:
        if method == "GET":
            h.send_json(view(store(cr, kb).get(VOICE_ID), worker=worker))
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
