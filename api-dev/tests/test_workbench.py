"""The bit workbench's server side (2026-10-29): a line of his becomes a version (kb unit add + part_of), a merge keeps
a typed idea's words as a version, a conversation can be about one bit (pinned sources, a scope line), and his own
attach. Pure: a four-node graph in a temp folder, a fake corpus for the retrieval; no API process, no real data.

    python -m pytest api-dev/tests/test_workbench.py -q
"""
from __future__ import annotations

import json
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

DESK = Path(__file__).resolve().parents[2]
for p in (DESK / "api-dev", DESK / "tools" / "explorer", DESK):
    sys.path.insert(0, str(p))
import kb as K  # noqa: E402
import merge_api as MA  # noqa: E402
from creative import askchat as AC  # noqa: E402

GRAPH = {"version": "t", "nodes": [
    {"id": "joke:main", "type": "joke", "title": "Parking meters", "stage": "working", "summary": "AI summary, not his"},
    {"id": "unit:u1", "type": "unit", "text": "The meter asked for my pronouns before my card", "date": "2025-01-02"},
    {"id": "joke:note", "type": "joke", "title": "Meter that talks", "stage": "idea", "summary": "The meter talks back when you feed it a quarter"},
    {"id": "joke:cat", "type": "joke", "title": "Dating my cat", "stage": "idea", "summary": "An app for dating my cat"},
], "edges": [{"id": "e:p1", "kind": "part_of", "src": "unit:u1", "dst": "joke:main", "status": "confirmed"}]}


def make_kb(tmp_path) -> K.KB:
    (tmp_path / "graph.json").write_text(json.dumps(GRAPH), encoding="utf-8")
    (tmp_path / "decisions.jsonl").write_text("", encoding="utf-8")
    return K.KB(tmp_path / "graph.json", tmp_path / "decisions.jsonl", None, None)


# ------------------------------------------------------------------------------------------ a line is a version
def test_a_unit_can_be_added_and_becomes_a_version_through_part_of(tmp_path):
    kb = make_kb(tmp_path)
    r1 = kb.decide({"target": "node", "action": "add", "reason": "written in the app",
                    "after": {"type": "unit", "text": "  New version: the meter unionised.  ", "source": "app"}})
    uid = r1["decision"]["id"]
    assert uid.startswith("unit:") and r1["decision"]["after"]["text"] == "New version: the meter unionised."
    assert r1["decision"]["after"]["kind"] == "tag" and r1["decision"]["after"]["source"] == "app"
    assert K.DATE_RE.match(r1["decision"]["after"]["date"])
    r2 = kb.decide({"target": "edge", "action": "add", "reason": "written in the app",
                    "after": {"src": uid, "dst": "joke:main", "kind": "part_of"}})
    v = kb.view()
    assert v.unit_joke[uid] == "joke:main" and v.nodes[uid]["_by_tristan"] is True
    d = v.joke_detail("joke:main")
    texts = [u["text"] for g in d["unit_groups"] for u in g["units"]]
    assert "New version: the meter unionised." in texts and "The meter asked for my pronouns before my card" in texts
    # undo both, newest first: the version is gone, the old one stays
    kb.undo(r2["index"])
    kb.undo(r1["index"])
    v = kb.view()
    assert uid not in v.nodes and v.unit_joke.get("unit:u1") == "joke:main"


def test_unit_add_validates_text_and_id():
    class V:
        nodes = {"unit:taken": {"type": "unit"}}
        alias = {}
    kb = K.KB.__new__(K.KB)
    with pytest.raises(K.DecisionError):
        kb._build(V(), {"target": "node", "action": "add", "after": {"type": "unit", "text": "   "}})
    with pytest.raises(K.DecisionError):
        kb._build(V(), {"target": "node", "action": "add", "after": {"type": "unit", "text": "x" * 4001}})
    with pytest.raises(K.DecisionError):
        kb._build(V(), {"target": "node", "action": "add", "after": {"type": "unit", "text": "ok", "id": "unit:taken"}})
    d = kb._build(V(), {"target": "node", "action": "add", "after": {"type": "unit", "text": "ok", "id": "unit:draft-abc", "date": "2024-02-03", "doc_title": "Notes"}})
    assert d["id"] == "unit:draft-abc" and d["after"]["date"] == "2024-02-03" and d["after"]["doc_title"] == "Notes"
    d = kb._build(V(), {"target": "node", "action": "add", "after": {"type": "unit", "text": "ok", "id": "bad id", "date": "yesterday"}})
    assert d["id"].startswith("unit:") and d["id"] != "bad id" and K.DATE_RE.match(d["after"]["date"])


