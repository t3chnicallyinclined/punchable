"""docs/API_CONTRACT.md "Merge" on both backends (files; SurrealDB in a scratch in-memory server) over TEMP copies of the real data (graph, empty decisions, temp
creative store). Never touches his files or his Drive (the API never calls Drive; gsync does that side).

    python -m pytest api-dev/tests/test_merge_api.py -q
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

MAIN, OTHER = "joke:marty-delivery-driver", "joke:karaoke-king-marty"


@pytest.fixture(scope="module", params=["files", "surreal"])
def api(request):
    t = TempKB("merge", decisions=False)
    db = None
    if request.param == "surreal":
        try:
            db = TempDB("merge")
            db.load(t)
        except Exception as e:  # noqa: BLE001  no SurrealDB binary on this machine
            t.cleanup()
            pytest.skip(f"SurrealDB scratch server not available: {e}")
    a = Api(t, request.param, db)
    yield a, t
    a.stop()
    if db:
        db.drop()
    t.cleanup()


def q(s):
    return quote(s, safe=":")


def lines(p: Path) -> list[str]:
    return [x for x in p.read_text(encoding="utf-8").splitlines() if x.strip()] if p.exists() else []


def test_candidates_shape(api):
    a, _ = api
    s, g = a.get("/api/merge/candidates")
    assert s == 200, g
    for k in ("jokes", "docs", "groups", "jokes_in_groups", "docs_in_groups", "jokes_after_merge"):
        assert isinstance(g["counts"][k], int), k
    assert isinstance(g["groups"], list) and g["rule"]
    s, one = a.get(f"/api/merge/candidates?joke={q(MAIN)}")
    assert s == 200 and one["joke_id"] == MAIN and "group" in one and "others" in one and "history" in one


def test_candidates_batch_is_one_call_for_many(api):
    a, _ = api
    s, one = a.get(f"/api/merge/candidates?joke={q(MAIN)}")
    s, many = a.get(f"/api/merge/candidates?jokes={q(MAIN)},{q(OTHER)}")
    assert s == 200 and set(many["items"]) == {MAIN, OTHER}, many
    it = many["items"][MAIN]
    assert set(it) == {"joke_id", "group", "others", "attached_to"} and len(it["others"]) <= 1
    assert (it["group"] or {}).get("id") == (one["group"] or {}).get("id")
    if one["others"]:
        assert it["others"][0]["joke_id"] == one["others"][0]["joke_id"]
    assert a.get("/api/merge/candidates?jokes=" + ",".join(["joke:x"] * 51))[0] == 400
    assert a.get("/api/merge/candidates?jokes=bad%20id")[0] == 400


def test_merge_keeps_every_line_and_undo_restores(api):
    a, t = api
    dec = t.kb / "decisions.jsonl"
    _, before_main = a.get(f"/api/jokes/{q(MAIN)}")
    _, before_other = a.get(f"/api/jokes/{q(OTHER)}")
    other_units = {u["id"]: u["text"] for v in before_other["versions"] for u in v["units"]}
    assert other_units
    n0 = lines(dec)

    s, r = a.post("/api/merge", {"main": MAIN, "others": [OTHER], "reason": "same Marty bit"})
    assert s == 200, r
    mid = r["merge_id"]
    assert mid.startswith("mg:") and r["indexes"] and r["drive"]["state"] == "queued"
    # the other joke is folded in: hidden from the Desk, its units (verbatim, dated) now under the main
    assert a.get(f"/api/jokes/{q(OTHER)}")[0] == 404
    _, after_main = a.get(f"/api/jokes/{q(MAIN)}")
    got = {u["id"]: (u["text"], u.get("date")) for v in after_main["versions"] for u in v["units"]}
    for uid, text in other_units.items():
        assert got[uid][0] == text, f"{uid} not kept verbatim"
        assert got[uid][1], f"{uid} lost its date"
    # his action: decision lines by owner, append-only; the merge event by owner with a verbatim snapshot
    n1 = lines(dec)
    assert n1[:len(n0)] == n0
    new = [json.loads(x) for x in n1[len(n0):]]
    assert new and all(d["by"] == "owner" and d["reason"].startswith("[merge] " + mid) for d in new)
    ev = [json.loads(x) for x in lines(t.creative / "merges.jsonl")]
    m = [e for e in ev if e["id"] == mid][0]
    assert m["op"] == "merge" and m["by"] == "owner"
    snap_units = {u["id"]: u["text"] for u in m["data"]["snapshot"][OTHER]["units"]}
    assert snap_units == other_units
    s, h = a.get(f"/api/merge/history?joke={q(MAIN)}")
    assert s == 200 and h["merges"][0]["id"] == mid
    s, one = a.get(f"/api/merge/{mid}")
    assert s == 200 and one["status"] == "merged" and "drive" in one

    s, u = a.post("/api/merge/undo", {"merge_id": mid})
    assert s == 200, u
    assert a.get(f"/api/jokes/{q(OTHER)}")[0] == 200
    _, back = a.get(f"/api/jokes/{q(OTHER)}")
    assert {x["id"]: x["text"] for v in back["versions"] for x in v["units"]} == other_units
    _, main_back = a.get(f"/api/jokes/{q(MAIN)}")
    assert {x["id"] for v in main_back["versions"] for x in v["units"]} == \
        {x["id"] for v in before_main["versions"] for x in v["units"]}
    n2 = lines(dec)
    assert n2[:len(n1)] == n1, "undo rewrote history instead of appending"
    assert a.post("/api/merge/undo", {"merge_id": mid})[0] == 409


def test_bad_merges_refused(api):
    a, _ = api
    assert a.post("/api/merge", {"main": MAIN, "others": [MAIN]})[0] == 400
    assert a.post("/api/merge", {"main": MAIN, "others": []})[0] == 400
    assert a.post("/api/merge", {"main": MAIN, "others": ["joke:no-such-joke-xyz"]})[0] == 404
    assert a.post("/api/merge/undo", {"merge_id": "mg:nope"})[0] == 404


def test_dismiss_is_recorded_once(api):
    a, t = api
    s, r = a.post("/api/merge/dismiss", {"jokes": [MAIN, OTHER], "reason": "not the same"})
    assert s == 200 and r["id"].startswith("dm:")
    s, r2 = a.post("/api/merge/dismiss", {"jokes": [OTHER, MAIN]})
    assert s == 200 and r2.get("already")
    ev = [json.loads(x) for x in lines(t.creative / "merges.jsonl") if '"dismiss"' in x]
    assert len(ev) == 1 and ev[0]["by"] == "owner"


def test_attached_note_leaves_the_desk_and_undo_brings_it_back(api):
    a, t = api
    s, r = a.post("/api/ideas", {"text": "zebra crossing guard union meeting bit, totally new thought"})
    assert s == 200 and "attach" in r, r
    assert r["attach"]["decision"] != "attach", r["attach"]   # nothing in his catalog is about this
    jid = r["id"]
    _, desk = a.get("/api/jokes?limit=5000")
    assert jid in {x["id"] for x in desk["items"]}
    # an AI attach event (what after_idea writes when the jot is clearly an existing bit)
    ev = {"ts": "2026-10-26T12:00:00+00:00", "op": "attach", "id": "at:test0001", "by": "ai:attach",
          "data": {"joke": jid, "into": MAIN, "text": "zebra crossing guard union meeting bit, totally new thought",
                   "date": "2026-10-26", "score": 0.9, "reasons": ["test"], "source": "app"}}
    with open(t.creative / "merges.jsonl", "a", encoding="utf-8") as f:
        f.write(json.dumps(ev) + "\n")
    _, desk = a.get("/api/jokes?limit=5000")
    assert jid not in {x["id"] for x in desk["items"]}, "an attached note still shows as a separate idea"
    _, desk_all = a.get("/api/jokes?limit=5000&include_hidden=1")
    assert jid in {x["id"] for x in desk_all["items"]}
    _, h = a.get(f"/api/merge/history?joke={q(MAIN)}")
    assert [n["joke_id"] for n in h["notes"]] == [jid] and h["notes"][0]["source"] == "inferred"
    s, u = a.post("/api/merge/undo", {"merge_id": "at:test0001"})
    assert s == 200, u
    _, desk = a.get("/api/jokes?limit=5000")
    assert jid in {x["id"] for x in desk["items"]}
    dec = [json.loads(x) for x in lines(t.kb / "decisions.jsonl")]
    assert not [d for d in dec if "attach" in (d.get("reason") or "")], "an AI attach must never be a decision line"
