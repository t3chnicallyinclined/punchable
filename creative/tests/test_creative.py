import json

import pytest

from creative import connections as C
from creative import story as S
from creative import venues as V
from creative import workingset as W
from creative.data import Corpus

from .conftest import FakeLLM, make_analyses, make_graph, walk_evidence

TS = "2026-10-26T12:00:00+00:00"


def board(ids, target=0, id_="sb:t"):
    return {"id": id_, "title": "t", "joke_ids": ids, "target_s": target}


# ---------------------------------------------------------------- transition labels
def test_transition_labels(corpus):
    assert S.transition(corpus, "joke:a", "joke:b")["label"] == "proven"          # back-to-back in 2 sets
    assert len(S.transition(corpus, "joke:a", "joke:b")["nights"]) == 2
    assert S.transition(corpus, "joke:b", "joke:c")["label"] == "link"            # segue edge; crowd work in between
    assert S.transition(corpus, "joke:d", "joke:e")["label"] == "link"            # shared persona
    assert S.transition(corpus, "joke:d", "joke:e")["needs_bridge"] is True
    assert S.transition(corpus, "joke:c", "joke:e")["label"] == "jump"
    assert S.transition(corpus, "joke:b", "joke:d")["label"] == "proven"
    # crowd work breaks back-to-back: b -> c is not proven
    assert ("joke:b", "joke:c") not in corpus.back_to_back


def test_shared_topic_is_a_link_that_needs_a_bridge():
    g = make_graph()
    g["edges"].append({"id": "e:lf", "src": "joke:f", "dst": "label:topic:kids", "kind": "has_label",
                       "status": "proposed", "confidence": 0.8, "evidence": [{"node": "unit:f1", "quote": "planes"}]})
    c = Corpus(g, make_analyses())
    t = S.transition(c, "joke:e", "joke:f")
    assert t["label"] == "link" and t["needs_bridge"] and t["via"][0]["kind"] == "topic"


def test_seed_reorders_only_untouched_ai_boards(corpus, tmp_path):
    corpus.insights = {"chunks": {"chunks": [{"name": "kids", "fits_5_15": True, "minutes_est": 6,
                                               "jokes": ["joke:b", "joke:d", "joke:a"]}]}}
    bs = S.board_store(tmp_path)
    b = S.seed_from_insights(corpus, bs, ts=TS)[0]
    assert b["source"] == "inferred" and b["by"] == "ai:seed" and sorted(b["joke_ids"]) == ["joke:a", "joke:b", "joke:d"]
    assert b["joke_ids"][-1] == "joke:a"  # the strongest taped bit closes
    S.set_jokes(bs, b["id"], ["joke:d", "joke:a", "joke:b"], ts=TS)  # the comic reorders it
    again = S.seed_from_insights(corpus, bs, ts=TS)[0]
    assert again["joke_ids"] == ["joke:d", "joke:a", "joke:b"] and again["source"] == "his"


def test_seed_leaves_an_archived_draft_alone(corpus, tmp_path):
    corpus.insights = {"chunks": {"chunks": [{"name": "kids", "fits_5_15": True, "minutes_est": 6,
                                               "jokes": ["joke:b", "joke:d", "joke:a"]}]}}
    bs = S.board_store(tmp_path)
    b = S.seed_from_insights(corpus, bs, ts=TS)[0]
    S.set_jokes(bs, b["id"], ["joke:d", "joke:a", "joke:b"], by="ai:seed", ts=TS)  # still untouched by him
    S.archive(bs, b["id"], ts=TS)  # he puts the draft away: the seeder must not try to reorder it (a refused op)
    again = S.seed_from_insights(corpus, bs, ts=TS)[0]
    assert again["archived"] is True and again["joke_ids"] == ["joke:d", "joke:a", "joke:b"]
    assert again["source"] == "inferred" and again["by"] == "ai:seed"  # archiving is not building: source unchanged
    assert bs.n_events == 3


def test_rejected_edge_is_not_a_link():
    g = make_graph()
    decisions = [{"target": "edge", "id": "e:seg_bc", "action": "reject", "by": "owner"}]
    c = Corpus(g, make_analyses(), decisions=decisions)
    assert S.transition(c, "joke:b", "joke:c")["label"] == "jump"


