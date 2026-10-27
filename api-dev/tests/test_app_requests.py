"""docs/API_CONTRACT.md "Requested by app (2026-10-26)": every row, on the files backend over TEMP copies of the real
data (graph, empty decisions, the corpus's analyses, a temp creative store and inbox). Never touches his files.

    python -m pytest api-dev/tests/test_app_requests.py -q

The SurrealDB side of the same routes is diffed against the files backend in db/tests/test_baseline.py and
db/tests/test_creative.py.
"""
from __future__ import annotations

import json
import sys
import urllib.request
from pathlib import Path
from urllib.parse import quote

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "db" / "tests"))
sys.path.insert(0, str(DESK / "api-dev"))
from common import Api, TempKB  # noqa: E402

NL = chr(10)
MABELS = "set:2024-07-19_FakeSet0006"
MOHEGAN = "set:2017-10-10_FakeSet0003"


@pytest.fixture(scope="module")
def api():
    t = TempKB("apireq", decisions=False)
    a = Api(t, "files")
    yield a, t
    a.stop()
    t.cleanup()


def q(s):
    return quote(s, safe=":")


def decisions(t):
    return [json.loads(x) for x in (t.kb / "decisions.jsonl").read_text(encoding="utf-8").splitlines() if x.strip()]


def test_analysis_summary_pass_through(api):
    a, t = api
    _, sets = a.get("/api/sets")
    sid = next(s["id"] for s in sets["sets"] if s["analysis"])
    _, sa = a.get(f"/api/sets/{q(sid)}/analysis")
    d = t.corpus / "sets" / (sid[4:8] + sid[9:11] + sid[12:14] + sid[14:]) / "analysis.json"
    raw = json.loads(d.read_text(encoding="utf-8"))["summary"]
    for k in ("laugh_s_per_min", "room_heat", "room_heat_basis", "pct_laughing", "laughs_per_min_merged",
              "longest_laugh_s", "longest_laugh_joke", "dead_air_max_s", "dead_air_max_at_ms", "material_s"):
        assert sa["summary"][k] == raw.get(k), k
    for r in sa["rows"]:
        for k in ("laugh_s_per_min", "vs_room", "time_to_first_laugh_s", "dead_air_max_s"):
            assert k in r


def test_setcard_fields(api):
    a, _ = api
    _, sets = a.get("/api/sets")
    by = {s["id"]: s for s in sets["sets"]}
    rd = by[MABELS]
    assert rd["show_date"] == "2024-07-13" and rd["show_date_source"].startswith("description")
    assert by[MOHEGAN]["show_date"] == "2017-08-16"
    for s in sets["sets"]:
        for k in ("show_date", "show_date_source", "description", "laugh_s_per_min", "room_heat", "audio_unreliable", "venue_id"):
            assert k in s, k
        assert s["audio_unreliable"] is None  # nobody marked anything yet
    _, venues = a.get("/api/venues")
    vids = {v["venue_id"] for v in venues["venues"]}
    assert {s["venue_id"] for s in sets["sets"] if s["analysis"]} <= vids


def test_set_edits_are_one_decision_each(api):
    a, t = api
    n0 = len(decisions(t))
    for after in ({"show_date": "2024-07-14"}, {"venue": "Mabel's"}, {"audio_unreliable": True}):
        st, r = a.post("/api/decision", {"target": "node", "action": "edit", "id": MABELS, "after": after})
        assert st == 200, r
    assert len(decisions(t)) == n0 + 3
    _, sets = a.get("/api/sets")
    rd = next(s for s in sets["sets"] if s["id"] == MABELS)
    assert rd["show_date"] == "2024-07-14" and rd["show_date_source"] == "yours"
    assert rd["venue"] == "Mabel's" and rd["audio_unreliable"] is True
    assert rd["venue_id"] == "venue:mabel-s-comedy-club"  # still the room /api/venues knows
    st, r = a.post("/api/undo", {"indexes": [n0, n0 + 1, n0 + 2]})
    assert st == 200
    _, sets = a.get("/api/sets")
    rd = next(s for s in sets["sets"] if s["id"] == MABELS)
    assert rd["show_date"] == "2024-07-13" and rd["audio_unreliable"] is None
    for bad in ({"show_date": "7/13/24"}, {"show_date": "2024-02-30"}, {"audio_unreliable": "yes"}, {"venue": ""},
                {"title": "x"}):
        st, _ = a.post("/api/decision", {"target": "node", "action": "edit", "id": MABELS, "after": bad})
        assert st == 400, bad


