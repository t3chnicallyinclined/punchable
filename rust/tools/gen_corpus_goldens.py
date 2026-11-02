"""Goldens for pk-creative's `corpus` and `notebook` (the Rust port of creative/data.py `Corpus`, creative/notebook.py
and the riff grounds of creative/write.py): the real `Corpus.load` over the synthetic notebook in notebook_fixture.py,
laid out as the hosted worker's workspace lays it out, and every structure the writing kinds read, dumped as JSON.
The Rust test builds its Corpus from the same files and compares as parsed values (ADR 0007).

    python rust/tools/gen_corpus_goldens.py        (from the repo root; writes crates/pk-creative/tests/goldens/corpus/)
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))
import notebook_fixture as F  # noqa: E402

OUT = ROOT / "rust" / "crates" / "pk-creative" / "tests" / "goldens" / "corpus"
QUERIES = [
    "airport security took my belt",
    "parrot\nhe says disclaimers\nlegal stuff",
    "landlord rent",
    "socks spinning in the dryer",
    "nothing matches zebra quantum",
    "the the and",
    "bin bins binning",
]
IDEAS = [("Airport bins again", ["the bin gets everything", "  ", "my belt"]), ("parrot lawyer", []),
         ("zebra quantum", [])]


def write_workspace(tmp: Path) -> tuple[Path, Path]:
    import numpy as np
    comedy, vec = tmp / "comedy", tmp / "cache" / "kb"
    for rel, text in F.files().items():
        if rel.startswith("vectors/"):
            continue
        p = comedy / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8", newline="\n")
    lines = "".join(json.dumps(d, ensure_ascii=False, separators=(",", ":")) + "\n" for d in F.decisions())
    (comedy / "kb" / "v0").mkdir(parents=True, exist_ok=True)
    (comedy / "kb" / "v0" / "decisions.jsonl").write_text(lines, encoding="utf-8", newline="\n")
    vec.mkdir(parents=True, exist_ok=True)
    part = json.loads(F.files()["vectors/vectors.part000.json"])
    np.asarray(part["rows"], dtype=np.float32).tofile(vec / "vectors.f32")
    (vec / "vectors_ids.json").write_text(json.dumps(part["ids"]), encoding="utf-8")
    (vec / "extra_summary.json").write_text(F.files()["vectors/extra_summary.json"], encoding="utf-8")
    return comedy, vec


def jsonable(x):
    if isinstance(x, dict):
        return {str(k) if not isinstance(k, tuple) else "|".join(k): jsonable(v) for k, v in x.items()}
    if isinstance(x, (list, tuple)):
        return [jsonable(v) for v in x]
    if isinstance(x, set):
        return sorted(jsonable(v) for v in x)
    return x


def main():
    os.environ.pop("PK_OWNER_ID", None)
    with tempfile.TemporaryDirectory(prefix="pk-corpus-golden-") as td:
        comedy, vec = write_workspace(Path(td))
        os.environ["PK_CACHE_DIR"] = str(Path(td) / "cache")
        from creative import data as D
        from creative import notebook as N
        from creative import write as W
        c = D.Corpus.load(comedy)
        c.vectors = D.Vectors(vec)
        jokes = list(c.jokes)
        per_joke = {}
        for j in jokes + ["joke:missing"]:
            per_joke[j] = {
                "lead_unit": c.lead_unit(j), "title": c.title(j), "stage": list(c.stage(j)),
                "labels_of": {l: [w, e["id"]] for l, (w, e) in c.labels_of(j).items()},
                "devices": c.devices(j), "duration": list(c.duration(j)), "energy": list(c.energy(j)),
                "texts": [list(t) for t in N._texts(c, j)], "stage_words": list(N.stage_words(c, j)),
                "best_line": N.best_line(c, j), "best_line_terms": N.best_line(c, j, {"belt", "parrot", "rent"}),
                "last_night": W.last_night_words(c, j),
            }
        out = {
            "inputs": {"files": F.files(), "decisions": F.decisions(), "queries": QUERIES,
                       "ideas": [{"idea": i, "lines": ls} for i, ls in IDEAS]},
            "jokes": jokes, "labels": list(c.labels), "edges": [e["id"] for e in c.edges],
            "edge_bodies": c.edges,
            "his_nodes": sorted(c.his_nodes), "his_stage": sorted(c.his_stage), "perf_overrides": c.perf_overrides,
            "his_lists": c.his_lists, "unit_joke": c.unit_joke, "joke_units": dict(c.joke_units),
            "joke_labels": {j: {l: [w, e["id"]] for l, (w, e) in ls.items()} for j, ls in c.joke_labels.items()},
            "label_df": c.label_df, "idf": c.idf, "inferred_jokes": sorted(c.inferred_jokes),
            "sets": c.sets, "perfs": dict(c.perfs), "order": {k: [[j, s["seg_id"]] for j, s in v] for k, v in c.order.items()},
            "back_to_back": jsonable(dict(c.back_to_back)), "set_keys": c.set_keys(),
            "per_joke": per_joke,
            "text_of": {n: c.text_of(n) for n in ["unit:bins-a", "label:travel", "joke:airport-bins",
                                                  "seg:2024-03-01_FakeSet0001#0", "seg:2024-03-01_FakeSet0001#9",
                                                  "seg:nope#1", "unit:none"]},
            "text_of_field": c.text_of("joke:airport-bins", "anatomy.reveal"),
            "edge_evidence": {e["id"]: c.edge_evidence(e, 2) for e in c.edges},
            "seg_evidence": c.seg_evidence("seg:2024-03-01_FakeSet0001#2"),
            "lexical": {q: [[j, s] for j, s in N.lexical(c, q)] for q in QUERIES},
            "related": {q: [{k: it.get(k) for k in ("joke_id", "title", "quote", "quote_node", "stage_line", "proven",
                                                     "nights", "rank", "why")}
                            for it in N.related(c, q, cap=6)["items"]] for q in QUERIES},
            "riff_grounds": [{"idea": i, "lines": ls, "grounds": W.riff_grounds(c, i, [x.strip() for x in ls if x.strip()])}
                             for i, ls in IDEAS],
            "venue_short": {v: D.venue_short(v, d) for v, d in [
                ("The Corner Room Comedy Club, Springfield", "2024-03-01"), ("maybe The Loft", None),
                ("Unknown (title 'x')", "2023"), ("", None), ("Comix at Riverside", "2019-01-01"),
                ("Mabel's (upstairs)", None), ("Ab Comedy", None), ("Joe's Poets Cafe", "2020")]},
            "month_year": {d: N.month_year(d) for d in ["2015-06-26", "2015", "2015-13-01", "", "20xx-ab"]},
        }
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "notebook.json").write_text(json.dumps(jsonable(out), indent=1, ensure_ascii=False) + "\n", encoding="utf-8",
                                       newline="\n")
    print(f"{len(jokes)} jokes, {len(c.edges)} edges, {len(c.sets)} sets -> {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