def test_confirmed_edge_is_his():
    decisions = [{"target": "edge", "id": "e:seg_bc", "action": "confirm", "after": {"status": "confirmed"}, "by": "owner"}]
    c = Corpus(make_graph(), make_analyses(), decisions=decisions)
    nxt = C.joke_connections(c, "joke:b")["groups"][0]["items"]
    assert any(i["joke_id"] == "joke:c" and i["source"] == "his" for i in nxt)


# ---------------------------------------------------------------- health rules: fire and stay silent
def _check(h, rule):
    return next(x for x in h["checks"] if x["rule"] == rule)


def test_health_runtime(corpus):
    total = S.health(corpus, board(["joke:a", "joke:b"]))["runtime_s"]
    assert _check(S.health(corpus, board(["joke:a", "joke:b"], int(total))), "runtime")["status"] == "ok"
    assert _check(S.health(corpus, board(["joke:a", "joke:b"], 30)), "runtime")["status"] == "warn"
    assert _check(S.health(corpus, board(["joke:a", "joke:b"], 3000)), "runtime")["status"] == "info"


def test_health_jumps(corpus):
    assert _check(S.health(corpus, board(["joke:a", "joke:b", "joke:c"])), "jumps")["status"] == "ok"
    h = S.health(corpus, board(["joke:c", "joke:e"]))
    assert _check(h, "jumps")["status"] == "warn" and _check(h, "jumps")["pairs"] == [["joke:c", "joke:e"]]


def test_health_repeated_devices(corpus):
    assert _check(S.health(corpus, board(["joke:a", "joke:b"])), "repeated_devices")["status"] == "warn"
    assert _check(S.health(corpus, board(["joke:b", "joke:c"])), "repeated_devices")["status"] == "ok"
    assert _check(S.health(corpus, board(["joke:a", "joke:c", "joke:b"])), "repeated_devices")["status"] == "ok"


def test_health_closer(corpus):
    assert _check(S.health(corpus, board(["joke:b", "joke:a"])), "closer")["status"] == "ok"
    h = S.health(corpus, board(["joke:a", "joke:b"]))
    assert _check(h, "closer")["status"] == "warn" and _check(h, "closer")["suggest"] == "joke:a"
    assert "no tape" in _check(S.health(corpus, board(["joke:a", "joke:f"])), "closer")["text"]


def test_health_energy_dip(corpus):
    assert _check(S.health(corpus, board(["joke:b", "joke:d"])), "energy_dip")["status"] == "warn"
    assert _check(S.health(corpus, board(["joke:b", "joke:a", "joke:d"])), "energy_dip")["status"] == "ok"


# ---------------------------------------------------------------- storyboard store
def test_storyboard_ops_and_replay(tmp_path):
    bs = S.board_store(tmp_path)
    S.create(bs, "Run", ["joke:a", "joke:b"], 600, ts=TS, id_="sb:run")
    S.add(bs, "sb:run", "joke:c", ts=TS)
    S.move(bs, "sb:run", 2, 0, ts=TS)
    S.remove(bs, "sb:run", "joke:a", ts=TS)
    S.update(bs, "sb:run", title="Run 2", ts=TS)
    assert bs.get("sb:run")["joke_ids"] == ["joke:c", "joke:b"]
    with pytest.raises(ValueError):
        S.add(bs, "sb:run", "joke:c", ts=TS)  # bad op never reaches the log
    snap = json.loads((tmp_path / "storyboards.json").read_text(encoding="utf-8"))
    assert snap["items"]["sb:run"]["title"] == "Run 2" and snap["events"] == 5
    assert len((tmp_path / "storyboards.jsonl").read_text(encoding="utf-8").splitlines()) == 5
    (tmp_path / "storyboards.json").unlink()
    assert S.board_store(tmp_path).get("sb:run") == bs.get("sb:run")  # snapshot = replayed log


