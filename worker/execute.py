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
    """The corpus over the data folder (online-first: the workspace). reload=True really re-reads it: load_corpus()
    caches per process, so a long-running worker kept the corpus of its first job (fixed 2026-10-27)."""
    global _CORPUS
    from creative.data import Corpus, comedy_dir
    if _CORPUS is None or reload:
        _CORPUS = Corpus.load(comedy_dir())
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
    if kind == "set_text":
        return analyze_transcript(inp, complete=complete)
    if kind in ("embed", "memolink"):
        raise ValueError(f"{kind} runs in online-first mode only (python -m worker --online)")
    raise ValueError(f"unknown job kind {kind}")


# ---------------------------------------------------------------- a set transcribed on the phone (no audio here)
TRANSCRIPT_MAX_CHARS = 400_000
WORDS_PER_S = 2.5   # for synthetic timestamps when the phone sent none: monotone, never trusted as timing


def phone_transcript(inp: dict) -> dict:
    """The job's segments -> the engine's transcript shape. Timestamps are `device` when every segment has start
    and end, else `synthetic` (word-count based, monotone) so the split step can run; meta.json says which."""
    segs = inp.get("segments")
    if not isinstance(segs, list) or not segs:
        raise ValueError("set_text needs segments: [{text, start?, end?}]")
    out, t, chars = [], 0.0, 0
    timed = all(isinstance(s, dict) and isinstance(s.get("start"), (int, float)) and isinstance(s.get("end"), (int, float))
                and not isinstance(s.get("start"), bool) and s["end"] >= s["start"] for s in segs)
    for s in segs:
        if not isinstance(s, dict) or not isinstance(s.get("text"), str) or not s["text"].strip():
            continue
        text = s["text"].strip()
        chars += len(text)
        if chars > TRANSCRIPT_MAX_CHARS:
            raise ValueError("transcript too long")
        if timed:
            a, b = float(s["start"]), float(s["end"])
        else:
            a = t
            b = t + max(0.5, len(text.split()) / WORDS_PER_S)
            t = b
        out.append({"start": a, "end": b, "text": text, "words": []})
    if not out:
        raise ValueError("set_text: every segment is empty")
    return {"model": "phone:" + str(inp.get("engine") or "speech")[:40], "segments": out,
            "timestamps": "device" if timed else "synthetic"}


def analyze_transcript(inp: dict, complete=None, make_inbox=None) -> dict:
    """The text-only part of set analysis for a set transcribed on the phone (job kind `set_text`,
    POST /api/sets/transcript). Runs the engine's split step (the Claude call: transcript -> bits) over the
    transcript and writes corpus/sets/<set_dir>/{transcript,meta,analysis}.json. Every audio-derived field (laughs,
    laugh seconds, room heat, dead air, clips) is ABSENT and `audio.status` is "unavailable" until a recording with
    timing is processed; synthetic timestamps are marked as such. Nothing here hears audio."""
    import hashlib
    from datetime import datetime
    from engine.config import Config
    from engine.inbox import Inbox
    tx = phone_transcript(inp)
    sha = hashlib.sha256(json.dumps(tx["segments"], ensure_ascii=False, sort_keys=True).encode("utf-8")).hexdigest()
    date = (inp.get("recorded_at") or "")[:10]
    if not re.match(r"^\d{4}-\d{2}-\d{2}$", date):
        date = datetime.now().strftime("%Y-%m-%d")
    set_dir = f"{date.replace('-', '')}_ph{sha[:9]}"
    title = (inp.get("title") or f"Set {date} (phone)")[:200]
    cfg = Config()
    if make_inbox is not None:
        ib = make_inbox(cfg)
    else:
        from engine.providers import Router
        router = Router(cfg, log_path=cfg.runs_log)
        if complete is not None:  # a caller-supplied completion (tests, the creative wrapper): one call shape
            class _R:
                def complete(self, task, messages, schema=None, **kw):
                    return complete(task, messages, schema)
            router = _R()
        # only split() is called: no audio stage, no embedder, no transcriber may run here
        ib = Inbox(cfg, router=router, embedder=_NoEmbedder(), transcriber=_never, laughs=_never,
                   normalizer=_never, prober=_never, log=lambda s: None)
    run_id = f"settext-{datetime.now().strftime('%Y%m%d-%H%M%S')}-{sha[:8]}"
    units = ib.split(tx, {"laughs": []}, True, run_id, set_dir)
    dur = float(inp.get("duration_s") or tx["segments"][-1]["end"] or 0)
    rows = []
    for u in units:
        rows.append({"id": u["id"], "name": u.get("name"), "kind": u.get("kind"), "text": u["text"],
                     "funny": bool(u.get("funny", True)), "start_ms": u.get("start_ms"), "end_ms": u.get("end_ms"),
                     # audio-derived fields: not here, and not pretended
                     "laugh_s": None, "laugh_s_per_min": None, "pct_laughing": None, "vs_room": None,
                     "time_to_first_laugh_s": None, "dead_air_max_s": None})
    audio = {"status": "unavailable", "why": "transcribed on the phone; laughs, room heat and timing need the recording",
             "timestamps": tx["timestamps"]}
    meta = {"id": f"ph{sha[:9]}", "title": title, "source": "phone_transcript", "recorded_at": inp.get("recorded_at"),
            "show_date": date, "show_date_source": "the phone (when it was recorded)" if inp.get("recorded_at") else "today",
            "show_venue": inp.get("venue"), "duration_s": dur or None, "transcript_sha256": sha, "audio": audio,
            "received_at": inp.get("received_at")}
    analysis = {"set": set_dir, "source": "set_text", "run_id": run_id, "audio": audio, "rows": rows,
                "summary": {"material_s": None, "laugh_s_total": None, "laugh_s_per_min": None, "pct_laughing": None,
                            "laughs_merged": None, "laughs_per_min_merged": None, "applause_s": None,
                            "longest_laugh_s": None, "longest_laugh_joke": None, "dead_air_max_s": None,
                            "dead_air_max_at_ms": None, "room_heat": None, "room_heat_basis": "unavailable: no audio",
                            "units": len(rows), "funny_units": sum(1 for r in rows if r["funny"])}}
    out_dir = cfg.p("out_corpus_dir") / "sets" / set_dir
    out_dir.mkdir(parents=True, exist_ok=True)
    for name, doc in (("transcript.json", tx), ("meta.json", {k: v for k, v in meta.items() if v is not None}),
                      ("analysis.json", analysis)):
        (out_dir / name).write_text(json.dumps(doc, ensure_ascii=False, indent=1), encoding="utf-8")
    return {"status": "analyzed_text", "kind": "set", "set_dir": set_dir, "units": len(rows),
            "funny_units": analysis["summary"]["funny_units"], "audio": audio, "run_id": run_id}


class _NoEmbedder:
    def embed(self, *a, **k):
        raise RuntimeError("set_text runs the split step only; nothing is embedded here")


def _never(*a, **k):
    raise RuntimeError("set_text has no audio: this stage must not run")


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
