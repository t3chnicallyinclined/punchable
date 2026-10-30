"""Simple Punchable engine (creative/notebook.py, creative/write.py) on the synthetic corpus. No private material."""
from __future__ import annotations

import json
import re

import pytest

from creative import notebook as N
from creative import story as S
from creative import write as WR
from creative.data import Corpus

from .conftest import FakeLLM, HAS_REAL, make_analyses, make_graph

BANNED = re.compile(r"\b(edges?|inferred|proposed|vs_room|confidence|pipeline|units?|x the room)\b|\d\.\d+x", re.I)
SHOWN = ("why", "words", "stage_line", "status_words", "title", "text")


def shown_strings(obj):
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k in SHOWN and isinstance(v, str):
                yield k, v
            yield from shown_strings(v)
    elif isinstance(obj, list):
        for v in obj:
            yield from shown_strings(v)


def quotes(obj):
    if isinstance(obj, dict):
        if obj.get("quote") and obj.get("quote_node"):
            yield obj
        for v in obj.values():
            yield from quotes(v)
    elif isinstance(obj, list):
        for v in obj:
            yield from quotes(v)


@pytest.fixture
def subj():
    """The synthetic corpus plus G: an untaped bit on the same subject as D (laundromat, trampoline...),
    while A is the strongest bit on tape and shares nothing with D."""
    g = make_graph()
    for n in g["nodes"]:
        if n["id"] == "joke:d":
            n["summary"] = "trampoline zipline sandbox birthday cupcakes"
    g["nodes"] += [
        {"id": "joke:g", "type": "joke", "title": "G bit", "stage": "idea", "lead_unit": "unit:g1",
         "anatomy": {"reveal": "trampoline"}},
        {"id": "unit:g1", "type": "unit", "kind": "setup+punch", "date": "2019-05-02",
         "text": "Golf story. My niece at the delta laundromat place with a trampoline, zipline, sandbox and birthday cupcakes."},
    ]
    g["edges"].append({"id": "e:pg", "src": "unit:g1", "dst": "joke:g", "kind": "part_of", "status": "inferred",
                       "confidence": 0.9, "evidence": []})
    return Corpus(g, make_analyses())


def test_stage_words_in_words(corpus):
    line, proven, n = N.stage_words(corpus, "joke:a")
    assert proven and n == 3
    assert re.fullmatch(r"(Killed|Worked) on stage, (Jan|Feb|Mar) 2024 · 3 nights", line), line
    assert N.stage_words(corpus, "joke:f") == (None, False, 0)
    assert N.month_year("2015-06-26") == "Jun 2015" and N.month_year("2015") == "2015"


def test_related_his_words_first_verbatim_and_proven_first(corpus):
    r = N.related(corpus, "my niece and the laundromat")
    ids = [it["joke_id"] for it in r["items"]]
    assert ids and ids[0] == "joke:d"  # d is on tape (night 2) and shares the most words
    assert "joke:e" in ids
    for it in r["items"]:
        assert it["quote"] in corpus.text_of(it["quote_node"]), it
        assert it["date"] is None or re.match(r"\d{4}", it["date"])
    # proven on stage floats above a closer written-only bit when both are in the top five
    r2 = N.related(corpus, "niece soccer schedule laundromat")
    top = [it for it in r2["items"] if it["rank"] < 5]
    seen_written = False
    for it in top:
        if not it["proven"]:
            seen_written = True
        assert not (seen_written and it["proven"]), [x["joke_id"] for x in top]


def test_related_quotes_the_sentence_that_shares_the_idea(corpus):
    r = N.related(corpus, "purple monkey dishwasher")
    b = next(it for it in r["items"] if it["joke_id"] == "joke:b")
    assert "purple monkey dishwasher" in b["quote"]


def test_related_his_own_idea_is_found(corpus):
    g = make_graph()
    c = Corpus(g, make_analyses(), decisions=[{"target": "node", "id": "joke:zz-idea", "action": "add", "by": "owner",
                                               "after": {"type": "joke", "title": "Robot waiter", "stage": "idea",
                                                         "summary": "A robot waiter that guesses your order wrong on purpose."}}])
    r = N.related(c, "robot waiter")
    assert r["items"][0]["joke_id"] == "joke:zz-idea"
    assert r["items"][0]["quote"].startswith("A robot waiter")


