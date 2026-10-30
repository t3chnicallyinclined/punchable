"""Goldens for pk-creative: synthetic event logs through every creative reducer (every op, every refusal), the
verbatim gate over a corpus, `ntok` (part 1, T3), and Ask (part 2, T4: `creative/askchat.py` over a synthetic
notebook: the flattened sources, `tokens`, retrieve, prompt, finish, header, chip). The Rust tests
(rust/crates/pk-creative/tests/goldens.rs, tests/ask_goldens.rs) replay the same inputs and compare as parsed JSON;
every refusal's message must match. Synthetic data only: nothing of the comic's is here, no hostnames, no other
project's names.

    PYTHONUSERBASE='C:\\Users\\<you>\\AppData\\Roaming\\Python' python rust/tools/gen_creative_goldens.py
        (from the repo root; writes rust/crates/pk-creative/tests/goldens/)

Each store golden is `{"store", "owner", "steps": [{"event", "ok" | "error" + "type", "items"}]}`: the events are
applied one at a time the way `Store.append` does it (on a copy; a refused event changes nothing), `items` is the
state after each step, `type` is the Python exception class (ValueError = a refusal the API answers 400 with;
KeyError / TypeError / AttributeError = a malformed event, a 500 today).
"""
from __future__ import annotations

import copy
import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
os.environ["PK_OWNER_ID"] = "owner"
for p in (ROOT, ROOT / "api-dev", ROOT / "tools" / "explorer"):
    sys.path.insert(0, str(p))

import creative_api as CR  # noqa: E402
import ask_api  # noqa: E402,F401  (registers chats)
import merge_api  # noqa: E402,F401  (registers merges)
import profile_api  # noqa: E402,F401  (registers profile)
import say_api  # noqa: E402,F401  (registers intents)
import suggest_api  # noqa: E402,F401  (registers suggestions)
from creative import askchat as AC  # noqa: E402
from creative import notebook as NB  # noqa: E402
from creative.data import Corpus, venue_short  # noqa: E402
from creative.util import mmss  # noqa: E402
from engine import verify as V  # noqa: E402
from infer.scriptdiff import ntok  # noqa: E402

OUT = ROOT / "rust" / "crates" / "pk-creative" / "tests" / "goldens"
OWNER = "owner"
LONG = " ".join(["w" * 100, "x" * 100, "y" * 100])  # 302 chars: cut at the last space before 280


def ev(n: int, op: str, id_: str, data=None, by: str = OWNER) -> dict:
    """One event; the timestamp counts up so every step is distinguishable."""
    out = {"ts": f"2026-10-29T12:{n // 60:02d}:{n % 60:02d}+00:00", "op": op, "id": id_, "by": by}
    if data is not None:
        out["data"] = data
    return out


def seq(rows: list) -> list[dict]:
    return [ev(n, *row) if isinstance(row, tuple) else ev(n, **row) for n, row in enumerate(rows)]


