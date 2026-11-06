"""Film it's two sessions (creative/rant.py): "Make it a rant" hunks and "Pick a hook" openers, checked before
anything is shown. Every joke here is invented. No model, no network: a fake completion returns the data.

    python -m pytest creative/tests/test_rant.py -q
"""
from __future__ import annotations

from types import SimpleNamespace

import pytest

from creative import rant as RT
from creative import voice

LINES = ["Self-checkout is the store hiring you for free.",
         "I scanned forty things and nobody clapped.",
         "At my job I at least get a chair.",
         "Unexpected item in the bagging area: my dignity."]
INP = {"joke_id": "joke:self-checkout", "title": "Self-checkout", "unit": "unit:abc", "cut": "stage",
       "lines": LINES, "punch": 3}


@pytest.fixture(autouse=True)
def no_profile():
    voice.clear()
    yield
    voice.clear()


def fake(data, calls=None):
    def complete(task, messages, schema):
        if calls is not None:
            calls.append((task, messages, schema))
        return SimpleNamespace(data=data, provider="anthropic", model="claude-test", call_id="c1")
    return complete


def test_a_hunk_must_quote_his_line_word_for_word():
    hunks, dropped = RT.rant_hunks({"hunks": [
        {"op": "change", "line_no": 1, "was": "Self-checkout is the store hiring you for free.",
         "now": "You know what self-checkout is? It's the store hiring you. For free.", "why": "direct address"},
        # a paraphrase of line 2 is not his line: dropped, never shown
        {"op": "change", "line_no": 2, "was": "I scanned 40 things and no one clapped.",
         "now": "You scan forty things. Nobody claps.", "why": "present tense"},
        # whitespace aside, the quote is the line
        {"op": "cut", "line_no": 3, "was": "At my job  I at least get a chair. ", "now": "ignored", "why": "tighter"},
        {"op": "add", "line_no": 0, "was": "", "now": "Stop scanning your own groceries.", "why": "hook"},
    ]}, LINES, 3)
    assert [h["op"] for h in hunks] == ["change", "cut", "add"] and dropped == 1
    assert hunks[0]["was"] == LINES[0] and hunks[1]["was"] == LINES[2] and hunks[1]["now"] == ""
    assert hunks[2] == {"op": "add", "line_no": 0, "was": "", "now": "Stop scanning your own groceries.", "why": "hook"}


def test_the_punch_keeps_its_reveal_word_and_is_never_cut():
    keep = {"op": "change", "line_no": 4, "was": LINES[3], "now": "\"Unexpected item in the bagging area.\" Yeah. It's my dignity.", "why": "beat"}
    lose = {"op": "change", "line_no": 4, "was": LINES[3], "now": "Unexpected item in the bagging area: my pride.", "why": "x"}
    cut = {"op": "cut", "line_no": 4, "was": LINES[3], "now": "", "why": "x"}
    assert RT.rant_hunks({"hunks": [keep]}, LINES, 3) == ([{**keep, "was": LINES[3]}], 0)
    assert RT.rant_hunks({"hunks": [lose]}, LINES, 3) == ([], 1)
    assert RT.rant_hunks({"hunks": [cut]}, LINES, 3) == ([], 1)
    # with no punch marked nothing is protected
    assert len(RT.rant_hunks({"hunks": [lose, ]}, LINES, None)[0]) == 1


def test_one_hunk_per_line_and_nothing_out_of_range_or_unchanged():
    data = {"hunks": [
        {"op": "change", "line_no": 2, "was": LINES[1], "now": "You scan forty things. Nobody claps.", "why": "a"},
        {"op": "cut", "line_no": 2, "was": LINES[1], "now": "", "why": "second on the same line"},
        {"op": "change", "line_no": 9, "was": "x", "now": "y", "why": "out of range"},
        {"op": "add", "line_no": 5, "was": "", "now": "past the end", "why": "out of range"},
        {"op": "change", "line_no": 3, "was": LINES[2], "now": "at my job, I at least get a chair", "why": "same words"},
        {"op": "change", "line_no": True, "was": LINES[0], "now": "a bool is not a line", "why": "x"},
        {"op": "move", "line_no": 1, "was": LINES[0], "now": "x", "why": "no such op"},
        {"op": "change", "line_no": 1, "was": LINES[0], "now": "x" * 401, "why": "too long"},
        "not a hunk",
    ]}
    hunks, dropped = RT.rant_hunks(data, LINES, 3)
    assert [(h["op"], h["line_no"]) for h in hunks] == [("change", 2)] and dropped == 8


