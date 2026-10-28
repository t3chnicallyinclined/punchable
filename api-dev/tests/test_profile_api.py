"""His voice profile as data (api-dev/profile_api.py) and the legal rules on the request path (creative_api.refuse_legal,
ask_api.ask). Pure unit tests over the store and the handlers with a fake request: no API process, no knowledge base,
no network. Invented text only. The owner/worker token separation itself is `Handler.authorize` (test_auth.py, the
NEW_ROUTES rows); here: the worker's view, the worker's actor rule, and the 422 that keeps court text out.

    python -m pytest api-dev/tests/test_profile_api.py -q
"""
from __future__ import annotations

import json
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "api-dev"))
sys.path.insert(0, str(DESK / "tools" / "explorer"))
sys.path.insert(0, str(DESK))
import ask_api as AK  # noqa: E402
import creative_api as CR  # noqa: E402
import kb as K  # noqa: E402
import profile_api as PF  # noqa: E402
from creative import voice  # noqa: E402
from creative.store import Store  # noqa: E402

LEGAL_TEXT = ("The petitioner and the respondent appeared before the court; index no 123 was read into the record, "
              "sworn to before me.")
PROFILE_V1 = "# Voice\n\nHe talks fast and sits on the punch word. The parrot bit is his closer.\n"
PROFILE_V2 = "# Voice\n\nHe talks slower now. The parrot bit is retired; the dentist bit closes.\n"


class FakeHandler:
    """The slice of server.Handler the route uses: kb, send_json, err, actor (the owner for an app token; a worker
    token must name import:/ai: in the body, as server.Handler.actor does with auth on)."""

    def __init__(self, worker: bool = False):
        self.kb = SimpleNamespace(backend="files")
        self.worker = worker
        self.out = None

    def send_json(self, obj, status=200):
        self.out = (status, obj)

    def err(self, status, msg):
        self.out = (status, {"error": msg})

    def actor(self, b=None):
        want = (b or {}).get("actor")
        if self.worker:
            if not isinstance(want, str) or ":" not in want:
                raise K.DecisionError("a worker write must name its actor: import:<source> or ai:<what>", 400)
            return want
        return "owner"


@pytest.fixture
def cr(tmp_path):
    voice.clear()
    yield CR.Creative(tmp_path / "creative", None)
    voice.clear()


def call(cr, method, path, body=None, worker=False):
    h = FakeHandler(worker)
    assert PF.handle(cr, h, method, path, {}, lambda: body if body is not None else {}) is True
    return h.out


def test_store_keeps_the_previous_version_and_refuses_other_ops(tmp_path):
    st = Store(tmp_path, "profile", PF.profile_apply)
    a = st.append("put", PF.VOICE_ID, {"kind": "voice", "text": PROFILE_V1}, by="owner", ts="2026-10-28T10:00:00+00:00")
    assert a["text"] == PROFILE_V1 and a["updated_at"] == "2026-10-28T10:00:00+00:00" and a["by"] == "owner" and a["previous"] is None
    b = st.append("put", PF.VOICE_ID, {"kind": "voice", "text": PROFILE_V2}, by="owner", ts="2026-10-29T10:00:00+00:00")
    assert b["text"] == PROFILE_V2 and b["previous"] == {"text": PROFILE_V1, "updated_at": "2026-10-28T10:00:00+00:00", "by": "owner"}
    for op, i, d in (("delete", PF.VOICE_ID, {}), ("put", "profile:other", {"text": "x"}), ("put", PF.VOICE_ID, {"text": 3})):
        with pytest.raises(ValueError):
            st.append(op, i, d)
    # a replay of the log derives the same document
    assert Store(tmp_path, "profile", PF.profile_apply).get(PF.VOICE_ID) == b
    assert "profile" in CR.APPLY and CR.APPLY["profile"] is PF.profile_apply


