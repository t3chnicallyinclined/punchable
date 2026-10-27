"""Run every SurrealDB-layer test. Each runs on its own in-memory SurrealDB with TEMP copies of the files.

    python db/tests/run_all.py            # baseline diff, mutation + live, guards, hidden
    python db/tests/run_all.py --e2e      # ... plus the app's e2e suite on the SurrealDB backend (needs Expo web on :8798)
"""
from __future__ import annotations

import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
TESTS = ["test_guards.py", "test_hidden.py", "test_mutation_live.py", "test_baseline.py", "test_creative.py", "test_wal.py", "test_online.py"]


def main():
    tests = TESTS + (["run_e2e.py"] if "--e2e" in sys.argv else [])
    results = []
    for t in tests:
        t0 = time.time()
        print(f"\n===== {t}", flush=True)
        rc = subprocess.call([sys.executable, str(HERE / t)], cwd=str(HERE))
        results.append((t, rc, time.time() - t0))
    print("\n===== summary")
    for t, rc, s in results:
        print(f"{'PASS' if rc == 0 else 'FAIL'}  {t}  ({s:.0f}s)")
    sys.exit(0 if all(rc == 0 for _, rc, _ in results) else 1)


if __name__ == "__main__":
    main()