# every op and every refusal of each reducer; (op, id, data?, by?) tuples, dicts for the odd shapes
EVENTS = {
    "jobs": seq([
        ("enqueue", "job:1", {"kind": "riff", "key": "joke:a", "input": {"idea": "a tight five about meters"}}),
        ("enqueue", "job:1", {"kind": "riff"}),
        ("enqueue", "job:2", {"kind": "whisper"}),
        ("enqueue", "job:3", {}),
        ("enqueue", "job:ps", {"kind": "process_set", "key": "set:s1"}),
        ("claim", "job:nope", {"worker": "w1"}),
        ("claim", "job:1", {"worker": "w1"}, "worker:w1"),
        ("claim", "job:1", {"worker": "w2"}, "worker:w2"),
        ("done", "job:ps", {"result": {}}),
        ("done", "job:1", {"result": {"ideas": [{"text": "a line"}]}}, "worker:w2"),
        ("claim", "job:1", {"worker": "w1"}),
        ("fail", "job:1", {"error": "late"}),
        ("enqueue", "job:4", {"kind": "bridge", "input": {"a": "joke:a", "b": "joke:b"}}),
        ("claim", "job:4", {"worker": "w1"}),
        ("fail", "job:4", {"error": "boom", "retry": True}),
        ("claim", "job:4", {"worker": "w1"}),
        ("fail", "job:4", {"error": {"code": 7}, "retry": True}),
        ("claim", "job:4", {"worker": "w1"}),
        ("fail", "job:4", {"error": "third strike", "retry": True}),
        ("fail", "job:ps", {"retry": True}),
        ("fail", "job:ps", {"error": "x" * 2500, "retry": False}),
        ("poke", "job:1", {}),
        ("enqueue", "job:5", {"kind": "facts", "input": None}),
        ("enqueue", "job:6", {"kind": "ask", "input": [1, 2]}, "ai:ask"),
        ("enqueue", "job:7", {"kind": "import_youtube", "key": "yt:abc"}),
        ("claim", "job:7", {"worker": "phone"}),
        ("done", "job:7", {}),
        ("enqueue", "job:8", {"kind": ["riff"]}),
    ]),
    "storyboards": seq([
        ("create", "sb:1", {"title": "Tonight", "joke_ids": ["joke:a", "joke:b"], "target_s": "1200", "notes": None,
                            "seed": {"from": "ask"}}),
        ("create", "sb:1", {"title": "Again"}),
        ("create", "sb:2", {"target_s": 0}, "ai:story"),
        ("update", "sb:1", {"title": "Tonight v2", "target_s": 600.7, "notes": "tighter"}),
        ("update", "sb:1", {"target_s": "abc"}),
        ("update", "sb:1", {"target_s": None}),
        ("set_jokes", "sb:1", {"joke_ids": ["joke:c", "joke:a"]}),
        ("set_jokes", "sb:1", {}),
        ("set_jokes", "sb:1", {"joke_ids": "ab"}),
        ("set_jokes", "sb:1", {"joke_ids": ["joke:c", "joke:a"]}),
        ("add", "sb:1", {"joke_id": "joke:d"}),
        ("add", "sb:1", {"joke_id": "joke:d"}),
        ("add", "sb:1", {"joke_id": "joke:e", "at": -5}),
        ("add", "sb:1", {"joke_id": "joke:f", "at": 99}),
        ("add", "sb:1", {"joke_id": "joke:g", "at": "1"}),
        ("add", "sb:1", {}),
        ("add", "sb:1", {"joke_id": "joke:h", "at": 1.9}),
        ("remove", "sb:1", {"joke_id": "joke:zz"}),
        ("remove", "sb:1", {"joke_id": "joke:g"}),
        ("remove", "sb:1", {"joke_id": "joke:h"}),
        ("move", "sb:1", {"from": 0, "to": 2}),
        ("move", "sb:1", {"from": 0, "to": 9}),
        ("move", "sb:1", {"from": "1", "to": 0}),
        ("move", "sb:1", {"to": 0}),
        ("move", "sb:1", {"from": -1, "to": 0}),
        ("update", "sb:2", {"notes": "the model's note"}, "ai:story"),
        ("update", "sb:2", {"notes": "his note"}),
        ("delete", "sb:2"),
        ("update", "sb:2", {"title": "gone"}),
        ("update", "sb:nope", {"title": "x"}),
        ("flip", "sb:1", {}),
        ("update", "sb:1", {"title": None}),
        ("create", "sb:3", {"title": "Keys", "joke_ids": {"joke:k": 1}}, "ai:story"),
        ("create", "sb:4", {"title": "Bad ids", "joke_ids": 5}),
    ]),
    "workingsets": seq([
        ("promote", "ws:1", {"title": "Set A", "storyboard_id": "sb:1", "target_s": "900",
                             "bits": [{"joke_id": "joke:a", "target_s": 300}, {"joke_id": "joke:b", "target_s": 600}]}),
        ("promote", "ws:1", {"title": "Set A", "target_s": 1, "bits": []}),
        ("promote", "ws:2", {}),
        ("promote", "ws:2", {"title": "x"}),
        ("promote", "ws:2", {"title": "x", "target_s": 1}),
        ("promote", "ws:2", {"title": "x", "target_s": "no", "bits": []}),
        ("promote", "ws:2", {"title": "Set B", "target_s": 300, "bits": [], "note": "seeded by the model"}, "ai:ws"),
        ("new_version", "ws:1", {}),
        ("new_version", "ws:1", {"note": "tightened", "bits": [{"joke_id": "joke:a", "target_s": 300}]}),
        ("new_version", "ws:1", {"note": "longer", "target_s": 1200, "bits": None}),
        ("new_version", "ws:1", {"note": "emptied", "bits": []}),
        ("new_version", "ws:1", {"note": "bad target", "target_s": "soon"}),
        ("lock_bit", "ws:1", {"joke_id": "joke:a"}),
        ("unlock_bit", "ws:1", {"joke_id": "joke:a"}),
        ("lock_bit", "ws:1", {}),
        ("lock_line", "ws:1", {"joke_id": "joke:b", "text": "the line", "node": "unit:1"}),
        ("lock_line", "ws:1", {"joke_id": "joke:b", "text": "the line"}),
        ("lock_line", "ws:1", {"joke_id": "joke:b"}),
        ("lock_line", "ws:1", {"joke_id": "joke:b", "text": "second"}),
        ("unlock_line", "ws:1", {"joke_id": "joke:zz"}),
        ("unlock_line", "ws:1", {"joke_id": "joke:b"}),
        ("unlock_line", "ws:1", {"joke_id": "joke:b", "text": "the line"}),
        ("unlock_line", "ws:1", {"joke_id": "joke:b", "text": "second"}),
        ("unlock_line", "ws:1", {"joke_id": "joke:b"}),
        ("try_add", "ws:1", {"note_id": "n1", "joke_id": "joke:a", "text": "  slow down  "}),
        ("try_add", "ws:1", {"note_id": "n1", "text": "again"}),
        ("try_add", "ws:1", {"note_id": "n2", "text": "   "}),
        ("try_add", "ws:1", {"note_id": "n2", "text": "x" * 501}),
        ("try_add", "ws:1", {"text": "no id"}),
        ("try_add", "ws:1", {"note_id": "n2", "text": 5}),
        ("try_add", "ws:1", {"note_id": "n2", "text": "x" * 500}, "ai:ws"),
        ("try_done", "ws:1", {"note_id": "n1", "done": 1}),
        ("try_done", "ws:1", {"note_id": "n1", "done": False}),
        ("try_done", "ws:1", {"note_id": "joke:a|put_back", "done": True}),
        ("try_done", "ws:1", {"note_id": "joke:a|put_back", "done": 0}),
        ("try_done", "ws:1", {"done": True}),
        ("try_done", "ws:1", {"note_id": "", "done": True}),
        ("try_done", "ws:1", {"note_id": 7, "done": True}),
        ("update", "ws:1", {"title": "Set A2"}),
        ("update", "ws:1", {}),
        ("delete", "ws:2"),
        ("update", "ws:2", {"title": "gone"}),
        ("update", "ws:nope", {}),
        ("burn", "ws:1", {}),
        ("try_done", "ws:2", {"note_id": "n9", "done": True}),
    ]),
    "venues": seq([
        ("pin", "v:1", {"lat": 51.5, "lon": -0.12}),
        ("pin", "v:1", {"lat": "51", "lon": 0}),
        ("pin", "v:1", {"lat": True, "lon": 0}),
        ("pin", "v:1", {"lat": 91, "lon": 0}),
        ("pin", "v:1", {"lat": 0, "lon": -180.5}),
        ("pin", "v:2", {"lat": 0, "lon": 180}),
        ("rename", "v:1", {"name": "  The Cellar  "}),
        ("rename", "v:1", {"name": "   "}),
        ("rename", "v:1", {"name": "x" * 121}),
        ("rename", "v:1", {}),
        ("rename", "v:1", {"name": "y" * 120}),
        ("reset", "v:1", {}, "import:places"),
        ("nuke", "v:3", {}),
        ("rename", "v:4", {"name": "Loft"}),
        ("pin", "v:4", {"lat": -90, "lon": 180}),
    ]),
    "account": seq([
        ("settings", "settings", {"provider": "chatgpt", "training": True}),
        ("settings", "settings", {}),
        ("settings", "settings", {"provider": "openai"}),
        ("settings", "settings", {"provider": None}),
        ("settings", "settings", {"colour": 1, "alpha": 2}),
        ("settings", "settings", {"cellular": "yes"}),
        ("settings", "settings", {"training": 1}),
        ("settings", "other", {"training": True}),
        ("settings", "settings", {"training": False, "upload_video": True, "cellular": False}),
        ("request_deletion", "deletion", {"by_sub": "sub-1", "would_delete": ["jokes", "sets"]}),
        ("request_deletion", "deletion", {}),
        ("request_deletion", "settings", {}),
        ("close", "settings", {}),
    ]),
    "merges": seq([
        ("merge", "mg:1", {"main": "joke:a", "others": ["joke:b", "joke:c"], "indexes": [3, 4],
                           "snapshot": {"joke:b": {"title": "B"}}, "group": "g1", "reasons": ["same premise"]}),
        ("merge", "mg:1", {"main": "joke:a", "others": ["joke:b"]}),
        ("merge", "mg:2", {"main": "joke:a", "others": []}),
        ("merge", "mg:2", {"others": ["joke:b"]}),
        ("merge", "mg:2", {"main": "joke:a", "others": ["joke:a", "joke:b"]}),
        ("merge", "mg:2", {"main": "joke:a", "others": ["joke:b", "joke:b"]}),
        ("merge", "mg:2", {"main": "joke:x", "others": ["joke:y"], "snapshot": None, "reasons": None}),
        ("undo", "mg:2", {"indexes": [7]}),
        ("undo", "mg:2", {}),
        ("undo", "mg:9", {}),
        ("attach", "at:1", {"joke": "joke:n", "into": "joke:a", "text": "a new note", "date": "2026-01-01",
                            "score": 0.83, "reasons": ["reads like your bit"], "source": "memo"}, "ai:attach"),
        ("attach", "at:1", {"joke": "joke:n", "into": "joke:a"}, "ai:attach"),
        ("attach", "at:2", {"joke": "joke:n", "into": "joke:n"}, "ai:attach"),
        ("attach", "at:2", {"joke": "joke:n"}, "ai:attach"),
        ("attach", "at:2", {"joke": "joke:m", "into": "joke:a"}, "ai:attach"),
        ("detach", "at:1"),
        ("detach", "at:1"),
        ("detach", "mg:1"),
        ("undo", "at:2", {}),
        ("dismiss", "ds:1", {"jokes": ["joke:c", "joke:b", "joke:c"]}),
        ("dismiss", "ds:2", {"jokes": ["joke:a"]}),
        ("dismiss", "ds:2", {}),
        ("dismiss", "ds:1", {"jokes": ["joke:z", "joke:y", "joke:x"]}),
        ("undo", "mg:1", {"indexes": []}),
        ("split", "mg:1", {}),
    ]),
    "chats": seq([
        ("new", "chat:1", {"title": "t" * 130, "joke": "joke:a"}),
        ("new", "chat:1", {"title": "again"}),
        ("new", "chat:2", {"joke": 5}),
        ("ask", "chat:1", {"turn": "t1", "q": "what killed on tuesday?", "job": "job:9", "sources": ["unit:1", "seg:s1#2"]}),
        ("ask", "chat:1", {"turn": "t2", "q": "   "}),
        ("ask", "chat:1", {"turn": "t2"}),
        ("ask", "chat:1", {"turn": "t2", "q": 7}),
        ("ask", "chat:1", {"turn": "t1", "q": "again"}),
        ("ask", "chat:1", {"q": "no turn"}),
        ("answer", "chat:1", {"turn": "t9", "result": {}}, "ai:ask"),
        ("answer", "chat:1", {"turn": "t1", "result": {"answer": "the meter bit", "found": True, "citations": []}}, "ai:ask"),
        ("answer", "chat:1", {"turn": "t1", "result": {}}, "ai:ask"),
        ("ask", "chat:1", {"turn": "t2", "q": "more?"}),
        ("fail", "chat:1", {"turn": "t2", "error": "e" * 600}, "ai:ask"),
        ("fail", "chat:1", {"turn": "t2"}, "ai:ask"),
        ("ask", "chat:1", {"turn": "t3", "q": "third"}),
        ("fail", "chat:1", {"turn": "t3"}, "ai:ask"),
        ("ask", "chat:1", {"turn": "t4", "q": "fourth", "sources": None}),
        ("answer", "chat:1", {"turn": "t4"}, "ai:ask"),
        ("patch", "chat:1", {"title": "  Renamed  "}),
        ("patch", "chat:1", {"title": ""}),
        ("patch", "chat:1", {"title": "x" * 121}),
        ("patch", "chat:1", {"title": None}),
        ("patch", "chat:1", {"deleted": 1}),
        ("patch", "chat:1", {"deleted": 0, "title": "ok"}),
        ("patch", "chat:1", {}),
        ("ask", "chat:nope", {"turn": "t1", "q": "?"}),
        ("close", "chat:1", {}),
        ("patch", "chat:2", {"title": 42}),
    ]),
    "intents": seq([
        ("classify", "in:1", {"text": "what killed last night", "kind": "question", "rule": "leading_wh_or_aux",
                              "context": "tonight"}, "ai:intent"),
        ("classify", "in:1", {"kind": "bit"}, "ai:intent"),
        ("classify", "in:2", {"kind": "poem"}, "ai:intent"),
        ("classify", "in:2", {"kind": "bit"}),
        ("classify", "in:2", {"kind": "command", "command": {"op": "new_set", "minutes": 7}, "context": ""}, "ai:intent"),
        {"op": "classify", "id_": "in:3", "data": {"kind": "bit"}, "by": None},
        ("override", "in:1", {"kind": "bit"}),
        ("override", "in:1", {"kind": "x"}),
        ("override", "in:1", {"kind": "bit"}, "ai:intent"),
        ("override", "in:1", {"kind": "command"}, "import:x"),
        ("acted", "in:1", {"ref": "joke:new"}),
        ("acted", "in:2", {}),
        ("acted", "in:9", {"ref": "x"}),
        ("retract", "in:1", {}),
    ]),
    "suggestions": seq([
        ("keep", "joke:a", {"kind": "not_a_joke"}),
        ("keep", "joke:b", {}),
        ("keep", "joke:c"),
        ("keep", "joke:", {}),
        ("keep", "unit:a", {}),
        ("keep", "joke:a\n", {}),
        ("keep", "joke:" + "a" * 291, {}),
        ("keep", "joke:" + "a" * 290, {}),
        ("unkeep", "joke:a"),
        ("unkeep", "joke:zz"),
        ("hide", "joke:b", {}),
        ("unkeep", "nope"),
    ]),
    "profile": seq([
        ("put", "profile:other", {"text": "x"}),
        ("put", "profile:voice", {"text": 5}),
        ("learn", "profile:voice", {"kept": "nope"}),
        ("learn", "profile:voice", {"kept": None}),
        ("learn", "profile:voice", {}),
        ("learn", "profile:voice", {"kept": [], "dropped": []}),
        ("learn", "profile:voice", {"kept": ["  A kept   line ", {"text": "With bit", "bit": "joke:a", "date": "2026-02-03",
                                                                   "edited": True}, 7, {"text": 5}, "", {"text": "No bit", "bit": ""}],
                                    "dropped": ["a kept line", "Passed over"]}),
        ("put", "profile:voice", {"text": "first voice", "kind": "voice"}),
        ("put", "profile:voice", {"text": "second voice"}, "import:profile"),
        ("learn", "profile:voice", {"dropped": ["with BIT"]}),
        ("learn", "profile:voice", {"kept": [LONG]}),
        ("learn", "profile:voice", {"kept": [f"kept line {n}" for n in range(30)]}),
        ("learn", "profile:voice", {"dropped": [f"dropped line {n}" for n in range(30)], "kept": ["dropped line 3"]}),
        ("put", "profile:voice", {"text": "third", "kind": None}),
        ("forget", "profile:voice", {}),
    ]),
}

