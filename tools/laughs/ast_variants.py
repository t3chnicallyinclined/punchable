"""Pick the smallest AST ONNX variant that keeps quality, by measurement.

Reference: the fp32 model.onnx. For each variant, on the same windows of real
set audio (1 s hop), report:
  * max / mean |p - p_fp32| over the laugh + applause classes (sigmoid probs)
  * Pearson r of the per-second laugh curve against fp32's
  * event agreement with fp32's events (same thresholds, +-0.5 s collar)
  * throughput (windows/s) on the CUDA provider, and on CPU for the int/q4 builds
Machine: whatever this runs on; INFER.md records the RTX 3090 PC result.

Usage: python -m tools.laughs.ast_variants SET [SET ...] [--variants a.onnx b.onnx]
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools.audio import pcm as PCM  # noqa: E402
from tools.laughs import ast_detect as A  # noqa: E402

LFS = {  # sha256 prefixes from the HF tree API at REVISION
    "model.onnx": "249b57721b775244", "model_fp16.onnx": "910808cad2642170", "model_int8.onnx": "6739e480a4faf465",
    "model_q4.onnx": "0f50b4abd54d454e", "model_q4f16.onnx": "484d2feb90e636a1", "model_bnb4.onnx": "f8d365922e23f890",
    "model_quantized.onnx": "65116d63d3a34eae", "model_uint8.onnx": "65116d63d3a34eae"}


def sha_ok(p: Path) -> bool:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest().startswith(LFS.get(p.name, "?"))


def events(curve_l, curve_a, hop, audio_s):
    return [(e["start_ms"] / 1000, e["end_ms"] / 1000) for e in A.events_from_curve(curve_l, curve_a, hop, None, audio_s)]


def match(ref, hyp, collar=0.5):
    hit_h = sum(any(a - collar < d and c < b + collar for c, d in ref) for a, b in hyp)
    hit_r = sum(any(a - collar < d and c < b + collar for a, b in hyp) for c, d in ref)
    return hit_h / len(hyp) if hyp else 1.0, hit_r / len(ref) if ref else 1.0


def run_variant(variant, fbs, cpu=False, batch=16):
    m = A.AST(variant, cpu=cpu)
    out, n, t = {}, 0, 0.0
    m.logits(A.make_batch(next(iter(fbs.values()))[0], [0] * batch))  # warm-up (cuDNN / kernels)
    for s, (fb, audio_s) in fbs.items():
        starts = A.window_starts(fb.shape[0], 100)
        P = np.empty((len(starts), 527), np.float32)
        t0 = time.perf_counter()
        for b in range(0, len(starts), batch):
            st = starts[b:b + batch]
            P[b:b + len(st)] = A.sigmoid(m.logits(A.make_batch(fb, st)))
        t += time.perf_counter() - t0
        n += len(starts)
        out[s] = (np.array(starts) / 100.0, P, audio_s)
    return out, n / t, m.provider


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("sets", nargs="+")
    ap.add_argument("--variants", nargs="*", default=["model_fp16.onnx", "model_int8.onnx", "model_q4.onnx", "model_q4f16.onnx"])
    ap.add_argument("--cpu-too", nargs="*", default=["model_int8.onnx"])
    ap.add_argument("--out")
    a = ap.parse_args(argv)
    fbs = {}
    for s in a.sets:
        x, _ = PCM.load(s)
        fbs[s] = (A.fbank(np.asarray(x)), len(x) / A.SR)
    labels = A.label_indices(A.load_config())
    cls = [i for _, i in labels["laugh"] + labels["applause"]]
    rows = []
    ref, ref_wps, ref_prov = run_variant("model.onnx", fbs)
    rows.append({"variant": "model.onnx", "mb": round((A.MODELS / "onnx" / "model.onnx").stat().st_size / 1e6, 1),
                 "sha_ok": sha_ok(A.MODELS / "onnx" / "model.onnx"), "provider": ref_prov, "win_per_s": round(ref_wps, 1)})
    todo = [(v, False) for v in a.variants] + [(v, True) for v in a.cpu_too]
    for v, cpu in todo:
        p = A.MODELS / "onnx" / v
        if not p.exists():
            continue
        try:
            res, wps, prov = run_variant(v, fbs, cpu=cpu)
        except Exception as e:  # an op the provider cannot run, etc.
            rows.append({"variant": v, "cpu": cpu, "error": f"{type(e).__name__}: {str(e)[:160]}"})
            continue
        dmax, dmean, rs, prec, rec = 0.0, [], [], [], []
        for s in fbs:
            st, P0, audio_s = ref[s]
            _, P1, _ = res[s]
            d = np.abs(P1[:, cls] - P0[:, cls])
            dmax, dmean = max(dmax, float(d.max())), dmean + [float(d.mean())]
            g0, g1 = A.group_probs(P0, labels), A.group_probs(P1, labels)
            c0 = {k: A.frame_curve(g0[k], st, 1.0, audio_s) for k in ("laugh", "applause")}
            c1 = {k: A.frame_curve(g1[k], st, 1.0, audio_s) for k in ("laugh", "applause")}
            rs.append(float(np.corrcoef(c0["laugh"], c1["laugh"])[0, 1]))
            pr, rc = match(events(c0["laugh"], c0["applause"], 1.0, audio_s), events(c1["laugh"], c1["applause"], 1.0, audio_s))
            prec.append(pr)
            rec.append(rc)
        rows.append({"variant": v, "cpu": cpu, "mb": round(p.stat().st_size / 1e6, 1), "sha_ok": sha_ok(p), "provider": prov,
                     "win_per_s": round(wps, 1), "max_abs_dp": round(dmax, 4), "mean_abs_dp": round(float(np.mean(dmean)), 5),
                     "laugh_curve_r_min": round(min(rs), 4), "event_precision_vs_fp32": round(float(np.mean(prec)), 3),
                     "event_recall_vs_fp32": round(float(np.mean(rec)), 3)})
    for r in rows:
        print(json.dumps(r))
    if a.out:
        Path(a.out).write_text(json.dumps({"sets": a.sets, "rows": rows}, indent=1), encoding="utf-8")


if __name__ == "__main__":
    main()