def test_performance_kind_banter(api):
    a, t = api
    _, sets = a.get("/api/sets")
    p = next(j["perf"] for s in sets["sets"] for j in s["jokes"] if j["kind"] != "banter")
    st, r = a.post("/api/decision", {"target": "node", "action": "edit", "id": p, "after": {"kind": "banter"}})
    assert st == 200 and r["decisions"][0]["after"] == {"kind": "banter"}
    st, _ = a.post("/api/decision", {"target": "node", "action": "edit", "id": p, "after": {"kind": "jazz"}})
    assert st == 400
    a.post("/api/undo", {"indexes": r["indexes"]})


def test_joke_summary_fields_and_punch_verbatim(api):
    a, _ = api
    _, page = a.get("/api/jokes?limit=5000")
    items = page["items"]
    assert len(items) == page["total"] > 500  # one page above 500
    with_punch = 0
    for j in items:
        for k in ("short_title", "punch", "nights", "avg_vs_room", "avg_laugh_s_per_min", "last_performed"):
            assert k in j, k
        assert "(" not in j["short_title"] and len(j["short_title"]) <= 28
        if j["punch"]:
            with_punch += 1
            _, d = a.get(f"/api/jokes/{q(j['id'])}") if with_punch <= 40 else (None, None)
            if d:
                texts = [u.get("text") or "" for g in d["versions"] for u in g["units"]] + [d["joke"].get("summary") or ""]
                assert any(j["punch"] in x for x in texts), (j["id"], j["punch"])
    assert with_punch > len(items) // 2
    performed = [j for j in items if j["nights"]]
    assert performed and any(j["avg_vs_room"] is not None for j in performed)
    assert all(j["last_performed"] for j in performed)


def test_sorts_facets_paging(api):
    a, _ = api
    _, base = a.get("/api/jokes?limit=5000")
    n = base["total"]
    for s in ("performed", "strongest", "written", "untouched"):
        st, r = a.get(f"/api/jokes?sort={s}&limit=5000")
        assert st == 200 and r["total"] == n and len(r["items"]) == n
        st, p2 = a.get(f"/api/jokes?sort={s}&limit=500&offset=500")
        assert [x["id"] for x in p2["items"]] == [x["id"] for x in r["items"][500:1000]]
    _, perf = a.get("/api/jokes?sort=performed&limit=50")
    lp = [x["last_performed"] for x in perf["items"] if x["last_performed"]]
    assert lp == sorted(lp, reverse=True) and lp
    _, strong = a.get("/api/jokes?sort=strongest&limit=50")
    vr = [x["avg_vs_room"] for x in strong["items"] if x["avg_vs_room"] is not None]
    assert vr == sorted(vr, reverse=True)
    assert a.get("/api/jokes?sort=bogus")[0] == 400
    _, meta = a.get("/api/meta?labels=12")
    l1, l2 = meta["top_labels"][0]["id"], meta["top_labels"][1]["id"]
    _, one = a.get(f"/api/jokes?label={q(l1)}&limit=5000")
    _, two = a.get(f"/api/jokes?label={q(l1)}&label={q(l2)}&limit=5000")
    _, other = a.get(f"/api/jokes?label={q(l2)}&limit=5000")
    assert {x["id"] for x in two["items"]} == {x["id"] for x in one["items"]} & {x["id"] for x in other["items"]}


def test_joke_punchup_never_blocks(api):
    a, _ = api
    _, page = a.get("/api/jokes?sort=performed&limit=5")
    jid = page["items"][0]["id"]
    st, r = a.post(f"/api/jokes/{q(jid)}/punchup", {})
    assert st == 202 and r["status"] == "queued" and r["job"]["kind"] == "punchup"
    st2, r2 = a.post(f"/api/jokes/{q(jid)}/punchup", {})
    assert st2 == 202 and r2["job"]["id"] == r["job"]["id"]  # the same job, not a second one
    _, never = a.get("/api/jokes?sort=untouched&limit=5000")
    untaped = next(x["id"] for x in never["items"] if not x["nights"])
    st, r = a.post(f"/api/jokes/{q(untaped)}/punchup", {})
    assert st == 200 and r["status"] == "no_tape"
    assert a.post("/api/jokes/joke:nope/punchup", {})[0] == 404


