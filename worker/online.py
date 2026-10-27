"""Online-first mode for the home worker (PK_ONLINE_FIRST=1, or `python -m worker --online`).

The hosted database is the source of truth. This PC is a GPU compute worker: for each job it

  1. refreshes a disposable WORKSPACE from the API (the text files as srcfile rows, decisions, creative logs,
     vectors): <PK_WORKSPACE> (default <PK_CACHE_DIR>/worker), never the comic's data folder;
  2. downloads only what the job needs (an uploaded recording, a YouTube video's audio) into that workspace;
  3. runs the same pipelines as before with COMEDY_DIR pointing at the workspace;
  4. posts every result file that changed (db/srcpaths.WORKER_PATTERNS, checked against the legal rules first) to
     POST /api/worker/files, and ideas/decisions/events through their worker routes, each with its actor;
  5. deletes all media from the workspace (tapes/, notes/inbox, *.wav/*.opus/..., the audio stage caches).

Nothing in the workspace is the truth: delete it at any time and the next job rebuilds it from the API. Its
manifest records what was downloaded (path -> sha256), so a refresh fetches only what changed.

What is kept of a recording: the transcript, the jokes/units and laughs (analysis, laughs_auto, clips), and a
REFERENCE: a YouTube set's video id (meta.json `id`; playback = youtube.com/watch?v=<id>&t=<s>), a voice memo's Drive
file id, an upload's original file name and date. The audio itself is not kept anywhere.
"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import sys
import time
import urllib.parse
from pathlib import Path

DESK = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(DESK / "db"))
import srcpaths as SP  # noqa: E402

MEDIA_EXT = {".wav", ".opus", ".m4a", ".mp3", ".mp4", ".mov", ".webm", ".aac", ".flac", ".ogg", ".caf", ".amr", ".3gp",
             ".m4v", ".mkv", ".part", ".ytdl"}
VEC_CACHES = {"vectors.f32": "vectors_ids.json", "vectors_v1.f32": "vectors_v1_ids.json"}
POST_CHUNK = 12 * 1024 * 1024
LOCAL_OWNED = ("kb/gsync/",)  # gsync's own state: written on this PC, pushed by gsync; downloaded only when missing
ACTOR = {"import_youtube": "import:youtube", "process_set": "import:upload", "embed": "ai:embed",
         "memolink": "ai:memolink"}


def log(*a):
    print(time.strftime("%Y-%m-%d %H:%M:%S"), *a, flush=True)


def enabled() -> bool:
    return os.environ.get("PK_ONLINE_FIRST") == "1"


def default_root() -> Path:
    if os.environ.get("PK_WORKSPACE"):
        return Path(os.environ["PK_WORKSPACE"])
    if os.environ.get("PK_CACHE_DIR"):
        return Path(os.environ["PK_CACHE_DIR"]) / "worker"
    return Path.home() / ".punchable-worker"


def _sha_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def _write(p: Path, text: str):
    p.parent.mkdir(parents=True, exist_ok=True)
    tmp = p.with_name(p.name + ".tmp")
    with open(tmp, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    os.replace(tmp, p)


class Workspace:
    def __init__(self, cl, root: Path | None = None, legal=None):
        self.cl = cl
        self.root = Path(root or default_root())
        self.comedy = self.root / "comedy"
        self.cache = self.root / "cache"
        self.vec = self.cache / "kb"
        self.man_path = self.root / "manifest.json"
        self._legal = legal

    # ------------------------------------------------------------------ environment
    def env(self) -> dict:
        """What a job sees: COMEDY_DIR is the workspace; caches go under it; the bge model stays where it is."""
        out = {"COMEDY_DIR": str(self.comedy), "PUNCH_COMEDY": str(self.comedy), "PK_CACHE_DIR": str(self.cache),
               "PUNCHABLE_CREATIVE_DIR": str(self.comedy / "kb" / "creative"),
               "PK_INFER_JUDGE_DIR": str(self.comedy / "kb" / "infer_judge")}
        sys.path.insert(0, str(DESK))
        from engine.config import bge_dir_override
        bge = bge_dir_override()
        if bge:
            out["PK_BGE_DIR"] = str(bge)
        return out

    def activate(self):
        os.environ.update(self.env())

    def legal(self, text: str) -> bool:
        if self._legal is None:
            import importlib.util
            spec = importlib.util.spec_from_file_location("pk_legal_rules", DESK / "gsync" / "rules.py")
            mod = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(mod)
            self._legal = mod.content_is_legal
        return self._legal(text)

    # ------------------------------------------------------------------ manifest
    def manifest(self) -> dict:
        try:
            return json.loads(self.man_path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return {"files": {}}

    def save_manifest(self, m: dict):
        _write(self.man_path, json.dumps(m, indent=0, sort_keys=True))

    def local(self, rel: str) -> Path:
        """Where a srcfile path lives in the workspace. vectors/extra_*.json sit with the vector caches."""
        if rel.startswith("vectors/"):
            return self.vec / rel[len("vectors/"):]
        return self.comedy / rel

    # ------------------------------------------------------------------ down
    def refresh(self) -> dict:
        """Bring the workspace up to date with the API. Fetches only files whose sha changed."""
        self.comedy.mkdir(parents=True, exist_ok=True)
        m = self.manifest()
        have = m.setdefault("files", {})
        listing = {f["path"]: f["sha256"] for f in self.cl.call("GET", "/api/worker/files")["files"]}
        got, vec_parts = 0, False
        for rel, sha in sorted(listing.items()):
            rel = SP.clean(rel)
            if not rel:
                continue
            p = self.local(rel) if not rel.startswith("vectors/") or rel.startswith("vectors/extra_") else None
            if rel.startswith(LOCAL_OWNED) and p is not None and p.is_file():
                continue  # gsync's own state: this PC is its writer
            if have.get(rel) == sha and (p is None or p.is_file()):
                continue
            d = self.cl.call("GET", "/api/worker/files/content?path=" + urllib.parse.quote(rel))
            if p is None:  # a vector part: kept as JSON next to the caches, rebuilt into .f32 below
                p = self.cache / "parts" / Path(rel).name
                vec_parts = True
            _write(p, d["text"])
            have[rel] = sha
            got += 1
        removed = 0
        for rel in [r for r in have if r not in listing]:
            if rel.startswith(LOCAL_OWNED):
                continue
            p = self.local(rel) if not rel.startswith("vectors/") or rel.startswith("vectors/extra_") else \
                self.cache / "parts" / Path(rel).name
            p.unlink(missing_ok=True)
            have.pop(rel, None)
            removed += 1
            vec_parts = vec_parts or (rel.startswith("vectors/") and not rel.startswith("vectors/extra_"))
        if vec_parts:
            self.rebuild_vectors()
        n = self.pull_decisions()
        cre = self.pull_creative()
        m["refreshed_at"] = time.strftime("%Y-%m-%dT%H:%M:%S")
        self.save_manifest(m)
        return {"fetched": got, "removed": removed, "files": len(listing), "decisions": n, "creative": cre}

    def rebuild_vectors(self):
        import numpy as np
        parts = sorted((self.cache / "parts").glob("*.part*.json"))
        acc: dict[str, tuple[str, list, list]] = {}
        for p in parts:
            d = json.loads(p.read_text(encoding="utf-8"))
            ids_file, ids, rows = acc.setdefault(d["cache"], (d["ids_file"], [], []))
            ids += d["ids"]
            rows += d["rows"]
        self.vec.mkdir(parents=True, exist_ok=True)
        for cache, (ids_file, ids, rows) in acc.items():
            np.asarray(rows, dtype=np.float32).tofile(self.vec / (cache + ".tmp"))
            os.replace(self.vec / (cache + ".tmp"), self.vec / cache)
            _write(self.vec / ids_file, json.dumps(ids))

    def pull_decisions(self) -> int:
        out, after = [], 0
        while True:
            page = self.cl.call("GET", f"/api/worker/decisions?after={after}")
            out += [x["decision"] for x in page["lines"]]
            after += len(page["lines"])
            if not page["lines"] or after >= page["total"]:
                break
        _write(self.comedy / "kb" / "v0" / "decisions.jsonl",
               "".join(json.dumps(d, ensure_ascii=False, separators=(",", ":")) + "\n" for d in out))
        return len(out)

    def pull_creative(self) -> dict:
        out = {}
        for name in ("storyboards", "workingsets", "venues", "merges", "suggestions"):
            try:
                lines = self.cl.call("GET", f"/api/worker/creative?store={name}")["lines"]
            except RuntimeError as e:  # an older API without that store
                out[name] = f"skipped: {str(e)[:80]}"
                continue
            p = self.comedy / "kb" / "creative" / f"{name}.jsonl"
            _write(p, "".join(x + "\n" for x in lines))
            (p.with_suffix(".json")).unlink(missing_ok=True)  # the snapshot is derived: rebuilt from the log
            out[name] = len(lines)
        return out

    # ------------------------------------------------------------------ up
    def snapshot(self) -> dict:
        """rel -> sha256 of every file here the worker may post."""
        out = {}
        if self.comedy.is_dir():
            for p in self.comedy.rglob("*"):
                if p.is_file() and p.suffix.lower() in SP.TEXT_SUFFIXES:
                    rel = p.relative_to(self.comedy).as_posix()
                    if SP.worker_may_write(rel):
                        out[rel] = _sha_bytes(p.read_bytes())
        if self.vec.is_dir():
            for p in self.vec.glob("extra_*.json"):
                rel = "vectors/" + p.name
                if SP.worker_may_write(rel):
                    out[rel] = _sha_bytes(p.read_bytes())
        return out

    def push(self, before: dict, actor: str, only: list[str] | None = None) -> dict:
        """Post every allowed file that changed since `before` (or the paths in `only`). Legal hits are deleted here
        and never sent."""
        after = self.snapshot()
        rels = [r for r in (only or sorted(after)) if r in after and before.get(r) != after[r]]
        sent, refused, batch, size = [], [], [], 0
        m = self.manifest()
        have = m.setdefault("files", {})

        def flush():
            nonlocal batch, size
            if batch:
                r = self.cl.call("POST", "/api/worker/files", {"files": batch, "actor": actor})
                for f in batch:
                    have[f["path"]] = f["sha256"]
                sent.extend(r.get("written") or [])
            batch, size = [], 0

        for rel in rels:
            p = self.local(rel)
            text = p.read_text(encoding="utf-8")
            if self.legal(text):
                p.unlink(missing_ok=True)
                refused.append(rel)
                continue
            b = len(text.encode("utf-8"))
            if size + b > POST_CHUNK:
                flush()
            batch.append({"path": rel, "text": text, "sha256": after[rel]})
            size += b
        flush()
        self.save_manifest(m)
        return {"sent": sent, "unchanged": len(after) - len(rels), "legal_refused": refused}

    # ------------------------------------------------------------------ media
    def purge_media(self) -> int:
        """Delete every recording from the workspace: tapes/, notes/inbox, media files anywhere, audio stage caches."""
        n = 0
        for d in (self.comedy / "tapes", self.comedy / "notes" / "inbox", self.cache / "audio"):
            if d.exists():
                n += sum(1 for p in d.rglob("*") if p.is_file())
                shutil.rmtree(d, ignore_errors=True)
        for p in list(self.root.rglob("*")):
            if p.is_file() and p.suffix.lower() in MEDIA_EXT:
                p.unlink(missing_ok=True)
                n += 1
        return n


# ---------------------------------------------------------------------------------------------------------- jobs
def run_job(cl, ws: Workspace, job: dict, file: Path | None = None, execute=None) -> dict:
    """One job in online-first mode: refresh, run on the workspace, post what changed, purge media."""
    if execute is None:
        from worker import execute
    ws.activate()
    ref = ws.refresh()
    before = ws.snapshot()
    kind = job["kind"]
    actor = ACTOR.get(kind, "ai:" + kind)
    try:
        if kind == "process_set":
            res = process_upload(cl, ws, job, file)
        elif kind == "import_youtube":
            res = execute.import_youtube(job.get("input") or {})
        elif kind == "embed":
            res = embed_job(ws)
        elif kind == "memolink":
            res = memolink_job(cl, ws, job)
        else:
            res = execute.run(job, file=file)
        pushed = ws.push(before, actor)
    finally:
        purged = ws.purge_media()
    if isinstance(res, dict):
        res = {**res, "online": {"refresh": {k: ref[k] for k in ("fetched", "files", "decisions")},
                                 "pushed": pushed["sent"], "legal_refused": len(pushed["legal_refused"]),
                                 "media_deleted": purged}}
    return res


def embed_job(ws: Workspace) -> dict:
    from creative import memolink as ML
    from creative.data import Corpus
    c = Corpus.load(ws.comedy)
    return {"status": "embedded", **ML.embed_summaries(c, ws.vec)}


def memolink_job(cl, ws: Workspace, job: dict) -> dict:
    """embed -> judge (merge_judge) -> triage -> clear attaches posted as ai:attach events (undoable)."""
    from creative import memolink as ML
    from creative import merge as MG
    from creative.data import Corpus
    from creative.store import Store
    from creative.story import _creative_complete
    steps = (job.get("input") or {}).get("steps") or ["embed", "judge", "triage", "attach"]
    c = Corpus.load(ws.comedy)
    out = {"status": "linked"}
    if "embed" in steps:
        out["embed"] = ML.embed_summaries(c, ws.vec)
        c = Corpus.load(ws.comedy)
    ms = ML.memos(ws.comedy)
    mj = ML.memo_jokes(c, ms)
    cdir = ws.comedy / "kb" / "creative"
    complete = _creative_complete() if ("judge" in steps or "triage" in steps) else None
    if "judge" in steps:
        out["judge"] = ML.judge_memos(c, cdir, mj, complete, log)
    if "triage" in steps:
        out["triage"] = ML.triage(ms, cdir / "suggest_judge.json", complete, log)
    st = Store(cdir, "merges", MG.store_apply)
    if "attach" in steps:
        done = []
        for x in ML.clear_attaches(c, cdir, mj, st.items):
            aid, data = ML.attach_event(x, mj.get(x["memo_joke"]))
            r = cl.call("POST", "/api/worker/events", {"store": "merges", "op": "attach", "id": aid, "data": data,
                                                       "actor": "ai:attach"})
            if not r.get("already"):
                done.append(aid)
        out["attached"] = done
    out["same_idea_groups"] = len(ML.suggestions(c, cdir, mj, st.items))
    return out


def process_upload(cl, ws: Workspace, job: dict, file: Path | None) -> dict:
    """An upload from the app (staged on the server until this job finishes; the API deletes it on our result).
    A voice note -> transcript + an idea (import:upload). A set -> the set pipeline (transcript, laughs, analysis)."""
    if not file or not Path(file).is_file():
        raise FileNotFoundError("the uploaded file is not here")
    inp = job.get("input") or {}
    meta = inp.get("meta") or {}
    kind = meta.get("kind") or "upload"
    sys.path.insert(0, str(DESK))
    from gsync import memos as GM
    pr = GM.probe(Path(file))
    dur = float(pr.get("duration_s") or meta.get("duration_s") or 0)
    if kind == "note" or (kind == "upload" and dur < 240):
        return upload_note(cl, ws, Path(file), meta, pr)
    return upload_set(ws, Path(file), meta, pr)


def upload_note(cl, ws: Workspace, path: Path, meta: dict, pr: dict, transcriber=None, normalizer=None) -> dict:
    from gsync import memos as GM
    sha = GM.sha256_file(path)
    at, date, src = GM.recorded(pr.get("tags") or {}, meta.get("original_name") or path.name, meta.get("recorded_at"),
                                path.stat().st_mtime)
    if meta.get("recorded_at"):
        at, date, src = meta["recorded_at"], meta["recorded_at"][:10], "the app (when it was recorded)"
    name = f"{date or 'undated'}_{sha[:8]}"
    work = ws.comedy / "notes" / "inbox" / name
    tdir = ws.comedy / "corpus" / "memos" / name
    wav = work / "audio16k.wav"
    if normalizer is None:
        from engine.audio import normalize as normalizer
    normalizer(path, wav)
    if transcriber is None:
        from engine.audio import Transcriber
        from engine.config import Config as EC
        transcriber = Transcriber(EC(), batch_size=4)
    tx = transcriber(wav, tdir)
    text = GM.transcript_text(tx)
    if ws.legal(text):
        shutil.rmtree(tdir, ignore_errors=True)
        return {"status": "skipped_legal"}
    title = (meta.get("title") or Path(meta.get("original_name") or path.name).stem)[:120]
    m = {"kind": "voice_note", "source": "app_upload", "title": title, "recorded_at": at, "date": date,
         "date_source": src, "duration_s": pr.get("duration_s"), "sha256": sha,
         "original_name": meta.get("original_name"), "received_at": meta.get("received_at")}
    _write(tdir / "meta.json", json.dumps(m, ensure_ascii=False, indent=1))
    out = {"status": "processed", "kind": "voice_note", "memo": name, "chars": len(text)}
    if text and len(text) <= SP_IDEA_MAX:
        r = cl.call("POST", "/api/worker/ideas", {
            "text": text, "title": GM.idea_title(title) or "", "actor": "import:upload",
            "from": {"note": f"Voice note '{title}', recorded {date or 'unknown'}"},
            "reason": f"[upload] voice note '{title}' (upload {sha[:12]}, recorded {date or 'unknown'})"})
        out.update(joke_id=r.get("id"), attach=(r.get("attach") or {}).get("decision"))
    else:
        out["long" if text else "no_speech"] = True
    return out


SP_IDEA_MAX = 4000


def upload_set(ws: Workspace, path: Path, meta: dict, pr: dict, runner=None) -> dict:
    """tapes/raw/<date>_<sha8>/audio16k.wav + corpus/sets/<set>/meta.json, then tools/audio/run_all (transcribe,
    laughs, VAD, then infer.sets + infer.clips), exactly as a YouTube set. The audio is purged after the job."""
    from gsync import memos as GM
    from engine.audio import normalize
    sha = GM.sha256_file(path)
    at, date, src = GM.recorded(pr.get("tags") or {}, meta.get("original_name") or path.name, meta.get("recorded_at"),
                                path.stat().st_mtime)
    if meta.get("recorded_at"):
        at, date = meta["recorded_at"], meta["recorded_at"][:10]
    set_dir = f"{(date or '00000000').replace('-', '')}_up{sha[:9]}"
    normalize(path, ws.comedy / "tapes" / "raw" / set_dir / "audio16k.wav")
    m = {"id": f"up{sha[:9]}", "title": meta.get("title") or Path(meta.get("original_name") or path.name).stem,
         "source": "app_upload", "original_name": meta.get("original_name"), "recorded_at": at,
         "show_date": date, "show_date_source": "the recording's date", "show_venue": meta.get("venue"),
         "gps": meta.get("gps"), "show_group": meta.get("show_group"), "duration_s": pr.get("duration_s"),
         "received_at": meta.get("received_at"), "sha256": sha}
    _write(ws.comedy / "corpus" / "sets" / set_dir / "meta.json",
           json.dumps({k: v for k, v in m.items() if v is not None}, ensure_ascii=False, indent=1))
    (runner or _run_all)(ws, set_dir)
    return {"status": "processed", "kind": "set", "set_dir": set_dir}


def _run_all(ws: Workspace, set_dir: str):
    import subprocess
    py = os.environ.get("PK_LAUGHS_PY", str(Path.home() / ".venvs" / "laughs" / "Scripts" / "python.exe"))
    r = subprocess.run([py, "-m", "tools.audio.run_all", "--sets", set_dir], cwd=str(DESK), capture_output=True, text=True,
                       encoding="utf-8", errors="replace", timeout=4 * 3600, env={**os.environ, **ws.env(), "PYTHONUTF8": "1"})
    if r.returncode != 0:
        raise RuntimeError(f"set pipeline failed (rc {r.returncode}): {(r.stderr or r.stdout)[-400:]}")
