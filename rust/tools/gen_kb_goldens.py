"""Goldens for pk-kb: a synthetic graph and a decision sequence that exercise every branch of the Python replay
(tools/explorer/kb.py View._apply, _resolve_merges, _index), the read models (brief, edge_view, joke_summaries,
search, joke_detail, label_index) and the decision builder (_build, undo). The Rust tests replay the same fixture and
compare as parsed JSON. Synthetic data only: nothing of the comic's is here.

    python rust/tools/gen_kb_goldens.py        (from the repo root; writes rust/crates/pk-kb/tests/goldens/)
"""
from __future__ import annotations

import copy
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "explorer"))
import kb as K  # noqa: E402

OUT = ROOT / "rust" / "crates" / "pk-kb" / "tests" / "goldens"
NOW = "2026-10-29T12:00:00+00:00"
K.now_iso = lambda: NOW  # the builder stamps dates and hashes with "now": pinned so both sides agree

GRAPH = {
    "version": "v0-test", "generated_at": "2026-10-01", "runs": ["r1"],
    "nodes": [
        {"id": "joke:meters", "type": "joke", "title": "Parking meters", "stage": "working", "summary": "AI summary", "first_seen": "2024-01-01", "last_seen": "2025-01-02"},
        {"id": "unit:m1", "type": "unit", "text": "The meter asked for my pronouns before my card", "date": "2025-01-02", "kind": "bit", "source": "gdoc", "doc_title": "Notes 2025"},
        {"id": "unit:m0", "type": "unit", "text": "Meters, first draft", "date": "2024-01-01", "kind": "bit"},
        {"id": "joke:talks", "type": "joke", "title": "Meter that talks", "stage": "idea", "summary": "The meter talks back"},
        {"id": "joke:cat", "type": "joke", "title": "Dating my cat", "stage": "testing", "summary": "An app for dating my cat", "last_seen": "2025-06-01"},
        {"id": "unit:c1", "type": "unit", "text": "Dating apps are a job interview", "date": "2025-06-01"},
        {"id": "label:topic:money", "type": "label", "kind": "topic", "name": "Money", "definition": "coins, bills, meters"},
        {"id": "label:topic:cash", "type": "label", "kind": "topic", "name": "Cash", "definition": "", "merged_into": "label:topic:money"},
        {"id": "label:device:callback", "type": "label", "kind": "device", "name": "Callback"},
        {"id": "set:s1", "type": "set", "title": "Tuesday", "venue": "The Cellar", "date": "2025-03-04"},
        {"id": "perf:p1", "type": "performance", "set": "set:s1", "start_ms": 60000, "end_ms": 90000, "text": "so the parking meter wants my pronouns now", "kind": "bit"},
        {"id": "perf:p2", "type": "performance", "set": "set:s1", "start_ms": 120000, "end_ms": 150000, "text": "my cat swiped left"},
        {"id": "list:tonight", "type": "list", "title": "Tonight"},
        {"id": "", "type": "joke", "title": "no id: dropped"},
        "not a dict",
    ],
    "edges": [
        {"id": "e:aaaaaaaaaaa1", "kind": "part_of", "src": "unit:m1", "dst": "joke:meters", "status": "confirmed", "confidence": 0.9},
        {"id": "e:aaaaaaaaaaa2", "kind": "part_of", "src": "joke:meters", "dst": "unit:m0", "status": "proposed", "confidence": 0.6},
        {"id": "e:aaaaaaaaaaa3", "kind": "part_of", "src": "unit:c1", "dst": "joke:cat", "status": "rejected"},
        {"id": "e:aaaaaaaaaaa4", "kind": "performed_as", "src": "perf:p1", "dst": "joke:meters", "status": "confirmed", "confidence": 0.95},
        {"id": "e:aaaaaaaaaaa5", "kind": "has_label", "src": "joke:meters", "dst": "label:topic:money", "status": "proposed", "confidence": 0.7},
        {"id": "e:aaaaaaaaaaa6", "kind": "has_label", "src": "joke:meters", "dst": "label:topic:cash", "status": "confirmed", "confidence": 0.8},
        {"id": "e:aaaaaaaaaaa7", "kind": "has_label", "src": "unit:c1", "dst": "label:device:callback", "status": "proposed"},
        {"id": "e:aaaaaaaaaaa8", "kind": "same_premise", "src": "joke:meters", "dst": "joke:talks", "status": "proposed", "confidence": 0.55},
        {"id": "e:aaaaaaaaaaa9", "kind": "custom", "custom_kind": "runner", "src": "joke:cat", "dst": "joke:talks", "status": "proposed"},
        {"id": "e:aaaaaaaaab10", "kind": "has_label", "src": "joke:cat", "dst": "label:topic:cash", "status": "proposed", "confidence": 0.3},
        {"id": "e:aaaaaaaaab11", "kind": "has_label", "src": "joke:cat", "dst": "label:topic:money", "status": "confirmed", "confidence": 0.9},
        {"id": "", "kind": "segue", "src": "joke:cat", "dst": "joke:talks"},
    ],
}

