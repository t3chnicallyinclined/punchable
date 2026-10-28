"""What the hosted API measures: `GET /metrics` (Prometheus text format; infra/OBSERVABILITY.md).

The route is open (no token) and reachable only inside the cluster: the public Ingress forwards `/api` and `/`
to the web pod, never `/metrics`, and the NetworkPolicy admits the observability namespace's pods to the API port.

Label discipline, the one rule that matters here: a label value is never the comic's material and never an id.
`route_pattern()` collapses every path segment that is not a plain lower-case word to `{id}`, so `/api/joke/x:abc12`
becomes `/api/joke/{id}` and a search query (`?q=...`) is dropped with the whole query string. Methods and statuses
are closed sets. engine/prom.py caps every metric at MAX_SERIES distinct label sets on top of that.

    pk_http_requests_total{method,route,status}          counter
    pk_http_request_duration_seconds{method,route}       histogram
    pk_http_requests_in_flight                           gauge (the WebSocket /api/live is not counted here)
    pk_live_clients                                      gauge: open /api/live sockets
    pk_jobs_queued{kind}                                 gauge, read at scrape time from the job store
    pk_jobs_waiting_transcriber                          gauge: queued jobs no worker can run (transcription = phone)
    pk_jobs_waiting_transcriber_oldest_seconds           gauge: age of the oldest of those (0 when none)
    pk_db_query_duration_seconds                         histogram: every SurrealDB round trip (db/sdb.py Rpc.query)
    pk_build_info{version,sha}                           gauge = 1
"""
from __future__ import annotations

import os
import re
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
if str(HERE.parent) not in sys.path:
    sys.path.insert(0, str(HERE.parent))
from engine import prom  # noqa: E402

_WORD = re.compile(r"^[a-z][a-z_-]{0,31}$")
_MAX_SEGMENTS = 6
VERSION = "0.1.0"

registry = prom.Registry()
http_requests = registry.counter("pk_http_requests_total", "HTTP requests by method, route pattern and status",
                                 ["method", "route", "status"])
http_duration = registry.histogram("pk_http_request_duration_seconds", "HTTP request latency by route pattern",
                                   ["method", "route"])
in_flight = registry.gauge("pk_http_requests_in_flight", "Requests being served right now (not /api/live)")
live_clients = registry.gauge("pk_live_clients", "Open /api/live WebSocket clients")
db_duration = registry.histogram("pk_db_query_duration_seconds", "SurrealDB round trips (Rpc.query)",
                                 buckets=(0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0))
build_info = registry.gauge("pk_build_info", "1, labelled with the version and the git sha of this image",
                            ["version", "sha"])
build_info.set(1, version=VERSION, sha=(os.environ.get("PK_BUILD_SHA") or "dev")[:12])

METHODS = ("GET", "HEAD", "POST", "PATCH", "DELETE", "OPTIONS")


def route_pattern(path: str) -> str:
    """`/api/joke/x:ab12?q=hi` -> `/api/joke/{id}`. Only plain lower-case words survive as themselves; anything with
    a digit, a colon, an upper-case letter, a dot or more than 32 characters is an id. Paths deeper than
    _MAX_SEGMENTS are cut. The query string is never looked at."""
    p = (path or "/").split("?", 1)[0].split("#", 1)[0]
    segs = [s for s in p.split("/") if s]
    out = []
    for s in segs[:_MAX_SEGMENTS]:
        out.append(s if _WORD.match(s) else "{id}")
    if len(segs) > _MAX_SEGMENTS:
        out.append("...")
    return "/" + "/".join(out) if out else "/"


def status_class(code) -> str:
    try:
        c = int(code)
    except (TypeError, ValueError):
        return "unknown"
    return str(c) if 100 <= c <= 599 else "unknown"


def record(method: str, path: str, status, seconds: float):
    m = method if method in METHODS else "OTHER"
    r = route_pattern(path)
    http_requests.inc(method=m, route=r, status=status_class(status))
    http_duration.observe(max(0.0, seconds), method=m, route=r)


