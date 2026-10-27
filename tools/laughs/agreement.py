"""AST vs PANNs agreement, per set and overall. AN AUTOMATED PROXY, NOT GROUND TRUTH.

Neither detector is an answer key; this measures how often two independent acoustic
models agree, and where they disagree most, so a human could look there first.

Per set, over in-set events only (both detectors mark walk-on / walk-off the same way):
  * event precision / recall of each against the other, an event matching when the two
    overlap after widening each by a +-0.5 s collar;
  * frame agreement on 100 ms bins over the material (IoU and Cohen's kappa);
  * per-joke laugh seconds from each detector over the same joke windows (from
    analysis.json), pooled across sets: Pearson r and Spearman rho;
  * the biggest disagreements: PANNs-only events (by duration x bars), AST-only events
    (by duration x peak), and jokes with the largest |laugh_s difference|, with timestamps
    and the words around them.
Writes tools/audio/cache/agreement.json (gitignored: it quotes the private transcripts).

Usage: python -m tools.laughs.agreement [--sets ...]
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path

import numpy as np

COMEDY = Path(os.environ.get("COMEDY_DIR", os.path.expanduser("~/punchable-data")))
SETS = COMEDY / "corpus" / "sets"
OUT = Path(__file__).resolve().parents[1] / "audio" / "cache" / "agreement.json"
COLLAR_S = 0.5
BIN_S = 0.1


def load(set_id):
    d = SETS / set_id
    p = json.loads((d / "laughs_auto.json").read_text(encoding="utf-8"))
    a = json.loads((d / "laughs_ast.json").read_text(encoding="utf-8"))
    pe = [e for e in p["laughs"] if e.get("segment", "set") == "set"]
    ae = [e for e in a["laughs"] if e.get("segment", "set") == "set"]
    tr = json.loads((d / "transcript.json").read_text(encoding="utf-8"))
    words = [(w["start"], w["end"], w["w"]) for s in tr["segments"] for w in s.get("words", []) if w.get("end") is not None]
    words.sort()
    an = json.loads((d / "analysis.json").read_text(encoding="utf-8")) if (d / "analysis.json").exists() else None
    return pe, ae, a, words, an


def iv(e):
    return e["start_ms"] / 1000.0, e["end_ms"] / 1000.0


def overlaps(x, y, collar=COLLAR_S):
    return x[0] - collar < y[1] and y[0] - collar < x[1]


def event_pr(ref, hyp, collar=COLLAR_S):
    """Precision of hyp against ref, and recall of ref by hyp."""
    R, H = [iv(e) for e in ref], [iv(e) for e in hyp]
    hit_h = [any(overlaps(h, r, collar) for r in R) for h in H]
    hit_r = [any(overlaps(r, h, collar) for h in H) for r in R]
    return (sum(hit_h) / len(H) if H else None, sum(hit_r) / len(R) if R else None, hit_h, hit_r)


def mask(events, t0, t1, bin_s=BIN_S):
    n = max(0, int(np.ceil((t1 - t0) / bin_s)))
    m = np.zeros(n, bool)
    for e in events:
        a, b = iv(e)
        i, j = int(max(0, (a - t0) / bin_s)), int(min(n, np.ceil((b - t0) / bin_s)))
        if j > i:
            m[i:j] = True
    return m


def kappa(a, b):
    if len(a) == 0:
        return None
    po = float((a == b).mean())
    pa, pb = a.mean(), b.mean()
    pe = pa * pb + (1 - pa) * (1 - pb)
    return None if pe >= 1 else round((po - pe) / (1 - pe), 3)


def union_s(events, a, b):
    ivs = sorted((max(a, x), min(b, y)) for x, y in (iv(e) for e in events) if min(b, y) > max(a, x))
    tot, cur = 0.0, None
    for x, y in ivs:
        if cur and x <= cur[1]:
            cur[1] = max(cur[1], y)
        else:
            if cur:
                tot += cur[1] - cur[0]
            cur = [x, y]
    return tot + (cur[1] - cur[0] if cur else 0.0)


def context(words, t, before=6, after=3):
    i = next((k for k, w in enumerate(words) if w[0] >= t), len(words))
    pre = " ".join(w[2] for w in words[max(0, i - before):i])
    post = " ".join(w[2] for w in words[i:i + after])
    return f"{pre} | {post}".strip()


def mmss(t):
    return f"{int(t) // 60}:{int(t) % 60:02d}"


def spearman(x, y):
    rx, ry = np.argsort(np.argsort(x)), np.argsort(np.argsort(y))
    return float(np.corrcoef(rx, ry)[0, 1])


def analyse(set_id):
    pe, ae, araw, words, an = load(set_id)
    t0 = 15.0
    t1 = (words[-1][1] - 4.0) if words else araw["audio_sec"]
    prec_ast, rec_panns, hit_a, hit_p = event_pr(pe, ae)  # AST judged against PANNs
    prec_panns, rec_ast, _, _ = event_pr(ae, pe)          # PANNs judged against AST
    mp, ma = mask(pe, t0, t1), mask(ae, t0, t1)
    inter, uni = (mp & ma).sum(), (mp | ma).sum()
    curve = araw["curves"]["laugh"]
    hop = araw["curve_hop_s"]

    def ast_max(a, b):
        i, j = int(a / hop), int(np.ceil(b / hop)) + 1
        return round(max(curve[max(0, i - 1):j] or [0.0]), 3)

    p_only = [e for e, h in zip(pe, hit_p) if not h]
    a_only = [e for e, h in zip(ae, hit_a) if not h]
    jokes = []
    if an:
        J = [j for j in an["jokes"] if j["status"] != "not_tristan"]
        for k, j in enumerate(J):
            a = j["start_ms"] / 1000
            b = (J[k + 1]["start_ms"] / 1000) if k + 1 < len(J) else j["end_ms"] / 1000 + 8.0
            jokes.append({"set": set_id, "idx": j["idx"], "title": j["title"], "status": j["status"], "start_s": a,
                          "panns_laugh_s": round(union_s(pe, a, b), 2), "ast_laugh_s": round(union_s(ae, a, b), 2)})
    return {
        "set": set_id, "panns_events": len(pe), "ast_events": len(ae),
        "ast_precision_vs_panns": None if prec_ast is None else round(prec_ast, 3),
        "panns_recall_by_ast": None if rec_panns is None else round(rec_panns, 3),
        "panns_precision_vs_ast": None if prec_panns is None else round(prec_panns, 3),
        "ast_recall_by_panns": None if rec_ast is None else round(rec_ast, 3),
        "frame_iou": round(float(inter / uni), 3) if uni else None, "frame_kappa": kappa(mp, ma),
        "panns_laugh_s": round(float(mp.sum() * BIN_S), 1), "ast_laugh_s": round(float(ma.sum() * BIN_S), 1),
        "panns_only": [{"at": mmss(iv(e)[0]), "start_ms": e["start_ms"], "dur_s": round((e["end_ms"] - e["start_ms"]) / 1000, 2),
                        "bars": e["bars"], "kind": e["kind"], "panns_conf": e.get("confidence"),
                        "ast_laugh_max": ast_max(*iv(e)), "words": context(words, iv(e)[0]),
                        "_w": (e["end_ms"] - e["start_ms"]) / 1000 * e["bars"]} for e in p_only],
        "ast_only": [{"at": mmss(iv(e)[0]), "start_ms": e["start_ms"], "dur_s": round((e["end_ms"] - e["start_ms"]) / 1000, 2),
                      "kind": e["kind"], "peak": e["peak"], "words": context(words, iv(e)[0]),
                      "_w": (e["end_ms"] - e["start_ms"]) / 1000 * e["peak"]} for e in a_only],
        "jokes": jokes,
    }


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--sets", nargs="*")
    a = ap.parse_args(argv)
    sets = a.sets or sorted(p.name for p in SETS.iterdir() if (p / "laughs_ast.json").exists() and (p / "laughs_auto.json").exists())
    per = [analyse(s) for s in sets]
    jk = [j for r in per for j in r["jokes"] if j["status"] in ("known", "new", "crowd_work")]
    x = np.array([j["panns_laugh_s"] for j in jk])
    y = np.array([j["ast_laugh_s"] for j in jk])
    pooled = {
        "sets": len(per), "panns_events": sum(r["panns_events"] for r in per), "ast_events": sum(r["ast_events"] for r in per),
        "jokes": len(jk),
        "joke_laugh_s_pearson": round(float(np.corrcoef(x, y)[0, 1]), 3) if len(jk) > 2 and x.std() and y.std() else None,
        "joke_laugh_s_spearman": round(spearman(x, y), 3) if len(jk) > 2 else None,
    }
    tp_a = sum(round(r["ast_precision_vs_panns"] * r["ast_events"]) for r in per if r["ast_precision_vs_panns"] is not None)
    tp_p = sum(round(r["panns_recall_by_ast"] * r["panns_events"]) for r in per if r["panns_recall_by_ast"] is not None)
    pooled["ast_precision_vs_panns"] = round(tp_a / pooled["ast_events"], 3) if pooled["ast_events"] else None
    pooled["panns_recall_by_ast"] = round(tp_p / pooled["panns_events"], 3) if pooled["panns_events"] else None
    mk = [r["frame_kappa"] for r in per if r["frame_kappa"] is not None]
    pooled["frame_kappa_median"] = round(float(np.median(mk)), 3) if mk else None
    top_p = sorted(((r["set"], e) for r in per for e in r["panns_only"]), key=lambda t: -t[1]["_w"])[:8]
    top_a = sorted(((r["set"], e) for r in per for e in r["ast_only"]), key=lambda t: -t[1]["_w"])[:8]
    top_j = sorted(jk, key=lambda j: -abs(j["panns_laugh_s"] - j["ast_laugh_s"]))[:8]
    for r in per:
        for e in r["panns_only"] + r["ast_only"]:
            e.pop("_w", None)
    for _, e in top_p + top_a:
        e.pop("_w", None)
    doc = {"note": "Automated proxy, not ground truth. Two detectors agreeing is not proof of a laugh; disagreeing is not proof of an error.",
           "collar_s": COLLAR_S, "bin_s": BIN_S, "pooled": pooled,
           "per_set": [{k: v for k, v in r.items() if k not in ("panns_only", "ast_only", "jokes")} for r in per],
           "biggest_disagreements": {"panns_only": [{"set": s, **e} for s, e in top_p],
                                     "ast_only": [{"set": s, **e} for s, e in top_a],
                                     "jokes": top_j},
           "detail": per}
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(doc, indent=1, ensure_ascii=False), encoding="utf-8")
    print(json.dumps(pooled))
    for r in doc["per_set"]:
        print(r)
    print("PANNs-only:")
    for s, e in top_p:
        print(" ", s, e)
    print("AST-only:")
    for s, e in top_a:
        print(" ", s, e)
    print("jokes:")
    for j in top_j:
        print(" ", j)


if __name__ == "__main__":
    main()