DECISIONS = [
    # 0 add a joke (idea) with his words
    {"ts": "2026-10-01T10:00:00+00:00", "target": "node", "id": "joke:raccoons", "action": "add", "before": None,
     "after": {"type": "joke", "title": "Raccoons", "stage": "idea", "summary": "A raccoon with a tight five", "first_seen": "2026-10-01", "last_seen": "2026-10-01"}, "reason": "said on the input bar", "by": "owner"},
    # 1 confirm an edge
    {"ts": "2026-10-01T10:01:00+00:00", "target": "edge", "id": "e:aaaaaaaaaaa5", "action": "confirm", "before": {"status": "proposed"}, "after": {"status": "confirmed"}, "reason": "", "by": "owner"},
    # 2 reject an edge
    {"ts": "2026-10-01T10:02:00+00:00", "target": "edge", "id": "e:aaaaaaaaaaa8", "action": "reject", "before": {"status": "proposed"}, "after": {"status": "rejected"}, "reason": "not the same", "by": "owner"},
    # 3 edit an edge (moves it): marks _edited
    {"ts": "2026-10-01T10:03:00+00:00", "target": "edge", "id": "e:aaaaaaaaaaa7", "action": "edit", "before": {"src": "unit:c1"}, "after": {"src": "joke:cat", "status": "confirmed"}, "reason": "", "by": "owner"},
    # 4 edit a node
    {"ts": "2026-10-01T10:04:00+00:00", "target": "node", "id": "joke:talks", "action": "edit", "before": {"stage": "idea"}, "after": {"stage": "to_try"}, "reason": "", "by": "owner"},
    # 5 add an edge with a legacy import reason (actor_of corrects it)
    {"ts": "2026-10-01T10:05:00+00:00", "target": "edge", "id": "e:bbbbbbbbbbb1", "action": "add", "before": None,
     "after": {"src": "unit:m1", "dst": "label:device:callback", "kind": "has_label", "confidence": 1.0}, "reason": "[gdocs] tagged in the doc", "by": "owner"},
    # 6 confirm an edge the pipeline dropped: resurrected from before
    {"ts": "2026-10-01T10:06:00+00:00", "target": "edge", "id": "e:ccccccccccc1", "action": "confirm",
     "before": {"src": "joke:cat", "dst": "joke:raccoons", "kind": "callback_to", "status": "proposed", "confidence": 0.4}, "after": {"status": "confirmed"}, "reason": "", "by": "owner"},
    # 7 edit a node the pipeline dropped: resurrected from before
    {"ts": "2026-10-01T10:07:00+00:00", "target": "node", "id": "joke:ghost", "action": "edit",
     "before": {"type": "joke", "title": "Ghost bit", "stage": "idea"}, "after": {"stage": "retired"}, "reason": "", "by": "owner"},
    # 8 an orphan: nothing to resurrect from
    {"ts": "2026-10-01T10:08:00+00:00", "target": "edge", "id": "e:ddddddddddd1", "action": "confirm", "before": {}, "after": {"status": "confirmed"}, "reason": "", "by": "owner"},
    # 9 delete a node he added
    {"ts": "2026-10-01T10:09:00+00:00", "target": "node", "id": "joke:ghost", "action": "delete", "before": {"type": "joke"}, "after": None, "reason": "", "by": "owner"},
    # 10 an undo of the edit at 3: clears _edited
    {"ts": "2026-10-01T10:10:00+00:00", "target": "edge", "id": "e:aaaaaaaaaaa7", "action": "edit", "before": {"src": "joke:cat"}, "after": {"src": "unit:c1", "status": None}, "reason": "undo", "by": "owner", "undo_of": 3},
    # 11 a line with an unknown target: ignored
    {"ts": "2026-10-01T10:11:00+00:00", "target": "blob", "id": "x", "action": "add", "after": {}, "reason": "", "by": "owner"},
    # 12 a line with no id: ignored
    {"ts": "2026-10-01T10:12:00+00:00", "target": "node", "action": "add", "after": {"type": "joke"}, "reason": "", "by": "owner"},
    # 13 add a unit + 14 part_of edge (the workbench's version)
    {"ts": "2026-10-01T10:13:00+00:00", "target": "node", "id": "unit:v1", "action": "add", "before": None,
     "after": {"type": "unit", "text": "Raccoons unionised", "kind": "tag", "source": "app", "date": "2026-10-01"}, "reason": "written in the app", "by": "owner"},
    {"ts": "2026-10-01T10:14:00+00:00", "target": "edge", "id": "e:eeeeeeeeeee1", "action": "add", "before": None,
     "after": {"src": "unit:v1", "dst": "joke:raccoons", "kind": "part_of", "status": "confirmed", "confidence": 1.0}, "reason": "written in the app", "by": "owner"},
    # 15 an ai actor's attach-like edit on a joke
    {"ts": "2026-10-01T10:15:00+00:00", "target": "node", "id": "joke:talks", "action": "edit", "before": {"hidden": None}, "after": {"hidden": True}, "reason": "[merge] mg:x", "by": "owner"},
    # 16 an import actor named outright
    {"ts": "2026-10-01T10:16:00+00:00", "target": "node", "id": "joke:cat", "action": "edit", "before": {"short_name": None}, "after": {"short_name": "Cat app"}, "reason": "", "by": "import:gsync"},
]

