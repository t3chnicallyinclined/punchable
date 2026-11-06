"""Film it's two sessions on one version of a bit (docs/design/SPEC-2026-11-03-film-it.md D10, D11, §2.5, §7):

    rant(inp)    "Make it a rant": whole-line hunks over the version (change / cut / add) that turn it to face a phone
                 camera: second person, present tense, one escalation per line, the punch kept as the last beat of
                 the run, tags after it. He keeps or skips each hunk on the phone; nothing here edits the bit.
    hooks(inp)   "Pick a hook": three one-line openers for the version; `exclude` (the three he already saw) asks
                 for three others (the phone asks once).

The input is built by the API from the version it holds (`POST /api/jokes/<id>/rant` / `/hook`, api-dev/
creative_api.py): `{joke_id, title, unit, cut, lines, punch, exclude?}`; the lines are the version's text split as
the app splits it, so a hunk's `line_no` points at the same line on both sides. The prompts are the punch-up's
pattern: his voice profile first, the learned block second (both cached, creative/voice.py), the task's own system
line (with the public method, invented examples only) third, the numbered lines last.

Every hunk that claims to quote his line is checked against the version word for word (`was` must be that line,
whitespace aside): a hunk whose quoted line is not there is dropped, never shown. With a punch line given, a change
that loses its reveal word (the last word of the punch line) or a cut of the punch line is dropped too. A hook equal
to a line of the version, or to one he already saw, is dropped. Nothing here writes: the result is drafts.
"""
from __future__ import annotations

import re

from . import voice
from .util import provider_why
from .write import DRAFT_LABEL

RANT_TASK = "creative_rant"
HOOK_TASK = "creative_hook"
MAX_LINE_CHARS = 400     # a hunk's new line, at most (a longer one is not a line he says in one breath)
MAX_HOOK_CHARS = 300
MAX_HOOK_WORDS = 24
HOOKS = 3                # openers shown per round

RANT_SYSTEM = ("You rework one comedian's own bit so he can say it straight into a phone's front camera. You "
               "suggest changes line by line; he keeps or skips each one. Never invent biography.")
RANT_METHOD = (
    "How a rant cut works. He is filming himself, looking into the lens, talking to one person. Rework the bit for "
    "that, one whole line at a time:\n"
    "- Second person and present tense: \"You scan forty things.\" not \"I scanned forty things.\" For this cut that "
    "overrides the first-person rule above; keep his words everywhere else.\n"
    "- Direct address: he talks to the viewer, never about \"the audience\" or \"you guys\".\n"
    "- Escalation: each line pushes a little further than the one before it; nothing goes flat in the middle.\n"
    "- The punch stays the last beat of the run: keep its reveal word, and keep it last before the tags.\n"
    "- Tags come after the punch, short, each a fresh hit on the same idea.\n"
    "- Leave a line alone when it already works for the camera: no hunk is better than a change for its own sake.\n"
    "- Spoken and short, one breath per line. No stage directions, no emoji, no hashtags.\n"
    "Example (invented): \"Self-checkout is the store hiring you for free.\" can become \"You know what self-checkout "
    "is? It's the store hiring you. For free.\"; \"I scanned forty things and nobody clapped.\" can become \"You "
    "scan forty things. Nobody claps.\""
)
HOOK_SYSTEM = ("You write openers for one comedian's bit, the first line he says into a phone's front camera. You "
               "suggest; he picks one or none. Never invent biography.")
HOOK_METHOD = (
    "A hook is one spoken line that makes a stranger stay for the second line. It earns the bit's first line: it "
    "points at the premise without giving away the punch. Short (at most 24 words), in his voice, no question bait "
    "(\"Have you ever…\"), no \"wait for it\", no emoji, no hashtags. Each of the three takes a different angle "
    "(a confession, a claim, a complaint, a scene). Invented examples for a bit about self-checkout: \"Stop scanning "
    "your own groceries.\" · \"I want to talk to whoever invented self-checkout.\" · \"I got fired today. From a "
    "store I don't work at.\""
)

