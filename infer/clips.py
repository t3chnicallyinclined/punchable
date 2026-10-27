"""Best clips per set -> comedy/corpus/sets/<set>/clips.json. No rendering here.

the earlier clip tool's prerank (src/prerank.rs), rebuilt on Punchable's own signals:
  * candidates come from the judged segments (never banter, never not_tristan): one candidate
    per laugh a segment gets, starting at the setup and ending after that laugh's tail;
  * every signal is put on a fixed 0..1 scale (not min-max per set, so a clip from one set can
    be ranked against a clip from another), and a signal that is MISSING for a set drops out
    with its weight shared by the ones present (the earlier clip tool's rule: turning a feature off must not
    reshape the ranking);
  * greedy non-maximum suppression keeps the best clip in each neighbourhood, so picks spread
    across the set; then picks come back in set order.

Boundaries snap to the transcript: a clip starts on the first word of a sentence (after a
.?! or a 0.7 s pause) no later than the setup's first word allows, and ends after the laugh
tail, never inside a word. Target 15-60 s. `transcript` is the verbatim span of the words
inside the clip (an exact substring of the set's canonical text, checked before writing).

The LLM pass (the earlier clip tool's ranker, ranker_system.txt, rewritten for stand-up) is optional and
OFF by default: --llm sends the top candidates through engine.providers.complete("clips_rank").

Usage: python -m infer.clips --all | <set> ... [--llm] [--dry-run] [--top N]
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import math
import os
import sys

import numpy as np

from .paths import DESK, INFERRED, SETS, set_dirs
from .transcript import Transcript

AUDIO_CACHE = (os.path.join(os.environ["PK_CACHE_DIR"], "audio") if os.environ.get("PK_CACHE_DIR")
               else os.path.join(DESK, "tools", "audio", "cache"))
MIN_S, MAX_S, TARGET_S = 15.0, 60.0, 35.0
PAD_START_MS = 300            # lead-in before the first word
TAIL_MS = 600                 # after the laugh ends
PAUSE_MS = 700                # a pause this long starts a sentence
ROLL_GAP_MS = 1000            # laughs closer than this are one rolling laugh (metrics.py's rule)
CLEAN_OPEN_MS = 4000          # a mid-bit clip may not start inside, or this soon before, a laugh
MIN_LAUGH_MS = 1500           # when he talks over the laugh, keep at least this much of it
MAX_OVERLAP = 0.30            # NMS: share of the shorter clip two picks may share
CLIP_STATUSES = ("known", "new", "crowd_work")

# Relative importance. Blunt on purpose, like the earlier clip tool's: the point is a sensible shortlist.
WEIGHTS = {
    "laugh_s": 3.0,       # seconds of laughter inside the clip
    "bars_peak": 1.5,     # the biggest laugh's meter bars (1-4)
    "rolling": 1.0,       # longest rolling laugh (bonus for a laugh that keeps going)
    "applause": 1.0,      # applause-break seconds (bars 4)
    "tightness": 1.0,     # setup start -> first laugh (shorter is tighter)
    "energy": 0.75,       # loudest second of the laugh vs the room's speech level (RMS)
    "standing": 1.5,      # a Working joke, or a new line that killed
    "novelty": 0.75,      # how unlike the rest of the set it is (bge-small)
    "ast": 0.75,          # the AST second opinion agrees there is laughter here
}
SCALES = {  # raw -> 0..1, fixed so scores compare across sets
    "laugh_s": "x/(x+6)", "bars_peak": "x/4", "rolling": "x/(x+4)", "applause": "x/(x+2)",
    "tightness": "exp(-ttl_s/20)", "energy": "clip(dB above speech/12)", "standing": "table",
    "novelty": "clip((1-cos)/0.5)", "ast": "clip(max p/0.5)",
}


def norm(name, x):
    if x is None:
        return None
    if name == "laugh_s":
        return x / (x + 6.0)
    if name == "bars_peak":
        return min(1.0, x / 4.0)
    if name == "rolling":
        return x / (x + 4.0)
    if name == "applause":
        return x / (x + 2.0)
    if name == "tightness":
        return math.exp(-x / 20.0)
    if name == "energy":
        return float(np.clip(x / 12.0, 0, 1))
    if name == "novelty":
        return float(np.clip(x / 0.5, 0, 1))
    if name == "ast":
        return float(np.clip(x / 0.5, 0, 1))
    return float(x)  # standing is already 0..1


def combine(parts: dict) -> tuple[float, dict]:
    """Weighted mean over the signals present; absent ones share out their weight."""
    total, wsum, out = 0.0, 0.0, {}
    for k, w in WEIGHTS.items():
        v = parts.get(k)
        if v is None or (isinstance(v, float) and math.isnan(v)):
            out[k] = None
            continue
        total += w * v
        wsum += w
        out[k] = v
    score = total / wsum if wsum > 0 else 0.5
    contrib = {k: (round(WEIGHTS[k] * v / wsum, 4) if v is not None and wsum else None) for k, v in out.items()}
    return score, contrib


def overlap_ratio(a, b):
    inter = max(0, min(a["end_ms"], b["end_ms"]) - max(a["start_ms"], b["start_ms"]))
    shortest = min(a["end_ms"] - a["start_ms"], b["end_ms"] - b["start_ms"])
    return inter / shortest if shortest > 0 else 0.0


def nms(cands, keep, max_overlap=MAX_OVERLAP):
    """Greedy NMS (the earlier clip tool prerank::select without the backfill: a shortlist of near-duplicates
    is worse than a short shortlist). Returns picks in set order."""
    order = sorted(range(len(cands)), key=lambda i: (-cands[i]["score"], cands[i]["start_ms"]))
    kept = []
    for i in order:
        if len(kept) >= keep:
            break
        if all(overlap_ratio(cands[i], cands[k]) <= max_overlap for k in kept):
            kept.append(i)
    return [cands[i] for i in sorted(kept, key=lambda i: cands[i]["start_ms"])]


def check_spacing(clips, max_overlap=MAX_OVERLAP):
    """Gate: no two picks may overlap by more than max_overlap of the shorter one."""
    for i in range(len(clips)):
        for j in range(i + 1, len(clips)):
            r = overlap_ratio(clips[i], clips[j])
            if r > max_overlap + 1e-9:
                raise AssertionError(f"clips {i} and {j} overlap {r:.2f} > {max_overlap}")
    return True


def check_verbatim(clips, text):
    """Gate: every verbatim field must be an exact substring of the set's canonical text."""
    for c in clips:
        for field in ("transcript", "punchline"):
            v = c.get(field)
            if v is not None and (not v or v not in text):
                raise AssertionError(f"clip {c.get('rank')} {field} is not verbatim: {v[:60]!r}")
    return True


