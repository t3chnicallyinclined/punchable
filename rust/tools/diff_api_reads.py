"""T9's gate: the same GET requests against the hosted Python API (api/entry.py) and pk-api, over ONE scratch
SurrealDB loaded the way production is (db/push_remote.py pack + apply), with Keycloak replaced by a local JWKS
(api-dev/tests/jwt_kit.py). Every answer must be equal: the status, the JSON body as a parsed value (ADR 0007) and
the headers the app reads (content type, no-store, the 401 challenge, CORS). Known differences are in ALLOW below,
each with its reason; there are none on the happy path.

    python rust/tools/diff_api_reads.py --pk-api rust/target/release/pk-api.exe [--bench 300] [--report out.json]

What it does, in order: build a SYNTHETIC data folder (invented bits, nights and labels; nothing of the comic's),
start `surreal start memory` with a throwaway password, pack and apply the bundle, start both APIs on loopback with
auth ON, write through pk-api (every write is forwarded to the Python: the T9 proxy) and check that the very next
GET from pk-api sees it (the read-after-write gate, docs/ARCHITECTURE.md section 2), run the request matrix, and
optionally time each route group on both (p50 / p95, sequential, warm) and read both processes' memory.
Exit 1 on any difference outside ALLOW. Needs: surreal 3.3.4 (PUNCH_SURREAL or ~/.punchable/bin/surreal[.exe]), the
repo's Python deps (the API's), and a built pk-api.
"""
from __future__ import annotations

import argparse
import json
import os
import platform
import secrets
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "api-dev" / "tests"))
import jwt_kit as JK  # noqa: E402  (test-only RSA key, JWKS and RS256 tokens)

NS, DB = "punchable_test", "t9"
ORIGIN = "https://punchable.io"
FLAGS = '{"glass_accessory": false}'

# ---------------------------------------------------------------- known differences (path in the response -> why)
# Keys are (request label, JSON path or "status" / "header:<name>"). Empty on purpose: add one only with a reason the
# owner would accept (the board's rule: the allow-list is for things that cannot hold, not a licence).
ALLOW: dict[tuple[str, str], str] = {}

# ---------------------------------------------------------------- the synthetic data folder
TEXT = {
    "a": "Alpha opener line one. The alpha punch lands here every time.",
    "b": "Bravo setup about the dog. The purple monkey dishwasher line here. Bravo closer words.",
    "c": "Charlie premise about cars and traffic and more traffic today.",
    "d": "Delta bit about my niece and the laundromat place.",
    "e": "Echo bit about my niece and her soccer schedule.",
    "f": "Foxtrot never performed idea about planes and pilots up there.",
    "g": "Golf hidden note that is not a joke at all, just a grocery list.",
}
TITLE = {"a": "Alpha bit", "b": "Bravo bit (the dog one)", "c": "Charlie bit", "d": "Delta bit", "e": "Echo bit",
         "f": "Foxtrot bit about the very long title that goes on", "g": "Golf list"}
STAGE = {"a": "testing", "b": "working", "c": "testing", "d": "to_try", "e": "idea", "f": "idea", "g": "idea"}
UNIT_DATE = {"a": "2024-01-10", "b": "2023-05-01", "c": "2024-04-04", "d": "2015-03-03", "e": "2024-08-08",
             "f": "2016-07-07", "g": "2024-09-09"}
SX, SY, SZ = "2024-01-01_x", "2024-02-01_y", "2024-03-01_z"


def set_dir(key: str) -> str:
    return key[:4] + key[5:7] + key[8:10] + key[10:]


