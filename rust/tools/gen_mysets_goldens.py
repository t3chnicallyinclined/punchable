"""Goldens for pk-creative's `mysets` writes helpers (creative/mysets.py `create`, `change_note`, `match_recording`) over
the synthetic notebook of notebook_fixture.py: the `promote` event a new set is made with (id and data), the words a
change of order, a swap, an addition or a cut is written down as, and the recording each set lines up with (the score,
his own pick at import, no match). The Rust test is crates/pk-creative/tests/mysets_goldens.rs.

    python rust/tools/gen_mysets_goldens.py        (from the repo root; writes crates/pk-creative/tests/goldens/mysets.json)
"""
from __future__ import annotations

import itertools
import json
import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
for p in (ROOT, ROOT / "api-dev", ROOT / "tools" / "explorer", Path(__file__).resolve().parent):
    sys.path.insert(0, str(p))
os.environ["PK_OWNER_ID"] = "owner"
import gen_corpus_goldens as G  # noqa: E402

OUT = ROOT / "rust" / "crates" / "pk-creative" / "tests" / "goldens" / "mysets.json"


class FakeStore:
    """What `mysets.create` needs of a Store: its item count, and an `append` that says what it was asked to write."""

    def __init__(self, n):
        self.items = {f"x{i}": {} for i in range(n)}
        self.last = None

    def append(self, op, id_, data, by=None, ts=None):
        self.last = {"op": op, "id": id_, "data": data}
        return self.last


def main():
    from creative import data as D
    from creative import mysets as M
    from creative import workingset as W
    with tempfile.TemporaryDirectory(prefix="pk-mysets-golden-") as td:
        comedy, vec = G.write_workspace(Path(td))
        os.environ["PK_CACHE_DIR"] = str(Path(td) / "cache")
        c = D.Corpus.load(comedy)
        c.vectors = D.Vectors(vec)
        jokes = list(c.jokes)
        keys = c.set_keys()
        # sets: one of each night's performed bits, one that shares nothing, one of a single bit, an empty one
        sets, items = [], {}
        plans = {}
        for k in keys:
            ids = M.performed_in(c, k)
            if ids:
                plans[f"night-{k}"] = ids
        plans["disjoint"] = [j for j in jokes if all(j not in M.performed_in(c, k) for k in keys)][:3] or jokes[-1:]
        plans["one"] = jokes[:1]
        plans["reversed"] = list(reversed(next(iter(plans.values()))))
        plans["empty"] = []
        for n, (name, ids) in enumerate(plans.items()):
            store = FakeStore(len(items))
            M.create(c, store, name, 600, ids, note=f"v1: {name}")
            ev = {"ts": f"2026-10-29T12:00:{n:02d}+00:00", "op": store.last["op"], "id": store.last["id"], "by": "owner", "data": store.last["data"]}
            W._apply(items, ev)
        sets = list(items.values())
        sets.append(dict(sets[0], id="ws:gone", deleted=True))
        matches = {k: M.match_recording(c, sets, k) for k in keys}
        # his own pick at import wins over the score
        pick = {}
        if keys:
            c.metas[keys[0]] = dict(c.metas.get(keys[0]) or {}, align_working_set=sets[-2]["id"])
            pick = {"key": keys[0], "id": sets[-2]["id"], "result": M.match_recording(c, sets, keys[0])}
        missing = M.match_recording(c, sets, "no-such-set")
        # notes: a swap in place, an addition, a cut, a move, a move of several, a length change, nothing
        a, b, d, e, f = (jokes + jokes)[:5]
        notes = []
        for old, new, ot, nt in [
            ([a, b, d], [a, e, d], 600, 600), ([a, b], [a, b, d], 600, 600), ([a, b, d], [a, d], 600, 600),
            ([a, b, d, e], [e, a, b, d], 600, 600), ([a, b, d, e], [d, e, a, b], 600, 600), ([a, b, d], [d, b, a], 300, 420),
            ([a, b], [a, b], 600, 600), ([a, b], [a, b], 450, 600), ([], [a], 300, 300), ([a, b, d], [], 600, 600),
            ([a, b, d], [b, a, e, f], 600, 900), ([a, "joke:missing"], ["joke:missing", a], 600, 600),
        ]:
            notes.append({"old": old, "new": new, "old_t": ot, "new_t": nt, "note": M.change_note(c, old, new, ot, nt)})
        creates = []
        for title, target, ids, note, sb, n in [
            ("Friday late show", 900, jokes[:3], "v1: a new set", None, 0), ("A title with  spaces & marks!", 300, jokes[:2], None, None, 4),
            ("From a board", 600, jokes[:2], "v1: made from the board 'X'", "sb:x-1", 2), ("Empty", 420, [], None, None, 1),
            ("x" * 60, 600, jokes[:1], "", None, 7),
        ]:
            store = FakeStore(n)
            M.create(c, store, title, target, ids, note=note, storyboard_id=sb)
            creates.append({"title": title, "target_s": target, "ids": ids, "note": note, "storyboard_id": sb, "n_items": n, **store.last})
    text = json.dumps({"sets": sets, "matches": matches, "pick": pick, "missing": missing, "notes": notes, "creates": creates, "keys": keys},
                      indent=1, ensure_ascii=False)
    OUT.write_text(text + chr(10), encoding="utf-8", newline=chr(10))
    print(f"wrote {OUT} ({len(sets)} sets, {len(matches)} matches, {len(notes)} notes, {len(creates)} creates)")


if __name__ == "__main__":
    main()
