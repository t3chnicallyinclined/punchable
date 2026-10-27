"""Guard tests for the SurrealDB layer (docs/SURREALDB_AUDIT.md traps), all on TEMP databases.

    python db/tests/test_guards.py

1. Empty-OK guard (T5): a write that changes nothing returns `OK []`; sdb.Rpc.write must raise EmptyWrite.
   Checked for a missing target, a row-count mismatch, and a write DENIED by table permissions under a record
   (end-user) session, which SurrealDB 3.3.4 still reports as `OK []`.
2. Change feeds present (T6): after a load every feed table has its CHANGEFEED; `DEFINE TABLE OVERWRITE` without
   the clause silently drops it; the check notices; the API refuses to serve that database; a re-run of
   db/load.py repairs it.
3. CBOR only (T1): text that looks like a record id or a date stays text.
4. SurrealDB version >= 3.3.1.
"""
from __future__ import annotations

import subprocess
import sys

import common  # noqa: F401  (puts db/ on sys.path)
import sdb
from common import Api, DESK, TempDB, TempKB

FAILS: list[str] = []


def check(ok: bool, msg: str):
    print(("PASS " if ok else "FAIL ") + msg)
    if not ok:
        FAILS.append(msg)


def raises(fn, exc) -> bool:
    try:
        fn()
    except exc:
        return True
    return False


def main():
    v = sdb.check_version()
    check(v >= (3, 3, 1), f"SurrealDB {'.'.join(map(str, v))} >= 3.3.1")

    t = TempKB("guard")
    db = TempDB("guard")
    side = TempDB("guardperm")
    try:
        db.load(t)
        r = db.rpc()

        # ---- 1. empty-OK guard
        res = r.last("UPDATE joke:`no-such-joke` SET title = 'x' RETURN id;")
        check(res == [], f"trap reproduced: UPDATE of a missing record returns OK {res}")
        check(raises(lambda: r.write("UPDATE joke:`no-such-joke` SET title = 'x' RETURN id;"), sdb.EmptyWrite),
              "guard: write() raises EmptyWrite on that empty result")
        check(raises(lambda: r.write("UPDATE joke SET title = title WHERE id = joke:`no-such-joke` RETURN id;"), sdb.EmptyWrite),
              "guard: a filtered UPDATE that matched nothing raises EmptyWrite")
        jid = r.last("SELECT VALUE id FROM joke LIMIT 2;")
        check(raises(lambda: r.write("UPDATE $ids SET stage = stage RETURN id;", {"ids": jid}, expect=3), sdb.EmptyWrite),
              "guard: a row-count mismatch (2 written, 3 expected) raises EmptyWrite")
        check(len(r.write("UPDATE $ids SET stage = stage RETURN id;", {"ids": jid}, expect=2)) == 2,
              "guard: a real write passes")
        # denied write under a record (end-user) session: OK [] (trap), guard raises
        rr = sdb.Rpc(None, ns=None, db=None)
        rr.query(f"DEFINE NAMESPACE IF NOT EXISTS {side.ns}; USE NS {side.ns}; DEFINE DATABASE {side.db};")
        rr.use(side.ns, side.db)
        rr.query("DEFINE TABLE usr SCHEMALESS PERMISSIONS FOR select FULL;"
                 "DEFINE ACCESS acc ON DATABASE TYPE RECORD SIGNIN (SELECT * FROM usr WHERE name = $name) DURATION FOR SESSION 1h;"
                 "CREATE usr:1 SET name = 'comic';"
                 "DEFINE TABLE decision_like SCHEMALESS PERMISSIONS FOR select, create FULL, FOR update, delete NONE;"
                 "CREATE decision_like:1 SET v = 1;")
        rr.close()
        u = sdb.Rpc(None, ns=None, db=None, signin=False)
        u.call("signin", [{"NS": side.ns, "DB": side.db, "AC": "acc", "name": "comic"}])
        res = u.last("UPDATE decision_like:1 SET v = 2 RETURN id;")
        check(res == [], f"trap reproduced: a write DENIED by permissions returns OK {res} (no error)")
        check(raises(lambda: u.write("UPDATE decision_like:1 SET v = 2 RETURN id;"), sdb.EmptyWrite),
              "guard: the denied write raises EmptyWrite instead of looking saved")
        u.close()

        # ---- 3. CBOR keeps text as text
        r.write("UPDATE joke:`" + jid[0].key + "` SET summary = $s, first_seen = $d, stage_reason = $j RETURN id;",
                {"s": "note: see doc", "d": "2015-02-18", "j": "joke:airport-bit"})
        back = r.last("SELECT summary, first_seen, stage_reason FROM ONLY $id;", {"id": jid[0]})
        check(all(isinstance(back[k], str) for k in back) and back["summary"] == "note: see doc"
              and back["first_seen"] == "2015-02-18" and back["stage_reason"] == "joke:airport-bit",
              f"CBOR: record-id-like and date-like text round-trips as text {back}")

        # ---- 2. change feeds
        check(sdb.check_changefeeds(r) == {}, f"change feeds present on {', '.join(sdb.CHANGEFEED_TABLES)}")
        r.query("DEFINE TABLE OVERWRITE joke SCHEMAFULL;")
        info = r.last("INFO FOR DB;")["tables"]["joke"]
        missing = sdb.check_changefeeds(r)
        check("CHANGEFEED" not in info and "joke" in missing,
              f"trap reproduced: DEFINE TABLE OVERWRITE without the clause silently drops the feed; the check reports {sorted(missing)}")
        try:
            Api(t, "surreal", db)
            refused = False
        except RuntimeError as e:
            refused = "change feed missing" in str(e)
        check(refused, "the API refuses to serve a database whose change feed is missing")
        r.close()
        p = subprocess.run([sys.executable, str(DESK / "db" / "load.py"), "--graph", str(t.graph), "--kb", str(t.kb),
                            "--corpus", str(t.corpus), "--ns", db.ns, "--db", db.db]
                           + (["--id-map", str(t.id_map)] if t.id_map else []), capture_output=True, text=True, encoding="utf-8")
        r = db.rpc()
        check(p.returncode == 0 and "REPAIR: change feed was missing on table joke" in p.stdout and sdb.check_changefeeds(r) == {},
              "db/load.py repairs the dropped feed (and says so)")
        r.close()
    finally:
        db.drop()
        side.drop()
        t.cleanup()
    print(f"\n{'PASS' if not FAILS else 'FAIL'}: {len(FAILS)} failed")
    sys.exit(1 if FAILS else 0)


if __name__ == "__main__":
    main()
