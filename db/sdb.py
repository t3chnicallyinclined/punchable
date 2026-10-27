"""sdb.py -- Punchable's SurrealDB 3.3.4 client: WebSocket RPC, CBOR only, guarded writes.

Rules (docs/CONTEXT.md "Storage decision, revised 2026-10-26"; docs/SURREALDB_AUDIT.md traps T1-T6):
- CBOR only. Over JSON-RPC, SurrealDB infers types from strings ("note: see doc" becomes a record id,
  "2015-02-18" a datetime). CBOR carries explicit types, so text stays text.
- Every RPC error raises, and every statement that is not OK raises. Nothing is swallowed.
- write() raises EmptyWrite when a write statement comes back empty: a denied CREATE/UPDATE/DELETE/RELATE
  returns `OK []` (T5), which must never look like a save.
- Change feeds are checked at startup (check_changefeeds): `DEFINE TABLE OVERWRITE` without the clause
  silently drops a feed (T6).
Credentials come from %USERPROFILE%\\.punchable\\secrets.env (never committed).
"""
from __future__ import annotations

import os
import socket
import subprocess
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

import cbor2
import websocket  # websocket-client

HERE = Path(__file__).resolve().parent
DESK = HERE.parent
SECRETS = Path(os.environ.get("PUNCH_SECRETS", Path.home() / ".punchable" / "secrets.env"))
SURREAL_EXE = Path(os.environ.get("PUNCH_SURREAL", Path.home() / ".punchable" / "bin" / "surreal.exe"))
COMEDY = Path(os.environ.get("PUNCH_COMEDY", os.path.expanduser("~/punchable-data")))
DATA_ROOT = COMEDY / "db" / "punchable"          # gitignored by comedy/db/.gitignore
STORE = DATA_ROOT / "data"                        # SurrealKV files
NS, DB = "punchable", "main"
MIN_VERSION = (3, 3, 1)                           # HNSW lives in RAM and is rebuilt on restart; identical-vector fix

# Tables that carry a change feed (the future phone sync pulls these). Checked at startup.
CHANGEFEED_TABLES = ["decision", "joke", "performance", "set", "label", "list", "performed_as"]


class EmptyWrite(RuntimeError):
    """A write statement returned no rows: denied, or its target does not exist."""


class SurrealError(RuntimeError):
    pass


def secrets() -> dict:
    out = {}
    if SECRETS.is_file():
        for line in SECRETS.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1)
                out[k.strip()] = v.strip()
    for k in ("SURREAL_USER", "SURREAL_PASS", "SURREAL_PORT", "SURREAL_URL", "SURREAL_API_USER", "SURREAL_API_PASS",
              "SURREAL_NS", "SURREAL_DB"):
        if os.environ.get(k):
            out[k] = os.environ[k]
    return out


def rpc_url(port_: int | None = None) -> str:
    """ws://127.0.0.1:<port>/rpc locally; SURREAL_URL (e.g. ws://surrealdb:8000/rpc in the cluster) when it is set
    and no explicit port was asked for."""
    if port_ is None and secrets().get("SURREAL_URL"):
        return secrets()["SURREAL_URL"]
    return f"ws://127.0.0.1:{port_ or port()}/rpc"


def db_user() -> tuple[str, str, str, str] | None:
    """(user, pass, ns, db) when a DATABASE-level user is configured (SURREAL_API_USER/PASS: the hosted API and
    dbload, infra/db-init.sh). That user is EDITOR on one database: it never defines namespaces or users."""
    s = secrets()
    if s.get("SURREAL_API_USER") and s.get("SURREAL_API_PASS"):
        return s["SURREAL_API_USER"], s["SURREAL_API_PASS"], s.get("SURREAL_NS", NS), s.get("SURREAL_DB", DB)
    return None


def port() -> int:
    return int(secrets().get("SURREAL_PORT", "8797"))


# ---------------------------------------------------------------------------
# CBOR (SurrealDB tags: 6 NONE, 8 record id, 12 compact datetime, 0 datetime string)
# ---------------------------------------------------------------------------
class RecordID:
    __slots__ = ("tb", "key")

    def __init__(self, tb: str, key):
        self.tb, self.key = tb, key

    def __repr__(self):
        return f"{self.tb}:{self.key!r}"

    def __eq__(self, o):
        return isinstance(o, RecordID) and (o.tb, _hk(o.key)) == (self.tb, _hk(self.key))

    def __hash__(self):
        return hash((self.tb, _hk(self.key)))


def _hk(k):
    return tuple(_hk(x) for x in k) if isinstance(k, list) else k