# ---------------- snapping ----------------

def sentence_starts(tr):
    out = []
    for i, w in enumerate(tr.words):
        if i == 0:
            out.append(i)
            continue
        p = tr.words[i - 1]
        if p["w"][-1:] in ".?!" or w["s_ms"] - p["e_ms"] >= PAUSE_MS or w["seg"] != p["seg"] and p["w"][-1:] in ".?!,":
            out.append(i)
    return out


def opens_on_laugh(start_ms, evs, guard_ms=CLEAN_OPEN_MS):
    """True if the room is laughing at, or within guard_ms after, the cut: the viewer would hear
    a laugh at a line that is not in the clip."""
    return any(e["end_ms"] > start_ms and e["start_ms"] < start_ms + guard_ms for e in evs)


def snap_start(tr, starts, seg_a, end_ms, evs=()):
    """First word of the clip: the setup's first word (the segment start) if the clip fits in
    MAX_S, else the earliest sentence start inside the segment that does and that does not
    open on someone else's laugh."""
    if end_ms - tr.words[seg_a]["s_ms"] + PAD_START_MS <= MAX_S * 1000:
        return seg_a
    for i in starts:
        if i > seg_a and end_ms - tr.words[i]["s_ms"] + PAD_START_MS <= MAX_S * 1000 \
                and not opens_on_laugh(start_ms_of(tr, i), evs):
            return i
    return None


