"""Audio stages, reusing the existing tools:

normalize   ffmpeg -> 16 kHz mono PCM wav (same arguments as tools/ingest/fetch_youtube.py,
            so the laugh detector sees the audio it was built on) + a 32 kbps Opus copy.
            Video files (.mov/.mp4/.m4v, H.264 or HEVC): audio only, -vn.
transcribe  tools/ingest/transcribe.py: faster-whisper large-v3 on the GPU, same VAD
            fallback rule; writes transcript.json/.md in the same format
laughs      tools/laughs/detect_laughs.py, run in its own venv through laughs_runner.py
"""
from __future__ import annotations

import json
import subprocess
import time
import wave
from pathlib import Path

from .config import Config
from .kbstore import _load_module

ENGINE_DIR = Path(__file__).resolve().parent
WHISPER_VERSION = "faster-whisper:large-v3:beam5:vad-fallback"


def probe_seconds(path: Path) -> float:
    r = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of",
                        "default=noprint_wrappers=1:nokey=1", str(path)], capture_output=True, text=True)
    try:
        return float(r.stdout.strip())
    except ValueError:
        return 0.0


def normalize(src: Path, dst_wav: Path) -> float:
    dst_wav.parent.mkdir(parents=True, exist_ok=True)
    r = subprocess.run(["ffmpeg", "-nostdin", "-y", "-loglevel", "error", "-i", str(src), "-vn", "-ac", "1",
                        "-ar", "16000", "-c:a", "pcm_s16le", str(dst_wav)], capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg failed: {r.stderr[-400:]}")
    # small Opus copy of the audio, as tapes/raw keeps (a video's picture is never copied)
    opus = dst_wav.parent / "audio.opus"
    if Path(src).resolve() != opus.resolve():
        r = subprocess.run(["ffmpeg", "-nostdin", "-y", "-loglevel", "error", "-i", str(src), "-vn", "-ac", "1",
                            "-c:a", "libopus", "-b:a", "32k", str(opus)], capture_output=True, text=True)
        if r.returncode != 0:
            raise RuntimeError(f"ffmpeg opus failed: {r.stderr[-400:]}")
    with wave.open(str(dst_wav)) as w:
        return w.getnframes() / w.getframerate()


class Transcriber:
    version = WHISPER_VERSION

    def __init__(self, cfg: Config, device="cuda", batch_size: int = 0):
        # batch_size > 0: faster-whisper's BatchedInferencePipeline (Silero VAD chunks decoded in parallel on the GPU,
        # several times faster than the sequential pass). Used for voice memos; sets keep the sequential default.
        self.cfg, self.device, self.batch_size = cfg, device, batch_size
        self._mod = None
        self._model = None
        self._pipe = None

    def _load(self):
        if self._model is None:
            self._mod = _load_module("wd_transcribe", self.cfg.p("tools_dir") / "ingest" / "transcribe.py")
            ct = "float16" if self.device == "cuda" else "int8"
            self._model = self._mod.WhisperModel(self._mod.MODEL, device=self.device, compute_type=ct)

    def __call__(self, wav: Path, out_dir: Path) -> dict:
        self._load()
        m = self._mod
        with wave.open(str(wav)) as w:
            audio_sec = w.getnframes() / w.getframerate()
        t = time.time()
        if self.batch_size > 0:
            if self._pipe is None:
                from faster_whisper import BatchedInferencePipeline
                self._pipe = BatchedInferencePipeline(model=self._model)
            try:
                segs, _ = self._pipe.transcribe(str(wav), language="en", word_timestamps=True, beam_size=5,
                                                batch_size=self.batch_size, condition_on_previous_text=False)
                segments = [m.seg_dict(s) for s in segs]
            except RuntimeError as e:  # the GPU is shared (other apps): out of memory -> this file goes sequential
                if "out of memory" not in str(e):
                    raise
                segments = m.run(self._model, str(wav), vad=True)
        else:
            segments = m.run(self._model, str(wav), vad=True)
        vad_used, fallback = True, None
        gap = m.gap_seconds(segments, audio_sec)
        if gap > 0.10 * audio_sec:
            fallback = f"VAD pass left {gap:.0f}s in gaps >15s; re-ran with VAD off"
            segments = m.run(self._model, str(wav), vad=False)
            vad_used = False
        compute = time.time() - t
        doc = {"model": m.MODEL, "engine": "faster-whisper", "device": self.device,
               "compute_type": "float16" if self.device == "cuda" else "int8", "language": "en",
               "vad_filter": vad_used, "vad_fallback": fallback, "word_timestamps": True, "beam_size": 5,
               "condition_on_previous_text": False, "batch_size": self.batch_size or None,
               "audio_sec": round(audio_sec, 1),
               "compute_sec": round(compute, 1), "rtf": round(audio_sec / max(compute, 1e-6), 1),
               "segments": segments}
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "transcript.json").write_text(json.dumps(doc, indent=1, ensure_ascii=False), encoding="utf-8")
        (out_dir / "transcript.md").write_text(
            f"# {out_dir.name}\n\n" + "\n".join(f"[{m.fmt(s['start'])}] {s['text']}" for s in segments) + "\n",
            encoding="utf-8")
        return doc


class LaughDetector:
    version = "panns_inference:Cnn14_DecisionLevelMax_mAP=0.385+transcript_fusion@1"

    def __init__(self, cfg: Config):
        self.cfg = cfg

    def __call__(self, set_name: str, tapes_dir: Path, corpus_sets_dir: Path, run_id: str) -> dict:
        r = subprocess.run([str(self.cfg.p("laughs_python")), str(ENGINE_DIR / "laughs_runner.py"),
                            "--tools", str(self.cfg.p("tools_dir")), "--set", set_name, "--tapes", str(tapes_dir),
                            "--corpus", str(corpus_sets_dir), "--run-id", run_id],
                           capture_output=True, text=True, encoding="utf-8")
        if r.returncode != 0:
            raise RuntimeError(f"laughs failed: {r.stderr[-800:]}")
        return json.loads((corpus_sets_dir / set_name / "laughs_auto.json").read_text(encoding="utf-8"))
