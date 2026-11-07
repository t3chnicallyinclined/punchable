"""Goldens for pk-creative's `all_map` (creative/notebook.py, `GET /api/map`): the bird's-eye map over the synthetic
notebook of notebook_fixture.py, at several caps, with and without the merge judge's same-idea groups.
The Rust test is crates/pk-creative/tests/map_goldens.rs.

    python rust/tools/gen_map_goldens.py        (from the repo root; writes crates/pk-creative/tests/goldens/map.json)
"""
from __future__ import annotations

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

OUT = ROOT / "rust" / "crates" / "pk-creative" / "tests" / "goldens" / "map.json"


def main():
    from creative import data as D
    from creative import notebook as N
    with tempfile.TemporaryDirectory(prefix="pk-map-golden-") as td:
        comedy, vec = G.write_workspace(Path(td))
        os.environ["PK_CACHE_DIR"] = str(Path(td) / "cache")
        c = D.Corpus.load(comedy)
        c.vectors = D.Vectors(vec)
        jokes = list(c.jokes)
        groups = [{"members": [{"joke_id": jokes[0]}, {"joke_id": jokes[1]}, {"joke_id": jokes[2]}]},
                  {"members": [{"joke_id": jokes[-1]}, {"joke_id": jokes[0]}]}, {"members": []}]
        cases = []
        for cap in (1, 2, 3, 400):
            for g in (None, groups):
                cases.append({"cap": cap, "same_idea": g, "map": N.all_map(c, cap=cap, same_idea=g)})
    OUT.write_text(json.dumps({"cases": cases}, indent=1, ensure_ascii=False) + chr(10), encoding="utf-8", newline=chr(10))
    n = cases[-1]["map"]
    print(f"wrote {OUT} ({len(cases)} cases; the full map has {len(n['nodes'])} nodes, {len(n['edges'])} edges {n['counts']['by_kind']})")


if __name__ == "__main__":
    main()
