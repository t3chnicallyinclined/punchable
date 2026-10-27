"""Say it or ask it (api-dev/say_api.py, docs/UX_2026-10.md): the classifier on its own (no data needed), then the
route over a TEMP copy of the data (files backend): provenance of the guess vs his override, the actor guard, the
Tonight screen in one call, and short names (kb.py `short_name`, the default cut, the guard on a long one).

    python -m pytest api-dev/tests/test_say_api.py -q
"""
from __future__ import annotations

import json
import sys
from pathlib import Path
from urllib.parse import quote

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "db" / "tests"))
sys.path.insert(0, str(DESK / "api-dev"))
sys.path.insert(0, str(DESK / "tools" / "explorer"))
sys.path.insert(0, str(DESK))
from common import Api, TempKB  # noqa: E402

import say_api as SY  # noqa: E402
from creative.notebook import short_name_of  # noqa: E402

# (text, kind) — his real phrasing first, then the edges. Invented material only.
EXAMPLES = [
    ("what do i have about millennials that works", "question"),
    ("oh and what if the vegan brings the grill", "bit"),
    ("build me a tight 5", "command"),
    ("new 7 for friday", "command"),
    ("How do I get from Vegan to Millennials?", "question"),
    ("What's my strongest closer right now", "question"),
    ("anything I wrote about billionaires I never tried", "question"),
    ("which bits killed at the corner room", "question"),
    ("did the kale thing ever work", "question"),
    ("show me everything about my dentist", "question"),
    ("give me a closer about food", "question"),
    ("my strongest opener", "question"),
    ("so um what are my untested bits", "question"),
    ("is the bunker bit too long", "question"),
    ("can I open with the vacuum one", "question"),
    ("my dad thinks the microwave is listening to him", "bit"),
    ("Vegans tell you at dinner. Millennials post it.", "bit"),
    ("imagine a hostage situation but the hostage keeps asking for oat milk", "bit"),
    ("the gym is just a library where everyone is wrong", "bit"),
    ("you know how every airport has one guy who lives there now", "bit"),
    ("and the waiter goes, no, YOU'RE gluten free", "bit"),
    ("what if hotels charged by how guilty you feel", "bit"),
    ("I respect anyone who can make a barbecue feel like a hostage situation", "bit"),
    ("kale trophy. that's the whole bit.", "bit"),
    ("", "bit"),
]


@pytest.mark.parametrize("text,kind", EXAMPLES)
def test_classifier(text, kind):
    assert SY.classify(text)[0] == kind, (text, SY.classify(text))


# Commands (docs/UX_2026-10.md "Commands"): a set list of a length, or a bit onto tonight's list. Negatives last:
# a question about the new set, a bit that merely contains the words, a number out of range.
COMMANDS = [
    ("new 7 for friday", "new_set", {"minutes": 7, "title": "Friday · 7"}),
    ("new 5", "new_set", {"minutes": 5}),
    ("make a 10 for the club", "new_set", {"minutes": 10, "title": "Club · 10"}),
    ("build me a tight 5", "new_set", {"minutes": 5}),
    ("a new seven for saturday night", "new_set", {"minutes": 7, "title": "Saturday night · 7"}),
    ("start a 15 at the cellar", "new_set", {"minutes": 15, "title": "Cellar · 15"}),
    ("New 20 min set for the showcase.", "new_set", {"minutes": 20, "title": "Showcase · 20"}),
    ("give me a quick 3", "new_set", {"minutes": 3}),
    ("add kale trophy to tonight", "add_to_tonight", {"query": "kale trophy", "words": ["kale", "trophy"]}),
    ("put the kale bit in tonight's set", "add_to_tonight", {"words": ["kale"]}),
    ("stick the dentist one into the set", "add_to_tonight", {"words": ["dentist"]}),
    ("throw robot vacuum on tonight’s list", "add_to_tonight", {"words": ["robot", "vacuum"]}),
    ("add it to tonight", "add_to_tonight", {"words": []}),
    ("Add the airport guy to my set for tonight", "add_to_tonight", {"words": ["airport", "guy"]}),
    # negatives: questions, bits, out-of-range
    ("what's new for friday", None, "question"),
    ("new 7 for friday?", None, "question"),
    ("is tonight's set too long", None, "question"),
    ("I need a new 7 for friday", None, "bit"),
    ("tonight's set is my therapy", None, "bit"),
    ("the new 7 killed", None, "bit"),
    ("new car smell is just chemicals", None, "bit"),
    ("add in the kale bit somewhere", None, "bit"),
    ("add my dad to the list of people who hate me", None, "bit"),
    ("new 0 for friday", None, "question"),
    ("new 500 for friday", None, "question"),
]


