"""load.py -- (re)build Punchable's SurrealDB from the KB files. Idempotent: every run rebuilds the same state.

    python db/load.py                    # graph auto (v1 if kb/v1/graph.json passes kb_validate, else v0)
    python db/load.py --graph v0         # or v1, or a path
    python db/load.py --ns t --db x --kb <tmp kb> --corpus <tmp corpus>     (tests: temp namespace + temp copies)

Order (docs/KB_SCHEMA.md precedence: the comic's decisions > inferred > proposed):
  1. graph.json (v1 if it exists and passes tools/kb/kb_validate.py with the decisions, else v0)
  2. inferred.json            \\  laid over the graph by api-dev/ai_overlay.py, the same code the files backend
  3. corpus/sets/*/analysis.json /  runs (ids resolved through kb/v1/id_map.json)
  4. corpus/sets/*/laughs_auto.json (measurements)
  5. decisions.jsonl, replayed line by line as `decision` records: the apply_decision EVENT applies each one
  6. unit embeddings from tools/kb/cache (bge-small), missing ones embedded locally (db/embed.mjs)
Then Working is backfilled through fn::recompute_working, and the change feeds are checked.

CBOR only (db/sdb.py). Every write is guarded: an empty result raises (sdb.EmptyWrite).
The files are only read. decisions.jsonl stays the canonical history.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
DESK = HERE.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(DESK / "api-dev"))
import ai_overlay as AI  # noqa: E402
import sdb  # noqa: E402
from kbmap import decision_to_row, edge_to_row, laugh_to_row, node_to_row  # noqa: E402

COMEDY = sdb.COMEDY
KB_V0 = COMEDY / "kb" / "v0"
KB_V1 = COMEDY / "kb" / "v1"
CORPUS = COMEDY / "corpus"
FIXTURES = DESK / "api-dev" / "fixtures"
VECTORS = Path(os.environ["PK_CACHE_DIR"]) / "kb" if os.environ.get("PK_CACHE_DIR") else DESK / "tools" / "kb" / "cache"
CHOICE = sdb.DATA_ROOT / "graph_choice.json"   # what the last real load used; the API's files backend follows it
EXTRA_VEC = sdb.DATA_ROOT / "cache" / "unit_vectors_extra.json"
BATCH = 400
NODE_ORDER = ["set", "label", "joke", "unit", "performance", "list"]
DATA_TABLES = ["decision", "analysis", "laugh", "laugh_run"] + sdb.EDGE_TABLES + ["performance", "unit", "joke", "label", "list", "set"]


def log(*a):
    print(*a, flush=True)


# ---------------------------------------------------------------- graph choice
def validate_graph(graph: Path, kbdir: Path, id_map: Path | None) -> tuple[bool, str]:
    """Valid = tools/kb/kb_validate.py passes on the graph's structure, AND replaying decisions.jsonl over
    graph + inferred + analysis (exactly what gets loaded) leaves no orphan decision.

    kb_validate's own --decisions check is stricter than the replay: it flags edits on items an earlier `add` line
    created (list:tonight) and confirms of AI-inferred edges that live in the overlay, not graph.json. The replay
    is what the app shows, so it is the test that matters."""
    try:
        p = subprocess.run([sys.executable, str(DESK / "tools" / "kb" / "kb_validate.py"), str(graph)],
                           cwd=str(DESK / "tools" / "kb"), capture_output=True, text=True, timeout=600)
    except (OSError, subprocess.TimeoutExpired) as e:
        return False, f"validator did not run: {e}"
    if p.returncode != 0:
        return False, "kb_validate failed: " + (p.stdout[-400:] or p.stderr[-400:])
    sys.path.insert(0, str(DESK / "tools" / "explorer"))
    import kb as K
    g = json.load(open(graph, encoding="utf-8"))
    merged, _ = AI.build_overlay(g, AI.AIFiles(kbdir, CORPUS, FIXTURES, id_map))
    decs, _ = read_decisions(kbdir / "decisions.jsonl")
    v = K.View(merged, decs, [], lambda *a: None)
    if v.orphans:
        return False, f"{v.orphans} decision(s) find no target when replayed over this graph"
    return True, f"kb_validate passed; all {len(decs)} decisions replay with no orphan"


def resolve_graph(choice: str, kbdir: Path) -> dict:
    """{graph, version, id_map, reason}."""
    if choice not in ("auto", "v0", "v1"):
        g = Path(choice)
        return {"graph": g, "version": "path", "id_map": (KB_V1 / "id_map.json") if g.parent == KB_V1 else None,
                "reason": "given on the command line"}
    if choice in ("auto", "v1") and (KB_V1 / "graph.json").is_file():
        im = KB_V1 / "id_map.json"
        im = im if im.is_file() else None
        ok, why = validate_graph(KB_V1 / "graph.json", kbdir, im)
        if ok:
            return {"graph": KB_V1 / "graph.json", "version": "v1", "id_map": im, "reason": why}
        if choice == "v1":
            raise SystemExit(f"v1 requested but {why}")
        log(f"graph: v1 not used ({why}); falling back to v0")
        return {"graph": kbdir / "graph.json", "version": "v0", "id_map": None, "reason": f"v1 invalid: {why}"}
    return {"graph": kbdir / "graph.json", "version": "v0", "id_map": None,
            "reason": "v0 requested" if choice == "v0" else "no kb/v1/graph.json"}


# ---------------------------------------------------------------- helpers
def read_decisions(p: Path) -> tuple[list[dict], int]:
    """Exactly kb.KB._read_decisions: valid JSON objects in order; everything else is a bad line."""
    out, bad = [], 0
    if not p.is_file():
        return out, bad
    with open(p, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                d = json.loads(line)
            except ValueError:
                bad += 1
                continue
            if isinstance(d, dict):
                out.append(d)
            else:
                bad += 1
    return out, bad


def load_vectors() -> dict[str, list[float]]:
    import numpy as np
    out = {}
    ids_p, f32 = VECTORS / "vectors_ids.json", VECTORS / "vectors.f32"
    if ids_p.is_file() and f32.is_file():
        ids = json.load(open(ids_p, encoding="utf-8"))
        m = np.fromfile(f32, dtype=np.float32).reshape(len(ids), 384)
        for i, k in enumerate(ids):
            if k.startswith("unit:"):
                out[k] = m[i].astype(float).tolist()
    if EXTRA_VEC.is_file():
        out.update(json.load(open(EXTRA_VEC, encoding="utf-8")))
    return out


def apply_schema(r: sdb.Rpc, ns: str, db: str):
    if not r.db_user:  # a database-level user (hosted dbload) cannot define namespaces; infra/db-init.sh made them
        r.query(f"DEFINE NAMESPACE IF NOT EXISTS {ns}; USE NS {ns}; DEFINE DATABASE IF NOT EXISTS {db} STRICT;")
    r.use(ns, db)
    r.query((HERE / "schema.surql").read_text(encoding="utf-8"))
    apply_extra_schema(r)
    r.query("IF !record::exists(meta:state) { CREATE meta:state CONTENT { gen: 0, ord_next: 0, orphans: 0, loading: false, load_info: {} } };")
    missing = sdb.check_changefeeds(r)
    if missing:
        # Trap T6: something re-defined a table without its CHANGEFEED clause. IF NOT EXISTS can't restore it, so
        # re-define exactly those tables from schema.surql with OVERWRITE (which carries the clause), loudly.
        import re
        text = (HERE / "schema.surql").read_text(encoding="utf-8")
        for t in missing:
            m = re.search(rf"^DEFINE TABLE IF NOT EXISTS {t} [^;]*;", text, re.M | re.S)
            if not m:
                raise SystemExit(f"change feed missing on {t} and schema.surql has no definition for it")
            log(f"REPAIR: change feed was missing on table {t} ({missing[t]}); re-defining it with its feed")
            r.query(m.group(0).replace("IF NOT EXISTS", "OVERWRITE", 1))
        missing = sdb.check_changefeeds(r)
        if missing:
            raise SystemExit(f"change feed still missing after repair: {missing}")


def apply_extra_schema(r: sdb.Rpc):
    """Tables added for hosting (db/schema_extra.surql: creative events, the job queue, the hosted file mirror).
    Additive and idempotent, so the API also applies it at startup without a reload."""
    r.query((HERE / "schema_extra.surql").read_text(encoding="utf-8"))


def insert_rows(r: sdb.Rpc, table: str | None, rows: list[dict], relation: bool = False):
    for i in range(0, len(rows), BATCH):
        chunk = rows[i:i + BATCH]
        if relation:
            r.write("INSERT RELATION $rows RETURN id;", {"rows": chunk}, expect=len(chunk))
        else:
            r.write(f"INSERT INTO {table} $rows RETURN id;", {"rows": chunk}, expect=len(chunk))


def write_side_tables(r: sdb.Rpc, corpus: Path, index: dict) -> int:
    """laugh_run + laugh (every set folder with a laughs_auto.json) and analysis (the AI index). Replaces them."""
    for tb in ("analysis", "laugh", "laugh_run"):
        r.query(f"DELETE {tb} RETURN NONE;")
    nl = 0
    sets_dir = corpus / "sets"
    if sets_dir.is_dir():
        for d in sorted(sets_dir.iterdir()):
            p = d / "laughs_auto.json"
            sid = AI.set_id_of(d.name)
            if not sid or not p.is_file():
                continue
            try:
                doc = json.loads(p.read_text(encoding="utf-8"))
            except ValueError as e:
                log(f"laughs: skipped unreadable {p}: {e}")
                continue
            if not isinstance(doc, dict):
                continue
            srid = sdb.node_rid(sid)
            r.write("CREATE type::record('laugh_run', $k) CONTENT $c RETURN id;",
                    {"k": srid.key, "c": {"set": srid, "set_dir": d.name, "doc": {k: v for k, v in doc.items() if k != "laughs"}}})
            ls = doc.get("laughs") if isinstance(doc.get("laughs"), list) else []
            rows = [laugh_to_row(l, srid, i) for i, l in enumerate(ls) if isinstance(l, dict)]
            if rows:
                insert_rows(r, "laugh", rows)
            nl += len(rows)
    # the AI index (set analyses and the inferred run), so the API needs no AI files at all
    for i, (sid, meta) in enumerate(index["analysis"].items()):
        srid = sdb.node_rid(sid)
        r.write("CREATE type::record('analysis', $k) CONTENT $c RETURN id;",
                {"k": srid.key, "c": {"set": srid, "ord": i, "meta": meta,
                                      "rows": [{"row": row, "perf": pid} for row, pid in index["rows"].get(sid, [])]}})
    return nl


def sources_signature(graph: Path, kbdir: Path, corpus: Path, fixtures: Path | None, id_map: Path | None) -> list:
    """(path, mtime_ns, size) of every input file except decisions.jsonl (the API writes that itself)."""
    paths = [graph, kbdir / "inferred.json"] + ([id_map] if id_map else [])
    for root in [corpus / "sets"] + ([fixtures / "sets", fixtures] if fixtures else []):
        if root.is_dir():
            for d in sorted(root.iterdir()):
                if d.is_dir():
                    paths += [d / "analysis.json", d / "laughs_auto.json", d / "meta.json"]
    if fixtures:
        paths.append(fixtures / "inferred.json")
    out = []
    for p in paths:
        try:
            st = p.stat()
            out.append([str(p), st.st_mtime_ns, st.st_size])
        except OSError:
            pass
    return out


def make_info(r, g, index, *, graph, kbdir, corpus, fixtures, id_map, version, reason, bad, n_decisions, laughs,
              embedded, mode) -> dict:
    counts = r.last("""RETURN {
        joke: count(SELECT id FROM joke), unit: count(SELECT id FROM unit), label: count(SELECT id FROM label),
        set: count(SELECT id FROM set), performance: count(SELECT id FROM performance), list: count(SELECT id FROM list),
        laugh: count(SELECT id FROM laugh), decision: count(SELECT id FROM decision), analysis: count(SELECT id FROM analysis),
        edges: math::sum([""" + ", ".join(f"count(SELECT id FROM {t})" for t in sdb.EDGE_TABLES) + """]),
        working: count(SELECT id FROM joke WHERE working = true),
        vectors: count(SELECT id FROM unit WHERE embedding != NONE),
        hidden_units: count(SELECT id FROM unit WHERE hidden = true)
    };""")
    st = r.last("SELECT orphans, gen FROM ONLY meta:state;")
    return {"graph": str(graph), "graph_version": version, "graph_reason": reason,
            "graph_meta": {k: g.get(k) for k in ("version", "generated_at", "runs")},
            "kb": str(kbdir), "decisions": str(kbdir / "decisions.jsonl"), "corpus": str(corpus),
            "fixtures": str(fixtures) if fixtures else None, "id_map": str(id_map) if id_map else None,
            "id_map_log": index.get("id_map"), "inferred": index.get("inferred"),
            "bad_lines": bad, "decision_lines": n_decisions, "orphans": st.get("orphans"),
            "loaded_at": datetime.now(timezone.utc).isoformat(timespec="seconds"), "mode": mode,
            "counts": counts, "missing_vectors_embedded": embedded, "laughs_loaded": laughs,
            "sources_sig": sources_signature(graph, kbdir, corpus, fixtures, id_map)}


# ---------------------------------------------------------------- the load
def load(r: sdb.Rpc, *, ns: str, db: str, graph: Path, kbdir: Path, corpus: Path, fixtures: Path | None,
         id_map: Path | None, vectors: bool = True, embed_missing: bool = True, version: str = "?", reason: str = "") -> dict:
    t0 = time.perf_counter()
    phases = {}
    apply_schema(r, ns, db)
    r.write("UPDATE meta:state SET loading = true, gen += 1 RETURN id;")
    try:
        # -- clear (definitions and change feeds stay; only rows go)
        t = time.perf_counter()
        for tb in DATA_TABLES:
            r.query(f"DELETE {tb} RETURN NONE;")
        r.write("UPDATE meta:state SET ord_next = 0, orphans = 0 RETURN id;")
        phases["clear"] = time.perf_counter() - t

        # -- graph -> inferred -> analysis (api-dev's own overlay code)
        t = time.perf_counter()
        g = json.load(open(graph, encoding="utf-8"))
        files = AI.AIFiles(kbdir, corpus, fixtures, id_map)
        merged, index = AI.build_overlay(g, files)
        phases["overlay"] = time.perf_counter() - t

        # -- nodes
        t = time.perf_counter()
        vec = load_vectors() if vectors else {}
        rows_by = {tb: [] for tb in NODE_ORDER}
        missing_vec = []
        ord_ = 0
        for n in merged["nodes"]:
            tb, row = node_to_row(n, ord_)
            ord_ += 1
            if tb == "unit" and vectors:
                v = vec.get(n["id"])
                if v is not None:
                    row["embedding"] = v
                elif (n.get("text") or "").strip():
                    missing_vec.append(row)
            rows_by.setdefault(tb, []).append(row)
        if missing_vec and embed_missing:
            from embedder import Embedder
            em = Embedder()
            try:
                vs = em.embed([x["text"] for x in missing_vec])
            finally:
                em.close()
            extra = json.load(open(EXTRA_VEC, encoding="utf-8")) if EXTRA_VEC.is_file() else {}
            for row, v in zip(missing_vec, vs):
                row["embedding"] = v
                extra[sdb.rid_str(row["id"])] = v
            EXTRA_VEC.parent.mkdir(parents=True, exist_ok=True)
            json.dump(extra, open(EXTRA_VEC, "w", encoding="utf-8"))
        for tb in NODE_ORDER:
            insert_rows(r, tb, rows_by.get(tb, []))
        phases["nodes"] = time.perf_counter() - t

        # -- edges
        t = time.perf_counter()
        erows = []
        for e in merged["edges"]:
            _, row = edge_to_row(e, ord_)
            ord_ += 1
            erows.append(row)
        insert_rows(r, None, erows, relation=True)
        phases["edges"] = time.perf_counter() - t

        t = time.perf_counter()
        nl = write_side_tables(r, corpus, index)
        phases["laughs+analysis"] = time.perf_counter() - t

        # -- decisions, replayed through the apply_decision EVENT (precedence: decisions > inferred > proposed)
        t = time.perf_counter()
        r.write("UPDATE meta:state SET ord_next = $n RETURN id;", {"n": ord_})
        decs, bad = read_decisions(kbdir / "decisions.jsonl")
        for i, d in enumerate(decs):
            try:
                r.write("CREATE decision CONTENT $row RETURN id;", {"row": decision_to_row(d, i)})
            except Exception as e:  # noqa: BLE001
                raise SystemExit(f"decisions.jsonl line {i} could not be replayed: {e}")
        phases["decisions"] = time.perf_counter() - t

        # -- Working, recomputed for every joke by the one function the events use
        t = time.perf_counter()
        r.query("FOR $j IN (SELECT VALUE id FROM joke) { fn::recompute_working($j); };")
        phases["working"] = time.perf_counter() - t

        info = make_info(r, g, index, graph=graph, kbdir=kbdir, corpus=corpus, fixtures=fixtures, id_map=id_map,
                         version=version, reason=reason, bad=bad, n_decisions=len(decs), laughs=nl,
                         embedded=len(missing_vec) if embed_missing else 0, mode="full load")
        r.write("UPDATE meta:state SET load_info = $info RETURN id;", {"info": info})
    finally:
        r.query("UPDATE meta:state SET loading = false, gen += 1 RETURN NONE;")
    missing = sdb.check_changefeeds(r)
    if missing:
        raise SystemExit(f"change feed missing after load: {missing}")
    info["phases_s"] = {k: round(v, 2) for k, v in phases.items()}
    info["total_s"] = round(time.perf_counter() - t0, 2)
    return info


def db_records(r: sdb.Rpc):
    """The DB's current nodes and edges as KB dicts, with their ord, and its decision rows."""
    from kbmap import row_to_edge, row_to_node
    q = r.query("SELECT * OMIT embedding FROM joke, unit, set, performance, label, list;"
                "SELECT * FROM " + ", ".join(sdb.EDGE_TABLES) + ";"
                "SELECT * FROM decision ORDER BY seq;")
    nodes = {sdb.rid_str(n["id"]): (n["ord"], row_to_node(n)) for n in q[0]["result"]}
    edges = {"e:" + str(e["id"].key): (e["ord"], row_to_edge(e)) for e in q[1]["result"]}
    return nodes, edges, q[2]["result"]


