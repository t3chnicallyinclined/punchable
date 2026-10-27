"""Punchable home worker: an outbound-only loop on the comic's PC.

    python -m worker                      # forever: pull phone decisions + creative edits, run queued jobs
    python -m worker --once               # one pass, then exit (a scheduled task can run this every few minutes)
    python -m worker --api http://127.0.0.1:8796 --no-auth     # against the local dev API (loopback, auth off)

It only ever makes HTTPS requests out (Keycloak for a token, punchable.io for jobs); nothing listens.
- Token: Keycloak client credentials as `pk-worker` (confidential, realm role `worker`). The secret is read from
  %USERPROFILE%\\.punchable\\secrets.env (PK_WORKER_SECRET=...). It is never printed, logged or written elsewhere.
- Decisions made on the phone are appended to comedy/kb/v0/decisions.jsonl: append-only, fsync, deduplicated by the
  line's content hash (the id of a decision line), `undo_of` remapped to the local line numbers.
- Creative edits (storyboards, working sets, venue pins) are appended to the local jsonl when the local file is a
  prefix of the server's; a divergence is reported and left alone.
- Jobs: claim -> run through engine.providers (claude_code / ollama, per engine/config.toml) -> post the result.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

DESK = Path(__file__).resolve().parent.parent
if str(DESK) not in sys.path:
    sys.path.insert(0, str(DESK))

SECRETS = Path(os.environ.get("PUNCH_SECRETS", Path.home() / ".punchable" / "secrets.env"))
DEFAULT_API = "https://punchable.io"
DEFAULT_TOKEN_URL = "https://auth.punchable.io/realms/punchable/protocol/openid-connect/token"
UA = "punchable-worker/1"


def log(*a):
    print(time.strftime("%Y-%m-%d %H:%M:%S"), *a, flush=True)


def secrets() -> dict:
    out = {}
    if SECRETS.is_file():
        for line in SECRETS.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1)
                out[k.strip()] = v.strip()
    for k in ("PK_WORKER_SECRET", "PK_WORKER_CLIENT_ID", "PK_TOKEN_URL", "PK_API"):
        if os.environ.get(k):
            out[k] = os.environ[k]
    return out


class Client:
    def __init__(self, api: str, auth: bool = True, token_url: str | None = None, client_id: str = "pk-worker",
                 secret: str | None = None, opener=None):
        self.api = api.rstrip("/")
        self.auth = auth
        self.token_url, self.client_id, self._secret = token_url, client_id, secret
        self._tok, self._exp = None, 0.0
        self.open = opener or urllib.request.urlopen
        if auth and not secret:
            raise SystemExit(f"no PK_WORKER_SECRET in {SECRETS} (see worker/README.md)")

    def token(self) -> str:
        if self._tok and time.time() < self._exp - 30:
            return self._tok
        body = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": self.client_id,
                                       "client_secret": self._secret}).encode()
        req = urllib.request.Request(self.token_url, data=body, method="POST",
                                     headers={"Content-Type": "application/x-www-form-urlencoded", "User-Agent": UA})
        try:
            with self.open(req, timeout=30) as r:
                d = json.loads(r.read())
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"Keycloak refused the worker's client credentials (HTTP {e.code})") from None
        self._tok, self._exp = d["access_token"], time.time() + float(d.get("expires_in", 60))
        return self._tok

    def call(self, method: str, path: str, body=None, raw: bool = False):
        h = {"User-Agent": UA, "Accept": "application/json"}
        if self.auth:
            h["Authorization"] = "Bearer " + self.token()
        data = None
        if body is not None:
            data = json.dumps(body).encode()
            h["Content-Type"] = "application/json"
        req = urllib.request.Request(self.api + path, data=data, method=method, headers=h)
        try:
            with self.open(req, timeout=120) as r:
                if r.status == 204:
                    return None
                return r if raw else json.loads(r.read() or b"null")
        except urllib.error.HTTPError as e:
            msg = e.read()[:300].decode("utf-8", "replace")
            if e.code == 401:
                self._tok = None
            raise RuntimeError(f"{method} {path}: HTTP {e.code} {msg}") from None

    def download(self, path: str, dest: Path):
        h = {"User-Agent": UA}
        if self.auth:
            h["Authorization"] = "Bearer " + self.token()
        req = urllib.request.Request(self.api + path, headers=h)
        tmp = dest.with_name(dest.name + ".part")
        with self.open(req, timeout=600) as r, open(tmp, "wb") as f:
            while True:
                chunk = r.read(1024 * 1024)
                if not chunk:
                    break
                f.write(chunk)
        os.replace(tmp, dest)


# ---------------------------------------------------------------- pulls (append-only)
def canon(d: dict) -> str:
    return json.dumps(d, ensure_ascii=False, separators=(",", ":"))


def h(d: dict) -> str:
    """A decision line's identity: the hash of its content with sorted keys (the database returns the same line with
    its keys in another order)."""
    return hashlib.sha256(json.dumps(d, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")).hexdigest()


def read_jsonl(p: Path) -> list[dict]:
    out = []
    if p.is_file():
        with open(p, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line:
                    try:
                        x = json.loads(line)
                    except ValueError:
                        continue
                    if isinstance(x, dict):
                        out.append(x)
    return out


def append_lines(p: Path, lines: list[str]):
    """Append-only, never glued onto a torn last line, fsynced (kb.py's append)."""
    if not lines:
        return
    p.parent.mkdir(parents=True, exist_ok=True)
    with open(p, "ab") as f:
        if f.tell() > 0:
            with open(p, "rb") as r:
                r.seek(-1, os.SEEK_END)
                if r.read(1) != b"\n":
                    f.write(b"\n")
        for line in lines:
            assert "\n" not in line
            f.write(line.encode("utf-8") + b"\n")
        f.flush()
        os.fsync(f.fileno())


def merge_decisions(local: list[dict], server: list[dict]) -> tuple[list[dict], list[str]]:
    """Server lines that are not already local (by content hash), in server order, with `undo_of` (a line number)
    remapped from the server's numbering to the local file's. Returns (new lines, warnings)."""
    pos = {h(d): i for i, d in enumerate(local)}
    srv_hash = [h(d) for d in server]
    new, warn = [], []
    n = len(local)
    for i, d in enumerate(server):
        if srv_hash[i] in pos:
            continue
        d = dict(d)
        if isinstance(d.get("undo_of"), int):
            tgt = d["undo_of"]
            if not (0 <= tgt < len(server)) or srv_hash[tgt] not in pos:
                warn.append(f"server line {i}: undo_of {tgt} has no local line; skipped")
                continue
            d["undo_of"] = pos[srv_hash[tgt]]
        pos[srv_hash[i]] = n
        n += 1
        new.append(d)
    return new, warn


def pull_decisions(cl: Client, path: Path) -> int:
    server, after = [], 0
    while True:
        page = cl.call("GET", f"/api/worker/decisions?after={after}")
        server += [x["decision"] for x in page["lines"]]
        after += len(page["lines"])
        if not page["lines"] or after >= page["total"]:
            break
    new, warn = merge_decisions(read_jsonl(path), server)
    for w in warn:
        log("decisions:", w)
    append_lines(path, [canon(d) for d in new])
    return len(new)


def pull_creative(cl: Client, cdir: Path) -> dict:
    out = {}
    for name in ("storyboards", "workingsets", "venues"):
        srv = cl.call("GET", f"/api/worker/creative?store={name}")["lines"]
        p = cdir / f"{name}.jsonl"
        loc = []
        if p.is_file():
            loc = [x.rstrip("\r\n") for x in p.read_text(encoding="utf-8").splitlines() if x.strip()]
        if srv[:len(loc)] == loc:
            append_lines(p, srv[len(loc):])
            out[name] = len(srv) - len(loc)  # the snapshot is derived: creative's Store replays the log on load
            continue
        have = set(loc)
        new = [x for x in srv if x not in have]
        if not new:
            out[name] = 0  # local is ahead: the next dbload pushes it
        elif replays(name, loc + new):
            append_lines(p, new)  # both sides added events: his phone's go after the PC's, if that still replays
            out[name] = len(new)
        else:
            log(f"creative {name}: {len(new)} server event(s) do not replay after the local ones; left alone "
                "(resolve by hand, e.g. undo the local edit)")
            out[name] = None
    return out


def replays(name: str, lines: list[str]) -> bool:
    sys.path.insert(0, str(DESK / "api-dev"))
    import creative_api as CR
    items: dict = {}
    try:
        for line in lines:
            CR.APPLY[name](items, json.loads(line))
        return True
    except (ValueError, KeyError, TypeError):
        return False


# ---------------------------------------------------------------- jobs
def run_one(cl: Client, kinds: list[str] | None, ws=None) -> bool:
    """Claim one job and run it. `ws` (worker/online.Workspace): online-first mode, the job runs on a workspace
    mirrored from the API, its results are posted back and its media deleted."""
    from worker import execute
    r = cl.call("POST", "/api/worker/claim", {"kinds": kinds, "worker": os.environ.get("COMPUTERNAME", "home")[:40]})
    if not r:
        return False
    job = r["job"]
    log(f"job {job['id']} ({job['kind']})")
    file = None
    try:
        if job["kind"] == "process_set":
            if ws is not None:
                inbox = ws.root / "upload"
            else:
                from engine.config import Config
                inbox = Config().p("inbox_dir")
            inbox.mkdir(parents=True, exist_ok=True)
            file = inbox / Path(job["input"]["file"]).name
            cl.download(f"/api/worker/jobs/{urllib.parse.quote(job['id'])}/file", file)
        if ws is not None:
            from worker import online
            try:
                res = online.run_job(cl, ws, job, file=file)
            finally:
                if file is not None:
                    file.unlink(missing_ok=True)  # the API deletes its staged copy when it gets our result
        else:
            res = execute.run(job, file=file)
        cl.call("POST", f"/api/worker/jobs/{urllib.parse.quote(job['id'])}/result", {"ok": True, "result": res})
        log(f"job {job['id']} done ({res.get('status') if isinstance(res, dict) else 'ok'})")
    except Exception as e:  # noqa: BLE001
        log(f"job {job['id']} failed: {type(e).__name__}: {e}")
        cl.call("POST", f"/api/worker/jobs/{urllib.parse.quote(job['id'])}/result",
                {"ok": False, "error": f"{type(e).__name__}: {e}"[:1500], "retry": isinstance(e, (OSError, TimeoutError))})
    return True


def main(argv=None):
    s = secrets()
    ap = argparse.ArgumentParser(prog="python -m worker", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--api", default=s.get("PK_API", DEFAULT_API))
    ap.add_argument("--token-url", default=s.get("PK_TOKEN_URL", DEFAULT_TOKEN_URL))
    ap.add_argument("--no-auth", action="store_true", help="the local dev API on loopback (auth off)")
    ap.add_argument("--once", action="store_true")
    ap.add_argument("--kinds", default="bridge,punchup,ask,process_set,import_youtube,facts,riff,ask_chat,embed,memolink")
    ap.add_argument("--online", action="store_true", default=os.environ.get("PK_ONLINE_FIRST") == "1",
                    help="online-first: run jobs on a workspace mirrored from the API (worker/online.py), post the "
                         "results, keep nothing; no pulls into the data folder (also PK_ONLINE_FIRST=1)")
    ap.add_argument("--idle", type=float, default=15.0, help="seconds between polls when there is nothing to do")
    ap.add_argument("--no-pull", action="store_true", help="do not pull decisions / creative edits")
    a = ap.parse_args(argv)
    if a.no_auth and not urllib.parse.urlparse(a.api).hostname in ("127.0.0.1", "localhost"):
        raise SystemExit("--no-auth is only for the loopback dev API")
    cl = Client(a.api, auth=not a.no_auth, token_url=a.token_url, client_id=s.get("PK_WORKER_CLIENT_ID", "pk-worker"),
                secret=s.get("PK_WORKER_SECRET"))
    ws = None
    if a.online:
        from worker import online
        ws = online.Workspace(cl)
        ws.activate()  # before creative/ is imported: its caches and data folder are the workspace's
        a.no_pull = True  # nothing is pulled into a data folder: the database is the truth
        log(f"online-first: workspace {ws.root}")
    from creative.data import comedy_dir, creative_dir
    decisions = comedy_dir() / "kb" / "v0" / "decisions.jsonl"
    kinds = [k for k in a.kinds.split(",") if k]
    log(f"worker up: {a.api} ({'auth' if not a.no_auth else 'no auth'}), kinds {kinds}")
    backoff, last_pull = 5, 0.0
    while True:
        try:
            if not a.no_pull and time.time() - last_pull > 60:
                n = pull_decisions(cl, decisions)
                c = pull_creative(cl, creative_dir())
                if n or any(c.values()):
                    log(f"pulled {n} decision line(s), creative {c}")
                last_pull = time.time()
            busy = run_one(cl, kinds, ws)
            backoff = 5
            if a.once and not busy:
                return 0
            if not busy:
                time.sleep(a.idle)
        except KeyboardInterrupt:
            return 0
        except Exception as e:  # noqa: BLE001
            log(f"error: {e}")
            if a.once:
                return 1
            time.sleep(backoff)
            backoff = min(backoff * 2, 300)


if __name__ == "__main__":
    sys.exit(main())
