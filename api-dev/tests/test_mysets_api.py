"""Your sets routes (docs/API_CONTRACT.md "Your sets") on both backends, over TEMP copies of his data (his real
boards and working sets are copied, never written).

    python -m pytest api-dev/tests/test_mysets_api.py -q
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
from common import Api, TempDB, TempKB  # noqa: E402

BANNED = re.compile(r"\b(edges?|inferred|proposed|vs_room|confidence|pipeline|units?)\b|\d\.\d+\s*x|laugh-s", re.I)
AC = "2025-10-11_FakeSet0007"   # his 20-minute set: 14 bits on tape


@pytest.fixture(scope="module", params=["files", "surreal"])
def api(request):
    t = TempKB("mysets", decisions=True)  # his ideas typed in the app (the AI board) live in decisions
    db = None
    if request.param == "surreal":
        try:
            db = TempDB("mysets")
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


def words(obj):
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k in ("words", "label_words", "last_words", "status_words", "why", "health", "stage_line") and isinstance(v, str):
                yield v
            yield from words(v)
    elif isinstance(obj, list):
        for v in obj:
            yield from words(v)


def performed(t: TempKB, key: str) -> list[str]:
    d = next(p for p in (t.corpus / "sets").iterdir() if p.name.startswith(key[:10].replace("-", "")))
    a = json.loads((d / "analysis.json").read_text(encoding="utf-8"))
    idm = json.loads(t.id_map.read_text(encoding="utf-8")).get("jokes", {}) if t.id_map else {}
    out = []
    for s in a["jokes"]:
        j = idm.get(s.get("joke_id"), s.get("joke_id"))
        if j and s.get("status") in ("known", "new") and j not in out:
            out.append(j)
    return out


def test_existing_working_sets_and_boards_survive(api):
    a, t = api
    st, ls = a.get("/api/mysets")
    assert st == 200 and ls["lengths"] == [300, 420, 600, 900]
    real_ws = [json.loads(x)["id"] for x in (t.creative / "workingsets.jsonl").read_text(encoding="utf-8").splitlines()
               if x.strip() and json.loads(x)["op"] == "promote"]
    assert {s["id"] for s in ls["sets"]} >= set(real_ws)         # every working set he has is one of his sets
    for s in ls["sets"]:
        assert set(s) >= {"id", "title", "target_s", "length_words", "n", "runtime", "health", "tough_n", "version"}
        st, v = a.get(f"/api/mysets/{q(s['id'])}")
        assert st == 200 and v["set"]["id"] == s["id"] and len(v["bits"]) == s["n"]
        assert st == 200 and a.get(f"/api/workingsets/{q(s['id'])}")[0] == 200   # the old screen still opens
    _, boards = a.get("/api/boards")
    assert any(b["title"] == "Robot vacuum" for b in boards["boards"])


def test_a_ten_from_his_tapes_then_a_five(api):
    a, t = api
    ids = performed(t, AC)
    assert len(ids) >= 10
    st, big = a.post("/api/mysets", {"title": "AC hour", "target_s": 1200, "joke_ids": ids})
    assert st == 201 and big["set"]["length_words"] == "20 min" and len(big["bits"]) == len(ids)
    sid = big["set"]["id"]
    # every bit says what's working, from the tapes, in words
    labels = {b["joke_id"]: b["label"] for b in big["bits"]}
    assert set(labels.values()) <= {"killed", "solid", "mixed", "quiet", "untested"}
    assert labels["joke:parking-meter"] == "killed" and labels["joke:budget-airline"] == "quiet"
    assert any(b["tough"] for b in big["bits"]) and big["tough_n"] >= 1
    assert big["tapes"]["n"] >= 1 and big["tapes"]["last"]["set"] == AC
    assert not [w for w in words(big) if BANNED.search(w)]
    # make a 10 from it: opener and closer kept, the length reached
    st, tr = a.get(f"/api/mysets/{q(sid)}/trim?target_s=600")
    assert st == 200 and tr["status"] == "on" and tr["runtime_s"] <= 600
    keep = [k["joke_id"] for k in tr["keep"]]
    assert keep[0] == ids[0] and keep[-1] == ids[-1] and "joke:parking-meter" in keep
    assert "joke:budget-airline" not in keep
    st, ten = a.post("/api/mysets", {"from_set": sid, "target_s": 600, "title": "10 min"})
    assert st == 201 and [b["joke_id"] for b in ten["bits"]] == keep
    assert ten["versions"][0]["note"] == "v1: a 10 min from 'AC hour'"
    # and a 5 from the 10
    st, five = a.post("/api/mysets", {"from_set": ten["set"]["id"], "target_s": 300})
    assert st == 201 and five["runtime_s"] <= 300 and five["set"]["title"] == "5 min from 10 min"
    _, ls = a.get("/api/mysets")
    mine = {s["title"]: s for s in ls["sets"]}
    assert {"AC hour", "10 min", "5 min from 10 min"} <= set(mine)
    assert all(re.fullmatch(r"solid|empty|untested|\d+ tough spots?", s["health"]) for s in ls["sets"])


def test_work_on_a_set_reorder_lock_versions(api):
    a, t = api
    ids = performed(t, AC)[:5]
    _, v = a.post("/api/mysets", {"title": "Work", "target_s": 420, "joke_ids": ids})
    sid = v["set"]["id"]
    new = [ids[1], ids[0]] + ids[2:]
    st, v2 = a.post(f"/api/mysets/{q(sid)}", {"joke_ids": new}, method="PATCH")
    assert st == 200 and [b["joke_id"] for b in v2["bits"]] == new and v2["set"]["version"] == 2
    assert v2["versions"][0]["note"].startswith("Moved ")
    st, v3 = a.post(f"/api/mysets/{q(sid)}", {"target_s": 300}, method="PATCH")
    assert v3["versions"][0]["note"] == "Length 5 min (was 7 min)" and v3["set"]["target_s"] == 300
    # lock a bit: no tough spots on it
    j = next((b["joke_id"] for b in v3["bits"] if any(s["kind"] != "transition" for s in b["tough"])), new[0])
    st, v4 = a.post(f"/api/mysets/{q(sid)}/lock", {"joke_id": j})
    b = next(b for b in v4["bits"] if b["joke_id"] == j)
    assert st == 200 and b["locked"] and all(s["kind"] == "transition" for s in b["tough"])
    _, v5 = a.post(f"/api/mysets/{q(sid)}/lock", {"joke_id": j, "unlock": True})
    assert not next(b for b in v5["bits"] if b["joke_id"] == j)["locked"]
    # rename is not a version
    _, v6 = a.post(f"/api/mysets/{q(sid)}", {"title": "Work in progress"}, method="PATCH")
    assert v6["set"]["title"] == "Work in progress" and v6["set"]["version"] == 3
    assert [x["v"] for x in v6["versions"]] == [3, 2, 1]
    # every write is his, in the working-set log
    evs = [json.loads(x) for x in (t.creative / "workingsets.jsonl").read_text(encoding="utf-8").splitlines() if x.strip()]
    mine = [e for e in evs if e["id"] == sid]
    assert [e["op"] for e in mine] == ["promote", "new_version", "new_version", "lock_bit", "unlock_bit", "update"]
    assert all(e["by"] == "owner" for e in mine)
    # bad input
    assert a.post(f"/api/mysets/{q(sid)}", {"joke_ids": [new[0], new[0]]}, method="PATCH")[0] == 400
    assert a.post(f"/api/mysets/{q(sid)}", {"target_s": 5}, method="PATCH")[0] == 400
    assert a.post(f"/api/mysets/{q(sid)}/lock", {"joke_id": "joke:not-in-it-xyz"})[0] in (400, 404)
    assert a.post("/api/mysets", {"title": "", "target_s": 300})[0] == 400
    assert a.get("/api/mysets/ws:nope")[0] == 404
    # delete: gone from the list, still in the log
    assert a.post(f"/api/mysets/{q(sid)}", {"delete": True}, method="PATCH")[0] == 200
    assert sid not in [s["id"] for s in a.get("/api/mysets")[1]["sets"]]


def test_from_a_board_and_bridge_drafts_queue(api):
    a, _ = api
    _, boards = a.get("/api/boards")
    b = next(b for b in boards["boards"] if b["title"] == "Robot vacuum")
    st, v = a.post("/api/mysets", {"board_id": b["id"], "target_s": 420})
    assert st == 201 and v["set"]["storyboard_id"] == b["id"] and len(v["bits"]) == b["n"]
    weak = next((x for x in v["bits"] if x["transition_in"] and x["transition_in"]["weak"]), None)
    assert weak, "the AI board has a weak transition"
    spot = next(s for s in weak["tough"] if s["kind"] == "transition")
    act = next(x for x in spot["actions"] if x["kind"] == "bridge")
    st, r = a.post(f"/api/mysets/{q(v['set']['id'])}/bridge", {"from": act["from"], "to": act["to"]})
    assert st == 202 and r["status"] == "queued" and r["job"]["kind"] == "bridge"
    assert a.post(f"/api/mysets/{q(v['set']['id'])}/bridge", {"from": act["from"], "to": "joke:parking-meter"})[0] == 400


def test_a_recording_lines_up_with_the_set_it_resembles(api):
    a, t = api
    ids = performed(t, AC)
    _, x = a.post("/api/mysets", {"title": "Match me", "target_s": 900, "joke_ids": ids[:9]})
    _, y = a.post("/api/mysets", {"title": "Not me", "target_s": 300, "joke_ids": ids[9:11]})
    st, m = a.get(f"/api/mysets/match?set={AC}")
    # "AC hour" (all 14 bits, made in an earlier test) beats "Match me" (9 of them), and both beat "Not me" (2)
    assert st == 200 and m["match"]["title"] == "AC hour" and m["match"]["how"] == "closest"
    order = [c["title"] for c in m["candidates"]]
    assert order.index("Match me") < len(order) and "Not me" not in order[:2]
    _, xv = a.get(f"/api/mysets/{q(x['set']['id'])}")
    assert xv["tapes"]["n"] == 0                                     # its recordings went to the closer set
    _, big = a.get(f"/api/mysets/{q(m['match']['id'])}")
    assert big["tapes"]["last"]["set"] == AC
    assert a.get("/api/mysets/match?set=2099-01-01_nope")[0] == 404
