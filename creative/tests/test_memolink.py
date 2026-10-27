"""creative/memolink.py on the synthetic merge corpus (no private material): summary vectors make new notes matchable,
a clear attach needs BOTH the judge (>= 0.90, one bit) and the matcher, set recordings are never grouped or
attached, and "not a joke?" follows the triage verdicts. Each guard fails when removed."""
from __future__ import annotations

import json

import numpy as np

from creative import memolink as ML
from creative import merge as M
from creative.data import Corpus, Vectors

from .conftest import FakeLLM
from .test_merge import _vec, judge_payload, make

MEMO_TEXT = "Marty knocked out the wrong guy again, this one was wearing brown too, the UPS guy got knocked out cold"


def with_memos(tmp_path):
    """The synthetic corpus + two summary-only ideas: m1 (the Marty bit, retold) and s1 (a set recording that also
    tells it), each with a `summary:` vector near the Marty vectors."""
    c0, table = make()
    nodes = [dict(n) for n in c0.nodes.values()]
    edges = [dict(e) for e in c0.edges]
    rng = np.random.default_rng(3)
    for k, title, text in (("m1", "Marty again", MEMO_TEXT),
                           ("s1", "What's up guys how we doing", "what's up guys how we doing " + MEMO_TEXT + " thank you guys")):
        nodes.append({"id": f"joke:{k}", "type": "joke", "title": title, "stage": "idea", "summary": text})
        table[f"summary:joke:{k}"] = _vec(rng, np.asarray(table["_vin"]), 0.02)
    c = Corpus({"nodes": nodes, "edges": edges}, {}, vectors=Vectors(table=table))
    return c, {"joke:m1": "memo1", "joke:s1": "memo2"}


def judge_all_same(c, tmp_path, conf):
    titles = {c.title(j) for j in ("joke:m1", "joke:s1")}
    same = {frozenset((t, b)): (True, conf) for t in titles for b in ("Marty delivery driver", "Karaoke king marty")}
    same[frozenset(("Marty delivery driver", "Karaoke king marty"))] = (True, 0.95)
    cache = M.judge(c, ML.memo_candidates(c, {"joke:m1": 1, "joke:s1": 1}) + M.judge_candidates(c), {},
                    FakeLLM(judge_payload(same)))
    (tmp_path / "merge_judge.json").write_text(json.dumps({"pairs": cache}), encoding="utf-8")


def test_summary_vectors_make_a_note_a_candidate(tmp_path):
    c, mj = with_memos(tmp_path)
    pairs = {(f["a"], f["b"]) for f in ML.memo_candidates(c, mj)}
    assert ("joke:m1", "joke:v1") in pairs or ("joke:m1", "joke:v2") in pairs


def test_clear_attach_needs_the_judge_and_the_matcher(tmp_path):
    c, mj = with_memos(tmp_path)
    judge_all_same(c, tmp_path, 0.95)
    out = ML.clear_attaches(c, tmp_path, mj, {})
    assert [x["memo_joke"] for x in out] == ["joke:m1", "joke:s1"] or [x["memo_joke"] for x in out] == ["joke:m1"]
    assert all(x["into"] == "joke:v1" for x in out)          # the Marty group's main
    # judge below the attach bar: a suggestion at most, never an attach
    c, mj = with_memos(tmp_path)
    judge_all_same(c, tmp_path, 0.85)
    assert ML.clear_attaches(c, tmp_path, mj, {}) == []


def test_set_recordings_are_never_grouped_or_attached(tmp_path):
    c, mj = with_memos(tmp_path)
    judge_all_same(c, tmp_path, 0.95)
    (tmp_path / "suggest_judge.json").write_text(json.dumps({"items": {
        "memo2": {"kind": "performed_set", "audience": True, "confidence": 0.9, "joke_id": "joke:s1"}}}), encoding="utf-8")
    out = ML.clear_attaches(c, tmp_path, mj, {})
    assert "joke:s1" not in [x["memo_joke"] for x in out]
    for g in ML.suggestions(c, tmp_path, mj, {}):
        assert "joke:s1" not in [m["joke_id"] for m in g["members"]]


def test_not_a_joke_follows_the_triage_bar():
    ms = {"a": {"joke_id": "joke:a", "text": "call the landlord", "title": "A", "date": "2026-01-01"},
          "b": {"joke_id": "joke:b", "text": "renew the certs", "title": "B", "date": "2026-01-02"},
          "c": {"joke_id": "joke:c", "text": "a bit about pigeons", "title": "C", "date": "2026-01-03"},
          "d": {"joke_id": "joke:d", "text": "ha ha", "title": "D", "date": "2026-01-04"},
          "e": {"joke_id": None, "text": "a long set", "title": "E", "date": "2026-01-05"}}
    cache = {"a": {"kind": "conversation", "confidence": 0.8}, "b": {"kind": "note", "confidence": 0.6},
             "c": {"kind": "joke", "confidence": 0.99}, "d": {"kind": "unclear", "confidence": 0.9, "chars": 5},
             "e": {"kind": "note", "confidence": 0.99}}
    for k, v in cache.items():
        v["text_sha"] = ML.sha(ms[k]["text"])
    assert [x["memo"] for x in ML.not_a_joke(cache, ms)] == ["a", "d"]
    cache["a"]["text_sha"] = "edited"          # the memo changed since it was judged: no stale flag
    assert [x["memo"] for x in ML.not_a_joke(cache, ms)] == ["d"]


def test_triage_never_sends_court_text(tmp_path):
    ms = {"a": {"text": "a bit about the subway pigeon with a monthly pass", "title": "A"},
          "b": {"text": "the petitioner and the respondent appeared, index no 7", "title": "B"}}
    llm = FakeLLM({"items": [{"id": "m1", "kind": "joke", "audience": False, "confidence": 0.9, "why": "a bit"}]})
    r = ML.triage(ms, tmp_path / "suggest_judge.json", llm, log=lambda *a: None)
    assert r["legal_skipped"] == 1 and r["sorted"] == 1
    sent = json.dumps([m for _, msgs in llm.calls for m in msgs])
    assert "petitioner" not in sent and "pigeon" in sent
