"""Your sets (creative/mysets.py): what's working per bit, tough spots and their actions, trim, matching a recording
to the set it resembles, version notes, and that every existing working set is a set (nothing to migrate)."""
import json
import re

import pytest

from creative import mysets as M
from creative import story as S
from creative import workingset as W
from creative.data import Corpus

from .conftest import COMEDY, HAS_REAL, TEXT, _analysis, _seg, make_analyses, make_graph

TS = "2026-10-26T12:00:00+00:00"
BANNED = re.compile(r"\b(edges?|inferred|proposed|vs_room|confidence|pipeline|units?)\b|\d\.\d+\s*x|laugh-s", re.I)


def words_of(obj):
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k in ("words", "label_words", "last_words", "status_words", "why", "health", "note") and isinstance(v, str):
                yield v
            yield from words_of(v)
    elif isinstance(obj, list):
        for v in obj:
            yield from words_of(v)


@pytest.fixture
def store(tmp_path):
    return W.ws_store(tmp_path)


def test_labels_from_tape(corpus):
    lab = {j: M.bit_label(corpus, f"joke:{j}") for j in "abcdef"}
    # a: 1.5, 1.4, 1.6 times the room on 3 nights
    assert lab["a"]["label"] == "killed" and lab["a"]["label_words"] == "Killed every night · 3 nights"
    # b: as loud as the room once, then well under it twice
    assert lab["b"]["label"] == "quiet" and lab["b"]["label_words"] == "Quiet 2 of 3 nights"
    assert lab["c"]["label"] == "solid" and lab["c"]["counted"] == 1
    assert lab["d"]["label"] == "quiet" and lab["d"]["label_words"].startswith("Some laughs, never a big one")
    assert lab["e"]["label"] == "untested" and lab["e"]["label_words"] == "Not on stage yet"
    assert lab["a"]["last_words"].startswith("Last time: killed")


def test_dead_room_nights_do_not_count():
    an = make_analyses()
    an["2024-04-01_w"] = _analysis("2024-04-01_w", 1.0, [_seg(1, "joke:e", 0, 60, TEXT["e"], 5.0, 1.0)])
    c = Corpus(make_graph(), an)
    lab = M.bit_label(c, "joke:e")
    assert lab["nights"] == 1 and lab["counted"] == 0 and lab["label"] == "untested"
    assert "barely laughed" in lab["label_words"]


def test_mixed_and_dropping():
    an = make_analyses()
    an["2024-04-01_w"] = _analysis("2024-04-01_w", 6.0, [_seg(1, "joke:c", 0, 60, TEXT["c"], 0.0, 6.0, ttfl=None)])
    c = Corpus(make_graph(), an)
    assert M.bit_label(c, "joke:c")["label"] == "mixed"
    ws = {"id": "ws:t", "title": "t", "locks": {}, "versions": [{"v": 1, "target_s": 600, "bits": [
        {"joke_id": j, "title": j, "target_s": 60} for j in ("joke:a", "joke:c")]}]}
    spots = M.tough_spots(c, ws)
    kinds = [s["kind"] for s in spots["bits"]["joke:c"]]
    assert "dropping" in kinds
    drop = next(s for s in spots["bits"]["joke:c"] if s["kind"] == "dropping")
    assert drop["words"].startswith("Dropping lately: quiet")


def _ws(store, corpus, ids, target=600, title="Set"):
    return M.create(corpus, store, title, target, ids, ts=TS)


def test_tough_spots_and_actions(corpus, store):
    ws = _ws(store, corpus, ["joke:b", "joke:c", "joke:e"])
    sp = M.tough_spots(corpus, ws)
    b = sp["bits"]["joke:b"]
    assert [s["kind"] for s in b] == ["quiet"]
    swaps = [a for a in b[0]["actions"] if a["kind"] == "swap"]
    assert swaps and swaps[0]["with"]["joke_id"] == "joke:a"          # the bit that kills, not in the set
    assert swaps[0]["with"]["label"] == "killed"
    tr = sp["bits"]["joke:e"][0]
    assert tr["kind"] == "transition" and tr["from"] == "joke:c"       # c -> e: nothing links them
    assert any(a["kind"] == "bridge" and a["from"] == "joke:c" and a["to"] == "joke:e" for a in tr["actions"])
    assert sp["set"] and sp["set"][0]["kind"] == "short"               # ~3 min of a 10
    assert M.health(corpus, ws)["word"] == f"{M.health(corpus, ws)['tough_n']} tough spots"
    assert not [w for w in words_of(M.view(corpus, ws)) if BANNED.search(w)]


def test_locked_bit_gets_no_spots(corpus, store):
    ws = _ws(store, corpus, ["joke:b", "joke:c"])
    assert M.tough_spots(corpus, ws)["bits"]["joke:b"]
    ws = W.lock_bit(store, ws["id"], "joke:b", ts=TS)
    assert M.tough_spots(corpus, ws)["bits"]["joke:b"] == []
    assert M.view(corpus, ws)["bits"][0]["locked"] is True


