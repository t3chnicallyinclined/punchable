"""kbmap.py -- KB dicts (graph.json nodes/edges, laughs, decision lines) <-> SurrealDB records, both ways.

The mapping is exact, so the API's SurrealDB backend reproduces the files backend's dicts key for key:
- node id "perf:2015-02-18_x@1200" <-> performance:`2015-02-18_x@1200` (table from the id prefix, key = the rest)
- edge id "e:<hash>" <-> <kind table>:`<hash>`; src/dst <-> in/out
- private replay markers `_foo` <-> `x_foo`; NULL stays null, NONE (absent) stays absent
- DB-only fields (ord, working, working_sets, embedding, run_id) never appear in a KB dict.
"""
from __future__ import annotations

from sdb import RecordID, TYPE_OF_TABLE, edge_key, node_rid, rid_str, strip_none

DB_ONLY_NODE = {"ord", "working", "working_sets", "embedding", "embedded_sha"}
DB_ONLY_EDGE = {"ord", "run_id"}
EDGE_TABLE_OF_KIND = {k: k for k in ["part_of", "variant_of", "performed_as", "tag_of", "same_shape", "callback_to",
                                     "same_premise", "has_label", "segue", "custom"]}
DECISION_KEYS = {"ts": str, "target": str, "id": str, "action": str, "before": dict, "after": dict, "reason": str,
                 "by": str, "undo_of": int}
LAUGH_KEYS = {"id": ("lid", str), "start_ms": ("start_ms", int), "end_ms": ("end_ms", int), "strength": ("strength", (int, float)),
              "bars": ("bars", int), "kind": ("kind", str), "confidence": ("confidence", (int, float)),
              "segment": ("segment", str), "detail": ("detail", object)}


def _priv_out(k: str) -> str:
    return "x_" + k[1:] if k.startswith("_") else k


def _priv_in(k: str) -> str:
    return "_" + k[2:] if k.startswith("x_") else k


# ---------------------------------------------------------------- nodes
def node_to_row(n: dict, ord_: int) -> tuple[str, dict]:
    rid = node_rid(n["id"])
    want = TYPE_OF_TABLE[rid.tb]
    if n.get("type") != want:
        raise ValueError(f"node {n['id']!r} has type {n.get('type')!r} but its id prefix means {want!r}")
    row = {"id": rid, "ord": ord_}
    for k, v in n.items():
        if k in ("id", "type"):
            continue
        k2 = _priv_out(k)
        if k2 in DB_ONLY_NODE:
            raise ValueError(f"node {n['id']!r}: field {k!r} collides with a DB-only field")
        row[k2] = v
    if rid.tb == "performance" and isinstance(row.get("set"), str):
        row["set"] = node_rid(row["set"])
    return rid.tb, row


def row_to_node(rec: dict) -> dict:
    rid = rec["id"]
    out = {"id": rid_str(rid), "type": TYPE_OF_TABLE.get(rid.tb, rid.tb)}
    for k, v in strip_none(rec).items():
        if k == "id" or k in DB_ONLY_NODE:
            continue
        if k == "set" and isinstance(v, RecordID):
            v = rid_str(v)
        out[_priv_in(k)] = v
    return out


# ---------------------------------------------------------------- edges
def edge_to_row(e: dict, ord_: int) -> tuple[str, dict]:
    kind = e.get("kind")
    if kind not in EDGE_TABLE_OF_KIND:
        raise ValueError(f"edge {e.get('id')!r}: unknown kind {kind!r}")
    row = {"id": RecordID(kind, edge_key(e["id"])), "in": node_rid(e["src"]), "out": node_rid(e["dst"]), "ord": ord_}
    for k, v in e.items():
        if k in ("id", "src", "dst", "kind"):
            continue
        k2 = _priv_out(k)
        if k2 in DB_ONLY_EDGE:
            raise ValueError(f"edge {e['id']!r}: field {k!r} collides with a DB-only field")
        row[k2] = v
    src = e.get("source")
    row["run_id"] = src[5:] if isinstance(src, str) and src.startswith("auto:") else None
    return kind, row


def row_to_edge(rec: dict) -> dict:
    rid = rec["id"]
    out = {"id": "e:" + str(rid.key), "src": rid_str(rec.get("in")), "dst": rid_str(rec.get("out")), "kind": rid.tb}
    for k, v in strip_none(rec).items():
        if k in ("id", "in", "out") or k in DB_ONLY_EDGE:
            continue
        out[_priv_in(k)] = v
    return out


# ---------------------------------------------------------------- laughs (measurements)
def laugh_to_row(l: dict, set_rid: RecordID, ord_: int) -> dict:
    row = {"set": set_rid, "ord": ord_}
    extra = {}
    for k, v in l.items():
        spec = LAUGH_KEYS.get(k)
        if spec and (v is None or isinstance(v, spec[1])) and not (spec[1] is int and isinstance(v, bool)):
            row[spec[0]] = v
        else:
            extra[k] = v
    if extra:
        row["extra"] = extra
    start = l.get("start_ms") if isinstance(l.get("start_ms"), int) else -1
    row["id"] = RecordID("laugh", [set_rid, start, ord_])
    return row


def row_to_laugh(rec: dict) -> dict:
    out = {}
    back = {v[0]: k for k, v in LAUGH_KEYS.items()}
    for k, v in strip_none(rec).items():
        if k in ("id", "set", "ord"):
            continue
        if k == "extra":
            out.update(v)
        else:
            out[back.get(k, k)] = v
    return out


# ---------------------------------------------------------------- decisions
def decision_to_row(d: dict, seq: int) -> dict:
    row = {"seq": seq}
    extra = {}
    for k, v in d.items():
        t = DECISION_KEYS.get(k)
        ok = t is not None and (v is None or (isinstance(v, t) and not (t is int and isinstance(v, bool))))
        if ok:
            row["target_id" if k == "id" else k] = v
        else:
            extra[k] = v
    if extra:
        row["extra"] = extra
    return row


def row_to_decision(rec: dict) -> dict:
    r = strip_none(rec)
    out = {}
    for k in DECISION_KEYS:
        src = "target_id" if k == "id" else k
        if src in r:
            out[k] = r[src]
    out.update(r.get("extra") or {})
    return out