def make_graph() -> dict:
    nodes, edges = [], []
    for k in "abcdefg":
        j = {"id": f"joke:{k}", "type": "joke", "title": TITLE[k], "stage": STAGE[k], "summary": f"A bit about {k}.",
             "first_seen": "2023-01-01", "last_seen": UNIT_DATE[k], "lead_unit": f"unit:{k}1"}
        u = {"id": f"unit:{k}1", "type": "unit", "kind": "setup+punch", "text": TEXT[k], "date": UNIT_DATE[k],
             "source": "gdoc", "doc_title": "Notes"}
        if k == "g":
            j["hidden"] = u["hidden"] = True
        nodes += [j, u]
        edges.append({"id": f"e:p{k}", "src": f"unit:{k}1", "dst": f"joke:{k}", "kind": "part_of", "status": "inferred",
                      "confidence": 0.9, "evidence": []})
    nodes.append({"id": "unit:a0", "type": "unit", "kind": "setup+punch", "text": "Alpha opener, first draft.", "date": "2023-02-02"})
    edges.append({"id": "e:pa0", "src": "unit:a0", "dst": "joke:a", "kind": "part_of", "status": "proposed", "confidence": 0.6,
                  "evidence": []})
    for lid, kind, name in [("label:persona:dad", "persona", "Dad"), ("label:topic:kids", "topic", "Kids"),
                            ("label:topic:cars", "topic", "Cars"), ("label:device:d1", "device", "D-one"),
                            ("label:device:d2", "device", "D-two"), ("label:structure:s1", "structure", "S-one")]:
        nodes.append({"id": lid, "type": "label", "kind": kind, "name": name, "definition": f"{name}, as a label"})

    def lab(j, lbl, w, q):
        edges.append({"id": f"e:l{j}-{lbl.split(':')[-1]}", "src": f"joke:{j}", "dst": lbl, "kind": "has_label",
                      "status": "proposed", "confidence": w, "reason": f"{q} reads as {lbl.split(':')[-1]}",
                      "evidence": [{"node": f"unit:{j}1", "quote": q}]})
    lab("a", "label:device:d1", 0.9, "Alpha opener")
    lab("b", "label:device:d1", 0.9, "purple monkey")
    lab("c", "label:device:d2", 0.9, "Charlie premise")
    lab("d", "label:persona:dad", 0.8, "my niece")
    lab("e", "label:persona:dad", 0.8, "my niece")
    lab("d", "label:topic:kids", 0.8, "laundromat")
    lab("e", "label:topic:kids", 0.8, "soccer schedule")
    lab("c", "label:topic:cars", 0.8, "cars and traffic")
    lab("g", "label:topic:kids", 0.5, "grocery list")
    edges.append({"id": "e:seg_bc", "src": "joke:b", "dst": "joke:c", "kind": "segue", "status": "proposed", "confidence": 0.7,
                  "reason": "dog leads to cars", "evidence": [{"node": "unit:b1", "quote": "Bravo closer words."}]})
    for key, title, venue, dur in ((SX, "Corner Room, Tuesday", "The Corner Room", 300),
                                   (SY, "Seaside open mic", "Seaside Comedy Club", 240),
                                   (SZ, "Mabel's, March", "Mabel's (upstairs)", 120)):
        n = {"id": f"set:{key}", "type": "set", "title": title, "venue": venue, "date": key[:10], "duration_s": dur}
        if key == SZ:
            n["audio_unreliable"] = True
        nodes.append(n)
    for start, end, text, kind, name, joke in ((0, 60000, TEXT["a"], "bit", "Alpha", "a"),
                                                (60000, 120000, TEXT["b"], "bit", "Bravo", "b"),
                                                (120000, 140000, "So who here has a car?", "crowd_work", "Crowd", None),
                                                (140000, 200000, TEXT["c"], "bit", "Charlie", "c")):
        pid = f"perf:{SX}@{start}"
        nodes.append({"id": pid, "type": "performance", "set": f"set:{SX}", "start_ms": start, "end_ms": end, "text": text,
                      "kind": kind, "name": name})
        if joke:
            edges.append({"id": f"e:pa-{joke}", "src": pid, "dst": f"joke:{joke}", "kind": "performed_as",
                          "status": "proposed", "confidence": 0.8, "reason": f"the {name.lower()} words",
                          "evidence": [{"node": pid, "quote": text.split(".")[0] + ".", "start_ms": start + 1000}]})
    return {"version": "t9-test", "generated_at": "2026-10-01", "runs": ["r1"], "nodes": nodes, "edges": edges}


