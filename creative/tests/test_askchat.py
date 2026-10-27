"""Ask (creative/askchat.py): the gate must be able to fail. Synthetic corpus only (conftest)."""
from __future__ import annotations

import re

from creative import askchat as A

from .conftest import TEXT, FakeLLM


def sid_of(msgs, needle):
    """The S-number the prompt gave the source whose header or text contains `needle`."""
    body = msgs[-1]["content"]
    for m in re.finditer(r"\[(S\d+)\] ([^\n]*)\nTEXT: ([^\n]*)", body):
        if needle in m.group(2) or needle in m.group(3):
            return m.group(1)
    raise AssertionError(f"{needle!r} not in the prompt")


def test_verbatim_rule():
    src = "I said “no” — and   left. It’s fine."
    assert A.verbatim('I said "no" - and left.', src) == "I said “no” — and   left."
    assert A.verbatim("It's fine", src) == "It’s fine"
    assert A.verbatim("it's fine", src) is None          # case is a word change
    assert A.verbatim("I said no and left", src) is None  # dropped punctuation is not typography
    assert A.verbatim("I said yes", src) is None


def test_real_quote_kept_fabricated_dropped(corpus):
    def pay(msgs):
        s = sid_of(msgs, "purple monkey")
        return {"found": True, "answer": f"You wrote about a dog and a dishwasher [{s}].",
                "citations": [{"source": s, "quote": "The purple monkey dishwasher line here."},
                              {"source": s, "quote": "The purple monkey washed the dishes."}]}
    r = A.ask_local(corpus, "what did I write about the purple monkey dishwasher?", complete=FakeLLM(pay))
    assert r["found"] and r["chips"]
    ch = r["chips"][0]
    assert ch["quote"] == "The purple monkey dishwasher line here."
    assert ch["quote"] in (corpus.text_of(ch["source"]) or "")
    assert any(d["why"] == "not word for word in the source" for d in r["dropped"])
    assert "[1]" in r["answer"] and "[S" not in r["answer"]


def test_only_fabricated_quote_leaves_chip_without_quote(corpus):
    def pay(msgs):
        s = sid_of(msgs, "purple monkey")
        return {"found": True, "answer": f"The dishwasher bit [{s}].",
                "citations": [{"source": s, "quote": "purple monkey dishwasher LINE HERE"}]}
    r = A.ask_local(corpus, "purple monkey dishwasher", complete=FakeLLM(pay))
    assert r["found"] and r["chips"][0]["quote"] is None
    assert len(r["dropped"]) == 1


def test_unretrieved_source_and_marker_cut(corpus):
    def pay(msgs):
        return {"found": True, "answer": "It killed [S99] everywhere [S1].",
                "citations": [{"source": "S99", "quote": "anything"}, {"source": "S1", "quote": ""}]}
    r = A.ask_local(corpus, "purple monkey dishwasher", complete=FakeLLM(pay))
    assert "S99" not in r["answer"] and "[99]" not in r["answer"]
    assert [c["n"] for c in r["chips"]] == [1]
    assert any("not a retrieved source" in d["why"] for d in r["dropped"])


def test_inline_quote_not_in_sources_is_removed(corpus):
    def pay(msgs):
        s = sid_of(msgs, "purple monkey")
        return {"found": True, "answer": f'You said "the monkey ate all of my dishes" once [{s}].',
                "citations": [{"source": s, "quote": "Bravo closer words."}]}
    r = A.ask_local(corpus, "purple monkey dishwasher", complete=FakeLLM(pay))
    assert "the monkey ate all of my dishes" not in r["answer"]
    assert "[quote removed]" in r["answer"]


def test_refusal_when_nothing_retrieved_makes_no_model_call(corpus):
    llm = FakeLLM({"found": True, "answer": "made up [S1]", "citations": []})
    r = A.ask_local(corpus, "zeppelins over antarctica", complete=llm)
    assert not llm.calls
    assert not r["found"] and r["answer"] == A.REFUSAL and r["chips"] == []


