"""The hosted Python API on this PC over a SCRATCH database, at the production data's scale, then pk-load against it
(T17; infra/README.md "Memory"). Never the live database, never the real data: everything is synthetic and lives in
a temp folder that is removed afterwards.

    cargo build --release -p pk-load                      (in rust/)
    PK_BGE_DIR=<bge-small folder> python tools/load/scratch_api.py [--keep] [-- <more pk-load args>]

What runs, in the order production runs it:
  1. A synthetic knowledge base shaped like the hosted one (1,414 bits with 1-3 versions each, 162 typed ideas,
     60 taped sets with 11 moments each, 14 topics, 5 set lists; since T10 round 3 also 20 working sets and a job
     log of 3,050 rows: 1,000 finished jobs and 50 queued), written as the comedy folder layout.
  2. A throwaway `surreal start memory` (SurrealDB 3.3.4 from ~/.punchable/bin) on a free loopback port, with a
     random root password in a temp secrets file (PUNCH_SECRETS), so the real ~/.punchable/secrets.env is never read.
     The namespace, the STRICT database and the EDITOR user `api` are defined as infra/db-init.sh does.
  3. `db/push_remote.py pack` + `apply`: the dbload path, as the database user `api`.
  4. `api/entry.py`: the hosted API's own entry (PK_AUTH off via PK_ALLOW_NO_AUTH=1, loopback only), with the
     bge-small model (PK_BGE_DIR) and its vector sweeper; we wait until the sweeper has embedded everything.
  5. `pk-load --base http://127.0.0.1:<port> --pid <the API's pid> --writes` plus your arguments.
SENTRY_DSN, the Anthropic key and every SURREAL_* variable are removed from the children's environment.
"""
from __future__ import annotations

import argparse
import json
import os
import random
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

DESK = Path(__file__).resolve().parents[2]
PK_LOAD = DESK / "rust" / "target" / "release" / ("pk-load.exe" if os.name == "nt" else "pk-load")

TOPICS = {
    "parking": "meter ticket garage curb tow lot spot valet",
    "airport": "gate boarding security luggage pilot delay lounge carousel",
    "dentist": "floss drill hygienist cavity chair mouthwash crown molar",
    "wedding": "toast bouquet vows caterer bridesmaid dj cake aisle",
    "gym": "treadmill trainer locker protein squat membership mirror sauna",
    "landlord": "lease rent radiator deposit plumber hallway mailbox super",
    "diet": "kale carbs cleanse smoothie calories gluten salad snack",
    "uncle": "thanksgiving recliner grill opinions fishing truck cousins sweater",
    "subway": "platform turnstile conductor delay pole seat transfer express",
    "karaoke": "microphone ballad duet lyrics stage crowd encore key",
    "grocery": "checkout coupon cart aisle produce cashier bagger receipt",
    "printer": "toner jam cartridge driver tray scanner office paper",
    "dating": "profile swipe match brunch texting ghosted dinner bio",
    "neighbor": "fence leafblower barbecue dog lawn package hedge driveway",
}
SHAPES = [
    "My {a} has a {b}, and honestly that is the most {c} thing in my life.",
    "Nobody warns you about the {a}. You walk in thinking {b}, you walk out thinking {c}.",
    "I asked the {a} about the {b} and got a speech about {c}.",
    "If the {a} had a {b} it would be {c}, and I would still pay for it.",
    "The {a} called me about the {b} at six in the morning to talk about {c}.",
    "Every {a} is just a {b} with better {c}.",
]
VENUES = ["The Corner Room", "Seaside Comedy Club", "The Basement", "Laugh Annex", "Back Room Bar", "The Loft"]


def line(rng: random.Random, topic: str) -> str:
    w = TOPICS[topic].split()
    return rng.choice(SHAPES).format(a=rng.choice(w), b=rng.choice(w), c=rng.choice(w))