def test_over_time_offers_a_cut_of_the_weakest(corpus, store):
    ws = _ws(store, corpus, ["joke:a", "joke:b", "joke:d", "joke:c"], target=120)
    over = next(s for s in M.tough_spots(corpus, ws)["set"] if s["kind"] == "over")
    cuts = [a["joke_id"] for a in over["actions"] if a["kind"] == "cut"]
    assert cuts and set(cuts) <= {"joke:b", "joke:d"}                  # never the opener or the closer
    assert over["words"].endswith("over your 2 min")


def test_move_suggests_a_proven_neighbour(corpus, store):
    # b is between two bits it has no link with; b -> a is done back to back on tape, c -> b is only a segue
    ws = _ws(store, corpus, ["joke:c", "joke:e", "joke:b", "joke:f", "joke:a"])
    tr = M._T(corpus)
    m = M.move_option(corpus, M.ids_of(ws), "joke:b", tr)
    assert m and m["kind"] == "move" and "joke:b" == m["joke_id"]
    seq = [x for x in M.ids_of(ws) if x != "joke:b"]
    seq.insert(m["to"], "joke:b")
    assert seq[m["to"] + 1] == "joke:a" and m["words"].startswith("Move it after F bit")  # right before a, as on tape
    ws2 = _ws(store, corpus, ["joke:c", "joke:b", "joke:a", "joke:e"], title="placed")
    assert M.move_option(corpus, M.ids_of(ws2), "joke:b", M._T(corpus)) is None  # already where it links best


def test_trim_keeps_opener_closer_and_the_strongest(corpus, store):
    ws = _ws(store, corpus, ["joke:c", "joke:b", "joke:a", "joke:d", "joke:e"], target=600)
    t = M.trim(corpus, ws, 140)
    keep = [k["joke_id"] for k in t["keep"]]
    assert keep == ["joke:c", "joke:a", "joke:e"]                      # opener, the one that kills, closer
    assert t["runtime_s"] <= 140 and t["status"] == "on"
    t2 = M.trim(corpus, ws, 240)                                       # room for more: a quiet bit only to reach the length
    assert [k["joke_id"] for k in t2["keep"]] == ["joke:c", "joke:b", "joke:a", "joke:e"]
    assert [k["joke_id"] for k in t2["cut"]] == ["joke:d"]
    an = make_analyses()                                               # a short quiet bit that would fit: left out
    an["2024-04-01_w"] = _analysis("2024-04-01_w", 6.0, [_seg(1, "joke:f", 0, 10, TEXT["f"], 0.0, 6.0, ttfl=None)])
    c2 = Corpus(make_graph(), an)
    t3 = M.trim(c2, _ws(store, c2, ["joke:c", "joke:a", "joke:f", "joke:e"], title="q"), 140)
    assert [k["joke_id"] for k in t3["keep"]] == ["joke:c", "joke:a", "joke:e"]


def test_match_recording_to_the_set_it_resembles(corpus, store):
    x = _ws(store, corpus, ["joke:a", "joke:b", "joke:c"], title="X")
    y = _ws(store, corpus, ["joke:d", "joke:e"], title="Y")
    m = M.match_recording(corpus, [x, y], "2024-01-01_x")
    assert m["match"]["id"] == x["id"] and m["match"]["how"] == "closest" and m["match"]["shared"] == 3
    m2 = M.match_recording(corpus, [x, y], "2024-02-01_y")             # a, b, d: still X (2 of 3) over Y (1)
    assert m2["match"]["id"] == x["id"]
    assert M.tapes_for(corpus, [x, y], y) == []
    assert M.tapes_summary(corpus, [x, y], x)["n"] == 3
    # his pick at import wins
    c2 = Corpus(make_graph(), make_analyses(), metas={"2024-01-01_x": {"align_working_set": y["id"]}})
    assert M.match_recording(c2, [x, y], "2024-01-01_x")["match"]["how"] == "his pick"


def test_a_new_recording_updates_the_set():
    g, an = make_graph(), make_analyses()
    c0 = Corpus(g, an)
    ws = {"id": "ws:t", "title": "t", "locks": {}, "updated_at": TS, "versions": [{"v": 1, "target_s": 600, "bits": [
        {"joke_id": j, "title": j, "target_s": 60} for j in ("joke:d", "joke:e", "joke:f")]}]}
    assert M.tapes_summary(c0, [ws], ws)["n"] == 0
    assert M.bit_label(c0, "joke:e")["label"] == "untested"
    an["2024-05-01_v"] = _analysis("2024-05-01_v", 6.0, [_seg(1, "joke:d", 0, 60, TEXT["d"], 9.0, 6.0),
                                                          _seg(2, "joke:e", 60, 60, TEXT["e"], 9.0, 6.0)])
    c1 = Corpus(g, an)
    t = M.tapes_summary(c1, [ws], ws)
    assert t["n"] == 1 and t["last"]["set"] == "2024-05-01_v" and t["last"]["bits_done"] == 2
    assert M.bit_label(c1, "joke:e")["label"] == "killed"


