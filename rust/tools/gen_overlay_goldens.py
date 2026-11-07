"""Goldens for pk-kb's overlay: api-dev/ai_overlay.py `build_overlay` (and `remap_ai_ids` inside it) over synthetic
graphs, inferred.json and analysis.json documents that reach every branch: the inferred stages, a performance that is
a graph performance under another start time, edges retargeted through that alias, the analysis rows (matched by
overlap, new moments, known / new / crowd work / banter, punchlines), a set the graph does not have, and the id map.
The Rust test (crates/pk-kb/tests/overlay.rs) runs the same inputs and compares as parsed JSON. Synthetic only.

    python rust/tools/gen_overlay_goldens.py        (from the repo root)
"""
from __future__ import annotations

import copy
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "api-dev"))
import ai_overlay as AI  # noqa: E402

OUT = ROOT / "rust" / "crates" / "pk-kb" / "tests" / "goldens" / "overlay.json"


class Files:
    """The four things build_overlay asks its files object for."""

    def __init__(self, id_map=None, inferred=None, analyses=None, set_meta=None):
        self._id_map, self._inf, self._an, self._meta = id_map, inferred, analyses or {}, set_meta or {}

    def id_map(self):
        return self._id_map

    def inferred(self):
        if self._inf is None:
            return None, None, None, None
        return self._inf["doc"], self._inf["source"], self._inf["path"], None

    def analyses(self):
        return {d: (x["doc"], x["source"], x["path"], None) for d, x in self._an.items()}

    def set_meta(self, d):
        return self._meta.get(d, {})


def graph():
    return {
        "version": "v1-test", "generated_at": "2026-10-01", "runs": ["r1"],
        "nodes": [
            {"id": "joke:meters", "type": "joke", "title": "Parking meters", "stage": "testing", "summary": "s"},
            {"id": "joke:cat", "type": "joke", "title": "Dating my cat", "stage": "idea"},
            {"id": "joke:old", "type": "joke", "title": "Old", "stage": "idea"},
            {"id": "unit:m1", "type": "unit", "text": "meter text"},
            {"id": "set:2025-03-04_cellar", "type": "set", "title": "Tuesday", "venue": "The Cellar", "date": "2025-03-04"},
            {"id": "perf:p1", "type": "performance", "set": "set:2025-03-04_cellar", "start_ms": 60000, "end_ms": 90000, "text": "meter", "kind": "bit"},
            {"id": "perf:p2", "type": "performance", "set": "set:2025-03-04_cellar", "start_ms": 120000, "end_ms": 150000, "text": "cat", "kind": "crowd_work"},
            {"id": "perf:p3", "type": "performance", "set": "set:2025-03-04_cellar", "start_ms": 200000, "end_ms": 230000, "text": "third"},
            {"id": "perf:p4", "type": "performance", "set": "set:2025-03-04_cellar", "start_ms": 300000, "end_ms": 330000, "text": "fourth"},
            # a repeated id: the later record replaces the earlier and keeps the earlier one's place (a dict's way)
            {"id": "joke:old", "type": "joke", "title": "Old, again", "stage": "testing"},
        ],
        "edges": [
            {"id": "e:aaaaaaaaaaa1", "kind": "performed_as", "src": "perf:p1", "dst": "joke:meters", "status": "proposed", "confidence": 0.5},
            {"id": "e:aaaaaaaaaaa2", "kind": "performed_as", "src": "perf:p1", "dst": "joke:old", "status": "proposed"},
            {"id": "e:aaaaaaaaaaa3", "kind": "performed_as", "src": "perf:p2", "dst": "joke:cat", "status": "confirmed"},
            {"id": "e:aaaaaaaaaaa4", "kind": "performed_as", "src": "perf:p3", "dst": "joke:cat"},
            {"id": "e:aaaaaaaaaaa5", "kind": "part_of", "src": "unit:m1", "dst": "joke:meters", "status": "proposed"},
            {"id": "e:aaaaaaaaaaa6", "kind": "performed_as", "src": "perf:p4", "dst": "joke:old", "status": "proposed"},
            {"id": "e:aaaaaaaaaaa5", "kind": "part_of", "src": "unit:m1", "dst": "joke:meters", "status": "confirmed"},
        ],
    }


