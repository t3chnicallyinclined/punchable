import json

import engine.inbox as inbox_mod
from engine.inbox import Inbox
from engine.tests.conftest import FakeEmbedder, FakeLaughs, FakeTranscriber, MockBackend, drop_audio, fake_normalize


def make(env):
    return Inbox(env["cfg"], embedder=FakeEmbedder(), transcriber=FakeTranscriber(), laughs=FakeLaughs(),
                 normalizer=fake_normalize, prober=lambda p: 300.0, log=lambda s: None)


def graph(env):
    return json.loads((env["root"] / "kb" / "graph.json").read_text(encoding="utf-8"))


def test_end_to_end_with_mock(env):
    dec = (env["root"] / "kb" / "decisions.jsonl").read_bytes()
    drop_audio(env["root"])
    [r] = make(env).run()
    assert r["status"] == "done"
    g = graph(env)
    perf = [n for n in g["nodes"] if n["type"] == "performance"]
    assert len(perf) == 2  # the intro is not a unit
    match = [e for e in g["edges"] if e["kind"] == "performed_as" and e["dst"] == "joke:airport-bit"]
    assert len(match) == 1 and match[0]["version"] == "unit:aaaa-00"
    new = [n for n in g["nodes"] if n["type"] == "joke" and n.get("proposed")]
    assert [n["title"] for n in new] == ["Disclaimer parrot"]
    labels = [e for e in g["edges"] if e["kind"] == "has_label" and e.get("origin")]
    # tampered quotes ('will vary', 'studio condo') and the out-of-vocabulary label are gone
    assert {e["dst"] for e in labels} == {"label:device:literal_interpretation"}
    nodes = {n["id"]: n for n in g["nodes"]}
    for e in g["edges"]:
        if e.get("origin"):
            for ev in e["evidence"]:
                assert ev["quote"] in nodes[ev["node"]]["text"], ev
    assert (env["root"] / "kb" / "decisions.jsonl").read_bytes() == dec
    assert not list((env["root"] / "inbox").iterdir())
    moved = list((env["root"] / "processed").glob("*/2026-10-20_test-room.opus"))
    assert len(moved) == 1 and moved[0].read_bytes() == b"fake-opus-bytes-1"
    runs = [json.loads(l) for l in env["cfg"].runs_log.read_text().splitlines()]
    assert runs and all(x["run_id"] == r["run_id"] for x in runs)


def test_idempotent_same_file_same_versions(env):
    drop_audio(env["root"])
    make(env).run()
    g1 = (env["root"] / "kb" / "graph.json").read_bytes()
    n_calls = len(MockBackend.calls)
    drop_audio(env["root"])  # the same bytes again
    [r] = make(env).run()
    assert r["status"] == "skipped"
    assert len(MockBackend.calls) == n_calls  # no model calls
    assert (env["root"] / "kb" / "graph.json").read_bytes() == g1
    copies = list((env["root"] / "processed").glob("*/2026-10-20_test-room*.opus"))
    assert len(copies) == 2  # never deleted: the duplicate is kept beside the original


