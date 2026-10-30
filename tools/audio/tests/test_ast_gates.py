"""AST gates: sigmoid not softmax (the the earlier clip tool bug), label indices from config.json,
fbank slicing, and the min frame curve. Run: python -m pytest tools/audio/tests -q"""
import json

import numpy as np
import pytest

from tools.laughs import ast_detect as A

SPEECH, LAUGHTER = 0, 16


def multi_label_logits():
    """A room laugh under the comic's voice: Speech and Laughter both clearly present."""
    z = np.full((1, 527), -6.0, dtype=np.float32)
    z[0, SPEECH] = 4.0
    z[0, LAUGHTER] = 3.0
    return z


def test_detector_maps_logits_with_sigmoid():
    assert A.LOGITS_TO_PROBS is A.sigmoid
    p = A.LOGITS_TO_PROBS(multi_label_logits())
    assert p[0, LAUGHTER] > 0.9 and p[0, SPEECH] > 0.9  # both present: multi-label
    A.assert_multilabel_probs(multi_label_logits(), p)


def test_softmax_is_the_cliptool_bug_and_the_gate_catches_it():
    """Would have caught the earlier clip tool src/ast.rs:196: softmax makes the classes share 1.0, so a laugh
    under speech reads as ~0.27, and the gate refuses it."""
    z = multi_label_logits()
    sm = A.softmax(z)
    assert abs(float(sm.sum()) - 1.0) < 1e-5
    assert sm[0, LAUGHTER] < 0.5  # the laugh is crushed by the speech class
    with pytest.raises(AssertionError, match="not sigmoid"):
        A.assert_multilabel_probs(z, sm)


def test_probs_for_set_runs_through_the_sigmoid_gate():
    """End to end through AST.probs_for_set with a stub session: the output equals sigmoid(logits)."""
    z = multi_label_logits()

    class Sess:
        def run(self, _out, feeds):
            n = next(iter(feeds.values())).shape[0]
            return [np.repeat(z, n, axis=0)]

    m = A.AST.__new__(A.AST)
    m.sess, m.input_name, m.in_dtype = Sess(), "input_values", np.float32
    fb = np.zeros((1500, 128), np.float32)  # 15 s of frames
    starts, P, _ = m.probs_for_set(None, hop_s=1.0, batch=4, fb=fb)
    assert len(starts) == 15
    np.testing.assert_allclose(P, np.repeat(A.sigmoid(z), 15, axis=0), atol=1e-6)


@pytest.fixture(scope="module")
def config():
    p = A.MODELS / "config.json"
    if not p.exists():
        pytest.skip("AST config.json not downloaded")
    return json.loads(p.read_text(encoding="utf-8"))


def test_label_indices_come_from_config(config):
    L = {n: i for g in A.label_indices(config).values() for n, i in g}
    assert L["Laughter"] == 16 and L["Applause"] == 67 and L["Cheering"] == 66 and L["Crowd"] == 69
    assert L["Giggle"] == 18 and L["Belly laugh"] == 20 and L["Chuckle, chortle"] == 21
    # the earlier clip tool hardcodes 13 for laughter and 36 for applause:
    assert config["id2label"]["13"] == "Children shouting"
    assert config["id2label"]["36"] == "Rapping"


def test_label_lookup_fails_on_broken_config(config):
    renamed = json.loads(json.dumps(config))
    renamed["id2label"]["16"] = "Laughing"  # the name the code asks for is gone
    with pytest.raises(KeyError):
        A.label_indices(renamed)
    dup = json.loads(json.dumps(config))
    dup["id2label"]["13"] = "Laughter"  # two classes answer to one name
    with pytest.raises(KeyError):
        A.label_indices(dup)
    short = json.loads(json.dumps(config))
    del short["id2label"]["526"]
    with pytest.raises(ValueError):
        A.label_indices(short)


def test_fbank_once_then_slice_equals_per_window_fbank():
    pytest.importorskip("torchaudio")
    rng = np.random.default_rng(0)
    x = (rng.standard_normal(16000 * 14) * 0.1).astype(np.float32)
    whole = A.fbank(x)
    start_s = 3  # window at 3 s = frame 300
    per = A.fbank(x[start_s * 16000: start_s * 16000 + 16000 * 10 + 240])
    np.testing.assert_allclose(whole[300:300 + A.REAL_FRAMES], per[:A.REAL_FRAMES], atol=1e-4)


def test_min_curve_localises_one_event():
    hop, audio_s = 1.0, 40.0
    starts = np.arange(0, 40, hop)
    event = (20.0, 21.0)
    win = A.REAL_FRAMES / 100
    p = np.array([0.9 if s < event[1] and s + win > event[0] else 0.05 for s in starts], np.float32)
    c = A.frame_curve(p, starts, hop, audio_s)
    assert c[20] > 0.5
    assert c[:19].max() < 0.1 and c[22:].max() < 0.1  # no smear across the 10 s window