# the verbatim corpus: (quote, source); both Python copies are run and recorded
VERBATIM = [
    ("the meter asked", "So the meter asked for my pronouns before my card"),
    ("  the meter asked  ", "So the meter asked for my pronouns before my card"),
    ("\"the meter asked\"", "So the meter asked for my pronouns before my card"),
    ("\u201cthe meter asked\u201d", "So the meter asked for my pronouns before my card"),
    ("the meter asked...", "So the meter asked for my pronouns before my card"),
    ("...the meter asked\u2026", "So the meter asked for my pronouns before my card"),
    ("The meter asked", "So the meter asked for my pronouns before my card"),
    ("the meter begged", "So the meter asked for my pronouns before my card"),
    ("me", "So the meter asked for my pronouns before my card"),
    ("", "So the meter asked"),
    ("the meter", ""),
    ("   ", "So the meter asked"),
    ("don't park there", "He said \u201cdon\u2019t park there\u201d \u2014 twice\u2026 really"),
    ("\"don't park there\"", "He said \u201cdon\u2019t park there\u201d \u2014 twice\u2026 really"),
    ("there\" - twice...", "He said \u201cdon\u2019t park there\u201d \u2014 twice\u2026 really"),
    ("there - twice...", "He said \u201cdon\u2019t park there\u201d \u2014 twice\u2026 really"),
    ("twice... really", "He said \u201cdon\u2019t park there\u201d \u2014 twice\u2026 really"),
    ("twice\u2026 really", "He said \u201cdon\u2019t park there\u201d - twice... really"),
    ("don\u2019t park", "He said don't park there"),
    ("a - b", "x a \u2013 b y a \u2014 b z"),
    ("a \u2014 b", "x a \u2013 b y a \u2014 b z"),
    ("a\u00a0b c", "say a b c now"),
    ("a b c", "say a\u00a0b\u00a0c now"),
    ("a b c", "say a  b\tc now"),
    ("a  b   c", "say a b c now"),
    ("a b c", "say a\u001cb\u001dc now"),
    ("\u001ca b c\u001f", "say a b c now"),
    ("a\u001cb", "say a b now"),
    ("a b c", "\U0001F600\U0001F600 a  b\u001cc"),
    ("\U0001F600 a b", "x \U0001F600 a  b y"),
    ("\U0001F600 a b", "x \U0001F600\U0001F600 a  b y"),
    ("a\u2026\u2026", "xx a\u2026 yy"),
    ("\"x\u2026\"", "x\u2026 and more"),
    ("\u2026", "\u2026\u2026\u2026"),
    ("..", "..."),
    ("'tis", "well 'tis the season"),
    ("\u2032tis the", "well 'tis the season"),
    ("rock \u2033n\u2033 roll", "rock \"n\" roll"),
    ("minus \u2212 one", "minus - one"),
    ("much longer than the source", "short"),
    ("the whole thing", "the whole thing"),
    ("a\nb", "x a b y"),
    ("a b", "x a\nb y"),
    ("... a", "x a y"),
    ("a b a b", "a b a b a b"),
    ("a  b", "a b a  b"),
    ("abc", "\U0001F600abc\U0001F600"),
    ("\u2018abc\u2019", "x 'abc' y"),
    ("'abc'", "x \u2018abc\u2019 y"),
    ("\u201e\u2018quoted\u2019", "it was \u201c'quoted'\u201d he said"),
    ("..abc", "x ..abc y"),
    ("abc....", "x abc. y"),
    ("the", "the the"),
    ("\u00a0\u00a0abc", "zabcz"),
    ("ab\u001cc", "x ab c"),
]

