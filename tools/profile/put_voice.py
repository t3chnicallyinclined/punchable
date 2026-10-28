"""Load the comic's voice profile into the hosted database, once, from a private file on the desk.

    python tools/profile/put_voice.py --file <path> [--api https://punchable.io] [--as worker|owner] [--dry-run]

The profile is DATA in the database (api-dev/profile_api.py: store `profile`, document profile:voice), the single
source of truth for every writing prompt. It is never a file in this repo or in an image: this tool reads it from a
path outside the repo and sends it over HTTPS, nothing else. The text is never printed; only its length, a short sha
and the server's `updated_at`.

Two ways in, both existing credentials:
  --as worker (default)  the pk-worker client credentials in ~/.punchable/secrets.env (PK_WORKER_SECRET; PK_API and
                         PK_TOKEN_URL optional), the same flow the worker uses (worker/__main__.Client), to
                         PUT /api/worker/profile/voice with actor import:profile. The document's `by` is then
                         import:profile ("loaded from your desk" in Settings), which is the truth.
  --as owner             a bearer token for the app's routes in $PK_OWNER_TOKEN (never on the command line), to
                         PUT /api/profile/voice: `by` is the owner. The Settings screen does the same thing by hand.
Before anything is sent, the text is checked locally with the legal rules every ingest applies (gsync/rules.py
content_is_legal); a hit stops here. The API checks again (422). --dry-run reads and checks, sends nothing.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

DESK = Path(__file__).resolve().parents[2]
if str(DESK) not in sys.path:
    sys.path.insert(0, str(DESK))

MAX_CHARS = 60_000  # api-dev/profile_api.py


def legal(text: str) -> bool:
    spec = importlib.util.spec_from_file_location("pk_legal_rules", DESK / "gsync" / "rules.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return bool(mod.content_is_legal(text))


def read_profile(path: Path) -> str:
    if DESK in path.resolve().parents:
        raise SystemExit(f"refusing: {path} is inside the public repo; the profile lives outside it")
    text = path.read_text(encoding="utf-8").replace("\r\n", "\n").strip("\n") + "\n"
    if not text.strip():
        raise SystemExit("the file is empty")
    if len(text) > MAX_CHARS:
        raise SystemExit(f"the profile is {len(text):,} characters; the API keeps at most {MAX_CHARS:,}")
    if legal(text):
        raise SystemExit("refusing: the legal rules match this text (court material is never stored); nothing was sent")
    return text


def describe(text: str) -> str:
    return f"{len(text):,} chars, {len(text.split()):,} words, sha {hashlib.sha256(text.encode('utf-8')).hexdigest()[:12]}"


def put_as_owner(api: str, text: str) -> dict:
    tok = os.environ.get("PK_OWNER_TOKEN", "").strip()
    if not tok:
        raise SystemExit("--as owner needs a bearer token in $PK_OWNER_TOKEN (the app's token; never on the command line)")
    req = urllib.request.Request(api.rstrip("/") + "/api/profile/voice", data=json.dumps({"text": text}).encode(), method="PUT",
                                 headers={"Content-Type": "application/json", "Authorization": "Bearer " + tok,
                                          "User-Agent": "punchable-put-voice/1"})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        raise SystemExit(f"PUT /api/profile/voice: HTTP {e.code} {e.read()[:300].decode('utf-8', 'replace')}") from None


def put_as_worker(api: str | None, text: str, no_auth: bool) -> dict:
    import worker.__main__ as W
    s = W.secrets()
    api = api or s.get("PK_API", W.DEFAULT_API)
    if no_auth and urllib.parse.urlparse(api).hostname not in ("127.0.0.1", "localhost"):
        raise SystemExit("--no-auth is only for the loopback dev API")
    cl = W.Client(api, auth=not no_auth, token_url=s.get("PK_TOKEN_URL", W.DEFAULT_TOKEN_URL),
                  client_id=s.get("PK_WORKER_CLIENT_ID", "pk-worker"), secret=s.get("PK_WORKER_SECRET"))
    return cl.call("PUT", "/api/worker/profile/voice", {"text": text, "actor": "import:profile"})


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="python tools/profile/put_voice.py", description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--file", required=True, type=Path, help="the profile (markdown), outside the repo")
    ap.add_argument("--api", default=None, help="the API base (default: PK_API from ~/.punchable/secrets.env, else https://punchable.io)")
    ap.add_argument("--as", dest="who", choices=["worker", "owner"], default="worker")
    ap.add_argument("--no-auth", action="store_true", help="a loopback dev API with auth off (worker route, no token)")
    ap.add_argument("--dry-run", action="store_true", help="read and check the file, send nothing")
    a = ap.parse_args(argv)
    text = read_profile(a.file)
    print(f"voice profile: {describe(text)}; legal rules: clear")
    if a.dry_run:
        print("dry run: nothing sent")
        return 0
    out = put_as_owner(a.api or "https://punchable.io", text) if a.who == "owner" else put_as_worker(a.api, text, a.no_auth)
    print(f"stored: by {out.get('by')}, updated_at {out.get('updated_at')}, {out.get('chars'):,} chars"
          + (" (replaced the previous version)" if out.get("previous") else ""))
    print("the hosted worker reads it on its next job (its log: 'voice profile: loaded (...)'); Settings > Voice profile shows it")
    return 0


if __name__ == "__main__":
    sys.exit(main())