BUILD_REQUESTS = [
    # each: a request to _build against the replayed view; expected = the built decision, or {"error": msg, "status": n}
    {"target": "edge", "action": "add", "after": {"src": "joke:meters", "dst": "joke:cat", "kind": "callback_to"}, "reason": "they chain"},
    {"target": "edge", "action": "add", "after": {"src": "joke:meters", "dst": "joke:cat", "kind": "callback_to"}},
    {"target": "edge", "action": "add", "after": {"src": "joke:meters", "dst": "joke:meters", "kind": "segue"}, "reason": "x"},
    {"target": "edge", "action": "add", "after": {"src": "joke:meters", "dst": "joke:cat", "kind": "has_label"}, "reason": "x"},
    {"target": "edge", "action": "add", "after": {"src": "joke:meters", "dst": "joke:cat", "kind": "custom", "custom_kind": "  runner "}, "reason": "x"},
    {"target": "edge", "action": "add", "after": {"src": "joke:meters", "dst": "joke:cat", "kind": "custom"}, "reason": "x"},
    {"target": "edge", "action": "add", "after": {"src": "unit:m1", "dst": "joke:meters", "kind": "part_of"}, "reason": "dup"},
    {"target": "edge", "action": "add", "after": {"src": "joke:nope", "dst": "joke:cat", "kind": "segue"}, "reason": "x"},
    {"target": "edge", "action": "add", "after": {"src": "label:topic:cash", "dst": "joke:cat", "kind": "segue"}, "reason": "aliased label is unknown"},
    {"target": "edge", "action": "confirm", "id": "e:aaaaaaaaab10"},
    {"target": "edge", "action": "reject", "id": "e:aaaaaaaaab10"},
    {"target": "edge", "action": "delete", "id": "e:aaaaaaaaab10"},
    {"target": "edge", "action": "delete", "id": "e:bbbbbbbbbbb1"},
    {"target": "edge", "action": "edit", "id": "e:aaaaaaaaab10", "after": {"dst": "label:device:callback"}},
    {"target": "edge", "action": "edit", "id": "e:aaaaaaaaab10", "after": {}},
    {"target": "edge", "action": "edit", "id": "e:aaaaaaaaab10", "after": {}, "reason": "just a note"},
    {"target": "edge", "action": "edit", "id": "e:aaaaaaaaab10", "after": {"kind": "segue"}},
    {"target": "edge", "action": "edit", "id": "e:nope", "after": {}},
    {"target": "node", "action": "add", "after": {"type": "label", "name": "  Timing ", "kind": "device", "definition": " beats "}},
    {"target": "node", "action": "add", "after": {"type": "label", "name": "Money", "kind": "topic"}},
    {"target": "node", "action": "add", "after": {"type": "label", "name": "", "kind": "topic"}},
    {"target": "node", "action": "add", "after": {"type": "label", "name": "X", "kind": "colour"}},
    {"target": "node", "action": "add", "after": {"type": "joke", "title": " Parking meters ", "summary": " a second one "}},
    {"target": "node", "action": "add", "after": {"type": "joke", "title": "", "summary": "x"}},
    {"target": "node", "action": "add", "after": {"type": "joke", "title": "Stage check", "stage": "flying"}},
    {"target": "node", "action": "add", "after": {"type": "unit", "text": "  A new line  ", "source": "app"}},
    {"target": "node", "action": "add", "after": {"type": "unit", "text": "ok", "id": "unit:draft-abc", "date": "2024-02-03", "doc_title": " Notes "}},
    {"target": "node", "action": "add", "after": {"type": "unit", "text": "ok", "id": "unit:m1"}},
    {"target": "node", "action": "add", "after": {"type": "unit", "text": "   "}},
    {"target": "node", "action": "add", "after": {"type": "set", "title": "x"}},
    {"target": "node", "action": "delete", "id": "joke:raccoons"},
    {"target": "node", "action": "delete", "id": "joke:meters"},
    {"target": "node", "action": "confirm", "id": "joke:meters"},
    {"target": "node", "action": "edit", "id": "joke:meters", "after": {"stage": "retired", "title": " Meters ", "short_name": None}},
    {"target": "node", "action": "edit", "id": "joke:meters", "after": {"stage": "flying"}},
    {"target": "node", "action": "edit", "id": "joke:meters", "after": {"title": "  "}},
    {"target": "node", "action": "edit", "id": "joke:meters", "after": {"text": "no"}},
    {"target": "node", "action": "edit", "id": "joke:meters", "after": {"stage": "working"}},
    {"target": "node", "action": "edit", "id": "joke:meters", "after": {"short_name": "a name that is far too long for a list"}},
    {"target": "node", "action": "edit", "id": "joke:meters", "after": {"hidden": "yes"}},
    {"target": "node", "action": "edit", "id": "set:s1", "after": {"show_date": "2025-02-30"}},
    {"target": "node", "action": "edit", "id": "set:s1", "after": {"show_date": "2025-02-28", "venue": " The Cellar ", "audio_unreliable": True}},
    {"target": "node", "action": "edit", "id": "set:s1", "after": {"venue": ""}},
    {"target": "node", "action": "edit", "id": "perf:p1", "after": {"kind": "crowd_work"}},
    {"target": "node", "action": "edit", "id": "perf:p1", "after": {"kind": "song"}},
    {"target": "node", "action": "edit", "id": "label:topic:money", "after": {"merged_into": "label:device:callback"}},
    {"target": "node", "action": "edit", "id": "label:topic:money", "after": {"merged_into": "label:topic:money"}},
    {"target": "node", "action": "edit", "id": "label:topic:money", "after": {"merged_into": "joke:cat"}},
    {"target": "node", "action": "edit", "id": "joke:nope", "after": {"stage": "idea"}},
    {"target": "thing", "action": "edit", "id": "joke:meters", "after": {}},
    {"target": "node", "action": "edit", "id": "joke:meters", "after": "nope"},
    {"target": "node", "action": "edit", "id": "joke:meters", "after": {"stage": "idea"}, "reason": "x" * 4001},
]


