"""The hosted worker (the server, the Anthropic API, no audio): which kinds it claims, what its claim says, the budget
pause, the heartbeat, and the text-only set analysis for a set transcribed on the phone. No network, no model, no
real data.

    python -m pytest worker/tests/test_hosted.py -q
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK))
import worker.__main__ as WM  # noqa: E402
from engine.tests.conftest import env  # noqa: E402,F401  (the engine's temp config + mock provider fixture)


class ClaimApi:
    """Records every claim body; hands out whatever `queue` holds (the API's job of refusing is tested in api-dev)."""

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


def test_hosted_kinds_never_include_a_transcriber_kind():
    for has_bge in (False, True):
        kinds = WM.hosted_kinds(has_bge)
        assert not set(kinds) & set(WM.TRANSCRIBER_KINDS), kinds
        assert {"riff", "punchup", "ask_chat", "bridge", "facts", "set_text"} <= set(kinds)
    assert "memolink" in WM.hosted_kinds(True) and "memolink" not in WM.hosted_kinds(False)
    assert set(WM.TRANSCRIBER_KINDS) == {"process_set", "import_youtube"}
    # the PC/dev policy is unchanged: it still asks for the audio kinds
    assert set(WM.TRANSCRIBER_KINDS) <= set(WM.default_kinds(online=False))
    assert not set(WM.default_kinds(online=True, hosted=True)) & set(WM.TRANSCRIBER_KINDS)


def test_claim_body_strips_transcriber_kinds_even_when_asked(monkeypatch):
    """`--kinds process_set,riff` on a hosted worker still never asks for process_set."""
    monkeypatch.setenv("PK_WORKER_NAME", "server-1")
    b = WM.claim_body(["process_set", "riff", "import_youtube", "ask_chat"], hosted=True)
    assert b["kinds"] == ["riff", "ask_chat"] and b["worker"] == "server-1" and b["hosted"] is True
    assert "paused" not in b
    b = WM.claim_body(["process_set", "riff"], hosted=False)
    assert b["kinds"] == ["process_set", "riff"] and b["hosted"] is False  # dev mode keeps them


def test_run_one_hosted_hands_back_a_transcriber_job_it_was_wrongly_given(monkeypatch):
    api = ClaimApi(queue=[{"id": "job:process_set-1", "kind": "process_set", "input": {"file": "x.m4a"}}])
    with pytest.raises(RuntimeError, match="handed back"):
        WM.run_one(api, ["riff"], ws=None, hosted=True)
    (path, body), = api.results
    assert "process_set-1" in path and body["ok"] is False and body["retry"] is True
    assert "phone" in body["error"]


def test_run_one_pauses_on_the_budget_with_a_visible_reason(tmp_path, monkeypatch):
    from engine.budget import Budget
    api = ClaimApi(queue=[{"id": "job:riff-1", "kind": "riff", "input": {}}])
    b = Budget(1.0, tmp_path / "spend.json")
    b.charge("claude-opus-5-5", {"input_tokens": 300_000})     # $1.20 of $1.00
    assert WM.run_one(api, ["riff"], ws=None, hosted=True, budget=b) is False
    [c] = api.claims
    assert c["kinds"] == ["riff"] and "over today's AI budget" in c["paused"] and "$1.20 of $1.00" in c["paused"]
    assert api.queue, "the job was not taken: it stays queued"
    # under the cap the claim carries no pause and the job is handed out
    b2 = Budget(5.0, tmp_path / "spend2.json")
    api2 = ClaimApi(queue=[{"id": "job:riff-2", "kind": "riff", "input": {"idea": "x", "lines": []}}])
    monkeypatch.setattr("worker.execute.run", lambda job, **k: {"status": "ok"})
    assert WM.run_one(api2, ["riff"], ws=None, hosted=True, budget=b2) is True
    assert "paused" not in api2.claims[0] and api2.results[0][1]["ok"] is True


def test_budget_exceeded_mid_job_goes_back_to_the_queue(monkeypatch):
    from engine.budget import BudgetExceeded
    api = ClaimApi(queue=[{"id": "job:riff-3", "kind": "riff", "input": {}}])

    def boom(job, **k):
        raise BudgetExceeded("over today's AI budget ($2.01 of $2.00); queued jobs run again after midnight UTC")

    monkeypatch.setattr("worker.execute.run", boom)
    assert WM.run_one(api, ["riff"], ws=None, hosted=True) is True
    (_, body), = api.results
    assert body["ok"] is False and body["retry"] is True and body["error"].startswith("over today's AI budget")


def test_heartbeat_touches_the_file(tmp_path):
    p = tmp_path / "hb" / "heartbeat"
    WM.heartbeat(p)
    assert p.is_file()
    WM.heartbeat(None)  # dev mode: nothing to touch


# ---------------------------------------------------------------- set_text: a set transcribed on the phone
SEGS = ["Put your hands together for the comic!",
        "You ever wonder what happens if you leave a belt in an airport bin?",
        "It's big enough to walk around, it's basically a studio apartment.",
        "So I bought a parrot that only speaks in legal disclaimers.",
        "Now every time I sneeze he says results may vary."]


def test_phone_transcript_timestamps_device_or_synthetic():
    from worker import execute
    tx = execute.phone_transcript({"segments": [{"text": "a b c", "start": 1.0, "end": 2.0}, {"text": "d", "start": 2, "end": 3}]})
    assert tx["timestamps"] == "device" and [s["start"] for s in tx["segments"]] == [1.0, 2.0]
    tx = execute.phone_transcript({"segments": [{"text": "one two three four five"}, {"text": "six"}, {"text": "  "}]})
    assert tx["timestamps"] == "synthetic" and len(tx["segments"]) == 2
    assert tx["segments"][1]["start"] == tx["segments"][0]["end"] > 0  # monotone
    with pytest.raises(ValueError):
        execute.phone_transcript({"segments": []})
    with pytest.raises(ValueError):
        execute.phone_transcript({"segments": [{"text": ""}]})


def test_set_text_runs_the_split_step_and_marks_audio_unavailable(env, monkeypatch):
    """The engine's mock provider answers the split; the files say what is NOT there (laughs, timing)."""
    from worker import execute
    monkeypatch.setenv("ENGINE_CONFIG", str(env["toml"]))
    inp = {"title": "Open mic", "recorded_at": "2026-10-20T21:00:00-04:00", "venue": "a room",
           "segments": [{"text": t} for t in SEGS]}
    r = execute.run({"kind": "set_text", "input": inp})
    assert r["status"] == "analyzed_text" and r["units"] == 2 and r["funny_units"] == 2  # the intro is not a unit
    assert r["audio"]["status"] == "unavailable" and r["audio"]["timestamps"] == "synthetic"
    d = env["root"] / "corpus" / "sets" / r["set_dir"]
    assert r["set_dir"].startswith("20261020_ph")
    a = json.loads((d / "analysis.json").read_text(encoding="utf-8"))
    assert [x["name"] for x in a["rows"]] == ["Belt in a bin", "Disclaimer parrot"]
    for row in a["rows"]:
        assert row["laugh_s"] is None and row["laugh_s_per_min"] is None and row["dead_air_max_s"] is None
    s = a["summary"]
    assert s["room_heat"] is None and s["laugh_s_total"] is None and "no audio" in s["room_heat_basis"]
    m = json.loads((d / "meta.json").read_text(encoding="utf-8"))
    assert m["source"] == "phone_transcript" and m["show_date"] == "2026-10-20" and m["audio"]["status"] == "unavailable"
    assert m["show_venue"] == "a room"
    tx = json.loads((d / "transcript.json").read_text(encoding="utf-8"))
    assert tx["timestamps"] == "synthetic" and len(tx["segments"]) == 5
    # the same transcript again is the same set (content-addressed), never a duplicate
    assert execute.run({"kind": "set_text", "input": inp})["set_dir"] == r["set_dir"]


def test_set_text_job_posts_funny_units_as_ideas_and_refuses_court_text(env, monkeypatch, tmp_path):
    from worker import execute, online
    monkeypatch.setenv("ENGINE_CONFIG", str(env["toml"]))
    posted = []

    class Api:
        def call(self, method, path, body=None):
            assert path == "/api/worker/ideas" and method == "POST"
            posted.append(body)
            return {"ok": True, "id": f"joke:new{len(posted)}", "attach": {"decision": "new"}}

    ws = online.Workspace(Api(), tmp_path / "ws", legal=lambda t: "petitioner" in t and "respondent" in t)
    ws.comedy = env["root"]  # the engine's out_corpus_dir is <root>/corpus: the job reads its files from there
    inp = {"title": "Open mic", "recorded_at": "2026-10-20T21:00:00-04:00", "segments": [{"text": t} for t in SEGS]}
    r = online.set_text_job(Api(), ws, {"kind": "set_text", "input": inp}, execute)
    assert r["status"] == "analyzed_text" and len(r["ideas"]) == 2
    assert all(p["actor"] == "ai:set_text" and p["from"]["set"] == r["set_dir"] for p in posted)
    assert posted[0]["title"] == "Belt in a bin" and "transcribed on the phone" in posted[0]["reason"]
    # court material: the transcript is deleted here and nothing is posted
    posted.clear()
    legal = {"segments": [{"text": "the petitioner and the respondent, index no 4"}, {"text": "sworn to before me"}]}
    r = online.set_text_job(Api(), ws, {"kind": "set_text", "input": legal}, execute)
    assert r == {"status": "skipped_legal"} and not posted
    assert not list((env["root"] / "corpus" / "sets").glob("*_ph*/transcript.json")) or \
        all("petitioner" not in p.read_text(encoding="utf-8") for p in (env["root"] / "corpus" / "sets").glob("*/transcript.json"))
