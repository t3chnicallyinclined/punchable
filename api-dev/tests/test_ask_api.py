"""Ask routes (api-dev/ask_api.py, docs/API_CONTRACT.md "Ask") end to end over HTTP, on TEMP copies of the data.
This test plays the home worker (POST /api/worker/claim → result), so no model runs.

    python -m pytest api-dev/tests/test_ask_api.py -q
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from urllib.parse import quote

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "db" / "tests"))
sys.path.insert(0, str(DESK / "api-dev"))
sys.path.insert(0, str(DESK))
from common import Api, TempKB  # noqa: E402

from creative.askchat import REFUSAL  # noqa: E402

FAKE = "this line was never written by anyone at all"


@pytest.fixture(scope="module")
def api():
    t = TempKB("ask", decisions=False)
    a = Api(t, "files")
    yield a, t
    a.stop()
    t.cleanup()


def sources_in(job):
    body = job["input"]["messages"][-1]["content"]
    return [(m.group(1), m.group(2), m.group(3)) for m in re.finditer(r"\[(S\d+)\] ([^\n]*)\nTEXT: ([^\n]*)", body)]


def claim(a):
    s, r = a.post("/api/worker/claim", {"kinds": ["ask_chat"], "worker": "test"})
    assert s == 200, (s, r)
    return r["job"]


def answer_with(a, job, data, ok=True):
    body = {"ok": True, "result": {"data": data, "provider": "fake", "model": "fake-1"}} if ok else \
        {"ok": False, "error": "ProviderError: claude_code failed rc=1: boom"}
    s, r = a.post(f"/api/worker/jobs/{quote(job['id'], safe='')}/result", body)
    assert s == 200, r


def real_quote(text):
    t = text.removeprefix("[...] ")
    words = t.split()
    return " ".join(words[:8])


def test_offline_then_answered_with_gated_quotes_and_resolving_chips(api):
    a, t = api
    s, ch = a.post("/api/ask/chats", {"question": "what have I written about AI?"})
    assert s == 201 and ch["id"].startswith("chat:")
    assert ch["turns"][0]["status"] == "thinking"
    assert ch["worker"]["online"] is False          # no worker has polled yet: the app says the PC is offline
    job = claim(a)
    assert job["kind"] == "ask_chat"
    srcs = sources_in(job)
    assert len(srcs) >= 2
    (s1, _, t1), (s2, _, _) = srcs[0], srcs[1]
    answer_with(a, job, {"found": True, "answer": f"You have a few AI bits [{s1}] and more [{s2}] and [S77].",
                         "citations": [{"source": s1, "quote": real_quote(t1)}, {"source": s2, "quote": FAKE}]})
    s, ch = a.get(f"/api/ask/chats/{quote(ch['id'], safe='')}")
    assert s == 200
    turn = ch["turns"][0]
    assert ch["worker"]["online"] is True
    assert turn["status"] == "done" and turn["found"]
    assert "[S" not in turn["answer"] and "[1]" in turn["answer"] and "[2]" in turn["answer"]
    assert turn["dropped_n"] >= 1
    assert [c["n"] for c in turn["chips"]] == [1, 2]
    assert turn["chips"][0]["quote"] == real_quote(t1)
    assert turn["chips"][1]["quote"] is None             # the made-up line is gone
    assert FAKE not in json.dumps(ch)
    for c in turn["chips"]:                               # every chip opens something that exists
        m = re.fullmatch(r"/joke/(.+)", c["href"]) or re.fullmatch(r"/set/([^?]+)\?at=\d+", c["href"])
        assert m, c["href"]
        path = f"/api/jokes/{quote(m.group(1), safe=':')}" if c["href"].startswith("/joke/") else f"/api/sets/{quote(m.group(1), safe=':')}"
        assert a.get(path)[0] == 200, path
    # stored: the conversation is in chats.jsonl (his data), the list shows it
    lines = [json.loads(x) for x in (t.creative / "chats.jsonl").read_text(encoding="utf-8").splitlines() if x.strip()]
    assert [x["op"] for x in lines][:3] == ["new", "ask", "answer"]
    s, lst = a.get("/api/ask/chats")
    assert s == 200 and lst["chats"][0]["id"] == ch["id"] and lst["chats"][0]["questions"] == 1


def test_follow_up_carries_the_conversation(api):
    a, _ = api
    s, ch = a.post("/api/ask/chats", {"question": "which bits killed at The Corner Room?"})
    job = claim(a)
    srcs = sources_in(job)
    assert srcs and all("The Corner Room" in h for _, h, _ in srcs)      # the room named pulls that room's sets
    s1, _, t1 = srcs[0]
    answer_with(a, job, {"found": True, "answer": f"This one [{s1}].", "citations": [{"source": s1, "quote": real_quote(t1)}]})
    cid = quote(ch["id"], safe="")
    s, ch = a.get(f"/api/ask/chats/{cid}")
    assert ch["turns"][0]["status"] == "done"
    s, ch = a.post(f"/api/ask/chats/{cid}", {"question": "which of those was earliest?"})
    assert s == 200 and ch["turns"][1]["status"] == "thinking"
    s, again = a.post(f"/api/ask/chats/{cid}", {"question": "and another?"})
    assert s == 409                                             # one question at a time
    job = claim(a)
    roles = [m["role"] for m in job["input"]["messages"]]
    assert roles[:3] == ["system", "user", "assistant"]
    assert job["input"]["messages"][1]["content"] == "which bits killed at The Corner Room?"
    assert any(h for _, h, _ in sources_in(job))
    answer_with(a, job, None, ok=False)
    s, ch = a.get(f"/api/ask/chats/{cid}")
    t2 = ch["turns"][1]
    assert t2["status"] == "failed" and "rc=1" not in t2["error"] and "PC" in t2["error"]


def test_refusal_when_nothing_is_found_needs_no_worker(api):
    a, _ = api
    s, ch = a.post("/api/ask/chats", {"question": "zeppelin quokka xylophone?"})
    assert s == 201
    turn = ch["turns"][0]
    assert turn["status"] == "done" and not turn["found"] and turn["answer"] == REFUSAL and turn["chips"] == []
    import urllib.request
    req = urllib.request.Request(a.url + "/api/worker/claim", data=json.dumps({"kinds": ["ask_chat"]}).encode(),
                                 method="POST", headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        assert r.status == 204                                    # no job was queued


def test_model_says_found_but_cites_nothing_real_is_a_refusal(api):
    a, _ = api
    s, ch = a.post("/api/ask/chats", {"question": "what have I written about my niece?"})
    job = claim(a)
    answer_with(a, job, {"found": True, "answer": "You killed with it in Boston in 2019.",
                         "citations": [{"source": "S99", "quote": FAKE}]})
    s, ch = a.get(f"/api/ask/chats/{quote(ch['id'], safe='')}")
    turn = ch["turns"][0]
    assert turn["answer"] == REFUSAL and not turn["found"] and not turn["chips"]


def test_rename_hide_and_bad_input(api):
    a, _ = api
    s, lst = a.get("/api/ask/chats")
    cid = lst["chats"][0]["id"]
    assert a.post(f"/api/ask/chats/{quote(cid, safe='')}", {"title": "AI stuff"}, method="PATCH")[1]["title"] == "AI stuff"
    assert a.post(f"/api/ask/chats/{quote(cid, safe='')}", {"deleted": True}, method="PATCH")[0] == 200
    assert cid not in [c["id"] for c in a.get("/api/ask/chats")[1]["chats"]]
    assert a.get(f"/api/ask/chats/{quote(cid, safe='')}")[0] == 404
    assert a.post("/api/ask/chats", {"question": ""})[0] == 400
    assert a.post("/api/ask/chats", {"question": "x" * 1001})[0] == 400
    assert a.get("/api/ask/chats/chat:nothere1")[0] == 404
    assert a.get("/api/ask/chats/..%2Fetc")[0] == 404
