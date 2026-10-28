"""His voice profile: the one stable block every writing prompt starts with (riff, punch-up, bridge and Ask).

Where it lives: the hosted database, as data (api-dev/profile_api.py: store `profile`, document `profile:voice`:
markdown, `updated_at`, `by`). He writes it in Settings > Voice profile (PUT /api/profile/voice) or the orchestrator
loads it once (tools/profile/put_voice.py). It is never a file in this repo or in an image. The worker fetches it per
job (GET /api/worker/profile/voice) and this module caches it by `updated_at`; every prompt then starts with it as the
FIRST system block, with the craft rules in the same block, so engine/providers.py's cache breakpoint on that block
is a prefix long enough to be served from the prompt cache (512 tokens on Opus 5.5 / Sonnet 5.5) on every call after
the first. Nothing here logs the text: only its length, a short sha and its `updated_at`.

    set_profile(text, updated_at, by)   the worker (voice.fetch) or a test puts the profile here
    set_learned(learned, laughs)        the learned section (creative/learned.py) and the laugh lines, from the same GET
    system_block(task)                  the first system block for a task, or None when no profile is loaded
    learned_block(task)                 the SECOND system block (writing tasks only): lines he kept / passed over / laughs
    messages(task, task_system, user)   [profile block] + [learned block] + the task's own system line + the user turn
    fetch(cl, log)                      GET /api/worker/profile/voice, cached by updated_at; never fails a job
    comic() / first() / named()         how the prompts name him (PK_COMIC_NAME / PK_COMIC_FIRST; "the comic" when unset)

The learned block is the app's own evidence (the lines he kept and passed over, the lines that got laughs), kept in
the same document by the API (profile_api.learn_kept) and never written by him; it is a separate block so his profile
block stays byte-stable (its cache entry survives every keep) and the learned block, which changes when he keeps a
line, is cached behind it. On its own the learned block is far under the 512-token minimum: it caches only as the
prefix [profile + learned], never alone (cacheable_learned() says which it is; the worker log says so once).
"""
from __future__ import annotations

import hashlib
import os

from . import learned as L

MIN_CACHE_TOKENS = 512          # the minimum cacheable prefix on Claude Opus 5.5 / Sonnet 5.5 (prompt caching docs)
MAX_CHARS = 60_000              # the longest profile the API accepts (api-dev/profile_api.py)
UNNAMED = "the comic"
WRITING_TASKS = ("creative_riff", "creative_punchup", "creative_bridge")
READING_TASKS = ("ask_chat",)

# The craft rules (public, generic). His named moves live in the profile itself, never here: the rules tell the
# model to use the profile's names for them.
CRAFT = (
    "How to draft for him. First person, spoken, short: a line he could say out loud tonight, not prose. The punch "
    "word lands last; nothing trails after it. Keep his premise and his angle: sharpen the line, never swap the joke "
    "for another one about the same subject. Prefer his own moves, the ones the profile above names, and name the one "
    "you used in `move`, with the profile's name for it; if the profile names none that fits, say what the line does "
    "in two or three plain words. No explaining the joke, no setup that announces a joke is coming (\"here's a joke "
    "about\"), no stage directions, no emoji, no hashtags. A line that needs a fact about his life he never said is "
    "out: build only on what his material gives you. One idea per line; short beats clever. He decides; you suggest."
)
READING = (
    "This profile is background for reading his notebook: it tells you who the people and the recurring premises in "
    "his material are, and how he talks. It is not a source. Never cite it, never quote it, never answer from it; "
    "answer only from the numbered SOURCES and the rules below."
)
HEAD = ("The comic you are working for, in his own words and the words of people who have studied his act. Read it "
        "before anything else.")

_STATE: dict = {"text": None, "updated_at": None, "by": None, "source": None}
_LEARNED: dict = {"learned": None, "laughs": [], "updated_at": None}


# ---------------------------------------------------------------- his name in a prompt
def comic() -> str:
    return (os.environ.get("PK_COMIC_NAME") or "").strip() or UNNAMED


def first() -> str:
    return (os.environ.get("PK_COMIC_FIRST") or "").strip() or comic()


def named() -> bool:
    return comic() != UNNAMED


def whose_idea() -> str:
    """"Pat's idea" when he is named, else "His idea"."""
    return f"{first()}'s idea" if named() else "His idea"


def he_could_say() -> str:
    return f"{first()} could say" if named() else "he could say"


def material_owner() -> str:
    """"stand-up comic Pat Example's" when named, else "a stand-up comic's" (never "stand-up comic the comic's")."""
    return f"stand-up comic {comic()}'s" if named() else "a stand-up comic's"


# ---------------------------------------------------------------- the profile
def set_profile(text: str | None, updated_at: str | None = None, by: str | None = None, source: str | None = None):
    text = (text or "").strip()
    if not text:
        clear()
        return
    _STATE.update(text=text[:MAX_CHARS], updated_at=updated_at, by=by, source=source)


def clear():
    _STATE.update(text=None, updated_at=None, by=None, source=None)
    _LEARNED.update(learned=None, laughs=[], updated_at=None)


def loaded() -> bool:
    return bool(_STATE["text"])


def profile() -> dict | None:
    return dict(_STATE) if loaded() else None


def set_learned(learned: dict | None, laughs: list | None = None):
    """The learned section as the API serves it ({kept, dropped, updated_at}) and the laugh lines it computed."""
    l = learned if isinstance(learned, dict) else None
    _LEARNED.update(learned=l, laughs=[x for x in (laughs or []) if isinstance(x, dict) and x.get("text")],
                    updated_at=(l or {}).get("updated_at"))