class _None:
    """SurrealDB NONE (absent), distinct from NULL (Python None)."""
    def __repr__(self):
        return "NONE"


NONE = _None()


def _default(enc, v):
    if isinstance(v, RecordID):
        enc.encode(cbor2.CBORTag(8, [v.tb, v.key]))
    elif isinstance(v, _None):
        enc.encode(cbor2.CBORTag(6, None))
    else:
        raise TypeError(f"can't CBOR-encode {type(v)}")


def _tag(tag, _immutable=False):
    if tag.tag == 8:
        tb, key = tag.value
        return RecordID(tb, key)
    if tag.tag == 6:
        return NONE
    if tag.tag == 12:
        s, ns = (list(tag.value) + [0, 0])[:2]
        return datetime.fromtimestamp(s + ns / 1e9, tz=timezone.utc)
    if tag.tag == 0:
        return str(tag.value)
    if tag.tag == 7:  # table name
        return tag.value
    return tag.value


def enc(o) -> bytes:
    return cbor2.dumps(o, default=_default, datetime_as_timestamp=False, timezone=timezone.utc)


def dec(b: bytes):
    return cbor2.loads(b, tag_hook=_tag)


def strip_none(o):
    """Drop NONE values (absent fields) recursively; keep NULL (None)."""
    if isinstance(o, dict):
        return {k: strip_none(v) for k, v in o.items() if not isinstance(v, _None)}
    if isinstance(o, list):
        return [strip_none(v) for v in o if not isinstance(v, _None)]
    return o


# ---------------------------------------------------------------------------
# RPC
# ---------------------------------------------------------------------------
class Rpc:
    """One WebSocket RPC connection. Thread-safe for request/response; live notifications are queued."""

    def __init__(self, port_: int | None = None, ns: str | None = NS, db: str | None = DB, signin: bool = True,
                 timeout: float = 300):
        self.url = rpc_url(port_)
        self.port = port_
        self.ws = websocket.create_connection(self.url, timeout=timeout, subprotocols=["cbor"])
        self.n = 0
        self.lock = threading.RLock()
        self.notifications: list = []
        self.on_notify = None  # callback(msg) for live notifications, if set
        self.ns, self.db = ns, db
        self.db_user = None
        if signin:
            du = db_user() if port_ is None else None
            if du:
                # A database-level user (EDITOR on one ns/db). It signs in to that database and stays in it, so an
                # Rpc(ns=None, db=None) still lands there.
                user, pw, dns, ddb = du
                self.call("signin", [{"user": user, "pass": pw, "ns": dns, "db": ddb}])
                self.db_user = user
                self.ns, self.db = ns or dns, db or ddb
                self.call("use", [self.ns, self.db])
            else:
                s = secrets()
                if not s.get("SURREAL_PASS"):
                    raise SurrealError(f"no SURREAL_PASS in {SECRETS}")
                self.call("signin", [{"user": s.get("SURREAL_USER", "root"), "pass": s["SURREAL_PASS"]}])
                if ns and db:
                    self.call("use", [ns, db])

    def use(self, ns: str, db: str):
        self.ns, self.db = ns, db
        self.call("use", [ns, db])

    def call(self, method: str, params: list):
        with self.lock:
            self.n += 1
            my = self.n
            self.ws.send_binary(enc({"id": my, "method": method, "params": params}))
            while True:
                msg = dec(self.ws.recv())
                if msg.get("id") is None:  # live notification
                    self._notify(msg)
                    continue
                if msg["id"] != my:
                    continue
                if "error" in msg:
                    raise SurrealError(f"{method} failed: {msg['error']} :: {str(params)[:300]}")
                return msg["result"]

    def _notify(self, msg):
        if self.on_notify:
            self.on_notify(msg)
        else:
            self.notifications.append(msg)

    def query(self, sql: str, vars: dict | None = None) -> list:
        """All statements; raises if any statement failed."""
        res = self.call("query", [sql, vars or {}])
        for i, r in enumerate(res):
            if r.get("status") != "OK":
                raise SurrealError(f"statement {i} failed: {r.get('result')}\n--- {sql[:600]}")
        return res

    def last(self, sql: str, vars: dict | None = None):
        return self.query(sql, vars)[-1]["result"]

    def write(self, sql: str, vars: dict | None = None, expect: int | None = None):
        """A write whose LAST statement must return rows (>= 1, or exactly `expect`). Empty -> EmptyWrite.

        Use RETURN AFTER / RETURN id (never RETURN NONE) on guarded writes."""
        res = self.last(sql, vars)
        rows = res if isinstance(res, list) else ([res] if res not in (None, NONE) else [])
        rows = [r for r in rows if r not in (None, NONE)]
        if not rows:
            raise EmptyWrite(f"write returned nothing (denied, or the target doesn't exist):\n--- {sql[:400]}")
        if expect is not None and len(rows) != expect:
            raise EmptyWrite(f"write returned {len(rows)} rows, expected {expect}:\n--- {sql[:400]}")
        return rows

    def recv_live(self, timeout: float) -> dict | None:
        """Block (outside any request) for one live notification."""
        if self.notifications:
            return self.notifications.pop(0)
        with self.lock:
            self.ws.settimeout(timeout)
            try:
                while True:
                    msg = dec(self.ws.recv())
                    if msg.get("id") is None:
                        return msg
            except websocket.WebSocketTimeoutException:
                return None
            finally:
                self.ws.settimeout(300)

    def close(self):
        try:
            self.ws.close()
        except Exception:  # noqa: BLE001
            pass