@pytest.mark.parametrize("text,op,want", COMMANDS)
def test_commands(text, op, want):
    kind, rule = SY.classify(text)
    cmd = SY.parse_command(text)
    if op is None:
        assert cmd is None and kind == want, (text, kind, rule, cmd)
        return
    assert kind == "command" and rule == "command_" + op and cmd and cmd["op"] == op, (text, kind, rule, cmd)
    for k, v in want.items():
        assert cmd[k] == v, (text, k, cmd)


def test_classifier_rules_named():
    assert SY.classify("why?")[1] == "question_mark"
    assert SY.classify("so my landlord thinks")[1] == "premise"
    assert SY.classify("just a line")[1] == "default"


def test_short_name_default_cut():
    assert short_name_of("Robot vacuum") == "Robot vacuum"
    assert short_name_of("Haunted dishwasher saga (closer, 2019)") == "Haunted dishwasher"
    assert short_name_of("The dog is the size of a couch") == "The dog is the"
    assert short_name_of("Antidisestablishmentarianism forever") == "Antidisestablishme"
    assert short_name_of("Robot vacuum", "  Roomba  ") == "Roomba"      # his own name wins
    assert short_name_of("Robot vacuum", "") == "Robot vacuum"           # an empty one is no name
    assert len(short_name_of("a" * 40)) <= 18


# ------------------------------------------------------------------------------------------------ over the API
@pytest.fixture(scope="module")
def api():
    t = TempKB("say", decisions=False)
    a = Api(t, "files")
    yield a, t
    a.stop()
    t.cleanup()


def events(t: TempKB, store: str) -> list[dict]:
    p = t.creative / f"{store}.jsonl"
    if not p.is_file():
        return []
    return [json.loads(x) for x in p.read_text(encoding="utf-8").splitlines() if x.strip()]


def test_bit_is_his_and_the_guess_is_the_apps(api):
    a, t = api
    st, r = a.post("/api/say", {"text": "Zorblax the waiter guesses every order wrong on purpose, as a kindness"})
    assert st == 200 and r["kind"] == "bit" and r["guess"] == "bit" and not r["overridden"]
    assert r["idea"]["id"].startswith("joke:") and r["undo"]["indexes"] == r["idea"]["indexes"]
    ev = events(t, "intents")
    cls = [e for e in ev if e["id"] == r["intent"]]
    assert [e["op"] for e in cls] == ["classify", "acted"]
    assert cls[0]["by"] == "ai:intent" and cls[1]["by"] == "owner" and cls[1]["data"]["ref"] == r["idea"]["id"]
    line = r["idea"]["decisions"][0]
    assert line["by"] == "owner" and line["action"] == "add" and "Zorblax" in line["after"]["summary"]
    # it is in his notebook at once
    st, page = a.get("/api/jokes?q=zorblax")
    assert st == 200 and any(x["id"] == r["idea"]["id"] for x in page["items"])
    # and the undo the response names works
    st, u = a.post("/api/undo", {"indexes": r["undo"]["indexes"]})
    assert st == 200
    _, page = a.get("/api/jokes?q=zorblax")
    assert not any(x["id"] == r["idea"]["id"] for x in page["items"])


def test_question_starts_a_conversation(api):
    a, t = api
    st, r = a.post("/api/say", {"text": "what do i have about waiters that works"})
    assert st == 201 and r["kind"] == "question" and r["chat"]["id"].startswith("chat:")
    assert r["chat"]["turns"][0]["q"] == "what do i have about waiters that works"
    assert r["worker"]["online"] is False          # no home PC has claimed anything: the app says so
    ev = events(t, "intents")
    me = [e for e in ev if e["id"] == r["intent"]]
    assert me[0]["by"] == "ai:intent" and me[-1]["data"]["ref"] == r["chat"]["id"]
    st, ch = a.get(f"/api/ask/chats/{quote(r['chat']['id'], safe=':')}")
    assert st == 200


def test_override_is_his_own_line(api):
    a, t = api
    # "Not a question? Save as a bit": the guess says question, his tap says bit
    st, r = a.post("/api/say", {"text": "what is a bunker but a basement with a budget", "kind": "bit"})
    assert st == 200 and r["kind"] == "bit" and r["guess"] == "question" and r["overridden"]
    ev = [e for e in events(t, "intents") if e["id"] == r["intent"]]
    assert [e["op"] for e in ev] == ["classify", "override", "acted"]
    assert ev[0]["by"] == "ai:intent" and ev[1]["by"] == "owner" and ev[1]["data"]["kind"] == "bit"
    # and the other way: a bit he says is a question
    st, r2 = a.post("/api/say", {"text": "the vacuum robot bit, the long version", "kind": "question"})
    assert st == 201 and r2["kind"] == "question" and r2["overridden"] and r2["guess"] == "bit"


