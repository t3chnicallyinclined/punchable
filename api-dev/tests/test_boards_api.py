"""Set lists over the API (api-dev/creative_api.py), on a TEMP copy of the data (files backend): the list is his
(the seeded drafts are labelled from the data, not hidden by a guess), a board can be archived and brought back
(undo), the refusals are plain words, delete stays terminal, tonight's list is one of his or nothing, and the stage
build carries each bit's notes (`text`, `idea`) so the stage screen opens a bit without a call per bit.

    python -m pytest api-dev/tests/test_boards_api.py -q
"""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from urllib.parse import quote

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "db" / "tests"))
sys.path.insert(0, str(DESK / "api-dev"))
sys.path.insert(0, str(DESK))
from common import Api, TempKB  # noqa: E402

import creative_api as CR  # noqa: E402
from creative import story as S  # noqa: E402

OWNER = os.environ.get("PK_OWNER_ID", "owner")
SEED_ID = "sb:seed-test-00000000"
TS = "2026-10-26T12:00:00+00:00"


def q(s):
    return quote(s, safe=":")


def _two_jokes(graph: Path) -> list[str]:
    g = json.loads(graph.read_text(encoding="utf-8"))
    return [n["id"] for n in g["nodes"] if n.get("type") == "joke"][:2]


@pytest.fixture(scope="module")
def api():
    t = TempKB("boards", decisions=False)
    # one machine-made draft, exactly as story.seed_from_insights writes it, so the test does not depend on what the
    # copied data holds (the snapshot is derived: the API replays the log)
    seed = {"ts": TS, "op": "create", "id": SEED_ID, "by": "ai:seed",
            "data": {"title": "Seed test", "joke_ids": _two_jokes(t.graph), "target_s": 300, "notes": "Order is a draft.",
                     "seed": {"from": "insights.chunks", "chunk": "Seed test", "minutes_est": 5}}}
    with open(t.creative / "storyboards.jsonl", "a", encoding="utf-8") as f:
        f.write(json.dumps(seed) + "\n")
    a = Api(t, "files")
    yield a, t
    a.stop()
    t.cleanup()


def events(t: TempKB) -> list[dict]:
    return [json.loads(x) for x in (t.creative / "storyboards.jsonl").read_text(encoding="utf-8").splitlines() if x.strip()]


def board_in(a: Api, bid: str, archived: bool = False):
    _, bl = a.get("/api/boards" + ("?archived=1" if archived else ""))
    assert bl["archived"] is archived
    return next((b for b in bl["boards"] if b["id"] == bid), None)


# ------------------------------------------------------------------------------------------------ tonight is his
def test_tonight_board_is_his_or_nothing(tmp_path):
    """No corpus is touched when he has no board of his own: a seeded draft is never tonight's list."""
    bs = S.board_store(tmp_path)
    S.create(bs, "Kids", ["joke:a", "joke:b"], 300, by="ai:seed", ts=TS, id_="sb:seed", seed={"from": "insights.chunks"})
    assert CR.tonight_board(None, {"storyboards": bs}) is None
    assert CR.own_boards({"storyboards": bs}) == []
    S.create(bs, "Mine", ["joke:a"], 300, by=OWNER, ts=TS, id_="sb:mine")
    assert [b["id"] for b in CR.own_boards({"storyboards": bs})] == ["sb:mine"]
    S.archive(bs, "sb:mine", by=OWNER, ts=TS)  # put away = not tonight's either
    assert CR.tonight_board(None, {"storyboards": bs}) is None
    S.unarchive(bs, "sb:mine", by=OWNER, ts=TS)
    S.create(bs, "Empty", [], 300, by=OWNER, ts=TS, id_="sb:empty")  # no bits: nothing to build
    assert [b["id"] for b in CR.own_boards({"storyboards": bs})] == ["sb:mine", "sb:empty"]