def test_same_subject_beats_strong_on_tape(subj):
    """the comic's rule: an old bit on the same subject comes before a bit that is merely strong on tape."""
    m = N.joke_map(subj, "joke:d")
    nxt = [it["joke_id"] for it in next(g for g in m["groups"] if g["key"] == "next")["items"]]
    assert "joke:g" in nxt
    if "joke:a" in nxt:
        assert nxt.index("joke:g") < nxt.index("joke:a"), nxt
    world = next(g for g in m["groups"] if g["key"] == "same_world")["items"]
    assert world[0]["joke_id"] == "joke:g" and world[0]["why"].startswith("Same subject"), world
    bn = [x["joke_id"] for x in S.best_next(subj, {"joke_ids": ["joke:d"]}, cap=5)]
    assert bn[0] == "joke:g", bn


def test_joke_map_groups_plain_and_verbatim(subj):
    m = N.joke_map(subj, "joke:b")
    assert [g["key"] for g in m["groups"]] == ["next", "tags", "same_world", "callbacks", "versions"]
    assert m["joke"]["title"] == "B bit" and m["joke"]["text"]
    nxt = next(g for g in m["groups"] if g["key"] == "next")["items"]
    assert any(it["joke_id"] == "joke:c" for it in nxt)  # the segue b -> c
    for k, v in shown_strings(m):
        assert not BANNED.search(v), (k, v)
    for q in quotes(m):
        assert q["quote"] in subj.text_of(q["quote_node"]), q


def test_build_runtime_estimates_weak_transitions(subj):
    board = {"id": "sb:t", "title": "T", "joke_ids": ["joke:a", "joke:b", "joke:f", "joke:c"], "target_s": 720}
    b = N.build(subj, board)
    assert b["board"]["target_lo_s"] == 600 and b["board"]["target_s"] == 720
    assert abs(b["runtime_s"] - sum(subj.duration(j)[0] for j in board["joke_ids"])) < 0.01
    assert b["estimated_n"] == 1 and b["steps"][2]["estimated"] and not b["steps"][0]["estimated"]
    assert b["steps"][0]["transition_in"] is None
    assert b["steps"][1]["transition_in"]["words"] == "Done back to back on stage" and not b["steps"][1]["transition_in"]["weak"]
    assert b["steps"][2]["transition_in"] == {"words": "No link yet", "weak": True, "kind": "jump",
                                              "from": "joke:b", "to": "joke:f"}  # b -> f: nothing links them
    assert b["weak_n"] == 1
    assert b["status"] == "short" and b["status_words"].endswith("to fill")
    assert all(x["joke_id"] not in board["joke_ids"] for x in b["next"])
    for k, v in shown_strings(b):
        assert not BANNED.search(v), (k, v)
    assert N.target_band(300) == (240, 300)


def test_check_facts_keeps_only_sourced_and_supported():
    llm = FakeLLM({"facts": [
        {"fact": "A language model predicts the next word from patterns in its training text.", "source_title": "Ref",
         "source_url": "https://example.org/lm"},
        {"fact": "Moon cheese is mostly cheddar.", "source_title": "Made up", "source_url": "https://example.org/moon"},
        {"fact": "No source here at all.", "source_title": "", "source_url": "not a url"},
    ]})
    pages = {"https://example.org/lm": "a language model predicts the next word using patterns learned from training text",
             "https://example.org/moon": "the moon is rock"}
    r = WR.check_facts("AI guesses what comes next", complete=llm, fetch=pages.get)
    assert r["status"] == "ok" and r["dropped"] == 2 and r["draft"] is True
    assert [f["source_url"] for f in r["facts"]] == ["https://example.org/lm"]

    def boom(*a):
        raise RuntimeError("down")
    e = WR.check_facts("x y z", complete=boom, fetch=pages.get)
    assert e["status"] == "provider_error" and e["facts"] == []


def test_riff_ideas_are_grounded_drafts(corpus):
    llm = FakeLLM({"ideas": [{"line": "My niece runs the laundromat like a nightclub.", "built_on": ["B1", "riff", "ZZ"]}]})
    r = WR.riff_ideas(corpus, "my niece and the laundromat", ["the castle has a bouncer"], complete=llm)
    assert r["status"] == "ok" and r["ideas"][0]["draft"] is True and r["ideas"][0]["built_on"] == ["B1", "riff"]
    prompt = llm.calls[0][1][1]["content"]
    for g in r["grounded_in"]:
        assert g["quote"] in prompt and g["quote"] in corpus.text_of(next(it["quote_node"] for it in
                                                                          N.related(corpus, "my niece and the laundromat\nthe castle has a bouncer", cap=6)["items"]
                                                                          if it["joke_id"] == g["joke_id"]))
    none = WR.riff_ideas(corpus, "zzqx", [], complete=llm)
    assert none["status"] == "no_grounding" and none["ideas"] == []


