"""The SurrealKV write-ahead log must not grow when nothing changed (2026-10-27: the real store's WAL reached 853 MB).

    python db/tests/test_wal.py

Cause found on the real store: 76% of the WAL was index keys of `unit`, under 26 different index ids. Every
incremental sync re-applied db/schema.surql, and `DEFINE INDEX ... OVERWRITE` rebuilds an index from every row even
when its definition is identical (HNSW over 2,200 vectors + full text, each time). The fix: db/load.apply_schema
keeps the schema's sha256 in meta:state and skips an unchanged schema.

This test runs its OWN SurrealDB on a temp SurrealKV folder (not memory: the WAL is what is measured), loads TEMP
copies of the KB, then:
  1. an incremental sync with nothing changed must add < 5 MB to the WAL (it was ~22 MB per sync before the fix);
  2. apply_schema() reports "unchanged" for the same schema, and "applied" with force=True;
  3. the forced apply, measured the same way, grows the WAL by > 4x that, so the measurement can see the bug;
  4. the API's startup schema step (apply_extra_schema) is also skipped when unchanged.
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import common  # noqa: F401  (puts db/ on sys.path)
import sdb
from common import DB_DIR, TempKB, free_port

FAILS: list[str] = []


def check(ok: bool, msg: str):
    print(("PASS " if ok else "FAIL ") + msg)
    if not ok:
        FAILS.append(msg)


def wal_bytes(store: Path) -> int:
    return sum(p.stat().st_size for p in (store / "wal").glob("*") if p.is_file())


def settle(store: Path) -> int:
    """WAL size once it stops moving (writes are group-committed)."""
    last, same = -1, 0
    for _ in range(50):
        n = wal_bytes(store)
        same = same + 1 if n == last else 0
        if same >= 3:
            return n
        last = n
        time.sleep(0.1)
    return last


def main():
    tmp = Path(tempfile.mkdtemp(prefix="punch_wal_"))
    store = tmp / "data"
    port = free_port()
    os.environ.pop("SURREAL_SURREALKV_MAX_MEMTABLE_SIZE", None)  # default memtable: every write stays in the WAL
    proc = sdb.start_scratch(port, "surrealkv://" + store.as_posix())
    os.environ["SURREAL_PORT"] = str(port)
    t = TempKB("wal")
    ns, db = "punchable_test", "wal"
    env = {**os.environ, "SURREAL_PORT": str(port)}

    def load(*extra):
        p = subprocess.run([sys.executable, str(DB_DIR / "load.py"), "--graph", str(t.graph), "--kb", str(t.kb),
                            "--corpus", str(t.corpus), "--ns", ns, "--db", db, *extra]
                           + (["--id-map", str(t.id_map)] if t.id_map else []),
                           capture_output=True, text=True, encoding="utf-8", env=env)
        if p.returncode != 0:
            raise RuntimeError("load failed:\n" + p.stdout[-2000:] + p.stderr[-3000:])
        return p.stdout

    try:
        load()
        r = sdb.Rpc(port, ns, db)
        units = r.last("RETURN count(SELECT id FROM unit WHERE embedding != NONE);")
        print(f"loaded: {units} units with vectors; WAL {settle(store) / 1e6:.1f} MB")

        load("--sync")  # the first sync after a full load may still write load_info etc.
        a = settle(store)
        load("--sync")
        b = settle(store)
        grow = (b - a) / 1e6
        # bounded by the side tables a sync rewrites (laughs, analysis: ~0.05 MB a set), never by an index rebuild
        check(grow < 5.0, f"an incremental sync with nothing changed grew the WAL by {grow:.2f} MB (< 5 MB)")

        import load as L
        check(L.apply_schema(r, ns, db) is False, "apply_schema: an unchanged schema is not re-applied")
        check(L.apply_extra_schema(r) is False, "apply_extra_schema (the API's startup step): unchanged -> skipped")
        c = settle(store)
        check(L.apply_schema(r, ns, db, force=True) is True, "apply_schema(force=True) applies it")
        d = settle(store)
        forced = (d - c) / 1e6
        check(forced > 4 * max(grow, 1.0), f"the measurement sees the bug: one forced schema apply grew the WAL by "
              f"{forced:.1f} MB (> 4x a no-change sync, {grow:.2f} MB)")
        cf = sdb.check_changefeeds(r)
        check(not cf, f"change feeds intact after all of it ({cf or 'all present'})")
        r.close()
    finally:
        proc.terminate()
        try:
            proc.wait(10)
        except subprocess.TimeoutExpired:
            proc.kill()
        t.cleanup()
        shutil.rmtree(tmp, ignore_errors=True)
    print(f"\n{'ALL PASS' if not FAILS else f'{len(FAILS)} FAILED'}")
    sys.exit(1 if FAILS else 0)


if __name__ == "__main__":
    main()
