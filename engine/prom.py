"""A small Prometheus registry (text exposition format 0.0.4), standard library only.

Used by the hosted API (api-dev/metrics.py) and the hosted worker (worker/metrics.py); both images are wheels-only
and tiny, and the whole surface we need is counters, gauges and histograms with a handful of labels. Nothing here
knows what a joke is: label values are route patterns, job kinds, model ids and status codes, never the comic's
material. Every label value is capped (MAX_SERIES per metric) so a mistake upstream becomes an `other` bucket, not an
unbounded series set.

    reg = Registry()
    req = reg.counter("pk_http_requests_total", "Requests", ["method", "route", "status"])
    req.inc(method="GET", route="/api/health", status="200")
    lat = reg.histogram("pk_http_request_duration_seconds", "Latency", ["route"], buckets=DEFAULT_BUCKETS)
    lat.observe(0.012, route="/api/health")
    g = reg.gauge("pk_live_clients", "Open /api/live sockets")
    g.set(3); g.inc(); g.dec()
    reg.gauge("pk_jobs_queued", "Queued jobs", ["kind"], collect=lambda: {("riff",): 2})   # read at scrape time
    text = reg.render()        # str, Content-Type: text/plain; version=0.0.4; charset=utf-8
"""
from __future__ import annotations

import math
import threading
from collections.abc import Callable, Iterable

DEFAULT_BUCKETS = (0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0)
MAX_SERIES = 200           # distinct label sets per metric; beyond it the label set collapses to OVERFLOW
OVERFLOW = "other"
CONTENT_TYPE = "text/plain; version=0.0.4; charset=utf-8"

_NAME_OK = __import__("re").compile(r"^[a-zA-Z_:][a-zA-Z0-9_:]*$")
_LABEL_OK = __import__("re").compile(r"^[a-zA-Z_][a-zA-Z0-9_]*$")


def _escape(v: str) -> str:
    return str(v).replace("\\", "\\\\").replace("\n", "\\n").replace('"', '\\"')


def _fmt(v: float) -> str:
    if isinstance(v, bool):
        return "1" if v else "0"
    if isinstance(v, int):
        return str(v)
    if math.isnan(v):
        return "NaN"
    if math.isinf(v):
        return "+Inf" if v > 0 else "-Inf"
    r = repr(float(v))
    return r[:-2] if r.endswith(".0") else r


class _Metric:
    kind = "untyped"

    def __init__(self, name: str, help_: str, labels: Iterable[str] = ()):
        if not _NAME_OK.match(name):
            raise ValueError(f"bad metric name {name!r}")
        self.name, self.help, self.labels = name, help_, tuple(labels)
        for lb in self.labels:
            if not _LABEL_OK.match(lb) or lb.startswith("__"):
                raise ValueError(f"bad label name {lb!r}")
        self._lock = threading.Lock()
        self._series: dict[tuple, object] = {}

    def _key(self, kw: dict) -> tuple:
        if set(kw) != set(self.labels):
            raise ValueError(f"{self.name}: labels {sorted(kw)} != {sorted(self.labels)}")
        key = tuple(str(kw[lb]) for lb in self.labels)
        if key not in self._series and self.labels:
            over = tuple(OVERFLOW for _ in self.labels)
            # the overflow series counts toward the cap, so a metric never holds more than MAX_SERIES series
            if len(self._series) >= MAX_SERIES - (0 if over in self._series else 1):
                key = over
        return key

    def _line(self, suffix: str, key: tuple, value, extra: dict | None = None) -> str:
        pairs = [f'{lb}="{_escape(v)}"' for lb, v in zip(self.labels, key)]
        pairs += [f'{k}="{_escape(v)}"' for k, v in (extra or {}).items()]
        lbl = "{" + ",".join(pairs) + "}" if pairs else ""
        return f"{self.name}{suffix}{lbl} {_fmt(value)}"

    def header(self) -> list[str]:
        return [f"# HELP {self.name} {_escape(self.help)}", f"# TYPE {self.name} {self.kind}"]

    def series_count(self) -> int:
        with self._lock:
            return len(self._series)