def inferred_doc():
    return {
        "run_id": "run-7", "generated_at": "2026-10-20", "notes": "synthetic",
        "nodes": [
            {"id": "perf:p1", "type": "performance", "ai_note": "kept", "start_ms": 1, "name": "Meters", "status": "x", "empty": None},
            {"id": "perf:new1", "type": "performance", "set": "set:2025-03-04_cellar", "start_ms": 61500, "end_ms": 91000, "text": "same moment, later start"},
            {"id": "perf:new2", "type": "performance", "set": "set:2025-03-04_cellar", "start_ms": 500000, "end_ms": 520000, "text": "a new one", "status": "proposed"},
            {"id": "joke:fresh", "type": "joke", "title": "Fresh", "status": "proposed"},
            {"id": "", "type": "joke"}, {"id": "x:nope"}, "junk",
        ],
        "jokes": [
            {"id": "joke:meters", "stage": "working", "stage_confidence": 0.8, "stage_reason": "it kills", "hit_sets": 3,
             "performances": [{"perf": "perf:p1", "ai_rating": "killed", "rating_basis": "laughs"},
                              {"perf": "perf:new1", "rating": "worked"}, {"perf": "perf:p2", "rating": "bogus"}, "junk", {"perf": None}]},
            {"joke_id": "joke:cat", "inferred_stage": "retired", "confidence": 0.4, "reason": "stale", "hit_sets": True},
            {"id": "joke:old", "stage": "nonsense", "hit_sets": 2.5},
            {"id": "unit:m1", "stage": "idea"}, {"id": "joke:missing", "stage": "idea"}, "junk",
        ],
        "edges": [
            {"id": "e:aaaaaaaaaaa1", "status": "inferred", "confidence": 0.9, "reason": "r", "evidence": [{"node": "perf:p1", "quote": "q"}], "source": "auto:x", "extra": 1},
            {"id": "e:bbbbbbbbbbb1", "kind": "performed_as", "src": "perf:new1", "dst": "joke:meters", "confidence": 0.7},
            {"id": "e:bbbbbbbbbbb2", "kind": "performed_as", "src": "perf:new1", "dst": "joke:fresh"},
            {"id": "e:bbbbbbbbbbb3", "kind": "custom", "custom_kind": "runner", "src": "joke:cat", "dst": "joke:fresh", "status": "proposed", "source": "own"},
            {"id": "e:bbbbbbbbbbb4", "kind": "part_of", "src": "joke:cat", "dst": "joke:nowhere"},
            {"id": "e:bbbbbbbbbbb5", "src": "joke:cat", "dst": "joke:fresh"},
            {"id": "", "kind": "part_of"}, "junk",
        ],
    }


def analysis_doc():
    return {
        "run_id": "an-1", "generated_at": "2026-10-21", "summary": {"duration_s": 3600, "best": "joke:meters", "weakest": "joke:old"},
        "jokes": [
            {"start_ms": 59000, "end_ms": 91000, "joke_id": "joke:meters", "status": "known", "match_confidence": 0.93, "match_reason": "same words",
             "ai_rating": "killed", "rating_basis": "big laugh", "punchlines": [{"text": "pronouns", "at_ms": 80000}, {"text": "two"}]},
            {"start_ms": 119000, "end_ms": 151000, "joke_id": "joke:cat", "status": "crowd_work", "ai_rating": "meh"},
            {"start_ms": 201000, "end_ms": 229000, "status": "new", "title": "Not in the graph", "transcript": "t", "ai_rating": "died"},
            {"start_ms": 300500, "joke_id": "joke:old", "match_confidence": 0},
            {"start_ms": 400000, "end_ms": 410000, "joke_id": "joke:cat", "status": "banter", "title": "chat", "transcript": "hello"},
            {"start_ms": 420000, "end_ms": 430000, "joke_id": "joke:meters", "title": "Meters again", "punchlines": [{"text": ""}, "x"]},
            {"start_ms": "440000", "end_ms": 450000, "joke_id": "unit:m1"},
            {"start_ms": 119500, "end_ms": 150500, "status": "known", "joke_id": "joke:cat"},
            {"end_ms": 5}, "junk",
        ],
    }


