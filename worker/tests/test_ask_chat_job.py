"""The home worker's half of Ask: execute.run(kind ask_chat) makes exactly one model call with askchat's own schema."""
from __future__ import annotations

from creative.askchat import SCHEMA
from worker import execute


class R:
    data = {"found": True, "answer": "x [S1]", "citations": [{"source": "S1", "quote": "y"}]}
    provider, model = "fake", "fake-1"


def test_ask_chat_job_runs_one_call_and_returns_the_raw_answer():
    calls = []

    def complete(task, messages, schema):
        calls.append((task, messages, schema))
        return R()
    msgs = [{"role": "system", "content": "s"}, {"role": "user", "content": "q"}]
    out = execute.run({"kind": "ask_chat", "input": {"messages": msgs}}, complete=complete)
    assert len(calls) == 1 and calls[0][0] == "ask_chat" and calls[0][2] is SCHEMA and calls[0][1] == msgs
    assert out == {"status": "answered", "data": R.data, "provider": "fake", "model": "fake-1"}


def test_ask_chat_job_refuses_bad_input():
    try:
        execute.run({"kind": "ask_chat", "input": {"messages": [{"role": "user", "content": 3}]}}, complete=lambda *a: R())
    except ValueError:
        return
    raise AssertionError("bad messages accepted")
