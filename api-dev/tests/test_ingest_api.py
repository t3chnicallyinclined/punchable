"""Online-first ingest (api-dev/ingest_api.py, suggest_api.py) and provenance, on both backends, over TEMP copies.

    python -m pytest api-dev/tests/test_ingest_api.py -q

Provenance: his own writes are by the owner id; imports name `import:<source>`, machine judgments `ai:<what>`; a
worker route without an actor, or with the owner's id, is refused. Legal: court text is refused (422) and nothing of
it is kept. Files: only db/srcpaths.WORKER_PATTERNS, never `..`, never audio. "Not a joke?": a suggestion only;
hiding is his own decision line and undo brings the idea back.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path
from urllib.parse import quote

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "db" / "tests"))
from common import Api, TempDB, TempKB  # noqa: E402

LEGAL_TEXT = "The petitioner and the respondent appeared; index no 123 was read into the record."


@pytest.fixture(scope="module", params=["files", "surreal"])
def api(request):
    t = TempKB("ingest", decisions=False)
    db = None
    if request.param == "surreal":
        try:
            db = TempDB("ingest")
            db.load(t)
        except Exception as e:  # noqa: BLE001
            t.cleanup()
            pytest.skip(f"SurrealDB scratch server not available: {e}")
    a = Api(t, request.param, db)
    yield a, t
    a.stop()
    if db:
        db.drop()
    t.cleanup()


def n_decisions(a) -> int:
    s, d = a.get("/api/decisions?limit=1")
    assert s == 200
    return d["total"]


def test_his_idea_is_by_him_an_import_by_its_source(api):
    a, _ = api
    s, r = a.post("/api/ideas", {"text": "a jot he typed in the app about airport security lines"})
    assert s == 200 and r["decisions"][0]["by"] == "owner"
    s, r = a.post("/api/ideas", {"text": "a memo about parking tickets and meter maids", "actor": "import:memo"})
    assert s == 200 and r["decisions"][0]["by"] == "import:memo"
    s, r = a.post("/api/ideas", {"text": "anything", "actor": "owner2"})
    assert s == 400
    s, d = a.get("/api/decisions?limit=2")
    assert [x["actor"] for x in d["decisions"]] == ["import:memo", "owner"]


def test_worker_routes_need_an_import_actor(api):
    a, _ = api
    before = n_decisions(a)
    s, r = a.post("/api/worker/ideas", {"text": "a gsync idea with no actor"})
    assert s == 400
    s, r = a.post("/api/worker/ideas", {"text": "a gsync idea from a google doc", "actor": "import:gsync"})
    assert s == 200 and r["decisions"][0]["by"] == "import:gsync"
    jid = r["id"]
    s, r = a.post("/api/worker/decision", {"target": "node", "action": "edit", "id": jid,
                                           "after": {"summary": "edited in google docs"}, "actor": "import:gsync",
                                           "reason": "[gdocs] edited"})
    assert s == 200 and r["decisions"][0]["by"] == "import:gsync"
    s, r = a.post("/api/worker/undo", {"indexes": [r["indexes"][0]], "actor": "import:gsync"})
    assert s == 200 and r["decisions"][0]["by"] == "import:gsync" and r["decisions"][0]["undo_of"] is not None
    assert n_decisions(a) == before + 3
    s, j = a.get("/api/worker/jokes/" + quote(jid, safe=":"))
    assert s == 200 and j["joke"]["summary"] == "a gsync idea from a google doc"


def test_court_text_is_refused_and_not_kept(api):
    a, t = api
    before = n_decisions(a)
    for path, body in (("/api/worker/ideas", {"text": LEGAL_TEXT, "actor": "import:memo"}),
                       ("/api/worker/files", {"files": [{"path": "corpus/memos/2026-01-01_aaaa1111/meta.json",
                                                         "text": json.dumps({"note": LEGAL_TEXT})}], "actor": "import:memo"})):
        s, r = a.post(path, body)
        assert s == 422, (path, r)
        assert "petitioner" not in json.dumps(r).lower()
    assert n_decisions(a) == before
    assert not (t.root / "corpus" / "memos" / "2026-01-01_aaaa1111").exists()


def test_files_whitelist_and_round_trip(api):
    a, t = api
    for bad in ("../kb/x.json", "tapes/raw/x/audio.opus", "kb/v0/decisions.jsonl", "corpus/sets/x/notes.txt", "C:/x.json"):
        s, r = a.post("/api/worker/files", {"files": [{"path": bad, "text": "{}"}], "actor": "import:upload"})
        assert s == 403, (bad, s, r)
    rel = "corpus/memos/2026-02-03_bbbb2222/transcript.json"
    doc = {"segments": [{"start": 0.0, "end": 1.5, "text": "a memo about pigeons"}]}
    s, r = a.post("/api/worker/files", {"files": [{"path": rel, "text": json.dumps(doc)}], "actor": "import:memo"})
    assert s == 200 and r["written"] == [rel] and r["actor"] == "import:memo"
    s, r = a.post("/api/worker/files", {"files": [{"path": rel, "text": json.dumps(doc)}], "actor": "import:memo"})
    assert s == 200 and r["written"] == [] and r["unchanged"] == 1
    s, ls = a.get("/api/worker/files?prefix=corpus/memos/")
    assert s == 200 and rel in [f["path"] for f in ls["files"]]
    s, c = a.get("/api/worker/files/content?path=" + quote(rel))
    assert s == 200 and json.loads(c["text"]) == doc
    s, r = a.post("/api/worker/files", {"files": [], "remove": [rel], "actor": "import:memo"})
    assert s == 200 and r["removed"] == [rel]
    assert not (t.root / rel).exists()


def test_a_new_sets_output_is_synced_into_the_graph(api):
    a, t = api
    sets = sorted(p for p in (t.corpus / "sets").iterdir() if (p / "analysis.json").is_file())
    src = sets[0]
    new = "20991231_TestSet00001"
    an = json.loads((src / "analysis.json").read_text(encoding="utf-8"))
    an["set"] = "2099-12-31_TestSet00001"
    files = [{"path": f"corpus/sets/{new}/analysis.json", "text": json.dumps(an)}]
    if (src / "meta.json").is_file():
        files.append({"path": f"corpus/sets/{new}/meta.json", "text": (src / "meta.json").read_text(encoding="utf-8")})
    s0, before = a.get("/api/sets")
    s, r = a.post("/api/worker/files", {"files": files, "actor": "import:youtube"})
    assert s == 200, r
    s, after = a.get("/api/sets")
    assert s == 200 and len(after["sets"]) == len(before["sets"]) + 1, (len(before["sets"]), len(after["sets"]), r)


def test_events_are_machine_judgments_only(api):
    a, _ = api
    s, r = a.post("/api/ideas", {"text": "the dog walker who only walks cats on tuesdays", "actor": "import:memo"})
    jid = r["id"]
    s, j = a.get("/api/jokes?limit=1")
    into = j["items"][0]["id"] if j["items"][0]["id"] != jid else j["items"][1]["id"]
    body = {"store": "merges", "op": "attach", "id": "at:0123456789ab", "actor": "import:memo",
            "data": {"joke": jid, "into": into, "text": "x", "date": "2026-10-27", "reasons": ["test"]}}
    s, r = a.post("/api/worker/events", body)
    assert s == 400
    s, r = a.post("/api/worker/events", {**body, "actor": "ai:attach"})
    assert s == 200 and r["record"]["by"] == "ai:attach", r
    s, c = a.get("/api/merge/candidates?joke=" + quote(jid, safe=":"))
    assert s == 200 and (c["attached_to"] or {}).get("into") == into
    s, r = a.post("/api/merge/undo", {"merge_id": "at:0123456789ab"})
    assert s == 200


def test_not_a_joke_is_a_suggestion_he_answers(api):
    a, t = api
    s, r = a.post("/api/ideas", {"text": "hey it's me, call the landlord back about the lease tomorrow",
                                 "actor": "import:memo"})
    talk = r["id"]
    s, r = a.post("/api/ideas", {"text": "remind me to renew the domain and update the kubernetes certs",
                                 "actor": "import:memo"})
    note = r["id"]
    (t.creative / "suggest_judge.json").write_text(json.dumps({"version": 1, "items": {
        "m1": {"kind": "conversation", "confidence": 0.9, "why": "a phone message", "joke_id": talk},
        "m2": {"kind": "note", "confidence": 0.95, "why": "a work reminder", "joke_id": note},
        "m3": {"kind": "joke", "confidence": 0.9, "why": "a bit", "joke_id": talk},
        "m4": {"kind": "note", "confidence": 0.5, "why": "unsure", "joke_id": note}}}), encoding="utf-8")
    s, g = a.get("/api/suggestions/not-a-joke")
    assert s == 200 and sorted(x["joke_id"] for x in g["items"]) == sorted([talk, note])
    assert all(x["source"] == "inferred" for x in g["items"])
    # nothing was hidden by the suggestion itself
    s, j = a.get("/api/jokes/" + quote(talk, safe=":"))
    assert s == 200
    s, h = a.post("/api/suggestions/not-a-joke/hide", {"joke_id": talk})
    assert s == 200 and h["decisions"][0]["by"] == "owner" and h["decisions"][0]["after"] == {"hidden": True}
    s, g = a.get("/api/suggestions/not-a-joke")
    assert [x["joke_id"] for x in g["items"]] == [note]
    s, _ = a.post("/api/undo", {"indexes": h["undo"]["indexes"]})
    assert s == 200
    s, g = a.get("/api/suggestions/not-a-joke")
    assert sorted(x["joke_id"] for x in g["items"]) == sorted([talk, note])
    s, k = a.post("/api/suggestions/not-a-joke/keep", {"joke_id": note})
    assert s == 200 and k["kept"]
    s, g = a.get("/api/suggestions/not-a-joke")
    assert [x["joke_id"] for x in g["items"]] == [talk]


def test_new_vectors_from_the_worker_are_picked_up_without_a_restart(tmp_path):
    sys.path.insert(0, str(DESK / "api-dev"))
    import creative_api as CR
    vec = tmp_path / "vectors"
    vec.mkdir()
    cr = CR.Creative(tmp_path / "creative", vec)
    assert cr.vectors().get("summary:joke:x") is None
    (vec / "extra_summary.json").write_text(json.dumps({"dim": 384, "rows": {"summary:joke:x": [0.5] * 384}}),
                                            encoding="utf-8")
    v = cr.vectors().get("summary:joke:x")
    assert v is not None and len(v) == 384


def test_youtube_sets_carry_their_link_instead_of_audio(api):
    a, t = api
    s, d = a.get("/api/sets")
    yt = [x for x in d["sets"] if x.get("source") == "youtube"]
    if not yt:
        pytest.skip("no YouTube set in this data")
    x = yt[0]
    vid = x["reference"]["video_id"]
    assert x["reference"]["kind"] == "youtube" and x["reference"]["url"].endswith("v=" + vid) and x["id"].endswith(vid)
    s, an = a.get("/api/sets/" + quote(x["id"], safe=":") + "/analysis")
    assert s == 200 and (an.get("reference") or {}).get("video_id") == vid
