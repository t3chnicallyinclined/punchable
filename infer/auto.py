"""Fallback for a set with no judge file yet (e.g. a new set from the inbox).

Segments and matches by rules + bge-small only, so the analysis exists at once;
every match it makes stays under the 0.75 bar, so performed_as edges from it are
"proposed", not "inferred", until a Claude judge pass writes judge/<set>.json.
The draft is saved as judge/<set>.auto.json to start that pass from."""
import json
import os
import re

import numpy as np

from . import laughs as L
from .paths import JUDGE

MATCH_COS = 0.77
CROWD = re.compile(r"\b(anybody|anyone|who here|where are you|where you from|raise your hand|what do you do|"
                   r"how old are you|are you guys|you guys together)\b", re.I)


def auto_judgment(set_dir, tr, raw, cat, emb):
    pieces = tr.pieces(0, len(tr.words), max_words=30)
    if not pieces:
        return {"auto": True, "segments": []}
    texts = [tr.span(a, pieces[min(i + 2, len(pieces) - 1)][1]) for i, (a, _b) in enumerate(pieces)]
    Q = emb.texts(texts)
    uids = list(cat.units)
    U = np.stack([emb.unit(u, cat.units[u]["text"]) for u in uids])
    S = Q @ U.T
    best = S.argmax(1)
    lab = []
    for i in range(len(pieces)):
        u = uids[best[i]]
        lab.append((cat.unit_joke.get(u), float(S[i, best[i]]), u) if S[i, best[i]] >= MATCH_COS else (None, float(S[i, best[i]]), None))
    # smooth: a short unmatched gap between two pieces of the same joke joins them
    # smooth: a one-piece blip between two pieces of the same joke takes that joke (twice, for A-B-A-B)
    for _ in range(2):
        for i in range(1, len(lab) - 1):
            if lab[i - 1][0] and lab[i - 1][0] == lab[i + 1][0] != lab[i][0]:
                lab[i] = (lab[i - 1][0], lab[i][1], lab[i - 1][2])
    # punchlines and tags rarely embed close to the written setup: an unmatched run
    # of up to FILL pieces after a matched piece belongs to that joke
    FILL = 8
    i = 0
    while i < len(lab):
        if lab[i][0] is None and i > 0 and lab[i - 1][0]:
            k = i
            while k < len(lab) and lab[k][0] is None:
                k += 1
            if k - i <= FILL:
                for m in range(i, k):
                    lab[m] = (lab[i - 1][0], lab[m][1], lab[i - 1][2])
            i = k
        else:
            i += 1
    counted = L.counted(raw, None)
    laugh_words = {L.punch_word(tr, l) for l in counted}
    segs, cur = [], None
    for i, (a, b) in enumerate(pieces):
        j = lab[i][0]
        if cur and cur["joke"] == j:
            cur["b"], cur["cos"] = b, cur["cos"] + [lab[i][1]]
            continue
        cur = {"a": a, "b": b, "joke": j, "unit": lab[i][2], "cos": [lab[i][1]]}
        segs.append(cur)
    out = []
    for s in segs:
        text = tr.span(s["a"], s["b"])
        has_laugh = any(w is not None and s["a"] <= w < s["b"] for w in laugh_words)
        cos = float(np.mean(s["cos"]))
        spec = {"seg": tr.words[s["a"]]["seg"], "anchor": " ".join(w["w"] for w in tr.words[s["a"]:s["a"] + 6])}
        if s["joke"]:
            spec.update({"status": "known", "title": cat.jokes[s["joke"]]["title"], "joke": s["joke"].split(":", 1)[1],
                         "unit": s["unit"], "confidence": round(min(0.7, max(0.3, (cos - 0.7) / 0.25)), 2),
                         "reason": f"Auto (no judge file yet): nearest written unit by bge-small, cosine {cos:.2f}. Needs a judge pass."})
        elif len(CROWD.findall(text)) >= 1 and text.count("?") >= 2:
            spec.update({"status": "crowd_work", "title": "Crowd work", "reason": "Auto: questions to the room, no written match."})
        elif has_laugh:
            spec.update({"status": "new", "title": "New material: " + " ".join(text.split()[:6]),
                         "reason": f"Auto: got a laugh and matches nothing written (best cosine {cos:.2f})."})
        else:
            spec.update({"status": "banter", "title": "Banter", "reason": "Auto: no written match and no laugh."})
        out.append(spec)
    j = {"set": set_dir, "auto": True, "judged_by": "auto (rules + bge-small); not reviewed", "laugh_drop": {}, "segments": out}
    os.makedirs(JUDGE, exist_ok=True)
    with open(os.path.join(JUDGE, f"{set_dir}.auto.json"), "w", encoding="utf-8") as f:
        json.dump(j, f, ensure_ascii=False, indent=1)
    return j
