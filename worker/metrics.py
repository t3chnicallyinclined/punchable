"""What the hosted worker measures: `GET /metrics` on a side port (PK_METRICS_PORT, default 9100; infra/k8s/35-worker.yaml
names it `metrics` and the ServiceMonitor scrapes it). The worker is otherwise outbound-only; this listener answers
`/metrics` and `/healthz` and nothing else, and the NetworkPolicy admits only the observability namespace to it.

    pk_worker_jobs_claimed_total{kind}                   counter
    pk_worker_jobs_done_total{kind}                      counter
    pk_worker_jobs_failed_total{kind,retry}              counter (retry="true" = handed back to the queue)
    pk_worker_job_duration_seconds{kind}                 histogram
    pk_ai_spend_usd_today                                gauge, from engine/budget.py at scrape time
    pk_ai_budget_usd                                     gauge: PK_AI_DAILY_USD
    pk_ai_paused                                         gauge: 1 while today's cap is spent
    pk_ai_calls_total{model}                             counter
    pk_ai_tokens_total{model,direction}                  counter: input | output | cache_read | cache_write
    pk_ai_spend_usd_total{model,task}                    counter (USD, this process's lifetime)
    pk_worker_heartbeat_age_seconds                      gauge: seconds since the last heartbeat touch
    pk_worker_build_info{version,sha}                    gauge = 1

Labels are job kinds, model ids and task names: never the comic's material, never a job id."""
from __future__ import annotations

import os
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

DESK = Path(__file__).resolve().parent.parent
if str(DESK) not in sys.path:
    sys.path.insert(0, str(DESK))
from engine import prom  # noqa: E402

VERSION = "0.1.0"
JOB_BUCKETS = (1, 2.5, 5, 10, 20, 30, 60, 120, 300, 600, 900, 1800)

registry = prom.Registry()
jobs_claimed = registry.counter("pk_worker_jobs_claimed_total", "Jobs claimed, by kind", ["kind"])
jobs_done = registry.counter("pk_worker_jobs_done_total", "Jobs finished with a result, by kind", ["kind"])
jobs_failed = registry.counter("pk_worker_jobs_failed_total", "Jobs that failed (retry=true went back to the queue)",
                               ["kind", "retry"])
job_duration = registry.histogram("pk_worker_job_duration_seconds", "Job wall time by kind", ["kind"],
                                  buckets=JOB_BUCKETS)
ai_calls = registry.counter("pk_ai_calls_total", "Model calls by model", ["model"])
ai_tokens = registry.counter("pk_ai_tokens_total", "Tokens by model and direction", ["model", "direction"])
ai_spend_total = registry.counter("pk_ai_spend_usd_total", "USD charged since this process started", ["model", "task"])
build_info = registry.gauge("pk_worker_build_info", "1, labelled with the version and git sha of this image",
                            ["version", "sha"])
build_info.set(1, version=VERSION, sha=(os.environ.get("PK_BUILD_SHA") or "dev")[:12])

_budget = None
_heartbeat_at: float | None = None
_lock = threading.Lock()


def _budget_status() -> dict:
    b = _budget
    if b is None:
        return {}
    try:
        return b.status()
    except Exception:  # noqa: BLE001
        return {}


ai_spend_today = registry.gauge("pk_ai_spend_usd_today", "USD spent today (UTC) per engine/budget.py",
                                collect=lambda: {(): float(_budget_status().get("spent_usd") or 0.0)})
ai_budget = registry.gauge("pk_ai_budget_usd", "Today's cap, PK_AI_DAILY_USD",
                           collect=lambda: {(): float(_budget_status().get("daily_usd") or 0.0)})
ai_paused = registry.gauge("pk_ai_paused", "1 while the worker takes no job because today's cap is spent",
                           collect=lambda: {(): 1 if _budget_status().get("over") else 0})