NTOK = ["Won\u2019t", "10", "20", "'Hello!'", "\u0130", "\u00dfeta", "\u2014", "it's", "''", "don't'", "3", "30",
        "Caf\u00e9", "MVC2", "under_score", "a-b", "", "  spaced  ", "1,000", "\U0001F600"]


# ============================================================== askchat (T4): a synthetic notebook
# Ten jokes, five sets, invented words; the shapes are the real ones (units with dates, an idea typed in the app, a
# voice note, moments with ratings and laughs, a crowd-work and a banter segment, an empty transcript, a transcript
# with leading whitespace, a segment whose joke is unknown).
LONG_C = ("Charlie premise about cars and the long drive home, the radio, the exits, the same billboard twice, " * 13
          + "and then the gridlock on the bridge, honestly.")
ASK_TEXT = {
    "unit:a1": "Alpha opener line one. The alpha punch lands here every time.",
    "unit:b1": "Bravo setup about the dog. The purple monkey dishwasher line here. Bravo closer words.",
    "unit:b2": "Bravo older draft about the dog and the purple monkey. Dishwasher humming.",
    "unit:b3": "Bravo oldest draft: a dog, a purple monkey, no dishwasher yet.",
    "unit:b4": "Bravo scrap: purple monkey, dishwasher, dog, the lot.",
    "unit:c1": LONG_C,
    "unit:d1": "Delta bit about my niece and the laundromat place.",
    "unit:e1": "Echo bit about my niece and her soccer schedule.",
    "unit:f1": "Foxtrot never performed idea about planes and pilots up there.",
    "unit:h1": "Hotel voice note: the meter asked for my pronouns \u2014 I don\u2019t have a card.",
}
ASK_DATES = {"unit:a1": "2015-06-26T10:00:00", "unit:b1": "2024-01-10", "unit:b2": "2023-05-01", "unit:b3": "2022-01-01",
             "unit:b4": "2021-01-01", "unit:d1": "2015-03-03", "unit:e1": "2024-04-04", "unit:f1": "2016-07-07",
             "unit:h1": "2024-08-08"}