def test_owner_puts_reads_and_undoes_by_the_previous_version(cr):
    assert call(cr, "GET", "/api/profile/voice") == (200, {"kind": "voice", "text": None, "updated_at": None, "by": None,
                                                           "chars": 0, "learned": {"kept": [], "dropped": [], "updated_at": None},
                                                           "previous": None})
    s, v1 = call(cr, "PUT", "/api/profile/voice", {"text": PROFILE_V1})
    assert s == 200 and v1["text"] == PROFILE_V1 and v1["by"] == "owner" and v1["previous"] is None and v1["chars"] == len(PROFILE_V1)
    s, v2 = call(cr, "PUT", "/api/profile/voice", {"text": PROFILE_V2.rstrip("\n")})  # the trailing newline is normalised
    assert s == 200 and v2["text"] == PROFILE_V2 and v2["previous"]["text"] == PROFILE_V1 and v2["updated_at"] >= v1["updated_at"]
    assert call(cr, "GET", "/api/profile/voice")[1] == v2
    # Undo = the app saves the previous text again (by him): the document moves back, with v2 as ITS previous
    s, v3 = call(cr, "PUT", "/api/profile/voice", {"text": v2["previous"]["text"]})
    assert s == 200 and v3["text"] == PROFILE_V1 and v3["previous"]["text"] == PROFILE_V2
    # shape and size
    assert call(cr, "PUT", "/api/profile/voice", {"text": "   "})[0] == 400
    assert call(cr, "PUT", "/api/profile/voice", {})[0] == 400
    assert call(cr, "PUT", "/api/profile/voice", {"text": "x" * (PF.MAX_CHARS + 1)})[0] == 413
    assert call(cr, "PATCH", "/api/profile/voice", {"text": "x"})[0] == 405
    assert PF.handle(cr, FakeHandler(), "GET", "/api/profile/other", {}, lambda: {}) is False


def test_worker_reads_its_copy_without_history_and_must_name_an_import_actor(cr):
    call(cr, "PUT", "/api/profile/voice", {"text": PROFILE_V1})
    s, w = call(cr, "GET", "/api/worker/profile/voice", worker=True)
    assert s == 200 and w == {"kind": "voice", "text": PROFILE_V1, "updated_at": w["updated_at"], "by": "owner", "chars": len(PROFILE_V1),
                              "learned": {"kept": [], "dropped": [], "updated_at": None}, "laugh_lines": []}
    assert "previous" not in w
    # the loader: a worker write names import:<what>; without it, 400 and nothing changes
    s, _ = call(cr, "PUT", "/api/worker/profile/voice", {"text": PROFILE_V2}, worker=True)
    assert s == 400 and call(cr, "GET", "/api/profile/voice")[1]["text"] == PROFILE_V1
    s, v = call(cr, "PUT", "/api/worker/profile/voice", {"text": PROFILE_V2, "actor": "import:profile"}, worker=True)
    assert s == 200 and v["by"] == "import:profile" and v["text"] == PROFILE_V2
    assert call(cr, "GET", "/api/profile/voice")[1]["previous"]["by"] == "owner"


def test_court_text_is_refused_on_both_puts_and_nothing_is_kept(cr):
    """The legal rules (gsync/rules.py) on the profile, the same way every ingest route applies them. Red-proof: with
    the check removed from profile_api.put the first assert fails (the PUT would be 200)."""
    call(cr, "PUT", "/api/profile/voice", {"text": PROFILE_V1})
    s, body = call(cr, "PUT", "/api/profile/voice", {"text": PROFILE_V1 + "\n" + LEGAL_TEXT})
    assert s == 422 and "court material" in body["error"], (s, body)
    s, _ = call(cr, "PUT", "/api/worker/profile/voice", {"text": LEGAL_TEXT, "actor": "import:profile"}, worker=True)
    assert s == 422
    doc = call(cr, "GET", "/api/profile/voice")[1]
    assert doc["text"] == PROFILE_V1 and doc["previous"] is None, "nothing of the refused text was written"
    log = (cr.dir / "profile.jsonl").read_text(encoding="utf-8")
    assert "petitioner" not in log and "index no" not in log


def test_the_api_loads_its_own_copy_for_an_in_process_job(cr):
    assert PF.load_into_voice(cr, SimpleNamespace(backend="files")) is False and not voice.loaded()
    call(cr, "PUT", "/api/profile/voice", {"text": PROFILE_V1})
    assert PF.load_into_voice(cr, SimpleNamespace(backend="files")) is True
    assert voice.profile()["text"] == PROFILE_V1.strip() and voice.profile()["source"] == "store"


# ---------------------------------------------------------------- the learned section
KB = SimpleNamespace(backend="files")
RIFF_IDEAS = [{"text": "A parrot that files its own taxes."}, {"text": "The parrot audits the vet."}, {"text": "Parrot, IRS, same bird."}]