def second_analysis():
    return {"summary": {"duration_s": 1800}, "jokes": [{"start_ms": 1000, "end_ms": 9000, "title": "Opener", "transcript": "hi", "ai_rating": "worked"},
                                                    {"start_ms": 20000, "end_ms": 25000, "joke_id": "joke:meters", "status": "known"}]}


def id_map_doc():
    return {"jokes": {"joke:v0meters": "joke:meters", "joke:v0gone": "joke:vanished"},
            "units": {"unit:v0m1": "unit:m1"}, "edges": {"e:v0edge": "e:v1edge"}}


def inferred_v0():
    return {"run_id": "r0", "jokes": {"joke:v0meters": {"stage": "working", "performances": []}, "joke:zzz": {"stage": "idea"}},
            "edges": [{"id": "e:v0edge", "kind": "part_of", "src": "unit:v0m1", "dst": "joke:v0meters", "status": "inferred"},
                      {"id": "e:v0edge2", "kind": "part_of", "src": "unit:v0q", "dst": "joke:v0meters"}]}


def analysis_v0():
    return {"summary": {"best": "joke:v0meters", "weakest": "joke:v0gone"},
            "jokes": [{"start_ms": 60000, "end_ms": 90000, "joke_id": "joke:v0meters", "version_unit_id": "unit:v0m1",
                       "similar": [{"joke_id": "joke:v0meters"}, {"joke_id": "joke:v0gone"}, "x"], "status": "known"}]}


def doc(d, source="real", path="x"):
    return {"doc": d, "source": source, "path": path}


def case(name, g, files):
    merged, index = AI.build_overlay(copy.deepcopy(g), files)
    rows = {k: [[r, p] for r, p in v] for k, v in index["rows"].items()}
    out_index = {"rows": rows, "analysis": index["analysis"], "inferred": index["inferred"], "id_map": index.get("id_map")}
    return {
        "name": name, "graph": g,
        "files": {"id_map": files._id_map,
                  "inferred": files._inf,
                  "analyses": [[d, x] for d, x in files._an.items()],
                  "set_meta": files._meta},
        "merged": merged, "index": out_index,
    }


def main():
    g = graph()
    cases = [
        case("nothing", g, Files()),
        case("inferred", g, Files(inferred=doc(inferred_doc(), "real", "kb/v0/inferred.json"))),
        case("inferred_jokes_as_dict", g, Files(inferred=doc({"run_id": "d", "jokes": {"joke:meters": {"stage": "working"}, "joke:cat": "bad"}}))),
        case("analysis", g, Files(analyses={"20250304_cellar": doc(analysis_doc(), "real", "corpus/sets/20250304_cellar/analysis.json"),
                                            "20250510_newroom": doc(second_analysis(), "fixture", "f.json"),
                                            "20250101_nometa": doc({"jokes": []})},
                                  set_meta={"20250510_newroom": {"title": "New room", "venue_guess": "Basement", "duration": 2400}})),
        case("both", g, Files(inferred=doc(inferred_doc()), analyses={"20250304_cellar": doc(analysis_doc())})),
        case("id_map", g, Files(id_map=id_map_doc(), inferred=doc(inferred_v0()),
                                analyses={"20250304_cellar": doc(analysis_v0())})),
        case("empty_id_map", g, Files(id_map={}, inferred=doc({"run_id": "e"}))),
    ]
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(cases, indent=1, ensure_ascii=False, sort_keys=False) + "\n", encoding="utf-8", newline=chr(10))
    print(f"wrote {OUT} ({len(cases)} cases)")


if __name__ == "__main__":
    main()