def row(idx, joke, start_s, dur_s, text, laugh_s, set_rate, status="known"):
    window = float(dur_s)
    lspm = round(laugh_s / (window / 60), 2) if window else 0.0
    return {"idx": idx, "start_ms": int(start_s * 1000), "end_ms": int((start_s + dur_s) * 1000), "title": f"T{idx}",
            "status": status, "joke_id": joke, "version_unit_id": f"unit:{joke[5:]}1" if joke else None,
            "match_confidence": 0.9 if joke else None, "match_reason": "same words" if joke else "", "transcript": text,
            "punchlines": [], "laughs": {"count": 1 if laugh_s else 0, "max_bars": 2 if laugh_s else 0, "total_s": laugh_s,
                                         "merged_count": 1 if laugh_s else 0, "laugh_s": laugh_s, "window_s": window},
            "laugh_s_per_min": lspm, "pct_laughing": round(laugh_s / window, 3), "time_to_first_laugh_s": 5.0 if laugh_s else None,
            "dead_air_max_s": 5.0, "vs_room": round(lspm / set_rate, 2), "ai_rating": "worked" if laugh_s else "died",
            "rating_basis": "the laugh" if laugh_s else "silence", "similar": [], "hint_labels": [],
            "script_diff": {"on_script_pct": 50, "dropped": [], "added": []}}


def analysis(key, rate, rows):
    return {"set": key, "run_id": "synthetic-1", "generated_at": "2026-10-01T00:00:00+00:00",
            "summary": {"laugh_s_per_min": rate, "room_heat": 1.0, "room_heat_basis": "set", "material_s": 300.0,
                        "laugh_s_total": 24.0, "pct_laughing": 0.08, "laughs_merged": 4, "laughs_per_min_merged": 0.8,
                        "applause_s": 0.0, "longest_laugh_s": 3.0, "longest_laugh_joke": rows[0]["joke_id"],
                        "dead_air_max_s": 9.0, "dead_air_max_at_ms": 100000, "duration_s": 300, "best": None,
                        "weakest": None, "jokes_counted": sum(1 for r in rows if r["status"] in ("known", "new")),
                        "new_material": sum(1 for r in rows if r["status"] == "new")},
            "jokes": rows}


SETS = {
    SX: {"analysis": analysis(SX, 6.0, [row(1, "joke:a", 0, 60, TEXT["a"], 9.0, 6.0), row(2, "joke:b", 60, 60, TEXT["b"], 6.0, 6.0),
                                        row(3, None, 120, 20, "So who here has a car?", 0.0, 6.0, status="crowd_work"),
                                        row(4, "joke:c", 140, 60, TEXT["c"], 6.1, 6.0)]),
         "meta": {"title": "Corner Room, Tuesday", "venue_guess": "The Corner Room", "show_date": "2024-01-05",
                  "show_date_source": "the folder date", "duration": 300.0}},
    SY: {"analysis": analysis(SY, 7.3, [row(1, "joke:a", 0, 60, TEXT["a"], 8.4, 7.3),
                                        row(2, "joke:b", 60, 90, "Bravo setup about the dog. Bravo closer words.", 2.0, 7.3),
                                        row(3, None, 150, 60, TEXT["d"], 3.0, 7.3, status="new")]),
         "meta": {"source": "app_upload", "title": "Seaside open mic", "show_venue": "Seaside Comedy Club", "duration_s": 240.0}},
    SZ: {"analysis": analysis(SZ, 5.0, [row(1, "joke:b", 0, 40, "Bravo setup about the dog.", 1.5, 5.0),
                                        row(2, "joke:a", 40, 30, "Alpha opener line one.", 4.0, 5.0)]),
         "meta": {"title": "Mabel's, March", "venue_guess": "Mabel's (upstairs)", "show_date": "2024-03-01"}},
}
INFERRED = {"run_id": "inf-1", "generated_at": "2026-10-01T00:00:00+00:00", "notes": "synthetic",
            "jokes": [{"id": "joke:a", "stage": "working", "stage_confidence": 0.8, "stage_reason": "three nights"},
                      {"id": "joke:f", "stage": "idea", "stage_confidence": 0.6, "stage_reason": "never taped"}]}
