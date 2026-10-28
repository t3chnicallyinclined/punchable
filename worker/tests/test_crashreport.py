"""The worker's crash reports (worker/crashreport.py over api-dev/crash.py): off without a DSN, named for the worker,
sent for a failed job with its kind and never for a budget pause. No network, no SDK.

    python -m pytest worker/tests/test_crashreport.py -q
"""
from __future__ import annotations

import sys
from pathlib import Path

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK))
import worker.__main__ as WM  # noqa: E402
from worker import crashreport as CR  # noqa: E402
from worker.tests.test_hosted import ClaimApi  # noqa: E402


def test_off_without_a_dsn_and_named_for_the_worker(monkeypatch):
    monkeypatch.delenv("SENTRY_DSN", raising=False)
    assert CR.init(log=lambda *a: None) is False and CR.enabled() is False
    assert CR.release().startswith("punchable-worker@0.1.0+")
    CR.report(RuntimeError("x"), kind="riff", retry=False)   # must not raise
    CR.flush()


def test_a_failed_job_is_reported_with_its_kind_and_a_budget_pause_is_not(monkeypatch):
    from engine.budget import BudgetExceeded
    seen = []
    monkeypatch.setattr(WM.CRASH, "report", lambda exc, **tags: seen.append((type(exc).__name__, tags)))

    def bad(job, **k):
        raise ValueError("the model refused the schema")

    monkeypatch.setattr("worker.execute.run", bad)
    api = ClaimApi(queue=[{"id": "job:riff-7", "kind": "riff", "input": {"text": "his line"}}])
    assert WM.run_one(api, ["riff"], ws=None, hosted=True) is True
    assert seen == [("ValueError", {"kind": "riff", "retry": False})]   # the kind, never the id or the input

    def slow(job, **k):
        raise TimeoutError("anthropic")

    monkeypatch.setattr("worker.execute.run", slow)
    api = ClaimApi(queue=[{"id": "job:riff-8", "kind": "riff", "input": {}}])
    WM.run_one(api, ["riff"], ws=None, hosted=True)
    assert seen[-1] == ("TimeoutError", {"kind": "riff", "retry": True})

    def over(job, **k):
        raise BudgetExceeded("over today's AI budget")

    monkeypatch.setattr("worker.execute.run", over)
    api = ClaimApi(queue=[{"id": "job:riff-9", "kind": "riff", "input": {}}])
    WM.run_one(api, ["riff"], ws=None, hosted=True)
    assert len(seen) == 2   # the pause is expected and shown in the app, not a crash
    (_, body), = api.results
    assert body["retry"] is True


def test_the_image_copies_what_the_shim_imports():
    """worker/crashreport.py imports api-dev/crash.py, which imports scrub.py: both must be in the worker image."""
    df = (DESK / "infra" / "worker" / "Dockerfile").read_text(encoding="utf-8")
    assert "COPY api-dev/crash.py api-dev/scrub.py api-dev/" in df
    assert CR.API_DEV.name == "api-dev" and (CR.API_DEV / "crash.py").is_file() and (CR.API_DEV / "scrub.py").is_file()
