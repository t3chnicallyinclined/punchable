"""Automatic audience laugh / applause detection for recorded sets.

Pipeline (one set at a time):
  1. PANNs Cnn14_DecisionLevelMax (AudioSet sound-event detection, framewise
     output at 100 frames/s) over the 16 kHz set audio, upsampled to the 32 kHz
     the model was trained on. Run in 30 s windows with 2 s context padding.
  2. Fuse with the faster-whisper word timestamps: frames in the gap just after
     the comic stops talking are boosted, frames inside continuous speech are
     down-weighted, frames dominated by music are suppressed.
  3. Hysteresis thresholding -> events; per-set room baseline = median event
     raw strength; strength = raw / baseline; bars 1-4 (applause has priority).

Writes ONLY corpus/sets/<set>/laughs_auto.json. Never touches transcripts.

Thresholds are fixed constants chosen on general grounds (model output
probabilities, typical laugh durations). They are NOT tuned per set and there
is no ground truth yet.

Usage:  python detect_laughs.py [--sets SET ...] [--cpu]
Needs:  torch (CUDA build preferred), panns_inference, soundfile, numpy.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import math
import os
import re
import sys
import time
from pathlib import Path

import numpy as np
import soundfile as sf

_COMEDY = Path(os.path.expanduser(os.environ.get("COMEDY_DIR", "~/punchable-data")))
CORPUS = _COMEDY / "corpus" / "sets"
TAPES = _COMEDY / "tapes" / "raw"
PANNS_DIR = Path.home() / "panns_data"
CKPT = PANNS_DIR / "Cnn14_DecisionLevelMax.pth"
CKPT_NAME = "Cnn14_DecisionLevelMax_mAP=0.385"

# ---- AudioSet classes (names exactly as in class_labels_indices.csv) ----
# Snicker is added: it is a child of "Laughter" in the AudioSet ontology and is
# among the top-scoring laugh classes on these room recordings.
LAUGH_CLASSES = ["Laughter", "Giggle", "Snicker", "Chuckle, chortle", "Belly laugh"]
APPLAUSE_CLASSES = ["Applause", "Cheering"]
CROWD_CLASSES = ["Crowd"]
MUSIC_CLASSES = ["Music"]
SPEECH_CLASSES = ["Speech"]

# ---- fixed constants (general grounds, not tuned per set) ----
FPS = 100                  # model frame rate (10 ms hop at 32 kHz)
SMOOTH_S = 0.30            # moving-average smoothing of class probs
T_ON = 0.20                # event starts when fused score crosses this
T_OFF = 0.10               # ...and continues while above this (hysteresis)
MERGE_GAP_S = 0.50         # events closer than this merge
MIN_DUR_S = 0.40           # shorter events dropped (single claps, noise)
SPEECH_JOIN_S = 0.35       # words closer than this form one speech run
POST_SPEECH_S = 2.5        # window after a speech run where laughs are expected
W_POST = 1.25              # boost in that window
W_IN_SPEECH = 0.65         # down-weight while the comic is mid-run
MUSIC_SUPPRESS = 0.30      # music prob above this and above audience prob -> suppress
APPLAUSE_MIN_DUR_S = 1.5   # applause shorter than this is scored as a laugh
CROWD_WEIGHT = 0.5         # crowd class contributes at half weight
WALK_ON_S = 15.0           # events starting this early are marked walk_on
WALK_OFF_S = 4.0           # events starting after (last word end - this) are walk_off


def load_labels():
    import csv
    with open(PANNS_DIR / "class_labels_indices.csv", newline="", encoding="utf-8") as f:
        rows = list(csv.reader(f))[1:]
    return [r[2] for r in rows]


def build_model(device):
    import torch
    from panns_inference.models import Cnn14_DecisionLevelMax
    model = Cnn14_DecisionLevelMax(sample_rate=32000, window_size=1024, hop_size=320,
                                   mel_bins=64, fmin=50, fmax=14000, classes_num=527)
    ck = torch.load(str(CKPT), map_location="cpu", weights_only=False)
    model.load_state_dict(ck["model"])
    model.to(device).eval()
    return model


def _forward(model, x, fp16: bool):
    """Cnn14_DecisionLevelMax.forward in eval mode. With fp16 the STFT/log-mel
    front end stays fp32 (|X|^2 of a 1024-sample frame can exceed fp16's 65504)
    and only the CNN body runs under autocast; the sigmoid is taken in fp32."""
    import torch
    import torch.nn.functional as F
    from panns_inference.pytorch_utils import pad_framewise_output
    if not fp16:
        return model(x, None)["framewise_output"]
    m = model
    x = m.logmel_extractor(m.spectrogram_extractor(x))
    frames_num = x.shape[2]
    x = m.bn0(x.transpose(1, 3)).transpose(1, 3)
    with torch.autocast("cuda", dtype=torch.float16):
        for blk, pool in ((m.conv_block1, (2, 2)), (m.conv_block2, (2, 2)), (m.conv_block3, (2, 2)),
                          (m.conv_block4, (2, 2)), (m.conv_block5, (2, 2)), (m.conv_block6, (1, 1))):
            x = blk(x, pool_size=pool, pool_type="avg")
        x = torch.mean(x, dim=3)
        x = F.max_pool1d(x, kernel_size=3, stride=1, padding=1) + F.avg_pool1d(x, kernel_size=3, stride=1, padding=1)
        x = F.relu_(m.fc1(x.transpose(1, 2)))
        logits = m.fc_audioset(x)
    seg = torch.sigmoid(logits.float())
    return pad_framewise_output(m.interpolator(seg), frames_num)


def framewise(model, device, wav32, win_s=30.0, pad_s=2.0, batch: int = 1, fp16: bool = False) -> np.ndarray:
    """(T, 527) framewise probabilities at 100 fps. `wav32` is a 32 kHz numpy array or
    torch tensor (already on `device` is fastest). Chunks are batched only with chunks of
    the SAME length, so no chunk is ever padded and batch=N computes the same windows as
    batch=1."""
    import torch
    sr = 32000
    wt = wav32 if isinstance(wav32, torch.Tensor) else torch.from_numpy(np.ascontiguousarray(wav32))
    wt = wt.float().to(device)
    n = int(wt.shape[0])
    n_frames = int(math.ceil(n / sr * FPS))
    out = np.zeros((n_frames, 527), dtype=np.float32)
    step = int(win_s * sr)
    pad = int(pad_s * sr)
    groups = {}
    for s in range(0, n, step):
        a, b = max(0, s - pad), min(n, s + step + pad)
        groups.setdefault(b - a, []).append((s, a, b))
    for _, jobs in sorted(groups.items()):
        for i in range(0, len(jobs), max(1, batch)):
            bj = jobs[i:i + max(1, batch)]
            with torch.no_grad():
                x = torch.stack([wt[a:b] for _, a, b in bj])
                fw = _forward(model, x, fp16).float().cpu().numpy()  # (B, frames, 527)
            for (s, a, b), f in zip(bj, fw):
                f0 = int(round((s - a) / sr * FPS))
                g0 = int(round(s / sr * FPS))
                g1 = min(n_frames, int(round(min(n, s + step) / sr * FPS)))
                take = f[f0:f0 + (g1 - g0)]
                out[g0:g0 + len(take)] = take
    return out


def smooth(x: np.ndarray, width: int) -> np.ndarray:
    if width <= 1:
        return x
    k = np.ones(width) / width
    return np.convolve(x, k, mode="same")


def load_words(set_dir: Path):
    t = json.loads((set_dir / "transcript.json").read_text(encoding="utf-8"))
    words = [w for s in t["segments"] for w in s.get("words", []) if w.get("end") is not None]
    words.sort(key=lambda w: w["start"])
    return words


def speech_runs(words):
    runs = []
    for w in words:
        if runs and w["start"] - runs[-1][1] <= SPEECH_JOIN_S:
            runs[-1][1] = max(runs[-1][1], w["end"])
        else:
            runs.append([w["start"], w["end"]])
    return runs


def rms_db(wav16: np.ndarray, n_frames: int) -> np.ndarray:
    """10 ms RMS in dBFS; vectorised (tools/audio/rms.py), same values as the old per-frame loop."""
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
    from tools.audio.rms import rms_db as _rms
    return _rms(wav16, 160, n_frames)


def segments_from_mask(mask: np.ndarray, min_len: int = 1):
    out, i, n = [], 0, len(mask)
    while i < n:
        if mask[i]:
            j = i
            while j < n and mask[j]:
                j += 1
            if j - i >= min_len:
                out.append((i, j))
            i = j
        else:
            i += 1
    return out


def detect(set_id: str, model, device, labels, run_id: str, detector: str, *, pcm=None, batch: int = 1,
           fp16: bool = False, speech_mask=None, write: bool = True, gpu_resample: bool = False):
    """speech_mask: optional [(start_s, end_s)] 'comic is speaking' regions (tools/audio/vad.speaking_mask).
    Default (None) = Whisper word runs, the canonical behaviour."""
    timing = {}
    t = time.perf_counter()
    set_dir = CORPUS / set_id
    if pcm is None:
        wav_path = TAPES / set_id / "audio16k.wav"
        wav16, sr = sf.read(str(wav_path), dtype="float32")
        if wav16.ndim > 1:
            wav16 = wav16.mean(axis=1)
        assert sr == 16000, sr
    else:
        wav16 = np.asarray(pcm, dtype=np.float32)
    timing["decode_s"] = time.perf_counter() - t
    import torch
    import torchaudio.functional as AF
    t = time.perf_counter()
    if gpu_resample and device != "cpu":
        wav32 = AF.resample(torch.from_numpy(np.ascontiguousarray(wav16)).to(device), 16000, 32000)
    else:
        wav32 = AF.resample(torch.from_numpy(np.ascontiguousarray(wav16)), 16000, 32000).numpy()
    timing["resample_s"] = time.perf_counter() - t

    t0 = time.time()
    P = framewise(model, device, wav32, batch=batch, fp16=fp16)
    infer_s = time.time() - t0
    timing["infer_s"] = infer_s
    t_post = time.perf_counter()
    T = P.shape[0]
    idx = {name: labels.index(name) for name in
           LAUGH_CLASSES + APPLAUSE_CLASSES + CROWD_CLASSES + MUSIC_CLASSES + SPEECH_CLASSES}
    sw = int(SMOOTH_S * FPS)
    laugh = smooth(P[:, [idx[c] for c in LAUGH_CLASSES]].max(axis=1), sw)
    appl = smooth(P[:, [idx[c] for c in APPLAUSE_CLASSES]].max(axis=1), sw)
    crowd = smooth(P[:, idx["Crowd"]], sw)
    music = smooth(P[:, idx["Music"]], sw)
    speech_p = smooth(P[:, idx["Speech"]], sw)
    audience = np.maximum.reduce([laugh, appl, CROWD_WEIGHT * crowd])

    # ---- transcript fusion ----
    words = load_words(set_dir)
    runs = speech_runs(words)
    mask_runs = runs if speech_mask is None else [[a, b] for a, b in speech_mask]
    tsec = np.arange(T) / FPS
    in_speech = np.zeros(T, bool)
    post = np.zeros(T, bool)
    for a, b in mask_runs:
        ia, ib = int(a * FPS), min(T, int(math.ceil(b * FPS)))
        in_speech[ia:ib] = True
        post[ib:min(T, ib + int(POST_SPEECH_S * FPS))] = True
    post &= ~in_speech
    weight = np.ones(T)
    weight[post] = W_POST
    weight[in_speech] = W_IN_SPEECH
    music_dom = (music > MUSIC_SUPPRESS) & (music > audience)
    fused = audience * weight
    fused[music_dom] = 0.0

    # ---- music regions (intro / outro / beds) for reporting ----
    music_regions = []
    for a, b in segments_from_mask(music > 0.4, int(1.0 * FPS)):
        sp = in_speech[a:b].mean()
        music_regions.append({"start_ms": int(a * 10), "end_ms": int(b * 10),
                              "where": "intro" if a < 30 * FPS else ("outro" if b > T - 30 * FPS else "mid"),
                              "speech_overlap": round(float(sp), 2)})

    # ---- events by hysteresis ----
    on = fused > T_ON
    keep = fused > T_OFF
    events = []
    i = 0
    while i < T:
        if on[i]:
            a = i
            while a > 0 and keep[a - 1]:
                a -= 1
            b = i
            while b < T and keep[b]:
                b += 1
            events.append([a, b])
            i = b
        else:
            i += 1
    merged = []
    for a, b in events:
        if merged and (a - merged[-1][1]) <= MERGE_GAP_S * FPS:
            merged[-1][1] = b
        else:
            merged.append([a, b])
    merged = [e for e in merged if (e[1] - e[0]) >= MIN_DUR_S * FPS]

    # loudness reference: median frame level during the comic's speech
    t_rms = time.perf_counter()
    db = rms_db(wav16, T)
    timing["rms_s"] = time.perf_counter() - t_rms
    speech_db = float(np.median(db[in_speech])) if in_speech.any() else float(np.median(db))

    evs = []
    for a, b in merged:
        dur = (b - a) / FPS
        seg = slice(a, b)
        peak = float(fused[seg].max())
        lmean, amean = float(laugh[seg].mean()), float(appl[seg].mean())
        apeak = float(appl[seg].max())
        sp_frac = float(in_speech[seg].mean())
        post_frac = float(post[seg].mean())
        # level of the loudest 20% of frames vs the comic's speech level
        ev_db = float(np.percentile(db[seg], 80))
        rel_db = ev_db - speech_db
        loud = float(np.clip(10 ** (rel_db / 20), 0.5, 2.0))
        raw = peak * (1.0 + math.log2(1.0 + dur)) * loud
        if amean >= lmean and apeak > T_ON and dur >= APPLAUSE_MIN_DUR_S:
            kind = "applause" if lmean < 0.5 * amean else "mixed"
        elif apeak > T_ON and lmean > T_OFF:
            kind = "mixed"
        else:
            kind = "laugh"
        # confidence: model certainty x context plausibility
        c_model = 1.0 - math.exp(-peak / 0.25)
        c_ctx = 1.0 - 0.45 * sp_frac            # mid-speech => maybe the comic / noise
        c_ctx *= 1.0 if post_frac > 0 or sp_frac < 0.5 else 0.85
        c_music = 1.0 - 0.5 * float(np.clip(music[seg].mean() / 0.5, 0, 1))
        conf = float(np.clip(c_model * c_ctx * c_music, 0.01, 0.99))
        evs.append(dict(a=a, b=b, dur=dur, peak=peak, raw=raw, kind=kind, conf=conf,
                        sp_frac=sp_frac, post_frac=post_frac, rel_db=rel_db,
                        laugh_mean=lmean, appl_mean=amean))

    baseline = float(np.median([e["raw"] for e in evs])) if evs else 1.0
    out_laughs = []
    for e in evs:
        strength = e["raw"] / baseline
        if e["kind"] in ("applause", "mixed") and e["appl_mean"] >= e["laugh_mean"] and e["dur"] >= APPLAUSE_MIN_DUR_S:
            bars = 4
        elif strength < 0.75:
            bars = 1
        elif strength < 1.5:
            bars = 2
        else:
            bars = 3
        # Walk-on / walk-off: crowd noise in the first 15 s, or after the comic's
        # last words ("that's my time"), is usually the intro/outro ovation, not a response to a line.
        # Kept (it is real audience noise) but marked so per-joke scoring skips it.
        s_s, e_s = e["a"] / FPS, e["b"] / FPS
        if s_s < WALK_ON_S:
            frame = "walk_on"
        elif runs and (s_s >= runs[-1][1] - WALK_OFF_S or e_s > T / FPS - 3.0):
            frame = "walk_off"
        else:
            frame = "set"
        out_laughs.append({
            "id": f"l{int(e['a'] * 10)}",
            "start_ms": int(e["a"] * 10),
            "end_ms": int(e["b"] * 10),
            "strength": round(strength, 3),
            "bars": bars,
            "kind": e["kind"],
            "confidence": round(e["conf"], 3),
            "segment": frame,
            "detail": {"peak": round(e["peak"], 3), "dur_s": round(e["dur"], 2),
                       "rel_db": round(e["rel_db"], 1), "speech_overlap": round(e["sp_frac"], 2),
                       "after_speech": round(e["post_frac"], 2),
                       "laugh_p": round(e["laugh_mean"], 3), "applause_p": round(e["appl_mean"], 3)},
        })

    doc = {
        "run_id": run_id,
        "detector": detector,
        "generated_at": dt.datetime.now().astimezone().isoformat(timespec="seconds"),
        "unvalidated": True,
        "room_baseline": {
            "median_raw_strength": round(baseline, 4),
            "n_events": len(out_laughs),
            "speech_level_dbfs": round(speech_db, 1),
            "raw_strength_formula": "peak_fused * (1 + log2(1 + dur_s)) * clip(10^(rel_db/20), 0.5, 2)",
            "bars_rule": "4 = applause (applause>=laugh, >=1.5 s); else strength<0.75 ->1, <1.5 ->2, else 3",
        },
        "params": {k: v for k, v in globals().items() if k.isupper() and isinstance(v, (int, float, list))},
        "excluded_music": music_regions,
        "audio_sec": round(len(wav16) / 16000, 1),
        "infer_sec": round(infer_s, 2),
        "laughs": out_laughs,
    }
    if speech_mask is not None:
        doc["speech_mask"] = "silero_vad_v5 regions within 0.3 s of a Whisper word"
    if batch != 1 or fp16:
        doc["inference"] = {"batch": batch, "fp16_body": fp16}
    timing["post_s"] = time.perf_counter() - t_post - timing["rms_s"]
    doc["_timing"] = {k: round(v, 4) for k, v in timing.items()}
    if write:
        out = {k: v for k, v in doc.items() if k != "_timing"}
        (set_dir / "laughs_auto.json").write_text(json.dumps(out, indent=1), encoding="utf-8")
    return doc


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sets", nargs="*")
    ap.add_argument("--cpu", action="store_true")
    ap.add_argument("--batch", type=int, default=4, help="same-length 34 s chunks per forward pass")
    ap.add_argument("--fp16", action="store_true", help="fp16 CNN body (front end stays fp32); off by default, see INFER.md")
    args = ap.parse_args()
    import torch
    import panns_inference
    device = "cuda" if torch.cuda.is_available() and not args.cpu else "cpu"
    try:
        from importlib.metadata import version
        pv = version("panns_inference")
    except Exception:
        pv = "?"
    detector = f"panns_inference@{pv}:{CKPT_NAME}+transcript_fusion@1"
    run_id = "auto-" + dt.datetime.now().strftime("%Y%m%d-%H%M") + "-laughs"
    labels = load_labels()
    t0 = time.time()
    model = build_model(device)
    sets = args.sets or sorted(p.name for p in CORPUS.iterdir() if (p / "transcript.json").exists())
    print(f"device={device} torch={torch.__version__} detector={detector} load={time.time()-t0:.1f}s")
    for s in sets:
        t1 = time.time()
        d = detect(s, model, device, labels, run_id, detector, batch=args.batch, fp16=args.fp16)
        bars = [0] * 5
        for l in d["laughs"]:
            bars[l["bars"]] += 1
        print(f"{s}: {len(d['laughs'])} events, {len(d['laughs'])/(d['audio_sec']/60):.2f}/min, "
              f"bars1-4={bars[1:]}, infer={d['infer_sec']}s total={time.time()-t1:.1f}s")
    print(f"all done in {time.time()-t0:.1f}s")


if __name__ == "__main__":
    main()