def test_storyboard_archive_is_undoable_and_delete_is_terminal(tmp_path):
    bs = S.board_store(tmp_path)
    S.create(bs, "Run", ["joke:a", "joke:b"], 600, ts=TS, id_="sb:run")
    assert "archived" not in bs.get("sb:run")  # the key appears once archive/unarchive has touched the board
    later = "2026-10-27T12:00:00+00:00"
    b = S.archive(bs, "sb:run", ts=later)
    assert b["archived"] is True and b["updated_at"] == later and b["source"] == "his" and b["deleted"] is False
    # every other op on an archived board is refused, in plain words, and nothing reaches the log
    n = bs.n_events
    for fn in (lambda: S.add(bs, "sb:run", "joke:c", ts=TS), lambda: S.remove(bs, "sb:run", "joke:a", ts=TS),
               lambda: S.move(bs, "sb:run", 0, 1, ts=TS), lambda: S.update(bs, "sb:run", title="x", ts=TS),
               lambda: S.set_jokes(bs, "sb:run", ["joke:b"], ts=TS)):
        with pytest.raises(ValueError, match=r'^"Run" is archived: unarchive it first$'):
            fn()
    with pytest.raises(ValueError, match=r'^"Run" is already archived$'):
        S.archive(bs, "sb:run", ts=TS)
    assert bs.n_events == n and bs.get("sb:run")["joke_ids"] == ["joke:a", "joke:b"]
    # undo = unarchive; the board is whole again and editable
    b = S.unarchive(bs, "sb:run", ts=TS)
    assert b["archived"] is False and b["joke_ids"] == ["joke:a", "joke:b"]
    with pytest.raises(ValueError, match=r'^"Run" is not archived$'):
        S.unarchive(bs, "sb:run", ts=TS)
    S.add(bs, "sb:run", "joke:c", ts=TS)
    # the log replays to the same state (archive and unarchive are events like any other)
    assert [e["op"] for e in bs.events()] == ["create", "archive", "unarchive", "add"]
    (tmp_path / "storyboards.json").unlink()
    assert S.board_store(tmp_path).get("sb:run") == bs.get("sb:run")
    # an old item (a snapshot or event written before `archived` existed) reads as not archived
    old = {"sb:old": {"id": "sb:old", "title": "Old", "joke_ids": [], "target_s": 300, "notes": "", "source": "his",
                      "by": "owner", "seed": None, "created_at": TS, "updated_at": TS, "deleted": False}}
    S._apply(old, {"ts": TS, "op": "add", "id": "sb:old", "by": "owner", "data": {"joke_id": "joke:a"}})
    assert old["sb:old"]["joke_ids"] == ["joke:a"]
    # archiving a seeded draft does not make it his
    S.create(bs, "Kids", ["joke:a"], 300, by="ai:seed", ts=TS, id_="sb:seed", seed={"from": "insights.chunks"})
    b = S.archive(bs, "sb:seed", ts=TS)
    assert b["source"] == "inferred" and b["by"] == "ai:seed" and b["seed"] == {"from": "insights.chunks"}
    assert S.unarchive(bs, "sb:seed", ts=TS)["source"] == "inferred"
    # delete works on an archived board and stays terminal: no unarchive, no restore
    S.archive(bs, "sb:run", ts=TS)
    bs.append("delete", "sb:run", {})
    assert bs.get("sb:run")["deleted"] is True
    for op in ("unarchive", "archive", "add", "update"):
        with pytest.raises(ValueError, match=r"^no storyboard sb:run$"):
            bs.append(op, "sb:run", {"joke_id": "joke:a", "title": "x"})


# ---------------------------------------------------------------- alignment on a synthetic set
@pytest.fixture
def ws(corpus, tmp_path):
    wss = W.ws_store(tmp_path)
    b = board(["joke:a", "joke:b", "joke:c"], id_="sb:plan")
    b["title"] = "Plan"
    return wss, W.promote(corpus, wss, b, ts=TS, id_="ws:plan",
                          bit_targets={"joke:a": 50, "joke:b": 60, "joke:c": 60})


