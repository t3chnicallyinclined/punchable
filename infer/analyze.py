"""Build <set>/analysis.json for one set (contract: docs/KB_SCHEMA.md, "Set analysis")."""
import datetime as dt
import json
import os

from . import laughs as L
from . import metrics as M
from .rate import ORDER, rate
from .scriptdiff import dice, diff
from .segments import load_judgment, resolve
from .similar import hint_labels, similar
from .transcript import Transcript
from .paths import SETS, set_id

JOKE_STATUSES = ("known", "new")
EXCLUDED_STATUS = "not_tristan"  # another comic or the MC: never the comic's joke, never counted


def pick_version(cat, joke_id, tr, a, b):
    """The joke's unit whose words the stage version follows best (Dice on aligned words)."""
    best = (None, -1)
    for uid in cat.joke_units.get(joke_id, []):
        u = cat.units.get(uid)
        if not u:
            continue
        d, cov = dice(u["text"], tr, a, b)
        if d > best[1]:
            best = (uid, d)
    return best[0]


def analyze_set(set_dir, cat, emb, run_id, judgment=None):
    tr = Transcript(set_dir)
    meta = json.load(open(os.path.join(SETS, set_dir, "meta.json"), encoding="utf-8"))
    raw = L.load(set_dir)
    if judgment is None:
        judgment = load_judgment(set_dir)
    if judgment is None:
        from .auto import auto_judgment
        judgment = auto_judgment(set_dir, tr, raw, cat, emb)
    segs = resolve(tr, judgment)
    counted = L.counted(raw, judgment if not judgment.get("auto") else None)
    rm = L.room(tr, counted)

    # Segments judged "not_tristan" (another comic, the MC) are excluded outright: no joke row,
    # no laughs attributed, and their span is cut out of the material window.
    excluded_idx = {i for i, (s, a, b) in enumerate(segs) if s["status"] == EXCLUDED_STATUS and b > a}

    # laughs -> segments by their punch word
    by_seg = {i: [] for i in range(len(segs))}
    for l in counted:
        w = L.punch_word(tr, l)
        if w is None:
            continue
        for i, (_s, a, b) in enumerate(segs):
            if a <= w < b:
                by_seg[i].append((l, w))
                break

    live = [(i, a, b) for i, (_s, a, b) in enumerate(segs) if b > a and i not in excluded_idx]
    window = M.material_window(tr.words[live[0][1]]["s_ms"], tr.words[live[-1][2] - 1]["e_ms"], raw) if live else []
    excl_spans = []
    for i in sorted(excluded_idx):
        _s, a, b = segs[i]
        nxt = next((tr.words[a2]["s_ms"] for j, (_s2, a2, b2) in enumerate(segs) if j > i and b2 > a2), None)
        excl_spans.append((tr.words[a]["s_ms"], nxt if nxt is not None else tr.words[b - 1]["e_ms"]))
    if excl_spans:
        window = M.subtract(window, excl_spans)
    sm = M.set_metrics(counted, window)
    next_start = {i: tr.words[live[k + 1][1]]["s_ms"] if k + 1 < len(live) else (window[-1][1] if window else None)
                  for k, (i, _a, _b) in enumerate(live)}

    jokes = []
    excluded = []
    for i, (s, a, b) in enumerate(segs):
        if b <= a:
            continue
        if i in excluded_idx:
            excluded.append({"start_ms": tr.words[a]["s_ms"], "end_ms": tr.words[b - 1]["e_ms"], "title": s["title"],
                             "status": EXCLUDED_STATUS, "reason": s.get("reason", ""),
                             "laughs_not_counted": len(by_seg[i])})
            continue
        status = s["status"]
        start_ms, end_ms = tr.words[a]["s_ms"], tr.words[b - 1]["e_ms"]
        # punchlines (one per distinct phrase; strongest laugh wins)
        pl, last_w = {}, {}
        for l, w in sorted(by_seg[i], key=lambda x: x[0]["start_ms"]):
            # a laugh whose punch word is the same as (or one word after) the previous
            # laugh's is the same laugh continuing: keep the strongest
            prev = next((t for t, pw in last_w.items() if 0 <= w - pw <= 1), None)
            pa, pb = L.punch_phrase(tr, w)
            pa = max(pa, a, max((pw + 1 for pw in last_w.values() if pw < w), default=a))
            text = tr.span(pa, pb) if prev is None else prev
            cur = pl.get(text)
            if cur is None or l["bars"] > cur["bars"]:
                pl[text] = {"text": text, "at_ms": cur["at_ms"] if cur else tr.words[pb - 1]["e_ms"],
                            "laugh_id": l["id"], "bars": l["bars"]}
            last_w[text] = w
        punchlines = sorted(pl.values(), key=lambda p: p["at_ms"])
        seg_laughs = [l for l, _ in by_seg[i]]
        r, basis, lstats, rel = rate(seg_laughs, end_ms - start_ms, rm)
        jwin = M.joke_window(start_ms, end_ms, seg_laughs, next_start[i], window)
        jm = M.joke_metrics(start_ms, seg_laughs, counted, jwin, sm["laugh_s_per_min"])
        lstats.update({"merged_count": jm.pop("merged_count"), "laugh_s": jm.pop("laugh_s"), "window_s": jm.pop("window_s")})

        joke_id = f"joke:{s['joke']}" if s.get("joke") else None
        if joke_id and joke_id not in cat.jokes:
            raise ValueError(f"{set_dir}: unknown joke {joke_id}")
        unit_id = None
        if status == "known" and joke_id:
            uid = s.get("unit") or pick_version(cat, joke_id, tr, a, b)
            if uid not in cat.units:
                raise ValueError(f"{set_dir}: unknown unit {uid}")
            unit_id = uid
        text = tr.span(a, b)

        if status in ("known", "new", "crowd_work") and (joke_id or status == "new"):
            excl = tuple(f"joke:{x}" for x in s.get("not_similar", []))
            sim = similar(cat, emb, joke_id=joke_id if joke_id in cat.joke_vectors(emb)[0] else None,
                          text=text, extra_labels={h["label"]: 0.8 for h in s.get("hints", [])},
                          exclude=excl + ((joke_id,) if joke_id else ()), unit_id=unit_id)
        else:
            sim = []
        for j_ in s.get("similar", []):  # judged similar jokes go first
            sim = [x for x in sim if x["joke_id"] != f"joke:{j_['joke']}"]
            sim.insert(0, {"joke_id": f"joke:{j_['joke']}", "why": j_["why"], "confidence": j_.get("confidence", 0.7)})
        sim = sim[:3]

        hints = [{"label_id": h["label"], "why": h["why"]} for h in s.get("hints", [])]
        if status == "known":
            for h in hint_labels(cat, joke_id, unit_id):
                if h["label_id"] not in {x["label_id"] for x in hints}:
                    hints.append(h)
        hints = hints[:5]

        sd = None
        if status == "known" and unit_id:
            sd, _ = diff(cat.units[unit_id]["text"], tr, a, b)

        jokes.append({
            "idx": len(jokes) + 1, "start_ms": start_ms, "end_ms": end_ms, "title": s["title"], "status": status,
            "joke_id": joke_id, "version_unit_id": f"unit:{unit_id}" if unit_id else None,
            "match_confidence": s.get("confidence", 0.0) if joke_id else 0.0,
            "match_reason": s.get("reason", ""),
            "transcript": text, "punchlines": punchlines, "laughs": lstats,
            **jm,
            "ai_rating": r, "rating_basis": basis,
            "similar": sim, "hint_labels": hints, "script_diff": sd,
            "_rel": rel, "_win": jwin,
        })

    cands = [j for j in jokes if j["status"] in JOKE_STATUSES and j["joke_id"]]
    # rank: rating first, then absolute payoff (laugh seconds + 2 per bar of the biggest laugh), then rate
    key = lambda j: (ORDER[j["ai_rating"]], j["laughs"]["total_s"] + 2 * j["laughs"]["max_bars"], j["_rel"])
    best = max(cands, key=key)["joke_id"] if cands else None
    weakest = min(cands, key=key)["joke_id"] if cands else None
    lg = sm.pop("_longest")
    lj = next((j for j in jokes if lg and any(wa <= lg[0] < wb for wa, wb in j["_win"])), None)
    for j in jokes:
        j.pop("_rel"), j.pop("_win")
    n_laughs = sum(j["laughs"]["count"] for j in jokes)
    summary = {
        "duration_s": int(round(meta.get("duration") or tr.audio_sec or 0)),
        "jokes_counted": sum(1 for j in jokes if j["status"] in JOKE_STATUSES),
        "known_jokes": sum(1 for j in jokes if j["status"] == "known"),
        "new_material": sum(1 for j in jokes if j["status"] == "new"),
        "crowd_work": sum(1 for j in jokes if j["status"] == "crowd_work"),
        "laughs": n_laughs,
        "laughs_per_min": round(n_laughs / rm["minutes"], 2),
        "best": best, "weakest": weakest,
        # laugh-time metrics (INFER.md "Laugh metrics"); laughs_per_min above is a raw event count
        **sm,
        "longest_laugh_joke": ({"idx": lj["idx"], "joke_id": lj["joke_id"], "title": lj["title"], "start_ms": lg[0]}
                               if lj else ({"idx": None, "joke_id": None, "title": None, "start_ms": lg[0]} if lg else None)),
        "room_heat": None,  # filled by sets.py once every set's laugh_s_per_min is known
    }
    return {"set": set_id(set_dir).split(":", 1)[1], "run_id": run_id,
            "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
            "summary": summary, "jokes": jokes, **({"excluded": excluded} if excluded else {})}, tr
