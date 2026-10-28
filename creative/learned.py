"""What the app has learned from him: the lines he kept, the lines he passed over, the lines that got laughs.

The hand-written voice profile (creative/voice.py, `profile:voice`.text) is his: the app never edits it. This module
is the SECOND, evidence-only section of the same document (`profile:voice`.learned) and the prompt block built from
it, maintained by the API on write events he already makes (no new chore, no PUT):

    kept     a riff or punch-up draft he took as his own ("Use as mine"): POST /api/ideas with the text of a finished
             riff job's idea (server.Handler.idea_result), or the Brainstorm decision that adds a unit with
             source "draft" (POST /api/decision) with the text of a finished punch-up job's draft
    dropped  a draft he dismissed ("Dismiss" on Write or Brainstorm: POST /api/profile/voice/learn {dropped},
             profile_api.learn_dropped), and the OTHER drafts of a job he kept one from: the ones he saw next to the
             kept line and neither kept nor dismissed (passed over beside a keep)
    edited   a kept line the app flagged as a draft (`draft: true` on POST /api/ideas) whose words no longer match
             any draft exactly: his line, edited from a draft; shown as such in the block
    laughs   not stored: computed from the corpus each time the worker fetches the profile (laugh_lines), the
             top punchlines by the laugh the detector tied to them (analysis.json punchlines + laughs_auto.json)

Everything here is pure (dicts in, dicts out) so creative/tests/test_learned.py proves the caps, the merge and the
block. Nothing is logged by this module; voice.status() reports counts only.
"""
from __future__ import annotations

import re

CAP_KEPT = 25
CAP_DROPPED = 25
CAP_LAUGH = 15
LINE_CHARS = 280          # a kept line longer than this is cut (a riff line is at most 2000 chars on the way in)
DRAFT_KINDS = ("riff", "punchup")

HEAD = ("What he has done with the app's drafts lately, newest first: evidence, not instructions. A line he kept is "
        "one he took as his own; a line he passed over sat next to a kept one and he left it. Draft toward the kept "
        "ones and away from the passed-over ones in shape, length and move; never repeat any of them.")
LAUGH_HEAD = "Lines of his that got the biggest laughs on tape (verbatim), for the shape and the rhythm:"


def empty() -> dict:
    return {"kept": [], "dropped": [], "updated_at": None}


def _norm(text: str) -> str:
    return re.sub(r"\s+", " ", (text or "")).strip().lower()