RANT_SCHEMA = {"type": "object", "properties": {"hunks": {"type": "array", "items": {
    "type": "object", "properties": {"op": {"type": "string", "enum": ["change", "cut", "add"]},
                                     "line_no": {"type": "integer"}, "was": {"type": "string"},
                                     "now": {"type": "string"}, "why": {"type": "string"}},
    "required": ["op", "line_no", "was", "now", "why"], "additionalProperties": False}}},
    "required": ["hunks"], "additionalProperties": False}
HOOK_SCHEMA = {"type": "object", "properties": {"hooks": {"type": "array", "items": {
    "type": "object", "properties": {"text": {"type": "string"}, "why": {"type": "string"}},
    "required": ["text", "why"], "additionalProperties": False}}},
    "required": ["hooks"], "additionalProperties": False}

WORD_RE = re.compile(r"\w+")


def _complete():
    from .story import _creative_complete
    return _creative_complete()


def same(a: str, b: str) -> bool:
    """His line and a quote of it are the same line: equal once runs of whitespace are one space."""
    return " ".join((a or "").split()) == " ".join((b or "").split())


def words(s: str) -> list[str]:
    return WORD_RE.findall((s or "").lower())


def reveal_word(line: str) -> str | None:
    """The punch line's reveal word: its last word."""
    w = words(line)
    return w[-1] if w else None


def numbered(lines: list[str], punch: int | None) -> str:
    return "\n".join(f"{i + 1}. {ln}" + ("   <- the punch" if punch == i else "") for i, ln in enumerate(lines))


def _clean_input(inp: dict) -> tuple[list[str], int | None]:
    lines = [x for x in (inp.get("lines") or []) if isinstance(x, str) and x.strip()]
    p = inp.get("punch")
    punch = p if isinstance(p, int) and not isinstance(p, bool) and 0 <= p < len(lines) else None
    return lines, punch


def _base(inp: dict, lines: list[str], punch: int | None) -> dict:
    return {"joke_id": inp.get("joke_id"), "title": inp.get("title"), "unit": inp.get("unit"),
            "cut": inp.get("cut") or "stage", "lines": lines, "punch": punch, "draft": True, "label": DRAFT_LABEL}


# ---------------------------------------------------------------- Make it a rant
def rant_messages(inp: dict) -> list[dict]:
    lines, punch = _clean_input(inp)
    head = f"The bit \"{inp.get('title') or 'untitled'}\", line by line (his words, verbatim):"
    msg = (head + "\n" + numbered(lines, punch) + "\n\n"
           "Make it a rant cut. Return one hunk per line you change and nothing for a line you keep:\n"
           "- change: line_no = the line, was = that line exactly as written above, now = the new line\n"
           "- cut: line_no = the line, was = that line exactly as written above, now = \"\"\n"
           "- add: line_no = the line it goes after (0 = before the first), was = \"\", now = the new line\n"
           "`why` in three to six plain words. Copy `was` exactly; a hunk whose `was` is not the line is thrown away.")
    return voice.messages(RANT_TASK, RANT_SYSTEM + "\n\n" + RANT_METHOD, {"role": "user", "content": msg})


def rant_hunks(data: dict | None, lines: list[str], punch: int | None) -> tuple[list[dict], int]:
    """Claude's hunks, checked: (kept, dropped). One change or cut per line (the first wins); `was` must be the line;
    a change must change the words; a punch line keeps its reveal word and is never cut."""
    out, dropped, touched = [], 0, set()
    rw = reveal_word(lines[punch]) if punch is not None else None
    n = len(lines)
    for h in (data or {}).get("hunks") or []:
        if not isinstance(h, dict):
            dropped += 1
            continue
        op, k = h.get("op"), h.get("line_no")
        was = h.get("was") if isinstance(h.get("was"), str) else ""
        now = (h.get("now") if isinstance(h.get("now"), str) else "").strip()
        why = (h.get("why") if isinstance(h.get("why"), str) else "").strip()[:80] or None
        if not isinstance(k, int) or isinstance(k, bool) or len(now) > MAX_LINE_CHARS:
            dropped += 1
            continue
        if op == "add":
            if not (0 <= k <= n) or not now:
                dropped += 1
                continue
            out.append({"op": "add", "line_no": k, "was": "", "now": now, "why": why})
            continue
        if op not in ("change", "cut") or not (1 <= k <= n) or k in touched or not same(was, lines[k - 1]):
            dropped += 1
            continue
        if op == "cut":
            if punch == k - 1:
                dropped += 1
                continue
            touched.add(k)
            out.append({"op": "cut", "line_no": k, "was": lines[k - 1], "now": "", "why": why})
            continue
        if not now or words(now) == words(lines[k - 1]) or (punch == k - 1 and rw and rw not in words(now)):
            dropped += 1
            continue
        touched.add(k)
        out.append({"op": "change", "line_no": k, "was": lines[k - 1], "now": now, "why": why})
    return out, dropped


