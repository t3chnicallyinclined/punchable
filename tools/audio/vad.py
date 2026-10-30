"""Silero VAD v5 (ONNX) speech regions for a set, plus Whisper coverage and gap-fill.

Port of the earlier clip tool src/vad.rs with two fixes, both from the official Silero v5
wrapper (snakers4/silero-vad utils_vad.py, OnnxWrapper + get_speech_timestamps):
  * each 512-sample chunk is fed with the previous 64 samples of context
    (576 samples in). the earlier clip tool feeds bare 512-sample chunks.
  * speech regions use Silero's own hysteresis (threshold / threshold-0.15),
    minimum speech and silence lengths, and padding, not a single cutoff.

Speed: Silero is a stateful RNN, so a set is one strictly sequential stream
(one ORT call per 32 ms; 502 s of audio in 2.3 s on the RTX 3090 PC's CPU).
Batching a set as lock-step sub-streams was measured and REJECTED: the state
carries memory for over 80 s (mean |dp| 0.04 eighty seconds after a restart),
so sub-streams do not reproduce the sequential output. run_all instead runs VAD
on the CPU concurrently with the GPU stages.

Usage: python -m tools.audio.vad <set> [...]   (prints regions and Whisper coverage)
"""
from __future__ import annotations

import json
import math
import os
import sys
from pathlib import Path

import numpy as np

MODEL = Path(os.environ.get("PUNCHABLE_MODELS", Path.home() / ".cache" / "punchable-models")) / "silero_vad_v5.onnx"
MODEL_SHA256 = "2623a2953f6ff3d2c1e61740c6cdb7168133479b267dfef114a4a3cc5bdd788f"  # copied from the earlier clip tool/models
SR = 16000
CHUNK = 512          # 32 ms, Silero v5 at 16 kHz
CONTEXT = 64         # samples of left context per chunk (official wrapper)

# Silero defaults (get_speech_timestamps)
THRESHOLD = 0.5
MIN_SPEECH_MS = 250
MIN_SILENCE_MS = 100
SPEECH_PAD_MS = 30

# Whisper coverage / gap-fill
WORD_TOL_S = 0.25    # a word "covers" VAD speech this far either side of its timing
MIN_GAP_S = 1.0      # uncovered VAD speech shorter than this is ignored
GAP_PAD_S = 0.25     # padding on each gap clip sent back to Whisper
GAP_JOIN_S = 0.5     # gaps closer than this are re-run as one clip


class Silero:
    def __init__(self, model: Path = MODEL, threads: int = 1):
        import onnxruntime as ort
        so = ort.SessionOptions()
        so.intra_op_num_threads = threads
        so.inter_op_num_threads = 1
        so.log_severity_level = 3
        self.sess = ort.InferenceSession(str(model), sess_options=so, providers=["CPUExecutionProvider"])
        self.sr = np.array(SR, dtype=np.int64)

    def probs_sequential(self, pcm: np.ndarray) -> np.ndarray:
        """Reference path: one stream, exactly as the official wrapper."""
        n = int(math.ceil(len(pcm) / CHUNK))
        x = np.zeros(n * CHUNK, dtype=np.float32)
        x[:len(pcm)] = pcm
        state = np.zeros((2, 1, 128), dtype=np.float32)
        ctx = np.zeros((1, CONTEXT), dtype=np.float32)
        out = np.empty(n, dtype=np.float32)
        for i in range(n):
            ch = x[i * CHUNK:(i + 1) * CHUNK][None, :]
            inp = np.concatenate([ctx, ch], axis=1)
            p, state = self.sess.run(None, {"input": inp, "state": state, "sr": self.sr})
            out[i] = p[0, 0]
            ctx = ch[:, -CONTEXT:]
        return out

    def probs(self, pcm: np.ndarray) -> np.ndarray:
        """Per-chunk speech probability (one value per 32 ms), one stream."""
        return self.probs_sequential(pcm)


