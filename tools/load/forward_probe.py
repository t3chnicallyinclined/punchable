"""Is a forwarded route slower behind pk-api, and whose fault is it? (T9 round 6.)

Runs against a pair started by `tools/load/scratch_api.py --rust <pk-api> --hold <file>` (a scratch database, the
Python and pk-api over it, pk-api forwarding to that Python). Every phase asks the SAME request at the SAME rate of
both bases, interleaved request by request (the order alternates), so the only difference between the two columns is
the hop through pk-api:

  seq     one request at a time, no writes: the forward's own cost (pool, keep-alive, body handling)
  conc    `--conc` threads, each alternating direct / via, no writes: the forward under concurrency
  writes  as `conc`, plus one jot (POST /api/ideas, straight to the Python) every `--write-every` seconds: the
          Python's per-decision rebuild, paid by both columns alike

    python tools/load/forward_probe.py --pair <file> [--n 120] [--conc 8] [--write-every 0.5] [--json out.json]

Only loopback bases are accepted (the writes are for a scratch database). Prints p50 / p95 / max ms per route and
column, and the via-minus-direct difference of the medians.
"""
from __future__ import annotations

import argparse
import http.client
import json
import statistics
import sys
import threading
import time
import urllib.parse
from pathlib import Path

ROUTES = [
    "/api/settings",
    "/api/mysets",
    "/api/boards",
    "/api/tonight/screen",
    "/api/search?q=parking&limit=6",
]


class Conn:
    """One keep-alive HTTP/1.1 connection, reopened when the server closes it."""

    def __init__(self, base: str):
        u = urllib.parse.urlsplit(base)
        if u.hostname not in ("127.0.0.1", "localhost", "::1"):
            raise SystemExit(f"{base} is not loopback")
        self.host, self.port = u.hostname, u.port
        self.c: http.client.HTTPConnection | None = None

    def call(self, method: str, path: str, body: bytes | None = None) -> tuple[float, int]:
        for attempt in (0, 1):
            if self.c is None:
                self.c = http.client.HTTPConnection(self.host, self.port, timeout=120)
            t = time.perf_counter()
            try:
                hdrs = {"Content-Type": "application/json"} if body is not None else {}
                self.c.request(method, path, body=body, headers=hdrs)
                r = self.c.getresponse()
                r.read()
                took = (time.perf_counter() - t) * 1000
                if r.getheader("connection", "").lower() == "close":
                    self.c.close()
                    self.c = None
                return took, r.status
            except (http.client.HTTPException, OSError):
                self.c.close()
                self.c = None
                if attempt:
                    raise
        raise AssertionError


def pct(xs: list[float], p: float) -> float:
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(p / 100 * (len(xs) - 1))))]


def run_phase(pair: dict, threads: int, n: int, write_every: float | None) -> dict:
    """`threads` workers, each sending `n` rounds over every route, each round direct then via (alternating)."""
    out: dict[tuple[str, str], list[float]] = {}
    errors: list[str] = []
    lock = threading.Lock()
    stop = threading.Event()
    writes = [0]

    def worker(w: int) -> None:
        d, v = Conn(pair["python"]), Conn(pair["rust"])
        for i in range(n):
            for k, route in enumerate(ROUTES):
                order = [("direct", d), ("via", v)]
                if (i + k + w) % 2:
                    order.reverse()
                for col, c in order:
                    try:
                        took, status = c.call("GET", route)
                    except Exception as e:  # noqa: BLE001
                        with lock:
                            errors.append(f"{col} {route}: {e}")
                        continue
                    with lock:
                        if status != 200:
                            errors.append(f"{col} {route}: HTTP {status}")
                        else:
                            out.setdefault((route, col), []).append(took)

    def writer() -> None:
        c = Conn(pair["python"])
        j = 0
        while not stop.wait(write_every):
            body = json.dumps({"text": f"probe jot {j}: the parking meter that talks back"}).encode()
            try:
                _, status = c.call("POST", "/api/ideas", body)
                if status == 200:
                    writes[0] += 1
            except Exception as e:  # noqa: BLE001
                errors.append(f"write: {e}")
            j += 1

    ts = [threading.Thread(target=worker, args=(w,)) for w in range(threads)]
    wt = threading.Thread(target=writer) if write_every else None
    t0 = time.perf_counter()
    if wt:
        wt.start()
    for t in ts:
        t.start()
    for t in ts:
        t.join()
    stop.set()
    if wt:
        wt.join()
    wall = time.perf_counter() - t0
    rows = []
    for route in ROUTES:
        d, v = out.get((route, "direct"), []), out.get((route, "via"), [])
        if not d or not v:
            continue
        rows.append({"route": route, "n": len(d),
                     "direct": [statistics.median(d), pct(d, 95), max(d)],
                     "via": [statistics.median(v), pct(v, 95), max(v)],
                     "diff_p50": statistics.median(v) - statistics.median(d)})
    return {"threads": threads, "rounds": n, "write_every": write_every, "writes": writes[0], "wall_s": wall,
            "errors": errors[:20], "error_count": len(errors), "rows": rows}


def show(name: str, r: dict) -> None:
    print(f"\n== {name}: {r['threads']} thread(s) x {r['rounds']} rounds, writes {r['writes']} "
          f"(every {r['write_every']} s), {r['wall_s']:.1f} s, errors {r['error_count']}")
    print(f"{'route':34} {'n':>5}  {'direct p50/p95/max':>24}  {'via pk-api p50/p95/max':>24}  {'d p50':>7}")
    for x in r["rows"]:
        f = lambda a: f"{a[0]:7.1f}/{a[1]:7.1f}/{a[2]:7.1f}"  # noqa: E731
        print(f"{x['route']:34} {x['n']:5}  {f(x['direct']):>24}  {f(x['via']):>24}  {x['diff_p50']:7.1f}")
    for e in r["errors"][:5]:
        print("  error:", e)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pair", type=Path, required=True, help="the --hold file scratch_api.py wrote")
    ap.add_argument("--n", type=int, default=120, help="rounds per thread")
    ap.add_argument("--conc", type=int, default=8)
    ap.add_argument("--write-every", type=float, default=0.5)
    ap.add_argument("--phases", default="seq,conc,writes")
    ap.add_argument("--json", type=Path)
    a = ap.parse_args()
    pair = json.loads(a.pair.read_text(encoding="utf-8"))
    res = {}
    for ph in a.phases.split(","):
        name, _, n = ph.partition(":")  # conc:4 / writes:1 = that many threads instead of --conc
        threads = 1 if name == "seq" else int(n or a.conc)
        rounds = a.n if threads == 1 else max(1, a.n // threads * 2)
        if name not in ("seq", "conc", "writes"):
            raise SystemExit(f"unknown phase {ph}")
        res[ph] = run_phase(pair, threads, rounds, a.write_every if name == "writes" else None)
        show(ph, res[ph])
    if a.json:
        a.json.write_text(json.dumps(res, indent=1), encoding="utf-8")
    return 1 if any(r["error_count"] for r in res.values()) else 0


if __name__ == "__main__":
    sys.exit(main())