def test_tonight_screen_and_put_in_tonight_with_no_own_board(api):
    """With every board of his put away: the Tonight screen says no list yet (board null), and a typed
    "put X in tonight" makes him "<Weekday> · tonight" instead of writing onto a seeded draft."""
    a, t = api
    _, bl = a.get("/api/boards")
    mine = [b["id"] for b in bl["boards"] if b["source"] == "his"]
    for bid in mine:
        st, _ = a.post(f"/api/boards/{q(bid)}", {"archive": True}, method="PATCH")
        assert st == 200
    seed = board_in(a, SEED_ID)
    assert seed and seed["source"] == "inferred" and seed["by"] == "ai:seed" and seed["seed"]["from"] == "insights.chunks"
    st, s = a.get("/api/tonight/screen")
    assert st == 200 and s["board"] is None and 1 <= len(s["questions"]) <= 3
    _, page = a.get("/api/jokes?sort=performed&limit=1")
    j = page["items"][0]
    st, r = a.post("/api/say", {"text": "add it to tonight", "choice": j["id"]})
    assert st == 200 and r["command"]["op"] == "add_to_tonight" and r["command"]["done"] is True, r
    assert r["command"]["created"] is True and r["command"]["board"]["title"].endswith(" · tonight")
    bid = r["command"]["board"]["id"]
    assert bid != SEED_ID and r["command"]["undo"] == {"PATCH": f"/api/boards/{bid}", "body": {"delete": True}}
    row = board_in(a, bid)
    assert row and row["source"] == "his" and row["by"] == OWNER and row["seed"] is None and row["names"]
    assert [e for e in events(t) if e["id"] == bid][0]["by"] == OWNER
    assert board_in(a, SEED_ID)["n"] == 2  # the draft was not written to
    st, s = a.get("/api/tonight/screen")
    assert st == 200 and s["board"]["id"] == bid and len(s["board"]["segments"]) == 1
    # put his boards back the way they were
    for bid2 in mine:
        assert a.post(f"/api/boards/{q(bid2)}", {"unarchive": True}, method="PATCH")[0] == 200


# ------------------------------------------------------------------------------------------------ archive / unarchive
def test_archive_is_undoable_refusals_are_plain_and_delete_is_terminal(api):
    a, t = api
    _, page = a.get("/api/jokes?sort=performed&limit=3")
    j0, j1, j2 = [x["id"] for x in page["items"]]
    st, b = a.post("/api/boards", {"title": "Mine", "joke_ids": [j0, j1], "target_s": 300})
    assert st == 201
    bid = b["board"]["id"]
    row = board_in(a, bid)
    assert row["archived"] is False and row["source"] == "his" and row["by"] == OWNER and row["seed"] is None
    assert set(row) >= {"id", "title", "n", "runtime_s", "runtime", "target_s", "updated", "source", "names"}
    # archive: gone from the list, on the archived list, still openable; the answer is the board view
    st, v = a.post(f"/api/boards/{q(bid)}", {"archive": True}, method="PATCH")
    assert st == 200 and v["board"]["archived"] is True and v["board"]["source"] == "his" and v["board"]["deleted"] is False
    assert board_in(a, bid) is None and board_in(a, bid, archived=True)["archived"] is True
    assert board_in(a, SEED_ID, archived=True) is None
    st, v = a.get(f"/api/boards/{q(bid)}")
    assert st == 200 and v["board"]["archived"] is True
    assert a.get(f"/api/boards/{q(bid)}/build")[0] == 200
    # every other change is refused in plain words, nothing written
    n = len(events(t))
    for body in ({"add": j2}, {"remove": j0}, {"move": {"from": 0, "to": 1}}, {"title": "x"}, {"joke_ids": [j1]},
                 {"notes": "n"}, {"target_s": 600}):
        st, e = a.post(f"/api/boards/{q(bid)}", body, method="PATCH")
        assert st == 400 and e["error"] == '"Mine" is archived: unarchive it first', (body, e)
    st, e = a.post(f"/api/boards/{q(bid)}", {"archive": True}, method="PATCH")
    assert st == 400 and e["error"] == '"Mine" is already archived'
    st, e = a.post(f"/api/boards/{q(bid)}/items", {"joke_id": j2})
    assert st == 400 and e["error"] == '"Mine" is archived: unarchive it first'
    assert len(events(t)) == n
    # undo: unarchive brings it back whole; a combined body brings it back and edits it in one go
    st, v = a.post(f"/api/boards/{q(bid)}", {"unarchive": True, "title": "Mine 2"}, method="PATCH")
    assert st == 200 and v["board"]["archived"] is False and v["board"]["title"] == "Mine 2" and v["board"]["joke_ids"] == [j0, j1]
    assert board_in(a, bid)["title"] == "Mine 2" and board_in(a, bid, archived=True) is None
    st, e = a.post(f"/api/boards/{q(bid)}", {"unarchive": True}, method="PATCH")
    assert st == 400 and e["error"] == '"Mine 2" is not archived'
    ops = [(e["op"], e["by"]) for e in events(t) if e["id"] == bid]
    assert ops == [("create", OWNER), ("archive", OWNER), ("unarchive", OWNER), ("update", OWNER)]
    # an empty body is refused and the message lists archive and unarchive
    st, e = a.post(f"/api/boards/{q(bid)}", {}, method="PATCH")
    assert st == 400 and "archive, unarchive, delete" in e["error"]
    # archiving a seeded draft does not make it his (and unarchiving brings it back as a draft)
    st, v = a.post(f"/api/boards/{q(SEED_ID)}", {"archive": True}, method="PATCH")
    assert st == 200 and v["board"]["archived"] is True and v["board"]["source"] == "inferred"
    assert board_in(a, SEED_ID) is None
    st, v = a.post(f"/api/boards/{q(SEED_ID)}", {"unarchive": True}, method="PATCH")
    assert st == 200 and v["board"]["source"] == "inferred" and board_in(a, SEED_ID)["source"] == "inferred"
    # delete is allowed on an archived board and stays terminal: no list shows it, nothing brings it back
    assert a.post(f"/api/boards/{q(bid)}", {"archive": True}, method="PATCH")[0] == 200
    st, d = a.post(f"/api/boards/{q(bid)}", {"delete": True}, method="PATCH")
    assert st == 200 and d == {"ok": True, "board": {"id": bid, "title": "Mine 2", "deleted": True}}
    assert board_in(a, bid) is None and board_in(a, bid, archived=True) is None
    assert a.get(f"/api/boards/{q(bid)}")[0] == 404
    assert a.post(f"/api/boards/{q(bid)}", {"unarchive": True}, method="PATCH")[0] == 404
    assert a.get("/api/boards?archived=0")[1]["archived"] is False


