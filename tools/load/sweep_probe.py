"""Does the vector sweeper slow a search? (T10 round 4, C4.) The model has one slot, so a search's query vector waits
for the sweeper's batch in hand; this measures that wait.

    python tools/load/scratch_api.py --rust <pk-api> --hold <file>      (PK_VEC_SWEEP_S=3, PK_VEC_BATCH=<n> in the env)
    python tools/load/sweep_probe.py --pair <file> [--jots 400]

What it does, on a scratch pair only (loopback):
  1. kills the Python (its lease lapses 2 x PK_VEC_SWEEP_S + 30 s after its last renewal; pk-api then holds it),
  2. measures `GET /api/search` on pk-api back to back for the idle seconds that follow (the baseline),
  3. posts `--jots` typed ideas through pk-api (each one is a vector to sweep) and keeps measuring until pk-api's log
     says the pass embedded them, and
  4. prints p50 / p95 / max ms of the idle samples and of the samples taken while the sweeper ran.
"""
from __future__ import annotations

import argparse
import http.client
import json
import os
import re
import statistics
import subprocess
import sys
import time
import urllib.parse
from pathlib import Path

WORDS = "badger kettle orbit pickle ladder velvet cactus mitten radar tundra walrus zipper hammock pepper quartz".split()


def pct(xs: list[float], p: float) -> float:
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(p / 100 * (len(xs) - 1))))]


def line(label: str, xs: list[float]) -> str:
    if not xs:
        return f"{label}: no samples"
    return f"{label}: n={len(xs)} p50 {statistics.median(xs):.1f} p95 {pct(xs, 95):.1f} max {max(xs):.1f} ms"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pair", type=Path, required=True)
    ap.add_argument("--jots", type=int, default=400)
    ap.add_argument("--idle-s", type=float, default=45.0)
    a = ap.parse_args()
    pair = json.loads(a.pair.read_text(encoding="utf-8"))
    u = urllib.parse.urlsplit(pair["rust"])
    if u.hostname not in ("127.0.0.1", "localhost"):
        raise SystemExit("loopback only")
    conn = http.client.HTTPConnection(u.hostname, u.port, timeout=120)
    n = [0]

    def search() -> float:
        n[0] += 1
        q = f"{WORDS[n[0] % len(WORDS)]} {WORDS[(n[0] * 7) % len(WORDS)]} parking {n[0]}"
        t = time.perf_counter()
        conn.request("GET", "/api/search?limit=6&q=" + urllib.parse.quote(q))
        r = conn.getresponse()
        r.read()
        if r.status != 200:
            raise SystemExit(f"search answered {r.status}")
        return (time.perf_counter() - t) * 1000

    log = Path(pair["rust_log"])
    mark = log.stat().st_size if log.exists() else 0
    subprocess.run(["taskkill", "/F", "/PID", str(pair["python_pid"])], capture_output=True)
    idle: list[float] = []
    t_end = time.time() + a.idle_s
    while time.time() < t_end:
        idle.append(search())
    mark = log.stat().st_size
    for i in range(a.jots):
        text = f"sweep probe {i}: " + " ".join(WORDS[(i * k + k) % len(WORDS)] for k in range(1, 9)) + f" number {i * 7919}"
        conn.request("POST", "/api/ideas", body=json.dumps({"text": text}), headers={"Content-Type": "application/json"})
        r = conn.getresponse()
        r.read()
        if r.status != 200:
            raise SystemExit(f"jot answered {r.status}")
    busy: list[float] = []
    deadline = time.time() + 240
    embedded = 0
    while time.time() < deadline and embedded < a.jots:
        busy.append(search())
        with open(log, "rb") as f:
            f.seek(mark)
            tail = f.read().decode("utf-8", "replace")
        embedded = sum(int(m) for m in re.findall(r'"joke": (\d+)', tail))
    print(line("idle  ", idle))
    print(line("sweep ", busy))
    print(f"batch {os.environ.get('PK_VEC_BATCH', 'default')}; sweep window {len(busy)} searches; jots embedded: {embedded} of {a.jots}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
