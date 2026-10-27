"""A tiny synthetic corpus: 6 jokes, 3 sets, known links. No private material."""
from __future__ import annotations

import os
from pathlib import Path

import pytest

from creative.data import Corpus

COMEDY = Path(os.environ.get("COMEDY_DIR", os.path.expanduser("~/punchable-data")))
HAS_REAL = (COMEDY / "kb" / "v1" / "graph.json").exists()

TEXT = {
    "a": "Alpha opener line one. The alpha punch lands here every time.",
    "b": "Bravo setup about the dog. The purple monkey dishwasher line here. Bravo closer words.",
    "c": "Charlie premise about cars and traffic and more traffic today.",
    "d": "Delta bit about my niece and the laundromat place.",
    "e": "Echo bit about my niece and her soccer schedule.",
    "f": "Foxtrot never performed idea about planes and pilots up there.",
}


def _seg(idx, joke, start_s, dur_s, text, laugh_s, set_rate, status="known", ttfl=5.0, punch=None):
    window = dur_s
    lspm = round(laugh_s / (window / 60), 2) if window else 0.0
    return {"idx": idx, "start_ms": int(start_s * 1000), "end_ms": int((start_s + dur_s) * 1000), "title": f"T{idx}",
            "status": status, "joke_id": joke, "version_unit_id": None, "match_confidence": 0.9, "transcript": text,
            "punchlines": punch or [], "laughs": {"count": 1, "max_bars": 2, "total_s": laugh_s, "merged_count": 1,
                                                 "laugh_s": laugh_s, "window_s": window},
            "laugh_s_per_min": lspm, "pct_laughing": 0.0, "time_to_first_laugh_s": ttfl if laugh_s else None,
            "dead_air_max_s": 5.0, "vs_room": round(lspm / set_rate, 2), "ai_rating": "worked" if laugh_s else "died",
            "rating_basis": "", "similar": [], "hint_labels": [],
            "script_diff": {"on_script_pct": 50, "dropped": [], "added": []}}


def _analysis(key, rate, segs):
    return {"set": key, "summary": {"laugh_s_per_min": rate, "room_heat": 1.0, "material_s": 300.0, "best": None,
                                    "weakest": None, "jokes_counted": len(segs), "new_material": 0}, "jokes": segs}


def make_graph():
    nodes, edges = [], []
    for k in "abcdef":
        nodes.append({"id": f"joke:{k}", "type": "joke", "title": k.upper() + " bit", "stage": "testing",
                      "lead_unit": f"unit:{k}1", "anatomy": {"reveal": "punch"},
                      "quality": {"notes": f"Note for {k}: keep it tight."}})
        nodes.append({"id": f"unit:{k}1", "type": "unit", "kind": "setup+punch", "text": TEXT[k]})
        edges.append({"id": f"e:p{k}", "src": f"unit:{k}1", "dst": f"joke:{k}", "kind": "part_of",
                      "status": "inferred", "confidence": 0.9, "evidence": []})
    for lid, kind, name in [("label:persona:dad", "persona", "Dad"), ("label:topic:kids", "topic", "Kids"),
                            ("label:topic:cars", "topic", "Cars"), ("label:device:d1", "device", "D-one"),
                            ("label:device:d2", "device", "D-two"), ("label:structure:s1", "structure", "S-one")]:
        nodes.append({"id": lid, "type": "label", "kind": kind, "name": name, "definition": name})

    def lab(j, l, w, q):
        edges.append({"id": f"e:l{j}{l}", "src": f"joke:{j}", "dst": l, "kind": "has_label", "status": "proposed",
                      "confidence": w, "evidence": [{"node": f"unit:{j}1", "quote": q}]})
    lab("a", "label:device:d1", 0.9, "Alpha opener")
    lab("b", "label:device:d1", 0.9, "purple monkey")
    lab("c", "label:device:d2", 0.9, "Charlie premise")
    lab("d", "label:persona:dad", 0.8, "my niece")
    lab("e", "label:persona:dad", 0.8, "my niece")
    lab("d", "label:topic:kids", 0.8, "laundromat")
    lab("e", "label:topic:kids", 0.8, "soccer schedule")
    lab("c", "label:topic:cars", 0.8, "cars and traffic")
    edges.append({"id": "e:seg_bc", "src": "joke:b", "dst": "joke:c", "kind": "segue", "status": "proposed",
                  "confidence": 0.7, "reason": "dog leads to cars",
                  "evidence": [{"node": "unit:b1", "quote": "Bravo closer words."},
                               {"node": "unit:c1", "quote": "Charlie premise"}]})
    edges.append({"id": "e:bad", "src": "joke:c", "dst": "joke:f", "kind": "callback_to", "status": "proposed",
                  "confidence": 0.6, "reason": "fabricated quote test",
                  "evidence": [{"node": "unit:c1", "quote": "THIS IS NOT IN THE TEXT"}]})
    return {"nodes": nodes, "edges": edges}


def make_analyses():
    s1, s2, s3 = "2024-01-01_x", "2024-02-01_y", "2024-03-01_z"
    return {
        s1: _analysis(s1, 6.0, [
            _seg(1, "joke:a", 0, 60, TEXT["a"], 9.0, 6.0),
            _seg(2, "joke:b", 60, 60, TEXT["b"], 6.0, 6.0,
                 punch=[{"text": "The purple monkey dishwasher line here.", "at_ms": 70000, "laugh_id": "l1", "bars": 3}]),
            _seg(3, None, 120, 20, "So who here has a car?", 0.0, 6.0, status="crowd_work"),
            _seg(4, "joke:c", 140, 60, TEXT["c"], 6.0, 6.0)]),
        s2: _analysis(s2, 6.0, [
            _seg(1, "joke:a", 0, 60, TEXT["a"], 8.4, 6.0),
            _seg(2, "joke:b", 60, 90, "Bravo setup about the dog. Bravo closer words.", 2.0, 6.0, ttfl=40.0),
            _seg(3, "joke:d", 150, 60, TEXT["d"], 3.0, 6.0)]),
        s3: _analysis(s3, 5.0, [
            _seg(1, "joke:b", 0, 40, "Bravo setup about the dog. Bravo closer words about the cat today.", 1.5, 5.0, ttfl=30.0),
            _seg(2, "joke:a", 40, 30, "Alpha opener line one.", 4.0, 5.0),
            _seg(3, "joke:a", 70, 30, "The alpha punch lands here every time.", 4.0, 5.0)]),
    }


@pytest.fixture
def corpus():
    return Corpus(make_graph(), make_analyses())


@pytest.fixture(scope="session")
def real():
    if not HAS_REAL:
        pytest.skip("real KB not on this machine")
    from creative.data import load_corpus
    return load_corpus(COMEDY)


def walk_evidence(obj):
    """Every {node, quote} pair anywhere in an output."""
    if isinstance(obj, dict):
        if "node" in obj and "quote" in obj:
            yield obj
        for v in obj.values():
            yield from walk_evidence(v)
    elif isinstance(obj, list):
        for v in obj:
            yield from walk_evidence(v)


class FakeCompletion:
    def __init__(self, data):
        self.data, self.provider, self.model, self.call_id = data, "fake", "fake-1", "c0"


class FakeLLM:
    """Records prompts; answers with a fixed payload (or a function of the prompt)."""

    def __init__(self, payload):
        self.payload, self.calls = payload, []

    def __call__(self, task, messages, schema):
        self.calls.append((task, messages))
        p = self.payload(messages) if callable(self.payload) else self.payload
        return FakeCompletion(p)