# ------------------------------------------------------------------------------------------------ the stage build
def test_build_steps_carry_the_bits_notes(api):
    a, _ = api
    _, page = a.get("/api/jokes?sort=written&limit=1")
    written = page["items"][0]["id"]
    st, w = a.post("/api/ideas", {"text": "Zorblax runs the valet stand. He parks every car in the same spot.", "reason": "test"})
    assert st == 200
    idea = w["id"]
    st, b = a.post("/api/boards", {"title": "Notes on stage", "joke_ids": [written, idea], "target_s": 300})
    assert st == 201
    st, v = a.get(f"/api/boards/{q(b['board']['id'])}/build")
    assert st == 200 and [s["joke_id"] for s in v["steps"]] == [written, idea]
    s0, s1 = v["steps"]
    # the written bit: the same text the joke page shows (one function), no idea
    _, jd = a.get(f"/api/jokes/{q(written)}")
    assert set(s0["text"]) == {"setup", "reveal", "from"} and s0["text"] == jd["text"]
    assert s0["text"]["from"] is not None and s0["idea"] is None
    assert len(s0["text"]["setup"]) + len(s0["text"]["reveal"]) <= 902
    # the typed idea: no written version yet, so `idea` is his words and `text` is its split (as the joke page does)
    assert s1["text"]["from"] is None and s1["idea"] == "Zorblax runs the valet stand. He parks every car in the same spot."
    assert s1["text"] == {"setup": "Zorblax runs the valet stand.", "reveal": "He parks every car in the same spot.", "from": None}
    # additive: every key the stage screen already reads is still there
    for s in (s0, s1):
        assert set(s) >= {"joke_id", "title", "short_name", "quote", "quote_node", "stage_line", "proven", "nights",
                          "duration_s", "transition_in", "position", "text", "idea"}