def test_try_notes_and_workingsets_key(api):
    a, _ = api
    _, lst = a.get("/api/workingsets")
    assert lst["workingsets"] == lst["working_sets"]
    if not lst["workingsets"]:
        _, page = a.get("/api/jokes?sort=performed&limit=3")
        st, b = a.post("/api/boards", {"title": "t", "joke_ids": [x["id"] for x in page["items"]]})
        st, ws = a.post("/api/workingsets", {"board_id": b["board"]["id"] if "board" in b else b["health"]["board"]})
        assert st == 201, ws
        _, lst = a.get("/api/workingsets")
    wid = lst["workingsets"][0]["id"]
    _, ws = a.get(f"/api/workingsets/{q(wid)}")
    jid = ws["bits"][0]["joke_id"]
    st, r = a.post(f"/api/workingsets/{q(wid)}/try", {"joke_id": jid, "text": "slow down on the reveal"})
    assert st == 201 and r["note"]["text"] == "slow down on the reveal" and r["note"]["done"] is False
    nid = r["note"]["id"]
    st, r = a.post(f"/api/workingsets/{q(wid)}/try/{q(nid)}", {"done": True}, method="PATCH")
    assert st == 200 and r["note"]["done"] is True and r["changed"]
    _, ws = a.get(f"/api/workingsets/{q(wid)}")
    mine = [n for n in ws["try_tonight"] if n["id"] == nid]
    assert mine and mine[0]["done"] is True and mine[0]["source"] == "his"
    ai = [n for n in ws["try_tonight"] if n["source"] != "his"]
    if ai:
        st, r = a.post(f"/api/workingsets/{q(wid)}/try/{quote(ai[0]['id'], safe='')}", {"done": True}, method="PATCH")
        assert st == 200
        _, ws = a.get(f"/api/workingsets/{q(wid)}")
        assert next(n for n in ws["try_tonight"] if n["id"] == ai[0]["id"])["done"] is True
    assert a.post(f"/api/workingsets/{q(wid)}/try", {"joke_id": jid, "text": ""})[0] == 400
    assert a.post(f"/api/workingsets/{q(wid)}/try/try:nope", {"done": True}, method="PATCH")[0] == 404


def test_upload_meta_headers(api):
    a, t = api
    req = urllib.request.Request(a.url + "/api/upload", data=b"RIFF0000WAVE", method="POST", headers={
        "Content-Type": "audio/wav", "X-Kind": "set", "X-Filename": "IMG_1.mov", "X-Recorded-At": "2026-10-24T21:41:00Z",
        "X-Venue": quote("Mabel's"), "X-Lat": "40.77", "X-Lng": "-73.95", "X-Show-Group": "g1",
        "X-Align-Working-Set": "ws:x"})
    with urllib.request.urlopen(req) as r:
        out = json.loads(r.read())
    meta = json.loads((t.inbox / (out["file"] + ".meta.json")).read_text(encoding="utf-8"))
    assert meta["recorded_at"] == "2026-10-24T21:41:00Z" and meta["venue"] == "Mabel's" and meta["show_group"] == "g1"
    assert meta["gps"] == {"lat": 40.77, "lon": -73.95} and meta["align_working_set"] == "ws:x"


def test_youtube_import_queues_only(api):
    a, t = api
    st, r = a.post("/api/import/youtube", {"url": "https://youtu.be/dQw4w9WgXcQ"})
    assert st == 202 and r["status"] == "queued" and r["job"]["kind"] == "import_youtube"
    _, j = a.get(f"/api/jobs/{q(r['job']['id'])}")
    assert j["status"] == "queued"  # nobody ran it here
    st, r = a.post("/api/import/youtube", {"url": "https://www.youtube.com/watch?v=FakeSet0006"})
    assert st == 200 and r["status"] == "exists"
    assert a.post("/api/import/youtube", {"url": "https://example.com/watch?v=dQw4w9WgXcQ"})[0] == 400


def test_settings(api):
    a, _ = api
    _, s = a.get("/api/settings")
    assert {k: s[k] for k in ("provider", "training", "upload_video", "cellular")} == \
        {"provider": "hosted", "training": False, "upload_video": False, "cellular": False}
    st, s = a.post("/api/settings", {"provider": "key", "cellular": True}, method="PATCH")
    assert st == 200 and s["provider"] == "key" and s["cellular"] is True and s["source"] == "his"
    assert a.get("/api/settings")[1]["provider"] == "key"
    for bad in ({"provider": "openai"}, {"training": "no"}, {"color": "red"}, {}):
        assert a.post("/api/settings", bad, method="PATCH")[0] == 400, bad


