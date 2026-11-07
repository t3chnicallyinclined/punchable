"""Goldens for pk-creative's `corpus` and `notebook` (the Rust port of creative/data.py `Corpus`, creative/notebook.py
and the riff grounds of creative/write.py): the real `Corpus.load` over the synthetic notebook in notebook_fixture.py,
laid out as the hosted worker's workspace lays it out, and every structure the writing kinds read, dumped as JSON.
The Rust test builds its Corpus from the same files and compares as parsed values (ADR 0007).

    python rust/tools/gen_corpus_goldens.py        (from the repo root; writes crates/pk-creative/tests/goldens/corpus/)
"""
from __future__ import annotations

import pin_blas  # noqa: F401  (first: the BLAS kernel, so the numbers are the same on every machine)

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


TEXT_PAIRS = [
    ("the bin gets my belt and my shoes", "So the bin gets my belt, my shoes and most of my confidence."),
    ("a b c d e f", "a x c d y e f"),
    ("one two three", "three two one"),
    ("I bought 2 parrots", "i bought two parrots today"),
    ("", "anything"),
    ("He read me my rights", "He read me my rights when I touched his perch."),
    ("dog cat dog cat dog", "cat dog cat dog cat dog"),
]
TRANSCRIPTS = [
    "So airport security wants my belt in a bin. The bin gets my belt, my shoes and most of my confidence. Okay.",
    "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen, "
    "short one; Another clause that is long enough here!",
    "Same words here again now. Same words here again now!",
]
WORKING_SETS = {
    "none": {"id": None, "title": None, "current": 0, "locks": {}},
    "line_locked": {"locks": {"joke:parrot-lawyer": {"lines": [{"text": "Past results are no guarantee of future crackers."}]},
                              "joke:airport-bins": {"lines": [{"text": "most of my confidence"}]}}},
    "bit_locked": {"locks": {"joke:airport-bins": {"bit": True}}},
    "all_locked": {"locks": {"joke:gym-mirror": {"lines": [{"text": "At the gym I stare at the mirror the way my parrot does."}]}}},
}


class _Said:
    def __init__(self, data):
        self.data, self.provider, self.model, self.call_id = data, "anthropic", "claude-sonnet-5-5", "c0ffee000000"


def story_golden(c) -> dict:
    """transition, the bridge links / user turn / finish, the subject words, the cosines, for every ordered pair."""
    from creative import story as S
    from creative import voice as V
    V.clear()
    jokes = list(c.jokes)
    pairs = {}
    for a in jokes:
        for b in jokes:
            if a == b:
                continue
            seen = []

            def complete(task, msgs, schema):
                seen.append(msgs[-1]["content"])
                return _Said({"bridge": "  So anyway, the parrot.  ", "uses": ["L2", "L9"]})

            r = S.bridge_draft(c, a, b, complete=complete)
            sc, ws, near = S.same_subject(c, [a], b)
            pairs[f"{a}|{b}"] = {"transition": S.transition(c, a, b), "bridge": r, "user": seen[0] if seen else None,
                                 "direct": [e["id"] for e in S._direct_edges(c, a, b)],
                                 "personas": S.shared_labels(c, a, b, ("persona",), 0.4),
                                 "topics": S.shared_labels(c, a, b, ("topic",), 0.4),
                                 "same_subject": [sc, ws, near], "cos": c.cos(a, b)}
    return {"pairs": pairs, "edge_text": {j: [S._edge_text(c, j, True), S._edge_text(c, j, False)] for j in jokes},
            "cos_band": {j: list(S._cos_band(c, j)) for j in jokes}}


WORDS = ["airport", "parrot", "landlord", "laundry", "gym", "mirror", "carousel", "luggage", "rent", "socks",
         "scanner", "dryer", "receipt", "pigeon", "subway", "ticket"]


