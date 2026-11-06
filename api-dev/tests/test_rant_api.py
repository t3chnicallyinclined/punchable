"""Film it's routes on the Python API (creative_api.Routes.joke_rant / joke_hook): the version is the one the API holds,
the job input is its lines, the key makes a repeat the same job, and a version's cut is read from its source. A tiny
knowledge base built here (invented jokes); the job queue is a stub that records what was enqueued.

    python -m pytest api-dev/tests/test_rant_api.py -q
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

DESK = Path(__file__).resolve().parents[2]
for p in (DESK / "api-dev", DESK / "tools" / "explorer", DESK):
    sys.path.insert(0, str(p))
import creative_api as CR  # noqa: E402
import kb as K  # noqa: E402

GRAPH = {"version": "t", "nodes": [
    {"id": "joke:sc", "type": "joke", "title": "Self-checkout", "stage": "working"},
    {"id": "unit:v1", "type": "unit", "text": "Self-checkout is the store hiring you for free.\n\nNobody clapped.  \n",
     "date": "2026-11-01", "source": "app"},
    {"id": "unit:r1", "type": "unit", "text": "You scan forty things.\nNobody claps.", "date": "2026-11-02", "source": "rant_session"},
    {"id": "unit:blank", "type": "unit", "text": "   \n  ", "date": "2026-11-02"},
    {"id": "joke:other", "type": "joke", "title": "Other", "stage": "idea"},
    {"id": "unit:o1", "type": "unit", "text": "Not this bit.", "date": "2026-11-01"},
], "edges": [
    {"id": "e:1", "kind": "part_of", "src": "unit:v1", "dst": "joke:sc", "status": "confirmed"},
    {"id": "e:2", "kind": "part_of", "src": "unit:r1", "dst": "joke:sc", "status": "confirmed"},
    {"id": "e:3", "kind": "part_of", "src": "unit:blank", "dst": "joke:sc", "status": "confirmed"},
    {"id": "e:4", "kind": "part_of", "src": "unit:o1", "dst": "joke:other", "status": "confirmed"},
]}


class StubCr:
    def __init__(self):
        self.calls = []

    def stores(self, kb):
        return {"jobs": None}

    def enqueue(self, st, kind, key, inp, fresh=False, kb=None, by=None):
        self.calls.append((kind, key, inp, fresh))
        return 202, {"status": "queued", "job": {"id": f"job:{kind}-x", "kind": kind}, "poll": f"/api/jobs/job:{kind}-x"}


@pytest.fixture()
def kb(tmp_path):
    (tmp_path / "graph.json").write_text(json.dumps(GRAPH), encoding="utf-8")
    (tmp_path / "decisions.jsonl").write_text("", encoding="utf-8")
    return K.KB(tmp_path / "graph.json", tmp_path / "decisions.jsonl", None, None)


def call(kb, name, jid, body):
    cr = StubCr()
    out = getattr(CR.Routes(cr, None, kb, {}, body), name)(jid)
    return out, cr.calls


def test_the_cut_of_a_version_is_its_source():
    assert CR.cut_of("rant") == "rant" and CR.cut_of("rant_session") == "rant" and CR.cut_of("rant_hook") == "rant"
    for s in ("app", "hook", "session", "rants", "rant-x", "", None, 3):
        assert CR.cut_of(s) == "stage", s
    assert CR.cut_lines("a\r\n\n  b  \n \n") == ["a", "  b"]


def test_rant_enqueues_the_version_the_api_holds(kb):
    (st, body), calls = call(kb, "joke_rant", "joke:sc", {"unit": "unit:v1", "text": "the phone's text is ignored"})
    assert st == 202 and body["job"]["kind"] == "rant"
    kind, key, inp, fresh = calls[0]
    assert kind == "rant" and key == "rant|joke:sc|unit:v1|-" and not fresh
    assert inp == {"joke_id": "joke:sc", "title": "Self-checkout", "unit": "unit:v1", "cut": "stage",
                   "lines": ["Self-checkout is the store hiring you for free.", "Nobody clapped."], "punch": None}
    (_, _), calls = call(kb, "joke_rant", "joke:sc", {"unit": "unit:r1", "punch": 1, "fresh": True})
    assert calls[0][1] == "rant|joke:sc|unit:r1|1" and calls[0][2]["cut"] == "rant" and calls[0][3] is True


@pytest.mark.parametrize("jid,body,status", [
    ("joke:sc", {}, 400),
    ("joke:sc", {"unit": 5}, 400),
    ("joke:nope", {"unit": "unit:v1"}, 404),
    ("joke:sc", {"unit": "unit:o1"}, 404),          # a version of another bit
    ("joke:sc", {"unit": "unit:missing"}, 404),
    ("joke:sc", {"unit": "unit:blank"}, 400),        # nothing to read
    ("joke:sc", {"unit": "unit:v1", "punch": 2}, 400),
    ("joke:sc", {"unit": "unit:v1", "punch": True}, 400),
    ("bad id!", {"unit": "unit:v1"}, 400),
])
def test_rant_refusals(kb, jid, body, status):
    with pytest.raises(CR.HttpError) as e:
        call(kb, "joke_rant", jid, body)
    assert e.value.status == status


def test_hook_keys_on_what_he_already_saw(kb):
    (_, _), c1 = call(kb, "joke_hook", "joke:sc", {"unit": "unit:v1"})
    (_, _), c2 = call(kb, "joke_hook", "joke:sc", {"unit": "unit:v1", "exclude": [" One. ", "Two.", "Three."]})
    assert c1[0][0] == "hook" and c1[0][2]["exclude"] == [] and c2[0][2]["exclude"] == ["One.", "Two.", "Three."]
    assert c1[0][1].startswith("hook|joke:sc|unit:v1|-|") and c1[0][1] != c2[0][1]
    import hashlib
    assert c2[0][1].endswith(hashlib.sha1(b"One.\nTwo.\nThree.").hexdigest()[:10])
    for bad in ("x", ["x"] * 7, [""], [3], ["y" * 301]):
        with pytest.raises(CR.HttpError) as e:
            call(kb, "joke_hook", "joke:sc", {"unit": "unit:v1", "exclude": bad})
        assert e.value.status == 400


def test_the_legal_rules_run_on_the_version(kb, monkeypatch):
    import ingest_api as IG
    monkeypatch.setattr(IG, "is_legal", lambda t: "free" in t)
    with pytest.raises(CR.HttpError) as e:
        call(kb, "joke_rant", "joke:sc", {"unit": "unit:v1"})
    assert e.value.status == 422
    (st, _), _ = call(kb, "joke_rant", "joke:sc", {"unit": "unit:r1"})
    assert st == 202


def test_the_kinds_are_known_to_the_queue_and_the_claim():
    assert {"rant", "hook"} <= set(CR.JOB_KINDS) and not {"rant", "hook"} & CR.TRANSCRIBER_KINDS
    kinds, _ = CR.check_claim({"kinds": ["rant", "hook"], "worker": "server-1", "hosted": True})
    assert kinds == ["rant", "hook"]
    assert ("joke_rant", ["joke:a b"]) == CR.match("POST", "/api/jokes/joke%3Aa%20b/rant")
    assert ("joke_hook", ["joke:a"]) == CR.match("POST", "/api/jokes/joke:a/hook")
