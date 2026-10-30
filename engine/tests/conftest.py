"""Fixtures: a tiny temp KB, a scripted mock provider, fake audio stages and a fake embedder.

Nothing here touches the real comedy folder."""
from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from engine.config import Config  # noqa: E402
from engine.providers import Backend, register_kind  # noqa: E402

TOOLS = ROOT / "tools"

AIRPORT_UNIT = ("I keep wondering how bad it could get if you leave a belt in an airport bin big enough "
             "to walk around. It’s basically a studio apartment with a view.")
DATING_UNIT = "Dating apps are just job interviews where nobody gets the job and everybody lies about their height."

SEGMENTS = [
    "Put your hands together for the comic!",
    "You ever wonder what happens if you leave a belt in an airport bin?",
    "It's big enough to walk around, it's basically a studio apartment.",
    "So I bought a parrot that only speaks in legal disclaimers.",
    "Now every time I sneeze he says results may vary.",
]


class MockBackend(Backend):
    kind = "mock"
    calls: list = []
    fail_first_json = False

    def call(self, model, messages, schema, task_cfg):
        MockBackend.calls.append({"model": model, "messages": messages, "schema": schema})
        if MockBackend.fail_first_json:
            MockBackend.fail_first_json = False
            return "not json at all", 5, 3
        return json.dumps(respond(schema, messages)), 100, 20


def respond(schema, messages):
    props = (schema or {}).get("properties", {})
    user = messages[-1]["content"]
    if "units" in props:
        return {"units": [{"first": 0, "last": 0, "kind": "intro", "name": "Intro", "funny": False},
                          {"first": 1, "last": 2, "kind": "bit", "name": "Belt in a bin", "funny": True},
                          {"first": 3, "last": 4, "kind": "bit", "name": "Disclaimer parrot", "funny": True}]}
    if "judgments" in props:
        ids = re.findall(r"joke_id (joke:[\w-]+)", user)
        if "airport" in user.split("CANDIDATE")[0]:
            return {"judgments": [{"joke_id": "joke:airport-bit", "same_joke": True, "confidence": 0.9,
                                   "reason": "Same belt-in-a-bin premise.", "version_unit": "unit:aaaa-00",
                                   "new_quote": "leave a belt in an airport bin",
                                   "old_quote": "how bad it could get"}]
                    + [{"joke_id": i, "same_joke": False, "confidence": 0.1, "reason": "no", "version_unit": "",
                        "new_quote": "", "old_quote": ""} for i in ids if i != "joke:airport-bit"]}
        return {"judgments": [{"joke_id": i, "same_joke": False, "confidence": 0.1, "reason": "Different premise.",
                               "version_unit": "", "new_quote": "", "old_quote": ""} for i in ids]}
    if "is_material" in props:
        return {"is_material": True, "title": "Disclaimer parrot", "summary": "A parrot that only speaks in legal "
                "disclaimers.", "quote": "he says results may vary", "reason": "Got a laugh; new premise."}
    if "labels" in props:
        return {"labels": [
            {"label_id": "label:device:literal_interpretation", "quote": "results may vary" if "parrot" in user
             else "basically a studio apartment", "reason": "Takes it literally.", "confidence": 0.8},
            # a tampered quote: the source says 'may vary', not 'will vary' -> must be dropped
            {"label_id": "label:topic:dating", "quote": "results will vary" if "parrot" in user else
             "basically a studio condo", "reason": "x", "confidence": 0.7},
            {"label_id": "label:device:not_in_vocab", "quote": "results may vary", "reason": "x", "confidence": 0.9}]}
    if "links" in props:
        return {"links": []}
    if "search_queries" in props:
        return {"search_queries": ["airport"], "label_ids": [], "year_min": 0, "year_max": 0, "performed": "any",
                "joke_titles": [], "by_year": False}
    if "citations" in props:
        return {"found": True, "answer": "You wrote the airport bit [unit:aaaa-00]. You said \"leave a belt in an airport bin forever\".",
                "citations": [{"source_id": "unit:aaaa-00", "quote": "how bad it could get", "note": "ok"},
                              {"source_id": "unit:aaaa-00", "quote": "leave a belt in an airport bin forever", "note": "tampered"}],
                "not_found": ""}
    raise AssertionError("unscripted schema " + json.dumps(props)[:200])


register_kind("mock", MockBackend)


class FakeEmbedder:
    name = "fake-embed"

    def embed(self, texts, batch=8):
        out = np.zeros((len(texts), 384), np.float32)
        for i, t in enumerate(texts):
            for w in re.findall(r"[a-z]+", (t or "").lower()):
                out[i, int(hashlib.md5(w.encode()).hexdigest(), 16) % 384] += 1
            n = np.linalg.norm(out[i])
            out[i] = out[i] / n if n else out[i]
        return out

    def query(self, t):
        return self.embed([t])[0]


class FakeTranscriber:
    version = "fake-whisper@1"

    def __call__(self, wav, out_dir):
        segs = [{"start": i * 5.0, "end": i * 5.0 + 4.5, "text": s, "words": []} for i, s in enumerate(SEGMENTS)]
        doc = {"model": "fake", "segments": segs}
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "transcript.json").write_text(json.dumps(doc), encoding="utf-8")
        return doc


class FakeLaughs:
    version = "fake-laughs@1"

    def __call__(self, name, tapes, corpus_sets, run_id):
        return {"laughs": [{"id": "l14000", "start_ms": 14000, "end_ms": 15500, "bars": 2},
                           {"id": "l24600", "start_ms": 24600, "end_ms": 26000, "bars": 3}]}


