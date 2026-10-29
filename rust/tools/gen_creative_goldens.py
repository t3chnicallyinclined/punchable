"""Goldens for pk-creative part 1: synthetic event logs through every creative reducer (every op, every refusal), the
verbatim gate over a corpus, and `ntok`. The Rust tests (rust/crates/pk-creative/tests/goldens.rs) replay the same
events and compare as parsed JSON; every refusal's message must match. Synthetic data only: nothing of the comic's
is here, no hostnames, no other project's names.

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


if __name__ == "__main__":
    main()