LEARNING = {"generated_at": "2026-10-01T00:00:00+00:00", "lessons": [{"what": "openers land", "n": 3}]}


def write_json(p: Path, obj) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(obj, ensure_ascii=False, indent=1), encoding="utf-8", newline="\n")


def build_comedy(comedy: Path) -> None:
    # kb/v0 (no kb/v1): db/load.py's v1 path runs kb_validate, whose id and evidence rules a small fixture need not meet
    v0 = comedy / "kb" / "v0"
    write_json(v0 / "graph.json", make_graph())
    (v0 / "decisions.jsonl").write_text("", encoding="utf-8")
    write_json(v0 / "inferred.json", INFERRED)
    write_json(v0 / "learning.json", LEARNING)
    (v0 / "KB_REPORT.md").write_text("# Report\n\n## first 50\n\n- e:pa0\n", encoding="utf-8", newline="\n")
    for key, s in SETS.items():
        d = comedy / "corpus" / "sets" / set_dir(key)
        write_json(d / "analysis.json", s["analysis"])
        write_json(d / "meta.json", s["meta"])
        write_json(d / "transcript.json", {"segments": [{"start": 0, "end": 5, "text": "Hello."}]})
        write_json(d / "laughs_auto.json", {"detector": "synthetic", "run_id": "laughs-1", "laughs": []})
    (comedy / "kb" / "creative").mkdir(parents=True)


# ---------------------------------------------------------------- processes
def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def surreal_exe() -> str:
    p = os.environ.get("PUNCH_SURREAL") or str(Path.home() / ".punchable" / "bin" / ("surreal.exe" if os.name == "nt" else "surreal"))
    if not Path(p).is_file():
        p = shutil.which("surreal") or p
    if not Path(p).is_file():
        sys.exit(f"no surreal binary (PUNCH_SURREAL or {p})")
    return p


def wait_http(url: str, proc: subprocess.Popen, what: str, secs: float = 120) -> None:
    t = time.time()
    while time.time() - t < secs:
        if proc.poll() is not None:
            raise SystemExit(f"{what} exited {proc.returncode}")
        try:
            with urlopen(url, timeout=2) as r:
                if r.status == 200:
                    return
        except (OSError, HTTPError):
            pass
        time.sleep(0.2)
    raise SystemExit(f"{what} did not become ready at {url}")


class JwksServer:
    def __init__(self, keys: list):
        doc = json.dumps({"keys": [k.jwk() for k in keys]}).encode()

        class H(BaseHTTPRequestHandler):
            def do_GET(self):  # noqa: N802
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(doc)))
                self.end_headers()
                self.wfile.write(doc)

            def log_message(self, *a):
                pass

        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.srv.server_address[1]}/certs"


def rss_mib(pid: int) -> float | None:
    try:
        import psutil
        return round(psutil.Process(pid).memory_info().rss / 2**20, 1)
    except Exception:  # noqa: BLE001
        return None


# ---------------------------------------------------------------- HTTP
def call(base: str, method: str, path: str, headers: dict | None = None, body=None):
    data = json.dumps(body).encode() if body is not None else None
    h = dict(headers or {})
    if data is not None:
        h["Content-Type"] = "application/json"
    req = Request(base + path, data=data, method=method, headers=h)
    try:
        with urlopen(req, timeout=60) as r:
            return r.status, {k.lower(): v for k, v in r.headers.items()}, r.read()
    except HTTPError as e:
        return e.code, {k.lower(): v for k, v in e.headers.items()}, e.read()


