"""Simple Punchable routes (docs/API_CONTRACT.md "Simple Punchable") on both backends (files; SurrealDB on a scratch
server, where "From your notebook" also fuses the database's text+vector search), over TEMP copies of the data.

    python -m pytest api-dev/tests/test_simple.py -q
"""
from __future__ import annotations

import re
import sys
from pathlib import Path
from urllib.parse import quote

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "db" / "tests"))
sys.path.insert(0, str(DESK / "api-dev"))
from common import Api, TempDB, TempKB  # noqa: E402

BANNED = re.compile(r"\b(edges?|inferred|proposed|vs_room|confidence|pipeline|units?|x the room)\b|\d\.\d+x", re.I)


@pytest.fixture(scope="module", params=["files", "surreal"])
def api(request):
    t = TempKB("simple", decisions=False)
    db = None
    if request.param == "surreal":
        try:
            db = TempDB("simple")
            db.load(t)
        except Exception as e:  # noqa: BLE001  no SurrealDB binary on this machine
            t.cleanup()
            pytest.skip(f"SurrealDB scratch server not available: {e}")
    a = Api(t, request.param, db)
    a.backend = request.param
    yield a, t
    a.stop()
    if db:
        db.drop()
    t.cleanup()


def q(s):
    return quote(s, safe=":")


def shown(obj):
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k in ("why", "words", "stage_line", "status_words") and isinstance(v, str):
                yield v
            yield from shown(v)
    elif isinstance(obj, list):
        for v in obj:
            yield from shown(v)


def test_related_finds_his_bits_and_his_new_idea(api):
    a, _ = api
    _, page = a.get("/api/jokes?sort=performed&limit=1")
    j = page["items"][0]
    st, r = a.get(f"/api/notebook/related?q={q(j['title'])}")
    assert st == 200 and r["items"] and r["total"] == len(r["items"]) <= 10
    assert j["id"] in [it["joke_id"] for it in r["items"][:5]]
    for it in r["items"]:
        assert set(it) >= {"joke_id", "title", "quote", "date", "stage_line", "proven", "why"}
        if it["quote"]:  # verbatim: the line is in the bit's own text as the API serves it
            _, jd = a.get(f"/api/jokes/{q(it['joke_id'])}")
            texts = [u.get("text") or "" for v in jd.get("versions", []) for u in v.get("units", [])]
            texts += [jd["joke"].get("summary") or ""] + [p.get("perf", {}).get("text") or "" for p in jd.get("performances", [])]
            texts += [x for p in jd.get("performances", []) for x in [(p.get("punch") or {}).get("text") or ""]]
            assert any(it["quote"] in t for t in texts) or it["quote_node"].startswith("seg:"), it
    assert not [s for s in shown(r) if BANNED.search(s)]
    # a riff line saved as an idea is in his notebook straight away
    st, w = a.post("/api/ideas", {"text": "Zorblax the waiter guesses every order wrong on purpose.", "reason": "riff"})
    assert st == 200
    _, r2 = a.get("/api/notebook/related?q=zorblax%20waiter")
    assert r2["items"][0]["joke_id"] == w["id"] and r2["items"][0]["quote"].startswith("Zorblax the waiter")
    assert a.get("/api/notebook/related")[0] == 400
    assert a.get("/api/notebook/related?q=x&cap=99")[0] == 400


def test_joke_map(api):
    a, _ = api
    _, page = a.get("/api/jokes?sort=performed&limit=1")
    jid = page["items"][0]["id"]
    st, m = a.get(f"/api/jokes/{q(jid)}/map")
    assert st == 200 and m["joke"]["joke_id"] == jid
    assert [g["key"] for g in m["groups"]] == ["next", "tags", "same_world", "callbacks", "versions"]
    assert sum(len(g["items"]) for g in m["groups"]) > 0
    assert not [s for s in shown(m) if BANNED.search(s)]
    assert a.get("/api/jokes/joke:nope/map")[0] == 404
    # the plain joke route still answers
    assert a.get(f"/api/jokes/{q(jid)}")[0] == 200