def test_align_synthetic(corpus, ws):
    _, w = ws
    a = W.align(corpus, w, "2024-02-01_y")
    assert a["order"]["planned"] == ["joke:a", "joke:b", "joke:c"]
    assert a["order"]["performed"] == ["joke:a", "joke:b"] and a["order"]["same_order"]
    assert a["dropped"] == ["joke:c"]
    assert [x["joke_id"] for x in a["added"]] == ["joke:d"]
    bb = {x["joke_id"]: x for x in a["bits"]}
    assert bb["joke:b"]["time_s"] == 90.0 and bb["joke:b"]["delta_s"] == 30.0
    assert bb["joke:b"]["laugh_s"] == 2.0 and bb["joke:b"]["vs_room"] == 0.22
    assert bb["joke:c"]["performed"] is False
    # z: b before a = moved; a was split over two segments and is merged
    z = W.align(corpus, w, "2024-03-01_z")
    assert z["order"]["performed"] == ["joke:b", "joke:a"]
    assert len(z["order"]["moved"]) == 1
    za = next(x for x in z["bits"] if x["joke_id"] == "joke:a")
    assert za["segments"] == [2, 3] and za["time_s"] == 60.0 and za["laugh_s"] == 8.0
    assert za["laugh_s_per_min"] == 8.0 and za["vs_room"] == 1.6
    # crowd work counts as added material, not a planned bit
    x = W.align(corpus, w, "2024-01-01_x")
    assert any(ad["status"] == "crowd_work" for ad in x["added"]) and x["dropped"] == []


def test_wording_effect_and_try_tonight(corpus, ws):
    _, w = ws
    h = W.bit_history(corpus, "joke:b")
    eff = h["wording"]["effects"]
    top = next(e for e in eff if "purple monkey" in e["examples"][0]["quote"])
    assert top["n_with"] == 1 and top["n_without"] == 2 and top["delta_laugh_s"] == 4.2 and top["low_n"]
    assert "+4.2 laugh-s over 1 night" in top["statement"] and "low n" in top["statement"]
    notes = W.try_tonight(corpus, w)
    assert any(n["kind"] == "put_back" and n["joke_id"] == "joke:b" for n in notes)
    assert any(n["kind"] == "dropped" and n["joke_id"] == "joke:c" for n in notes)


# ---------------------------------------------------------------- lock rule
def test_locked_line_never_punched_up(corpus, ws):
    wss, w = ws
    locked = "Bravo setup about the dog."
    w = W.lock_line(corpus, wss, "ws:plan", "joke:b", locked, "seg:2024-03-01_z#1", ts=TS)
    # the model tries to punch up every numbered line it was shown
    llm = FakeLLM(lambda msgs: {"drafts": [{"line_no": i, "alt": f"alt {i}", "move": "x"} for i in range(1, 6)]})
    r = W.punch_up(corpus, w, "joke:b", complete=llm)
    prompt = llm.calls[0][1][-1]["content"]
    assert locked not in prompt
    assert r["status"] == "ok" and r["blocked_locked"] >= 1
    assert all(not W.is_locked_line(w, "joke:b", d["target_line"]) for d in r["drafts"])
    assert all(d["draft"] and d["label"].startswith("DRAFT") for d in r["drafts"])
    # a locked bit: no provider call, no drafts, no notes
    w = W.lock_bit(wss, "ws:plan", "joke:b", ts=TS)
    llm2 = FakeLLM({"drafts": [{"line_no": 1, "alt": "x", "move": "x"}]})
    r2 = W.punch_up(corpus, w, "joke:b", complete=llm2)
    assert r2["status"] == "locked" and r2["drafts"] == [] and llm2.calls == []
    assert not any(n["joke_id"] == "joke:b" for n in W.try_tonight(corpus, w))
    assert W.perfected(corpus, w)["locked_share"] > 0


def test_lock_line_must_be_verbatim(corpus, ws):
    wss, _ = ws
    with pytest.raises(ValueError):
        W.lock_line(corpus, wss, "ws:plan", "joke:b", "words he never said", "seg:2024-03-01_z#1", ts=TS)


