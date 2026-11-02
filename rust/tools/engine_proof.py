"""The engine fork's offline proof, Python half (docs/adr/0013-fork-the-upstream-engine.md; the Rust half is
rust/crates/pk-store/examples/engine_proof.rs). Code only: every file holding his material (questions, gold, the
sample, the results) lives OUTSIDE this repository and is passed by path.

Runs against a SCRATCH SurrealDB on 127.0.0.1 that db/load.py filled (`--ns proof --no-vectors`), with PUNCH_SECRETS
pointing at a scratch-only secrets file, so nothing here can reach the hosted database.

    python rust/tools/engine_proof.py embed    --port P --ns proof --db engine
    python rust/tools/engine_proof.py sample   --out <private>/sample.md [--n 80]
    python rust/tools/engine_proof.py baseline --port P --ns proof --db engine --questions <q.json> --out <b.json>
    python rust/tools/engine_proof.py score    --questions <q.json> --fork <f.json> --baseline <b.json> --out <s.json>

`embed` runs the hosted API's own sweep (api-dev/search_api.py) with the bge model (PK_BGE_DIR) over the scratch
database: units, typed ideas and set moments get the vectors the hosted ones have. `baseline` runs Punchable's two
current retrievals on the same questions: Ask (creative/askchat.py `retrieve`, what answers a question today) over the
same material from the data folder, and Search (api-dev/search_api.py `search_all`, full-text + vectors fused in
SurrealDB) over the scratch database. `score` folds every arm's ordered records to bits (a unit to its bit, a taped
moment to the bit it performs, a bit to itself) and reports, per arm, the share of questions with a right bit in the
first 5 / 10 / 24 places, the mean share of right bits found in 24, and the latency.

questions.json: [{"id": "q01", "q": "...", "gold": ["joke:..."], "type": "words|paraphrase|stage"}]
"""
from __future__ import annotations

import argparse
import json
import random
import statistics
import sys
import time
from pathlib import Path
from types import SimpleNamespace

DESK = Path(__file__).resolve().parents[2]
for p in (DESK / "db", DESK / "api-dev", DESK / "tools" / "explorer", DESK):
    if str(p) not in sys.path:
        sys.path.insert(0, str(p))

DEAD = {"rejected", "ruled_out", "superseded"}
CUTS = (5, 10, 24)


def corpus():
    from creative.data import Corpus, comedy_dir
    return Corpus.load(comedy_dir())


def rpc(a):
    """A connection to the scratch server only: an explicit port (loopback, never SURREAL_URL), a scratch secrets file
    (never the real one, whose password must not go to whatever listens on a port), a `proof*` namespace."""
    import os
    import sdb
    if a.port is None:
        raise SystemExit("--port is required: the scratch server only")
    secrets = os.environ.get("PUNCH_SECRETS")
    if not secrets or Path(secrets).resolve() == (Path.home() / ".punchable" / "secrets.env").resolve():
        raise SystemExit("set PUNCH_SECRETS to a scratch-only secrets file (never ~/.punchable/secrets.env)")
    if not a.ns.startswith("proof"):
        raise SystemExit(f"the namespace must start with 'proof' (scratch only), not {a.ns!r}")
    return sdb.Rpc(a.port, a.ns, a.db)


def cmd_embed(a):
    import search_api
    c = corpus()
    emb = search_api.make_embedder(log=print)
    if emb is None or emb.name != "bge-small-en-v1.5":
        raise SystemExit("no bge model (PK_BGE_DIR): the proof needs the hosted vectors")
    v = SimpleNamespace(jokes=[{**n, "id": k} for k, n in c.jokes.items()], joke_units=c.joke_units)
    r = rpc(a)
    try:
        t = time.perf_counter()
        done = search_api.sweep(r, emb, v)
        print(json.dumps({"embedded": done, "s": round(time.perf_counter() - t, 1)}))
    finally:
        r.close()


