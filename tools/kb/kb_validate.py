"""Validate a KB v0 graph (KB_SCHEMA.md contract).

Checks:
- every evidence quote is a verbatim substring of its node's text (unit text or
  performance transcript span), and evidence nodes exist;
- no dangling edge endpoints;
- edge IDs are e:<sha1(src|kind|dst)[:12]> and unique; node IDs unique;
- enums (node type, joke stage, edge kind, status, label kind, unit kind) and
  confidence in [0, 1];
- node ids are deterministic in form (unit:<unit_id>, joke:<slug>, label:<kind>:<slug>,
  perf:<set>@<ms>, set:<date>_<id>); hidden units may have empty text but can't be evidence;
- with --decisions <path>, every id referenced in decisions.jsonl must resolve.
Status "inferred" (AI-accepted, KB_SCHEMA.md AI-first rule) is valid.
With drop=True, failing edges are removed (and counted) instead of raising.

Usage: py kb_validate.py [path/to/graph.json] [--decisions path/to/decisions.jsonl]
"""
import collections
import json
import re
import os
import sys

from kblib import OUT_DIR, edge_id

NODE_TYPES = {"joke", "unit", "set", "performance", "label"}
STAGES = {"idea", "to_try", "testing", "working", "retired"}
EDGE_KINDS = {"part_of", "variant_of", "performed_as", "tag_of", "same_shape", "callback_to", "same_premise", "has_label", "segue", "custom"}
LABEL_KINDS = {"device", "structure", "form", "attitude", "theory", "topic", "persona", "custom"}
UNIT_KINDS = {"setup+punch", "premise", "tag", "topper", "callback", "sketch", "note", "set_list"}
STATUSES = {"proposed", "inferred", "confirmed", "ruled_out", "superseded"}
ID_FORMS = {
    "unit": re.compile(r"^unit:[A-Za-z0-9_\-]+-\d{2}$"),
    "joke": re.compile(r"^joke:[a-z0-9]+(-[a-z0-9]+)*$"),
    "label": re.compile(r"^label:(device|structure|form|attitude|theory|topic|persona|custom):[a-z0-9_\-]+$"),
    "performance": re.compile(r"^perf:\d{4}-\d{2}-\d{2}_[A-Za-z0-9_\-]+@\d+$"),
    "set": re.compile(r"^set:\d{4}-\d{2}-\d{2}_[A-Za-z0-9_\-]+$"),
}


def validate(graph, drop=False):
    nodes = {}
    problems = collections.Counter()
    examples = collections.defaultdict(list)

    def bad(kind, msg):
        problems[kind] += 1
        if len(examples[kind]) < 5:
            examples[kind].append(msg)

    set_ids = {x["id"] for x in graph["nodes"] if x["type"] == "set"}
    for n in graph["nodes"]:
        form = ID_FORMS.get(n.get("type"))
        if form and not form.match(n["id"]):
            bad("node_id_form", n["id"])
        if n["type"] == "unit" and not n.get("hidden") and not n.get("text"):
            bad("unit_no_text", n["id"])
        if n["id"] in nodes:
            bad("dup_node_id", n["id"])
        nodes[n["id"]] = n
        if n["type"] not in NODE_TYPES:
            bad("node_type", n["id"])
        if n["type"] == "joke" and n.get("stage") not in STAGES:
            bad("joke_stage", n["id"])
        if n["type"] == "label" and n.get("kind") not in LABEL_KINDS:
            bad("label_kind", n["id"])
        if n["type"] == "unit" and n.get("kind") not in UNIT_KINDS:
            bad("unit_kind", n["id"])
        if n["type"] == "performance" and n.get("set") not in set_ids:
            bad("perf_set_missing", n["id"])

    keep, seen = [], set()
    dropped = collections.Counter()
    for e in graph["edges"]:
        errs = []
        if e["src"] not in nodes:
            errs.append("dangling_src")
        if e["dst"] not in nodes:
            errs.append("dangling_dst")
        if e["id"] != edge_id(e["src"], e["kind"], e["dst"]):
            errs.append("edge_id_not_deterministic")
        if e["id"] in seen:
            errs.append("dup_edge_id")
        if e["kind"] not in EDGE_KINDS:
            errs.append("edge_kind")
        if e.get("status") not in STATUSES:
            errs.append("status")
        if not (0.0 <= float(e.get("confidence", -1)) <= 1.0):
            errs.append("confidence")
        if not e.get("reason"):
            errs.append("no_reason")
        if not e.get("evidence"):
            errs.append("no_evidence")
        for ev in e.get("evidence", []):
            n = nodes.get(ev.get("node"))
            if n is None:
                errs.append("evidence_node_missing")
            elif n["type"] not in ("unit", "performance"):
                errs.append("evidence_node_not_text")
            elif not n.get("text") or not ev.get("quote") or ev["quote"] not in n["text"]:
                errs.append("quote_not_verbatim")
        if errs:
            for x in set(errs):
                bad(x, e["id"] + " " + e["kind"] + " " + e["src"] + " -> " + e["dst"])
                dropped[x] += 1
            if drop:
                continue
        seen.add(e["id"])
        keep.append(e)
    if drop:
        graph["edges"] = keep
    report = {"nodes": len(graph["nodes"]), "edges_checked": len(keep) + sum(1 for _ in []) + (sum(dropped.values()) if drop else 0),
              "edges_kept": len(keep), "edges_dropped_by_reason": dict(dropped), "problems": dict(problems),
              "examples": {k: v for k, v in examples.items()}}
    return graph, report


def load_decisions(path):
    return [json.loads(l) for l in open(path, encoding="utf-8") if l.strip()]


def check_decisions(graph, decisions):
    """Ids referenced by the comic's decisions that don't exist in the graph (target id,
    and the before.src/before.dst of edge decisions). Added items ('add') are exempt."""
    ids = {n["id"] for n in graph["nodes"]} | {e["id"] for e in graph["edges"]}
    out = []
    for d in decisions:
        if d.get("action") == "add":
            continue
        refs = [d["id"]] + [(d.get("before") or {}).get(k) for k in ("src", "dst")]
        for r in refs:
            if r and r not in ids and r not in out:
                out.append(r)
    return out


def main():
    args = sys.argv[1:]
    dec = None
    if "--decisions" in args:
        i = args.index("--decisions")
        dec = args[i + 1]
        args = args[:i] + args[i + 2:]
    p = args[0] if args else os.path.join(OUT_DIR, "graph.json")
    g = json.load(open(p, encoding="utf-8"))
    n_before = len(g["edges"])
    _, rep = validate(g, drop=False)
    rep["edges_in_file"] = n_before
    if dec:
        unresolved = check_decisions(g, load_decisions(dec))
        rep["decision_ids_unresolved"] = unresolved
        if unresolved:
            rep["problems"]["decision_id_unresolved"] = len(unresolved)
    print(json.dumps(rep, indent=1))
    sys.exit(1 if rep["problems"] else 0)


if __name__ == "__main__":
    main()