def test_deterministic(subj):
    a = json.dumps([N.related(subj, "son castle"), N.joke_map(subj, "joke:d"), N.build(subj, {"id": "x", "title": "x", "joke_ids": ["joke:d", "joke:e"], "target_s": 600})], sort_keys=True)
    b = json.dumps([N.related(subj, "son castle"), N.joke_map(subj, "joke:d"), N.build(subj, {"id": "x", "title": "x", "joke_ids": ["joke:d", "joke:e"], "target_s": 600})], sort_keys=True)
    assert a == b


@pytest.mark.skipif(not HAS_REAL, reason="real KB not on this machine")
def test_real_related_and_map_verbatim_plain(real):
    r = N.related(real, "phone autocorrect")
    assert r["items"] and any("google" in (it["quote"] or "").lower() for it in r["items"][:3])
    for q in quotes(r):
        assert q["quote"] in real.text_of(q["quote_node"]), q
    jid = r["items"][0]["joke_id"]
    m = N.joke_map(real, jid)
    for q in quotes(m):
        assert q["quote"] in real.text_of(q["quote_node"]), q
    for k, v in shown_strings(m):
        assert not BANNED.search(v), (k, v)


def test_riff_prompt_shows_whole_bits_with_their_stage_line_and_asks_for_the_move(corpus):
    from creative import voice
    from creative.tests.test_voice import FAKE_PROFILE
    voice.clear()
    msgs, grounds = WR.riff_messages(corpus, "my niece and the laundromat", ["the castle has a bouncer"])
    assert [m["role"] for m in msgs] == ["system", "user"] and msgs[0]["content"] == WR.RIFF_SYSTEM
    user = msgs[1]["content"]
    assert user.startswith("His idea: ")  # PK_COMIC_NAME unset: never "the comic's idea"
    d = next(g for g in grounds if g["joke_id"] == "joke:d")
    assert d["text"] == "Delta bit about my niece and the laundromat place." and d["text"] in user, "the whole lead text"
    assert d["stage_line"] and d["stage_line"] in user and d["last_night"] and d["last_night"] in user
    f = [g for g in grounds if g["joke_id"] == "joke:f"]
    assert not f or "not on tape" in user
    assert len(grounds) <= WR.RIFF_BITS and all(len(g["text"]) <= WR.RIFF_BIT_CHARS + 6 for g in grounds)
    assert "name the move in `move`" in user and "No new facts about his life" in user
    assert "move" in WR.RIFF_SCHEMA["properties"]["ideas"]["items"]["required"]
    # with the profile loaded (the worker's copy) it is the FIRST block; the task line is second; the user turn last
    voice.set_profile(FAKE_PROFILE, "2026-10-28T00:00:00+00:00", "owner")
    try:
        msgs2, _ = WR.riff_messages(corpus, "my niece and the laundromat", [])
        assert [m["role"] for m in msgs2] == ["system", "system", "user"]
        assert FAKE_PROFILE in msgs2[0]["content"] and msgs2[0]["content"].endswith(voice.CRAFT)
        assert msgs2[1]["content"] == WR.RIFF_SYSTEM
        llm = FakeLLM({"ideas": [{"line": "A line.", "built_on": ["B1"], "move": "poll the room"}]})
        r = WR.riff_ideas(corpus, "my niece and the laundromat", [], complete=llm)
        assert r["ideas"][0]["move"] == "poll the room" and llm.calls[0][1][0]["content"].startswith(voice.HEAD)
    finally:
        voice.clear()


def test_a_refusal_reaches_the_screen_in_plain_words(corpus):
    from engine.providers import CutShort, Refusal

    def declined(task, messages, schema):
        raise Refusal("general_harms", "it names a real person")
    r = WR.riff_ideas(corpus, "my niece and the laundromat", [], complete=declined)
    assert r["status"] == "provider_error" and r["why"] == "Claude declined this one: it names a real person. Try rewording."
    assert "ProviderError" not in r["why"] and "Refusal" not in r["why"]

    def cut(task, messages, schema):
        raise CutShort(16000)
    r = WR.check_facts("airports and belts", complete=cut, fetch=lambda u: None)
    assert r["status"] == "provider_error" and r["why"].startswith("Claude ran out of room on this one")

    def down(task, messages, schema):
        raise RuntimeError("connection reset")
    r = WR.riff_ideas(corpus, "my niece and the laundromat", [], complete=down)
    assert r["why"] == "RuntimeError: connection reset"  # anything else still names itself, short
