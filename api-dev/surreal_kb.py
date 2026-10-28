"""The dev API's SurrealDB backend (db/DB.md). Same endpoints, same shapes, data from SurrealDB 3.3.4.

How it stays identical to the files backend: SurrealDB holds the replayed state (graph -> inferred -> analysis ->
decisions applied by the apply_decision EVENT). This module reads that state into the same kb.View the files
backend builds, so every endpoint runs the same code over the same dicts. db/tests/test_baseline.py proves it,
endpoint by endpoint, on the real data.

What comes from SurrealDB itself rather than from Python:
- Working (`rooms.hit_sets`, `rooms.ready_for_working`): joke.working / joke.working_sets, recomputed by
  DEFINE EVENT + fn::working inside the write that changed a match or a rating.
- Writes: a `decision` record (the event applies it in the same transaction), then the same line appended to
  decisions.jsonl, which stays the canonical history.
- Live: LIVE SELECT on meta/joke/decision, pushed to the app over the /api/live WebSocket.
- Hybrid search (?mode=hybrid): BM25 with snowball stemming + HNSW KNN with the hidden filter inside the index,
  fused by search::rrf.
Transcripts and audio are still read from the corpus/tapes folders (they are not graph data).
"""
from __future__ import annotations

import json
import os
import sys
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "db"))
import sdb  # noqa: E402
from kbmap import decision_to_row, row_to_decision, row_to_edge, row_to_laugh, row_to_node  # noqa: E402

import kb as K  # noqa: E402  (tools/explorer, already on sys.path via server.py)

NODE_TABLES = ["joke", "unit", "set", "performance", "label", "list"]


class SnapshotView(K.View):
    """kb.View built from SurrealDB's already-replayed records (no replay here: the DB event did it)."""

    def __init__(self, nodes, edges, decisions, report, laughs_for, orphans, graph_meta):
        self.graph_meta = graph_meta
        self.nodes = {n["id"]: dict(n) for n in nodes}
        self.edges = {e["id"]: dict(e) for e in edges}
        self.decisions = decisions
        self.report = report
        self.laughs_for = laughs_for
        self.orphans = orphans
        self.undone = {d["undo_of"] for d in decisions if isinstance(d.get("undo_of"), int)}
        self._resolve_merges()
        self._index()


class LiveHub:
    """One SurrealDB connection holding LIVE SELECTs; coalesces notifications and fans them out to /api/live."""

    TABLES = ["meta", "joke", "decision", "creative_event"]  # creative_event: boards, working sets, pins, jobs

    def __init__(self, kb: "SurrealKB"):
        self.kb = kb
        self.clients: set = set()
        self.lock = threading.Lock()
        self.pending: list = []
        self.connected = False
        self.last_event_at = None
        self.sent = 0
        threading.Thread(target=self._run, daemon=True, name="live-hub").start()
        threading.Thread(target=self._flush_loop, daemon=True, name="live-flush").start()

    def subscribe(self, q):
        with self.lock:
            self.clients.add(q)

    def unsubscribe(self, q):
        with self.lock:
            self.clients.discard(q)

    def _run(self):
        backoff = 1
        while True:
            r = None
            try:
                r = sdb.Rpc(self.kb.port, self.kb.ns, self.kb.db)
                for t in self.TABLES:
                    r.query(f"LIVE SELECT * FROM {t};")
                self.connected = True
                backoff = 1
                while True:
                    msg = r.recv_live(30)
                    if msg is not None:
                        self._on(msg)
            except Exception as e:  # noqa: BLE001
                self.connected = False
                sys.stderr.write(f"live: connection lost ({type(e).__name__}: {e}); retrying in {backoff}s\n")
                if r:
                    r.close()
                time.sleep(backoff)
                backoff = min(backoff * 2, 30)

    def _on(self, msg):
        res = msg.get("result") if isinstance(msg, dict) else None
        if not isinstance(res, dict):
            return
        rec = res.get("result") if isinstance(res.get("result"), dict) else {}
        rid = res.get("record") or rec.get("id")
        tb = rid.tb if isinstance(rid, sdb.RecordID) else None
        ch = {"table": tb, "action": str(res.get("action") or "").lower(), "id": sdb.rid_str(rid) if rid is not None else None}
        if tb == "joke":
            ch["working"] = rec.get("working")
            ch["hit_sets"] = len(rec.get("working_sets") or [])
        if tb == "creative_event":
            ch["store"], ch["op"], ch["item"] = rec.get("store"), rec.get("op"), rec.get("item")
        if tb == "meta":
            ch["gen"] = rec.get("gen")
            ch["loading"] = rec.get("loading")
        with self.lock:
            self.pending.append(ch)

    def _flush_loop(self):
        while True:
            time.sleep(0.15)
            with self.lock:
                if not self.pending:
                    continue
                if any(c.get("table") == "meta" and c.get("loading") for c in self.pending[-1:]):
                    continue  # a reload is running: one event when it ends
                batch, self.pending = self.pending, []
                clients = list(self.clients)
            gen = next((c["gen"] for c in reversed(batch) if c.get("gen") is not None), None)
            ev = {"type": "change", "rev": f"db:{gen}" if gen is not None else None, "count": len(batch),
                  "changes": [c for c in batch if c["table"] != "meta"][:50], "at": time.time()}
            self.last_event_at = ev["at"]
            for q in clients:
                q.put(ev)
            self.sent += 1