def test_refusal_when_model_cannot_ground_it(corpus):
    r = A.ask_local(corpus, "purple monkey dishwasher", complete=FakeLLM({"found": False, "answer": "", "citations": []}))
    assert r["answer"] == A.REFUSAL and not r["chips"]
    # found=true but nothing it cites was retrieved: still a refusal, never a guess
    r = A.ask_local(corpus, "purple monkey dishwasher",
                    complete=FakeLLM({"found": True, "answer": "It killed in Boston.", "citations": [{"source": "S40", "quote": "x"}]}))
    assert r["answer"] == A.REFUSAL and not r["chips"] and not r["found"]


def test_citations_resolve_to_a_joke_map_or_a_set_moment(corpus):
    def pay(msgs):
        a = sid_of(msgs, "at 0:00")  # a set moment
        b = sid_of(msgs, "written bit")
        return {"found": True, "answer": f"On stage [{a}] and on paper [{b}].",
                "citations": [{"source": a, "quote": ""}, {"source": b, "quote": ""}]}
    r = A.ask_local(corpus, "alpha opener punch", complete=FakeLLM(pay))
    assert len(r["chips"]) == 2
    for ch in r["chips"]:
        if ch["kind"] == "moment":
            m = re.fullmatch(r"/set/(set:[^?]+)\?at=(\d+)", ch["href"])
            assert m and m.group(1)[4:] in corpus.sets
            assert any(s["start_ms"] == int(m.group(2)) for s in corpus.segments[m.group(1)[4:]])
            assert re.search(r"\d+:\d\d$", ch["label"])
        else:
            m = re.fullmatch(r"/joke/(joke:[^/]+)", ch["href"])
            assert m and m.group(1) in corpus.jokes


def test_follow_up_carries_the_last_answers_sources(corpus):
    first = A.retrieve(corpus, "the purple monkey dishwasher")
    bravo = [x for x in first if A.sources(corpus)[x].get("joke_id") == "joke:b" and A.sources(corpus)[x]["kind"] == "bit"]
    assert bravo
    # the earlier question shares no words with it: only carrying the cited source can bring it back
    hist = [{"q": "zebra quokka", "answer": "The Bravo bit [1].", "cited": bravo[:1]}]
    ids = A.retrieve(corpus, "which of those killed?", hist)
    assert bravo[0] in ids
    # its nights on tape come along, so "killed" can be answered from the laughs
    assert any(A.sources(corpus)[x]["kind"] == "moment" and A.sources(corpus)[x]["joke_id"] == "joke:b" for x in ids)
    msgs = A.prompt(corpus, "which of those killed?", hist, ids)
    assert [m["role"] for m in msgs[:3]] == ["system", "user", "assistant"]
    assert "not evidence" in msgs[0]["content"]


def test_quotes_only_from_text_never_from_the_header(corpus):
    def pay(msgs):
        s = sid_of(msgs, "at 0:00")
        return {"found": True, "answer": f"It went fine [{s}].", "citations": [{"source": s, "quote": "set moment"}]}
    r = A.ask_local(corpus, "alpha opener punch", complete=FakeLLM(pay))
    assert all(ch["quote"] is None for ch in r["chips"])


def test_run_job_rejects_bad_input_and_uses_its_own_schema():
    seen = {}

    def llm(task, messages, schema):
        seen["task"], seen["schema"] = task, schema
        return type("R", (), {"data": {"found": False, "answer": "", "citations": []}, "provider": "fake", "model": "f"})()
    out = A.run_job({"messages": [{"role": "user", "content": "hi"}], "schema": {"evil": True}}, complete=llm)
    assert seen == {"task": "ask_chat", "schema": A.SCHEMA} and out["provider"] == "fake"
    for bad in ({}, {"messages": "x"}, {"messages": [{"role": "tool", "content": "x"}]}):
        try:
            A.run_job(bad, complete=llm)
        except ValueError:
            continue
        raise AssertionError("bad input accepted")


def test_copy_has_no_jargon(corpus):
    S = A.sources(corpus)
    banned = re.compile(r"\b(edges?|inferred|proposed|vs_room|confidence|pipeline|units?)\b|\d\.\d+x", re.I)
    for i, s in enumerate(S.values(), 1):
        ch = A.chip(s, i, None)
        assert not banned.search(ch["label"] + " " + ch["sub"]), ch
    assert TEXT["b"]  # the fixture is the synthetic one