def finished_riff(cr, kb=KB, ideas=RIFF_IDEAS, key="riff|abc"):
    """A finished riff job in the API's job store, as the worker leaves it (creative_api.job_apply)."""
    st = cr.stores(kb)["jobs"]
    st.append("enqueue", f"job:{key}", {"kind": "riff", "key": key, "input": {"idea": "parrots", "lines": []}})
    st.append("claim", f"job:{key}", {"worker": "w"})
    st.append("done", f"job:{key}", {"result": {"status": "ok", "ideas": ideas}})


def test_store_learn_op_merges_and_a_put_keeps_the_learned_section(tmp_path):
    """The loader and Settings both PUT: a put replaces his text and never the learned section (merge, not replace).
    Red-proof: with `learned` dropped from profile_apply's put, the third assert fails."""
    st = Store(tmp_path, "profile", PF.profile_apply)
    # learn before any text: the document exists with text None and the learned lines
    a = st.append("learn", PF.VOICE_ID, {"kept": [{"text": "Kept one.", "bit": "joke:k"}], "dropped": ["Passed one."]},
                  by="owner", ts="2026-10-28T10:00:00+00:00")
    assert a["text"] is None and a["previous"] is None
    assert a["learned"] == {"kept": [{"text": "Kept one.", "bit": "joke:k", "date": "2026-10-28"}],
                            "dropped": [{"text": "Passed one.", "bit": None, "date": "2026-10-28"}], "updated_at": "2026-10-28T10:00:00+00:00"}
    b = st.append("put", PF.VOICE_ID, {"kind": "voice", "text": PROFILE_V1}, by="import:profile", ts="2026-10-29T10:00:00+00:00")
    assert b["text"] == PROFILE_V1 and b["learned"] == a["learned"], "a put keeps what was learned"
    assert b["previous"] is None, "a document with no text yet has no previous version to undo to"
    c = st.append("learn", PF.VOICE_ID, {"kept": ["Kept two."]}, by="owner", ts="2026-10-30T10:00:00+00:00")
    assert c["text"] == PROFILE_V1 and c["updated_at"] == "2026-10-29T10:00:00+00:00", "a learn never touches his text or its date"
    assert [e["text"] for e in c["learned"]["kept"]] == ["Kept two.", "Kept one."]
    d = st.append("put", PF.VOICE_ID, {"kind": "voice", "text": PROFILE_V2}, by="owner", ts="2026-10-31T10:00:00+00:00")
    assert d["previous"]["text"] == PROFILE_V1 and d["learned"] == c["learned"]
    for data in ({"kept": "x"}, {"kept": [], "dropped": []}, {}):
        with pytest.raises(ValueError):
            st.append("learn", PF.VOICE_ID, data)
    assert Store(tmp_path, "profile", PF.profile_apply).get(PF.VOICE_ID) == d, "a replay derives the same document"


def test_both_gets_return_the_learned_section_and_the_worker_gets_laugh_lines(cr):
    assert call(cr, "GET", "/api/profile/voice")[1]["learned"] == {"kept": [], "dropped": [], "updated_at": None}
    s, w = call(cr, "GET", "/api/worker/profile/voice", worker=True)
    assert s == 200 and w["learned"] == {"kept": [], "dropped": [], "updated_at": None} and w["laugh_lines"] == []
    PF.store(cr, KB).append("learn", PF.VOICE_ID, {"kept": ["Kept one."]}, by="owner")
    call(cr, "PUT", "/api/profile/voice", {"text": PROFILE_V1})
    own = call(cr, "GET", "/api/profile/voice")[1]
    assert own["text"] == PROFILE_V1 and [e["text"] for e in own["learned"]["kept"]] == ["Kept one."]
    w = call(cr, "GET", "/api/worker/profile/voice", worker=True)[1]
    assert w["learned"] == own["learned"] and "previous" not in w and w["laugh_lines"] == [], "no corpus here: no laugh lines, no error"
    # the in-process load carries the learned section into creative.voice
    assert PF.load_into_voice(cr, KB) is True
    assert voice.status()["learned"] == {"kept": 1, "dropped": 0, "updated_at": own["learned"]["updated_at"], "laughs": 0}
    assert "Kept one." in voice.learned_block("creative_riff")