def start_ms_of(tr, i):
    ms = tr.words[i]["s_ms"] - PAD_START_MS
    if i > 0:
        ms = max(ms, tr.words[i - 1]["e_ms"] + 10)
    return max(0, ms)


def snap_end(tr, laugh_end_ms, floor_ms, laugh_start_ms=None):
    """End after the laugh tail, never inside a word. If he starts his next line while the room
    is still laughing, end just before that line (keeping at least MIN_LAUGH_MS of the laugh),
    so a clip never trails off into half a sentence. Never before `floor_ms` (the punch word's end)."""
    end = laugh_end_ms + TAIL_MS
    nxt = next((w for w in tr.words if w["s_ms"] > floor_ms + 50), None)
    if nxt is not None and nxt["s_ms"] < end:
        keep = (laugh_start_ms if laugh_start_ms is not None else floor_ms) + MIN_LAUGH_MS
        end = max(nxt["s_ms"] - 20, min(end, keep))
    for w in tr.words:
        if w["s_ms"] < end < w["e_ms"]:
            end = max(floor_ms, w["s_ms"] - 20)
            break
    return end


def words_inside(tr, a_ms, b_ms):
    idx = [i for i, w in enumerate(tr.words) if w["s_ms"] >= a_ms and w["e_ms"] <= b_ms]
    return (idx[0], idx[-1] + 1) if idx else (None, None)


# ---------------- signals ----------------

def union_ms(ivs):
    tot, cur = 0, None
    for a, b in sorted(ivs):
        if cur and a <= cur[1]:
            cur[1] = max(cur[1], b)
        else:
            if cur:
                tot += cur[1] - cur[0]
            cur = [a, b]
    return tot + (cur[1] - cur[0] if cur else 0)


def rolling_max_s(evs):
    best, cur = 0.0, None
    for e in sorted(evs, key=lambda e: e["start_ms"]):
        if cur and e["start_ms"] - cur[1] < ROLL_GAP_MS:
            cur[1] = max(cur[1], e["end_ms"])
        else:
            if cur:
                best = max(best, (cur[1] - cur[0]) / 1000)
            cur = [e["start_ms"], e["end_ms"]]
    if cur:
        best = max(best, (cur[1] - cur[0]) / 1000)
    return best


def load_json(p):
    return json.load(open(p, encoding="utf-8")) if os.path.exists(p) else None


def standing_of(j, inferred):
    """1.0 a Working joke or a new line that killed; 0.7 a known joke that killed; 0.5 worked;
    0.25 otherwise."""
    st = inferred.get(j.get("joke_id") or "", {}).get("stage")
    if st == "working":
        return 1.0, f"Working joke ({inferred[j['joke_id']].get('hit_venues')} rooms)"
    if j["status"] == "new" and j["ai_rating"] == "killed":
        return 1.0, "new line that killed"
    if j["ai_rating"] == "killed":
        return 0.7, "killed tonight"
    if j["ai_rating"] == "worked":
        return 0.5, "worked"
    return 0.25, j["ai_rating"]


