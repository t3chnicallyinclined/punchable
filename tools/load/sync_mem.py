"""How much memory does pk-api's graph sync take, and does it give it back? (T10 round 6b.)

Runs against a pair started by `tools/load/scratch_api.py --rust <pk-api> --hold <file>` (set PK_SCRATCH_EDGES=13070 and
PK_SCRATCH_LAUGHS=300 for the production scale the SRE's trial measured: 4.8k nodes, 13k edges). Samples the resident
set of the pk-api process every 20 ms while a worker post of a set's analysis syncs the graph tables, three times, with a
search (which loads the embedding model) between the second and the third:

    python tools/load/sync_mem.py --pair <file> [--json out.json]

Prints MiB: idle, then for each sync the peak while it ran and the level 6 s after.
"""
from __future__ import annotations

import argparse
import http.client
import json
import threading
import time
import urllib.parse
from pathlib import Path

import psutil


def call(base: str, method: str, path: str, body: dict | None = None):
    u = urllib.parse.urlsplit(base)
    c = http.client.HTTPConnection(u.hostname, u.port, timeout=300)
    c.request(method, path, body=json.dumps(body).encode() if body is not None else None,
              headers={"Content-Type": "application/json"} if body is not None else {})
    r = c.getresponse()
    data = r.read()
    c.close()
    return r.status, json.loads(data) if data else None


def mib(p: psutil.Process) -> float:
    return p.memory_info().rss / 1048576


def watch(p: psutil.Process, stop: threading.Event, out: list) -> None:
    while not stop.is_set():
        out.append(mib(p))
        time.sleep(0.02)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pair", type=Path, required=True)
    ap.add_argument("--json", type=Path)
    a = ap.parse_args()
    pair = json.loads(a.pair.read_text(encoding="utf-8"))
    base, p = pair["rust"], psutil.Process(pair["rust_pid"])
    _, files = call(base, "GET", "/api/worker/files?prefix=corpus/sets/")
    path = next(f["path"] for f in files["files"] if f["path"].endswith("/analysis.json"))
    _, got = call(base, "GET", "/api/worker/files/content?path=" + urllib.parse.quote(path, safe="/"))
    doc = json.loads(got["text"])
    n_files = len(files["files"])
    out: dict = {"set_files": n_files, "idle_mib": None, "syncs": []}
    time.sleep(3)
    out["idle_mib"] = round(mib(p), 1)

    def sync(i: int) -> dict:
        samples: list[float] = []
        stop = threading.Event()
        t = threading.Thread(target=watch, args=(p, stop, samples))
        t.start()
        t0 = time.time()
        st, r = call(base, "POST", "/api/worker/files",
                     {"files": [{"path": path, "text": json.dumps(dict(doc, generated_at=f"mem probe {i}"))}], "actor": "import:probe"})
        took = time.time() - t0
        stop.set()
        t.join()
        time.sleep(6)
        return {"status": st, "s": round(took, 2), "peak_mib": round(max(samples), 1), "after_mib": round(mib(p), 1),
                "synced": (r or {}).get("synced")}

    out["syncs"].append(sync(1))
    out["syncs"].append(sync(2))
    st, _ = call(base, "GET", "/api/search?q=parking&limit=6")
    time.sleep(3)
    out["search_status"], out["after_search_mib"] = st, round(mib(p), 1)
    out["syncs"].append(sync(3))
    print(json.dumps(out, indent=1))
    if a.json:
        a.json.write_text(json.dumps(out, indent=1), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
