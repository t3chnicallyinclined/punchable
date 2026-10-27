"""Test isolation for every suite in the repo: the tests never read YOUR private settings.

$PUNCHABLE_HOME (default ~/.punchable) holds your Drive ids, legal rules, venue table and secrets, and modules read it at
import time, so it is pointed at an empty temporary folder before any test module is imported. The identity settings
(PK_OWNER_ID, PK_COMIC_NAME, PK_COMIC_FIRST) are cleared so the tests see the public defaults."""
import os
import tempfile

os.environ["PUNCHABLE_HOME"] = tempfile.mkdtemp(prefix="punchable-test-home-")
for _k in ("PUNCHABLE_LEGAL_RULES", "PUNCHABLE_VENUES", "PK_OWNER_ID", "PK_COMIC_NAME", "PK_COMIC_FIRST", "PK_CACHE_DIR",
           "PK_INFER_JUDGE_DIR"):
    os.environ.pop(_k, None)