def test_rant_runs_with_the_voice_blocks_first_and_says_none_when_nothing_survives():
    voice.set_profile("He talks fast. " * 200, "2026-11-01")
    voice.set_learned({"kept": [{"text": "Kept line."}], "dropped": [], "updated_at": "2026-11-02"})
    calls = []
    r = RT.rant(INP, complete=fake({"hunks": [{"op": "change", "line_no": 2, "was": LINES[1],
                                                "now": "You scan forty things. Nobody claps.", "why": "tense"}]}, calls))
    assert r["status"] == "ok" and r["task"] == "creative_rant" and r["lines"] == LINES and r["unit"] == "unit:abc"
    assert r["hunks"][0]["draft"] is True and r["hunks"][0]["label"] == RT.DRAFT_LABEL
    task, msgs, schema = calls[0]
    assert task == "creative_rant" and schema is RT.RANT_SCHEMA
    assert [m["role"] for m in msgs] == ["system", "system", "system", "user"]
    assert msgs[0]["content"].startswith(voice.HEAD) and "Kept line." in msgs[1]["content"]
    assert msgs[2]["content"].startswith(RT.RANT_SYSTEM) and "Second person" in msgs[2]["content"]
    assert "4. Unexpected item in the bagging area: my dignity.   <- the punch" in msgs[3]["content"]
    none = RT.rant(INP, complete=fake({"hunks": [{"op": "change", "line_no": 2, "was": "not it", "now": "x", "why": "x"}]}))
    assert none["status"] == "none" and none["hunks"] == [] and none["dropped"] == 1

    def boom(*a):
        raise RuntimeError("down")
    assert RT.rant(INP, complete=boom)["status"] == "provider_error"
    assert RT.rant({**INP, "lines": []}, complete=boom)["status"] == "empty"


def test_hooks_are_three_new_lines_never_his_own_or_ones_he_saw():
    data = {"hooks": [
        {"text": "Stop scanning your own groceries.", "why": "a claim"},
        {"text": "self-checkout is the store hiring you for free", "why": "a line of the bit"},
        {"text": "I want to talk to whoever invented self-checkout.", "why": "complaint"},
        {"text": "I got fired today. From a store I don't work at.", "why": "confession"},
        {"text": "stop scanning your own groceries!", "why": "a repeat"},
        {"text": " ".join(["word"] * 25), "why": "too long"},
        {"text": "One more good one.", "why": "past three"},
    ]}
    rows, dropped = RT.hook_rows(data, LINES, ["I want to talk to whoever invented self-checkout."])
    assert [h["text"] for h in rows] == ["Stop scanning your own groceries.",
                                         "I got fired today. From a store I don't work at.",
                                         "One more good one."]
    assert dropped == 4 and all(h["draft"] for h in rows)
    calls = []
    r = RT.hooks({**INP, "exclude": ["Old one."]}, complete=fake(data, calls))
    assert r["status"] == "ok" and len(r["hooks"]) == 3 and r["exclude"] == ["Old one."]
    assert calls[0][0] == "creative_hook" and "- Old one." in calls[0][1][-1]["content"]
    assert "Show others" not in calls[0][1][-1]["content"]


def test_the_worker_runs_both_kinds_and_the_hosted_worker_asks_for_them():
    from worker import execute
    from worker import __main__ as WM
    r = execute.run({"kind": "rant", "input": INP}, complete=fake({"hunks": []}))
    assert r["status"] == "none" and r["task"] == "creative_rant"
    h = execute.run({"kind": "hook", "input": INP}, complete=fake({"hooks": [{"text": "A new opener.", "why": "x"}]}))
    assert h["status"] == "ok" and h["hooks"][0]["text"] == "A new opener."
    assert {"rant", "hook"} <= set(WM.hosted_kinds(False)) and {"rant", "hook"} <= set(WM.hosted_kinds(True))


@pytest.mark.parametrize("planted", ["was", "reveal"])
def test_red_arm_a_lenient_check_would_let_a_bad_hunk_through(planted, monkeypatch):
    """The gate can fail: with the quote check (or the reveal-word check) made lenient, the bad hunk survives and the
    assertion that guards it fails. `--red` style: we assert the planted rule DOES let it through."""
    bad = ({"op": "change", "line_no": 2, "was": "I scanned 40 things", "now": "You scan.", "why": "x"} if planted == "was"
           else {"op": "change", "line_no": 4, "was": LINES[3], "now": "Bagging area: my pride.", "why": "x"})
    if planted == "was":
        monkeypatch.setattr(RT, "same", lambda a, b: True)
    else:
        monkeypatch.setattr(RT, "reveal_word", lambda line: None)
    hunks, _ = RT.rant_hunks({"hunks": [bad]}, LINES, 3)
    assert len(hunks) == 1, "the lenient rule must let the bad hunk through (proves the real check is what stops it)"