def test_versions_need_a_note(corpus, ws):
    wss, _ = ws
    with pytest.raises(ValueError):
        W.new_version(wss, "ws:plan", "", ts=TS)
    w = W.new_version(wss, "ws:plan", "cut C, tightened B", bits=[{"joke_id": "joke:a", "title": "A", "target_s": 50}], ts=TS)
    assert w["current"] == 2 and W.current(w)["note"] == "cut C, tightened B"


def test_perfected_meter(corpus, ws):
    _, w = ws
    p = W.perfected(corpus, w)
    assert p["locked_share"] == 0.0
    a = next(b for b in p["bits"] if b["joke_id"] == "joke:a")
    # a: laugh-s/min 9.0, 8.4, 8.0 -> cv = sd/mean
    import statistics
    vals = [9.0, 8.4, 8.0]
    assert a["cv_laugh_s_per_min"] == round(statistics.stdev(vals) / statistics.mean(vals), 2)


# ---------------------------------------------------------------- bridge drafts
def test_bridge_is_a_grounded_draft(corpus):
    llm = FakeLLM({"bridge": "Speaking of my niece...", "uses": ["L1", "L99"]})
    r = S.bridge_draft(corpus, "joke:d", "joke:e", complete=llm)
    assert r["status"] == "ok" and r["draft"] is True and r["label"].startswith("DRAFT")
    assert [g["id"] for g in r["grounded_in"]] == ["L1"]  # an invented link id is dropped
    assert r["grounded_in"][0]["label_id"] == "label:persona:dad"
    nothing = S.bridge_draft(corpus, "joke:a", "joke:f", complete=FakeLLM({"bridge": "x", "uses": []}))
    assert nothing["status"] == "no_grounding" and nothing["text"] is None


# ---------------------------------------------------------------- venues: politeness + cache
def test_geocoder_rate_limit_and_cache(tmp_path):
    t = [0.0]
    slept, urls = [], []

    def clock():
        return t[0]

    def sleep(s):
        slept.append(s)
        t[0] += s

    def fetch(url):
        urls.append(url)
        return [{"lat": "40.7", "lon": "-73.9", "display_name": "X", "category": "amenity", "type": "theatre",
                 "osm_type": "node", "osm_id": 1}]

    g = V.Geocoder(tmp_path / "geo.json", fetch=fetch, sleep=sleep, clock=clock)
    g.search("A")
    g.search("B")
    g.search("A")  # cached: never re-sent
    assert len(urls) == 2 and slept and abs(slept[0] - 1.0) < 1e-9
    g2 = V.Geocoder(tmp_path / "geo.json", fetch=lambda u: pytest.fail("cache miss"), sleep=sleep, clock=clock)
    assert g2.search("B")[0]["lat"] == 40.7
    assert V.USER_AGENT == "Punchable/0.1 (personal)"
    gv = V.geocode_venue(g2, "Unknown (title 'Beach jokes'; maybe Seaside)", offline=True)
    assert gv["ambiguous"] and gv["status"] == "inferred"


# ---------------------------------------------------------------- determinism + verbatim evidence
def _outputs(c, w):
    out = {"conn": {j: C.joke_connections(c, j) for j in sorted(c.jokes)},
           "health": S.health(c, board(["joke:a", "joke:b", "joke:c", "joke:e"])),
           "next": S.best_next(c, board(["joke:a"])),
           "align": W.align_all(c, w), "hist": W.history(c, w), "try": W.try_tonight(c, w),
           "perfected": W.perfected(c, w), "venues": V.venue_stats(c)}
    return out


def test_deterministic(corpus, ws):
    _, w = ws
    a = json.dumps(_outputs(corpus, w), sort_keys=True)
    b = json.dumps(_outputs(Corpus(make_graph(), make_analyses()), w), sort_keys=True)
    assert a == b


def test_every_evidence_quote_is_verbatim(corpus, ws):
    _, w = ws
    evs = list(walk_evidence(_outputs(corpus, w)))
    assert evs, "expected evidence in the outputs"
    bad = [e for e in evs if not corpus.verify(e)]
    assert not bad, bad[:3]
    # the fabricated quote on e:bad was dropped, not passed through
    cb = C.joke_connections(corpus, "joke:c")["groups"][3]["items"]
    assert cb and all(it["evidence"] == [] for it in cb)