ASK_SETS = {  # key -> venue (the set node's), then the segments
    "2024-01-01_x": ("The Corner Room", [
        (1, "joke:a", 0, 60_000, "known", "killed", 7, ASK_TEXT["unit:a1"]),
        (2, "joke:b", 70_500, 130_000, "known", "worked", 3, ASK_TEXT["unit:b1"]),
        (3, None, 130_000, 150_000, "crowd_work", None, 0, "So who here has a car?"),
        (4, "joke:c", 150_000, 210_000, "known", "meh", 1, "Charlie premise about cars and traffic and more traffic today."),
        (5, None, 210_000, 215_000, "banter", None, 0, "Thanks for coming out tonight folks."),
    ]),
    "2024-02-01_y": ("Seaside Comedy Club", [
        (1, "joke:a", 0, 60_000, "known", "worked", 4, ASK_TEXT["unit:a1"]),
        (2, "joke:b", 60_000, 150_000, "known", "died", 0, "Bravo setup about the dog. Bravo closer words."),
        (3, "joke:d", 150_000, 210_000, "new", "killed", 12, ASK_TEXT["unit:d1"]),
    ]),
    "2023-03-01_z": ("Mabel's (upstairs)", [
        (1, "joke:b", 0, 40_000, "known", "killed", 9, "Bravo setup about the dog. Bravo closer words about the cat today."),
        (2, "joke:a", 40_000, 70_000, "known", "worked", 2, "Alpha opener line one."),
        (3, "joke:a", 71_500, 100_000, "known", "worked", 2, "The alpha punch lands here every time."),
        (4, "joke:zz", 100_000, 120_000, "known", "meh", 1, "A line about a zebra and a quokka."),
        (5, None, 120_000, 121_000, "known", None, 0, "   "),
        (6, "joke:h", 121_000, 150_000, "known", "worked", 1, "  Leading space moment about the meter.  "),
    ]),
    "2024-05-05_w": ("Unknown", [
        (1, "joke:e", 0, 60_000, "known", "meh", 1, ASK_TEXT["unit:e1"]),
    ]),
    "2015-09-09_v": ("maybe The Loft", [
        (1, "joke:b", 5_000, 60_000, "known", "died", 0, "Bravo setup about the dog. Nobody laughed at the purple monkey."),
        (2, "joke:a", 60_000, 90_000, "known", "died", 0, "Alpha opener line one, to silence."),
    ]),
}


def ask_corpus() -> Corpus:
    nodes, edges = [], []
    for k in "abcdefh":
        nodes.append({"id": f"joke:{k}", "type": "joke", "title": k.upper() + " bit (draft)", "stage": "testing",
                      "lead_unit": f"unit:{k}1"})
    for uid, text in ASK_TEXT.items():
        n = {"id": uid, "type": "unit", "kind": "setup+punch", "text": text}
        if uid in ASK_DATES:
            n["date"] = ASK_DATES[uid]
        if uid == "unit:h1":
            n["source"] = "memo"  # a voice note, not a doc
        nodes.append(n)
        jid = f"joke:{uid[5]}"
        edges.append({"id": f"e:p{uid[5:]}", "src": uid, "dst": jid, "kind": "part_of", "status": "inferred",
                      "confidence": 0.9, "evidence": []})
    for key, (venue, _) in ASK_SETS.items():
        nodes.append({"id": f"set:{key}", "type": "set", "venue": venue})
    # his idea typed in the app (no doc yet): the summary is the source text; the node's own `text` is what
    # `Corpus.text_of` returns, so the gate reads a different string from the one the prompt shows
    decisions = [{"target": "node", "action": "add", "id": "joke:g", "by": OWNER, "after": {
        "type": "joke", "title": "Golf idea (jot)", "summary": "Golf idea about caddies and the cart that talks back.",
        "text": "Golf idea typed: caddies and the cart that talks back.", "date": "2024-09-09T08:00:00+00:00"}}]
    analyses = {}
    for key, (_, segs) in ASK_SETS.items():
        rows = []
        for idx, joke, start, end, status, rating, laughs, text in segs:
            rows.append({"idx": idx, "start_ms": start, "end_ms": end, "title": f"T{idx}", "status": status,
                         "joke_id": joke, "transcript": text, "ai_rating": rating,
                         "laughs": {"count": laughs, "laugh_s": float(laughs), "window_s": (end - start) / 1000}})
        analyses[key] = {"set": key, "summary": {"laugh_s_per_min": 5.0, "room_heat": 1.0}, "jokes": rows}
    return Corpus({"nodes": nodes, "edges": edges}, analyses, decisions=decisions)


TOKENS_CASES = ["The purple monkey's dishwasher", "Mabel\u2019s and Mabel's", "boxes churches babies glasses buses ties",
                "it's its don't I'm we're", "AI TV us US go ex", "2015 90s 100s 3 42", "\u0130stanbul Caf\u00e9 na\u00efve",
                "'quoted' 'em ol' ''", "", "   ", "a-b c_d e.f", "hello,world!what?", "the and that", "Niece's nieces nieces' NIECE",
                "process processes success successes", "us", "dishes wishes fixes", "kills killed killing", "a'b'c's"]