def server_version(p: int | None = None) -> tuple[int, int, int]:
    r = Rpc(p, ns=None, db=None, signin=False)
    try:
        v = str(r.call("version", []))
    finally:
        r.close()
    nums = [int(x) for x in v.split("-")[-1].split("+")[0].split(".")[:3] if x.isdigit()] if "-" in v else \
        [int(x) for x in v.split(".")[:3]]
    return tuple(nums)  # type: ignore[return-value]


def check_version(p: int | None = None):
    v = server_version(p)
    if v < MIN_VERSION:
        raise SurrealError(f"SurrealDB {'.'.join(map(str, v))} is older than {'.'.join(map(str, MIN_VERSION))}: "
                           "the vector index needs >= 3.3.1")
    return v


def check_changefeeds(r: Rpc, tables=CHANGEFEED_TABLES) -> dict:
    """{table: definition} for tables whose feed is missing. Empty dict = all present."""
    info = r.last("INFO FOR DB;")
    tbs = info.get("tables", {}) if isinstance(info, dict) else {}
    missing = {}
    for t in tables:
        d = str(tbs.get(t, ""))
        if "CHANGEFEED" not in d.upper():
            missing[t] = d or "(table missing)"
    return missing


def is_up(p: int | None = None) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", p or port()), timeout=0.3):
            return True
    except OSError:
        return False


def start_scratch(port_: int, store: str = "memory", log_path: Path | None = None):
    """A throwaway server for experiments (memory store). Not used for the real database."""
    s = secrets()
    env = {**os.environ, "SURREAL_USER": s.get("SURREAL_USER", "root"), "SURREAL_PASS": s["SURREAL_PASS"]}
    out = open(log_path, "w") if log_path else subprocess.DEVNULL
    proc = subprocess.Popen([str(SURREAL_EXE), "start", "--log", "warn", "--bind", f"127.0.0.1:{port_}", store],
                            stdout=out, stderr=subprocess.STDOUT, env=env)
    for _ in range(300):
        if is_up(port_):
            break
        if proc.poll() is not None:
            raise SurrealError(f"surreal exited {proc.returncode}")
        time.sleep(0.1)
    time.sleep(0.2)
    return proc


# ---------------------------------------------------------------------------
# ids: KB string ids <-> record ids
# ---------------------------------------------------------------------------
NODE_TABLES = {"joke": "joke", "unit": "unit", "set": "set", "perf": "performance", "label": "label", "list": "list"}
TABLE_PREFIX = {v: k for k, v in NODE_TABLES.items()}
TYPE_OF_TABLE = {"joke": "joke", "unit": "unit", "set": "set", "performance": "performance", "label": "label", "list": "list"}
EDGE_TABLES = ["part_of", "variant_of", "performed_as", "tag_of", "same_shape", "callback_to",
               "same_premise", "has_label", "segue", "custom"]


def node_rid(nid: str) -> RecordID:
    prefix, key = nid.split(":", 1)
    if prefix not in NODE_TABLES:
        raise ValueError(f"node id {nid!r}: unknown prefix {prefix!r} (add a table to db/schema.surql)")
    return RecordID(NODE_TABLES[prefix], key)


def rid_str(r) -> str:
    if isinstance(r, RecordID):
        if r.tb in TABLE_PREFIX:
            return f"{TABLE_PREFIX[r.tb]}:{r.key}"
        return f"{r.tb}:{r.key}"
    return str(r)


def edge_key(eid: str) -> str:
    if not eid.startswith("e:"):
        raise ValueError(f"edge id {eid!r} is not e:<hash>")
    return eid[2:]
