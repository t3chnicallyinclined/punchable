"""Goldens for pk-creative's `say` (the Rust port of the classifier and the command parser in api-dev/say_api.py):
`classify`, `parse_command` and `match_bits` over a corpus of phrasings (questions with and without a mark, premises,
fillers, requests, the two commands in their spellings, number words, apostrophes of both kinds, case, the things
that look like one and are not) and the synthetic notebook of notebook_fixture.py. The weekday a command without a
target falls back to is pinned (the Python asks the clock). The Rust test is crates/pk-creative/tests/say_goldens.rs.

    python rust/tools/gen_say_goldens.py        (from the repo root; writes crates/pk-creative/tests/goldens/say.json)
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
import types
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
for p in (ROOT, ROOT / "api-dev", ROOT / "tools" / "explorer", Path(__file__).resolve().parent):
    sys.path.insert(0, str(p))
os.environ["PK_OWNER_ID"] = "owner"
import gen_corpus_goldens as G  # noqa: E402

OUT = ROOT / "rust" / "crates" / "pk-creative" / "tests" / "goldens" / "say.json"

PHRASES = [
    # questions: the mark, the wh-words, the auxiliaries, the requests
    "What's my strongest closer?", "what is my best bit", "whats my last set", "when did I do the dentist bit", "where was that",
    "which bit killed", "who laughed", "why does the airport bit work", "how do I get from A to B", "hows the closer",
    "do I have a bit about dogs", "does the parrot bit work", "did I try the belt one", "can you find the kale bit",
    "should I open with the dentist", "would the landlord bit work", "is the parrot bit tight", "are my bits any good",
    "was that a good set", "have I done this one", "any bit about trains", "anything about socks", "any", "got a bit about cats",
    "got", "find", "find the dentist bit", "look at my closer", "look", "list my bits", "show me my last set", "pull up the airport bit",
    "give me a closer", "build me a tight 5", "make a 10 for the club", "tell me about the parrot bit", "search for belts",
    "remind me what I said about dryers",
    # fillers in front
    "um what's my best bit", "oh and the dentist bit", "so uh like the airport", "well hmm, how do I end", "ok okay also wait",
    "and", "oh", "but why", "like what", "hey you there", "yeah", "um uh",
    # a question mark makes anything one
    "The dentist is a drug dealer?", "What if the dog could talk?", "  spaced   out   question ?  ", "?",
    # premises are bits
    "What if the dentist charged by the minute", "what if", "imagine a world without socks", "picture this: a parrot lawyer",
    "you know how airports are", "you know when you lose a sock", "you know what I hate", "so my landlord is a ghost",
    "my dad bought a drone", "my mom thinks I'm a doctor", "my wife says I snore", "my husband hums", "my boyfriend is a raccoon", "my kid ate a crayon", "my kids", "my daughter learned to swear", "my brother",
    "my sister runs a cult", "my roommate is a mime", "my neighbor's dog", "my neighbour", "my landlord", "my boss is a bot",
    "my doctor said breathe", "my dentist", "my uber was late", "my therapist", "my cat judges me", "my dog",
    # my material
    "how are my bits doing", "my bits", "my best", "tell me my strongest", "what about my tight 5", "my last set was rough",
    "my 5", "my 12 minutes", "my closer needs work", "my opener", "my notebook", "my stuff", "my weakest", "my jokes", "my sets",
    # commands
    "new 7 for friday", "new 5", "make a 10 for the club", "build me a tight 5", "start a quick 15 for the late show",
    "create another 20", "give me a fresh 8", "set up a 12 minute set", "setup 3", "a new 6 spot", "new 240", "new 241", "new 0",
    "new seven for friday", "new forty five", "new forty-five for tuesday", "make a clean sixty", "build me a killer ten set list",
    "make me a solid 7 min set on friday night", "new 7 for the.", "new 7 for", "new 5.", "new 5!", "New 5 For Friday",
    "new 5 for 'the club'", "new 10 for THE late show", "new 10 for an open mic", "new 7 at the cellar", "new 7 on saturday",
    "add the kale bit to tonight", "put the dentist bit in tonight's set", "add parrot to tonight's set list",
    "stick the airport bit into tonight", "throw socks onto tonight's setlist", "drop the belt one in my set list for tonight",
    "slot the closer in the set", "add in the kale bit to tonight", "add the kale bit to the setlist", "add kale to my list",
    "add the kale bit", "add to tonight", "put it on tonight", "add the kale bit to tonight?", "add the kale bit to tonight.",
    "add the 'kale' bit to tonight", "add THE Kale Bit to Tonights set", "add  the   kale  bit  to  tonight",
    "put the dentist bit in tonight’s set", "add the kale bit to tonights list",
    # bits (the default)
    "The dentist is a drug dealer", "I went to the airport and they took my belt", "Dogs are just small wolves with anxiety",
    "", "   ", "Okay", "my", "new", "a new", "build", "make it work", "write that down", "put that in a bit", "run", "sort of",
    "I made a 5 minute set for a friend", "tell my mother nothing", "get lost", "look at me now",
    "Why did the chicken cross the road", "What a day", "whatever", "who cares", "Is this thing on", "How about that",
    "Remind me never to fly again", "Give it up for the band", "Show business is hard", "Make America great", "Pull the cord",
    "Do not go gentle", "Am I the only one", "Had a dream about a sock",
    "café owners are like that", "Über driver tonight", "niño", "’tis the season", "it’s a bit about my mother",
]

BITS_QUERIES = [
    ("kale bit", ["kale"]), ("airport", ["airport"]), ("the airport bit", ["airport"]), ("parrot lawyer", ["parrot", "lawyer"]),
    ("socks", ["socks"]), ("sock", ["sock"]), ("dryer", ["dryer"]), ("nothing like it", ["zebra"]), ("", []),
    ("airport bins", ["airport", "bins"]), ("landlord rent", ["landlord", "rent"]), ("belt", ["belt"]), ("bins", ["bin"]),
    ("the", []), ("air", ["air"]), ("airp", ["airp"]), ("parrots", ["parrots"]),
]


def main():
    import say_api as S
    S.time = types.SimpleNamespace(strftime=lambda fmt: "Friday")
    classified = []
    for t in PHRASES:
        kind, rule = S.classify(t)
        classified.append({"text": t, "kind": kind, "rule": rule, "command": S.parse_command(t)})
    # parse_command on its own: what classify feeds it is lower-cased words, the route feeds it the text as said
    with tempfile.TemporaryDirectory(prefix="pk-say-golden-") as td:
        comedy, vec = G.write_workspace(Path(td))
        os.environ["PK_CACHE_DIR"] = str(Path(td) / "cache")
        from creative import data as D
        c = D.Corpus.load(comedy)
        c.vectors = D.Vectors(vec)
        bits = [{"query": q, "words": ws, "rows": S.match_bits(c, ws, q), "rows_cap_1": S.match_bits(c, ws, q, 1)} for q, ws in BITS_QUERIES]
    OUT.write_text(json.dumps({"phrases": classified, "bits": bits, "weekday": "Friday"}, indent=1, ensure_ascii=False) + "\n", encoding="utf-8", newline=chr(10))
    print(f"wrote {OUT} ({len(classified)} phrases, {len(bits)} bit queries)")


if __name__ == "__main__":
    main()
