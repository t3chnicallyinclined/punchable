"""The hosted worker's /metrics (worker/metrics.py): job counters and durations from run_one, tokens and USD from the
budget's charge hook, the spend gauges at scrape time, the heartbeat age, and the side listener. No network beyond
loopback, no model.

    python -m pytest worker/tests/test_metrics.py -q
"""
from __future__ import annotations

import re
import sys
import urllib.request
from pathlib import Path

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK))
import worker.__main__ as WM  # noqa: E402
import worker.metrics as MX  # noqa: E402
from engine.budget import Budget  # noqa: E402


class ClaimApi:
    def __init__(self, queue=None):
        self.claims, self.results, self.queue = [], [], list(queue or [])

    def call(self, method, path, body=None):
        if path == "/api/worker/claim":
            self.claims.append(body)
            return {"job": self.queue.pop(0)} if self.queue and not body.get("paused") else None
        if path.endswith("/result"):
            self.results.append((path, body))
            return {"ok": True}
        raise AssertionError((method, path))


def test_run_one_counts_claimed_done_failed_and_duration(monkeypatch):
    c0 = MX.jobs_claimed.value(kind="riff")
    d0, f0 = MX.jobs_done.value(kind="riff"), MX.jobs_failed.value(kind="riff", retry="false")
    n0 = MX.job_duration.count(kind="riff")
    api = ClaimApi(queue=[{"id": "job:riff-1", "kind": "riff", "input": {}}, {"id": "job:riff-2", "kind": "riff", "input": {}}])
    monkeypatch.setattr("worker.execute.run", lambda job, **k: {"status": "ok"})
    assert WM.run_one(api, ["riff"], ws=None, hosted=True) is True
    monkeypatch.setattr("worker.execute.run", lambda job, **k: (_ for _ in ()).throw(ValueError("bad input")))
    assert WM.run_one(api, ["riff"], ws=None, hosted=True) is True
    assert MX.jobs_claimed.value(kind="riff") == c0 + 2
    assert MX.jobs_done.value(kind="riff") == d0 + 1
    assert MX.jobs_failed.value(kind="riff", retry="false") == f0 + 1
    assert MX.job_duration.count(kind="riff") == n0 + 2
    # a transcriber job handed back counts as a retry failure
    r0 = MX.jobs_failed.value(kind="process_set", retry="true")
    api = ClaimApi(queue=[{"id": "job:ps-1", "kind": "process_set", "input": {"file": "x.m4a"}}])
    with pytest.raises(RuntimeError, match="handed back"):
        WM.run_one(api, ["riff"], ws=None, hosted=True)
    assert MX.jobs_failed.value(kind="process_set", retry="true") == r0 + 1


def test_budget_charge_feeds_tokens_usd_and_the_spend_gauges(tmp_path):
    b = Budget(2.0, tmp_path / "spend.json")
    MX.attach_budget(b)
    MX.attach_budget(b)   # idempotent: one listener
    assert b.listeners.count(MX.on_charge) == 1
    t0 = MX.ai_tokens.value(model="claude-opus-5-5", direction="input")
    usd = b.charge("claude-opus-5-5", {"input_tokens": 1000, "output_tokens": 200, "cache_read_input_tokens": 50},
                   task="creative_riff")
    assert MX.ai_tokens.value(model="claude-opus-5-5", direction="input") == t0 + 1000
    assert MX.ai_tokens.value(model="claude-opus-5-5", direction="output") >= 200
    assert MX.ai_tokens.value(model="claude-opus-5-5", direction="cache_read") >= 50
    assert MX.ai_spend_total.value(model="claude-opus-5-5", task="creative_riff") >= usd > 0
    txt = MX.render().decode()
    assert re.search(r"^pk_ai_spend_usd_today 0\.0\d+$", txt, re.M), txt
    assert "pk_ai_budget_usd 2" in txt and "pk_ai_paused 0" in txt
    b.charge("claude-opus-5-5", {"input_tokens": 600_000}, task="creative_riff")   # $2.40 > $2.00
    assert "pk_ai_paused 1" in MX.render().decode()


def test_labels_are_bounded_closed_sets():
    assert MX.model_label("Claude-Opus-5-5") == "claude-opus-5-5"
    assert MX.model_label("x" * 65) == "unknown" and MX.model_label("a b") == "unknown" and MX.model_label(None) == "unknown"
    assert MX.kind_label("set_text") == "set_text" and MX.kind_label("job:riff-abc/..") == "other"
    MX.on_charge("m", {"input_tokens": 1}, 0.0, "a-task-name-that-is-far-too-long-to-be-a-label-x")
    assert MX.ai_spend_total.value(model="m", task="other") == 0.0


def test_heartbeat_age_and_the_side_listener():
    MX.heartbeat()
    srv = MX.serve(port=0)   # 0 = off
    assert srv is None
    srv = MX.serve(port=None) if False else None
    import socket
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    srv = MX.serve(port=port, host="127.0.0.1")
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/metrics", timeout=5) as r:
            assert r.status == 200 and r.headers["Content-Type"].startswith("text/plain; version=0.0.4")
            txt = r.read().decode()
        assert re.search(r"^pk_worker_heartbeat_age_seconds \d", txt, re.M)
        assert re.search(r'^pk_worker_build_info\{version="[0-9.]+",sha="[0-9a-z]+"\} 1$', txt, re.M)
        assert "# TYPE pk_worker_job_duration_seconds histogram" in txt
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/healthz", timeout=5) as r:
            assert r.status == 200 and b"heartbeat" in r.read()
        try:
            urllib.request.urlopen(f"http://127.0.0.1:{port}/anything", timeout=5)
            raise AssertionError("expected 404")
        except urllib.error.HTTPError as e:
            assert e.code == 404
    finally:
        srv.shutdown()


def test_no_label_value_is_a_job_id_or_text():
    txt = MX.render().decode()
    for line in txt.splitlines():
        for k, v in re.findall(r'(\w+)="((?:[^"\\]|\\.)*)"', line):
            assert k in ("kind", "retry", "model", "direction", "task", "version", "sha", "le"), (k, v)
            assert "job:" not in v and " " not in v, (k, v)
