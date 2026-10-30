"""clips.py: NMS spacing, weight redistribution, word-edge snapping, and the verbatim gate."""
import glob
import json
import os

import pytest

from infer import clips as C
from infer.paths import SETS


def cand(s, e, score):
    return {"start_ms": int(s * 1000), "end_ms": int(e * 1000), "score": score}


def test_nms_spreads_picks_and_keeps_set_order():
    cluster = [cand(100 + i, 140 + i, 0.9 - i * 0.001) for i in range(10)]  # one loud moment, ten windows
    spread = [cand(300 + 100 * i, 330 + 100 * i, 0.5) for i in range(5)]
    picks = C.nms(cluster + spread, keep=4)
    assert sum(1 for p in picks if p["start_ms"] < 200_000) == 1
    assert [p["start_ms"] for p in picks] == sorted(p["start_ms"] for p in picks)
    assert C.check_spacing(picks)


def test_spacing_gate_fails_on_overlapping_picks():
    with pytest.raises(AssertionError, match="overlap"):
        C.check_spacing([cand(0, 40, 1), cand(10, 50, 1)])


def test_missing_signal_does_not_reshape_the_ranking():
    a = {"laugh_s": 0.8, "bars_peak": 0.75, "tightness": 0.6}
    b = {"laugh_s": 0.4, "bars_peak": 0.5, "tightness": 0.9}
    sa, _ = C.combine(a)
    sb, _ = C.combine(b)
    sa2, contrib = C.combine({**a, "novelty": None, "ast": None})
    sb2, _ = C.combine({**b, "novelty": None, "ast": None})
    assert (sa > sb) == (sa2 > sb2) and abs(sa - sa2) < 1e-12
    assert contrib["novelty"] is None
    assert abs(sum(v for v in contrib.values() if v is not None) - sa) < 1e-3


class FakeTr:
    def __init__(self, words):
        self.words = [{"w": w, "s_ms": s, "e_ms": e, "seg": 0} for w, s, e in words]


def test_snap_end_never_cuts_a_word_and_never_trails_into_the_next_line():
    tr = FakeTr([("airport", 1000, 1400), ("forever.", 1450, 2000), ("Twenty", 3600, 4000), ("minutes,", 4050, 4600)])
    end = C.snap_end(tr, laugh_end_ms=5200, floor_ms=2000, laugh_start_ms=2050)
    assert end < 3600  # stops before he starts the next line
    assert end >= 2050 + C.MIN_LAUGH_MS or end == 3600 - 20
    assert not any(w["s_ms"] < end < w["e_ms"] for w in tr.words)


def test_verbatim_gate_fails_on_an_altered_span():
    text = "you have to sit to have a lap"
    ok = [{"rank": 1, "transcript": "sit to have a lap", "punchline": "have a lap"}]
    assert C.check_verbatim(ok, text)
    bad = [{"rank": 1, "transcript": "sit to have a lpa", "punchline": "have a lap"}]
    with pytest.raises(AssertionError, match="not verbatim"):
        C.check_verbatim(bad, text)


@pytest.mark.skipif(not glob.glob(os.path.join(SETS, "*", "clips.json")), reason="no clips.json written yet")
def test_every_written_clip_is_verbatim_spaced_and_in_range():
    from infer.transcript import Transcript
    n = 0
    for p in glob.glob(os.path.join(SETS, "*", "clips.json")):
        doc = json.load(open(p, encoding="utf-8"))
        tr = Transcript(doc["set"])
        C.check_verbatim(doc["clips"], tr.text)
        C.check_spacing(doc["clips"])
        for c in doc["clips"]:
            assert C.MIN_S <= c["duration_s"] <= C.MAX_S
            assert c["transcript"].endswith(c["punchline"]) or c["punchline"] in c["transcript"]
            n += 1
    assert n > 0