def _entry(x, ts: str | None) -> dict | None:
    """One line as stored: {text, bit, date}. A string is a bare line; a dict may carry its bit id and date."""
    if isinstance(x, str):
        x = {"text": x}
    if not isinstance(x, dict) or not isinstance(x.get("text"), str):
        return None
    text = re.sub(r"\s+", " ", x["text"]).strip()
    if not text:
        return None
    if len(text) > LINE_CHARS:
        cut = text.rfind(" ", 0, LINE_CHARS)
        text = text[:cut if cut > LINE_CHARS // 2 else LINE_CHARS].rstrip() + " [...]"
    bit = x.get("bit")
    date = x.get("date") or (ts or "")[:10] or None
    out = {"text": text, "bit": bit if isinstance(bit, str) and bit else None, "date": date}
    if x.get("edited") is True:   # his own words, edited from a draft (the app said `draft: true`, no exact match)
        out["edited"] = True
    return out


def merge(cur: dict | None, kept=None, dropped=None, ts: str | None = None) -> dict:
    """The learned section after one event: the new lines first, duplicates (same words, any case or spacing) folded
    into the newest, a line that is kept leaves `dropped` and the other way round (the latest action wins), each
    list cut to its cap. `cur` is never mutated."""
    cur = cur if isinstance(cur, dict) else empty()
    new_k = [e for e in (_entry(x, ts) for x in (kept or [])) if e]
    new_d = [e for e in (_entry(x, ts) for x in (dropped or [])) if e]
    k_keys = {_norm(e["text"]) for e in new_k}
    new_d = [e for e in new_d if _norm(e["text"]) not in k_keys]   # a line kept in this event is not also dropped
    d_keys = {_norm(e["text"]) for e in new_d}

    def fold(fresh: list[dict], old: list, drop_keys: set) -> list[dict]:
        out, seen = [], set()
        for e in fresh + [y for y in (_entry(x, None) for x in (old or [])) if y]:
            key = _norm(e["text"])
            if key in seen or key in drop_keys:
                continue
            seen.add(key)
            out.append(e)
        return out

    return {"kept": fold(new_k, cur.get("kept"), d_keys)[:CAP_KEPT],
            "dropped": fold(new_d, cur.get("dropped"), k_keys)[:CAP_DROPPED],
            "updated_at": ts if (new_k or new_d) else cur.get("updated_at")}


def counts(learned: dict | None) -> dict:
    l = learned if isinstance(learned, dict) else empty()
    return {"kept": len(l.get("kept") or []), "dropped": len(l.get("dropped") or []), "updated_at": l.get("updated_at")}


# ---------------------------------------------------------------- which job drafted a line he kept
def drafted(jobs: dict, text: str) -> tuple[str | None, list[str]]:
    """Was `text` one of the app's drafts? Looks through the finished riff and punch-up jobs (the API's job store,
    creative_api.job_apply: `result.ideas[].text` / `result.drafts[].alt`). Returns (kind, the OTHER drafts of the
    newest such job) or (None, []) when the text is his own, not a draft. Exact words, case and spacing aside."""
    key = _norm(text)
    if not key:
        return None, []
    best, best_ts = None, ""
    for j in (jobs or {}).values():
        if not isinstance(j, dict) or j.get("status") != "done" or j.get("kind") not in DRAFT_KINDS:
            continue
        r = j.get("result") or {}
        items = r.get("ideas") if j["kind"] == "riff" else r.get("drafts")
        texts = [(x.get("text") if j["kind"] == "riff" else x.get("alt")) for x in (items or []) if isinstance(x, dict)]
        texts = [t for t in texts if isinstance(t, str) and t.strip()]
        if key not in {_norm(t) for t in texts}:
            continue
        ts = str(j.get("done_at") or "")
        if best is None or ts > best_ts:
            best, best_ts = (j["kind"], [t for t in texts if _norm(t) != key]), ts
    return best if best else (None, [])


# ---------------------------------------------------------------- the laugh lines, from the corpus
def laugh_lines(c, cap: int = CAP_LAUGH) -> list[dict]:
    """His top punchlines by the laugh the detector tied to them, across every taped night: {text, bit, date,
    laugh_s} (laugh_s from laughs_auto.json when the map has timing, else from the bars). Empty when no tape has a
    laugh map (phone transcripts have none). Never raises: a corpus without perfs gives []."""
    perfs = getattr(c, "perfs", None) or {}
    laughs = getattr(c, "laughs", None) or {}
    best: dict[str, dict] = {}
    for jid, ps in perfs.items():
        for p in ps or []:
            events = laughs.get(p.get("set")) or {}
            for pl in p.get("punchlines") or []:
                text = re.sub(r"\s+", " ", str(pl.get("text") or "")).strip()
                if not text or len(text) < 8:
                    continue
                ev = events.get(pl.get("laugh_id")) or {}
                s0, s1 = ev.get("start_ms"), ev.get("end_ms")
                if isinstance(s0, (int, float)) and isinstance(s1, (int, float)) and s1 > s0:
                    secs = round((s1 - s0) / 1000, 1)
                else:
                    secs = round(float(pl.get("bars") or 0) * 1.0, 1)    # no timing: a bar counts as a second
                if secs <= 0:
                    continue
                key = _norm(text)
                cur = best.get(key)
                if cur is None or secs > cur["laugh_s"]:
                    best[key] = {"text": text[:LINE_CHARS], "bit": jid, "date": p.get("date"), "laugh_s": secs}
    out = sorted(best.values(), key=lambda e: (-e["laugh_s"], e["date"] or "", e["text"]))
    return out[:cap]


# ---------------------------------------------------------------- the prompt block
def block(learned: dict | None, laughs: list[dict] | None = None) -> str | None:
    """The second system block for a writing prompt, or None when there is nothing learned yet (the prompt is then
    exactly what it was). Dates are shown as the month and year, never ids."""
    l = learned if isinstance(learned, dict) else empty()
    kept, dropped, laughs = l.get("kept") or [], l.get("dropped") or [], laughs or []
    if not kept and not dropped and not laughs:
        return None

    def line(e: dict) -> str:
        d = (e.get("date") or "")[:7]
        tail = ", his edit of a draft" if e.get("edited") else ""
        return f"- \"{e['text']}\"" + (f" ({d}{tail})" if d else (f" ({tail[2:]})" if tail else ""))

    parts = []
    if kept or dropped:
        parts.append(HEAD)
        if kept:
            parts.append("Lines he kept recently:\n" + "\n".join(line(e) for e in kept))
        if dropped:
            parts.append("Lines he passed over:\n" + "\n".join(line(e) for e in dropped))
    if laughs:
        parts.append(LAUGH_HEAD + "\n" + "\n".join(f"- \"{e['text']}\" ({e.get('laugh_s', 0):.0f} s)" for e in laughs))
    return "\n\n".join(parts)