def test_learn_kept_records_a_kept_draft_and_its_siblings_and_ignores_his_own_line(cr):
    finished_riff(cr)
    # his own typed line: not a draft, nothing written
    assert PF.learn_kept(cr, KB, "My own line about parrots.", "joke:x", "owner") is None
    assert call(cr, "GET", "/api/profile/voice")[1]["learned"]["kept"] == []
    # one of the riff's drafts, saved as his (POST /api/ideas): kept, the other two passed over
    out = PF.learn_kept(cr, KB, "The parrot audits the vet.", "joke:new", "owner")
    assert out == {"kept": 1, "dropped": 2, "updated_at": out["updated_at"]}
    l = call(cr, "GET", "/api/profile/voice")[1]["learned"]
    assert [e["text"] for e in l["kept"]] == ["The parrot audits the vet."] and l["kept"][0]["bit"] == "joke:new"
    assert [e["text"] for e in l["dropped"]] == ["A parrot that files its own taxes.", "Parrot, IRS, same bird."]
    # later he keeps one of the passed-over lines: it moves to kept, the first keep stays
    PF.learn_kept(cr, KB, "Parrot, IRS, same bird.", "joke:new2", "owner")
    l = call(cr, "GET", "/api/profile/voice")[1]["learned"]
    assert [e["text"] for e in l["kept"]] == ["Parrot, IRS, same bird.", "The parrot audits the vet."]
    assert [e["text"] for e in l["dropped"]] == ["A parrot that files its own taxes."]
    # the event log says which job kind it came from and never fails the caller
    ev = [json.loads(x) for x in (cr.dir / "profile.jsonl").read_text(encoding="utf-8").splitlines()]
    assert [e["op"] for e in ev] == ["learn", "learn"] and ev[0]["data"]["from"] == "riff" and ev[0]["by"] == "owner"
    # the store down: None, no raise (his save already stands)
    def down(kb):
        raise RuntimeError("store down")
    assert PF.learn_kept(SimpleNamespace(lock=cr.lock, stores=down, dir=cr.dir), KB, "The parrot audits the vet.", None, "owner") is None


def test_a_flagged_draft_counts_as_kept_even_when_he_edited_it(cr):
    """POST /api/ideas {draft: true}: kept by the flag OR the exact text. Edited words: kept, marked `edited`, no job to
    name siblings from. Exact words with the flag: the job's siblings are passed over as before."""
    finished_riff(cr)
    out = PF.learn_kept(cr, KB, "The parrot audits the vet, and the vet pays.", "joke:ed", "owner", draft=True)
    assert out == {"kept": 1, "dropped": 0, "updated_at": out["updated_at"]}
    l = call(cr, "GET", "/api/profile/voice")[1]["learned"]
    assert l["kept"] == [{"text": "The parrot audits the vet, and the vet pays.", "bit": "joke:ed", "date": l["kept"][0]["date"], "edited": True}]
    out = PF.learn_kept(cr, KB, "The parrot audits the vet.", "joke:ex", "owner", draft=True)
    assert out["kept"] == 2 and out["dropped"] == 2
    l = call(cr, "GET", "/api/profile/voice")[1]["learned"]
    assert "edited" not in l["kept"][0] and l["kept"][0]["text"] == "The parrot audits the vet."
    ev = [json.loads(x) for x in (cr.dir / "profile.jsonl").read_text(encoding="utf-8").splitlines()]
    assert [e["data"]["from"] for e in ev] == ["draft_flag", "riff"]
    # unflagged and no match: still his own line, nothing written
    assert PF.learn_kept(cr, KB, "Typed by him.", "joke:t", "owner", draft=False) is None