class SurrealKB:
    """Mixin over server.AppKB (composed in server.py): the view, laughs, AI index and writes come from SurrealDB."""

    backend = "surreal"

    def init_surreal(self, ns: str, db: str, port: int | None = None, live: bool = True, watch: bool = True):
        self.ns, self.db, self.port = ns, db, port  # None: SURREAL_URL/SURREAL_PORT from the env
        self.version = sdb.check_version(self.port)
        self.rpc = sdb.Rpc(self.port, ns, db)
        import load as L
        L.apply_extra_schema(self.rpc)  # additive tables (creative events, file mirror): no reload needed
        missing = sdb.check_changefeeds(self.rpc)
        if missing:
            raise SystemExit(f"refusing to serve: change feed missing on {sorted(missing)} (db/DB.md, trap T6). "
                             "Re-apply db/schema.surql with python db/load.py.")
        st = self._state()
        if not st or not (st.get("load_info") or {}).get("graph"):
            raise SystemExit(f"refusing to serve: {ns}/{db} has never been loaded (python db/load.py)")
        self._snap = None
        self._snap_gen = None
        self._sview = None
        self.embedder = None
        self.live = LiveHub(self) if live else None
        self.writes = 0
        self.syncs = []
        self.sync_error = None
        self._jsig = None  # first watcher pass counts the lines: catches writes made while no API was watching
        if watch:  # hosted: no watcher; the database is the source there and push_remote.py loads it
            threading.Thread(target=self._watch, daemon=True, name="file-watch").start()

    # ---- new pipeline output: the files backend sees it on the next request; so must we ------------
    def _watch(self, interval: float = 1.0):
        """Every second: if an input file changed (graph.json, inferred.json, any analysis.json / laughs_auto.json /
        meta.json, id_map.json), or another writer appended to decisions.jsonl, run db/load.py's incremental sync
        (writes only what changed; new decision lines go through the event). Holds the KB lock, so requests wait
        for it (about a second) rather than see a half-synced state."""
        import load as L
        failed_sig = None
        while True:
            time.sleep(interval)
            try:
                li = self._state().get("load_info") or {}
                if not li.get("graph"):
                    continue
                src = {"graph": Path(li["graph"]), "kbdir": Path(li["kb"]), "corpus": Path(li["corpus"]),
                       "fixtures": Path(li["fixtures"]) if li.get("fixtures") else None,
                       "id_map": Path(li["id_map"]) if li.get("id_map") else None}
                sig = L.sources_signature(**src)
                why = None
                if sig != li.get("sources_sig"):
                    why = "input files changed"
                jsig = self._sig(self.decisions_path)
                if jsig != self._jsig:
                    lines, _ = self._read_file_decisions()
                    if len(lines) != self.rpc.last("RETURN count(SELECT id FROM decision);"):
                        why = why or "decisions.jsonl was appended by another writer"
                    self._jsig = jsig
                if not why or (sig, jsig) == failed_sig:
                    continue
                t = time.perf_counter()
                with self.lock:
                    r = sdb.Rpc(self.port, None, None)
                    try:
                        info = L.sync(r, ns=self.ns, db=self.db, version=li.get("graph_version", "?"),
                                      reason=li.get("graph_reason", ""), **src)
                    finally:
                        r.close()
                    self._jsig = self._sig(self.decisions_path)
                self.syncs.append({"at": time.time(), "why": why, "mode": info.get("mode"), "changed": info.get("sync_changed"),
                                   "s": round(time.perf_counter() - t, 2)})
                self.syncs = self.syncs[-20:]
                self.sync_error = failed_sig = None
                sys.stderr.write(f"sync ({why}): {info.get('mode')} {info.get('sync_changed')} in {self.syncs[-1]['s']}s\n")
            except Exception as e:  # noqa: BLE001
                self.sync_error = f"{type(e).__name__}: {e}"
                failed_sig = (locals().get("sig"), locals().get("jsig"))
                sys.stderr.write(f"sync failed: {self.sync_error}\n")

    def sync_now(self, why: str) -> dict:
        """Re-sync the graph tables from this API's own files now (after the home worker posted pipeline output,
        api-dev/ingest_api.py). Same code as the watcher (db/load.py sync: writes only what changed). Hosted, the
        files are this pod's cache of the srcfile rows, in another folder than the dbload pod's: allow_moved."""
        import load as L
        li = self._state().get("load_info") or {}
        graph = Path(getattr(self, "creative_graph", None) or self.graph_path)
        t = time.perf_counter()
        with self.lock:
            r = sdb.Rpc(self.port, None, None)
            try:
                info = L.sync(r, ns=self.ns, db=self.db, graph=graph, kbdir=Path(self.kbdir), corpus=Path(self.corpus),
                              fixtures=self.ai.fixtures, id_map=self.ai.id_map_path,
                              version=li.get("graph_version", "?"), reason=li.get("graph_reason", ""),
                              allow_moved=bool(os.environ.get("PK_DATA")))
            finally:
                r.close()
            self._jsig = self._sig(self.decisions_path)
            self._sview = None
        out = {"at": time.time(), "why": why, "mode": info.get("mode"), "changed": info.get("sync_changed"),
               "s": round(time.perf_counter() - t, 2)}
        self.syncs = (self.syncs + [out])[-20:]
        return {k: out[k] for k in ("mode", "changed", "s")}

    # ---- state ---------------------------------------------------------------
    def _state(self) -> dict:
        r = self.rpc.last("SELECT gen, loading, orphans, load_info FROM meta:state;")
        return sdb.strip_none(r[0]) if r else {}

    def _fetch(self, st: dict):
        t = time.perf_counter()
        q = self.rpc.query(
            "SELECT * OMIT embedding FROM joke, unit, set, performance, label, list;"
            "SELECT * FROM " + ", ".join(sdb.EDGE_TABLES) + ";"
            "SELECT * FROM decision ORDER BY seq;"
            "SELECT * FROM laugh_run;"
            "SELECT * FROM laugh;"
            "SELECT * FROM analysis ORDER BY ord;")
        nodes_r, edges_r, dec_r, runs_r, laughs_r, an_r = (x["result"] for x in q)
        nodes_r.sort(key=lambda x: x["ord"])
        edges_r.sort(key=lambda x: x["ord"])
        working = {sdb.rid_str(n["id"]): (bool(n.get("working")), [sdb.rid_str(s) for s in (n.get("working_sets") or [])])
                   for n in nodes_r if n["id"].tb == "joke"}
        laughs_by_set: dict = {}
        for l in sorted(laughs_r, key=lambda x: x["ord"]):
            laughs_by_set.setdefault(sdb.rid_str(l["set"]), []).append(row_to_laugh(l))
        laugh_docs = {}
        for run in runs_r:
            sid = sdb.rid_str(run["set"])
            laugh_docs[run["set_dir"]] = {**sdb.strip_none(run["doc"]), "laughs": laughs_by_set.get(sid, [])}
        info = st.get("load_info") or {}
        ai = {"rows": {}, "analysis": {}, "inferred": info.get("inferred")}
        if info.get("id_map"):
            ai["id_map"] = info.get("id_map_log")
        for a in an_r:
            sid = sdb.rid_str(a["set"])
            ai["analysis"][sid] = sdb.strip_none(a["meta"])
            ai["rows"][sid] = [(sdb.strip_none(x["row"]), x["perf"]) for x in a["rows"]]
        self._nodes = {sdb.rid_str(n["id"]): (n["ord"], row_to_node(n)) for n in nodes_r}
        self._edges = {"e:" + str(e["id"].key): (e["ord"], row_to_edge(e)) for e in edges_r}
        self._snap = {"decisions": [row_to_decision(d) for d in dec_r], "laughs": laugh_docs, "ai": ai,
                      "working": working, "info": info, "orphans": st.get("orphans") or 0,
                      "fetch_s": round(time.perf_counter() - t, 3)}
        self._snap_gen = st.get("gen")

    def full_view(self) -> K.View:
        with self.lock:
            st = self._state()
            stale = self._snap is None or st.get("gen") != self._snap_gen
            if stale and (not st.get("loading") or self._snap is None):
                self._fetch(st)
                self._sview = None
            if self._sview is None:
                s = self._snap
                info = s["info"]
                self.graph_path = Path(info.get("graph") or self.graph_path)
                self.graph_state = "ok"
                self.bad_lines = info.get("bad_lines", 0) + getattr(self, "_bad_since", 0)
                report = K.parse_report(self.report_path) if self.report_path else []
                nodes = [x[1] for x in sorted(self._nodes.values(), key=lambda x: x[0])]
                edges = [x[1] for x in sorted(self._edges.values(), key=lambda x: x[0])]
                v = SnapshotView(nodes, edges, s["decisions"], report, self.laughs_for, s["orphans"],
                                 info.get("graph_meta") or {})
                v.ai = s["ai"]
                v.db_working = s["working"]
                self.ai_index = s["ai"]
                self._sview = v
                self.rev = f"db:{self._snap_gen}"
            return self._sview

    # ---- measurements --------------------------------------------------------
    def laughs_doc(self, set_id):
        d = self.set_dir(set_id)
        if not d:
            return None
        self.full_view()
        return self._snap["laughs"].get(d)

    # ---- writes: DB first (the event applies it), then the canonical jsonl --------
    def _append(self, d: dict) -> int:
        with self.lock:
            self.full_view()  # a current snapshot to patch
            q = self.rpc.query("SELECT gen, loading, orphans FROM ONLY meta:state; RETURN count(SELECT id FROM decision);")
            st, n_db = sdb.strip_none(q[0]["result"]), q[1]["result"]
            if st.get("loading"):
                raise K.DecisionError("the database is reloading; try again in a few seconds", 503)
            lines, bad = self._read_file_decisions()
            if n_db != len(lines):
                raise K.DecisionError(f"SurrealDB has {n_db} decisions but decisions.jsonl has {len(lines)}; "
                                      "run python db/load.py to resync before writing", 409)
            seq = n_db
            row = decision_to_row(d, seq)
            # The decision and its effect (apply_decision event, then the Working events) commit together; the
            # same statement batch reads back what changed, so the snapshot is patched without a full re-read.
            tgt = "fn::edge_find($tid)" if d.get("target") == "edge" else "fn::rid($tid)"
            q = self.rpc.query(
                "BEGIN TRANSACTION;"
                "CREATE decision CONTENT $row RETURN id;"
                f"LET $t = {tgt};"
                "IF $t != NONE AND record::exists($t) { (SELECT * OMIT embedding FROM ONLY $t) } ELSE { NONE };"
                "SELECT id, working, working_sets FROM joke WHERE working = true OR array::len(working_sets) > 0;"
                "SELECT gen, orphans FROM ONLY meta:state;"
                "COMMIT TRANSACTION;",
                {"row": row, "tid": d.get("id") if d.get("target") in ("edge", "node") else "joke:none"})
            # results: 0 BEGIN, 1 CREATE, 2 LET, 3 target, 4 working, 5 meta, 6 COMMIT
            if not q[1]["result"]:
                raise sdb.EmptyWrite(f"decision {seq} was not written")
            self._write_line(d)
            self._jsig = self._sig(self.decisions_path)  # our own append is not "another writer"
            rec, wk, st2 = q[3]["result"], q[4]["result"], sdb.strip_none(q[5]["result"])
            if (st2.get("orphans") or 0) > (st.get("orphans") or 0):
                sys.stderr.write(f"WARNING: decision {seq} found no target in SurrealDB ({d.get('id')})\n")
            self.writes += 1
            if st2.get("gen") == (self._snap_gen or 0) + 1 and st.get("gen") == self._snap_gen:
                self._patch(d, rec, wk, row, st2)  # only this write changed the DB since the snapshot
            self._sview = None
            return seq

    def _patch(self, d: dict, rec, working_rows: list, row: dict, st: dict):
        did = d.get("id")
        store = self._edges if d.get("target") == "edge" else self._nodes if d.get("target") == "node" else None
        if store is not None and did:
            if isinstance(rec, dict) and rec:
                store[did] = (rec["ord"], row_to_edge(rec) if store is self._edges else row_to_node(rec))
            else:
                store.pop(did, None)
        w = self._snap["working"]
        for k in w:
            w[k] = (False, [])
        for x in working_rows:
            w[sdb.rid_str(x["id"])] = (bool(x["working"]), [sdb.rid_str(s) for s in x.get("working_sets") or []])
        self._snap["decisions"].append(row_to_decision(row))
        self._snap["orphans"] = st.get("orphans") or 0
        self._snap_gen = st.get("gen")

    def _read_file_decisions(self):
        out, bad = [], 0
        try:
            with open(self.decisions_path, "r", encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        x = json.loads(line)
                    except ValueError:
                        bad += 1
                        continue
                    if isinstance(x, dict):
                        out.append(x)
                    else:
                        bad += 1
        except FileNotFoundError:
            pass
        return out, bad

    def _write_line(self, d: dict):
        """kb.KB._append's file half: append-only, fsync, never glue onto a torn last line."""
        line = json.dumps(d, ensure_ascii=False, separators=(",", ":"))
        assert "\n" not in line
        self.decisions_path.parent.mkdir(parents=True, exist_ok=True)
        with open(self.decisions_path, "ab") as f:
            if f.tell() > 0:
                with open(self.decisions_path, "rb") as r:
                    r.seek(-1, os.SEEK_END)
                    if r.read(1) != b"\n":
                        f.write(b"\n")
            f.write(line.encode("utf-8") + b"\n")
            f.flush()
            os.fsync(f.fileno())

    # ---- search (api-dev/search_api.py) --------------------------------------------
    def _emb(self):
        """The embedder, resolved once: search_api.make_embedder (the model under PK_BGE_DIR, or PK_EMBEDDER=hash),
        else db/embedder.py's node sidecar on a PC that still has it, else none (False: resolved, nothing here)."""
        if self.embedder is None:
            import search_api as SR
            e = SR.make_embedder(log=lambda *a: sys.stderr.write(" ".join(map(str, a)) + "\n"))
            if e is None:
                try:
                    from embedder import Embedder
                    e = Embedder()
                    e.name = getattr(e, "engine", "node")
                except Exception:  # noqa: BLE001
                    e = None
            self.embedder = e if e is not None else False
        return self.embedder or None

    def search_all(self, q: str, limit: int, include_hidden: bool) -> dict:
        """Raw fused hits over unit text, joke title, joke summary, performance text and the three vector indexes
        (search_api.search_all); the caller folds units into jokes with the view."""
        import search_api as SR
        with self.lock:
            return SR.search_all(self.rpc, self._emb(), q, limit, include_hidden)

    def similar_vectors(self, v, jid: str, limit: int, include_hidden: bool) -> dict:
        import search_api as SR
        with self.lock:
            return SR.similar(self.rpc, self._emb(), v, jid, limit, include_hidden)

    def hybrid_search(self, q: str, limit: int, include_hidden: bool):
        """Joke ids, best first (the Bits list's search): BM25 over unit text, joke titles and typed ideas + HNSW KNN
        over unit and joke vectors, fused with search::rrf. Words only when no query vector can be made."""
        import search_api as SR
        res = self.search_all(q, limit, include_hidden)
        with self.lock:
            v = self.view(include_hidden)
        return SR.fold_jokes(res["hits"], v)

    # ---- status ------------------------------------------------------------------
    def status(self) -> dict:
        st = self._state()
        info = st.get("load_info") or {}
        return {"backend": "surreal", "surreal": ".".join(map(str, self.version)), "url": sdb.rpc_url(self.port),
                "ns": self.ns, "db": self.db, "gen": st.get("gen"), "loading": st.get("loading"),
                "graph": info.get("graph"), "graph_version": info.get("graph_version"), "loaded_at": info.get("loaded_at"),
                "counts": info.get("counts"), "id_map": info.get("id_map_log"),
                "changefeeds_missing": sdb.check_changefeeds(self.rpc),
                "live": {"connected": bool(self.live and self.live.connected), "clients": len(self.live.clients) if self.live else 0,
                         "events_sent": self.live.sent if self.live else 0},
                "snapshot_fetch_s": (self._snap or {}).get("fetch_s"), "writes": self.writes,
                "vectors": (self.sweeper.status() if getattr(self, "sweeper", None) else
                            {"embedder": getattr(self.embedder, "name", None) if self.embedder else None}),
                "file_syncs": self.syncs[-5:], "sync_error": self.sync_error, "load_mode": info.get("mode"),
                "last_sync_changed": info.get("sync_changed")}