def sync(r: sdb.Rpc, *, ns: str, db: str, graph: Path, kbdir: Path, corpus: Path, fixtures: Path | None,
         id_map: Path | None, version: str = "?", reason: str = "", force_full: bool = False) -> dict:
    """Bring the DB up to date with the files by writing only what changed (new pipeline output, a new
    analysis.json, decision lines appended by another writer). Falls back to load() when it can't be incremental.

    The target is computed by the same code as the files backend (overlay, then kb.View's replay of the decision
    lines the DB already has); records that differ are rewritten, and new decision lines are appended as `decision`
    records so the event applies them, exactly as on a full load."""
    sys.path.insert(0, str(DESK / "tools" / "explorer"))
    import kb as K
    from kbmap import row_to_decision

    class ReplayOnly(K.View):  # replay only: merges are resolved by the reader, as with the DB
        def _resolve_merges(self):
            self.alias = {}

        def _index(self):
            pass

    def full(why):
        log(f"sync: full load ({why})")
        return load(r, ns=ns, db=db, graph=graph, kbdir=kbdir, corpus=corpus, fixtures=fixtures, id_map=id_map,
                    version=version, reason=reason)

    apply_schema(r, ns, db)
    st = sdb.strip_none(r.last("SELECT * FROM ONLY meta:state;") or {})
    li = st.get("load_info") or {}
    if force_full or not li.get("graph"):
        return full("forced" if force_full else "never loaded")
    if li.get("graph") != str(graph) or li.get("kb") != str(kbdir) or li.get("corpus") != str(corpus):
        return full("graph, kb or corpus folder changed")
    t0 = time.perf_counter()
    g = json.load(open(graph, encoding="utf-8"))
    merged, index = AI.build_overlay(g, AI.AIFiles(kbdir, corpus, fixtures, id_map))
    decs, bad = read_decisions(kbdir / "decisions.jsonl")
    nodes_db, edges_db, dec_rows = db_records(r)
    n_db = len(dec_rows)
    if n_db > len(decs) or [row_to_decision(x) for x in dec_rows] != decs[:n_db]:
        return full("the DB's decisions are not a prefix of decisions.jsonl")
    want = ReplayOnly(merged, decs[:n_db], [], None)

    # ord: keep every existing record's ord; slot new ones between their neighbours (ord is a number).
    # Order matters within nodes and within edges (each is sorted on its own), not across them.
    ords = {}
    for kind, keys, dbmap in (("n", list(want.nodes), nodes_db), ("e", list(want.edges), edges_db)):
        last, pending = None, []
        for k in keys:
            if k in dbmap:
                o = dbmap[k][0]
                if last is not None and o <= last:
                    return full("record order changed")
                lo = last if last is not None else o - 1
                for i, pk in enumerate(pending):
                    ords[(kind, pk)] = lo + (o - lo) * (i + 1) / (len(pending) + 1)
                pending, last = [], o
                ords[(kind, k)] = o
            else:
                pending.append(k)
        top = max([last if last is not None else -1] + [v[0] for v in nodes_db.values()] + [v[0] for v in edges_db.values()])
        for i, pk in enumerate(pending):
            ords[(kind, pk)] = top + i + 1

    r.write("UPDATE meta:state SET loading = true, gen += 1 RETURN id;")
    changed = {"nodes": 0, "edges": 0, "removed": 0, "decisions": 0}
    try:
        vec = None
        for nid in [k for k in nodes_db if k not in want.nodes]:
            r.write("DELETE $id RETURN BEFORE;", {"id": sdb.node_rid(nid)})
            changed["removed"] += 1
        for nid, n in want.nodes.items():
            cur = nodes_db.get(nid)
            if cur and cur[1] == n:
                continue
            tb, row = node_to_row(n, ords[("n", nid)])
            if tb == "unit":
                vec = vec if vec is not None else load_vectors()
                if nid in vec:
                    row["embedding"] = vec[nid]
            if tb == "joke":
                # working / working_sets are DB-only (fn::working): CONTENT replaces the record and a field DEFAULT
                # does not fill an existing one, so set them and recompute right after (was: "Expected bool but
                # found NONE" on any joke a pipeline changed)
                row.update(working=False, working_sets=[])
            r.write("UPSERT $id CONTENT $row RETURN id;", {"id": row["id"], "row": {k: v for k, v in row.items() if k != "id"}})
            if tb == "joke":
                r.query("fn::recompute_working($id);", {"id": row["id"]})
            changed["nodes"] += 1
        for eid in [k for k in edges_db if k not in want.edges]:
            r.write("DELETE $id RETURN BEFORE;", {"id": sdb.RecordID(edges_db[eid][1]["kind"], sdb.edge_key(eid))})
            changed["removed"] += 1
        for eid, e in want.edges.items():
            cur = edges_db.get(eid)
            if cur and cur[1] == e:
                continue
            if cur:
                r.write("DELETE $id RETURN BEFORE;", {"id": sdb.RecordID(cur[1]["kind"], sdb.edge_key(eid))})
            _, row = edge_to_row(e, ords[("e", eid)])
            r.write("INSERT RELATION [$row] RETURN id;", {"row": row}, expect=1)
            changed["edges"] += 1
        nl = write_side_tables(r, corpus, index)
        mx = max(ords.values(), default=-1)
        r.write("UPDATE meta:state SET ord_next = $n RETURN id;", {"n": int(mx) + 1})
        for i in range(n_db, len(decs)):
            r.write("CREATE decision CONTENT $row RETURN id;", {"row": decision_to_row(decs[i], i)})
            changed["decisions"] += 1
        info = make_info(r, g, index, graph=graph, kbdir=kbdir, corpus=corpus, fixtures=fixtures, id_map=id_map,
                         version=li.get("graph_version", version), reason=li.get("graph_reason", reason), bad=bad,
                         n_decisions=len(decs), laughs=nl, embedded=0, mode="sync")
        info["sync_changed"] = changed
        r.write("UPDATE meta:state SET load_info = $info RETURN id;", {"info": info})
    finally:
        r.query("UPDATE meta:state SET loading = false, gen += 1 RETURN NONE;")
    info["total_s"] = round(time.perf_counter() - t0, 2)
    return info


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--graph", default="auto", help="auto | v0 | v1 | <path to graph.json>")
    ap.add_argument("--kb", type=Path, default=KB_V0, help="folder with decisions.jsonl and inferred.json (default kb/v0)")
    ap.add_argument("--corpus", type=Path, default=CORPUS)
    ap.add_argument("--fixtures", type=Path, default=FIXTURES)
    ap.add_argument("--no-fixtures", action="store_true")
    ap.add_argument("--id-map", type=Path, default=None, help="default: kb/v1/id_map.json when the graph is v1")
    ap.add_argument("--ns", default=sdb.NS)
    ap.add_argument("--db", default=sdb.DB)
    ap.add_argument("--port", type=int, default=None)
    ap.add_argument("--no-vectors", action="store_true")
    ap.add_argument("--sync", action="store_true",
                    help="incremental: write only what changed since the last load (what the API's file watcher runs)")
    a = ap.parse_args()
    sdb.check_version(a.port)
    sel = resolve_graph(a.graph, a.kb)
    idm = a.id_map or sel["id_map"]
    log(f"graph {sel['version']}: {sel['graph']} ({sel['reason']})")
    r = sdb.Rpc(a.port, ns=None, db=None)
    try:
        fn = sync if a.sync else load
        kw = {} if a.sync else {"vectors": not a.no_vectors}
        info = fn(r, ns=a.ns, db=a.db, graph=sel["graph"], kbdir=a.kb, corpus=a.corpus,
                  fixtures=None if a.no_fixtures else a.fixtures, id_map=idm, version=sel["version"], reason=sel["reason"], **kw)
    finally:
        r.close()
    if (a.ns, a.db) == (sdb.NS, sdb.DB):
        CHOICE.parent.mkdir(parents=True, exist_ok=True)
        json.dump({"graph": str(sel["graph"]), "version": sel["version"], "id_map": str(idm) if idm else None,
                   "kb": str(a.kb), "loaded_at": info["loaded_at"]}, open(CHOICE, "w", encoding="utf-8"), indent=1)
    log(json.dumps({k: v for k, v in info.items() if k not in ("graph_meta", "inferred", "sources_sig")}, indent=1, default=str))


if __name__ == "__main__":
    main()