def instrument(fn, skip=("/api/live", "/metrics")):
    """Wrap a BaseHTTPRequestHandler do_* method: time it, count it by route pattern and the status the handler
    sent (send_response is watched in server.Handler). Nested calls (do_HEAD -> do_GET) are counted once. The
    WebSocket (pk_live_clients) and the scrape itself are not app traffic and are left out."""
    def wrapped(self, *a, **k):
        path = (self.path or "/").split("?", 1)[0]
        if getattr(self, "_pk_timing", False) or path in skip:
            return fn(self, *a, **k)
        self._pk_timing = True
        self._pk_status = None
        in_flight.inc()
        t = time.monotonic()
        try:
            return fn(self, *a, **k)
        except BaseException:
            self._pk_status = self._pk_status or 500
            raise
        finally:
            in_flight.dec()
            record(self.command or "OTHER", path, self._pk_status or 0, time.monotonic() - t)
            self._pk_timing = False
    wrapped.__name__ = getattr(fn, "__name__", "do")
    wrapped.__wrapped__ = fn
    return wrapped


# ---------------------------------------------------------------- jobs (read at scrape time)
def _ts(s) -> float:
    """creative.util.now_iso(): tz-aware ISO seconds. Anything else counts as unknown (0)."""
    try:
        d = datetime.fromisoformat(str(s).replace("Z", "+00:00"))
        return (d if d.tzinfo else d.replace(tzinfo=timezone.utc)).timestamp()
    except (TypeError, ValueError):
        return 0.0


def job_stats(jobs) -> dict:
    """{queued: {kind: n}, waiting: n, oldest_waiting_s: float} from the job store's items (creative_api.job_apply)."""
    queued: dict[str, int] = {}
    waiting, oldest = 0, 0.0
    now = time.time()
    for j in jobs or ():
        if not isinstance(j, dict) or j.get("status") != "queued":
            continue
        kind = str(j.get("kind") or "unknown")
        queued[kind] = queued.get(kind, 0) + 1
        if j.get("waiting_for") == "transcriber":
            waiting += 1
            t = _ts(j.get("created_at"))
            if t:
                oldest = max(oldest, now - t)
    return {"queued": queued, "waiting": waiting, "oldest_waiting_s": oldest}


_jobs_source = None   # callable -> iterable of job dicts; server.py installs it once Handler.creative exists


def set_jobs_source(fn):
    global _jobs_source
    _jobs_source = fn


def _jobs_now():
    if _jobs_source is None:
        return []
    try:
        return list(_jobs_source())
    except Exception:  # noqa: BLE001  the scrape must not fail because the store is busy
        return []


jobs_queued = registry.gauge("pk_jobs_queued", "Queued jobs by kind", ["kind"],
                             collect=lambda: {(k,): n for k, n in job_stats(_jobs_now())["queued"].items()} or {})
jobs_waiting = registry.gauge("pk_jobs_waiting_transcriber", "Queued jobs waiting for a transcriber (the phone)",
                              collect=lambda: {(): job_stats(_jobs_now())["waiting"]})
jobs_waiting_oldest = registry.gauge("pk_jobs_waiting_transcriber_oldest_seconds",
                                     "Age of the oldest job waiting for a transcriber",
                                     collect=lambda: {(): job_stats(_jobs_now())["oldest_waiting_s"]})


# ---------------------------------------------------------------- DB latency
def instrument_rpc(cls) -> bool:
    """Wrap db/sdb.py Rpc.query once: every SurrealDB round trip lands in pk_db_query_duration_seconds."""
    if getattr(cls.query, "__pk_timed__", False):
        return False
    orig = cls.query

    def query(self, sql, vars=None):
        t = time.monotonic()
        try:
            return orig(self, sql, vars)
        finally:
            db_duration.observe(time.monotonic() - t)
    query.__pk_timed__ = True
    query.__wrapped__ = orig
    cls.query = query
    return True


def render() -> bytes:
    return registry.render().encode("utf-8")


# db/sdb.py is the one choke point every SurrealDB round trip goes through (surreal_kb, ingest, creative mirrors).
try:
    if str(HERE.parent / "db") not in sys.path:
        sys.path.insert(0, str(HERE.parent / "db"))
    import sdb as _sdb
    instrument_rpc(_sdb.Rpc)
except ImportError:  # the files backend alone: nothing to time
    pass


CONTENT_TYPE = prom.CONTENT_TYPE