def test_change_note_says_what_changed(corpus):
    n = M.change_note
    assert n(corpus, ["joke:a", "joke:b", "joke:c"], ["joke:b", "joke:c", "joke:a"], 600, 600) == "Moved A bit to #3"
    assert n(corpus, ["joke:a", "joke:b"], ["joke:a", "joke:d"], 600, 600) == "Swapped B bit for D bit"
    assert n(corpus, ["joke:a"], ["joke:a", "joke:c"], 600, 300) == "Added C bit; Length 5 min (was 10 min)"
    assert n(corpus, ["joke:a", "joke:c"], ["joke:a"], 600, 600) == "Cut C bit"


def test_versions_replay_and_an_emptied_set(corpus, store, tmp_path):
    ws = _ws(store, corpus, ["joke:a", "joke:b"])
    W.new_version(store, ws["id"], "Moved", bits=list(reversed(W.current(ws)["bits"])), ts=TS)
    store.append("new_version", ws["id"], {"note": "Cut everything", "bits": []}, ts=TS)
    again = W.ws_store(tmp_path)
    w = again.get(ws["id"])
    assert [v["v"] for v in w["versions"]] == [1, 2, 3] and W.current(w)["bits"] == []
    assert [b["joke_id"] for b in w["versions"][1]["bits"]] == ["joke:b", "joke:a"]
    v = M.view(corpus, w)
    assert v["health"] == "empty" and [x["v"] for x in v["versions"]] == [3, 2, 1]


def test_existing_working_sets_are_sets(corpus, tmp_path):
    """Nothing to migrate: a working set promoted from a board before this existed is listed and opens as a set,
    with its versions, locks and try notes intact, and the board itself is untouched."""
    boards, wss = S.board_store(tmp_path), W.ws_store(tmp_path)
    b = S.create(boards, "Robot vacuum", ["joke:a", "joke:b", "joke:c"], target_s=420, ts=TS, id_="sb:ai")
    ws = W.promote(corpus, wss, b, ts=TS, id_="ws:old")
    ws = W.lock_bit(wss, "ws:old", "joke:a", ts=TS)
    ws, _ = W.add_try(wss, "ws:old", "joke:b", "slower on the reveal", ts=TS)
    before = json.dumps(boards.items, sort_keys=True)
    s = M.summary(corpus, ws)
    assert s["id"] == "ws:old" and s["n"] == 3 and s["storyboard_id"] == "sb:ai" and s["made_by"] == "you"
    v = M.view(corpus, ws)
    assert v["bits"][0]["locked"] and v["versions"][0]["note"].startswith("v1: promoted")
    assert W.ws_store(tmp_path).get("ws:old")["try_notes"][0]["text"] == "slower on the reveal"
    assert json.dumps(S.board_store(tmp_path).items, sort_keys=True) == before


@pytest.mark.skipif(not HAS_REAL, reason="real KB not on this machine")
def test_real_labels_match_the_tapes(real):
    """Three labels checked against the set analysis files themselves (not through the corpus)."""
    raw = {}
    for d in sorted((COMEDY / "corpus" / "sets").iterdir()):
        p = d / "analysis.json"
        if p.is_file():
            a = json.loads(p.read_text(encoding="utf-8"))
            for s in a["jokes"]:
                if s.get("joke_id"):
                    raw.setdefault(real.map_joke(s["joke_id"]), []).append((a["set"], a["summary"]["laugh_s_per_min"], s))
    # Parking meter: every taped night at least 1.2 times the room's own laugh rate -> killed every night
    bh = [s for _, rate, s in raw["joke:parking-meter"] if rate >= M.DEAD_ROOM]
    assert len(bh) >= 5 and all(s["vs_room"] >= 1.2 for s in bh)
    assert M.bit_label(real, "joke:parking-meter")["label"] == "killed"
    # Budget airline: one night, no laugh at all
    dt = raw["joke:budget-airline"]
    assert len(dt) == 1 and dt[0][2]["laughs"]["laugh_s"] == 0
    assert M.bit_label(real, "joke:budget-airline")["label"] == "quiet"
    # The dog is the size of a couch: killed at The Corner Room in 2015, nearly silent in 2025 -> mixed, and dropping in a set
    rex = sorted(raw["joke:dog-couch-size"], key=lambda x: x[0])
    assert max(s["vs_room"] for _, _, s in rex[:3]) >= 1.2 and rex[-1][2]["vs_room"] < 0.5
    assert M.bit_label(real, "joke:dog-couch-size")["label"] == "mixed"
    ws = {"id": "ws:t", "title": "t", "locks": {}, "versions": [{"v": 1, "target_s": 600, "bits": [
        {"joke_id": j, "title": j, "target_s": 60} for j in ("joke:parking-meter", "joke:dog-couch-size")]}]}
    assert "dropping" in [s["kind"] for s in M.tough_spots(real, ws, with_actions=False)["bits"]["joke:dog-couch-size"]]