HEADERS = ("content-type", "cache-control", "www-authenticate", "access-control-allow-origin", "vary",
           "access-control-expose-headers", "access-control-allow-methods", "access-control-allow-headers",
           "access-control-max-age")


def diff(a, b, path="$", out=None):
    out = [] if out is None else out
    if isinstance(a, dict) and isinstance(b, dict):
        for k in sorted(set(a) | set(b), key=str):
            if k not in a or k not in b:
                out.append((f"{path}.{k}", a.get(k, "<absent>"), b.get(k, "<absent>")))
            else:
                diff(a[k], b[k], f"{path}.{k}", out)
    elif isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            out.append((f"{path}.len", len(a), len(b)))
        for i, (x, y) in enumerate(zip(a, b)):
            diff(x, y, f"{path}[{i}]", out)
    elif isinstance(a, bool) or isinstance(b, bool):
        if a is not b:
            out.append((path, a, b))
    elif a != b:
        out.append((path, a, b))
    return out


def compare(label: str, py: tuple, rs: tuple) -> list[str]:
    bad = []
    if py[0] != rs[0] and (label, "status") not in ALLOW:
        bad.append(f"{label}: status python {py[0]} rust {rs[0]}")
    for h in HEADERS:
        if py[1].get(h) != rs[1].get(h) and (label, f"header:{h}") not in ALLOW:
            bad.append(f"{label}: header {h}: python {py[1].get(h)!r} rust {rs[1].get(h)!r}")
    if py[2] or rs[2]:
        try:
            a, b = json.loads(py[2] or b"null"), json.loads(rs[2] or b"null")
        except ValueError:
            if py[2] != rs[2]:
                bad.append(f"{label}: bodies differ and are not both JSON")
            return bad
        for p, x, y in diff(a, b):
            if (label, p) not in ALLOW:
                bad.append(f"{label}: {p}: python {json.dumps(x)[:120]} rust {json.dumps(y)[:120]}")
    return bad