def view() -> K.View:
    return K.View(copy.deepcopy(GRAPH), copy.deepcopy(DECISIONS), ["e:aaaaaaaaab10", "e:aaaaaaaaaaa5"], lambda *a: None)


def dump_view(v: K.View) -> dict:
    return {
        "graph_meta": v.graph_meta,
        "nodes": v.nodes, "edges": v.edges, "orphans": v.orphans, "undone": sorted(v.undone), "alias": v.alias,
        "adj": v.adj, "jokes": [j["id"] for j in v.jokes], "labels": [l["id"] for l in v.labels],
        "unit_joke": v.unit_joke, "joke_units": {k: [list(x) for x in xs] for k, xs in v.joke_units.items()}, "degree": v.degree,
        "briefs": {nid: v.brief(nid) for nid in list(v.nodes) + ["missing:x"]},
        "edge_views": {eid: v.edge_view(e) for eid, e in v.edges.items()},
        "edge_views_focus": {eid: v.edge_view(e, e.get("src")) for eid, e in v.edges.items()},
        "summaries": v.joke_summaries(),
        "search": {q: v.search(q) for q in ("", "meter", "METER pronouns", "cat job", "money", "runner", "nothing-here", "joke:cat")},
        "details": {j["id"]: v.joke_detail(j["id"]) for j in v.jokes},
        "detail_missing": v.joke_detail("unit:m1"),
        "label_index": v.label_index(),
        "actors": [K.actor_of(d) for d in v.decisions],
    }


