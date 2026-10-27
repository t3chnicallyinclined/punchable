"""An Rpc must survive SurrealDB restarting (2026-10-27: `infra/ship.sh base` restarted the hosted database and the API
pod stayed unready until it was restarted by hand).

    python db/tests/test_reconnect.py

Cause: websocket-client's recv() returns '' once the server has closed the socket, cbor2.loads('') raised
"TypeError: a bytes-like object is required, not 'str'", and the API's one long-lived connection was never reopened.
The fix (db/sdb.py Rpc): a dropped socket raises ConnectionLost for that one call (never replayed: a write may or may not
have landed) and the next call reconnects and signs in again.

This test runs its OWN SurrealDB (memory store) on a free port, then:
  1. a query works;
  2. the server is killed and started again on the same port;
  3. the next call raises ConnectionLost (not TypeError);
  4. the call after that works on the same Rpc object, signed in, in the same ns/db.
"""
from __future__ import annotations

import sys

import common  # noqa: F401  (puts db/ on sys.path)
import sdb
from common import free_port


def main() -> int:
    fails = []

    def check(ok, what):
        print(("PASS " if ok else "FAIL ") + what)
        if not ok:
            fails.append(what)

    port = free_port()
    proc = sdb.start_scratch(port)
    try:
        r = sdb.Rpc(port, "t", "t")
        r.query("DEFINE NAMESPACE IF NOT EXISTS t; DEFINE DATABASE IF NOT EXISTS t;")
        check(r.last("RETURN 1 + 1;") == 2, "a query works before the restart")

        proc.kill()
        proc.wait()
        proc = sdb.start_scratch(port)

        try:
            r.last("RETURN 1;")
            check(False, "the first call after the restart reports the dropped connection")
        except sdb.ConnectionLost:
            check(True, "the first call after the restart raises ConnectionLost")
        except Exception as e:  # noqa: BLE001
            check(False, f"the first call after the restart raises ConnectionLost, got {type(e).__name__}: {e}")

        try:
            r.query("DEFINE NAMESPACE IF NOT EXISTS t; DEFINE DATABASE IF NOT EXISTS t;")
            check(r.last("RETURN 2 + 2;") == 4, "the next call on the same Rpc works")
            check(r.last("RETURN session::ns();") == "t" and r.last("RETURN session::db();") == "t",
                  "the reconnected Rpc is back in the same namespace and database")
        except Exception as e:  # noqa: BLE001
            check(False, f"the next call on the same Rpc works, got {type(e).__name__}: {e}")
        r.close()
    finally:
        proc.kill()
    print(f"{'OK' if not fails else 'FAILED'}: {len(fails)} failure(s)")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