# ---------------------------------------------------------------- the run
def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pk-api", required=True, type=Path)
    ap.add_argument("--bench", type=int, default=0, help="requests per route group per server for the timing (0 = none)")
    ap.add_argument("--report", type=Path, default=None)
    ap.add_argument("--keep", action="store_true", help="keep the temp folder (logs)")
    a = ap.parse_args()

    tmp = Path(tempfile.mkdtemp(prefix="pk-t9-"))
    comedy, data, home = tmp / "comedy", tmp / "data", tmp / "home"
    for d in (data, home, tmp / "cache"):
        d.mkdir()
    build_comedy(comedy)
    port = free_port()
    pw = secrets.token_hex(16)
    env = {**os.environ, "PUNCHABLE_HOME": str(home), "COMEDY_DIR": str(comedy), "PUNCH_COMEDY": str(comedy),
           "PK_CACHE_DIR": str(tmp / "cache"), "PK_DBLOAD_DIR": str(tmp / "dbload"), "SURREAL_PORT": str(port),
           "SURREAL_USER": "root", "SURREAL_PASS": pw, "SURREAL_NS": NS, "SURREAL_DB": DB, "PYTHONHASHSEED": "0",
           "PUNCH_API_QUIET": "1", "PYTHONIOENCODING": "utf-8",
           # pk-api reads PUNCH_SECRETS (default ~/.punchable/secrets.env) under the process env: point it at nothing, so
           # no real DSN, database user or token can reach this run
           "PUNCH_SECRETS": str(home / "secrets.env")}
    for k in ("SURREAL_URL", "SURREAL_API_USER", "SURREAL_API_PASS", "PK_BGE_DIR", "SENTRY_DSN", "PUNCHABLE_LEGAL_RULES"):
        env.pop(k, None)
    procs: list[subprocess.Popen] = []
    logs = []

    def start(args, extra, name):
        log = open(tmp / f"{name}.log", "w", encoding="utf-8")
        logs.append(log)
        p = subprocess.Popen(args, env={**env, **extra}, stdout=log, stderr=subprocess.STDOUT, cwd=str(ROOT))
        procs.append(p)
        return p

    problems: list[str] = []
    report: dict = {"machine": f"{platform.node()} ({platform.system()} {platform.release()}, {platform.processor()})",
                    "python": platform.python_version()}
    try:
        sp = start([surreal_exe(), "start", "--user", "root", "--pass", pw, "--bind", f"127.0.0.1:{port}", "memory"], {}, "surreal")
        wait_http(f"http://127.0.0.1:{port}/health", sp, "surreal")
        tgz = tmp / "bundle.tgz"
        for step in (["pack", "--comedy", str(comedy), "--tar", str(tgz)], ["apply", "--tar", str(tgz)]):
            r = subprocess.run([sys.executable, str(ROOT / "db" / "push_remote.py"), *step], env=env, cwd=str(ROOT),
                               capture_output=True, text=True, encoding="utf-8")
            if r.returncode != 0:
                raise SystemExit(f"push_remote {step[0]} failed:\n{r.stdout[-2000:]}\n{r.stderr[-3000:]}")

        key, other = JK.Key("t9", seed=9), JK.Key("t9-other", seed=10)
        jwks = JwksServer([key])
        auth_env = {"PK_AUTH": "1", "OIDC_ISSUER": JK.ISS, "OIDC_AUDIENCE": JK.AUD, "OIDC_JWKS_URL": jwks.url,
                    "PUBLIC_URL": ORIGIN + "/", "PK_APP_FLAGS": FLAGS, "PK_DATA": str(data), "HOST": "127.0.0.1",
                    "PK_BOOT_POLL_S": "1"}
        py_port, rs_port = free_port(), free_port()
        py = f"http://127.0.0.1:{py_port}"
        rs = f"http://127.0.0.1:{rs_port}"
        pp = start([sys.executable, str(ROOT / "api" / "entry.py")], {**auth_env, "PORT": str(py_port)}, "python-api")
        wait_http(py + "/api/health?ready=1", pp, "python api")
        rp = start([str(a.pk_api)], {**auth_env, "PORT": str(rs_port), "PK_PYTHON_URL": py, "RUST_LOG": os.environ.get("RUST_LOG", "info")}, "pk-api")
        wait_http(rs + "/api/health?ready=1", rp, "pk-api")

        now = time.time()
        app = {"Authorization": "Bearer " + JK.token(key, now=now)}
        tok = lambda **c: {"Authorization": "Bearer " + JK.token(key, now=now, **c)}  # noqa: E731

        # -- writes through pk-api (forwarded to the Python), each followed at once by a pk-api read
        raw_ms = []
        writes = [("POST", "/api/ideas", {"text": "Trains are late every morning here. The conductor waves anyway."}),
                  ("POST", "/api/decision", {"target": "node", "id": "joke:c", "action": "edit", "after": {"stage": "working"}}),
                  ("POST", "/api/decision", {"target": "edge", "id": "e:la-d1", "action": "confirm"}),
                  ("POST", "/api/review/rate", {"perf": f"perf:{SX}@0", "rating": "killed"}),
                  ("POST", "/api/review/rate", {"perf": f"perf:{SX}@60000", "rating": "meh"}),
                  ("POST", "/api/decision", {"target": "node", "id": "joke:e", "action": "edit", "after": {"stage": "testing"}})]
        want = json.loads(call(rs, "GET", "/api/decisions?limit=1", app)[2])["total"]
        for i, (m, p, b) in enumerate(writes):
            st, _, body = call(rs, m, p, app, b)
            if st != 200:
                raise SystemExit(f"write {p} through pk-api: HTTP {st} {body[:300]!r}")
            want += len(json.loads(body)["indexes"])
            t = time.perf_counter()
            st, _, body = call(rs, "GET", "/api/decisions?limit=1", app)
            raw_ms.append((time.perf_counter() - t) * 1000)
            total = json.loads(body)["total"]
            if total != want:
                problems.append(f"read-after-write: after write {i + 1} pk-api shows {total} decisions, wanted {want}")
        st, _, body = call(rs, "POST", "/api/undo", app, {"index": want - 1})
        if st != 200:
            raise SystemExit(f"undo through pk-api: HTTP {st} {body[:300]!r}")
        report["read_after_write_ms"] = [round(x, 1) for x in raw_ms]
        report["read_after_write_max_ms"] = round(max(raw_ms), 1)
        if max(raw_ms) > 100:
            problems.append(f"read-after-write: a pk-api read took {max(raw_ms):.0f} ms after a write (gate 100 ms)")

        # -- the matrix
        reqs: list[tuple[str, str, str, dict]] = []
        for p in ("/api/labels", "/api/labels?include_hidden=1", "/api/decisions", "/api/decisions?limit=2",
                  "/api/decisions?limit=0", "/api/decisions?limit=-2", "/api/decisions?limit=abc", "/api/decisions?limit=9999",
                  "/api/decisions?limit=%203%20", "/api/meta", "/api/meta?labels=2", "/api/meta?labels=-1",
                  "/api/meta?include_hidden=1", "/api/meta?labels=x", "/api/pipeline", "/api/pipeline?limit=1",
                  "/api/pipeline?include_hidden=true", "/api/pipeline?limit=-1", "/api/learned", "/api/health",
                  "/api/health?ready=1", "/api/labels?backend=files", "/api/labels?backend=surreal",
                  "/api/labels?backend=", "/api/health?backend=x"):
            reqs.append((p, "GET", p, app))
        reqs += [("HEAD meta", "HEAD", "/api/meta", app),
                 ("health signed out", "GET", "/api/health", {}),
                 ("health bad token", "GET", "/api/health", {"Authorization": "Bearer x.y.z"}),
                 ("OPTIONS labels", "OPTIONS", "/api/labels", {"Origin": ORIGIN}),
                 ("OPTIONS labels evil", "OPTIONS", "/api/labels", {"Origin": "https://evil.example"}),
                 ("CORS public", "GET", "/api/labels", {**app, "Origin": ORIGIN}),
                 ("CORS loopback", "GET", "/api/meta", {**app, "Origin": "http://localhost:8081"}),
                 ("CORS evil", "GET", "/api/meta", {**app, "Origin": "http://localhost.evil.example"}),
                 ("CORS 401", "GET", "/api/meta", {"Origin": ORIGIN}),
                 ("no token", "GET", "/api/labels", {}),
                 ("not bearer", "GET", "/api/labels", {"Authorization": "Basic abc"}),
                 ("garbage", "GET", "/api/labels", {"Authorization": "Bearer garbage"}),
                 ("bad chars", "GET", "/api/labels", {"Authorization": "Bearer a$b.c.d"}),
                 ("expired", "GET", "/api/labels", tok(exp=int(now) - 3600)),
                 ("in skew", "GET", "/api/labels", tok(exp=int(now) - 30)),
                 ("no exp", "GET", "/api/labels", tok(exp=None)),
                 ("wrong aud", "GET", "/api/labels", tok(aud="account")),
                 ("aud string", "GET", "/api/labels", tok(aud=JK.AUD)),
                 ("wrong iss", "GET", "/api/labels", tok(iss="https://elsewhere.example/realms/x")),
                 ("nbf ahead", "GET", "/api/labels", tok(nbf=int(now) + 3600)),
                 ("iat ahead", "GET", "/api/labels", tok(iat=int(now) + 3600)),
                 ("refresh typ", "GET", "/api/labels", tok(typ="Refresh")),
                 ("no typ", "GET", "/api/labels", tok(typ=None)),
                 ("worker on app", "GET", "/api/labels", tok(realm_access={"roles": ["worker"]})),
                 ("unknown kid", "GET", "/api/labels", {"Authorization": "Bearer " + JK.token(other, now=now)}),
                 ("HS256", "GET", "/api/labels", {"Authorization": "Bearer " + JK.token(key, now=now, header={"alg": "HS256"})}),
                 ("no kid", "GET", "/api/labels", {"Authorization": "Bearer " + JK.token(key, now=now, header={"kid": None})}),
                 ("bad sig", "GET", "/api/labels", {"Authorization": "Bearer " + JK.token(key, now=now)[:-6] + "AAAAAA"}),
                 ("proxied 404", "GET", "/api/nothing-here", app),
                 ("proxied jokes", "GET", "/api/jokes?limit=3", app)]
        n_ok = 0
        for label, m, p, h in reqs:
            got = compare(label, call(py, m, p, h), call(rs, m, p, h))
            problems += got
            n_ok += not got
        # the whole Bits-list row (AppIndex.summaries) before /api/jokes moves: the Python's page without the creative
        # layer's duration, against pk-api's diagnostics route (a build with --features diagnostics)
        for hid in ("", "&include_hidden=1"):
            label = "summaries" + hid
            s_py = call(py, "GET", "/api/jokes?limit=5000" + hid, app)
            s_rs = call(rs, "GET", "/api/_t9/summaries?x=1" + hid, app)
            if s_rs[0] != 200:
                problems.append(f"{label}: pk-api has no diagnostics route (build it with --features diagnostics)")
                continue
            items = json.loads(s_py[2])["items"]
            for x in items:
                x.pop("duration_s", None)
                x.pop("duration_estimated", None)
            got = compare(label, (200, {}, json.dumps({"items": items}).encode()), (200, {}, s_rs[2]))
            problems += got
            n_ok += not got
            reqs.append((label, "GET", "", {}))
        report["requests"] = len(reqs)
        report["equal"] = n_ok
        # proof the answers came from pk-api and not through its fallback: its own counters saw every moved route
        mtext = call(rs, "GET", "/metrics")[2].decode()
        for route in ("/api/health", "/api/labels", "/api/decisions", "/api/meta", "/api/pipeline", "/api/learned"):
            if f'route="{route}",status="200"' not in mtext:
                problems.append(f"pk-api did not answer {route} itself (no 200 in its own /metrics)")
        if 'route="/api/jokes"' in mtext:
            problems.append("pk-api counted a forwarded route (/api/jokes): the fallback must count nothing")

        # -- timing and memory
        if a.bench:
            # (path, signed in?): /metrics and the signed-out health split the HTTP, auth and database costs apart
            groups = {"metrics (no auth, no database)": ("/metrics", False), "health signed out (database)": ("/api/health", False),
                      "health": ("/api/health", True), "labels": ("/api/labels", True),
                      "decisions": ("/api/decisions?limit=100", True), "meta": ("/api/meta", True),
                      "pipeline": ("/api/pipeline", True), "learned": ("/api/learned", True)}
            timing = {}
            for g, (p, signed) in groups.items():
                row_ = {}
                h = app if signed else {}
                for name, base in (("python", py), ("rust", rs)):
                    for _ in range(10):
                        call(base, "GET", p, h)
                    xs = []
                    for _ in range(a.bench):
                        t = time.perf_counter()
                        call(base, "GET", p, h)
                        xs.append((time.perf_counter() - t) * 1000)
                    xs.sort()
                    row_[name] = {"p50_ms": round(statistics.median(xs), 2), "p95_ms": round(xs[int(len(xs) * 0.95) - 1], 2)}
                timing[g] = row_
            report["timing"] = timing
            report["rss_mib"] = {"python": rss_mib(pp.pid), "rust": rss_mib(rp.pid)}
    finally:
        for p in reversed(procs):
            p.terminate()
            try:
                p.wait(10)
            except subprocess.TimeoutExpired:
                p.kill()
        for lg in logs:
            lg.close()
        if not a.keep:
            shutil.rmtree(tmp, ignore_errors=True)
        else:
            print(f"kept {tmp}", file=sys.stderr)
    report["problems"] = problems
    out = json.dumps(report, indent=1)
    print(out)
    if a.report:
        a.report.write_text(out + "\n", encoding="utf-8", newline="\n")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