def rant(inp: dict, complete=None) -> dict:
    lines, punch = _clean_input(inp)
    base = dict(_base(inp, lines, punch), hunks=[], dropped=0)
    if not lines:
        return dict(base, status="empty", why="This version has no lines.")
    complete = complete or _complete()
    try:
        r = complete(RANT_TASK, rant_messages(inp), RANT_SCHEMA)
    except Exception as ex:  # noqa: BLE001  provider down: say so, never fake a rant
        return dict(base, status="provider_error", why=provider_why(ex))
    hunks, dropped = rant_hunks(getattr(r, "data", None), lines, punch)
    for h in hunks:
        h.update(draft=True, label=DRAFT_LABEL)
    return dict(base, status="ok" if hunks else "none", hunks=hunks, dropped=dropped,
                provider=getattr(r, "provider", None), model=getattr(r, "model", None),
                call_id=getattr(r, "call_id", None), task=RANT_TASK)


# ---------------------------------------------------------------- Pick a hook
def _exclude(inp: dict) -> list[str]:
    return [x.strip() for x in (inp.get("exclude") or []) if isinstance(x, str) and x.strip()]


def hook_messages(inp: dict) -> list[dict]:
    lines, punch = _clean_input(inp)
    ex = _exclude(inp)
    msg = (f"The bit \"{inp.get('title') or 'untitled'}\", line by line (his words, verbatim):\n"
           + numbered(lines, punch) + "\n\n"
           + f"Write {HOOKS} openers he could say before line 1, each one line, each a different angle; `why` in "
             "three to six plain words."
           + ("\nHe has seen these and wants others (do not repeat them or their angle):\n"
              + "\n".join(f"- {x}" for x in ex) if ex else ""))
    return voice.messages(HOOK_TASK, HOOK_SYSTEM + "\n\n" + HOOK_METHOD, {"role": "user", "content": msg})


def hook_rows(data: dict | None, lines: list[str], exclude: list[str]) -> tuple[list[dict], int]:
    """Claude's openers, checked: (kept, dropped). One line, at most 24 words; never a line of the bit, one he
    already saw, or a repeat; three at most."""
    seen = {tuple(words(x)) for x in lines} | {tuple(words(x)) for x in exclude}
    out, dropped = [], 0
    for h in (data or {}).get("hooks") or []:
        text = (h.get("text") if isinstance(h, dict) and isinstance(h.get("text"), str) else "").strip()
        w = tuple(words(text))
        if (not w or "\n" in text or len(text) > MAX_HOOK_CHARS or len(text.split()) > MAX_HOOK_WORDS or w in seen
                or len(out) >= HOOKS):
            dropped += 1
            continue
        seen.add(w)
        why = (h.get("why") if isinstance(h.get("why"), str) else "").strip()[:80] or None
        out.append({"text": text, "why": why, "draft": True, "label": DRAFT_LABEL})
    return out, dropped


def hooks(inp: dict, complete=None) -> dict:
    lines, punch = _clean_input(inp)
    ex = _exclude(inp)
    base = dict(_base(inp, lines, punch), hooks=[], exclude=ex, dropped=0)
    if not lines:
        return dict(base, status="empty", why="This version has no lines.")
    complete = complete or _complete()
    try:
        r = complete(HOOK_TASK, hook_messages(inp), HOOK_SCHEMA)
    except Exception as ex_:  # noqa: BLE001
        return dict(base, status="provider_error", why=provider_why(ex_))
    rows, dropped = hook_rows(getattr(r, "data", None), lines, ex)
    return dict(base, status="ok" if rows else "none", hooks=rows, dropped=dropped,
                provider=getattr(r, "provider", None), model=getattr(r, "model", None),
                call_id=getattr(r, "call_id", None), task=HOOK_TASK)
