"""Home worker: decision pull (append-only, dedupe, undo remap), creative pull, job execution with a mock model,
and the client-credentials token flow. No network, no real secrets, no real data.

    python -m pytest worker/tests -q
"""
from __future__ import annotations

import io
import json
import sys
from pathlib import Path

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK))
import worker.__main__ as WM  # noqa: E402


def d(i, **kw):
    return {"ts": f"2026-10-26T00:00:{i:02d}+00:00", "target": "node", "id": f"joke:j{i}", "action": "confirm", "by": "owner", **kw}


def test_merge_dedupes_by_content_and_keeps_order():
    local = [d(0), d(1), d(2)]
    server = [d(0), d(1), {**d(5)}]                 # the phone added line 5 after the last load
    new, warn = WM.merge_decisions(local, server)
    assert new == [d(5)] and not warn
    new2, _ = WM.merge_decisions(local + new, server)
    assert new2 == []                               # idempotent


def test_merge_key_order_does_not_matter():
    local = [d(0)]
    server = [dict(reversed(list(d(0).items())))]
    assert WM.merge_decisions(local, server)[0] == []


def test_merge_remaps_undo_of():
    local = [d(0), d(1), d(9)]                      # line 2 was made on the PC after the last load
    server = [d(0), d(1), d(5), {**d(6), "undo_of": 2}]  # the phone: line 2 = d(5), then an undo of it
    new, warn = WM.merge_decisions(local, server)
    assert not warn
    assert new[0] == d(5)
    assert new[1]["undo_of"] == 3                   # d(5) lands at local line 3


def test_merge_skips_undo_without_target():
    new, warn = WM.merge_decisions([d(0)], [d(0), {**d(1), "undo_of": 7}])
    assert new == [] and warn


def test_append_lines_is_append_only(tmp_path):
    p = tmp_path / "decisions.jsonl"
    p.write_bytes(b'{"a":1}')                       # a last line without its newline
    WM.append_lines(p, ['{"b":2}'])
    assert p.read_bytes() == b'{"a":1}\n{"b":2}\n'


class FakeCl:
    def __init__(self, server_lines):
        self.server = server_lines

    def call(self, method, path, body=None):
        if path.startswith("/api/worker/decisions"):
            after = int(path.split("after=")[1])
            rows = [{"index": i, "decision": x} for i, x in enumerate(self.server)][after:after + 2]
            return {"total": len(self.server), "after": after, "lines": rows}
        if path.startswith("/api/worker/creative"):
            name = path.split("store=")[1]
            return {"store": name, "lines": self.server_creative.get(name, [])}
        raise AssertionError(path)


def test_pull_decisions_pages_and_appends(tmp_path):
    p = tmp_path / "decisions.jsonl"
    p.write_text("\n".join(json.dumps(x) for x in [d(0), d(1)]) + "\n", encoding="utf-8")
    before = p.read_bytes()
    cl = FakeCl([d(0), d(1), d(2), d(3), d(4)])
    assert WM.pull_decisions(cl, p) == 3
    assert p.read_bytes().startswith(before)        # nothing rewritten
    assert WM.pull_decisions(cl, p) == 0
    assert len(WM.read_jsonl(p)) == 5


def ev(op, i, by="owner", **data):
    return json.dumps({"ts": "2026-10-26T00:00:00+00:00", "op": op, "id": i, "by": by, "data": data}, ensure_ascii=False)


def test_pull_creative_prefix_merge_and_conflict(tmp_path):
    a = ev("create", "sb:a", title="A", joke_ids=["joke:x"])
    b = ev("add", "sb:a", joke_id="joke:y")
    c = ev("update", "sb:a", title="A2")
    bad = ev("add", "sb:a", joke_id="joke:x")       # already on the board: does not replay after `a`
    cl = FakeCl([])
    (tmp_path / "storyboards.jsonl").write_text(a + "\n", encoding="utf-8")
    cl.server_creative = {"storyboards": [a, b]}
    assert WM.pull_creative(cl, tmp_path)["storyboards"] == 1                 # server is ahead: append
    (tmp_path / "storyboards.jsonl").write_text(a + "\n" + c + "\n", encoding="utf-8")
    assert WM.pull_creative(cl, tmp_path)["storyboards"] == 1                 # both added: merge, still replays
    assert (tmp_path / "storyboards.jsonl").read_text(encoding="utf-8").splitlines() == [a, c, b]
    cl.server_creative = {"storyboards": [a, bad]}
    assert WM.pull_creative(cl, tmp_path)["storyboards"] is None              # would not replay: left alone
    assert (tmp_path / "storyboards.jsonl").read_text(encoding="utf-8").splitlines() == [a, c, b]


def test_execute_bridge_and_punchup_with_mock_model(monkeypatch):
    """The worker runs the same creative functions; the lock rule still holds with the job's working set."""
    from creative.data import Corpus
    from creative.tests.conftest import make_analyses, make_graph  # synthetic corpus from creative's own tests
    from worker import execute
    c = Corpus(make_graph(), make_analyses())
    monkeypatch.setattr(execute, "corpus", lambda reload=False: c)

    class R:
        def __init__(self, data):
            self.data, self.provider, self.model, self.call_id = data, "mock", "m", "c1"

    seen = []

    def mock(task, messages, schema):
        seen.append(messages[-1]["content"])
        return R({"bridge": "So anyway.", "uses": ["L1"]} if task == "creative_bridge" else
                 {"drafts": [{"line_no": 1, "alt": "a sharper line", "move": "reframe"}]})

    jokes = [j for j in c.nodes if j.startswith("joke:")]
    out = execute.run({"kind": "bridge", "input": {"from": jokes[0], "to": jokes[1]}}, complete=mock)
    assert out["draft"] is True and out["status"] in ("ok", "no_grounding")
    ws = {"id": "ws:t", "locks": {jokes[0]: {"bit": True, "lines": []}}, "versions": [{"v": 1, "bits": []}], "current": 1}
    n = len(seen)
    out = execute.run({"kind": "punchup", "input": {"working_set": ws, "joke_id": jokes[0]}}, complete=mock)
    assert out["status"] == "locked" and len(seen) == n  # a locked bit never reaches the model


def test_execute_unknown_kind():
    from worker import execute
    with pytest.raises(ValueError):
        execute.run({"kind": "rm -rf", "input": {}})


def test_client_credentials_token_and_no_secret_in_logs(capsys):
    calls = []

    class Resp(io.BytesIO):
        status = 200

        def __enter__(self):
            return self

        def __exit__(self, *a):
            return False

    def opener(req, timeout=0):
        calls.append(req)
        if req.full_url.endswith("/token"):
            assert b"grant_type=client_credentials" in req.data and b"client_id=pk-worker" in req.data
            return Resp(json.dumps({"access_token": "tok-123", "expires_in": 300}).encode())
        assert req.headers["Authorization"] == "Bearer tok-123"
        return Resp(b'{"ok": true}')

    cl = WM.Client("https://punchable.example", token_url="https://auth.example/token", secret="s3cr3t-value", opener=opener)
    assert cl.call("GET", "/api/worker/decisions?after=0") == {"ok": True}
    assert cl.call("GET", "/api/worker/decisions?after=0") == {"ok": True}
    assert sum(1 for c in calls if c.full_url.endswith("/token")) == 1          # cached until near expiry
    assert "s3cr3t-value" not in capsys.readouterr().out


def test_no_auth_only_on_loopback():
    with pytest.raises(SystemExit):
        WM.main(["--api", "https://punchable.io", "--no-auth", "--once"])