def write_json(p: Path, obj) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(obj, ensure_ascii=False), encoding="utf-8", newline="\n")


def build_comedy(root: Path, bits: int = 1414, ideas: int = 162, sets: int = 60, per_set: int = 11,
                 jobs_done: int = 1000, jobs_queued: int = 50) -> dict:
    """The synthetic comedy folder (kb/v0, corpus/sets, kb/creative). Deterministic (seeded)."""
    rng = random.Random(7)
    topics = list(TOPICS)
    nodes, edges, texts = [], [], {}
    for t in topics:
        nodes.append({"id": f"label:topic:{t}", "type": "label", "kind": "topic", "name": t.title(),
                      "definition": f"bits about {t}"})
    for i in range(1, bits + 1):
        t = topics[i % len(topics)]
        jid, w = f"joke:b{i:04d}", TOPICS[t].split()
        year = 2015 + i % 10
        nodes.append({"id": jid, "type": "joke", "title": f"{t.title()} {rng.choice(w)} {i}",
                      "stage": rng.choice(["idea", "to_try", "testing", "working"]), "summary": f"A bit about {t}.",
                      "first_seen": f"{year}-01-01", "last_seen": f"{year}-06-01", "lead_unit": f"unit:b{i:04d}_1"})
        for v in range(1, rng.randint(1, 3) + 1):
            uid = f"unit:b{i:04d}_{v}"
            text = " ".join(line(rng, t) for _ in range(rng.randint(2, 4)))
            texts.setdefault(jid, text)
            nodes.append({"id": uid, "type": "unit", "kind": "setup+punch", "text": text, "date": f"{year}-0{v}-15",
                          "source": "gdoc", "doc_title": "Notes"})
            edges.append({"id": f"e:p{i}_{v}", "src": uid, "dst": jid, "kind": "part_of", "status": "inferred",
                          "confidence": 0.9, "evidence": []})
        if i % 3 == 0:
            edges.append({"id": f"e:l{i}", "src": jid, "dst": f"label:topic:{t}", "kind": "has_label",
                          "status": "proposed", "confidence": 0.8, "reason": t, "evidence": []})
    for i in range(1, ideas + 1):
        t = topics[(i * 5) % len(topics)]
        nodes.append({"id": f"joke:i{i:03d}", "type": "joke", "title": f"Idea about {t} {i}", "stage": "idea",
                      "summary": line(rng, t), "first_seen": "2026-09-01", "last_seen": "2026-09-01"})
    for s in range(sets):
        key = f"{2021 + s // 12}-{1 + s % 12:02d}-{1 + s % 27:02d}_night{s:02d}"
        sdir = key[:4] + key[5:7] + key[8:10] + key[10:]
        venue = VENUES[s % len(VENUES)]
        nodes.append({"id": f"set:{key}", "type": "set", "title": f"{venue}, night {s}", "venue": venue,
                      "date": key[:10], "duration_s": per_set * 60})
        segs, rows = [], []
        for k in range(per_set):
            jid = f"joke:b{1 + (s * 37 + k * 11) % bits:04d}"
            start = k * 60_000
            pid = f"perf:{key}@{start}"
            text = texts[jid]
            nodes.append({"id": pid, "type": "performance", "set": f"set:{key}", "start_ms": start,
                          "end_ms": start + 55_000, "text": text, "kind": "bit", "name": jid[5:]})
            edges.append({"id": f"e:pa{s}_{k}", "src": pid, "dst": jid, "kind": "performed_as", "status": "proposed",
                          "confidence": 0.8, "reason": "same words", "evidence": []})
            segs.append({"start": start / 1000, "end": start / 1000 + 55, "text": text})
            laugh = round(rng.uniform(0, 12), 1)
            rows.append({"idx": k + 1, "start_ms": start, "end_ms": start + 55_000, "title": f"T{k + 1}",
                         "status": "known", "joke_id": jid, "version_unit_id": f"unit:{jid[5:]}_1",
                         "match_confidence": 0.9, "match_reason": "same words", "transcript": text, "punchlines": [],
                         "laughs": {"count": 1, "max_bars": 2, "total_s": laugh, "merged_count": 1, "laugh_s": laugh,
                                    "window_s": 55.0},
                         "laugh_s_per_min": round(laugh / (55 / 60), 2), "pct_laughing": round(laugh / 55, 3),
                         "time_to_first_laugh_s": 5.0, "dead_air_max_s": 4.0, "vs_room": 1.0,
                         "ai_rating": "worked" if laugh > 4 else "died", "rating_basis": "the laugh after the punch",
                         "similar": [], "hint_labels": [], "script_diff": {"on_script_pct": 80, "dropped": [], "added": []}})
        d = root / "corpus" / "sets" / sdir
        write_json(d / "transcript.json", {"segments": segs})
        write_json(d / "meta.json", {"title": f"{venue}, night {s}", "venue_guess": venue, "show_date": key[:10],
                                     "show_date_source": "the folder date", "duration": per_set * 60.0})
        n_laughs = int(os.environ.get("PK_SCRATCH_LAUGHS", "0"))  # production scale: a few hundred laughs per taped set
        laughs = [{"id": f"l{x}", "start_ms": x * 2000, "end_ms": x * 2000 + 1500, "strength": round(rng.uniform(0, 1), 3),
                   "bars": rng.randint(1, 4), "kind": "laugh", "confidence": 0.9, "segment": f"s{x // 10}"} for x in range(n_laughs)]
        write_json(d / "laughs_auto.json", {"detector": "synthetic", "run_id": "laughs-1", "laughs": laughs})
        write_json(d / "analysis.json", {
            "set": key, "run_id": "synthetic-1", "generated_at": "2026-10-01T00:00:00+00:00",
            "summary": {"laugh_s_per_min": 5.0, "room_heat": 1.0, "room_heat_basis": "set", "material_s": per_set * 55.0,
                        "laugh_s_total": 30.0, "pct_laughing": 0.08, "laughs_merged": per_set, "laughs_per_min_merged": 1.0,
                        "applause_s": 0.0, "longest_laugh_s": 3.0, "longest_laugh_joke": rows[0]["joke_id"],
                        "dead_air_max_s": 4.0, "dead_air_max_at_ms": 0, "duration_s": per_set * 60, "best": None,
                        "weakest": None, "jokes_counted": per_set, "new_material": 0},
            "jokes": rows})
    # production scale (PK_SCRATCH_EDGES): more links among the bits, up to this many edges in all
    want_edges = int(os.environ.get("PK_SCRATCH_EDGES", "0"))
    k = 0
    while len(edges) < want_edges:
        a, b = 1 + (k * 7) % bits, 1 + (k * 13 + 5) % bits
        if a != b:
            edges.append({"id": f"e:x{k}", "src": f"joke:b{a:04d}", "dst": f"joke:b{b:04d}", "kind": "same_premise",
                          "status": "proposed", "confidence": 0.5, "reason": "a shared premise", "evidence": []})
        k += 1
    write_json(root / "kb" / "v0" / "graph.json",
               {"version": "load-test", "generated_at": "2026-10-01", "runs": ["r1"], "nodes": nodes, "edges": edges})
    (root / "kb" / "v0" / "decisions.jsonl").write_text("", encoding="utf-8", newline="\n")
    write_json(root / "kb" / "v0" / "inferred.json", {"run_id": "inf-1", "generated_at": "2026-10-01T00:00:00+00:00",
                                                      "notes": "synthetic", "jokes": []})
    boards = []
    for b in range(5):
        ids = [f"joke:b{1 + (b * 101 + k * 13) % bits:04d}" for k in range(8)]
        boards.append(json.dumps({"ts": f"2026-10-0{b + 1}T20:00:00+00:00", "op": "create", "id": f"sb:load-{b}",
                                  "by": "owner", "data": {"title": f"Set list {b}", "joke_ids": ids, "target_s": 900,
                                                          "notes": "", "seed": None}}))
    (root / "kb" / "creative").mkdir(parents=True, exist_ok=True)
    (root / "kb" / "creative" / "storyboards.jsonl").write_text("\n".join(boards) + "\n", encoding="utf-8", newline="\n")
    # T10 round 3: his sets (one per set list plus fifteen more, each promoted with eight bits) and a job log at the
    # production scale the row named (a few thousand rows: a thousand finished jobs, enqueue / claim / done, and fifty
    # still queued), so the working-set routes, the claim and the enqueue routes replay what the server replays
    wsets = []
    for w in range(20):
        ids = [f"joke:b{1 + (w * 53 + k * 17) % bits:04d}" for k in range(8)]
        wsets.append(json.dumps({"ts": f"2026-10-{1 + w % 28:02d}T21:00:00+00:00", "op": "promote", "id": f"ws:load-{w}", "by": "owner",
                                 "data": {"title": f"Set {w}", "storyboard_id": f"sb:load-{w}" if w < 5 else None, "target_s": 600,
                                          "bits": [{"joke_id": j, "title": f"Bit {j[6:]}", "target_s": 75} for j in ids]}}))
    (root / "kb" / "creative" / "workingsets.jsonl").write_text("\n".join(wsets) + "\n", encoding="utf-8", newline="\n")
    jobs = []
    kinds = ["riff", "punchup", "ask_chat", "bridge", "facts"]
    for j in range(jobs_done + jobs_queued):
        kind = kinds[j % len(kinds)]
        jid = f"job:{kind}-{j:012x}"
        day = f"2026-{1 + j % 9:02d}-{1 + j % 27:02d}"
        jobs.append(json.dumps({"ts": f"{day}T10:00:00+00:00", "op": "enqueue", "id": jid, "by": "owner",
                                "data": {"kind": kind, "key": f"{kind}|load-{j}", "input": {"joke": f"joke:b{1 + j % bits:04d}", "n": j}}}))
        if j < jobs_done:
            jobs.append(json.dumps({"ts": f"{day}T10:00:05+00:00", "op": "claim", "id": jid, "by": "worker:load", "data": {"worker": "load"}}))
            jobs.append(json.dumps({"ts": f"{day}T10:00:40+00:00", "op": "done", "id": jid, "by": "worker",
                                    "data": {"result": {"ideas": [{"text": f"A drafted line number {j} about the {TOPICS[topics[j % len(topics)]].split()[0]}."}],
                                                        "model": "load-test"}}}))
    (root / "kb" / "creative" / "jobs.jsonl").write_text("\n".join(jobs) + "\n", encoding="utf-8", newline="\n")
    return {"bits": bits + ideas, "units": sum(1 for n in nodes if n["type"] == "unit"), "sets": sets,
            "moments": sets * per_set, "working_sets": len(wsets), "job_rows": len(jobs), "jobs_queued": jobs_queued}


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def get(url: str, timeout: float = 5):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.loads(r.read())