def test_export_learned_delete(api):
    a, t = api
    st, body, h = a.get("/api/export", raw=True)
    ex = json.loads(body)
    assert st == 200 and "attachment" in h.get("Content-Disposition", "")
    assert ex["decisions"] == decisions(t) and ex["sets"] and "creative" in ex
    st, learned = a.get("/api/learned")
    real = json.loads((t.kb / "learning.json").read_text(encoding="utf-8"))
    assert st == 200 and learned == real
    before = sorted(p.name for p in t.root.rglob("*"))
    assert a.post("/api/me", {}, method="DELETE")[0] == 400
    assert a.post("/api/me", {"confirm": "delete"}, method="DELETE")[0] == 400
    st, r = a.post("/api/me", {"confirm": "delete my material"}, method="DELETE")
    assert st == 200 and r["status"] == "dry_run" and r["deleted"] is False and r["would_delete"]
    assert sorted(p.name for p in t.root.rglob("*")) == before  # nothing on disk changed


def test_connections_status(api):
    """Settings > Connections (GET /api/connections): read-only, counts only, from gsync's own files."""
    a, t = api
    g = t.root / "gsync"
    st, c = a.get("/api/connections")
    assert st == 200
    assert c["google_docs"]["status"] == "not_set_up" and c["google_docs"]["docs_linked"] == 0
    assert c["ai"]["worker"]["online"] is False and c["ai"]["worker"]["last_seen"] is None
    assert c["ai"]["provider"] in ("hosted", "chatgpt", "key", "custom")
    assert c["audio_storage"]["status"] == "not_set_up"
    assert c["youtube"]["sets"] >= 7 and c["youtube"]["import"] == "link"
    # gsync has run: 3 docs (one hidden), a board, one conflict, a dry run that must not count
    g.mkdir(exist_ok=True)
    (g / "map.json").write_text(json.dumps({"docs": {
        "d1": {"origin": "gdocs", "title": "SECRET TITLE"}, "d2": {"origin": "corpus"},
        "d3": {"origin": "app", "hidden": True}}, "boards": {"sb:x": {"doc_id": "b1"}},
        "excluded": {"legal1": "folder"}}), encoding="utf-8")
    from datetime import datetime, timezone
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    runs = [{"run_id": "gs-1", "ts": "2026-01-01T00:00:00Z", "mode": "pull", "errors": [{"error": "x"}]},
            {"run_id": "gs-2", "ts": now, "ts_end": now, "mode": "pull", "errors": []},
            {"run_id": "gs-3", "ts": now, "mode": "pull", "dry_run": True, "errors": [{"error": "dry"}]}]
    (g / "runs.jsonl").write_text(NL.join(json.dumps(r) for r in runs) + NL + "{torn", encoding="utf-8")
    (g / "conflicts.jsonl").write_text('{"kind": "own_doc_not_written"}' + NL, encoding="utf-8")
    _, c = a.get("/api/connections")
    gd = c["google_docs"]
    assert gd["status"] == "connected", gd
    assert (gd["docs_linked"], gd["joke_docs"], gd["boards"], gd["conflicts"]) == (2, 1, 1, 1)
    assert gd["last_sync"] == now and gd["last_ok"] == now and gd["errors_last_run"] == 0
    body = json.dumps(c)
    assert "SECRET TITLE" not in body and "legal1" not in body and "excluded" not in body
    # the last real run failed -> "error"; a run a week old -> "stale"; the local API was down -> "api_down"
    for run, want in (({"ts": now, "mode": "pull", "errors": [{"error": "boom"}]}, "error"),
                      ({"ts": "2020-01-01T00:00:00Z", "mode": "pull", "errors": []}, "stale"),
                      ({"ts": now, "mode": "api_down", "errors": []}, "api_down")):
        (g / "runs.jsonl").write_text(json.dumps(run) + NL, encoding="utf-8")
        assert a.get("/api/connections")[1]["google_docs"]["status"] == want, want
    # the home worker asks for a job -> online, with when and which machine
    st, _ = a.post("/api/worker/claim", {"worker": "HOME-PC"})
    assert st in (200, 204)
    w = a.get("/api/connections")[1]["ai"]["worker"]
    assert w["online"] is True and w["name"] == "HOME-PC" and w["last_seen"]