def test_command_new_set_is_his_board_with_undo(api):
    a, t = api
    st, r = a.post("/api/say", {"text": "new 7 for friday"})
    assert st == 200 and r["kind"] == "command" and r["guess"] == "command" and r["rule"] == "command_new_set", r
    cmd = r["command"]
    assert cmd["done"] is True and cmd["minutes"] == 7 and cmd["board"]["title"] == "Friday · 7"
    bid = cmd["board"]["id"]
    # the guess is the app's; the board is his
    ev = [e for e in events(t, "intents") if e["id"] == r["intent"]]
    assert [e["op"] for e in ev] == ["classify", "acted"] and ev[0]["by"] == "ai:intent" and ev[0]["data"]["command"]["op"] == "new_set"
    assert ev[1]["by"] == "owner" and ev[1]["data"]["ref"] == bid
    _, bl = a.get("/api/boards")
    b = next(x for x in bl["boards"] if x["id"] == bid)
    assert b["source"] == "his" and b["target_s"] == 420 and b["n"] == 0
    # Undo = the inverse op the response names (a delete, also his)
    assert cmd["undo"]["PATCH"] == f"/api/boards/{bid}" and cmd["undo"]["body"] == {"delete": True}
    st, u = a.post(cmd["undo"]["PATCH"], cmd["undo"]["body"], method="PATCH")
    assert st == 200 and u["board"]["deleted"] is True
    _, bl = a.get("/api/boards")
    assert not any(x["id"] == bid for x in bl["boards"])
    assert a.get(f"/api/boards/{quote(bid, safe=':')}")[0] == 404


def test_command_add_asks_when_unsure_and_adds_on_his_pick(api):
    a, t = api
    _, page = a.get("/api/jokes?sort=performed&limit=2")
    j0, j1 = page["items"][0], page["items"][1]
    # a word no bit has: the app asks, nothing is written
    st, r = a.post("/api/say", {"text": "add qwxzy flooglehorn to tonight"})
    assert st == 200 and r["kind"] == "command" and r["command"]["op"] == "add_to_tonight"
    assert r["command"]["done"] is False and r["command"]["ask"] == "none"
    ev = [e for e in events(t, "intents") if e["id"] == r["intent"]]
    assert [e["op"] for e in ev] == ["classify"]
    # his pick (choice) is the write; tonight's board is the one he touched last
    st, b = a.post("/api/boards", {"title": "Tonight test", "joke_ids": [j0["id"]], "target_s": 300})
    bid = b["board"]["id"]
    st, r = a.post("/api/say", {"text": "add qwxzy flooglehorn to tonight", "choice": j1["id"]})
    assert st == 200 and r["command"]["done"] is True and r["command"]["board"]["id"] == bid and r["command"]["joke"]["joke_id"] == j1["id"]
    assert r["command"]["position"] == 1 and r["command"]["undo"]["body"] == {"remove": {"joke_id": j1["id"]}}
    ev = [e for e in events(t, "intents") if e["id"] == r["intent"]]
    assert ev[-1]["op"] == "acted" and ev[-1]["by"] == "owner" and ev[-1]["data"]["joke"] == j1["id"]
    _, v = a.get(f"/api/boards/{quote(bid, safe=':')}")
    assert v["board"]["joke_ids"] == [j0["id"], j1["id"]]
    # the same bit again: already there, nothing written twice
    st, r = a.post("/api/say", {"text": "add it to tonight", "choice": j1["id"]})
    assert st == 200 and r["command"]["already"] is True
    _, v = a.get(f"/api/boards/{quote(bid, safe=':')}")
    assert v["board"]["joke_ids"] == [j0["id"], j1["id"]]
    # a query that half-matches two bits: the app asks, nothing is written (his pick is the write)
    import re as _re
    def word(t):
        return next((w for w in _re.findall(r"[a-z]{4,}", t.lower()) if w not in ("the", "with", "that", "this", "from")), None)
    w0, w1 = word(j0["title"]), word(j1["title"])
    if w0 and w1 and w0 != w1:
        _, v0 = a.get(f"/api/boards/{quote(bid, safe=':')}")
        st, r = a.post("/api/say", {"text": f"add {w0} {w1} to tonight"})
        assert st == 200 and r["command"]["op"] == "add_to_tonight", r
        if len(r["command"]["candidates"]) >= 2 and not r["command"]["candidates"][0].get("sure"):
            assert r["command"]["done"] is False and r["command"]["ask"] == "which"
            _, v1 = a.get(f"/api/boards/{quote(bid, safe=':')}")
            assert v1["board"]["joke_ids"] == v0["board"]["joke_ids"], "an unsure add wrote to the board"
    # an unmistakable name: added at once (the screen shows Undo)
    name = j0["short_name"] if j0.get("short_name") else j0["title"].split(" (")[0]
    st, r = a.post("/api/say", {"text": f"add {name} to tonight"})
    assert st == 200 and r["command"]["op"] == "add_to_tonight"
    assert r["command"]["candidates"] and r["command"]["candidates"][0]["joke_id"] == j0["id"], r["command"]["candidates"]
    assert a.post("/api/say", {"text": "add it to tonight", "choice": "joke:nope-not-a-bit"})[0] == 400
    assert a.post("/api/say", {"text": "the vacuum robot bit", "kind": "command"})[0] == 400