def test_reject_is_never_reproposed_and_confirmed_is_kept(env, monkeypatch):
    drop_audio(env["root"])
    make(env).run()
    g = graph(env)
    match = next(e for e in g["edges"] if e["kind"] == "performed_as" and e["dst"] == "joke:airport-bit")
    lab = next(e for e in g["edges"] if e["kind"] == "has_label" and e.get("origin"))
    new_joke = next(n for n in g["nodes"] if n.get("proposed"))
    dpath = env["root"] / "kb" / "decisions.jsonl"
    with open(dpath, "a", encoding="utf-8") as f:  # the comic's decisions, as the UI writes them
        f.write(json.dumps({"ts": "t", "target": "edge", "id": match["id"], "action": "reject", "before": match,
                            "after": {"status": "rejected"}, "by": "owner"}) + "\n")
        f.write(json.dumps({"ts": "t", "target": "edge", "id": lab["id"], "action": "confirm", "before": lab,
                            "after": {"status": "confirmed"}, "by": "owner"}) + "\n")
        f.write(json.dumps({"ts": "t", "target": "node", "id": new_joke["id"], "action": "reject",
                            "before": new_joke, "after": {"status": "rejected"}, "by": "owner"}) + "\n")
    dec = dpath.read_bytes()
    monkeypatch.setattr(inbox_mod, "PROMPT_VERSION", "inbox-test-v2")  # new versions -> a real re-run
    MockBackend.calls = []
    drop_audio(env["root"])
    [r] = make(env).run()
    assert r["status"] == "done" and r["stats"]["skipped_rejected"] >= 1
    g2 = graph(env)
    # the rejected match is not re-proposed: graph.json keeps the OLD record (so his decision replays on it),
    # no fresh proposal from the new run replaces it, and the replayed view shows it rejected
    same = [e for e in g2["edges"] if (e["src"], e["kind"], e["dst"]) == (match["src"], match["kind"], match["dst"])]
    assert same == [match], "rejected match was re-proposed"
    assert [n for n in g2["nodes"] if n["id"] == new_joke["id"]] == [new_joke]
    assert not [n for n in g2["nodes"] if n.get("origin_span") == new_joke["origin_span"] and n["id"] != new_joke["id"]]
    from engine.kbstore import KB
    kb = KB(env["cfg"])
    assert kb.view.edges[match["id"]]["status"] == "rejected"
    assert match["id"] in kb.rejected_edges and new_joke["id"] in kb.rejected_nodes
    assert r["run_id"] not in json.dumps([e for e in g2["edges"] if e["id"] == match["id"]])
    kept = next(e for e in g2["edges"] if e["id"] == lab["id"])
    assert kept == lab  # the confirmed item is kept exactly as it was
    assert dpath.read_bytes() == dec  # decisions untouched
    # few-shot: the comic's confirmed and ruled-out decisions are in the prompts, auto-confirmed never
    prompts = "\n".join(m["content"] for c in MockBackend.calls for m in c["messages"])
    assert "Dating apps are just job interviews" in prompts
    assert "the comic RULED OUT" in prompts
    assert "AUTO-CONFIRMED-NEVER-AN-EXAMPLE" not in prompts


def test_fewshot_excludes_auto_confirmed(env):
    from engine.kbstore import KB
    pos, neg = KB(env["cfg"]).examples({"has_label"})
    assert [p["reason"] for p in pos] == ["OWNER-CONFIRMED-EXAMPLE"]


def test_short_recording_with_laughs_is_a_clip(env):
    drop_audio(env["root"], "short.m4a", b"clip")
    ib = make(env)
    ib.prober = lambda p: 60.0
    [r] = ib.run()
    assert r["kind"] == "clip"  # FakeLaughs hears 2 laughs
    assert any(n["type"] == "performance" for n in graph(env)["nodes"])


def test_voice_note_by_length(env):
    drop_audio(env["root"], "idea.m4a", b"memo")
    ib = make(env)
    ib.prober = lambda p: 30.0
    ib.laughs = lambda *a: {"laughs": []}  # a short recording with no crowd laughter is a note
    [r] = ib.run()
    assert r["kind"] == "voice_note"
    assert (env["root"] / "corpus" / "memos").exists() and not (env["root"] / "corpus" / "sets").exists()
    g = graph(env)
    memo_units = [n for n in g["nodes"] if n.get("source") == "memo"]
    assert memo_units and all(n["type"] == "unit" for n in memo_units)
    assert any(e["kind"] == "part_of" and e["src"] in {n["id"] for n in memo_units} for e in g["edges"])


def test_lines_from_words_and_merge_adjacent(env):
    tx = {"segments": [{"start": 0, "end": 6, "text": "my shirt is small. Wedding gig here.", "words": [
        {"w": "my", "start": 0, "end": .3}, {"w": "shirt", "start": .3, "end": .6}, {"w": "is", "start": .6, "end": .8},
        {"w": "small.", "start": .8, "end": 1.2}, {"w": "Wedding", "start": 1.5, "end": 1.9},
        {"w": "gig", "start": 1.9, "end": 2.2}, {"w": "here.", "start": 2.2, "end": 2.6}]}]}
    lines = Inbox.lines(tx)
    assert [l["text"] for l in lines] == ["my shirt is small.", "Wedding gig here."]
    ib = make(env)
    ib._cands = {}
    us = [{"id": f"perf:x@{i}", "start_ms": i * 1000, "end_ms": i * 1000 + 900, "text": t, "_segs": [],
           "laughs": {"hits": 0, "max_bars": 0}} for i, t in enumerate(["a b", "c d", "e f"])]
    m = {"joke_id": "joke:j", "confidence": 0.7, "reason": "r", "version": "unit:v", "new_quote": "", "old_quote": ""}
    out, mm = ib.merge_adjacent(us, {us[0]["id"]: m, us[1]["id"]: dict(m, confidence=0.9), us[2]["id"]: None},
                                {"laughs": []}, True, "x")
    assert [u["text"] for u in out] == ["a b c d", "e f"] and mm[out[0]["id"]]["confidence"] == 0.9