def hold(path: Path, info: dict) -> int:
    """`--hold`: publish where the servers are, then wait until the caller deletes the file."""
    path.write_text(json.dumps(info), encoding="utf-8")
    print(f"holding: {info} (delete {path} to stop)", flush=True)
    while path.exists():
        time.sleep(1)
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--keep", action="store_true", help="leave the temp folder (logs) behind")
    ap.add_argument("--rust", type=Path, default=None,
                    help="a pk-api binary: after the Python run, run pk-load against it over the same database (T9)")
    ap.add_argument("--hold", type=Path, default=None,
                    help="run no load: start the Python (and pk-api with --rust), write their bases and pids to this "
                         "JSON file, and keep both up until the file is deleted (for probes: tools/load/forward_probe.py)")
    ap.add_argument("rest", nargs=argparse.REMAINDER, help="-- then arguments for pk-load")
    a = ap.parse_args()
    bge = os.environ.get("PK_BGE_DIR")
    if not bge or not (Path(bge) / "onnx" / "model.onnx").is_file():
        raise SystemExit("set PK_BGE_DIR to the bge-small folder (python tools/kb/fetch_bge.py --dest <dir>)")
    if not PK_LOAD.is_file():
        raise SystemExit(f"no {PK_LOAD}: cargo build --release -p pk-load (in rust/)")
    root = Path(tempfile.mkdtemp(prefix="pk-load-"))
    sport, aport = free_port(), free_port()
    root_pass, api_pass = secrets.token_hex(16), secrets.token_hex(16)
    (root / "secrets.env").write_text(f"SURREAL_USER=root\nSURREAL_PASS={root_pass}\nSURREAL_PORT={sport}\n",
                                      encoding="utf-8")
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("SURREAL_", "SENTRY_", "ANTHROPIC_", "PK_", "PUNCH", "OIDC_", "COMEDY"))}
    env.update(PUNCH_SECRETS=str(root / "secrets.env"), PUNCH_COMEDY=str(root / "comedy"),
               PUNCHABLE_HOME=str(root / "home"), PK_CACHE_DIR=str(root / "cache"), PK_OWNER_ID="owner",
               PK_DBLOAD_DIR=str(root / "dbload"),
               PYTHONHASHSEED="0", PYTHONUTF8="1")
    # the one setting a run may ask for: the uploads in the database (docs/adr/0014), on both servers or neither
    env.update({k: v for k, v in os.environ.items() if k == "PK_UPLOAD_DB"})
    os.environ.clear()
    os.environ.update(env)  # sdb reads PUNCH_SECRETS at import: only the scratch credentials exist from here
    sys.path.insert(0, str(DESK / "db"))
    import sdb  # noqa: E402

    procs: list[subprocess.Popen] = []
    log = None
    try:
        t0 = time.time()
        shape = build_comedy(root / "comedy")
        print(f"synthetic data: {shape} ({time.time() - t0:.1f}s)", flush=True)
        procs.append(sdb.start_scratch(sport, "memory", log_path=root / "surreal.log"))
        r = sdb.Rpc(sport, ns=None, db=None)
        try:  # infra/db-init.sh: the STRICT database first, then the database-level EDITOR user
            r.query("DEFINE NAMESPACE IF NOT EXISTS punchable; USE NS punchable; DEFINE DATABASE IF NOT EXISTS main STRICT;"
                    f" USE NS punchable DB main; DEFINE USER OVERWRITE api ON DATABASE PASSWORD '{api_pass}' ROLES EDITOR;")
        finally:
            r.close()
        db_env = {**env, "SURREAL_URL": f"ws://127.0.0.1:{sport}/rpc", "SURREAL_NS": "punchable", "SURREAL_DB": "main",
                  "SURREAL_API_USER": "api", "SURREAL_API_PASS": api_pass}
        tar = root / "bundle.tgz"
        for cmd in (["pack", "--tar", str(tar), "--comedy", str(root / "comedy")], ["apply", "--tar", str(tar)]):
            t = time.time()
            p = subprocess.run([sys.executable, str(DESK / "db" / "push_remote.py"), *cmd], env=db_env,
                               capture_output=True, text=True, encoding="utf-8")
            if p.returncode != 0:
                raise SystemExit(f"push_remote {cmd[0]} failed:\n{p.stdout[-2000:]}\n{p.stderr[-3000:]}")
            print(f"push_remote {cmd[0]}: ok ({time.time() - t:.1f}s)", flush=True)
        api_env = {**db_env, "PORT": str(aport), "HOST": "127.0.0.1", "PK_AUTH": "0", "PK_ALLOW_NO_AUTH": "1",
                   "PK_DATA": str(root / "data"), "PK_BGE_DIR": bge, "MALLOC_ARENA_MAX": "2", "PUNCH_API_QUIET": "1"}
        log = open(root / "api.log", "w", encoding="utf-8")
        api = subprocess.Popen([sys.executable, str(DESK / "api" / "entry.py")], env=api_env, stdout=log,
                               stderr=subprocess.STDOUT)
        procs.append(api)
        base = f"http://127.0.0.1:{aport}"
        t = time.time()
        while True:  # loaded, then the sweeper's first pass done (every unit, idea and moment has its vector)
            if api.poll() is not None:
                raise SystemExit(f"the API exited {api.returncode}: {(root / 'api.log').read_text()[-3000:]}")
            if time.time() - t > 1800:
                raise SystemExit("the API did not load and embed within 30 minutes")
            try:
                h = get(base + "/api/health")
                vec = (get(base + "/api/backend").get("backends", {}).get("surreal") or {}).get("vectors") or {}
                if h.get("ok") and vec.get("runs", 0) >= 1:
                    break
            except OSError:
                pass
            time.sleep(2)
        print(f"API pid {api.pid} on {base}: loaded and embedded ({time.time() - t:.0f}s, sweep {vec.get('last')})", flush=True)
        time.sleep(10)  # settle before the idle sample
        rest = [x for x in a.rest if x != "--"]
        rc = 0 if a.hold else subprocess.run([str(PK_LOAD), "--base", base, "--pid", str(api.pid), "--writes", *rest]).returncode
        if not a.rust and a.hold:
            return hold(a.hold, {"python": base, "python_pid": api.pid})
        if not a.rust:
            return rc
        # T9: pk-api over the same database, everything it does not answer forwarded to this Python; the same
        # pk-load run (its routes not ported yet measure the forward plus the Python, so read only the ported ones)
        rport = free_port()
        rs_env = {**db_env, "PORT": str(rport), "HOST": "127.0.0.1", "PK_AUTH": "0", "PK_ALLOW_NO_AUTH": "1",
                  "PK_DATA": str(root / "data"), "PK_PYTHON_URL": base, "RUST_LOG": "info",
                  # round 6: pk-api searches with the same model (loaded on its first search)
                  "PK_BGE_DIR": bge}
        rlog = open(root / "pk-api.log", "w", encoding="utf-8")
        rs = subprocess.Popen([str(a.rust)], env=rs_env, stdout=rlog, stderr=subprocess.STDOUT)
        procs.append(rs)
        rbase = f"http://127.0.0.1:{rport}"
        t = time.time()
        while True:
            if rs.poll() is not None:
                raise SystemExit(f"pk-api exited {rs.returncode}")
            if time.time() - t > 300:
                raise SystemExit("pk-api did not become ready")
            try:
                with urllib.request.urlopen(rbase + "/api/health?ready=1", timeout=5) as resp:
                    if resp.status == 200:
                        break
            except OSError:
                pass
            time.sleep(1)
        print(f"pk-api pid {rs.pid} on {rbase}: ready ({time.time() - t:.0f}s)", flush=True)
        if a.hold:
            return hold(a.hold, {"python": base, "python_pid": api.pid, "rust": rbase, "rust_pid": rs.pid,
                                 "rust_log": str(root / "pk-api.log")})
        time.sleep(10)
        rc2 = subprocess.run([str(PK_LOAD), "--base", rbase, "--pid", str(rs.pid), "--writes", *rest]).returncode
        rlog.close()
        return rc or rc2
    finally:
        for p in reversed(procs):
            p.terminate()
            try:
                p.wait(15)
            except subprocess.TimeoutExpired:
                p.kill()
        if log is not None:
            log.close()  # Windows will not delete a file this process still holds open
        if a.keep:
            print(f"kept {root}")
        else:
            shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
