"""AST (Audio Spectrogram Transformer, AudioSet) second-opinion laugh detector.

A second, independent acoustic opinion next to PANNs (detect_laughs.py). It is an
AUTOMATED PROXY, not ground truth: there is no answer key, by design.

Model: onnx-community/ast-finetuned-audioset-10-10-0.4593-ONNX @2fe67046 (a port of
MIT/ast-finetuned-audioset-10-10-0.4593). The variant is chosen by measurement in
tools/laughs/ast_variants.py (see infer/INFER.md, "Audio stack").

Three things the earlier clip tool's src/ast.rs gets wrong, fixed here:
 1. SIGMOID, not softmax. AudioSet is multi-label; the AST recipe trains it with
    BCEWithLogitsLoss and evaluates with torch.sigmoid (YuanGongND/ast
    egs/audioset/run.sh:61 `loss=BCE`; src/traintest.py:57-58, :291). Softmax
    forces the 527 classes to share 1.0, so laughter under speech is crushed.
 2. Class indices come from the model's own config.json id2label. the earlier clip tool's
    hardcoded 13 is "Children shouting" and 36 is "Rapping"; Laughter is 16 and
    Applause is 67 in this model.
 3. Features are the HF ASTFeatureExtractor's: torchaudio Kaldi fbank (hanning,
    25 ms / 10 ms, 128 mel bins, dither 0), padded with zeros to 1024 frames and
    then normalised (x - mean) / (2 * std). the earlier clip tool builds its own HTK mel stack.

Speed: every Kaldi frame depends only on its own 400 samples, so the fbank is
computed ONCE per set and windows are slices of it (a 1 s hop would otherwise
recompute each frame ~10 times). Windows are 998 real frames (10 s, like the
AudioSet training clips) + 26 pad frames, batched through the CUDA provider.

Frame curve: each window gives one clip-level probability for its 10 s. The value
of a hop-sized bin is the MIN over every window that fully covers it: a laugh at
t raises every window containing t, so the min localises the event to within one
hop while one loud moment cannot smear across the whole 10 s.

Writes corpus/sets/<set>/laughs_ast.json. Never touches laughs_auto.json.
Usage: python -m tools.laughs.ast_detect [--sets S ...] [--hop 1.0] [--variant model_fp16.onnx] [--cpu]
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import math
import os
import sys
import time
from pathlib import Path

import numpy as np

MODELS = Path(os.environ.get("PUNCHABLE_MODELS", Path.home() / ".cache" / "punchable-models")) / "ast"
REPO = "onnx-community/ast-finetuned-audioset-10-10-0.4593-ONNX"
REVISION = "2fe67046c486b8b8bc90a7b469239b2dec822f72"
VARIANT = "model_fp16.onnx"   # chosen by ast_variants.py; see INFER.md "Audio stack"
CORPUS = Path(os.environ.get("COMEDY_DIR", os.path.expanduser("~/punchable-data"))) / "corpus" / "sets"

SR = 16000
MEAN, STD = -4.2677393, 4.5689974     # preprocessor_config.json (AudioSet stats)
TARGET_FRAMES = 1024
REAL_FRAMES = 998                     # a 10 s AudioSet clip at 25 ms / 10 ms
PAD_VALUE = (0.0 - MEAN) / (2 * STD)  # HF pads with 0 BEFORE normalising

# Same class groups as PANNs (detect_laughs.py) so the two opinions are comparable.
CLASSES = {
    "laugh": ["Laughter", "Giggle", "Snicker", "Chuckle, chortle", "Belly laugh"],
    "applause": ["Applause", "Cheering"],
    "crowd": ["Crowd"],
    "music": ["Music"],
    "speech": ["Speech"],
}
# Fixed a priori, on the same probability scale as PANNs (sigmoid), NOT tuned
# against PANNs (that would make the agreement report circular).
T_ON, T_OFF = 0.20, 0.10
APPLAUSE_MIN_DUR_S = 1.5
WALK_ON_S, WALK_OFF_S = 15.0, 4.0     # same framing rule as detect_laughs.py


# ---------------- gates ----------------

def sigmoid(x: np.ndarray) -> np.ndarray:
    x = np.asarray(x, dtype=np.float64)
    return (0.5 * (1.0 + np.tanh(0.5 * x))).astype(np.float32)


def softmax(x: np.ndarray) -> np.ndarray:  # only for the tests (the the earlier clip tool bug)
    x = np.asarray(x, dtype=np.float64)
    e = np.exp(x - x.max(axis=-1, keepdims=True))
    return (e / e.sum(axis=-1, keepdims=True)).astype(np.float32)


LOGITS_TO_PROBS = sigmoid


def assert_multilabel_probs(logits: np.ndarray, probs: np.ndarray, atol: float = 1e-4) -> None:
    """Gate: probabilities must be the element-wise sigmoid of the logits.
    A softmax (the earlier clip tool's bug) fails this on any input with more than one class."""
    ref = sigmoid(logits)
    if probs.shape != ref.shape or not np.allclose(probs, ref, atol=atol):
        rows = np.asarray(probs, dtype=np.float64).sum(axis=-1)
        raise AssertionError(f"AST probabilities are not sigmoid(logits) (row sums {rows.min():.3f}..{rows.max():.3f}); "
                             "AudioSet AST is multi-label (BCE), never softmax")


def load_config(models: Path = MODELS) -> dict:
    return json.loads((models / "config.json").read_text(encoding="utf-8"))


def label_indices(config: dict, groups: dict = CLASSES) -> dict:
    """{group: [(name, idx), ...]} looked up by exact name in config id2label.
    Gate: every name must exist exactly once, and there must be 527 labels."""
    id2 = {int(k): v for k, v in config["id2label"].items()}
    if sorted(id2) != list(range(527)):
        raise ValueError(f"expected 527 contiguous AudioSet labels, got {len(id2)}")
    by_name = {}
    for i, n in id2.items():
        by_name.setdefault(n, []).append(i)
    out = {}
    for g, names in groups.items():
        out[g] = []
        for n in names:
            hits = by_name.get(n, [])
            if len(hits) != 1:
                raise KeyError(f"label {n!r} found {len(hits)} times in config.json")
            out[g].append((n, hits[0]))
    return out


# ---------------- features ----------------

def fbank(pcm: np.ndarray, device: str = "cpu") -> np.ndarray:
    """Normalised Kaldi fbank of the whole set, (T, 128) float32, frame i = samples [160 i, 160 i + 400)."""
    import torch
    import torchaudio.compliance.kaldi as K
    w = torch.from_numpy(np.ascontiguousarray(pcm, dtype=np.float32))[None, :].to(device)
    fb = K.fbank(w, sample_frequency=SR, window_type="hanning", num_mel_bins=128)  # HF defaults (dither 0)
    return ((fb - MEAN) / (2 * STD)).cpu().numpy().astype(np.float32)


def window_starts(n_frames: int, hop_frames: int) -> list[int]:
    return list(range(0, max(1, n_frames), hop_frames))


def make_batch(fb: np.ndarray, starts: list[int], dtype=np.float32) -> np.ndarray:
    X = np.full((len(starts), TARGET_FRAMES, 128), PAD_VALUE, dtype=np.float32)
    for k, s in enumerate(starts):
        seg = fb[s:s + REAL_FRAMES]
        X[k, :len(seg)] = seg
    return X.astype(dtype, copy=False)


# ---------------- model ----------------

class AST:
    def __init__(self, variant: str = VARIANT, cpu: bool = False, models: Path = MODELS):
        import torch  # noqa: F401  (loads the CUDA/cuDNN DLLs onnxruntime-gpu reuses)
        import onnxruntime as ort
        so = ort.SessionOptions()
        so.log_severity_level = 3
        so.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
        prov = ["CPUExecutionProvider"] if cpu else [("CUDAExecutionProvider", {"cudnn_conv_algo_search": "HEURISTIC"}), "CPUExecutionProvider"]
        self.path = models / "onnx" / variant if (models / "onnx" / variant).exists() else models / variant
        self.sess = ort.InferenceSession(str(self.path), sess_options=so, providers=prov)
        self.provider = self.sess.get_providers()[0]
        i = self.sess.get_inputs()[0]
        self.input_name = i.name
        self.in_dtype = np.float16 if "float16" in i.type else np.float32
        self.variant = variant
        self.config = load_config(models)
        self.labels = label_indices(self.config)

    def logits(self, X: np.ndarray) -> np.ndarray:
        out = self.sess.run(None, {self.input_name: X.astype(self.in_dtype, copy=False)})[0]
        return np.asarray(out, dtype=np.float32)

    def probs_for_set(self, pcm: np.ndarray, hop_s: float = 1.0, batch: int = 16, fb: np.ndarray | None = None):
        """(window starts in s, sigmoid probs (n_windows, 527), timings)."""
        t0 = time.perf_counter()
        if fb is None:
            fb = fbank(pcm)
        t1 = time.perf_counter()
        hop = max(1, int(round(hop_s * 100)))
        starts = window_starts(fb.shape[0], hop)
        P = np.empty((len(starts), 527), dtype=np.float32)
        for b in range(0, len(starts), batch):
            st = starts[b:b + batch]
            lg = self.logits(make_batch(fb, st))
            pr = LOGITS_TO_PROBS(lg)
            assert_multilabel_probs(lg, pr)
            P[b:b + len(st)] = pr
        t2 = time.perf_counter()
        return np.array(starts) / 100.0, P, {"fbank_s": round(t1 - t0, 3), "infer_s": round(t2 - t1, 3), "windows": len(starts)}


# ---------------- curve and events ----------------

def group_probs(P: np.ndarray, labels: dict) -> dict:
    return {g: P[:, [i for _, i in idx]].max(axis=1) for g, idx in labels.items()}


def frame_curve(win_p: np.ndarray, starts_s: np.ndarray, hop_s: float, audio_s: float, how: str = "min") -> np.ndarray:
    """Per-bin value from window probabilities: min (default) or mean over the windows
    that fully cover the bin; bins covered by no window fall back to every window that touches them."""
    n_bins = int(math.ceil(audio_s / hop_s))
    win_len = REAL_FRAMES / 100.0
    out = np.zeros(n_bins, dtype=np.float32)
    for k in range(n_bins):
        a, b = k * hop_s, min(audio_s, (k + 1) * hop_s)
        full = (starts_s <= a + 1e-9) & (starts_s + win_len >= b - 1e-9)
        sel = full if full.any() else (starts_s < b) & (starts_s + win_len > a)
        v = win_p[sel]
        out[k] = (v.min() if how == "min" else v.mean()) if len(v) else 0.0
    return out


def events_from_curve(laugh: np.ndarray, appl: np.ndarray, hop_s: float, last_word_end_s: float | None, audio_s: float):
    aud = np.maximum(laugh, appl)
    on, keep = aud > T_ON, aud > T_OFF
    ev, i, n = [], 0, len(aud)
    while i < n:
        if on[i]:
            a = i
            while a > 0 and keep[a - 1]:
                a -= 1
            b = i
            while b < n and keep[b]:
                b += 1
            ev.append((a, b))
            i = b
        else:
            i += 1
    out = []
    for a, b in ev:
        s_s, e_s = a * hop_s, min(audio_s, b * hop_s)
        lm, am = float(laugh[a:b].mean()), float(appl[a:b].mean())
        dur = e_s - s_s
        if am >= lm and dur >= APPLAUSE_MIN_DUR_S:
            kind = "applause" if lm < 0.5 * am else "mixed"
        elif appl[a:b].max() > T_ON and lm > T_OFF:
            kind = "mixed"
        else:
            kind = "laugh"
        if s_s < WALK_ON_S:
            frame = "walk_on"
        elif last_word_end_s is not None and (s_s >= last_word_end_s - WALK_OFF_S or e_s > audio_s - 3.0):
            frame = "walk_off"
        else:
            frame = "set"
        out.append({"id": f"a{int(round(s_s * 1000))}", "start_ms": int(round(s_s * 1000)), "end_ms": int(round(e_s * 1000)),
                    "kind": kind, "peak": round(float(aud[a:b].max()), 3), "laugh_p": round(lm, 3),
                    "applause_p": round(am, 3), "segment": frame})
    return out


def last_word_end(set_dir: Path):
    tp = set_dir / "transcript.json"
    if not tp.exists():
        return None
    t = json.loads(tp.read_text(encoding="utf-8"))
    ends = [w["end"] for s in t["segments"] for w in s.get("words", []) if w.get("end") is not None]
    return max(ends) if ends else None


def detect(set_id: str, pcm: np.ndarray, model: AST, hop_s: float = 1.0, batch: int = 16, write: bool = True) -> dict:
    audio_s = len(pcm) / SR
    starts, P, tm = model.probs_for_set(pcm, hop_s=hop_s, batch=batch)
    g = group_probs(P, model.labels)
    curves = {k: frame_curve(v, starts, hop_s, audio_s, "min") for k, v in g.items()}
    lwe = last_word_end(CORPUS / set_id)
    evs = events_from_curve(curves["laugh"], curves["applause"], hop_s, lwe, audio_s)
    doc = {
        "detector": f"ast@{REPO}@{REVISION[:8]}:{model.variant}+sigmoid+min_curve@1",
        "generated_at": dt.datetime.now().astimezone().isoformat(timespec="seconds"),
        "unvalidated": True,
        "note": "Second opinion. An automated proxy, not ground truth. laughs_auto.json (PANNs) stays canonical for metrics.",
        "provider": model.provider,
        "labels": {k: [[n, i] for n, i in v] for k, v in model.labels.items()},
        "params": {"hop_s": hop_s, "window_s": REAL_FRAMES / 100, "batch": batch, "T_ON": T_ON, "T_OFF": T_OFF,
                   "curve": "min over fully covering windows", "APPLAUSE_MIN_DUR_S": APPLAUSE_MIN_DUR_S},
        "audio_sec": round(audio_s, 1), "timing": tm,
        "curve_hop_s": hop_s,
        "curves": {k: [round(float(x), 3) for x in v] for k, v in curves.items()},
        "laughs": evs,
    }
    if write:
        (CORPUS / set_id / "laughs_ast.json").write_text(json.dumps(doc, indent=1), encoding="utf-8")
    return doc


def main(argv=None):
    ap = argparse.ArgumentParser(prog="python -m tools.laughs.ast_detect")
    ap.add_argument("--sets", nargs="*")
    ap.add_argument("--hop", type=float, default=1.0)
    ap.add_argument("--batch", type=int, default=16)
    ap.add_argument("--variant", default=VARIANT)
    ap.add_argument("--cpu", action="store_true")
    a = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
    from tools.audio import pcm as PCM
    m = AST(a.variant, cpu=a.cpu)
    sets = a.sets or sorted(p.name for p in CORPUS.iterdir() if (p / "transcript.json").exists())
    for s in sets:
        x, _ = PCM.load(s)
        t = time.perf_counter()
        d = detect(s, np.asarray(x), m, a.hop, a.batch)
        print(f"{s}: {len(d['laughs'])} events, {d['timing']}, total {time.perf_counter() - t:.2f}s on {m.provider}")


if __name__ == "__main__":
    main()
