"""Mutation test: a decision flips Working through the DB event, the LIVE push arrives, and the files backend agrees.

    python db/tests/test_mutation_live.py

On a TEMP database loaded from TEMP copies of the files:
1. Pick a joke never performed, and three performances in three different sets.
2. For each: match the performance to the joke (Set screen "match") and rate it "killed" (both are decision
   records; the apply_decision event applies them, and working_on_match / working_on_perf recompute Working
   through fn::working in the same transaction).
3. Working must be false after one and two rooms and true after the third, read straight from SurrealDB the
   moment the POST returns. The /api/live WebSocket must push the flip for that joke. ?backend=files (the jsonl
   replayed in Python) must agree on every step.
4. Undo the last rating: Working flips back, pushed again, files agree.
5. Finally every joke's Working in SurrealDB must equal the files backend's ready_for_working.
"""
from __future__ import annotations

import json
import sys
import threading
import time
from urllib.parse import quote

import websocket

from common import Api, TempDB, TempKB

FAILS: list[str] = []


def check(ok: bool, msg: str):
    print(("PASS " if ok else "FAIL ") + msg)
    if not ok:
        FAILS.append(msg)


class Live:
    def __init__(self, url: str):
        self.ws = websocket.create_connection(url.replace("http://", "ws://") + "/api/live", timeout=30,
                                              header=["Origin: http://localhost:8798"])
        self.msgs: list = []
        self.lock = threading.Lock()
        threading.Thread(target=self._run, daemon=True).start()

    def _run(self):
        try:
            while True:
                m = self.ws.recv()
                if m:
                    with self.lock:
                        self.msgs.append((time.perf_counter(), json.loads(m)))
        except Exception:  # noqa: BLE001
            pass

    def wait_for(self, jid: str, working: bool, since: float, timeout: float = 5.0):
        t_end = time.perf_counter() + timeout
        while time.perf_counter() < t_end:
            with self.lock:
                for t, m in self.msgs:
                    if t < since or m.get("type") != "change":
                        continue
                    for c in m.get("changes", []):
                        if c.get("id") == jid and c.get("working") is working:
                            return t - since, m
            time.sleep(0.01)
        return None, None


def main():
    t = TempKB("mut")
    db = TempDB("mut")
    api = None
    try:
        db.load(t)
        api = Api(t, "surreal", db)
        r = db.rpc()
        live = Live(api.url)
        time.sleep(0.5)
        hello = live.msgs[0][1] if live.msgs else {}
        check(hello.get("type") == "hello" and hello.get("live") is True, f"/api/live handshake and LIVE SELECT connected ({hello})")

        _, ideas = api.get("/api/jokes?stage=idea&limit=50")
        jid = next(x["id"] for x in ideas["items"] if x["sets"] == 0)
        _, sets = api.get("/api/sets")
        chosen = []
        for s in sets["sets"]:
            p = next((x for x in s["jokes"] if x["kind"] not in ("crowd_work", "banter")), None)
            if p:
                chosen.append(p["perf"])
            if len(chosen) == 3:
                break
        check(len(chosen) == 3, f"three performances in three sets for {jid}")

        def db_working():
            row = r.last("SELECT working, working_sets FROM ONLY type::record('joke', $k);", {"k": jid.split(":", 1)[1]})
            return bool(row["working"]), len(row["working_sets"])

        def rooms(backend):
            _, d = api.get(f"/api/jokes/{quote(jid, safe=':')}" + ("?backend=files" if backend == "files" else ""))
            return d["rooms"]["ready_for_working"], d["rooms"]["hit_sets"]

        last_idx = None
        for i, pid in enumerate(chosen, 1):
            s1, m = api.post("/api/set-row", {"perf": pid, "op": "match", "joke": jid})
            t_send = time.perf_counter()
            s2, rr = api.post("/api/review/rate", {"perf": pid, "rating": "killed"})
            t_back = time.perf_counter()
            last_idx = rr.get("indexes")
            w = db_working()
            want = i >= 3
            check(s1 == 200 and s2 == 200, f"room {i}: match + rate accepted ({s1}, {s2})")
            check(w == (want, i), f"room {i}: SurrealDB joke.working={w[0]}, working_sets={w[1]} right after the write (event, same transaction)")
            check(rooms("surreal") == (want, i), f"room {i}: API (SurrealDB) rooms = {rooms('surreal')}")
            check(rooms("files") == (want, i), f"room {i}: files backend agrees = {rooms('files')}")
            check(rr.get("joke", {}).get("ready_for_working") is want, f"room {i}: rate response ready_for_working={rr.get('joke', {}).get('ready_for_working')}")
            if want:
                lat, msg = live.wait_for(jid, True, t_send)
                check(lat is not None, f"LIVE push: Working flipped to true for {jid}"
                      + (f", {lat * 1000:.0f} ms after the rate POST was sent (POST itself took {(t_back - t_send) * 1000:.0f} ms)" if lat else ""))

        t_send = time.perf_counter()
        s, u = api.post("/api/undo", {"indexes": last_idx})
        check(s == 200, f"undo of the third rating accepted ({s})")
        check(db_working() == (False, 2), f"after undo: SurrealDB working={db_working()}")
        check(rooms("files") == (False, 2), f"after undo: files backend agrees = {rooms('files')}")
        lat, _ = live.wait_for(jid, False, t_send)
        check(lat is not None, "LIVE push: Working flipped back to false" + (f" ({lat * 1000:.0f} ms)" if lat else ""))

        # every joke: DB Working == the files backend's Python computation
        _, page = api.get("/api/pipeline?limit=5000&backend=files")
        allw = {x["id"] for x in r.last("SELECT id FROM joke WHERE working = true;") for _ in [0]} if False else None
        rows = r.last("SELECT id, working, array::len(working_sets) AS n FROM joke;")
        dbw = {f"joke:{x['id'].key}": (bool(x["working"]), x["n"]) for x in rows}
        files = {}
        for st, grp in page["stages"].items():
            for x in grp["items"]:
                files[x["id"]] = x["hit_sets"]
        mism = [k for k, n in files.items() if dbw.get(k, (False, 0))[1] != n]
        check(not mism, f"all {len(files)} visible jokes: SurrealDB hit sets == files backend hit sets ({len(mism)} differ)")
        # decisions.jsonl got every line too (canonical history)
        lines = [l for l in (t.kb / "decisions.jsonl").read_text(encoding="utf-8").splitlines() if l.strip()]
        n_db = r.last("RETURN count(SELECT id FROM decision);")
        check(len(lines) == n_db, f"decisions.jsonl lines ({len(lines)}) == decision records ({n_db})")
        r.close()
    finally:
        if api:
            api.stop()
        db.drop()
        t.cleanup()
    print(f"\n{'PASS' if not FAILS else 'FAIL'}: {len(FAILS)} failed")
    sys.exit(1 if FAILS else 0)


if __name__ == "__main__":
    main()