def test_all_bits_map_is_capped_names_only_and_cached(api):
    import time
    a, _ = api
    t0 = time.time()
    s, m = a.get("/api/map?cap=50")
    assert s == 200, m
    assert set(m) >= {"rev", "cap", "nodes", "edges", "counts", "groups"}
    assert 0 < len(m["nodes"]) <= 50 and m["counts"]["nodes"] == len(m["nodes"]) and m["counts"]["jokes"] >= len(m["nodes"])
    ids = {n["id"] for n in m["nodes"]}
    for n in m["nodes"]:
        assert set(n) == {"id", "short_name", "title", "stage", "nights", "heat", "last_seen", "degree"}
        assert len(n["short_name"]) <= 24 and "\n" not in n["title"]
    for e in m["edges"]:
        assert e["a"] in ids and e["b"] in ids and e["kind"] in {g["key"] for g in m["groups"]}
    assert len(m["edges"]) <= 4 * len(m["nodes"])
    deg = {}
    for e in m["edges"]:
        deg[e["a"]] = deg.get(e["a"], 0) + 1
        deg[e["b"]] = deg.get(e["b"], 0) + 1
    assert max(deg.values(), default=0) <= 6
    # ranked: nights never go up as the list goes down
    nights = [n["nights"] for n in m["nodes"]]
    assert nights == sorted(nights, reverse=True)
    # the second call is served from the cache (same rev)
    t1 = time.time()
    s, m2 = a.get("/api/map?cap=50")
    assert s == 200 and m2["rev"] == m["rev"] and m2["counts"] == m["counts"]
    assert time.time() - t1 < 2, "cached answers are quick"
    assert a.get("/api/map?cap=0")[0] == 400 and a.get("/api/map?cap=9999")[0] == 400
    assert time.time() - t0 < 60


def test_board_build_and_working_set(api):
    a, _ = api
    _, page = a.get("/api/jokes?sort=performed&limit=3")
    ids = [x["id"] for x in page["items"]]
    st, b = a.post("/api/boards", {"title": "Set from here", "joke_ids": ids[:2], "target_s": 720})
    assert st == 201
    bid = b["board"]["id"]
    st, v = a.get(f"/api/boards/{q(bid)}/build")
    assert st == 200 and v["board"]["target_lo_s"] == 600 and v["board"]["target_s"] == 720
    assert [s["joke_id"] for s in v["steps"]] == ids[:2] and v["steps"][1]["transition_in"]["words"]
    assert v["status"] in ("short", "on", "over") and v["working_set"] is None
    assert len(v["next"]) <= 5 and all(x["joke_id"] not in ids[:2] for x in v["next"])
    assert not [s for s in shown(v) if BANNED.search(s)]
    st, ws = a.post("/api/workingsets", {"board_id": bid, "target_s": 720})
    assert st == 201
    _, v2 = a.get(f"/api/boards/{q(bid)}/build")
    assert v2["working_set"] == ws["working_set"]["id"]
    assert a.get("/api/boards/sb:nope/build")[0] == 404


def test_facts_and_riff_never_block(api):
    a, _ = api
    st, r = a.post("/api/facts", {"text": "AI guesses what comes next"})
    assert st == 202 and r["status"] == "queued" and r["job"]["kind"] == "facts"
    st2, r2 = a.post("/api/facts", {"text": "ai  guesses what comes NEXT"})
    assert st2 == 202 and r2["job"]["id"] == r["job"]["id"]
    st, r = a.post("/api/riff", {"idea": "AI guesses what comes next", "lines": ["the room is the model"]})
    assert st == 202 and r["job"]["kind"] == "riff"
    assert a.post("/api/facts", {})[0] == 400
    assert a.post("/api/riff", {"idea": "x", "lines": "nope"})[0] == 400