MONTH_YEAR_CASES = ["2015-06-26", "2015", "2015-13-01", "2015-00-01", "2015-1", "", None, "2015-06", "2024-12-31T10:00:00",
                    "abcd-ef-gh", "2015-0a-01", "2015-12"]
MMSS_CASES = [0, 0.5, 1.5, 2.5, 59.5, 60, 70.5, 71.5, 3599.4, 3599.5, 3600, 125.49, 125.5, 126.5, 0.0005]
SHORT_TITLE_CASES = ["A bit (v2)", "  spaced  ", "(all)", "no parens", None, "", "Two (one) (two)", " x (y"]
SNIP_TEXT = "aaa bbb ccc ddd eee fff ggg hhh iii jjj KKK lll mmm nnn ooo ppp"
SNIPPET_CASES = [  # (text, terms, n)
    (SNIP_TEXT, ["kkk"], 40), (SNIP_TEXT, [], 40), (SNIP_TEXT, ["zzz"], 40), (SNIP_TEXT, ["aaa"], 40),
    (SNIP_TEXT, ["ppp"], 40), (SNIP_TEXT, ["ooo", "ccc"], 20), (SNIP_TEXT, ["kkk"], 1000), (SNIP_TEXT, ["kkk"], 8),
    ("nospaceshereatallnospaceshereatall", ["here"], 12), (SNIP_TEXT, ["mmm"], 12), (SNIP_TEXT, ["aaa"], 4),
    ("x" * 30 + " " + "y" * 30, ["yyy"], 20), ("\u00e9\u00e9\u00e9 \u00e9\u00e9\u00e9 kkk \u00e9\u00e9\u00e9 zzz", ["kkk"], 10),
]
HIST_B = [{"q": "zebra quokka", "answer": "The Bravo bit [1].", "cited": ["unit:b1"]}]
RETRIEVE_CASES = [  # name, question, history, cap, pin (a joke id for pinned(), or None)
    ("plain", "what did I write about the purple monkey dishwasher?", None, AC.MAX_SOURCES, None),
    ("qstop_only", "what did I write?", None, AC.MAX_SOURCES, None),
    ("qstop_killed", "which bits killed?", None, AC.MAX_SOURCES, None),
    ("room_corner", "what killed at The Corner Room?", None, AC.MAX_SOURCES, None),
    ("room_ac", "how did the dog bit go in AC?", None, AC.MAX_SOURCES, None),
    ("room_seaside_word", "the seaside night, what worked?", None, AC.MAX_SOURCES, None),
    ("room_mabels", "anything from Mabel's?", None, AC.MAX_SOURCES, None),
    ("room_mabels_curly", "anything from Mabel\u2019s?", None, AC.MAX_SOURCES, None),
    ("room_loft_quiet", "which went quiet at the loft?", None, AC.MAX_SOURCES, None),
    ("room_unknown_skipped", "what about the unknown room?", None, AC.MAX_SOURCES, None),
    ("year_2015", "what did I write about my niece in 2015?", None, AC.MAX_SOURCES, None),
    ("year_none_match", "purple monkey in 1999?", None, AC.MAX_SOURCES, None),
    ("year_two", "alpha opener in 2015 or 2024", None, AC.MAX_SOURCES, None),
    ("follow_up_killed", "which of those killed?", HIST_B, AC.MAX_SOURCES, None),
    ("follow_up_plain", "and the dishwasher line?", HIST_B, AC.MAX_SOURCES, None),
    ("follow_up_room", "which of those were at the corner room?", HIST_B, AC.MAX_SOURCES, None),
    ("carried_dupes", "which of those killed?",
     [{"q": "zebra quokka", "answer": "x", "cited": ["unit:b1", "unit:b1", "unit:nope", "seg:2023-03-01_z#4"]}],
     AC.MAX_SOURCES, None),
    ("history_five_turns", "which of those killed?",
     [{"q": "gridlock bridge", "cited": ["unit:c1"]}, {"q": "laundromat"}, {"q": "soccer", "cited": ["unit:e1"]},
      {"q": "", "cited": None}, {"q": "zebra quokka", "answer": None, "cited": ["unit:b1"]}], AC.MAX_SOURCES, None),
    ("history_no_cited", "purple monkey", [{"q": "the laundromat place", "answer": "Delta [1]."}], AC.MAX_SOURCES, None),
    ("pinned_b", "what else do I have like this?", None, AC.MAX_SOURCES, "joke:b"),
    ("pinned_b_cap", "purple monkey dishwasher dog", None, 4, "joke:b"),
    ("pinned_unknown", "purple monkey", None, AC.MAX_SOURCES, "joke:nope"),
    ("per_joke_cap", "purple monkey dishwasher", None, AC.MAX_SOURCES, None),
    ("cap_two", "alpha opener purple monkey niece", None, 2, None),
    ("cap_zero", "alpha opener", None, 0, None),
    ("worst_bombed", "what bombed worst?", None, AC.MAX_SOURCES, None),
    ("laughs_min_ten", "my niece, best laughs?", None, AC.MAX_SOURCES, None),
    ("long_text_hit", "the gridlock on the bridge", None, AC.MAX_SOURCES, None),
    ("note_and_idea", "the meter and my pronouns, the caddies", None, AC.MAX_SOURCES, None),
]
PROMPT_CASES = [  # name, question, history, scope, named
    ("plain", "what did I write about the purple monkey dishwasher?", None, None, False),
    ("named_with_history", "which of those killed?", HIST_B, None, True),
    ("scope", "what else do I have like this?", None, "joke:b", False),
    ("long_text_hit", "the gridlock on the bridge", None, None, False),
    ("long_text_no_hit", "the long drive home", [{"q": "drive"}, {"q": "", "answer": "skipped"}], None, True),
    ("history_five_turns", "which of those killed?",
     [{"q": "gridlock bridge", "cited": ["unit:c1"]}, {"q": "laundromat"}, {"q": "soccer", "cited": ["unit:e1"]},
      {"q": "", "cited": None}, {"q": "zebra quokka", "answer": None, "cited": ["unit:b1"]}], "joke:g", False),
]
FIN_IDS = ["unit:b1", "seg:2024-01-01_x#2", "joke:g", "unit:h1", "unit:nope", "seg:2023-03-01_z#6"]
FINISH_CASES = [  # name, data, provider, model
    ("all_good", {"found": True, "answer": "You wrote about a dog and a dishwasher [S1]. On tape it worked [S2].",
                  "citations": [{"source": "S1", "quote": "The purple monkey dishwasher line here."},
                                {"source": "S2", "quote": "Bravo setup about the dog."}]}, "anthropic", "m-1"),
    ("not_verbatim", {"found": True, "answer": "The dishwasher bit [S1].",
                      "citations": [{"source": "S1", "quote": "The purple monkey washed the dishes."}]}, "anthropic", "m-1"),
    ("typography_ok", {"found": True, "answer": "The meter line [S4] and the monkey [S1].",
                       "citations": [{"source": "S4", "quote": "\"the meter asked for my pronouns - I don't have a card\""},
                                     {"source": "S1", "quote": "\u201c\u2026purple monkey dishwasher line here.\u201d"}]}, None, None),
    ("unknown_source", {"found": True, "answer": "It killed [S9] everywhere [S1] and [S5].",
                        "citations": [{"source": "S9", "quote": "anything"}, {"source": "S5", "quote": "x" * 250},
                                      {"source": "S1", "quote": ""}]}, "p", "m"),
    ("malformed_markers", {"found": True, "answer": "Fine [S3a] and [S1, S2b] then [S1].",
                           "citations": [{"source": "S1", "quote": "Bravo closer words."}]}, "p", "m"),
    ("duplicate_citations", {"found": True, "answer": "Twice [S1] and again [S1], [s2].",
                             "citations": [{"source": "S1", "quote": "Bravo closer words."},
                                           {"source": "S1", "quote": "The purple monkey dishwasher line here."},
                                           {"source": "[s2]", "quote": "Bravo closer words."}]}, "p", "m"),
    ("prose_quote_removed", {"found": True,
                             "answer": "You said \"the monkey ate all of my dishes\" once [S1] and \u201cthe cat did nothing at all\u201d too.",
                             "citations": [{"source": "S1", "quote": "Bravo closer words."}]}, "p", "m"),
    ("prose_quote_short_kept", {"found": True, "answer": "You said \"the monkey ate\" once [S1].",
                                "citations": [{"source": "S1", "quote": ""}]}, "p", "m"),
    ("prose_quote_in_source_kept", {"found": True,
                                    "answer": "You wrote \"The purple monkey dishwasher line here.\" and \"Bravo setup about the dog\" [S1] [S2].",
                                    "citations": []}, "p", "m"),
    ("found_no_chips", {"found": True, "answer": "It killed in a room I made up.", "citations": []}, "p", "m"),
    ("found_false", {"found": False, "answer": "", "citations": [{"source": "S1", "quote": "nope"}]}, "p", "m"),
    ("non_dict", "nope", "p", "m"),
    ("none_data", None, None, None),
    ("double_spaces", {"found": True, "answer": "  Two  spaces   here [S1] and\t\ttabs\t here [ S1 ; S2 ].  ",
                       "citations": [{"source": "s1", "quote": "Bravo closer words."}]}, "p", "m"),
    ("combined_marker_unknown", {"found": True, "answer": "All [S2,S9;s1] of them.",
                                 "citations": [{"source": 2, "quote": "x"}, "S1", None, {"source": "S1"}]}, "p", "m"),
    ("citations_dict", {"found": True, "answer": "Hm [S1].", "citations": {"source": "S1", "quote": "x"}}, "p", "m"),
    ("citations_number", {"found": True, "answer": "Hm [S1].", "citations": 5}, "p", "m"),
    ("answer_number_found_string", {"found": "yes", "answer": 7, "citations": [{"source": "S1", "quote": ""}]}, "p", "m"),
    ("found_zero", {"found": 0, "answer": "x [S1]", "citations": []}, "p", "m"),
    ("gate_text_idea", {"found": True, "answer": "Typed [S3] and taped [S6].",
                        "citations": [{"source": "S3", "quote": "Golf idea typed"},
                                      {"source": "S6", "quote": "Leading space moment about the meter."}]}, "p", "m"),
    ("gate_text_prose", {"found": True, "answer": "You typed \"Golf idea typed: caddies and the cart\" [S3].",
                         "citations": []}, "p", "m"),
    ("moment_quote", {"found": True, "answer": "At the corner room [S2].",
                      "citations": [{"source": "S2", "quote": "Bravo closer words"}]}, "p", "m"),
    ("uncited_but_quoted", {"found": True, "answer": "No markers at all.",
                            "citations": [{"source": "S2", "quote": "Bravo closer words"},
                                          {"source": "S4", "quote": "I don\u2019t have a card"}]}, "p", "m"),
]
OWNER_NAMES = ["Pat Example", "", "   ", "the comic", None, " Pat ", "The Comic"]