def dump_builds(v: K.View) -> list:
    kb = K.KB.__new__(K.KB)
    out = []
    for req in BUILD_REQUESTS:
        try:
            out.append({"request": req, "built": kb._build(v, req)})
        except K.DecisionError as e:
            out.append({"request": req, "error": str(e), "status": e.status})
    return out


def dump_undo(v: K.View) -> list:
    """The inverse line undo() would append for each decision (without appending), and the refusals."""
    out = []
    for i in range(len(v.decisions) + 1):
        try:
            out.append({"index": i, "line": undo_line(v, i)})
        except K.DecisionError as e:
            out.append({"index": i, "error": str(e), "status": e.status})
    return out


def undo_line(v: K.View, index: int) -> dict:
    ds = v.decisions
    if not (0 <= index < len(ds)):
        raise K.DecisionError("no such decision", 404)
    d = ds[index]
    if index in v.undone:
        raise K.DecisionError("already undone", 409)
    if "undo_of" in d:
        raise K.DecisionError("that line is itself an undo", 409)
    t, did, a = d.get("target"), d.get("id"), d.get("action")
    store = v.edges if t == "edge" else v.nodes
    cur = store.get(did)
    before = d.get("before") if isinstance(d.get("before"), dict) else {}
    after = d.get("after") if isinstance(d.get("after"), dict) else {}
    if a == "add":
        inv = {"action": "delete", "before": K.public(cur) if cur else after, "after": None}
    elif a == "delete":
        inv = {"action": "add", "before": None, "after": before}
    else:
        inv = {"action": "edit", "before": K.public(cur) if cur else None, "after": {k: before.get(k) for k in after}}
    reason = "undo" + (f": {d.get('reason')}" if d.get("reason") else "")
    out = {"ts": NOW, "target": t, "id": did, **inv, "reason": reason, "by": "owner", "undo_of": index}
    return {k: out[k] for k in ("ts", "target", "id", "action", "before", "after", "reason", "by", "undo_of")}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    v = view()
    (OUT / "fixture.json").write_text(json.dumps({"graph": GRAPH, "decisions": DECISIONS, "report": ["e:aaaaaaaaab10", "e:aaaaaaaaaaa5"], "now": NOW}, ensure_ascii=False, indent=1), encoding="utf-8")
    (OUT / "view.json").write_text(json.dumps(dump_view(v), ensure_ascii=False, indent=1), encoding="utf-8")
    (OUT / "builds.json").write_text(json.dumps(dump_builds(v), ensure_ascii=False, indent=1), encoding="utf-8")
    (OUT / "undo.json").write_text(json.dumps(dump_undo(v), ensure_ascii=False, indent=1), encoding="utf-8")
    (OUT / "helpers.json").write_text(json.dumps({
        "edge_ids": [[s, k, d, K.edge_id(s, k, d)] for s, k, d in (("joke:a", "segue", "joke:b"), ("unit:x", "part_of", "joke:y"), ("joke:a", "runner", "joke:b"))],
        "slugs": [[s, K.slugify(s)] for s in ("Parking meters", "  Hello, World!  ", "ÀÉÎ ünïcode", "", "zq" * 40, "--x--")],
        "buckets": [[s, K.bucket(s)] for s in ("confirmed", "edited", "rejected", "ruled_out", "superseded", "proposed", None, "weird")],
    }, ensure_ascii=False, indent=1), encoding="utf-8")
    print("wrote", OUT, "nodes", len(v.nodes), "edges", len(v.edges), "orphans", v.orphans, "builds", len(BUILD_REQUESTS))


if __name__ == "__main__":
    main()