def test_guards(api):
    a, _ = api
    assert a.post("/api/say", {})[0] == 400
    assert a.post("/api/say", {"text": "x", "kind": "poem"})[0] == 400
    assert a.post("/api/say", {"text": "x", "context": "nope"})[0] == 400
    # an import or ai actor may not speak through the input bar (a loopback API accepts a named actor elsewhere)
    assert a.post("/api/say", {"text": "a line from a machine", "actor": "import:memo"})[0] == 403
    assert a.get("/api/say")[0] == 405


def test_tonight_screen_is_one_call(api):
    a, _ = api
    _, page = a.get("/api/jokes?sort=performed&limit=3")
    ids = [x["id"] for x in page["items"]]
    st, b = a.post("/api/boards", {"title": "Tuesday open mic", "joke_ids": ids, "target_s": 300})
    assert st == 201
    st, s = a.get("/api/tonight/screen")
    assert st == 200
    assert set(s) >= {"date", "weekday", "board", "to_sort", "latest_set", "questions", "worker", "counts"}
    assert s["board"]["id"] == b["board"]["id"] and len(s["board"]["segments"]) == 3
    seg = s["board"]["segments"][0]
    assert set(seg) == {"joke_id", "short_name", "duration_s", "gap"} and len(seg["short_name"]) <= 24
    assert 1 <= len(s["questions"]) <= 3 and all(q.endswith("?") for q in s["questions"])
    assert isinstance(s["to_sort"], int)


def test_short_name_is_a_decision_and_shows_everywhere(api):
    a, t = api
    _, page = a.get("/api/jokes?sort=performed&limit=1")
    j = page["items"][0]
    assert j["short_name"] == short_name_of(j["title"]) and "duration_s" in j
    st, w = a.post("/api/decision", {"target": "node", "action": "edit", "id": j["id"], "after": {"short_name": "Sock puppet"}})
    assert st == 200 and w["decisions"][0]["by"] == "owner"
    _, jd = a.get(f"/api/jokes/{quote(j['id'], safe=':')}")
    assert jd["joke"]["short_name"] == "Sock puppet"
    _, page = a.get("/api/jokes?sort=performed&limit=1")
    assert page["items"][0]["short_name"] == "Sock puppet"
    # the set builder's steps and the boards list carry it
    st, b = a.post("/api/boards", {"title": "Named", "joke_ids": [j["id"]], "target_s": 300})
    _, v = a.get(f"/api/boards/{quote(b['board']['id'], safe=':')}/build")
    assert v["steps"][0]["short_name"] == "Sock puppet"
    _, bl = a.get("/api/boards")
    assert next(x for x in bl["boards"] if x["id"] == b["board"]["id"])["names"] == ["Sock puppet"]
    # the guard: too long is refused, null goes back to the default
    assert a.post("/api/decision", {"target": "node", "action": "edit", "id": j["id"], "after": {"short_name": "x" * 25}})[0] == 400
    assert a.post("/api/decision", {"target": "node", "action": "edit", "id": j["id"], "after": {"short_name": "   "}})[0] == 400
    st, _ = a.post("/api/decision", {"target": "node", "action": "edit", "id": j["id"], "after": {"short_name": None}})
    assert st == 200
    _, page = a.get("/api/jokes?sort=performed&limit=1")
    assert page["items"][0]["short_name"] == short_name_of(j["title"])
