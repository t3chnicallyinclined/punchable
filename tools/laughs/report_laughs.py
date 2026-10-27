"""Sanity checks over laughs_auto.json (no ground truth needed).

Prints, per set: laughs/min, bars histogram, kinds, agreement with caption
[Laughter]/[Applause] markers, music regions; and for the chosen spot-listen
sets, the transcript text just before the 5 strongest and 5 weakest events.
Emits JSON to stdout with --json so the report can be written from it.
"""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

CORPUS = Path(r"~/punchable-data\corpus\sets")
MATCH_TOL_S = 2.0


def words_of(sid):
    t = json.loads((CORPUS / sid / "transcript.json").read_text(encoding="utf-8"))
    return sorted((w for s in t["segments"] for w in s.get("words", [])), key=lambda w: w["start"])


def line_before(words, t_s, span_s=7.0):
    ws = [w["w"].strip() for w in words if t_s - span_s <= w["end"] <= t_s + 0.3]
    return " ".join(ws)


def caption_agreement(sid, laughs):
    p = CORPUS / sid / "caption_events.json"
    if not p.exists():
        return None
    evs = json.loads(p.read_text(encoding="utf-8")).get("events", [])
    rows = []
    for e in evs:
        if not re.search(r"laugh|applause|cheer", e["marker"], re.I):
            continue
        a, b = e["start"] - MATCH_TOL_S, e["end"] + MATCH_TOL_S
        hit = [l for l in laughs if l["start_ms"] / 1000 <= b and l["end_ms"] / 1000 >= a]
        rows.append({"marker": e["marker"], "start": e["start"], "end": e["end"],
                     "matched": bool(hit),
                     "match": [{"start": l["start_ms"] / 1000, "bars": l["bars"], "kind": l["kind"]} for l in hit]})
    music = [e for e in evs if "music" in e["marker"].lower()]
    return {"markers": rows, "music_markers": [(e["start"], e["end"]) for e in music]}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--spot", nargs="*", default=["20240719_FakeSet0006", "20251011_FakeSet0007"])
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()
    out = {"sets": {}, "spot": {}}
    for d in sorted(CORPUS.iterdir()):
        f = d / "laughs_auto.json"
        if not f.exists():
            continue
        doc = json.loads(f.read_text(encoding="utf-8"))
        L = doc["laughs"]
        mins = doc["audio_sec"] / 60
        bars = [sum(1 for l in L if l["bars"] == k) for k in range(5)]
        kinds = {k: sum(1 for l in L if l["kind"] == k) for k in ("laugh", "mixed", "applause")}
        cap = caption_agreement(d.name, L)
        out["sets"][d.name] = {
            "minutes": round(mins, 2), "n": len(L), "per_min": round(len(L) / mins, 2),
            "bars": bars[1:], "kinds": kinds, "infer_sec": doc["infer_sec"],
            "laugh_seconds_pct": round(100 * sum((l["end_ms"] - l["start_ms"]) for l in L) / 1000 / doc["audio_sec"], 1),
            "mean_conf": round(sum(l["confidence"] for l in L) / max(1, len(L)), 2),
            "mid_speech_events": sum(1 for l in L if l["detail"]["speech_overlap"] > 0.6),
            "music": doc["excluded_music"], "captions": cap,
        }
        if d.name in args.spot:
            words = words_of(d.name)
            srt = sorted(L, key=lambda l: l["strength"])
            def row(l):
                return {"t": round(l["start_ms"] / 1000, 1), "bars": l["bars"], "kind": l["kind"],
                        "strength": l["strength"], "conf": l["confidence"],
                        "before": line_before(words, l["start_ms"] / 1000)}
            out["spot"][d.name] = {"strongest": [row(l) for l in srt[::-1][:5]],
                                   "weakest": [row(l) for l in srt[:5]]}
    if args.json:
        print(json.dumps(out, indent=1))
        return
    for sid, s in out["sets"].items():
        print(f"{sid}: {s['n']} ev {s['per_min']}/min bars={s['bars']} kinds={s['kinds']} "
              f"laugh%={s['laugh_seconds_pct']} conf={s['mean_conf']} midspeech={s['mid_speech_events']}")
        for m in s["music"]:
            print("   music", m)
        if s["captions"]:
            for r in s["captions"]["markers"]:
                print("   cap", r["marker"], r["start"], "->", "HIT" if r["matched"] else "miss", r["match"])
    for sid, sp in out["spot"].items():
        print("==", sid)
        for k in ("strongest", "weakest"):
            print(" ", k)
            for r in sp[k]:
                print(f"    {r['t']:7.1f}s b{r['bars']} {r['kind']:8s} s={r['strength']:.2f} c={r['conf']:.2f} | {r['before']}")


if __name__ == "__main__":
    main()
