"""push_remote.py -- Punchable's data to the hosted SurrealDB on the server. Text/JSON only, never audio. Idempotent.

    python db/push_remote.py pack --tar bundle.tgz     # on the comic's PC: build the bundle (infra/ship.sh dbload does it)
    python db/push_remote.py apply --tar -             # in the cluster (the dbload pod): load it, read from stdin
    python db/push_remote.py apply --tar - --check     # what would change, and whether it is safe; writes nothing
    python db/push_remote.py cutover [--undo]          # in the cluster: the database becomes the source of truth
                                                       # (meta:state.online_first); every later `apply` is refused

The bundle (bundle/manifest.json + bundle/comedy/...):
  comedy/kb/v1/graph.json, id_map.json        the graph db/load.py chose (graph_choice.json), with its id map
  comedy/kb/v0/decisions.jsonl                the comic's history (canonical)
  comedy/kb/v0/inferred.json, insights.json, learning.json, KB_REPORT.md
  comedy/corpus/sets/<set>/{analysis,meta,laughs_auto,transcript,clips}.json
  comedy/kb/creative/{storyboards,workingsets,venues}.jsonl, geocode_cache.json, venue_addresses.json
  vectors/<cache>.partNNN.json                bge-small vectors as JSON floats (exact float32 round trip, %.9g)
  vectors/unit_vectors_extra.json             unit embeddings db/load.py made locally for units the cache lacks

apply, in order (each step is a no-op when nothing changed; the same bundle twice changes nothing):
  1. Safety: every decision already in the database must be in the bundle's decisions.jsonl (by content), and every
     creative_event line in the bundle's jsonl. If the phone added lines the PC has not pulled yet, it REFUSES
     (exit 3): run the home worker once (python -m worker --once) so they reach the PC, then pack again. When the
     database is a prefix of the bundle the load is incremental; otherwise (both sides added lines and the worker
     merged them) the database is rebuilt from the bundle, which holds every line.
  2. db/load.py's incremental sync (a full load the first time) of graph -> inferred -> analysis -> laughs ->
     decisions through the apply_decision event, as a DATABASE-level EDITOR (never root).
  3. srcfile: the text files the hosted API serves from (transcripts, analyses, clips, ...), upserted by content hash;
     files no longer in the bundle are removed.
  4. creative_event: the bundle's creative lines past what the database has.
  5. meta:state.load_info.bundle = {sha, files, at}; the API notices and refreshes its /tmp copies.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import shutil
import sys
import tarfile
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
DESK = HERE.parent
sys.path.insert(0, str(HERE))
import sdb  # noqa: E402

import srcpaths as SP  # noqa: E402  (one list of what is data, shared with the API's worker route)

SET_FILES = SP.SET_FILES
KB0_FILES = SP.KB0_FILES
CREATIVE_LOGS = SP.CREATIVE_LOGS
CREATIVE_FILES = SP.CREATIVE_FILES + ("suggest_judge.json",)
GSYNC_FILES = SP.GSYNC_FILES  # gsync's state: the Notes list, Google Docs status, and what an online-first gsync resumes from
VEC_CACHES = (("vectors_ids.json", "vectors.f32"), ("vectors_v1_ids.json", "vectors_v1.f32"))
VEC_PART = 400
TEXT_SUFFIXES = SP.TEXT_SUFFIXES
NOT_MIRRORED = SP.NOT_MIRRORED
WORK = Path(os.environ.get("PK_DBLOAD_DIR", Path(tempfile.gettempdir()) / "pk-dbload"))


def log(*a):
    print(*a, file=sys.stderr, flush=True)


def sha256b(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


# ====================================================================== pack (the PC)
def pack(out_tar: Path, comedy: Path | None = None) -> dict:
    import numpy as np
    import load as L
    comedy = Path(comedy or sdb.COMEDY)
    sel = L.resolve_graph("auto", comedy / "kb" / "v0")
    graph, id_map = Path(sel["graph"]), sel.get("id_map")
    files: dict[str, bytes] = {}

    def add(rel: str, p: Path):
        if p.is_file():
            if p.suffix.lower() not in TEXT_SUFFIXES:
                raise SystemExit(f"refusing to pack a non-text file: {p}")
            b = p.read_bytes()
            b.decode("utf-8")  # text only
            files["comedy/" + rel] = b

    add(graph.relative_to(comedy).as_posix(), graph)
    if id_map:
        add(Path(id_map).relative_to(comedy).as_posix(), Path(id_map))
    for f in KB0_FILES:
        add(f"kb/v0/{f}", comedy / "kb" / "v0" / f)
    sets = comedy / "corpus" / "sets"
    for d in sorted(sets.iterdir()) if sets.is_dir() else []:
        if d.is_dir():
            for f in SET_FILES:
                add(f"corpus/sets/{d.name}/{f}", d / f)
    cdir = comedy / "kb" / "creative"
    for n in CREATIVE_LOGS:
        add(f"kb/creative/{n}.jsonl", cdir / f"{n}.jsonl")
    for f in CREATIVE_FILES:
        add(f"kb/creative/{f}", cdir / f)
    # voice memos (the final load before the online-first cutover): their transcripts and gsync's memo state, so the
    # Notes list works from the database; never their audio
    for f in GSYNC_FILES:
        add(f"kb/gsync/{f}", comedy / "kb" / "gsync" / f)
    memos = comedy / "corpus" / "memos"
    for d in sorted(memos.iterdir()) if memos.is_dir() else []:
        if d.is_dir():
            for f in SP.MEMO_FILES:
                add(f"corpus/memos/{d.name}/{f}", d / f)
    for xp in sorted(L.VECTORS.glob("extra_*.json")) if L.VECTORS.is_dir() else []:
        files["vectors/" + xp.name] = xp.read_bytes()  # vectors for summary-only ideas (creative/memolink.py)
    # vectors: float32 -> JSON text, exact (%.9g round-trips a float32)
    for ids_name, vec_name in VEC_CACHES:
        ids_p, vec_p = L.VECTORS / ids_name, L.VECTORS / vec_name
        if not (ids_p.is_file() and vec_p.is_file()):
            continue
        ids = json.loads(ids_p.read_text(encoding="utf-8"))
        m = np.fromfile(vec_p, dtype=np.float32).reshape(len(ids), -1)
        for k, i in enumerate(range(0, len(ids), VEC_PART)):
            rows = [[float(f"{x:.9g}") for x in r.tolist()] for r in m[i:i + VEC_PART]]
            doc = {"cache": vec_name, "ids_file": ids_name, "dim": int(m.shape[1]), "ids": ids[i:i + VEC_PART], "rows": rows}
            files[f"vectors/{Path(vec_name).stem}.part{k:03d}.json"] = json.dumps(doc, separators=(",", ":")).encode()
    if L.EXTRA_VEC.is_file():
        files["vectors/unit_vectors_extra.json"] = L.EXTRA_VEC.read_bytes()
    manifest = {"version": 1, "packed_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
                "graph": "comedy/" + graph.relative_to(comedy).as_posix(), "graph_version": sel["version"],
                "graph_reason": sel["reason"], "id_map": ("comedy/" + Path(id_map).relative_to(comedy).as_posix()) if id_map else None,
                "files": {k: {"sha256": sha256b(v), "bytes": len(v)} for k, v in sorted(files.items())}}
    manifest["sha"] = sha256b(json.dumps(manifest["files"], sort_keys=True).encode())
    out_tar.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(out_tar, "w:gz") as t:
        for name, b in [("manifest.json", json.dumps(manifest, indent=1).encode())] + sorted(files.items()):
            ti = tarfile.TarInfo("bundle/" + name)
            ti.size, ti.mtime, ti.mode = len(b), 0, 0o644
            t.addfile(ti, io.BytesIO(b))
    total = sum(len(v) for v in files.values())
    log(f"packed {len(files)} files, {total / 1e6:.1f} MB of text -> {out_tar} ({out_tar.stat().st_size / 1e6:.1f} MB gz), sha {manifest['sha'][:12]}")
    return manifest


# ====================================================================== apply (the cluster)
def unpack(src, dest: Path) -> dict:
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    fobj = sys.stdin.buffer if src == "-" else open(src, "rb")
    with tarfile.open(fileobj=fobj, mode="r|gz") as t:
        for m in t:
            if not m.isfile() or not m.name.startswith("bundle/") or ".." in Path(m.name).parts or m.name.startswith("/"):
                raise SystemExit(f"bad bundle member {m.name!r}")
            out = dest / m.name[len("bundle/"):]
            out.parent.mkdir(parents=True, exist_ok=True)
            with t.extractfile(m) as r, open(out, "wb") as w:
                shutil.copyfileobj(r, w)
            os.utime(out, (0, 0))  # stable mtimes: load.sources_signature compares them
    man = json.loads((dest / "manifest.json").read_text(encoding="utf-8"))
    for rel, meta in man["files"].items():
        b = (dest / rel).read_bytes()
        if sha256b(b) != meta["sha256"]:
            raise SystemExit(f"bundle file {rel} does not match its manifest hash")
    return man


def vectors_from_json(bundle: Path, out: Path):
    import numpy as np
    out.mkdir(parents=True, exist_ok=True)
    ids: dict[str, tuple[str, list]] = {}
    for p in sorted((bundle / "vectors").glob("*.part*.json")):  # one part at a time (memory)
        d = json.loads(p.read_text(encoding="utf-8"))
        ids_file, acc = ids.setdefault(d["cache"], (d["ids_file"], []))
        if not acc:
            (out / d["cache"]).unlink(missing_ok=True)
        acc += d["ids"]
        with open(out / d["cache"], "ab") as f:
            np.asarray(d["rows"], dtype=np.float32).tofile(f)
    for cache, (ids_file, acc) in ids.items():
        (out / ids_file).write_text(json.dumps(acc), encoding="utf-8")


def read_lines(p: Path) -> list[str]:
    if not p.is_file():
        return []
    out = []
    for line in p.read_text(encoding="utf-8").splitlines():
        if line.strip():
            try:
                json.loads(line)
            except ValueError:
                continue
            out.append(line.rstrip("\r"))
    return out


def _dkey(d: dict) -> str:
    return json.dumps(d, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def safety(r, bundle: Path) -> list[str]:
    """Reasons to refuse: database rows (decisions, creative events) that the bundle does not contain. Nothing that
    exists only in the database (made on the phone) may be lost by a load."""
    from collections import Counter
    from kbmap import row_to_decision
    import load as L
    problems = []
    decs, _ = L.read_decisions(bundle / "comedy" / "kb" / "v0" / "decisions.jsonl")
    dbd = [row_to_decision(x) for x in r.last("SELECT * FROM decision ORDER BY seq;")]
    missing = Counter(_dkey(d) for d in dbd) - Counter(_dkey(d) for d in decs)
    if missing:
        problems.append(f"decisions: {sum(missing.values())} of the database's {len(dbd)} lines are not in the bundle "
                        f"({len(decs)} lines). Decisions made on the phone have not reached the PC: run "
                        "`python -m worker --once`, then pack again.")
    for n in CREATIVE_LOGS:
        fl = read_lines(bundle / "comedy" / "kb" / "creative" / f"{n}.jsonl")
        dl = [x["line"] for x in r.last("SELECT seq, line FROM creative_event WHERE store = $s ORDER BY seq;", {"s": n})]
        extra = Counter(dl) - Counter(fl)
        if extra:
            problems.append(f"creative {n}: {sum(extra.values())} database events are not in the bundle "
                            f"(database {len(dl)}, bundle {len(fl)}). Run `python -m worker --once`, then pack again.")
    return problems


def mirror_files(r, bundle: Path, man: dict, check: bool) -> dict:
    want = {rel[len("comedy/"):]: meta for rel, meta in man["files"].items()
            if rel.startswith("comedy/") and rel[len("comedy/"):] not in NOT_MIRRORED}
    want.update({rel: meta for rel, meta in man["files"].items() if rel.startswith("vectors/")})
    have = {x["path"]: x["sha256"] for x in r.last("SELECT path, sha256 FROM srcfile;")}
    up = [p for p, m in want.items() if have.get(p) != m["sha256"]]
    gone = [p for p in have if p not in want]
    if not check:
        for p in up:
            src = bundle / ("comedy/" + p if not p.startswith("vectors/") else p)
            text = src.read_text(encoding="utf-8")
            key = hashlib.sha1(p.encode()).hexdigest()
            r.write("UPSERT type::record('srcfile', $k) CONTENT $c RETURN id;",
                    {"k": key, "c": {"path": p, "sha256": want[p]["sha256"], "bytes": want[p]["bytes"], "text": text}}, expect=1)
        for p in gone:
            r.write("DELETE srcfile WHERE path = $p RETURN BEFORE;", {"p": p})
    return {"files": len(want), "upserted": len(up), "removed": len(gone)}


def creative_events(r, bundle: Path, check: bool) -> dict:
    """Append the bundle's lines past the database's when the database is a prefix; otherwise (the worker merged
    phone and PC lines in a new order) replace that store's rows with the bundle's, which contain them all."""
    out = {}
    for n in CREATIVE_LOGS:
        fl = read_lines(bundle / "comedy" / "kb" / "creative" / f"{n}.jsonl")
        dl = [x["line"] for x in r.last("SELECT seq, line FROM creative_event WHERE store = $s ORDER BY seq;", {"s": n})]
        have = len(dl)
        if dl != fl[:len(dl)]:
            out[n] = f"rebuilt ({len(fl)} events)"
            if check:
                continue
            r.query("DELETE creative_event WHERE store = $s RETURN NONE;", {"s": n})
            have = 0
        else:
            out[n] = len(fl) - have
        if not check:
            for i in range(have, len(fl)):
                ev = json.loads(fl[i])
                r.write("CREATE creative_event CONTENT $row RETURN id;",
                        {"row": {"store": n, "seq": i, "line": fl[i], "h": sha256b(fl[i].encode()), "op": ev.get("op"),
                                 "item": ev.get("id"), "at": ev.get("ts")}}, expect=1)
    return out


def apply(src, check: bool = False, force: bool = False) -> int:
    import functools
    import load as L
    t0 = time.perf_counter()
    bdir = WORK / "bundle"
    man = unpack(src, bdir)
    sdb.check_version(None)
    r = sdb.Rpc(None, ns=os.environ.get("SURREAL_NS", sdb.NS), db=os.environ.get("SURREAL_DB", sdb.DB))
    try:
        ns, db = r.ns, r.db
        if check and "srcfile" not in (r.last("INFO FOR DB;") or {}).get("tables", {}):
            print(json.dumps({"status": "check", "sha": man["sha"], "files": len(man["files"]), "database": "never loaded"}))
            return 0
        if not check:
            L.apply_schema(r, ns, db)
        st = sdb.strip_none(r.last("SELECT * FROM ONLY meta:state;") or {})
        if st.get("online_first"):
            log("REFUSED: this database is the source of truth since the online-first cutover "
                f"({st['online_first'].get('at')}); a bundle from the PC would overwrite what was written online. "
                "Roll the cutover back first (push_remote.py cutover --undo) only if that is really meant.")
            return 3
        problems = safety(r, bdir)
        if problems:
            for p in problems:
                log("REFUSED: " + p)
            return 3
        prev = ((st.get("load_info") or {}).get("bundle") or {}).get("sha")
        if prev == man["sha"] and not force:
            log(f"bundle {man['sha'][:12]} is already loaded: nothing to do")
            print(json.dumps({"status": "unchanged", "sha": man["sha"]}))
            return 0
        if check:
            print(json.dumps({"status": "check", "sha": man["sha"], "files": len(man["files"]),
                              "srcfile": mirror_files(r, bdir, man, True), "creative": creative_events(r, bdir, True)}, indent=1))
            return 0
        vec = WORK / "vectors"
        vectors_from_json(bdir, vec)
        L.VECTORS = vec
        L.EXTRA_VEC = bdir / "vectors" / "unit_vectors_extra.json"
        L.load = functools.partial(L.load, embed_missing=False)  # no model in the cluster; sync() falls back to load()
        comedy = bdir / "comedy"
        info = L.sync(r, ns=ns, db=db, graph=bdir / man["graph"], kbdir=comedy / "kb" / "v0", corpus=comedy / "corpus",
                      fixtures=None, id_map=(bdir / man["id_map"]) if man.get("id_map") else None,
                      version=man.get("graph_version", "?"), reason=man.get("graph_reason", ""))
        files = mirror_files(r, bdir, man, False)
        cre = creative_events(r, bdir, False)
        bundle = {"sha": man["sha"], "packed_at": man.get("packed_at"), "files": len(man["files"]),
                  "graph": man["graph"][len("comedy/"):], "id_map": (man.get("id_map") or "")[len("comedy/"):] or None,
                  "loaded_at": datetime.now(timezone.utc).isoformat(timespec="seconds")}
        r.write("UPDATE meta:state SET load_info.bundle = $b, gen += 1 RETURN id;", {"b": bundle})
        missing = sdb.check_changefeeds(r)
        if missing:
            raise SystemExit(f"change feed missing after load: {missing}")
        out = {"status": "loaded", "sha": man["sha"], "mode": info.get("mode"), "sync_changed": info.get("sync_changed"),
               "counts": info.get("counts"), "srcfile": files, "creative": cre, "s": round(time.perf_counter() - t0, 1)}
        print(json.dumps(out, indent=1, default=str))
        return 0
    finally:
        r.close()
        shutil.rmtree(WORK, ignore_errors=True)


def cutover(undo: bool = False, by: str | None = None) -> int:
    """Mark the database as the source of truth (docs: comedy/docs/punchable/ONLINE_FIRST.md). After this, `apply`
    refuses. --undo clears it (the rollback path); both are recorded in meta:state.cutover_log."""
    sdb.check_version(None)
    r = sdb.Rpc(None, ns=os.environ.get("SURREAL_NS", sdb.NS), db=os.environ.get("SURREAL_DB", sdb.DB))
    try:
        L = __import__("load")
        L.apply_extra_schema(r)
        st = sdb.strip_none(r.last("SELECT * FROM ONLY meta:state;") or {})
        now = datetime.now(timezone.utc).isoformat(timespec="seconds")
        bundle = ((st.get("load_info") or {}).get("bundle") or {}).get("sha")
        if undo:
            if not st.get("online_first"):
                print(json.dumps({"status": "not_cut_over"}))
                return 0
            r.write("UPDATE meta:state SET online_first = NONE, cutover_log = array::append(cutover_log ?? [], $e) RETURN id;",
                    {"e": {"undo": True, "at": now, "by": by or "owner", "was": st["online_first"]}})
            print(json.dumps({"status": "rolled_back", "at": now}))
            return 0
        if not bundle:
            log("REFUSED: no bundle was ever loaded: run the final infra/ship.sh dbload first")
            return 3
        mark = {"at": now, "by": by or "owner", "last_bundle": bundle,
                "decisions": r.last("RETURN count(SELECT id FROM decision);")}
        r.write("UPDATE meta:state SET online_first = $m, cutover_log = array::append(cutover_log ?? [], $e) RETURN id;",
                {"m": mark, "e": {"undo": False, **mark}})
        print(json.dumps({"status": "cut_over", **mark}))
        return 0
    finally:
        r.close()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=["pack", "apply", "cutover"])
    ap.add_argument("--tar", help="pack: output .tgz; apply: input .tgz or - for stdin")
    ap.add_argument("--undo", action="store_true", help="cutover: roll it back (apply works again)")
    ap.add_argument("--comedy", type=Path, default=None, help="pack: the comedy folder (default the real one)")
    ap.add_argument("--check", action="store_true", help="apply: report only")
    ap.add_argument("--force", action="store_true", help="apply: re-sync even if this bundle is already loaded")
    a = ap.parse_args()
    if a.cmd == "cutover":
        return cutover(undo=a.undo)
    if not a.tar:
        ap.error("--tar is required for pack and apply")
    if a.cmd == "pack":
        pack(Path(a.tar), a.comedy)
        return 0
    return apply(a.tar, check=a.check, force=a.force)


if __name__ == "__main__":
    sys.exit(main())