heartbeat_age = registry.gauge("pk_worker_heartbeat_age_seconds", "Seconds since the loop last touched its heartbeat",
                               collect=lambda: {(): (time.time() - _heartbeat_at) if _heartbeat_at else float("nan")})


def model_label(model) -> str:
    """A model id is fine as a label (a closed set of a few names); anything odd collapses to `unknown`."""
    m = str(model or "").strip().lower()
    return m if m and len(m) <= 64 and all(c.isalnum() or c in "-._:" for c in m) else "unknown"


def _n(usage, name) -> int:
    v = getattr(usage, name, None) if not isinstance(usage, dict) else usage.get(name)
    try:
        return int(v or 0)
    except (TypeError, ValueError):
        return 0


def on_charge(model, usage, usd: float, task: str):
    """engine.budget.Budget.listeners entry: tokens and USD per model."""
    m = model_label(model)
    ai_calls.inc(model=m)
    for direction, field in (("input", "input_tokens"), ("output", "output_tokens"),
                             ("cache_read", "cache_read_input_tokens"), ("cache_write", "cache_creation_input_tokens")):
        n = _n(usage, field)
        if n:
            ai_tokens.inc(n, model=m, direction=direction)
    t = str(task or "?")
    ai_spend_total.inc(max(0.0, float(usd or 0.0)), model=m, task=t if len(t) <= 40 else "other")


def attach_budget(budget):
    """Read the spend gauges from this Budget at scrape time and count tokens on every charge."""
    global _budget
    _budget = budget
    if budget is not None and on_charge not in budget.listeners:
        budget.listeners.append(on_charge)


def heartbeat():
    global _heartbeat_at
    _heartbeat_at = time.time()


def kind_label(kind) -> str:
    k = str(kind or "unknown")
    return k if k.replace("_", "").isalnum() and len(k) <= 32 else "other"


class JobTimer:
    """with JobTimer(kind) as t: ...; t.done() | t.failed(retry)   (claimed is counted at enter)."""

    def __init__(self, kind):
        self.kind = kind_label(kind)
        self.t0 = time.monotonic()
        self._closed = False
        jobs_claimed.inc(kind=self.kind)

    def _close(self):
        if not self._closed:
            self._closed = True
            job_duration.observe(time.monotonic() - self.t0, kind=self.kind)

    def done(self):
        self._close()
        jobs_done.inc(kind=self.kind)

    def failed(self, retry: bool = False):
        self._close()
        jobs_failed.inc(kind=self.kind, retry="true" if retry else "false")


def render() -> bytes:
    return registry.render().encode("utf-8")


# ---------------------------------------------------------------- the side listener
class _Handler(BaseHTTPRequestHandler):
    server_version = "punchable-worker-metrics/0"

    def log_message(self, fmt, *args):   # a scrape every 30 s is not a log line
        return

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path == "/metrics":
            b = render()
            self.send_response(200)
            self.send_header("Content-Type", prom.CONTENT_TYPE)
        elif path == "/healthz":
            age = (time.time() - _heartbeat_at) if _heartbeat_at else None
            ok = age is not None and age < 20 * 60
            b = (f"ok heartbeat {age:.0f}s ago\n" if ok and age is not None else "no recent heartbeat\n").encode()
            self.send_response(200 if ok else 503)
            self.send_header("Content-Type", "text/plain; charset=utf-8")
        else:
            b = b"not found\n"
            self.send_response(404)
            self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(b)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(b)


def serve(port: int | None = None, host: str = "0.0.0.0") -> ThreadingHTTPServer | None:
    """Start the listener in a daemon thread. port None -> PK_METRICS_PORT (default 9100); 0 = off."""
    if port is None:
        try:
            port = int(os.environ.get("PK_METRICS_PORT", "9100"))
        except ValueError:
            port = 9100
    if port <= 0:
        return None
    srv = ThreadingHTTPServer((host, port), _Handler)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True, name="metrics").start()
    return srv