def retrieve_with_scores(c, question, history, cap, pin):
    """`retrieve` plus the score dict it sorted (captured from its one `sorted` call): the Rust side must sum the
    same floats, not just land on the same order."""
    import builtins
    seen = []

    def spy(it, *a, **k):
        if isinstance(it, dict):
            seen.append(dict(it))
        return builtins.sorted(it, *a, **k)
    AC.sorted = spy
    try:
        ids = AC.retrieve(c, question, history, cap, pin)
    finally:
        del AC.sorted
    assert len(seen) == 1, "retrieve sorts its score dict exactly once"
    score = seen[0]
    vals = sorted(score.values())
    for a, b in zip(vals, vals[1:]):  # a tie must be an exact tie (the same arithmetic), never a near one
        assert a == b or b - a > 1e-9, f"{question!r}: scores {a!r} and {b!r} too close to sort robustly"
    return ids, {k: score[k] for k in sorted(score)}


def owner_env(name):
    if name is None:
        os.environ.pop("PK_COMIC_NAME", None)
    else:
        os.environ["PK_COMIC_NAME"] = name


def main_ask() -> dict:
    from creative import voice
    c = ask_corpus()
    S = AC.sources(c)
    out_sources = {}
    for sid, s in S.items():
        row = dict(s)
        gate = c.text_of(sid) or s["text"]
        if gate != s["text"]:
            row["gate_text"] = gate
        out_sources[sid] = dict(sorted(row.items()))  # keys sorted per row; the rows keep the Python's order
    notebook = {"sources": out_sources,
                "rooms": {k: venue_short(s.get("venue"), None) for k, s in c.sets.items()},
                "joke_titles": {jid: c.title(jid) for jid in c.jokes}}
    (OUT / "ask_notebook.json").write_text(json.dumps(notebook, ensure_ascii=False, indent=1), encoding="utf-8")

    owners = []
    for name in OWNER_NAMES:
        owner_env(name)
        owners.append({"name": name, "owner": voice.material_owner(), "system_text": AC.system_text()})
    owner_env(None)
    text = {"tokens": [[t, NB.tokens(t)] for t in TOKENS_CASES],
            "month_year": [[d, NB.month_year(d)] for d in MONTH_YEAR_CASES],
            "mmss": [[x, mmss(x)] for x in MMSS_CASES],
            "short_title": [[t, NB.short_title(t)] for t in SHORT_TITLE_CASES],
            "snippet": [{"text": t, "terms": sorted(terms), "n": n, "out": AC._snippet(t, set(terms), n)}
                        for t, terms, n in SNIPPET_CASES],
            "owners": owners, "rules": AC.RULES, "refusal": AC.REFUSAL,
            "headers": {sid: AC.header(s) for sid, s in S.items()},
            "chips": [AC.chip(s, i, "a quote" if i % 2 else None) for i, s in enumerate(S.values(), 1)]}
    (OUT / "ask_text.json").write_text(json.dumps(text, ensure_ascii=False, indent=1), encoding="utf-8")

    retrieve = []
    for name, q, hist, cap, pin_joke in RETRIEVE_CASES:
        pin = AC.pinned(c, pin_joke)
        ids, score = retrieve_with_scores(c, q, hist, cap, pin)
        retrieve.append({"name": name, "question": q, "history": hist, "cap": cap, "pin_joke": pin_joke,
                         "pinned": sorted(pin), "rooms": sorted(AC.rooms_named(c, q)), "scores": score, "ids": ids})
    (OUT / "ask_retrieve.json").write_text(json.dumps(retrieve, ensure_ascii=False, indent=1), encoding="utf-8")

    prompts = []
    for name, q, hist, scope_joke, named in PROMPT_CASES:
        owner_env("Pat Example" if named else None)
        ids = AC.retrieve(c, q, hist)
        scope = AC.scope_line(c, scope_joke)
        prompts.append({"name": name, "question": q, "history": hist, "ids": ids, "scope_joke": scope_joke, "scope": scope,
                        "owner": voice.material_owner(), "messages": AC.prompt(c, q, hist, ids, scope)})
    owner_env(None)
    prompts.append({"name": "unknown_id", "question": "x", "history": None, "ids": ["unit:b1", "unit:nope"], "scope_joke": None,
                    "scope": None, "owner": voice.material_owner(), "error": "'unit:nope'", "type": "KeyError"})
    try:
        AC.prompt(c, "x", None, ["unit:b1", "unit:nope"])
    except KeyError as x:
        assert str(x) == "'unit:nope'"
    else:
        raise AssertionError("prompt accepted an unknown id")
    (OUT / "ask_prompt.json").write_text(json.dumps(prompts, ensure_ascii=False, indent=1), encoding="utf-8")

    finishes = []
    for name, data, provider, model in FINISH_CASES:
        try:
            out = AC.finish(c, FIN_IDS, data, provider, model)
        except Exception as x:  # noqa: BLE001  (recorded: the Rust side must fault the same way)
            finishes.append({"name": name, "ids": FIN_IDS, "data": data, "provider": provider, "model": model,
                             "error": str(x), "type": type(x).__name__})
            continue
        finishes.append({"name": name, "ids": FIN_IDS, "data": data, "provider": provider, "model": model, "out": out})
    refusals = [{"ids": ids, "out": AC.refusal(ids)} for ids in ([], ["unit:b1", "x", "y"], None)]
    (OUT / "ask_finish.json").write_text(json.dumps({"finish": finishes, "refusal": refusals}, ensure_ascii=False, indent=1),
                                         encoding="utf-8")
    return {"sources": len(S), "retrieve": len(retrieve), "prompt": len(prompts), "finish": len(finishes),
            "finish_errors": sum(1 for f in finishes if "error" in f), "tokens": len(TOKENS_CASES),
            "snippet": len(SNIPPET_CASES)}