def candidates(set_dir, an, tr, raw, ast, rms, inferred):
    """One candidate per counted laugh event that follows a punch word inside a clip-worthy segment."""
    evs = [e for e in raw["laughs"] if e.get("segment", "set") == "set"]
    starts = sentence_starts(tr)
    J = [j for j in an["jokes"]]
    out = []
    speech_db = raw.get("room_baseline", {}).get("speech_level_dbfs")
    for k, j in enumerate(J):
        if j["status"] not in CLIP_STATUSES:
            continue
        a, b = words_inside(tr, j["start_ms"], j["end_ms"])
        if a is None:
            continue
        nxt = J[k + 1]["start_ms"] if k + 1 < len(J) else None
        own = [e for e in evs if j["start_ms"] <= e["start_ms"] < (nxt or 10 ** 12)]
        for p in j["punchlines"]:
            ev = next((e for e in own if e["id"] == p["laugh_id"]), None)
            if ev is None:
                continue
            # the laugh keeps going while the next laugh starts within ROLL_GAP_MS
            lend = ev["end_ms"]
            for e in sorted(own, key=lambda e: e["start_ms"]):
                if e["start_ms"] > ev["start_ms"] and e["start_ms"] - lend < ROLL_GAP_MS:
                    lend = max(lend, e["end_ms"])
            end_ms = snap_end(tr, lend, p["at_ms"], ev["start_ms"])
            if nxt is not None:
                end_ms = min(end_ms, max(p["at_ms"] + 200, nxt - 50))
            i0 = snap_start(tr, starts, a, end_ms, evs)
            if i0 is None:
                continue
            start_ms = start_ms_of(tr, i0)
            dur = (end_ms - start_ms) / 1000
            if dur < MIN_S:  # too short: reach back to earlier sentence starts in the segment
                for i in reversed([s for s in starts if a <= s < i0] or [a]):
                    if i != a and opens_on_laugh(start_ms_of(tr, i), evs):
                        continue
                    start_ms = start_ms_of(tr, i)
                    if (end_ms - start_ms) / 1000 >= MIN_S:
                        break
                dur = (end_ms - start_ms) / 1000
            if not (MIN_S <= dur <= MAX_S):
                continue
            wa, wb = words_inside(tr, start_ms, end_ms)
            if wa is None:
                continue
            inside = [e for e in evs if e["end_ms"] > start_ms and e["start_ms"] < end_ms]
            clipped = [(max(start_ms, e["start_ms"]), min(end_ms, e["end_ms"])) for e in inside]
            first = min((e["start_ms"] for e in inside if e["start_ms"] >= start_ms), default=None)
            raw_sig = {
                "laugh_s": union_ms(clipped) / 1000,
                "bars_peak": max((e["bars"] for e in inside), default=0),
                "rolling": rolling_max_s(inside),
                "applause": union_ms([iv for iv, e in zip(clipped, inside) if e["bars"] == 4]) / 1000,
                "tightness": None if first is None else (first - start_ms) / 1000,
            }
            if rms is not None and speech_db is not None:
                s0, s1 = int(ev["start_ms"] // 1000), int(math.ceil(lend / 1000)) + 1
                seg = rms[s0:s1]
                raw_sig["energy"] = float(seg.max() - speech_db) if len(seg) else None
            else:
                raw_sig["energy"] = None
            if ast is not None:
                hop = ast["curve_hop_s"]
                cv = ast["curves"]["laugh"]
                i_a, i_b = int(start_ms / 1000 / hop), int(math.ceil(end_ms / 1000 / hop))
                raw_sig["ast"] = max(cv[i_a:i_b] or [0.0])
            else:
                raw_sig["ast"] = None
            st, st_why = standing_of(j, inferred)
            raw_sig["standing"] = st
            out.append({"set": set_dir, "start_ms": int(start_ms), "end_ms": int(end_ms), "duration_s": round(dur, 2),
                        "joke_id": j["joke_id"], "title": j["title"], "status": j["status"], "ai_rating": j["ai_rating"],
                        "joke_idx": j["idx"], "punchline": p["text"], "punch_at_ms": p["at_ms"], "laugh_id": p["laugh_id"],
                        "transcript": tr.span(wa, wb), "_raw": raw_sig, "_standing_why": st_why})
    return out


def add_novelty(cands, emb):
    if not cands or emb is None:
        return False
    try:
        V = emb.texts([c["transcript"] for c in cands])
    except Exception:
        return False
    mean = V.mean(axis=0)
    mean /= np.linalg.norm(mean) or 1.0
    for c, v in zip(cands, V):
        c["_raw"]["novelty"] = float(1.0 - float(v @ mean))
    return True


def why_line(c):
    r = c["_raw"]
    bits = [f"{r['laugh_s']:.1f} s of laughter"]
    if r["bars_peak"] >= 3:
        bits[-1] += f" with a {r['bars_peak']}-bar laugh" if r["bars_peak"] < 4 else " and an applause break"
    if r["rolling"] >= 3:
        bits.append(f"a rolling laugh of {r['rolling']:.1f} s")
    if r.get("tightness") is not None:
        bits.append(f"first laugh {r['tightness']:.0f} s in")
    bits.append(c["_standing_why"])
    if r.get("ast") is not None and r["ast"] >= 0.25:
        bits.append("AST hears it too")
    return "; ".join(bits) + "."


def score(cands, signals_present):
    for c in cands:
        parts = {k: (norm(k, c["_raw"].get(k)) if k in signals_present else None) for k in WEIGHTS}
        c["score"], contrib = combine(parts)
        c["score"] = round(c["score"], 4)
        c["score_breakdown"] = {k: {"raw": (round(c["_raw"][k], 3) if c["_raw"].get(k) is not None else None),
                                    "norm": (round(parts[k], 3) if parts[k] is not None else None),
                                    "weight": WEIGHTS[k], "contribution": contrib[k]} for k in WEIGHTS}
        c["why"] = why_line(c)


LLM_SCHEMA = {"type": "object", "required": ["clips"], "properties": {"clips": {"type": "array", "items": {
    "type": "object", "required": ["index", "score", "hook", "reason"],
    "properties": {"index": {"type": "integer"}, "score": {"type": "integer"}, "hook": {"type": "string"},
                   "reason": {"type": "string"}}}}}}
LLM_SYSTEM = """You pick short-form clips from a stand-up comic's own recorded set.
Score each candidate 0-100 for a vertical clip (Shorts/Reels/TikTok): HOOK (does the first line
stop the scroll, 0-30), SELF-CONTAINED (a stranger gets the setup and the punch inside the
window, 0-30), PAYOFF (a clear punchline and an audible laugh, 0-25), SHAREABILITY (0-15).
Use ONLY the transcript and numbers given. Never invent lines. The laugh numbers come from a
detector, not a person. Return JSON only."""


def llm_rank(set_dir, picks, run_id):
    from engine.providers import complete  # optional; only with --llm
    rows = [{"index": i, "title": c["title"], "duration_s": c["duration_s"], "laugh_s": c["_raw"]["laugh_s"],
             "bars_peak": c["_raw"]["bars_peak"], "transcript": c["transcript"]} for i, c in enumerate(picks)]
    msgs = [{"role": "system", "content": LLM_SYSTEM},
            {"role": "user", "content": "CANDIDATES:\n" + json.dumps(rows, ensure_ascii=False)}]
    r = complete("clips_rank", msgs, schema=LLM_SCHEMA, run_id=run_id)
    by = {x["index"]: x for x in r.data["clips"] if 0 <= x["index"] < len(picks)}
    for i, c in enumerate(picks):
        if i in by:
            c["llm"] = {"score": max(0, min(100, by[i]["score"])), "hook": by[i]["hook"][:200],
                        "reason": by[i]["reason"][:300], "provider": r.provider, "model": r.model}
    return {"provider": r.provider, "model": r.model, "call_id": r.call_id}


def clips_for(set_dir, inferred, emb, top=None, use_llm=False, run_id=None):
    an = load_json(os.path.join(SETS, set_dir, "analysis.json"))
    raw = load_json(os.path.join(SETS, set_dir, "laughs_auto.json"))
    if an is None or raw is None:
        return None
    tr = Transcript(set_dir)
    ast = load_json(os.path.join(SETS, set_dir, "laughs_ast.json"))
    rp = os.path.join(AUDIO_CACHE, set_dir, "rms.npz")
    rms = np.load(rp)["db_1s"] if os.path.exists(rp) else None
    cands = candidates(set_dir, an, tr, raw, ast, rms, inferred)
    present = {"laugh_s", "bars_peak", "rolling", "applause", "tightness", "standing"}
    if rms is not None:
        present.add("energy")
    if ast is not None:
        present.add("ast")
    if add_novelty(cands, emb):
        present.add("novelty")
    score(cands, present)
    material_min = (an["summary"].get("material_s") or 300) / 60
    keep = top or int(min(6, max(2, round(material_min / 2))))
    picks = nms(cands, keep)
    check_spacing(picks)
    llm = llm_rank(set_dir, picks, run_id) if use_llm and picks else None
    for rank, c in enumerate(sorted(picks, key=lambda c: -c["score"]), 1):
        c["rank"] = rank
    out = []
    for c in picks:
        out.append({"rank": c["rank"], "start_ms": c["start_ms"], "end_ms": c["end_ms"], "duration_s": c["duration_s"],
                    "joke_id": c["joke_id"], "title": c["title"], "status": c["status"], "ai_rating": c["ai_rating"],
                    "score": c["score"], "score_breakdown": c["score_breakdown"], "why": c["why"],
                    "punchline": c["punchline"], "punch_at_ms": c["punch_at_ms"], "laugh_id": c["laugh_id"],
                    "transcript": c["transcript"], **({"llm": c["llm"]} if c.get("llm") else {})})
    check_verbatim(out, tr.text)
    meta = load_json(os.path.join(SETS, set_dir, "meta.json")) or {}
    return {"set": set_dir, "run_id": run_id, "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
            "show_date": meta.get("show_date"), "venue": meta.get("show_venue"),
            "signals_present": sorted(present), "signals_missing": sorted(set(WEIGHTS) - present),
            "params": {"weights": WEIGHTS, "scales": SCALES, "min_s": MIN_S, "max_s": MAX_S, "max_overlap": MAX_OVERLAP,
                       "keep": keep, "candidates": len(cands)},
            "llm": llm, "clips": out}


def main(argv=None):
    ap = argparse.ArgumentParser(prog="python -m infer.clips")
    ap.add_argument("sets", nargs="*")
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--llm", action="store_true", help="optional ranker pass via engine.providers (off by default)")
    ap.add_argument("--no-novelty", action="store_true")
    ap.add_argument("--top", type=int)
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args(argv)
    todo = set_dirs() if a.all else a.sets
    if not todo:
        ap.error("give set dirs or --all")
    if a.llm:
        sys.path.insert(0, DESK)
    inf = load_json(INFERRED) or {"jokes": []}
    inferred = {j["id"]: j for j in inf["jokes"]}
    emb = None
    if not a.no_novelty:
        from .embed import Embedder
        emb = Embedder()
    run_id = "auto-" + dt.datetime.now().strftime("%Y%m%d-%H%M") + "-clips"
    every = []
    for d in todo:
        doc = clips_for(d, inferred, emb, a.top, a.llm, run_id)
        if doc is None:
            print(f"{d}: no analysis/laughs, skipped")
            continue
        every += [dict(c, set=d) for c in doc["clips"]]
        print(f"{d}: {len(doc['clips'])} clips from {doc['params']['candidates']} candidates "
              f"(missing signals: {', '.join(doc['signals_missing']) or 'none'})")
        if not a.dry_run:
            p = os.path.join(SETS, d, "clips.json")
            with open(p + ".tmp", "w", encoding="utf-8", newline="\n") as f:
                json.dump(doc, f, ensure_ascii=False, indent=1)
                f.write("\n")
            os.replace(p + ".tmp", p)
    for c in sorted(every, key=lambda c: -c["score"])[:5]:
        print(f"  {c['score']:.3f} {c['set']} {c['start_ms'] / 1000:.0f}-{c['end_ms'] / 1000:.0f}s {c['title']}: {c['why']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