def learned() -> dict | None:
    return dict(_LEARNED) if (_LEARNED["learned"] or _LEARNED["laughs"]) else None


def sha(text: str | None) -> str:
    return hashlib.sha256((text or "").encode("utf-8")).hexdigest()[:12]


def status() -> dict:
    """What may be logged or reported: never the text, never a line; the learned section as counts."""
    t = _STATE["text"]
    return {"loaded": bool(t), "chars": len(t or ""), "sha": sha(t) if t else None, "updated_at": _STATE["updated_at"],
            "by": _STATE["by"], "source": _STATE["source"],
            "learned": {**L.counts(_LEARNED["learned"]), "laughs": len(_LEARNED["laughs"])}}


def est_tokens(text: str) -> int:
    """A rough, slightly pessimistic token count (about 4 characters per token for English prose)."""
    return len(text or "") // 4


# ---------------------------------------------------------------- the block and the messages
def system_block(task: str) -> str | None:
    """The first system block for `task`: the profile, then the craft rules (writing tasks) or the reading note (Ask).
    None when no profile is loaded, so a prompt without one is exactly what it was before (and the worker's log says
    so, once, in voice.fetch)."""
    t = _STATE["text"]
    if not t:
        return None
    tail = READING if task in READING_TASKS else CRAFT
    return HEAD + "\n\n" + t + "\n\n" + tail


def cacheable(task: str) -> bool:
    b = system_block(task)
    return bool(b) and est_tokens(b) >= MIN_CACHE_TOKENS


def learned_block(task: str) -> str | None:
    """The second system block: the lines he kept, passed over and the laugh lines (creative/learned.py), for the
    writing tasks only (Ask reads his notebook, it does not draft). None when nothing is learned yet."""
    if task in READING_TASKS:
        return None
    return L.block(_LEARNED["learned"], _LEARNED["laughs"])


def cacheable_learned(task: str) -> str:
    """How the learned block is served from the cache: "prefix" (behind a profile block that reaches the minimum: the
    cached prefix is [profile + learned]), "alone_too_short" (no profile, or one too short: the block is sent but
    never cached; nothing is forced) or "none" (no learned block)."""
    b = learned_block(task)
    if not b:
        return "none"
    p = system_block(task) or ""
    return "prefix" if est_tokens(p) + est_tokens(b) >= MIN_CACHE_TOKENS and cacheable(task) else "alone_too_short"


def messages(task: str, task_system: str, *rest: dict) -> list[dict]:
    """The messages for one call: the profile block first (stable, cached), the learned block second (changes when he
    keeps a line; cached behind the profile), the task's own system line last of the system blocks, then the turns
    given (the user turn with the material last, after the cache breakpoints)."""
    out = []
    b = system_block(task)
    if b:
        out.append({"role": "system", "content": b})
    lb = learned_block(task)
    if lb:
        out.append({"role": "system", "content": lb})
    out.append({"role": "system", "content": task_system})
    out.extend(rest)
    return out


# ---------------------------------------------------------------- the worker's fetch
def fetch(cl, log=None) -> dict:
    """GET /api/worker/profile/voice through the worker's client. The copy here is replaced only when `updated_at`
    moved; a failed fetch keeps the last copy and says so (a job never fails for it). Logs: length, sha, updated_at."""
    say = log or (lambda *a: None)
    try:
        d = cl.call("GET", "/api/worker/profile/voice") or {}
    except Exception as e:  # noqa: BLE001  the API or the network: keep what we have, say so
        say(f"voice profile: not fetched ({type(e).__name__}: {str(e)[:120]}); "
            + ("using the copy from " + str(_STATE["updated_at"]) if loaded() else "drafts run without it"))
        return {**status(), "error": type(e).__name__}
    text = d.get("text") if isinstance(d, dict) else None
    if not isinstance(text, str) or not text.strip():
        if loaded():
            say("voice profile: removed on the server; drafts run without it")
        _STATE.update(text=None, updated_at=None, by=None, source=None)   # the learned section stays (below)
        _take_learned(d if isinstance(d, dict) else {}, say)
        return {**status(), "cached": False}
    if loaded() and d.get("updated_at") == _STATE["updated_at"] and sha(text) == sha(_STATE["text"]):
        _take_learned(d, say)
        return {**status(), "cached": True}
    set_profile(text, d.get("updated_at"), d.get("by"), source="api")
    s = status()
    say(f"voice profile: loaded ({s['chars']} chars, sha {s['sha']}, updated {s['updated_at']})")
    _take_learned(d, say)   # after the profile, so the cache line below tells the truth about the prefix
    return {**status(), "cached": False}


def _take_learned(d: dict, say) -> None:
    """The learned section and the laugh lines from the same GET; logged as counts, once per change."""
    l, laughs = d.get("learned"), d.get("laugh_lines")
    before = (_LEARNED["updated_at"], len(_LEARNED["laughs"]))
    set_learned(l if isinstance(l, dict) else None, laughs if isinstance(laughs, list) else [])
    after = (_LEARNED["updated_at"], len(_LEARNED["laughs"]))
    if after != before and learned():
        c = status()["learned"]
        how = cacheable_learned("creative_riff")
        say(f"voice profile: learned section ({c['kept']} kept, {c['dropped']} passed over, {c['laughs']} laugh lines, "
            f"updated {c['updated_at']}); cached "
            + ("as the prefix behind the profile" if how == "prefix" else "never: too short on its own, no profile ahead of it"))
