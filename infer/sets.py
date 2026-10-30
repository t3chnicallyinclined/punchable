"""CLI: python -m infer.sets --all | <set_dir> ... [--no-rollup] [--dry-run]

Writes <set>/analysis.json for each set, then the cross-set rollup
kb/v0/inferred.json. Idempotent: same inputs give the same files (apart from
generated_at). Never touches graph.json or decisions.jsonl."""
import argparse
import datetime as dt
import json
import os
import sys

from .analyze import analyze_set
from .catalog import Catalog
from .embed import Embedder
from .metrics import median
from .paths import SETS, set_dirs
from .validate import validate

MODEL = "claude-opus-5-5 (judge files) + rules; bge-small-en-v1.5 candidates"


def write_json(path, obj):
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8", newline="\n") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.write("\n")
    os.replace(tmp, path)


def room_heat(fresh):
    """room_heat = a set's laugh_s_per_min / the median laugh_s_per_min over all of
    the comic's analysed sets. Sets not re-run this time are read from their
    analysis.json and returned only if their room_heat changed."""
    allan = {}
    for d in set_dirs():
        if d in fresh:
            allan[d] = fresh[d]
        else:
            p = os.path.join(SETS, d, "analysis.json")
            if os.path.exists(p):
                allan[d] = json.load(open(p, encoding="utf-8"))
    vals = [a["summary"]["laugh_s_per_min"] for a in allan.values() if "laugh_s_per_min" in a["summary"]]
    med = median(vals)
    out = dict(fresh)
    for d, an in allan.items():
        s = an["summary"]
        if "laugh_s_per_min" not in s:
            continue
        h = round(s["laugh_s_per_min"] / med, 2) if med else None
        basis = {"median_laugh_s_per_min": round(med, 2) if med is not None else None, "sets": len(vals)}
        if d in fresh or s.get("room_heat") != h or s.get("room_heat_basis") != basis:
            s["room_heat"], s["room_heat_basis"] = h, basis
            out[d] = an
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(prog="python -m infer.sets")
    ap.add_argument("sets", nargs="*")
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--no-rollup", action="store_true")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args(argv)
    todo = set_dirs() if args.all else args.sets
    if not todo:
        ap.error("give set dirs or --all")
    run_id = "auto-" + dt.datetime.now().strftime("%Y%m%d-%H%M") + "-infer"
    cat, emb = Catalog(), Embedder()
    fresh = {}
    for d in todo:
        an, tr = analyze_set(d, cat, emb, run_id)
        v = validate(an, tr, cat)
        fresh[d] = an
        s = an["summary"]
        print(f"{d}: {s['jokes_counted']} jokes ({s['known_jokes']} known, {s['new_material']} new), "
              f"{s['crowd_work']} crowd work, {s['laughs']} laughs, {s['laughs_per_min']}/min raw, "
              f"{s['laughs_per_min_merged']}/min merged, {s['laugh_s_per_min']} laugh-s/min, {s['pct_laughing']}% laughing, "
              f"best {s['best']}, weakest {s['weakest']} | verbatim {v['checked'] - v['failed']}/{v['checked']} ok")
        for x in v["dropped"]:
            print("   dropped:", x)
    for d, an in room_heat(fresh).items():
        if not args.dry_run:
            write_json(os.path.join(SETS, d, "analysis.json"), an)
    if not args.no_rollup and not args.dry_run:
        from .rollup import rollup
        inf = rollup(cat, emb, run_id, MODEL)
        print(f"inferred.json: {len(inf['jokes'])} performed jokes, "
              f"{sum(1 for j in inf['jokes'] if j['stage'] == 'working')} working, {len(inf['edges'])} edges "
              f"({sum(1 for e in inf['edges'] if e['status'] == 'inferred')} inferred); evidence "
              f"{inf['validation']['evidence_checked'] - inf['validation']['evidence_failed']}/{inf['validation']['evidence_checked']} verbatim")
    return 0


if __name__ == "__main__":
    sys.exit(main())
