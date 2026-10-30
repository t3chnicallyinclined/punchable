"""VAD gap-fill and coverage (tools/audio/vad.py) and the transcribe gap-fill guards."""
import math
import sys
from collections import namedtuple
from pathlib import Path

import numpy as np
import pytest

from tools.audio import vad as V


def test_gaps_finds_uncovered_speech_and_pads_it():
    regions = [(0.0, 10.0), (20.0, 30.0)]
    words = [(0.2, 1.0), (1.1, 2.0), (2.1, 4.0), (20.1, 29.9)]
    g = V.gaps(regions, words, audio_s=40.0)
    assert len(g) == 1
    a, b = g[0]
    assert abs(a - (4.0 + V.WORD_TOL_S - V.GAP_PAD_S)) < 1e-6 and abs(b - (10.0 + V.GAP_PAD_S)) < 1e-6


def test_short_gaps_are_ignored_and_close_gaps_are_joined():
    regions = [(0.0, 30.0)]
    words = [(0.0, 5.0), (5.6, 10.0), (12.0, 20.0), (20.8, 30.0)]  # holes 0.6 s, 2.0 s, 0.8 s
    g = V.gaps(regions, words, audio_s=30.0)
    # only the 2.0 s hole survives: uncovered (10.25, 11.75) after the 0.25 s word tolerance, padded 0.25 s
    assert [(round(a, 3), round(b, 3)) for a, b in g] == [(10.0, 12.0)]
    regions2 = [(0.0, 40.0)]
    words2 = [(0.0, 5.0), (8.6, 9.0), (12.0, 40.0)]  # two gaps 0.6 s apart after padding -> one clip
    g2 = V.gaps(regions2, words2, audio_s=40.0)
    assert len(g2) == 1


def test_coverage_counts_only_vad_speech():
    regions = [(0.0, 10.0)]
    words = [(0.0, 4.75)]
    c = V.coverage(regions, words)
    assert c["vad_speech_s"] == 10.0 and c["covered_s"] == 5.0 and c["coverage"] == 0.5


def test_gap_check_rejects_word_timings_in_milliseconds():
    """Deliberately broken input: word times in ms (a units mix-up) must not silently produce
    'no gaps' - the gate raises."""
    regions = [(0.0, 10.0)]
    with pytest.raises(ValueError):
        V.gaps(regions, [(200.0, 1000.0), (1100.0, 2000.0)])
    with pytest.raises(ValueError):
        V.gaps(regions, [(5.0, 6.0), (1.0, 2.0)])  # unsorted


def test_speech_timestamps_follow_silero_rules():
    sr, chunk = V.SR, V.CHUNK
    per_s = sr / chunk
    probs = np.array([0.05] * int(per_s) + [0.9] * int(2 * per_s) + [0.05] * int(per_s), np.float32)
    regions = V.speech_timestamps(probs, len(probs) * chunk)
    assert len(regions) == 1
    a, b = regions[0]
    assert abs(a - (1.0 - V.SPEECH_PAD_MS / 1000)) < 0.05 and abs(b - (3.0 + V.SPEECH_PAD_MS / 1000)) < 0.1
    # a 100 ms blip is shorter than MIN_SPEECH_MS and is dropped
    blip = np.array([0.05] * 40 + [0.9] * 3 + [0.05] * 40, np.float32)
    assert V.speech_timestamps(blip, len(blip) * chunk) == []


def test_speaking_mask_trims_whisper_word_smear_to_vad_edges():
    regions = [(0.0, 5.0)]
    words = [(0.5, 4.0), (4.0, 9.0)]  # Whisper smeared the last word into the laugh
    m = V.speaking_mask(regions, words)
    assert m == [(0.2, 5.0)]


def test_silero_model_runs_with_context():
    if not V.MODEL.exists():
        pytest.skip("silero model not present")
    pytest.importorskip("onnxruntime")
    s = V.Silero()
    x = np.zeros(16000 * 2, np.float32)
    p = s.probs(x)
    assert len(p) == math.ceil(len(x) / V.CHUNK) and float(p.max()) < 0.5  # silence is not speech


# ---- transcribe.py gap-fill guards (faster-whisper only needed for import) ----

Seg = namedtuple("Seg", "text words no_speech_prob avg_logprob compression_ratio start end")
Word = namedtuple("Word", "word probability start end")


@pytest.fixture(scope="module")
def T():
    pytest.importorskip("faster_whisper")
    sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "ingest"))
    import transcribe
    return transcribe


def seg(text, probs):
    ws = [Word(w, p, 0.0, 0.1) for w, p in zip(text.split(), probs)]
    return Seg(text, ws, 0.1, -0.3, 1.2, 0.0, 1.0)


def test_guard_keeps_a_real_line(T):
    assert T.guard(seg("I'd have parked the car on the lawn.", [0.6] * 8)) is None


def test_guard_rejects_laughter_and_noise(T):
    assert "laughter" in T.guard(seg("Ha, ha, ha, ha, ha.", [0.9] * 5))
    assert "laughter" in T.guard(seg("HAHAHAHAHAHA", [0.9]))
    assert "single word" in T.guard(seg("speech.", [0.07]))
    assert "hallucination" in T.guard(seg("Thank you.", [0.9, 0.9]))
    assert "mean word" in T.guard(seg("you know what I mean", [0.1, 0.2, 0.2, 0.1, 0.2]))