# ---------------------------------------------------------------- real data (skipped elsewhere)
def test_real_connections_verbatim_and_deterministic(real):
    a = C.joke_connections(real, "joke:airport-bit")
    b = C.joke_connections(real, "joke:airport-bit")
    assert json.dumps(a, sort_keys=True) == json.dumps(b, sort_keys=True)
    evs = list(walk_evidence(a))
    assert evs and all(real.verify(e) for e in evs)
    for g in a["groups"]:
        assert len(g["items"]) <= 5
        assert all(i["source"] in ("inferred", "proposed", "his") for i in g["items"])


def test_real_demo_verbatim(real, tmp_path):
    from creative import story as S2
    out = W.demo(real, S2.board_store(tmp_path), W.ws_store(tmp_path), ts=TS, with_drafts=False)
    assert len(out["alignments"]) == len(real.set_keys())  # one per analysed set (7, then 17 after the 2015-16 sets landed)
    evs = list(walk_evidence(out))
    bad = [e for e in evs if not real.verify(e)]
    assert evs and not bad, bad[:3]
    out2 = W.demo(real, S2.board_store(tmp_path), W.ws_store(tmp_path), ts=TS, with_drafts=False)
    assert json.dumps(out, sort_keys=True) == json.dumps(out2, sort_keys=True)


def test_punch_up_marks_every_line_from_the_laugh_map_and_aims_at_the_silence(tmp_path):
    """Each numbered line carries [laugh 2.1 s] (the laugh the detector tied to it, timed from laughs_auto.json),
    [laugh · 3/4 bars] without timing, or [silence]; the instruction targets the silent ones."""
    from .conftest import TEXT
    an = make_analyses()
    seg = an["2024-03-01_z"]["jokes"][0]  # joke:b on the latest night: two lines, one laugh
    seg["punchlines"] = [{"text": "Bravo closer words about the cat today.", "at_ms": 30000, "laugh_id": "l30", "bars": 3}]
    laughs = {"2024-03-01_z": {"l30": {"id": "l30", "start_ms": 30000, "end_ms": 32100, "bars": 3}}}
    c = Corpus(make_graph(), an, laughs=laughs)
    wss = W.ws_store(tmp_path)
    w = W.promote(c, wss, board(["joke:b"], id_="sb:p"), ts=TS, id_="ws:p", bit_targets={"joke:b": 60})
    msgs, info = W.punch_messages(c, w, "joke:b")
    user = msgs[-1]["content"]
    assert "1. Bravo setup about the dog.   [silence]" in user
    assert "2. Bravo closer words about the cat today.   [laugh 2.1 s]" in user
    assert "Target the lines that got nothing ([silence]: 1 of 2)" in user and "leave a line that already lands" in user
    assert f"His written version of the setup (verbatim):\n\"{TEXT['b']}\"" in user
    assert "by the name his profile gives it" in user
    assert W.laugh_marks(c, info["perf"], info["lines"]) == ["silence", "laugh 2.1 s"]
    # no timing in the laugh map: the bars word; no profile loaded: the task line is the first and only system block
    c2 = Corpus(make_graph(), an)
    assert W.laugh_marks(c2, c2.perfs["joke:b"][-1], info["lines"]) == ["silence", "laugh · 3/4 bars"]
    assert [m["role"] for m in msgs] == ["system", "user"] and msgs[0]["content"] == W.PUNCH_SYSTEM
    # what the API's legal check sees before enqueue: every line the model would be shown
    pt = W.punch_text(c, "joke:b")
    assert "Bravo closer words about the cat today." in pt and TEXT["b"] in pt
    # the drafts still land on the numbered line (the laugh mark is not part of the target)
    llm = FakeLLM({"drafts": [{"line_no": 1, "alt": "Bravo setup, now with a cat.", "move": "repeat the word"}]})
    r = W.punch_up(c, w, "joke:b", complete=llm)
    assert r["status"] == "ok" and r["drafts"][0]["target_line"] == "Bravo setup about the dog."
    assert r["drafts"][0]["target_evidence"]["quote"] == "Bravo setup about the dog."