def speech_timestamps(probs: np.ndarray, n_samples: int, threshold: float = THRESHOLD,
                      min_speech_ms: int = MIN_SPEECH_MS, min_silence_ms: int = MIN_SILENCE_MS,
                      speech_pad_ms: int = SPEECH_PAD_MS) -> list[tuple[float, float]]:
    """Silero get_speech_timestamps (v5 semantics) -> [(start_s, end_s)]."""
    neg = max(threshold - 0.15, 0.01)
    min_speech = SR * min_speech_ms / 1000
    min_sil = SR * min_silence_ms / 1000
    pad = SR * speech_pad_ms / 1000
    triggered, temp_end, cur, speeches = False, 0, {}, []
    for i, p in enumerate(probs):
        pos = CHUNK * i
        if p >= threshold and temp_end:
            temp_end = 0
        if p >= threshold and not triggered:
            triggered = True
            cur = {"start": pos}
            continue
        if p < neg and triggered:
            if not temp_end:
                temp_end = pos
            if pos - temp_end < min_sil:
                continue
            cur["end"] = temp_end
            if cur["end"] - cur["start"] > min_speech:
                speeches.append(cur)
            cur, temp_end, triggered = {}, 0, False
    if cur and n_samples - cur["start"] > min_speech:
        cur["end"] = n_samples
        speeches.append(cur)
    for i, sp in enumerate(speeches):
        if i == 0:
            sp["start"] = int(max(0, sp["start"] - pad))
        if i != len(speeches) - 1:
            gap = speeches[i + 1]["start"] - sp["end"]
            if gap < 2 * pad:
                sp["end"] += int(gap // 2)
                speeches[i + 1]["start"] = int(max(0, speeches[i + 1]["start"] - gap // 2))
            else:
                sp["end"] = int(min(n_samples, sp["end"] + pad))
                speeches[i + 1]["start"] = int(max(0, speeches[i + 1]["start"] - pad))
        else:
            sp["end"] = int(min(n_samples, sp["end"] + pad))
    return [(s["start"] / SR, s["end"] / SR) for s in speeches]


# ---------- interval helpers (seconds) ----------

def union(iv):
    out = []
    for a, b in sorted(iv):
        if b <= a:
            continue
        if out and a <= out[-1][1]:
            out[-1][1] = max(out[-1][1], b)
        else:
            out.append([a, b])
    return [(a, b) for a, b in out]


def intersect(xs, ys):
    xs, ys, out, i, j = union(xs), union(ys), [], 0, 0
    while i < len(xs) and j < len(ys):
        a, b = max(xs[i][0], ys[j][0]), min(xs[i][1], ys[j][1])
        if a < b:
            out.append((a, b))
        if xs[i][1] < ys[j][1]:
            i += 1
        else:
            j += 1
    return out


def subtract(xs, ys):
    out = []
    ys = union(ys)
    for a, b in union(xs):
        cur = a
        for c, d in ys:
            if d <= cur or c >= b:
                continue
            if c > cur:
                out.append((cur, c))
            cur = max(cur, d)
        if cur < b:
            out.append((cur, b))
    return out


def total(iv):
    return sum(b - a for a, b in union(iv))


def check_words(words):
    """Gate: word timings must be seconds, ordered, non-negative and end >= start."""
    prev = -1.0
    for w in words:
        s, e = w
        if not (0 <= s <= e) or s + 1e-6 < prev:
            raise ValueError(f"bad word timing {w} (prev start {prev}); expected sorted seconds")
        if e - s > 30:
            raise ValueError(f"word longer than 30 s: {w} (milliseconds passed as seconds?)")
        prev = s
    return words


def word_spans(transcript: dict) -> list[tuple[float, float]]:
    ws = [(w["start"], w["end"]) for s in transcript["segments"] for w in s.get("words", []) if w.get("end") is not None]
    return check_words(sorted(ws))


def coverage(regions, words, tol: float = WORD_TOL_S) -> dict:
    """Share of VAD speech time that has a Whisper word within `tol` s."""
    covered = union([(max(0.0, s - tol), e + tol) for s, e in words])
    sp = total(regions)
    cov = total(intersect(regions, covered))
    return {"vad_speech_s": round(sp, 2), "covered_s": round(cov, 2),
            "coverage": round(cov / sp, 4) if sp else None}


def gaps(regions, words, min_gap_s: float = MIN_GAP_S, tol: float = WORD_TOL_S,
         pad_s: float = GAP_PAD_S, join_s: float = GAP_JOIN_S, audio_s: float | None = None):
    """VAD speech with no Whisper word nearby, >= min_gap_s long, padded and joined:
    the clips to send back to Whisper."""
    check_words(words)
    covered = union([(max(0.0, s - tol), e + tol) for s, e in words])
    raw = [(a, b) for a, b in subtract(regions, covered) if b - a >= min_gap_s]
    padded = [(max(0.0, a - pad_s), b + pad_s if audio_s is None else min(audio_s, b + pad_s)) for a, b in raw]
    out = []
    for a, b in padded:
        if out and a - out[-1][1] < join_s:
            out[-1] = (out[-1][0], b)
        else:
            out.append((a, b))
    return out


def speaking_mask(regions, words, dilate_s: float = 0.30):
    """'Comic is speaking' for laugh fusion: VAD speech edges, kept only near
    Whisper words (so a laugh that trips the VAD is not called speech, and a
    Whisper word end smeared into the laugh is trimmed back to the VAD edge)."""
    near = union([(max(0.0, s - dilate_s), e + dilate_s) for s, e in words])
    return intersect(regions, near)


def run(pcm: np.ndarray, silero: Silero | None = None) -> dict:
    silero = silero or Silero()
    p = silero.probs(pcm)
    regions = speech_timestamps(p, len(pcm))
    return {"model": "silero_vad_v5.onnx", "model_sha256": MODEL_SHA256, "sr": SR, "chunk": CHUNK,
            "context": CONTEXT,
            "params": {"threshold": THRESHOLD, "min_speech_ms": MIN_SPEECH_MS, "min_silence_ms": MIN_SILENCE_MS,
                       "speech_pad_ms": SPEECH_PAD_MS},
            "audio_s": round(len(pcm) / SR, 3),
            "regions": [[round(a, 3), round(b, 3)] for a, b in regions],
            "speech_s": round(total(regions), 2)}, p


def main(argv=None):
    from . import pcm as P
    sets_dir = Path(os.environ.get("COMEDY_DIR", os.path.expanduser("~/punchable-data"))) / "corpus" / "sets"
    sil = Silero()
    for s in (argv or sys.argv[1:]):
        x, _ = P.load(s)
        v, _ = run(np.asarray(x), sil)
        tp = sets_dir / s / "transcript.json"
        cov = coverage(v["regions"], word_spans(json.loads(tp.read_text(encoding="utf-8")))) if tp.exists() else None
        print(s, len(v["regions"]), "regions", v["speech_s"], "s speech", cov)


if __name__ == "__main__":
    main()