def test_dismiss_route_records_a_dropped_line_by_him_and_refuses_bad_shapes(cr):
    """POST /api/profile/voice/learn {dropped}: Write's and Brainstorm's Dismiss. A dismissed line is not passed over
    again later (learn_kept leaves it out of a keep's siblings)."""
    s, out = call(cr, "POST", "/api/profile/voice/learn", {"dropped": ["Parrot, IRS, same bird."], "bit": "joke:p"})
    assert s == 200 and out == {"ok": True, "learned": {"kept": 0, "dropped": 1, "updated_at": out["learned"]["updated_at"]}}
    s, out = call(cr, "POST", "/api/profile/voice/learn", {"dropped": [{"text": "  Punch alt B. ", "bit": "joke:q"}, "Plain one."]})
    assert s == 200 and out["learned"]["dropped"] == 3
    l = call(cr, "GET", "/api/profile/voice")[1]["learned"]
    assert [(e["text"], e["bit"]) for e in l["dropped"]] == [("Punch alt B.", "joke:q"), ("Plain one.", None), ("Parrot, IRS, same bird.", "joke:p")]
    ev = [json.loads(x) for x in (cr.dir / "profile.jsonl").read_text(encoding="utf-8").splitlines()]
    assert all(e["op"] == "learn" and e["by"] == "owner" and e["data"]["from"] == "dismiss" for e in ev)
    # a keep from the riff: the dismissed sibling is not listed as passed over a second time
    finished_riff(cr)
    PF.learn_kept(cr, KB, "The parrot audits the vet.", "joke:new", "owner")
    l = call(cr, "GET", "/api/profile/voice")[1]["learned"]
    assert [e["text"] for e in l["dropped"]] == ["A parrot that files its own taxes.", "Punch alt B.", "Plain one.", "Parrot, IRS, same bird."]
    # shapes, size, method, legal
    for body, st in (({}, 400), ({"dropped": []}, 400), ({"dropped": "x"}, 400), ({"dropped": [""]}, 400), ({"dropped": [3]}, 400),
                     ({"dropped": ["x"] * 11}, 400), ({"dropped": ["x" * 2001]}, 413), ({"dropped": [LEGAL_TEXT]}, 422)):
        assert call(cr, "POST", "/api/profile/voice/learn", body)[0] == st, body
    assert call(cr, "GET", "/api/profile/voice/learn", {})[0] == 405
    assert call(cr, "GET", "/api/profile/voice")[1]["learned"]["dropped"] == l["dropped"], "nothing of a refused body was kept"
    log = (cr.dir / "profile.jsonl").read_text(encoding="utf-8")
    assert "petitioner" not in log


def test_a_flagged_draft_counts_as_kept_even_when_he_edited_it(cr):
    """POST /api/ideas {draft: true}: kept by the flag OR the exact text. Edited words: kept, marked `edited`, no job to
    name siblings from. Exact words with the flag: the job's siblings are passed over as before."""
    finished_riff(cr)
    out = PF.learn_kept(cr, KB, "The parrot audits the vet, and the vet pays.", "joke:ed", "owner", draft=True)
    assert out == {"kept": 1, "dropped": 0, "updated_at": out["updated_at"]}
    l = call(cr, "GET", "/api/profile/voice")[1]["learned"]
    assert l["kept"] == [{"text": "The parrot audits the vet, and the vet pays.", "bit": "joke:ed", "date": l["kept"][0]["date"], "edited": True}]
    out = PF.learn_kept(cr, KB, "The parrot audits the vet.", "joke:ex", "owner", draft=True)
    assert out["kept"] == 2 and out["dropped"] == 2
    l = call(cr, "GET", "/api/profile/voice")[1]["learned"]
    assert "edited" not in l["kept"][0] and l["kept"][0]["text"] == "The parrot audits the vet."
    ev = [json.loads(x) for x in (cr.dir / "profile.jsonl").read_text(encoding="utf-8").splitlines()]
    assert [e["data"]["from"] for e in ev] == ["draft_flag", "riff"]
    # unflagged and no match: still his own line, nothing written
    assert PF.learn_kept(cr, KB, "Typed by him.", "joke:t", "owner", draft=False) is None