def cmd_sample(a):
    """A private sample of his bits to write questions from (title, typed idea, a version's opening, nights)."""
    c = corpus()
    rng = random.Random(a.seed)
    jids = sorted(j for j in c.jokes if c.joke_units.get(j))
    pick = rng.sample(jids, min(a.n, len(jids)))
    out = []
    for jid in pick:
        n = c.jokes[jid]
        units = c.joke_units.get(jid, [])
        text = (c.nodes.get(units[0]) or {}).get("text", "") if units else ""
        perfs = sum(1 for e in c.edges_by_node.get(jid, []) if e.get("kind") == "performed_as")
        out.append(f"## {jid}\ntitle: {n.get('title')}\nidea: {n.get('summary') or ''}\nversions: {len(units)} "
                   f"nights: {perfs}\n{text[:500]}\n")
    Path(a.out).write_text("\n".join(out), encoding="utf-8")
    print(f"wrote {len(out)} bits to {a.out}")


def cmd_baseline(a):
    import search_api
    from creative import askchat
    qs = json.loads(Path(a.questions).read_text(encoding="utf-8"))
    c = corpus()
    askchat.sources(c)  # built once, as the API caches it
    emb = search_api.make_embedder(log=lambda *_: None)
    r = rpc(a)
    rows = []
    try:
        for q in qs:
            ask_us, search_us = [], []
            for _ in range(a.repeats):
                t = time.perf_counter()
                ask_ids = askchat.retrieve(c, q["q"])
                ask_us.append(int((time.perf_counter() - t) * 1e6))
                t = time.perf_counter()
                res = search_api.search_all(r, emb, q["q"], limit=20)
                search_us.append(int((time.perf_counter() - t) * 1e6))
            merged = sorted([(s or 0.0, i, k) for i, (k, s) in enumerate(res["hits"])]
                            + [(s or 0.0, i, k) for i, (k, s) in enumerate(res["moments"])],
                            key=lambda x: (-x[0], x[1]))
            rows.append({"id": q["id"], "ask": ask_ids, "search": [k for _, _, k in merged],
                         "vectors": res["vectors"],
                         "us": {"ask_cold": ask_us[0], "ask_warm_median": int(statistics.median(ask_us[1:] or ask_us)),
                                "search_cold": search_us[0],
                                "search_warm_median": int(statistics.median(search_us[1:] or search_us))}})
    finally:
        r.close()
    Path(a.out).write_text(json.dumps({"questions": rows}, indent=1), encoding="utf-8")
    print(f"wrote {len(rows)} baselines to {a.out}")


def folder(c):
    """record id -> bit id (or None), the same for every arm."""
    from creative import askchat
    src = askchat.sources(c)

    def fold(key: str) -> str | None:
        if key.startswith("joke:"):
            return key if key in c.jokes else None
        if key.startswith("unit:"):
            return c.unit_joke.get(key)
        if key.startswith("seg:"):
            return (src.get(key) or {}).get("joke_id")
        if key.startswith("perf:"):
            for e in c.edges_by_node.get(key, []):
                if e.get("kind") == "performed_as" and (e.get("status") or "") not in DEAD:
                    other = e["dst"] if e["src"] == key else e["src"]
                    if other in c.jokes:
                        return other
        return None
    return fold


def score_arm(ordered: list[str], gold: set[str], fold) -> dict:
    bits = [fold(k) for k in ordered[:max(CUTS)]]
    out = {f"hit@{k}": any(b in gold for b in bits[:k] if b) for k in CUTS}
    out["found@24"] = len({b for b in bits if b in gold}) / max(1, len(gold))
    first = next((i + 1 for i, b in enumerate(bits) if b in gold), None)
    out["first"] = first
    return out


def pct(v, p):
    v = sorted(v)
    return v[min(len(v) - 1, round((len(v) - 1) * p))] if v else 0


