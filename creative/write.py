"""The Write screen's two slow drafts, run by the home worker (never by the API process):

    check_facts(text)          "Check this": a few plain facts about an idea, each with a source that was fetched
                               and actually contains the fact's key words. No source, or a source that doesn't
                               load or doesn't say it: the fact is dropped, never shown bare.
    riff_ideas(c, idea, lines) "Ideas from the AI": up to 3 draft lines in his voice, grounded in his own related
                               bits (quoted to the model verbatim). Always `draft: true`, never added for him.
"""
from __future__ import annotations

import os
import html
import re
import urllib.request

from .data import Corpus
from .notebook import related, tokens

COMIC = os.environ.get("PK_COMIC_NAME", "the comic")  # how the AI prompts name you; PK_COMIC_NAME
COMIC_FIRST = os.environ.get("PK_COMIC_FIRST", COMIC)

DRAFT_LABEL = "DRAFT · AI · not your words yet"
FACTS_SCHEMA = {"type": "object", "properties": {"facts": {"type": "array", "items": {
    "type": "object", "properties": {"fact": {"type": "string"}, "source_title": {"type": "string"},
                                     "source_url": {"type": "string"}},
    "required": ["fact", "source_title", "source_url"], "additionalProperties": False}}},
    "required": ["facts"], "additionalProperties": False}
RIFF_SCHEMA = {"type": "object", "properties": {"ideas": {"type": "array", "items": {
    "type": "object", "properties": {"line": {"type": "string"}, "built_on": {"type": "array", "items": {"type": "string"}}},
    "required": ["line", "built_on"], "additionalProperties": False}}},
    "required": ["ideas"], "additionalProperties": False}
URL_RE = re.compile(r"^https://[A-Za-z0-9.-]+\.[A-Za-z]{2,}(/[^\s]*)?$")


def _complete():
    from .story import _creative_complete
    return _creative_complete()


def fetch_text(url: str, timeout: float = 8.0) -> str | None:
    """The page's visible text (lower-cased), or None if it doesn't load."""
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 (Punchable fact check)"})
        with urllib.request.urlopen(req, timeout=timeout) as r:
            if r.status != 200:
                return None
            raw = r.read(2_000_000).decode("utf-8", "replace")
    except Exception:  # noqa: BLE001
        return None
    raw = re.sub(r"(?is)<(script|style)[^>]*>.*?</\1>", " ", raw)
    return html.unescape(re.sub(r"<[^>]+>", " ", raw)).lower()


def supported(fact: str, page: str | None) -> bool:
    """At least two thirds of the fact's content words (and at least 2) appear on the page."""
    if not page:
        return False
    ws = [w for w in dict.fromkeys(tokens(fact)) if len(w) >= 4]
    if len(ws) < 2:
        return False
    hit = sum(1 for w in ws if w in page)
    return hit >= 2 and hit * 3 >= len(ws) * 2


def check_facts(text: str, complete=None, fetch=None, cap: int = 3) -> dict:
    text = (text or "").strip()
    base = {"idea": text, "draft": True, "label": "Checked by the AI against its sources · read the source"}
    if not text:
        return dict(base, status="empty", facts=[], dropped=0)
    complete = complete or _complete()
    fetch = fetch or fetch_text
    msg = (f"A comedian is writing about this idea:\n\"{text}\"\n\nGive up to {cap + 2} short, plain, checkable facts that "
           "would help him understand the subject (how it works, real numbers, history). One sentence each. For each, "
           "the https URL of a public page (Wikipedia, an encyclopedia, a news or reference site) that states it. "
           "No jokes. If you are not sure of a fact or its source, leave it out.")
    try:
        r = complete("creative_facts", [{"role": "system", "content": "You are a careful fact checker. Every fact must be "
                                         "stated on the page you cite."}, {"role": "user", "content": msg}], FACTS_SCHEMA)
    except Exception as ex:  # noqa: BLE001  provider down: say so, never fake facts
        return dict(base, status="provider_error", facts=[], dropped=0, why=f"{type(ex).__name__}: {str(ex)[:160]}")
    out, dropped = [], 0
    for f in (getattr(r, "data", None) or {}).get("facts") or []:
        fact, url = (f.get("fact") or "").strip(), (f.get("source_url") or "").strip()
        if not fact or not URL_RE.match(url):
            dropped += 1
            continue
        if not supported(fact, fetch(url)):
            dropped += 1
            continue
        out.append({"text": fact, "source_title": (f.get("source_title") or url).strip()[:120], "source_url": url})
        if len(out) >= cap:
            break
    return dict(base, status="ok" if out else "none", facts=out, dropped=dropped,
                provider=getattr(r, "provider", None), model=getattr(r, "model", None))


def riff_ideas(c: Corpus, idea: str, lines: list[str] | None = None, complete=None, cap: int = 3) -> dict:
    idea = (idea or "").strip()
    lines = [x.strip() for x in (lines or []) if isinstance(x, str) and x.strip()][:20]
    base = {"idea": idea, "draft": True, "label": DRAFT_LABEL}
    rel = related(c, idea + "\n" + "\n".join(lines), cap=6)["items"]
    grounds = [{"id": f"B{i + 1}", "joke_id": it["joke_id"], "title": it["title"], "quote": it["quote"]}
               for i, it in enumerate(rel) if it.get("quote")]
    if not grounds and not lines:
        return dict(base, status="no_grounding", ideas=[], grounded_in=[],
                    why="Nothing in your notebook or your riff to build on yet. Write a line first.")
    msg = (f"{COMIC_FIRST}'s idea: \"{idea}\"\n"
           + ("His riff so far (his words):\n" + "\n".join(f"- {x}" for x in lines) + "\n" if lines else "")
           + ("Related bits from his own notebook (verbatim):\n" + "\n".join(f"{g['id']} {g['title']}: \"{g['quote']}\"" for g in grounds) + "\n"
              if grounds else "")
           + f"Draft up to {cap} new lines he could try next, in his voice (first person, spoken, short). Each line "
             "builds on his riff or on the bits above; cite their ids in `built_on` (use \"riff\" for his riff). "
             "No new facts about his life.")
    complete = complete or _complete()
    try:
        r = complete("creative_riff", [{"role": "system", "content": "You help one comedian brainstorm. You suggest; "
                                        "he decides. Never invent biography."}, {"role": "user", "content": msg}], RIFF_SCHEMA)
    except Exception as ex:  # noqa: BLE001
        return dict(base, status="provider_error", ideas=[], grounded_in=grounds, why=f"{type(ex).__name__}: {str(ex)[:160]}")
    known = {g["id"] for g in grounds} | {"riff"}
    ideas = []
    for x in (getattr(r, "data", None) or {}).get("ideas") or []:
        line = (x.get("line") or "").strip()
        if not line:
            continue
        ideas.append({"text": line[:400], "built_on": [b for b in x.get("built_on") or [] if b in known],
                      "draft": True, "label": DRAFT_LABEL})
        if len(ideas) >= cap:
            break
    return dict(base, status="ok" if ideas else "none", ideas=ideas, grounded_in=grounds,
                provider=getattr(r, "provider", None), model=getattr(r, "model", None))
