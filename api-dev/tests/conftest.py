"""Most of these tests run the API over TEMP copies of a real knowledge base (db/tests/common.py: TempKB copies
graph.json, decisions and the corpus sets from PUNCH_COMEDY / ~/punchable-data). Without that data they are skipped;
tests that build their own data still run."""
import sys
from pathlib import Path

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "db"))
import sdb  # noqa: E402

HAS_KB = (sdb.COMEDY / "kb" / "v0" / "graph.json").is_file() and (sdb.COMEDY / "corpus" / "sets").is_dir()


def pytest_collection_modifyitems(config, items):
    if HAS_KB:
        return
    skip = pytest.mark.skip(reason=f"needs a knowledge base at {sdb.COMEDY} (set PUNCH_COMEDY)")
    for it in items:
        if hasattr(it.module, "TempKB") or "api" in getattr(it, "fixturenames", ()):
            it.add_marker(skip)
