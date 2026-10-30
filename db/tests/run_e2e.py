"""Run the app's e2e suite (app/tests/e2e.mjs) against the SurrealDB backend.

    python db/tests/run_e2e.py                 # REAL_KB / APP_URL / SHOTS pass through to e2e.mjs

Starts a throwaway in-memory SurrealDB 3.3.4, then runs e2e.mjs with PUNCH_BACKEND=surreal and
PUNCH_SURREAL_TEMP=1: every API the suite spawns loads its own TEMP kb copy into a fresh temp database there.
The real store and the real decisions are never touched. Afterwards it deletes the temp folders the run left
in %TEMP% (each run leaves a ~150 MB Chrome profile otherwise; the disk is tight).
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from common import DESK, scratch_server


def main():
    t0 = time.time()
    port = scratch_server()
    tmp = Path(tempfile.gettempdir())
    before = {p.name for p in tmp.glob("punch_*")}
    env = {**os.environ, "PUNCH_BACKEND": "surreal", "PUNCH_SURREAL_TEMP": "1", "SURREAL_PORT": str(port)}
    print(f"e2e on the SurrealDB backend (scratch server 127.0.0.1:{port})", flush=True)
    rc = subprocess.call(["node", str(DESK / "app" / "tests" / "e2e.mjs")], env=env, cwd=str(DESK))
    for p in tmp.glob("punch_*"):
        if p.name not in before and p.is_dir():
            shutil.rmtree(p, ignore_errors=True)
    print(f"e2e exit {rc} in {time.time() - t0:.0f}s; temp folders removed", flush=True)
    sys.exit(rc)


if __name__ == "__main__":
    main()