def cmd_score(a):
    qs = {q["id"]: q for q in json.loads(Path(a.questions).read_text(encoding="utf-8"))}
    fork = {r["id"]: r for r in json.loads(Path(a.fork).read_text(encoding="utf-8"))["questions"]}
    base = {r["id"]: r for r in json.loads(Path(a.baseline).read_text(encoding="utf-8"))["questions"]}
    fold = folder(corpus())
    arms: dict[str, dict[str, dict]] = {}
    for qid, q in qs.items():
        gold = set(q["gold"])
        arms.setdefault("ask (current)", {})[qid] = score_arm(base[qid]["ask"], gold, fold)
        arms.setdefault("search (current)", {})[qid] = score_arm(base[qid]["search"], gold, fold)
        arms.setdefault("fork default", {})[qid] = score_arm(fork[qid]["packet"], gold, fold)
        for name, keys in fork[qid]["arms"].items():
            arms.setdefault(f"fork:{name}", {})[qid] = score_arm(keys, gold, fold)
    table = []
    for name, per in arms.items():
        n = len(per)
        row = {"arm": name, "n": n}
        for k in CUTS:
            row[f"hit@{k}"] = sum(r[f"hit@{k}"] for r in per.values())
        row["found@24"] = round(sum(r["found@24"] for r in per.values()) / max(1, n), 3)
        row["by_type"] = {}
        for t in sorted({q.get("type", "?") for q in qs.values()}):
            ids = [i for i, q in qs.items() if q.get("type", "?") == t]
            row["by_type"][t] = f"{sum(per[i]['hit@24'] for i in ids)}/{len(ids)}"
        table.append(row)
    lat = {
        "ask warm (in memory, Python)": [b["us"]["ask_warm_median"] for b in base.values()],
        "search warm (SurrealDB + embed, Python)": [b["us"]["search_warm_median"] for b in base.values()],
        "fork embed (bge, Rust)": [f["us"]["embed"] for f in fork.values()],
        "fork open warm (SurrealDB legs + pack, Rust)": [f["us"]["open_warm_median"] for f in fork.values()],
        "fork open cold": [f["us"]["open_cold"] for f in fork.values()],
        "fork words-only open": [f["us"]["words_only_open"] for f in fork.values()],
    }
    lat["fork end to end (embed + open warm)"] = [f["us"]["embed"] + f["us"]["open_warm_median"]
                                                  for f in fork.values()]
    latency = {k: {"p50_ms": round(pct(v, 0.5) / 1000, 1), "p95_ms": round(pct(v, 0.95) / 1000, 1),
                   "max_ms": round(max(v) / 1000, 1) if v else 0} for k, v in lat.items()}
    per_q = {qid: {name: arms[name][qid]["first"] for name in ("ask (current)", "search (current)", "fork default")}
             for qid in qs}
    out = {"table": table, "latency": latency, "first_right_place": per_q}
    Path(a.out).write_text(json.dumps(out, indent=1), encoding="utf-8")
    w = max(len(r["arm"]) for r in table)
    print(f"{'arm':<{w}}  hit@5 hit@10 hit@24 found@24  by type (hit@24)")
    for r in table:
        print(f"{r['arm']:<{w}}  {r['hit@5']:>5} {r['hit@10']:>6} {r['hit@24']:>6} {r['found@24']:>8}  {r['by_type']}")
    for k, v in latency.items():
        print(f"{k}: p50 {v['p50_ms']} ms, p95 {v['p95_ms']} ms, max {v['max_ms']} ms")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("embed", "baseline"):
        s = sub.add_parser(name)
        s.add_argument("--port", type=int, required=True)
        s.add_argument("--ns", required=True)
        s.add_argument("--db", required=True)
        if name == "baseline":
            s.add_argument("--questions", required=True)
            s.add_argument("--out", required=True)
            s.add_argument("--repeats", type=int, default=5)
    s = sub.add_parser("sample")
    s.add_argument("--out", required=True)
    s.add_argument("--n", type=int, default=80)
    s.add_argument("--seed", type=int, default=20261102)
    s = sub.add_parser("score")
    for k in ("questions", "fork", "baseline", "out"):
        s.add_argument(f"--{k}", required=True)
    a = ap.parse_args()
    {"embed": cmd_embed, "sample": cmd_sample, "baseline": cmd_baseline, "score": cmd_score}[a.cmd](a)


if __name__ == "__main__":
    main()