class Counter(_Metric):
    kind = "counter"

    def inc(self, n: float = 1, **labels):
        if n < 0:
            raise ValueError("a counter only goes up")
        with self._lock:
            k = self._key(labels)
            self._series[k] = self._series.get(k, 0) + n

    def value(self, **labels) -> float:
        with self._lock:
            return self._series.get(self._key(labels), 0)

    def render(self) -> list[str]:
        with self._lock:
            return self.header() + [self._line("", k, v) for k, v in sorted(self._series.items())]


class Gauge(_Metric):
    """A value that goes up and down. `collect` (-> {label tuple: value}) replaces the stored values at scrape time
    for things that are cheaper to read than to track (queue depths, a file's age, today's spend)."""
    kind = "gauge"

    def __init__(self, name, help_, labels=(), collect: Callable[[], dict[tuple, float]] | None = None):
        super().__init__(name, help_, labels)
        self.collect = collect

    def set(self, v: float, **labels):
        with self._lock:
            self._series[self._key(labels)] = v

    def inc(self, n: float = 1, **labels):
        with self._lock:
            k = self._key(labels)
            self._series[k] = self._series.get(k, 0) + n

    def dec(self, n: float = 1, **labels):
        self.inc(-n, **labels)

    def value(self, **labels) -> float:
        with self._lock:
            return self._series.get(self._key(labels), 0)

    def render(self) -> list[str]:
        if self.collect is not None:
            try:
                got = self.collect()
            except Exception:  # noqa: BLE001  a collector that fails must not take the scrape down
                got = None
            if got is not None:
                with self._lock:
                    self._series = {}
                    for k, v in got.items():
                        k = (k,) if not isinstance(k, tuple) else k
                        self._series[self._key(dict(zip(self.labels, k)))] = v
        with self._lock:
            return self.header() + [self._line("", k, v) for k, v in sorted(self._series.items())]


class Histogram(_Metric):
    kind = "histogram"

    def __init__(self, name, help_, labels=(), buckets: Iterable[float] = DEFAULT_BUCKETS):
        super().__init__(name, help_, labels)
        self.buckets = tuple(sorted(float(b) for b in buckets))
        if not self.buckets:
            raise ValueError("a histogram needs buckets")

    def observe(self, v: float, **labels):
        with self._lock:
            k = self._key(labels)
            s = self._series.get(k)
            if s is None:
                s = self._series[k] = [[0] * len(self.buckets), 0, 0.0]   # bucket counts, count, sum
            for i, b in enumerate(self.buckets):
                if v <= b:
                    s[0][i] += 1
            s[1] += 1
            s[2] += v

    def count(self, **labels) -> int:
        with self._lock:
            s = self._series.get(self._key(labels))
            return s[1] if s else 0

    def render(self) -> list[str]:
        out = self.header()
        with self._lock:
            for k, (counts, n, total) in sorted(self._series.items()):
                for b, c in zip(self.buckets, counts):
                    out.append(self._line("_bucket", k, c, {"le": _fmt(b)}))
                out.append(self._line("_bucket", k, n, {"le": "+Inf"}))
                out.append(self._line("_sum", k, total))
                out.append(self._line("_count", k, n))
        return out


class Registry:
    def __init__(self):
        self._metrics: dict[str, _Metric] = {}
        self._lock = threading.Lock()

    def _add(self, m: _Metric):
        with self._lock:
            if m.name in self._metrics:
                raise ValueError(f"metric {m.name} is already registered")
            self._metrics[m.name] = m
        return m

    def counter(self, name, help_, labels=()) -> Counter:
        c = self._add(Counter(name, help_, labels))
        if not c.labels:
            c.inc(0)   # a label-less counter exists from the start: `0`, not a missing series
        return c

    def gauge(self, name, help_, labels=(), collect=None) -> Gauge:
        g = self._add(Gauge(name, help_, labels, collect))
        if not g.labels and collect is None:
            g.set(0)
        return g

    def histogram(self, name, help_, labels=(), buckets=DEFAULT_BUCKETS) -> Histogram:
        return self._add(Histogram(name, help_, labels, buckets))

    def get(self, name: str):
        return self._metrics.get(name)

    def render(self) -> str:
        with self._lock:
            ms = list(self._metrics.values())
        lines: list[str] = []
        for m in ms:
            lines += m.render()
        return "\n".join(lines) + "\n"
