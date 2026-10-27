"""Online-first, end to end on this PC, nothing on the server: the hosted API (api/entry.py, auth off on loopback)
on a scratch SurrealDB loaded through the real bundle path, and the home worker in online mode against it.

    python db/tests/test_online.py

  1. pack -> apply (db/push_remote.py) into a temp database; two API pods (entry.py) serve it from their own /tmp.
  2. The worker's workspace (worker/online.py) is rebuilt from the API alone: files, decisions, creative logs.
  3. A job's results (a "YouTube" set's analysis + meta, written by a stand-in pipeline) are posted back, become
     srcfile rows, are synced into the graph tables (the set appears in /api/sets), and EVERY media file the job
     left in the workspace (audio16k.wav, audio caches) is deleted. The set keeps only its YouTube reference.
  4. Pod B (another API process) sees a file pod A received (files_rev -> refresh): the Notes list.
  5. Provenance: an import is by import:<source>; a worker write without an actor is refused; court text is refused.
  6. Cutover: after `push_remote.py cutover`, a bundle load is refused (exit 3); `cutover --undo` rolls it back.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

import common  # noqa: F401
import sdb
from common import DB_DIR, DESK, TempDB, TempKB, free_port

sys.path.insert(0, str(DESK))
FAILS: list[str] = []


def check(ok: bool, msg: str):
    print(("PASS " if ok else "FAIL ") + msg, flush=True)
    if not ok:
        FAILS.append(msg)


class Pod:
    def __init__(self, data: Path, db: TempDB, tag: str):
        self.port = free_port()
        self.url = f"http://127.0.0.1:{self.port}"
        data.mkdir(parents=True, exist_ok=True)
        self.log = open(data.parent / f"pod_{tag}.log", "w", encoding="utf-8")
        env = {**os.environ, "PORT": str(self.port), "HOST": "127.0.0.1", "PK_AUTH": "0", "PK_ALLOW_NO_AUTH": "1",
               "PK_DATA": str(data), "PK_DBLOAD_DIR": str(data / "dbload"), "SURREAL_NS": db.ns, "SURREAL_DB": db.db,
               "PK_BOOT_POLL_S": "1", "PYTHONHASHSEED": "0"}
        env.pop("COMEDY_DIR", None)
        self.proc = subprocess.Popen([sys.executable, str(DESK / "api" / "entry.py")], stdout=self.log,
                                     stderr=subprocess.STDOUT, env=env)
        for _ in range(600):
            if self.proc.poll() is not None:
                raise RuntimeError(f"pod {tag} exited: {Path(self.log.name).read_text()[-2000:]}")
            try:
                with urllib.request.urlopen(self.url + "/api/sets", timeout=5) as r:  # 503 until it serves the data
                    if r.status == 200:
                        return
            except OSError:
                pass
            time.sleep(0.25)
        raise RuntimeError(f"pod {tag} never served the data: {Path(self.log.name).read_text()[-2500:]}")

    def call(self, method, path, body=None):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(self.url + path, data=data, method=method, headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=300) as r:
                return r.status, json.loads(r.read() or b"null")
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read() or b"{}")

    def stop(self):
        self.proc.terminate()
        try:
            self.proc.wait(10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.log.close()


def comedy_from(t: TempKB, root: Path) -> Path:
    """The data-folder layout push_remote.pack reads, from the temp copies."""
    c = root / "comedy"
    (c / "kb" / "v1").mkdir(parents=True)
    shutil.copy(t.graph, c / "kb" / "v1" / "graph.json")
    if t.id_map:
        shutil.copy(t.id_map, c / "kb" / "v1" / "id_map.json")
    shutil.copytree(t.kb, c / "kb" / "v0")
    shutil.copytree(t.corpus, c / "corpus")
    shutil.copytree(t.creative, c / "kb" / "creative")
    return c


def run(args, env, check_rc=True):
    p = subprocess.run([sys.executable, *args], capture_output=True, text=True, encoding="utf-8", env=env)
    if check_rc and p.returncode != 0:
        raise RuntimeError(f"{args[:2]} failed ({p.returncode}):\n{p.stdout[-1500:]}\n{p.stderr[-2500:]}")
    return p


def main():
    root = Path(tempfile.mkdtemp(prefix="punch_online_"))
    t = TempKB("online", decisions=False)
    db = TempDB("online")
    pods = []
    try:
        comedy = comedy_from(t, root)
        env = {**os.environ, "SURREAL_NS": db.ns, "SURREAL_DB": db.db, "PK_DBLOAD_DIR": str(root / "dbload")}
        env.pop("PK_CACHE_DIR", None)
        tar = root / "bundle.tgz"
        run([str(DB_DIR / "push_remote.py"), "pack", "--tar", str(tar), "--comedy", str(comedy)], env)
        r = sdb.Rpc(None, ns=None, db=None)
        r.query(f"DEFINE NAMESPACE IF NOT EXISTS {db.ns}; USE NS {db.ns}; DEFINE DATABASE IF NOT EXISTS {db.db} STRICT;")
        r.close()
        p = run([str(DB_DIR / "push_remote.py"), "apply", "--tar", str(tar)], env)
        out = json.loads(p.stdout[p.stdout.index("{"):])
        check(out["status"] == "loaded", f"bundle loaded into the temp database ({out['srcfile']['files']} srcfiles)")

        a = Pod(root / "podA", db, "A")
        pods.append(a)
        b = Pod(root / "podB", db, "B")
        pods.append(b)
        s, sets0 = a.call("GET", "/api/sets")
        check(s == 200 and sets0["sets"], f"pod A serves the data ({len(sets0['sets'])} sets)")

        # ---- the worker, online-first: a workspace rebuilt from the API only
        import worker.__main__ as W
        from worker import online
        cl = W.Client(a.url, auth=False)
        ws = online.Workspace(cl, root / "workspace")
        ref = ws.refresh()
        check(ref["fetched"] == ref["files"] and ref["files"] > 0 and (ws.comedy / "kb" / "v1" / "graph.json").is_file(),
              f"workspace rebuilt from the API: {ref['fetched']} files, {ref['decisions']} decisions, creative {ref['creative']}")
        ref2 = ws.refresh()
        check(ref2["fetched"] == 0, f"a second refresh downloads nothing ({ref2['fetched']})")

        src = sorted(p for p in (comedy / "corpus" / "sets").iterdir() if (p / "analysis.json").is_file())[0]
        new = "20991230_FakeYT00001"

        class FakePipeline:  # stands in for fetch_youtube + tools/audio/run_all: writes what they write
            @staticmethod
            def import_youtube(inp):
                c = Path(os.environ["COMEDY_DIR"])
                an = json.loads((src / "analysis.json").read_text(encoding="utf-8"))
                an["set"] = "2099-12-30_FakeYT00001"
                (c / "corpus" / "sets" / new).mkdir(parents=True, exist_ok=True)
                (c / "corpus" / "sets" / new / "analysis.json").write_text(json.dumps(an), encoding="utf-8")
                (c / "corpus" / "sets" / new / "meta.json").write_text(json.dumps(
                    {"id": "FakeYT00001", "title": "a set", "upload_date": "20991230"}), encoding="utf-8")
                raw = c / "tapes" / "raw" / new
                raw.mkdir(parents=True, exist_ok=True)
                (raw / "audio16k.wav").write_bytes(b"RIFF" + b"\0" * 4096)
                (raw / "audio.opus").write_bytes(b"OggS" + b"\0" * 1024)
                (Path(os.environ["PK_CACHE_DIR"]) / "audio" / new).mkdir(parents=True, exist_ok=True)
                (Path(os.environ["PK_CACHE_DIR"]) / "audio" / new / "panns.npy").write_bytes(b"\0" * 256)
                return {"status": "imported", "set_dir": new, "video_id": "FakeYT00001"}

        res = online.run_job(cl, ws, {"id": "job:t", "kind": "import_youtube",
                                      "input": {"video_id": "FakeYT00001"}}, execute=FakePipeline)
        pushed = res["online"]["pushed"]
        check(sorted(pushed) == [f"corpus/sets/{new}/analysis.json", f"corpus/sets/{new}/meta.json"],
              f"the job posted exactly its two result files ({pushed})")
        media = [p for p in ws.root.rglob("*") if p.is_file() and p.suffix.lower() in online.MEDIA_EXT]
        check(not media and not (ws.comedy / "tapes").exists() and res["online"]["media_deleted"] >= 3,
              f"no media left in the workspace ({res['online']['media_deleted']} files deleted, {len(media)} left)")
        s, sets1 = a.call("GET", "/api/sets")
        ids = [x.get("id") for x in sets1["sets"]]
        check(len(sets1["sets"]) == len(sets0["sets"]) + 1 and any("FakeYT00001" in str(i) for i in ids),
              f"the new set is in the graph tables at once (sets {len(sets0['sets'])} -> {len(sets1['sets'])})")
        time.sleep(2.5)  # > two boot polls: a sync that dropped load_info.bundle sent the pod back to "not loaded"
        s, _ = a.call("GET", "/api/sets")
        check(s == 200, f"pod A still serves after its own sync (the bundle record survived; {s})")
        rr = sdb.Rpc(None, db.ns, db.db)
        rows = rr.last("SELECT path FROM srcfile WHERE string::contains(path, 'FakeYT');")
        check(len(rows) == 2, f"the results are srcfile rows in the database ({len(rows)})")
        check(not rr.last("SELECT path FROM srcfile WHERE string::ends_with(path, '.wav') OR string::ends_with(path, '.opus');"),
              "no audio in the database")

        # ---- pod B refreshes its cache from files_rev (the Notes list reads memos.json)
        memo_t = {"segments": [{"start": 0, "end": 2, "text": "the pigeon on the subway has a monthly pass"}]}
        memos = {"version": 1, "items": {"drive1": {"id": "drive1", "source": "drive", "drive_id": "drive1",
                                                    "status": "done", "title": "pigeon", "date": "2026-10-20",
                                                    "transcript": "corpus/memos/2026-10-20_abcd1234/transcript.json"}}}
        s, r1 = a.call("POST", "/api/worker/files", {"actor": "import:memo", "files": [
            {"path": "corpus/memos/2026-10-20_abcd1234/transcript.json", "text": json.dumps(memo_t)},
            {"path": "kb/gsync/memos.json", "text": json.dumps(memos)}]})
        check(s == 200 and len(r1["written"]) == 2, f"pod A took the memo files ({s}, files_rev {r1.get('files_rev')})")
        seen = None
        for _ in range(40):
            s, m = b.call("GET", "/api/memos")
            if s == 200 and m.get("items"):
                seen = m["items"][0]
                break
            time.sleep(0.25)
        check(bool(seen) and "pigeon" in (seen or {}).get("text", "") and (seen or {}).get("reference", {}).get("kind") == "drive"
              and seen.get("audio") is None,
              "pod B lists the memo from its refreshed cache, with a Drive reference and no audio")

        # ---- provenance and the legal net through the hosted API
        s, r2 = a.call("POST", "/api/worker/ideas", {"text": "the pigeon commutes better than me", "actor": "import:memo"})
        check(s == 200 and r2["decisions"][0]["by"] == "import:memo", "an imported memo idea is by import:memo")
        s, _ = a.call("POST", "/api/worker/ideas", {"text": "no actor here"})
        check(s == 400, "a worker write without an actor is refused")
        s, _ = a.call("POST", "/api/worker/ideas", {"actor": "import:memo",
                                                    "text": "the petitioner and the respondent, index no 5, sworn to before me"})
        check(s == 422, "court text is refused")
        s, c1 = a.call("POST", "/api/worker/claim", {"kinds": ["embed", "memolink"], "worker": "t"})
        s2, c2 = a.call("POST", "/api/worker/claim", {"kinds": ["embed", "memolink"], "worker": "t"})
        got = sorted(x["job"]["kind"] for x in (c1, c2) if isinstance(x, dict) and x.get("job"))
        check(got == ["embed", "memolink"], f"an imported memo idea queued vector + linking jobs for the worker ({got})")
        s, r3 = a.call("POST", "/api/ideas", {"text": "a jot he typed on his phone about parallel parking"})
        s4, c4 = a.call("POST", "/api/worker/claim", {"kinds": ["embed"], "worker": "t"})
        check(r3["decisions"][0]["by"] == os.environ.get("PK_OWNER_ID", "owner") and s4 == 200
              and (c4 or {}).get("job", {}).get("kind") == "embed",
              f"his own typed idea is by him, and the hosted API queues its vector job ({s4})")
        n = rr.last("RETURN count(SELECT id FROM decision WHERE by = 'import:memo');")
        check(n == 1, f"the database holds exactly that one import:memo line ({n})")

        # ---- the cutover guard
        p = run([str(DB_DIR / "push_remote.py"), "cutover"], env)
        check('"cut_over"' in p.stdout, "cutover marks the database as the source of truth")
        p = run([str(DB_DIR / "push_remote.py"), "apply", "--tar", str(tar), "--force"], env, check_rc=False)
        check(p.returncode == 3 and "online-first cutover" in p.stderr, f"a bundle load after the cutover is refused (exit {p.returncode})")
        p = run([str(DB_DIR / "push_remote.py"), "cutover", "--undo"], env)
        check('"rolled_back"' in p.stdout, "cutover --undo rolls it back")
        p = run([str(DB_DIR / "push_remote.py"), "apply", "--tar", str(tar), "--force"], env, check_rc=False)
        check(p.returncode == 3 and "not in the bundle" in p.stderr,
              "after the rollback the old guard still protects online writes (decisions the bundle lacks)")
        log = rr.last("SELECT cutover_log FROM ONLY meta:state;")
        check(len(sdb.strip_none(log).get("cutover_log") or []) == 2, "both cutover steps are in meta:state.cutover_log")
        rr.close()
    finally:
        for p in pods:
            p.stop()
        db.drop()
        t.cleanup()
        shutil.rmtree(root, ignore_errors=True)
    print(f"\n{'ALL PASS' if not FAILS else f'{len(FAILS)} FAILED'}")
    sys.exit(1 if FAILS else 0)


if __name__ == "__main__":
    main()