def run(apply, events: list[dict]) -> list[dict]:
    """Each event through `apply` on a copy, the way Store.append probes: a refused event changes nothing."""
    items: dict = {}
    steps = []
    for e in events:
        probe = json.loads(json.dumps(items))
        try:
            apply(probe, copy.deepcopy(e))
        except Exception as x:  # noqa: BLE001  (the whole point: record what the Python raises)
            steps.append({"event": e, "error": str(x), "type": type(x).__name__, "items": copy.deepcopy(items)})
            continue
        items = probe
        steps.append({"event": e, "ok": True, "items": copy.deepcopy(items)})
    return steps


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    counts = {}
    for name, events in EVENTS.items():
        steps = run(CR.APPLY[name], events)
        (OUT / f"store_{name}.json").write_text(
            json.dumps({"store": name, "owner": OWNER, "steps": steps}, ensure_ascii=False, indent=1), encoding="utf-8")
        counts[name] = (len(steps), sum(1 for s in steps if "error" in s))
    corpus = []
    differ = []
    for q, s in VERBATIM:
        a, v = AC.verbatim(q, s), V.find_verbatim(q, s)
        corpus.append({"quote": q, "source": s, "askchat": a, "verify": v, "same": a == v})
        if a != v:
            differ.append((q, a, v))
    (OUT / "verbatim.json").write_text(json.dumps(corpus, ensure_ascii=False, indent=1), encoding="utf-8")
    (OUT / "ntok.json").write_text(json.dumps([[t, ntok(t)] for t in NTOK], ensure_ascii=False, indent=1), encoding="utf-8")
    print("wrote", OUT)
    for name, (n, bad) in counts.items():
        print(f"  {name}: {n} events, {bad} refused")
    print(f"  verbatim: {len(corpus)} pairs, {len(differ)} where askchat and verify differ: {differ!r}")
    print(f"  ntok: {len(NTOK)}")
    ask = main_ask()
    print("  askchat: " + ", ".join(f"{k} {v}" for k, v in ask.items()))


if __name__ == "__main__":
    main()
