"""One Python gsync CLI call, patched for the recording harness (rust/tools/gen_gsync_tapes.py runs this in a subprocess).

Only the edges are patched, never the logic under test: the clock (so a run id and every stamp are fixed), sleeps (no
waiting), the Google token URI and the Drive endpoint (to the harness's fake server), the browser opener, the memo
processor's spawn (recorded as an exchange instead of started) and the free-disk probe. Everything else is gsync as it
runs on the PC. Input: env GSYNC_TAPE = {"base", "now", "argv", "free_mb"}."""
import datetime as _dt
import json
import os
import sys
import time
import types
import urllib.request
import webbrowser
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO))
T = json.loads(os.environ["GSYNC_TAPE"])
BASE = T["base"]
NOW = _dt.datetime.fromisoformat(T["now"])
EPOCH = NOW.timestamp()

time.time = lambda: EPOCH          # before gsync is imported: default arguments bind these
time.sleep = lambda s: None
webbrowser.open = lambda url, *a, **k: True


class FixedDT(_dt.datetime):
    @classmethod
    def now(cls, tz=None):
        return NOW if tz else NOW.replace(tzinfo=None)


import googleapiclient.discovery as D  # noqa: E402

from googleapiclient import discovery_cache  # noqa: E402


def _patched_build(service, version, credentials=None, **k):
    """The bundled Drive v3 discovery document with its root moved to the fake server (client_options would leave the
    upload URLs on the real host)."""
    doc = json.loads(discovery_cache.get_static_doc(service, version))
    doc["rootUrl"] = BASE + "/"
    doc["baseUrl"] = BASE + "/" + doc["servicePath"]
    return D.build_from_document(doc, credentials=credentials)


D.build = _patched_build

import gsync.__main__ as M  # noqa: E402
import gsync.auth as A  # noqa: E402
import gsync.merge as MG  # noqa: E402
import gsync.memos as MM  # noqa: E402
import gsync.store as S  # noqa: E402
import gsync.sync as SY  # noqa: E402

fixed = lambda: T["now"]  # noqa: E731
for mod in (S, SY, MG, MM):
    mod.now_iso = fixed
SY.datetime = FixedDT
A.GOOGLE_TOKEN_URI = BASE + "/google/token"
import google.oauth2.credentials as GC  # noqa: E402

GC._GOOGLE_OAUTH2_TOKEN_ENDPOINT = BASE + "/google/token"   # the desktop fallback ignores token_uri
MM.free_mb = lambda p: int(T.get("free_mb", 50000))


def _spawn(cfg):
    req = urllib.request.Request(BASE + "/_spawn", data=b"{}", method="POST",
                                 headers={"Content-Type": "application/json"})
    urllib.request.urlopen(req, timeout=10).read()


MM.spawn_processor = _spawn
if __name__ == "__main__":
    rc = 0
    try:
        rc = M.main(T["argv"])
    except SystemExit as e:
        rc = e.code if isinstance(e.code, int) else (print(e.code) or 1)
    sys.stdout.flush()
    sys.exit(rc)