def fake_normalize(src, wav):
    wav.parent.mkdir(parents=True, exist_ok=True)
    wav.write_bytes(b"RIFF-fake")
    return 25.0


def build_graph():
    nodes = [
        {"id": "joke:airport-bit", "type": "joke", "title": "Airport bit", "stage": "testing", "summary": "Belt in a bin.",
         "first_seen": "2014-11-10", "last_seen": "2025-10-11"},
        {"id": "joke:dating-apps", "type": "joke", "title": "Dating apps interviews", "stage": "idea",
         "summary": "Apps as job interviews.", "first_seen": "2019-01-01", "last_seen": "2019-01-01"},
        {"id": "unit:aaaa-00", "type": "unit", "text": AIRPORT_UNIT, "kind": "setup+punch", "source": "gdoc",
         "doc_title": "Airport security", "date": "2014-11-10"},
        {"id": "unit:bbbb-00", "type": "unit", "text": DATING_UNIT, "kind": "setup+punch", "source": "gdoc",
         "doc_title": "Dating apps", "date": "2019-01-01"},
        {"id": "label:device:literal_interpretation", "type": "label", "kind": "device", "name": "Literal interpretation",
         "definition": "Takes an expression at face value."},
        {"id": "label:device:fake_statistic", "type": "label", "kind": "device", "name": "Fake statistic",
         "definition": "A made-up number delivered as fact."},
        {"id": "label:topic:dating", "type": "label", "kind": "topic", "name": "dating", "definition": ""},
    ]
    E = lambda s, k, d, **kw: {"id": "e:" + hashlib.sha1(f"{s}|{k}|{d}".encode()).hexdigest()[:12], "src": s, "dst": d,
                               "kind": k, "status": kw.pop("status", "proposed"), "confidence": 0.8,
                               "reason": kw.pop("reason", "r"), "evidence": kw.pop("evidence", []), "source": "auto:seed"}
    edges = [
        E("unit:aaaa-00", "part_of", "joke:airport-bit"),
        E("unit:bbbb-00", "part_of", "joke:dating-apps"),
        E("joke:dating-apps", "has_label", "label:topic:dating",
          evidence=[{"node": "unit:bbbb-00", "quote": "Dating apps are just job interviews", "start_ms": None}],
          reason="OWNER-CONFIRMED-EXAMPLE"),
        E("joke:airport-bit", "has_label", "label:device:fake_statistic", status="confirmed_auto",
          evidence=[{"node": "unit:aaaa-00", "quote": "basically a studio apartment", "start_ms": None}],
          reason="AUTO-CONFIRMED-NEVER-AN-EXAMPLE"),
    ]
    return {"version": 0, "generated_at": "2026-10-25T00:00:00Z", "runs": [], "nodes": nodes, "edges": edges}


@pytest.fixture
def env(tmp_path, monkeypatch):
    monkeypatch.delenv("ANTHROPIC_API_KEY", raising=False)
    MockBackend.calls = []
    MockBackend.fail_first_json = False
    root = tmp_path / "root"
    (root / "kb").mkdir(parents=True)
    (root / "inbox").mkdir()
    g = build_graph()
    (root / "kb" / "graph.json").write_text(json.dumps(g), encoding="utf-8")
    confirm = {"ts": "2026-10-26T00:00:00+00:00", "target": "edge",
               "id": [e for e in g["edges"] if e["reason"] == "OWNER-CONFIRMED-EXAMPLE"][0]["id"],
               "action": "confirm", "before": {"status": "proposed"}, "after": {"status": "confirmed"}, "by": "owner"}
    auto = {"ts": "2026-10-26T00:00:01+00:00", "target": "edge",
            "id": [e for e in g["edges"] if e["reason"] == "AUTO-CONFIRMED-NEVER-AN-EXAMPLE"][0]["id"],
            "action": "confirm", "before": {}, "after": {"status": "confirmed"}, "by": "auto", "auto": True}
    (root / "kb" / "decisions.jsonl").write_text(json.dumps(confirm) + "\n" + json.dumps(auto) + "\n", encoding="utf-8")
    toml = tmp_path / "config.toml"
    tasks = "\n".join(f'[tasks.{t}]\nproviders = ["mock"]\nmodels = {{ mock = "mock-1" }}\noffline = true\n'
                      for t in ["default", "split", "match_judge", "propose_joke", "label", "link_judge", "ask_plan",
                                "ask_answer"])
    toml.write_text(f"""
[paths]
comedy_dir = "{tmp_path.as_posix()}"
kb_dir = "{(root / 'kb').as_posix()}"
corpus_dir = "{(root / 'corpus').as_posix()}"
out_corpus_dir = "{(root / 'corpus').as_posix()}"
inbox_dir = "{(root / 'inbox').as_posix()}"
processed_dir = "{(root / 'processed').as_posix()}"
tools_dir = "{TOOLS.as_posix()}"
vectors_dir = "{(tmp_path / 'novectors').as_posix()}"
bge_dir = "{(tmp_path / 'nobge').as_posix()}"
secrets_env = "{(tmp_path / 'secrets.env').as_posix()}"

[inbox]
set_min_seconds = 240
match_candidates = 2
match_min_confidence = 0.5
link_candidates = 2
audio_ext = [".opus", ".m4a", ".mov", ".mp4", ".m4v"]

[providers.mock]
kind = "mock"

{tasks}
""", encoding="utf-8")
    cfg = Config(toml)
    return {"root": root, "cfg": cfg, "toml": toml, "tmp": tmp_path}


def drop_audio(root: Path, name="2026-10-20_test-room.opus", payload=b"fake-opus-bytes-1"):
    p = root / "inbox" / name
    p.write_bytes(payload)
    return p
