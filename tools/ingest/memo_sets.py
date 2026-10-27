"""Voice memos that are recordings of a performed set -> real sets (transcript, laughs, analysis), like a YouTube set.

    python tools/ingest/memo_sets.py                      # list what would be routed (triage: performed_set with an
                                                          # audience), long memos only (> 4,000 characters)
    python tools/ingest/memo_sets.py --all-lengths        # ... every performed-set memo, short ones too
    python tools/ingest/memo_sets.py --run [--all-lengths] [--limit N]   # create the sets and run the set pipeline

Which memos: creative/memolink.py `triage` (Claude, offline, cached in kb/creative/suggest_judge.json) reads each memo's
transcript and says `performed_set` when it hears a live audience (laughter, crowd work, a host). Each one becomes
corpus/sets/<yyyymmdd>_m<sha8>/ with the memo's own transcript (Whisper is not run again) and a meta.json that keeps the
reference (the memo's Drive file id, its name and recording date), and tapes/raw/<set>/audio16k.wav decoded from the
memo's audio, so tools/audio/run_all.py can measure the laughs and infer.sets can line the performances up with his
bits. The memo's item in memos.json gets `set: <set dir>`. Court material: the legal rules run on the transcript first.
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK))

LONG_CHARS = 4000


def comedy() -> Path:
    return Path(os.environ.get("COMEDY_DIR", Path.home() / "punchable-data"))


def plan(c: Path, all_lengths: bool) -> list[dict]:
    from creative import memolink as ML
    ms = ML.memos(c)
    try:
        tc = json.loads((c / "kb" / "creative" / "suggest_judge.json").read_text(encoding="utf-8")).get("items") or {}
    except (OSError, ValueError):
        raise SystemExit("no triage yet: run python -m creative memolink triage first")
    out = []
    for k, it in sorted(ms.items(), key=lambda kv: (kv[1].get("date") or "", kv[0])):
        v = tc.get(k) or {}
        if v.get("kind") != "performed_set" or not v.get("audience") or v.get("text_sha") != ML.sha(it["text"]):
            continue
        if not all_lengths and len(it["text"]) <= LONG_CHARS:
            continue
        name = it.get("name_dir") or ""
        sha8 = name.rsplit("_", 1)[-1] if "_" in name else ""
        date = (it.get("date") or "").replace("-", "")
        if len(sha8) != 8 or len(date) != 8 or not it.get("audio"):
            continue
        out.append({"memo": k, "set_dir": f"{date}_m{sha8}", "name_dir": name, "audio": it["audio"],
                    "transcript": it["transcript"], "title": it.get("title"), "date": it.get("date"),
                    "recorded_at": it.get("recorded_at"), "duration_s": it.get("duration_s"),
                    "drive_id": it.get("drive_id"), "original_name": it.get("name"), "chars": len(it["text"]),
                    "triage": {"confidence": v.get("confidence"), "why": v.get("why")}, "text": it["text"]})
    return out


def make_set(c: Path, p: dict, legal) -> str:
    sets, raw = c / "corpus" / "sets" / p["set_dir"], c / "tapes" / "raw" / p["set_dir"]
    if (sets / "laughs_auto.json").is_file() and (sets / "meta.json").is_file():
        return "done"  # analysed already (its decoded audio may be gone: never decoded or copied again)
    if (sets / "meta.json").is_file() and (raw / "audio16k.wav").is_file():
        return "exists"
    if legal(p["text"]):
        return "skipped_legal"
    src = c / p["audio"]
    if not src.is_file():
        return "no_audio"
    raw.mkdir(parents=True, exist_ok=True)
    r = subprocess.run(["ffmpeg", "-nostdin", "-y", "-loglevel", "error", "-i", str(src), "-vn", "-ac", "1", "-ar", "16000",
                        "-c:a", "pcm_s16le", str(raw / "audio16k.wav")], capture_output=True, text=True)
    if r.returncode != 0:
        shutil.rmtree(raw, ignore_errors=True)
        return "ffmpeg_failed: " + r.stderr[-200:]
    sets.mkdir(parents=True, exist_ok=True)
    shutil.copy(c / p["transcript"], sets / "transcript.json")
    meta = {"id": "m" + p["set_dir"].rsplit("_m", 1)[1], "title": p["title"], "source": "voice_memo",
            "memo": p["memo"], "drive_id": p["drive_id"], "original_name": p["original_name"],
            "recorded_at": p["recorded_at"], "duration_s": p["duration_s"],
            "show_date": p["date"], "show_date_source": "the voice memo's recording date", "show_date_inferred": False,
            "show_venue": None, "show_venue_source": "none: a voice memo names no room",
            "routed_by": {"task": "memo_triage", "kind": "performed_set", **p["triage"]}}
    (sets / "meta.json").write_text(json.dumps(meta, ensure_ascii=False, indent=1), encoding="utf-8")
    return "created"


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--run", action="store_true")
    ap.add_argument("--all-lengths", action="store_true")
    ap.add_argument("--limit", type=int, default=None)
    ap.add_argument("--keep-wav", action="store_true", help="keep the decoded audio after the set pipeline")
    a = ap.parse_args(argv)
    c = comedy()
    ps = plan(c, a.all_lengths)[: a.limit]
    if not a.run:
        print(json.dumps({"would_route": len(ps), "hours": round(sum(p["duration_s"] or 0 for p in ps) / 3600, 2),
                          "sets": [{k: p[k] for k in ("set_dir", "title", "date", "chars")} for p in ps]}, indent=1))
        return 0
    from creative.memolink import _legal
    from gsync.memos import MemoStore
    store = MemoStore(c / "kb" / "gsync" / "memos.json")
    made, status = [], {}
    for p in ps:
        st = make_set(c, p, _legal)
        status[st.split(":")[0]] = status.get(st.split(":")[0], 0) + 1
        if st in ("created", "exists"):
            made.append(p["set_dir"])
            store.set_item(p["memo"], set=p["set_dir"])
    print(json.dumps({"status": status, "sets": len(made)}), flush=True)
    if not made:
        return 0
    py = os.environ.get("PK_LAUGHS_PY", str(Path.home() / ".venvs" / "laughs" / "Scripts" / "python.exe"))
    r = subprocess.run([py, "-m", "tools.audio.run_all", "--sets", *made], cwd=str(DESK),
                       env={**os.environ, "PYTHONUTF8": "1", "COMEDY_DIR": str(c)})
    if r.returncode == 0 and not a.keep_wav:
        purge_wavs(c, made)
    return r.returncode


def purge_wavs(c: Path, sets: list[str]) -> int:
    """No media library: once a set's laughs are measured, its decoded audio goes (the memo's own audio and its
    Drive original are the recording). A set whose pipeline did not finish keeps it, so a rerun can resume."""
    n = 0
    for d in sets:
        wav = c / "tapes" / "raw" / d / "audio16k.wav"
        if wav.is_file() and (c / "corpus" / "sets" / d / "laughs_auto.json").is_file():
            wav.unlink()
            n += 1
    return n


if __name__ == "__main__":
    sys.exit(main())
