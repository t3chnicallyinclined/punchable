"""Resolve a set's segmentation (judge/<set>.json anchors) to word ranges."""
import json
import os

from .paths import JUDGE


def load_judgment(set_dir):
    p = os.path.join(JUDGE, f"{set_dir}.json")
    return json.load(open(p, encoding="utf-8")) if os.path.exists(p) else None


def resolve(tr, judgment):
    """[(seg_spec, a, b)] word ranges; a segment ends where the next starts,
    or before its `end_before` anchor."""
    specs = judgment["segments"]
    starts, prev = [], 0
    for s in specs:
        a, _ = tr.find(s["anchor"], s.get("seg", 0), prev)
        starts.append(a)
        prev = a + 1
    out = []
    for i, s in enumerate(specs):
        a = starts[i]
        b = starts[i + 1] if i + 1 < len(starts) else len(tr.words)
        if s.get("end_before"):
            b = min(b, tr.find(s["end_before"], tr.words[a]["seg"], a + 1)[0])
        if s.get("end_after"):
            b = min(b, tr.find(s["end_after"], tr.words[a]["seg"], a)[1])
        out.append((s, a, b))
    return out
