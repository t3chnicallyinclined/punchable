"""Run one queued job on the comic's PC through engine.providers (claude_code / ollama), never on the server.

Each kind calls the same creative/engine function the CLI calls, over the LOCAL corpus (comedy/, with decisions
pulled from the phone first), and returns the JSON the API caches and serves:
  bridge       creative.story.bridge_draft(corpus, from, to)
  punchup      creative.workingset.punch_up(corpus, working_set, joke_id)   (the working set comes from the job,
               so his locks on the server are the ones honoured: a locked line is never shown to the model)
  ask          engine.ask.ask(question)
  process_set  engine.inbox.Inbox().process(file)   (the file was downloaded into the local inbox first)
  facts        creative.write.check_facts(text)   (Write's "Check this": every fact's source is fetched and must say it)
  riff         creative.write.riff_ideas(corpus, idea, lines)   (Write's folded "Ideas from the AI": drafts only)
  ask_chat     creative.askchat.run_job(input)   (Ask: one model call over the messages the API built from his
               retrieved sources; the API gates the answer word for word when it reads the result back)
  import_youtube  POST /api/import/youtube: tools/ingest/fetch_youtube.py (yt-dlp: info.json, captions, audio into
               comedy/tapes/raw/<upload_date>_<id>/), tools/ingest/show_dates.py (corpus/sets/<set>/meta.json with
               the show date), then tools/audio/run_all.py --sets <set> in the laughs venv (transcribe, laughs, VAD,
               AST, then infer.sets + infer.clips). Never run by the API process.
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

DESK = Path(__file__).resolve().parent.parent
if str(DESK) not in sys.path:
    sys.path.insert(0, str(DESK))

_CORPUS = None


def corpus(reload: bool = False):
    global _CORPUS
    from creative.data import load_corpus
    if _CORPUS is None or reload:
        _CORPUS = load_corpus()
    return _CORPUS


def run(job: dict, inbox: Path | None = None, file: Path | None = None, complete=None) -> dict:
    kind, inp = job["kind"], job.get("input") or {}
    if kind == "bridge":
        from creative import story as S
        return S.bridge_draft(corpus(reload=True), inp["from"], inp["to"], complete=complete)
    if kind == "punchup":
        from creative import workingset as W
        return W.punch_up(corpus(reload=True), inp["working_set"], inp["joke_id"], complete=complete)
    if kind == "ask":
        from engine.ask import ask
        r = ask(inp["question"])
        return json.loads(json.dumps(r, default=str))
    if kind == "process_set":
        path = file or (Path(inbox) / inp["file"] if inbox else None)
        if not path or not Path(path).is_file():
            raise FileNotFoundError("the uploaded file is not here")
        meta = inp.get("meta") or {}
        side = {"kind": {"set": "set", "note": "voice_note"}.get(meta.get("kind"), None),
                "date": (meta.get("recorded_at") or "")[:10] or None, "venue": meta.get("venue"), "title": meta.get("title")}
        side = {k: v for k, v in side.items() if v}
        if side:
            Path(str(path) + ".json").write_text(json.dumps(side, ensure_ascii=False), encoding="utf-8")
        from engine.inbox import Inbox
        rep = Inbox().process(Path(path))
        keep = ("status", "kind", "set", "set_id", "stats", "totals", "date", "venue", "error")
        return {"status": "processed", "report": {k: rep.get(k) for k in keep if k in rep}}
    if kind == "import_youtube":
        return import_youtube(inp)
    if kind == "facts":
        from creative import write as WR
        return WR.check_facts(inp["text"], complete=complete)
    if kind == "riff":
        from creative import write as WR
        return WR.riff_ideas(corpus(reload=True), inp["idea"], inp.get("lines") or [], complete=complete)
    if kind == "ask_chat":
        from creative import askchat as AC
        return AC.run_job(inp, complete=complete)
    raise ValueError(f"unknown job kind {kind}")


LAUGHS_PY = os.environ.get("PK_LAUGHS_PY", str(Path.home() / ".venvs" / "laughs" / "Scripts" / "python.exe"))


def import_youtube(inp: dict) -> dict:
    """Fetch one YouTube set and run the audio + infer pipelines on it, with the same tools the corpus was built with."""
    from creative.data import comedy_dir
    vid, url = inp.get("video_id") or "", inp.get("url") or ""
    if not re.match(r"^[A-Za-z0-9_-]{11}$", vid) or url != f"https://www.youtube.com/watch?v={vid}":
        raise ValueError("bad import_youtube input")
    comedy = comedy_dir()
    raw = comedy / "tapes" / "raw"
    raw.mkdir(parents=True, exist_ok=True)
    steps = []

    def step(name, cmd, timeout):
        r = subprocess.run(cmd, cwd=str(DESK), capture_output=True, text=True, encoding="utf-8", errors="replace",
                           timeout=timeout, env={**os.environ, "PYTHONUTF8": "1", "COMEDY_DIR": str(comedy)})
        steps.append({"step": name, "rc": r.returncode, "tail": (r.stdout + r.stderr)[-600:]})
        if r.returncode != 0:
            raise RuntimeError(f"{name} failed (rc {r.returncode}): {(r.stderr or r.stdout)[-400:]}")
        return r

    with tempfile.TemporaryDirectory(prefix="pk-yt-") as td:  # downloaded data stays data: nothing in it is run
        links = Path(td) / "links.txt"
        links.write_text(url + "\n", encoding="utf-8")
        r = step("fetch", [sys.executable, "-I", str(DESK / "tools" / "ingest" / "fetch_youtube.py"), str(links), str(raw)], 3600)
    if "FAIL" in r.stdout:
        raise RuntimeError("fetch failed: " + r.stdout[-400:])
    dirs = sorted(d for d in raw.iterdir() if d.is_dir() and d.name.endswith("_" + vid))
    if not dirs:
        raise RuntimeError(f"fetch wrote no tapes/raw/*_{vid} folder")
    set_dir = dirs[-1].name
    if (DESK / "tools" / "ingest" / "show_dates.py").exists():  # optional: your own venue table and date rules
        step("show_dates", [sys.executable, str(DESK / "tools" / "ingest" / "show_dates.py")], 600)
    step("audio", [LAUGHS_PY, "-m", "tools.audio.run_all", "--sets", set_dir], 4 * 3600)
    return {"status": "imported", "set_dir": set_dir, "video_id": vid, "steps": steps}
