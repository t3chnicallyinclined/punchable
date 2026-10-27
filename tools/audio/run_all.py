"""Process sets end to end with per-stage timing and a content-hash cache.

  decode (once, memory-mapped float32, tools/audio/pcm.py)
    -> VAD (Silero v5, CPU thread)  ||  PANNs laughs -> AST second opinion (GPU)
    -> transcribe if missing (faster-whisper, run in the global Python that has it, with --gap-fill)
    -> RMS curve
  then, once for all sets: metrics (python -m infer.sets) and clips (python -m infer.clips).

Every stage has a key = sha256 of what it reads (WAV bytes, transcript bytes, its own
parameters/version). A stage whose key matches tools/audio/cache/<set>/manifest.json and whose
output exists is skipped. Change the transcript and PANNs/AST re-run; touch nothing and
nothing re-runs.

Run in the laughs venv (torch CUDA + onnxruntime-gpu):
  ~\\.venvs\\laughs\\Scripts\\python.exe -m tools.audio.run_all --sets 20161101_FakeSet0017
  ... -m tools.audio.run_all --all [--force STAGE ...] [--no-infer] [--report]
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import datetime as dt
import json
import os
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]  # punchable
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "tools" / "laughs"))

from tools.audio import pcm as PCM  # noqa: E402
from tools.audio import rms as RMS  # noqa: E402
from tools.audio import vad as VAD  # noqa: E402

COMEDY = Path(os.environ.get("COMEDY_DIR", os.path.expanduser("~/punchable-data")))
SETS = COMEDY / "corpus" / "sets"
RAW = COMEDY / "tapes" / "raw"
CACHE = ROOT / "tools" / "audio" / "cache"
GLOBAL_PY = os.environ.get("WHISPER_PY", r"C:\Python313\python.exe")

# Bump a version when a stage's code changes what it writes.
VERSIONS = {"vad": "silero_v5@1", "panns": "panns_cnn14_dlm+fusion@1:batch4", "ast": "ast_fp16@1:hop1.0", "rms": "rms@1"}
PANNS_BATCH = 4


def sha_file(p: Path) -> str:
    return PCM.file_sha256(p) if p.exists() else "missing"


class Manifest:
    def __init__(self, set_id: str):
        self.dir = CACHE / set_id
        self.dir.mkdir(parents=True, exist_ok=True)
        self.path = self.dir / "manifest.json"
        self.data = json.loads(self.path.read_text(encoding="utf-8")) if self.path.exists() else {}

    def fresh(self, stage: str, key: str, outputs: list[Path]) -> bool:
        e = self.data.get(stage)
        return bool(e) and e.get("key") == key and all(p.exists() for p in outputs)

    def record(self, stage: str, key: str, seconds: float, audio_s: float, **extra):
        self.data[stage] = {"key": key, "seconds": round(seconds, 4), "audio_s": round(audio_s, 2),
                            "rtf": round(audio_s / seconds, 1) if seconds > 0 else None,
                            "at": dt.datetime.now().astimezone().isoformat(timespec="seconds"), **extra}
        tmp = self.path.with_suffix(".tmp")
        tmp.write_text(json.dumps(self.data, indent=1), encoding="utf-8")
        os.replace(tmp, self.path)


def needs(m: Manifest, stage: str, key: str, outputs: list[Path], force: set) -> bool:
    return stage in force or "all" in force or not m.fresh(stage, key, outputs)


class Models:
    """Loaded once per process (the PANNs checkpoint load alone is ~3-7 s)."""
    def __init__(self, cpu=False):
        self._panns = self._ast = self._sil = None
        self.cpu = cpu
        self.load_s = {}

    @property
    def silero(self):
        if self._sil is None:
            t = time.perf_counter()
            self._sil = VAD.Silero()
            self.load_s["vad"] = time.perf_counter() - t
        return self._sil

    @property
    def panns(self):
        if self._panns is None:
            import torch
            import detect_laughs as D
            t = time.perf_counter()
            dev = "cpu" if self.cpu or not torch.cuda.is_available() else "cuda"
            self._panns = (D, D.build_model(dev), dev, D.load_labels())
            self.load_s["panns"] = time.perf_counter() - t
        return self._panns

    @property
    def ast(self):
        if self._ast is None:
            from tools.laughs import ast_detect as A
            t = time.perf_counter()
            self._ast = (A, A.AST(cpu=self.cpu))
            self.load_s["ast"] = time.perf_counter() - t
        return self._ast


def stage_vad(set_id, m, pcm, wav_sha, models, force, log):
    out = m.dir / "vad.json"
    key = PCM.text_sha256(wav_sha, VERSIONS["vad"])
    if not needs(m, "vad", key, [out], force):
        return json.loads(out.read_text(encoding="utf-8")), None
    t = time.perf_counter()
    v, _ = VAD.run(pcm, models.silero)
    dt_ = time.perf_counter() - t
    out.write_text(json.dumps(v), encoding="utf-8")
    m.record("vad", key, dt_, len(pcm) / PCM.SR, regions=len(v["regions"]), speech_s=v["speech_s"])
    log(f"  vad: {len(v['regions'])} regions, {v['speech_s']} s speech in {dt_:.2f}s")
    return v, dt_


def stage_transcribe(set_id, m, force, log):
    tp = SETS / set_id / "transcript.json"
    if tp.exists() and "transcribe" not in force:
        doc = json.loads(tp.read_text(encoding="utf-8"))
        if doc.get("gap_fill"):
            return None
        args = ["--gap-fill-only"]
    else:
        args = ["--gap-fill"] + (["--force"] if "transcribe" in force else [])
    t = time.perf_counter()
    cmd = [GLOBAL_PY, str(ROOT / "tools" / "ingest" / "transcribe.py"), str(RAW), str(SETS), "--sets", set_id] + args
    r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
    dt_ = time.perf_counter() - t
    if r.returncode != 0:
        raise RuntimeError(f"transcribe failed for {set_id}:\n{r.stderr[-2000:]}")
    doc = json.loads(tp.read_text(encoding="utf-8"))
    m.record("transcribe", PCM.text_sha256(sha_file(tp)), dt_, doc["audio_sec"],
             whisper_compute_s=doc.get("compute_sec"), mode=" ".join(args))
    log(f"  transcribe ({' '.join(args)}): {dt_:.1f}s wall")
    return dt_


def stage_panns(set_id, m, pcm, wav_sha, tr_sha, models, force, log, speech_mask=None):
    out = SETS / set_id / "laughs_auto.json"
    key = PCM.text_sha256(wav_sha, tr_sha, VERSIONS["panns"], "mask=" + ("vad" if speech_mask else "words"))
    if not needs(m, "panns", key, [out], force):
        return None
    D, model, dev, labels = models.panns
    import torch
    run_id = "auto-" + dt.datetime.now().strftime("%Y%m%d-%H%M") + "-laughs"
    try:
        from importlib.metadata import version
        pv = version("panns_inference")
    except Exception:
        pv = "?"
    detector = f"panns_inference@{pv}:{D.CKPT_NAME}+transcript_fusion@1"
    if dev == "cuda":
        torch.cuda.synchronize()
    t = time.perf_counter()
    d = D.detect(set_id, model, dev, labels, run_id, detector, pcm=pcm, batch=PANNS_BATCH, speech_mask=speech_mask)
    if dev == "cuda":
        torch.cuda.synchronize()
    dt_ = time.perf_counter() - t
    m.record("panns", key, dt_, d["audio_sec"], events=len(d["laughs"]), parts=d["_timing"])
    log(f"  panns: {len(d['laughs'])} events in {dt_:.2f}s {d['_timing']}")
    return dt_


def stage_ast(set_id, m, pcm, wav_sha, tr_sha, models, force, log):
    out = SETS / set_id / "laughs_ast.json"
    key = PCM.text_sha256(wav_sha, tr_sha, VERSIONS["ast"])
    if not needs(m, "ast", key, [out], force):
        return None
    A, model = models.ast
    t = time.perf_counter()
    d = A.detect(set_id, pcm, model, hop_s=1.0, batch=16)
    dt_ = time.perf_counter() - t
    m.record("ast", key, dt_, d["audio_sec"], events=len(d["laughs"]), parts=d["timing"], provider=model.provider)
    log(f"  ast: {len(d['laughs'])} events in {dt_:.2f}s {d['timing']}")
    return dt_


def stage_rms(set_id, m, pcm, wav_sha, force, log):
    out = m.dir / "rms.npz"
    key = PCM.text_sha256(wav_sha, VERSIONS["rms"])
    if not needs(m, "rms", key, [out], force):
        return None
    t = time.perf_counter()
    c = RMS.curve(pcm)
    dt_ = time.perf_counter() - t
    np.savez_compressed(out, db_10ms=c["db_10ms"].astype(np.float16), db_1s=c["db_1s"])
    m.record("rms", key, dt_, len(pcm) / PCM.SR)
    log(f"  rms: {dt_:.3f}s")
    return dt_


def process(set_id, models, force, log, speech_mask_mode="words"):
    m = Manifest(set_id)
    wav = PCM.wav_path(set_id)
    t = time.perf_counter()
    wav_sha = PCM.file_sha256(wav)
    pcm, _ = PCM.load(set_id, sha=wav_sha)
    pcm = np.asarray(pcm)
    dec = time.perf_counter() - t
    m.record("decode", wav_sha, dec, len(pcm) / PCM.SR)
    log(f"{set_id}: {len(pcm) / PCM.SR:.0f}s audio, decode+hash {dec:.3f}s")
    with cf.ThreadPoolExecutor(max_workers=1) as ex:
        fut_vad = ex.submit(stage_vad, set_id, m, pcm, wav_sha, models, force, log)  # CPU, overlaps the GPU work
        if not (SETS / set_id / "transcript.json").exists():
            fut_vad.result()  # transcription needs the GPU to itself
        stage_transcribe(set_id, m, force, log)
        tr_sha = sha_file(SETS / set_id / "transcript.json")
        mask = None
        if speech_mask_mode == "vad":
            v, _ = fut_vad.result()
            tr = json.loads((SETS / set_id / "transcript.json").read_text(encoding="utf-8"))
            mask = VAD.speaking_mask(v["regions"], VAD.word_spans(tr))
        stage_panns(set_id, m, pcm, wav_sha, tr_sha, models, force, log, speech_mask=mask)
        stage_ast(set_id, m, pcm, wav_sha, tr_sha, models, force, log)
        stage_rms(set_id, m, pcm, wav_sha, force, log)
        fut_vad.result()
    return m


def infer_all(sets, log):
    env = dict(os.environ, PYTHONUTF8="1")
    out = {}
    for name, cmd in (("metrics", [GLOBAL_PY, "-m", "infer.sets", "--all"]),
                      ("clips", [GLOBAL_PY, "-m", "infer.clips", "--all"])):
        t = time.perf_counter()
        r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace", env=env)
        out[name] = time.perf_counter() - t
        log(f"{name}: {out[name]:.1f}s (exit {r.returncode})")
        tail = (r.stdout or "")[-1500:]
        if tail:
            log(tail)
        if r.returncode != 0:
            raise RuntimeError(f"{name} failed:\n{r.stderr[-3000:]}")
    return out


def report(sets):
    rows = {}
    for s in sets:
        p = CACHE / s / "manifest.json"
        if not p.exists():
            continue
        for st, e in json.loads(p.read_text(encoding="utf-8")).items():
            r = rows.setdefault(st, [0.0, 0.0, 0])
            r[0] += e.get("seconds") or 0
            r[1] += e.get("audio_s") or 0
            r[2] += 1
    print(f"{'stage':12} {'sets':>4} {'audio_s':>9} {'compute_s':>10} {'RTF':>8}")
    for st, (sec, aud, n) in rows.items():
        print(f"{st:12} {n:>4} {aud:>9.0f} {sec:>10.2f} {aud / sec if sec else float('nan'):>8.0f}")
    return rows


def main(argv=None):
    ap = argparse.ArgumentParser(prog="python -m tools.audio.run_all")
    ap.add_argument("--sets", nargs="*")
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--force", nargs="*", default=[], help="stages to re-run regardless of cache (or 'all')")
    ap.add_argument("--no-infer", action="store_true")
    ap.add_argument("--report", action="store_true")
    ap.add_argument("--cpu", action="store_true")
    ap.add_argument("--speech-mask", choices=["words", "vad"], default="words")
    a = ap.parse_args(argv)
    sets = sorted(p.name for p in RAW.iterdir() if (p / "audio16k.wav").exists()) if a.all else (a.sets or [])
    if a.report:
        report(sets)
        return 0
    if not sets:
        ap.error("give --sets or --all")
    log = lambda s: print(s, flush=True)  # noqa: E731
    models = Models(cpu=a.cpu)
    t0 = time.perf_counter()
    for s in sets:
        process(s, models, set(a.force), log, a.speech_mask)
        PCM.evict(keep={PCM.file_sha256(PCM.wav_path(s))})
    log(f"model load: { {k: round(v, 2) for k, v in models.load_s.items()} }")
    if not a.no_infer:
        infer_all(sets, log)
    log(f"total {time.perf_counter() - t0:.1f}s")
    report(sets)
    return 0


if __name__ == "__main__":
    sys.exit(main())