def rnd(seed: str):
    """384 deterministic pseudo-random floats in [-1, 1) from sha256 (no RNG state, the same on every machine)."""
    import hashlib
    import numpy as np
    out = []
    k = 0
    while len(out) < 384:
        h = hashlib.sha256(f"{seed}/{k}".encode()).digest()
        out += [(x - 128) / 128 for x in h]
        k += 1
    return np.asarray(out[:384], dtype=np.float32)


def big_corpus():
    """60 bits, one unit each with a vector: near-duplicate pairs share most of a vector and two rare words, so the
    vector band (it needs 50 other bits) and the rare-word score both decide links."""
    import numpy as np
    from creative import data as D
    nodes, edges, table = [], [], {}
    for n in range(60):
        jid, uid = f"joke:big-{n:02d}", f"unit:big-{n:02d}"
        w = [WORDS[n % 16], WORDS[(n * 7) % 16], WORDS[(n * 3 + 1) % 16]]
        if n % 10 in (0, 1):    # pairs 0/1, 10/11, ... share two rare words
            w = [f"zany{chr(97 + n // 10)}", f"quirk{chr(97 + n // 10)}"] + w[:1]
        nodes.append({"id": jid, "type": "joke", "title": f"Big bit {n} {w[0]}"})
        nodes.append({"id": uid, "type": "unit", "text": f"A bit about the {w[0]}, the {w[1]} and the {w[2]} again."})
        edges.append({"id": f"e:big-part-{n}", "kind": "part_of", "src": uid, "dst": jid, "confidence": 0.9})
        base = rnd(f"seed-{n // 2 if n < 20 else n}")
        v = base + (0.15 if n < 20 else 0.0) * rnd(f"noise-{n}")
        table[uid] = [float(x) for x in v.astype(np.float32)]
    graph = {"nodes": nodes, "edges": edges}
    return graph, table, D.Corpus(graph, vectors=D.Vectors(table=table))


def big_golden() -> dict:
    from creative import story as S
    graph, table, c = big_corpus()
    jokes = list(c.jokes)
    probe = jokes[:4] + jokes[10:12] + jokes[30:32]
    return {"graph": graph, "vectors": table,
            "cos_band": {j: list(S._cos_band(c, j)) for j in probe},
            "same_subject": {f"{a}|{b}": list(S.same_subject(c, [a], b)) for a in probe for b in jokes if a != b},
            "transition": {f"{a}|{b}": S.transition(c, a, b) for a in probe for b in probe if a != b}}


def punch_golden(c) -> dict:
    from creative import textdiff as T
    from creative import voice as V
    from creative import workingset as WS
    V.clear()
    out = {"pairs": [{"a": a, "b": b, "toks_a": T.toks(a), "coverage": T.coverage(a, b), "present": T.present(a, b),
                      "same_line": WS._same_line(a, b)} for a, b in TEXT_PAIRS],
           "phrases": {t: T.phrases(t) for t in TRANSCRIPTS}, "jokes": {}}
    for j in list(c.jokes) + ["joke:missing"]:
        p, lines = WS.latest_lines(c, j)
        per = {"latest": [list(x) for x in lines], "based_on": p["set"] if p else None,
               "marks": WS.laugh_marks(c, p, lines) if p else None, "ws": {}}
        for name, ws in WORKING_SETS.items():
            msgs, info = WS.punch_messages(c, ws, j)
            per["ws"][name] = {"user": msgs[-1]["content"] if msgs else None,
                               "lines": [list(x) for x in info["lines"]], "blocked": info["blocked"],
                               "locked_line": [WS.is_locked_line(ws, j, ph) for ph, _ in lines],
                               "bit_locked": WS.bit_locked(ws, j)}
        out["jokes"][j] = per
    out["working_sets"] = WORKING_SETS
    return out


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
            "punch": punch_golden(c),
            "story": story_golden(c),
            "big": big_golden(),
        }
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "notebook.json").write_text(json.dumps(jsonable(out), indent=1, ensure_ascii=False) + "\n", encoding="utf-8",
                                       newline="\n")
    print(f"{len(jokes)} jokes, {len(c.edges)} edges, {len(c.sets)} sets -> {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
