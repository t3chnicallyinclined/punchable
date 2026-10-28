"""The riff eval: the same five bits through the OLD riff prompt and the NEW one, side by side, for the comic to judge.

    python tools/prompts/eval_riff.py --bits joke:a,joke:b,joke:c,joke:d,joke:e [--profile <private path>]
                                      [--comedy DIR] [--out DIR] [--dry-run] [--cap 3]

Runs on the desk over the local data folder (COMEDY_DIR, default ~/punchable-data) with the LOCAL engine config
(engine/config.toml or ENGINE_CONFIG: the Claude Code CLI or an ANTHROPIC_API_KEY in ~/.punchable/secrets.env). It
never calls the hosted API and needs no worker token. For each bit the idea is the bit's own title and matching line,
so the riff builds on what he has:
  BEFORE  the prompt as it was before 2026-10-28: one sentence per related bit, no voice profile, no `move`
  AFTER   creative/write.riff_messages: whole bits with their stage line and latest night, the voice profile (from
          --profile, a private file read here and never written anywhere) as the first system block, `move` per idea
Output: one markdown file per bit in a PRIVATE folder (default $COMEDY_DIR/docs/punchable/evals/), with both prompts,
both sets of drafts and a two-column score (0-2 "sounds like me", 0-2 "I'd try it"); the audit's protocol: keep a
change only when the AFTER total is higher. --dry-run writes the prompts and skips the model.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path

DESK = Path(__file__).resolve().parents[2]
if str(DESK) not in sys.path:
    sys.path.insert(0, str(DESK))

# the riff prompt before 2026-10-28 (creative/write.py at commit 03cea28), kept here so BEFORE stays comparable
LEGACY_SCHEMA = {"type": "object", "properties": {"ideas": {"type": "array", "items": {
    "type": "object", "properties": {"line": {"type": "string"}, "built_on": {"type": "array", "items": {"type": "string"}}},
    "required": ["line", "built_on"], "additionalProperties": False}}},
    "required": ["ideas"], "additionalProperties": False}


def legacy_messages(c, idea: str, lines: list[str], cap: int = 3):
    from creative import voice
    from creative.notebook import related
    rel = related(c, idea + "\n" + "\n".join(lines), cap=6)["items"]
    grounds = [{"id": f"B{i + 1}", "joke_id": it["joke_id"], "title": it["title"], "quote": it["quote"]}
               for i, it in enumerate(rel) if it.get("quote")]
    msg = (f"{voice.first()}'s idea: \"{idea}\"\n"
           + ("His riff so far (his words):\n" + "\n".join(f"- {x}" for x in lines) + "\n" if lines else "")
           + ("Related bits from his own notebook (verbatim):\n" + "\n".join(f"{g['id']} {g['title']}: \"{g['quote']}\"" for g in grounds) + "\n"
              if grounds else "")
           + f"Draft up to {cap} new lines he could try next, in his voice (first person, spoken, short). Each line "
             "builds on his riff or on the bits above; cite their ids in `built_on` (use \"riff\" for his riff). "
             "No new facts about his life.")
    return [{"role": "system", "content": "You help one comedian brainstorm. You suggest; he decides. Never invent biography."},
            {"role": "user", "content": msg}], grounds


def idea_for(c, jid: str) -> str:
    from creative.notebook import card
    k = card(c, jid)
    return (k.get("title") or jid) + (f": {k['quote']}" if k.get("quote") else "")


def fence(msgs: list[dict]) -> str:
    out = []
    for m in msgs:
        out.append(f"**{m['role']}**\n\n```text\n{m['content']}\n```")
    return "\n\n".join(out)


def drafts_md(data: dict | None, err: str | None) -> str:
    if err:
        return f"_no drafts: {err}_"
    ideas = (data or {}).get("ideas") or []
    if not ideas:
        return "_no drafts_"
    return "\n".join(f"{i}. {x.get('line', '').strip()}  \n   built on: {', '.join(x.get('built_on') or []) or '-'}"
                     + (f" · move: {x['move']}" if x.get("move") else "") for i, x in enumerate(ideas, 1))


def run(complete, task, msgs, schema):
    if complete is None:
        return None, "dry run"
    try:
        r = complete(task, msgs, schema)
        return getattr(r, "data", None), None
    except Exception as e:  # noqa: BLE001  the eval reports it, in the file
        return None, f"{type(e).__name__}: {str(e)[:200]}"


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="python tools/prompts/eval_riff.py", description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bits", required=True, help="comma-separated joke ids (five, per the audit's protocol)")
    ap.add_argument("--profile", type=Path, default=None, help="the voice profile (a private file; read, never copied)")
    ap.add_argument("--comedy", type=Path, default=Path(os.path.expanduser(os.environ.get("COMEDY_DIR", "~/punchable-data"))))
    ap.add_argument("--out", type=Path, default=None, help="default: <comedy>/docs/punchable/evals")
    ap.add_argument("--cap", type=int, default=3)
    ap.add_argument("--dry-run", action="store_true", help="write the prompts only; no model call")
    a = ap.parse_args(argv)
    out = a.out or (a.comedy / "docs" / "punchable" / "evals")
    if DESK in out.resolve().parents or out.resolve() == DESK:
        raise SystemExit(f"refusing: {out} is inside the public repo; evals quote his material")
    out.mkdir(parents=True, exist_ok=True)
    from creative import voice
    from creative import write as WR
    from creative.data import Corpus
    from creative.util import slug
    c = Corpus.load(a.comedy)
    bits = [b.strip() for b in a.bits.split(",") if b.strip()]
    missing = [b for b in bits if b not in c.jokes]
    if missing:
        raise SystemExit(f"not in the corpus: {missing}")
    if a.profile:
        if DESK in a.profile.resolve().parents:
            raise SystemExit("refusing: the profile must live outside the public repo")
        voice.set_profile(a.profile.read_text(encoding="utf-8"), "eval", "owner", source="file")
        print(f"voice profile: {voice.status()['chars']} chars, sha {voice.status()['sha']}, cacheable: {voice.cacheable('creative_riff')}")
    else:
        voice.clear()
        print("no --profile: AFTER runs without the voice profile (the craft rules still apply)")
    complete = None
    if not a.dry_run:
        from creative.story import _creative_complete
        complete = _creative_complete()
    stamp = time.strftime("%Y%m%d-%H%M")
    written = []
    for jid in bits:
        idea = idea_for(c, jid)
        before_msgs, _ = legacy_messages(c, idea, [], a.cap)
        voice_was = voice.profile()
        voice.clear()  # BEFORE never had the profile
        try:
            before_data, before_err = run(complete, "creative_riff", before_msgs, LEGACY_SCHEMA)
        finally:
            if voice_was:
                voice.set_profile(voice_was["text"], voice_was["updated_at"], voice_was["by"], voice_was["source"])
        after_msgs, _ = WR.riff_messages(c, idea, [], a.cap)
        after_data, after_err = run(complete, "creative_riff", after_msgs, WR.RIFF_SCHEMA)
        p = out / f"riff_{stamp}_{slug(jid.split(':', 1)[-1])}.md"
        p.write_text("\n".join([
            f"# Riff eval · {c.title(jid)} ({jid})", "",
            f"Idea given to both: `{idea}`  ", f"Model: local engine config; profile: {'yes' if voice.loaded() else 'no'}; cap {a.cap}", "",
            "| | sounds like me (0-2) | I'd try it (0-2) |", "|---|---|---|", "| BEFORE | | |", "| AFTER | | |", "",
            "## BEFORE (the prompt before 2026-10-28)", "", "### Drafts", "", drafts_md(before_data, before_err), "",
            "### Prompt", "", fence(before_msgs), "",
            "## AFTER (whole bits, stage line, voice profile first, `move`)", "", "### Drafts", "", drafts_md(after_data, after_err), "",
            "### Prompt", "", fence(after_msgs), "",
        ]), encoding="utf-8")
        written.append(p)
        print(f"{jid}: {'dry run' if a.dry_run else 'done'} -> {p}")
    print(json.dumps({"files": [str(p) for p in written], "out": str(out)}, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
