"""Laughs per set: which events count, the room baseline, and punchlines.

A laugh is a measurement, not a claim. Every detector event in the performance
counts, except walk-on applause and events a judge file drops (`laugh_drop`,
each with a reason). Without a judge file, events under AUTO_MIN_CONF that
overlap his speech are dropped (they are mostly his own voice)."""
import json
import os

from .paths import SETS

AUTO_MIN_CONF = 0.4
PUNCH_MAX_WORDS = 14
PUNCH_MIN_WORDS = 4
PAUSE_MS = 700


def load(set_dir):
    return json.load(open(os.path.join(SETS, set_dir, "laughs_auto.json"), encoding="utf-8"))


def counted(raw, judgment):
    drop = (judgment or {}).get("laugh_drop", {})
    out = []
    for l in raw["laughs"]:
        if l.get("segment") == "walk_on":
            continue
        if l["id"] in drop:
            continue
        if judgment is None and l["confidence"] < AUTO_MIN_CONF and l["detail"].get("speech_overlap", 0) > 0.5:
            continue
        out.append(l)
    return out


def punch_word(tr, l):
    """The word the laugh answers: the last word he finished by laugh onset
    (+100 ms detector slack). If the laugh overlaps continued speech, the words
    after onset are ignored. Falls back to the last word started before onset."""
    on = l["start_ms"]
    idx = None
    for i, w in enumerate(tr.words):
        if w["e_ms"] <= on + 100:
            idx = i
        elif w["s_ms"] >= on:
            break
    return idx if idx is not None else tr.word_at_or_before(on + 100)


def punch_phrase(tr, i):
    """Words [a, i+1) ending at word i: back to a sentence end or a pause,
    at least PUNCH_MIN_WORDS and at most PUNCH_MAX_WORDS words."""
    a = i
    while a > 0 and (i - a + 1) < PUNCH_MAX_WORDS:
        prev, cur = tr.words[a - 1], tr.words[a]
        boundary = prev["w"][-1:] in ".?!" or cur["s_ms"] - prev["e_ms"] >= PAUSE_MS
        if boundary and (i - a + 1) >= PUNCH_MIN_WORDS:
            break
        a -= 1
    return a, i + 1


def room(tr, laughs):
    """Baseline for the room: laugh count, laugh-seconds per minute of stage time."""
    if not tr.words:
        return {"minutes": 0, "laughs": 0, "laugh_s": 0, "laugh_s_per_min": 0, "laughs_per_min": 0}
    t0, t1 = tr.words[0]["s_ms"], tr.words[-1]["e_ms"]
    minutes = max(0.5, (t1 - t0) / 60000)
    ls = sum((l["end_ms"] - l["start_ms"]) / 1000 for l in laughs)
    return {"minutes": minutes, "laughs": len(laughs), "laugh_s": ls,
            "laugh_s_per_min": ls / minutes, "laughs_per_min": len(laughs) / minutes}
