"""The job queue since the PC was retired (2026-10-27): a kind no worker runs is honest about it (`waiting_for:
"transcriber"`, persisted, never dropped), a hosted worker's claim is checked (never a transcriber kind; a budget
pause hands out nothing), and the worker line says "the server", not "your PC". Pure unit tests over the job store
and the claim check: no API process, no data, no network.

    python -m pytest api-dev/tests/test_jobs_waiting.py -q
"""
from __future__ import annotations

import sys
import time
from pathlib import Path
from types import SimpleNamespace

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "api-dev"))
sys.path.insert(0, str(DESK))
import creative_api as CR  # noqa: E402
from creative.store import Store  # noqa: E402


def jobs_store(tmp_path) -> Store:
    return Store(tmp_path, "jobs", CR.job_apply)


def enqueue(st: Store, kind: str, i: str = "job:x"):
    st.append("enqueue", i, {"kind": kind, "key": f"{kind}|k", "input": {}})
    return st.get(i)


def test_transcriber_kinds_wait_and_the_state_survives_a_replay(tmp_path):
    st = jobs_store(tmp_path)
    for kind in ("process_set", "import_youtube"):
        j = enqueue(st, kind, f"job:{kind}")
        assert j["status"] == "queued" and j["waiting_for"] == "transcriber"
        b = CR.job_brief(j)
        assert b["waiting_for"] == "transcriber" and "phone" in b["note"]
    for kind in ("riff", "ask_chat", "set_text", "memolink"):
        j = enqueue(st, kind, f"job:{kind}")
        assert j["waiting_for"] is None and "waiting_for" not in CR.job_brief(j)
    # persisted: a fresh replay of the same log derives the same state
    again = Store(tmp_path, "jobs", CR.job_apply)
    assert again.get("job:process_set")["waiting_for"] == "transcriber" and again.get("job:riff")["waiting_for"] is None
    # a claim clears it; a retry-fail of a transcriber kind puts it back; a hard fail leaves nothing waiting
    st.append("claim", "job:process_set", {"worker": "x"})
    assert st.get("job:process_set")["waiting_for"] is None
    st.append("fail", "job:process_set", {"error": "no audio pipeline", "retry": True})
    assert st.get("job:process_set")["status"] == "queued" and st.get("job:process_set")["waiting_for"] == "transcriber"
    st.append("claim", "job:import_youtube", {"worker": "x"})
    st.append("fail", "job:import_youtube", {"error": "boom", "retry": False})
    assert st.get("job:import_youtube")["status"] == "failed" and st.get("job:import_youtube")["waiting_for"] is None


def test_queued_body_carries_the_reason(tmp_path):
    st = jobs_store(tmp_path)
    q = CR._queued(enqueue(st, "process_set"))
    assert q["status"] == "queued" and q["waiting_for"] == "transcriber" and q["note"].startswith("Transcription happens on your phone")
    assert q["poll"] == "/api/jobs/job:x" and q["job"]["waiting_for"] == "transcriber"
    q = CR._queued(enqueue(st, "riff", "job:r"))
    assert "waiting_for" not in q and "note" not in q


def test_hosted_claim_never_gets_a_transcriber_kind(tmp_path):
    kinds, seen = CR.check_claim({"kinds": ["riff", "ask_chat"], "worker": "server-1", "hosted": True,
                                  "model": "Anthropic API · claude-opus-5-5"})
    assert kinds == ["riff", "ask_chat"] and seen["hosted"] is True and seen["name"] == "server-1"
    assert seen["model"].startswith("Anthropic API") and seen["paused"] is None
    with pytest.raises(CR.HttpError) as e:
        CR.check_claim({"kinds": ["riff", "process_set"], "worker": "server-1", "hosted": True})
    assert e.value.status == 400 and "phone" in str(e.value)
    with pytest.raises(CR.HttpError):
        CR.check_claim({"worker": "server-1", "hosted": True})  # "anything" is not a hosted worker's answer
    with pytest.raises(CR.HttpError):
        CR.check_claim({"kinds": ["rm -rf"], "worker": "x"})
    # a development PC may still ask for the audio kinds
    kinds, seen = CR.check_claim({"kinds": ["process_set", "riff"], "worker": "HOME-PC"})
    assert kinds == ["process_set", "riff"] and seen["hosted"] is False
    # the store side: whatever the claim says, a hosted worker's kinds list never selects a transcriber job
    st = jobs_store(tmp_path)
    enqueue(st, "process_set", "job:p")
    enqueue(st, "riff", "job:r")
    cr = CR.Creative(tmp_path / "creative", None)
    j = cr.claim({"jobs": st}, ["riff", "ask_chat"], "server-1")
    assert j["id"] == "job:r" and st.get("job:p")["status"] == "queued" and st.get("job:p")["waiting_for"] == "transcriber"
    assert cr.claim({"jobs": st}, ["riff", "ask_chat"], "server-1") is None  # nothing else it may take


def test_paused_claim_reports_the_budget_and_the_worker_line_says_the_server():
    kinds, seen = CR.check_claim({"kinds": ["riff"], "worker": "server-1", "hosted": True,
                                  "paused": "over today's AI budget ($2.01 of $2.00); queued jobs run again after midnight UTC"})
    assert seen["paused"].startswith("over today's AI budget")
    cr = SimpleNamespace(jobs_mode="worker", worker_seen=seen)
    w = CR.worker_status(cr)
    assert w["online"] is True and w["hosted"] is True and w["where"] == "the server" and w["name"] == "server-1"
    assert w["paused"].startswith("over today's AI budget") and w["online_window_s"] == CR.WORKER_ONLINE_S
    assert "PC" not in str(w)
    # quiet for longer than the window: offline, and a stale pause is not reported as current
    cr.worker_seen = {**seen, "t": time.time() - CR.WORKER_ONLINE_S - 1}
    w = CR.worker_status(cr)
    assert w["online"] is False and w["paused"] is None and w["last_seen"] == seen["at"]
    # never heard from
    w = CR.worker_status(SimpleNamespace(jobs_mode="worker", worker_seen=None))
    assert w["online"] is False and w["name"] is None and w["hosted"] is False
    assert CR.worker_status(SimpleNamespace(jobs_mode="inproc"))["online"] is True


def test_set_text_is_a_job_kind_the_worker_may_run():
    assert "set_text" in CR.JOB_KINDS and "set_text" not in CR.TRANSCRIBER_KINDS and "set_text" not in CR.WORKER_ONLY
    assert CR.TRANSCRIBER_KINDS == {"process_set", "import_youtube"}