def test_dismiss_route_records_a_dropped_line_by_him_and_refuses_bad_shapes(cr):
    """POST /api/profile/voice/learn {dropped}: Write's and Brainstorm's Dismiss. A dismissed line is not passed over
    again later (learn_kept leaves it out of a keep's siblings)."""
    s, out = call(cr, "POST", "/api/profile/voice/learn", {"dropped": ["Parrot, IRS, same bird."], "bit": "joke:p"})
    assert s == 200 and out == {"ok": True, "learned": {"kept": 0, "dropped": 1, "updated_at": out["learned"]["updated_at"]}}
    s, out = call(cr, "POST", "/api/profile/voice/learn", {"dropped": [{"text": "  Punch alt B. ", "bit": "joke:q"}, "Plain one."]})
    assert s == 200 and out["learned"]["dropped"] == 3
    l = call(cr, "GET", "/api/profile/voice")[1]["learned"]
    assert [(e["text"], e["bit"]) for e in l["dropped"]] == [("Punch alt B.", "joke:q"), ("Plain one.", None), ("Parrot, IRS, same bird.", "joke:p")]
    ev = [json.loads(x) for x in (cr.dir / "profile.jsonl").read_text(encoding="utf-8").splitlines()]
    assert all(e["op"] == "learn" and e["by"] == "owner" and e["data"]["from"] == "dismiss" for e in ev)
    # a keep from the riff: the dismissed sibling is not listed as passed over a second time
    finished_riff(cr)
    PF.learn_kept(cr, KB, "The parrot audits the vet.", "joke:new", "owner")
    l = call(cr, "GET", "/api/profile/voice")[1]["learned"]
    assert [e["text"] for e in l["dropped"]] == ["A parrot that files its own taxes.", "Punch alt B.", "Plain one.", "Parrot, IRS, same bird."]
    # shapes, size, method, legal
    for body, st in (({}, 400), ({"dropped": []}, 400), ({"dropped": "x"}, 400), ({"dropped": [""]}, 400), ({"dropped": [3]}, 400),
                     ({"dropped": ["x"] * 11}, 400), ({"dropped": ["x" * 2001]}, 413), ({"dropped": [LEGAL_TEXT]}, 422)):
        assert call(cr, "POST", "/api/profile/voice/learn", body)[0] == st, body
    assert call(cr, "GET", "/api/profile/voice/learn", {})[0] == 405
    assert call(cr, "GET", "/api/profile/voice")[1]["learned"]["dropped"] == l["dropped"], "nothing of a refused body was kept"
    log = (cr.dir / "profile.jsonl").read_text(encoding="utf-8")
    assert "petitioner" not in log

# ---------------------------------------------------------------- the legal rules on the request path
def test_refuse_legal_is_the_ingest_refusal_shape():
    with pytest.raises(CR.HttpError) as e:
        CR.refuse_legal("a plain idea about parrots", LEGAL_TEXT)
    assert e.value.status == 422 and str(e.value) == CR.LEGAL_MSG
    CR.refuse_legal("a plain idea about parrots", "", None)  # nothing to refuse


class NeverEnqueue:
    """A Creative stand-in whose enqueue must not be reached when the text is refused."""

    def __init__(self, corpus=None):
        self.calls = []
        self._c = corpus

    def enqueue(self, *a, **k):
        self.calls.append((a, k))
        return 202, {"job": {"id": "job:x"}}

    def stores(self, kb):
        return {"workingsets": SimpleNamespace(items={})}

    def corpus(self, kb):
        return self._c


def routes(cr, body):
    return CR.Routes(cr, SimpleNamespace(kb=None), None, {}, body)


def test_riff_facts_punchup_and_ask_refuse_court_text_before_any_job_is_queued(tmp_path):
    cr = NeverEnqueue()
    for name, body in (("riff", {"idea": LEGAL_TEXT, "lines": []}), ("riff", {"idea": "parrots", "lines": ["ok", LEGAL_TEXT]}),
                       ("facts", {"text": LEGAL_TEXT})):
        with pytest.raises(CR.HttpError) as e:
            getattr(routes(cr, body), name)()
        assert e.value.status == 422 and str(e.value) == CR.LEGAL_MSG, name
    assert cr.calls == []
    assert routes(cr, {"idea": "parrots that file taxes", "lines": ["one line"]}).riff()[0] == 202  # plain text queues
    # a punch-up shows the bit's own latest-night lines: those are what is checked (creative.workingset.punch_text)
    from creative.data import Corpus
    from creative.tests.conftest import make_analyses, make_graph
    an = make_analyses()
    an["2024-03-01_z"]["jokes"][0]["transcript"] = "Bravo setup about the dog. " + LEGAL_TEXT
    cr2 = NeverEnqueue(Corpus(make_graph(), an))
    with pytest.raises(CR.HttpError) as e:
        routes(cr2, {}).joke_punchup("joke:b")
    assert e.value.status == 422 and cr2.calls == []
    assert routes(NeverEnqueue(Corpus(make_graph(), make_analyses())), {}).joke_punchup("joke:b")[0] == 202
    # Ask: the question itself, before retrieval
    st = Store(tmp_path, "chats", AK.chat_apply)
    st.append("new", "chat:abcdef12", {"title": "x"})
    cr3 = NeverEnqueue()
    with pytest.raises(CR.HttpError) as e:
        AK.ask(cr3, None, st, "chat:abcdef12", LEGAL_TEXT)
    assert e.value.status == 422 and st.get("chat:abcdef12")["turns"] == [] and cr3.calls == []