# --------------------------------------------------------------------------------- a merge keeps a typed idea
def test_typed_notes_are_the_ideas_with_his_words_and_no_version(tmp_path):
    kb = make_kb(tmp_path)
    # joke:note's summary is not his until a decision says so (the graph's summaries are the catalog's)
    assert MA.typed_notes(kb.view(), ["joke:note", "joke:main"]) == []
    kb.decide({"target": "node", "action": "edit", "id": "joke:note", "after": {"summary": "The meter talks back when you feed it a quarter. It wants a tip."}})
    got = MA.typed_notes(kb.view(), ["joke:note", "joke:main", "joke:cat"])
    assert [(j, t) for j, t, _ in got] == [("joke:note", "The meter talks back when you feed it a quarter. It wants a tip.")]
    # a typed idea that already has a version is not a typed note: its unit carries the words
    r = kb.decide({"target": "node", "action": "add", "after": {"type": "unit", "text": "a version"}})
    kb.decide({"target": "edge", "action": "add", "reason": "a version", "after": {"src": r["decision"]["id"], "dst": "joke:note", "kind": "part_of"}})
    assert MA.typed_notes(kb.view(), ["joke:note"]) == []


def test_merge_pre_plan_builds_a_unit_and_its_edge_into_main(tmp_path):
    """The two decisions do_merge writes before the plan for a typed note, built the way do_merge builds them."""
    kb = make_kb(tmp_path)
    kb.decide({"target": "node", "action": "edit", "id": "joke:note", "after": {"summary": "His words, typed."}})
    v = kb.view()
    [(o, text, date)] = MA.typed_notes(v, ["joke:note"])
    why = "[merge] mg:test: kept"
    # (the base KB has decide(); the hosted KB's append_lines writes the same lines in do_merge)
    r1 = kb.decide({"target": "node", "action": "add", "reason": why, "after": {"type": "unit", "text": text, "kind": "tag", "source": "idea", "date": date}})
    uid = r1["decision"]["id"]
    r2 = kb.decide({"target": "edge", "action": "add", "reason": why, "after": {"src": uid, "dst": "joke:main", "kind": "part_of"}})
    v = kb.view()
    assert v.unit_joke[uid] == "joke:main" and v.nodes[uid]["text"] == "His words, typed." and v.nodes[uid]["source"] == "idea"
    assert v.nodes[uid]["date"] == (date or K.now_iso()[:10])   # a note with no first_seen is dated today
    for i in (r2["index"], r1["index"]):
        kb.undo(i)
    assert uid not in kb.view().nodes


# ------------------------------------------------------------------------------ a conversation about one bit
def fake_corpus():
    S = {
        "unit:u1": {"id": "unit:u1", "kind": "bit", "text": "The meter asked for my pronouns before my card", "title": "Parking meters",
                    "date": "2025-01-02", "joke_id": "joke:main", "stage_line": "working", "nights": 2, "set_id": None, "start_ms": None, "venue": None, "rating": None},
        "joke:note": {"id": "joke:note", "kind": "idea", "text": "The meter talks back when you feed it a quarter", "title": "Meter that talks",
                      "date": "2025-03-01", "joke_id": "joke:note", "stage_line": "idea", "nights": 0, "set_id": None, "start_ms": None, "venue": None, "rating": None},
        "seg:s1#4": {"id": "seg:s1#4", "kind": "moment", "text": "so the parking meter wants my pronouns now", "title": "Parking meters",
                     "date": "2025-02-02", "joke_id": "joke:main", "set_id": "set:s1", "start_ms": 60000, "venue": "The Cellar", "rating": "worked",
                     "status": "known", "words": "", "laughs": 3, "stage_line": None, "nights": None},
        "joke:cat": {"id": "joke:cat", "kind": "idea", "text": "An app for dating my cat", "title": "Dating my cat",
                     "date": "2025-04-04", "joke_id": "joke:cat", "stage_line": "idea", "nights": 0, "set_id": None, "start_ms": None, "venue": None, "rating": None},
    }
    c = SimpleNamespace(jokes={"joke:main": {"title": "Parking meters"}, "joke:note": {"title": "Meter that talks"}, "joke:cat": {"title": "Dating my cat"}},
                        sets={}, metas={}, segments={}, nodes={})
    c.title = lambda j: c.jokes[j]["title"]
    c.__dict__["_ask_sources"] = S
    return c


def test_pinned_sources_lead_and_the_prompt_names_the_bit():
    c = fake_corpus()
    pin = AC.pinned(c, "joke:main")
    assert pin == {"unit:u1", "seg:s1#4"} and AC.pinned(c, None) == set()
    ids = AC.retrieve(c, "is the cat thing like this?", None, pin=pin)
    assert ids[:2] == sorted(pin, key=lambda k: k) or set(ids[:2]) == pin   # the bit's own sources come first
    assert "joke:cat" in ids                                               # the rest of the notebook still comes
    msgs = AC.prompt(c, "is the cat thing like this?", None, ids, scope=AC.scope_line(c, "joke:main"))
    body = msgs[-1]["content"]
    assert body.startswith("THIS CONVERSATION IS ABOUT HIS BIT: Parking meters.") and "QUESTION: is the cat thing like this?" in body
    plain = AC.prompt(c, "q", None, ids)[-1]["content"]
    assert not plain.startswith("THIS CONVERSATION") and AC.scope_line(c, "joke:nope") is None
