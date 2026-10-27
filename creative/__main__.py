"""CLI: python -m creative <command> ...   (prints the JSON the API would serve)

  connections <joke_id> [--cap 5]
  story list | show ID | create --title T [--jokes a,b] [--target S] [--notes N] | add ID JOKE [--at I]
        | remove ID JOKE | move ID FROM TO | update ID [--title] [--target] [--notes]
        | health ID | next ID | transition A B | bridge A B | seed | export ID
  ws    list | promote BOARD_ID [--target S] | show ID | version ID --note N | lock ID JOKE [--line TEXT --node NODE]
        | unlock ID JOKE [--line TEXT] | align ID [--set KEY] | history ID | try ID | punchup ID JOKE
        | perfected ID | demo [--no-drafts]
  venues [--offline]
  today [--ws ID] [--date YYYY-MM-DD]
  merge groups [--cap N] | for JOKE | match TEXT | judge     same-idea groups (judge = offline, Claude via claude_code)
  memolink report | embed | judge | triage | attach [--apply] | all [--apply]    voice memos <-> bits (creative/memolink.py)
  demo-all [--no-drafts] [--offline]      write every demo output under the creative dir

--dir overrides the creative dir (default comedy/kb/creative, or PUNCHABLE_CREATIVE_DIR).
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from . import connections as C
from . import story as S
from . import today as T
from . import venues as V
from . import workingset as W
from .data import creative_dir, load_corpus
from .util import dumps, write_json


def _out(obj):
    sys.stdout.write(dumps(obj) + "\n")


def _board(bs, i):
    b = bs.get(i)
    if not b or b.get("deleted"):
        sys.exit(f"no storyboard {i}")
    return b


def _ws(wss, i):
    w = wss.get(i)
    if not w or w.get("deleted"):
        sys.exit(f"no working set {i}")
    return w


def main(argv=None):
    ap = argparse.ArgumentParser(prog="python -m creative", description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dir", help="creative dir (stores and outputs)")
    ap.add_argument("cmd")
    ap.add_argument("args", nargs="*")
    ap.add_argument("--cap", type=int, default=5)
    ap.add_argument("--title")
    ap.add_argument("--jokes")
    ap.add_argument("--target", type=int)
    ap.add_argument("--notes")
    ap.add_argument("--note")
    ap.add_argument("--at", type=int)
    ap.add_argument("--set")
    ap.add_argument("--line")
    ap.add_argument("--node")
    ap.add_argument("--ws")
    ap.add_argument("--date")
    ap.add_argument("--offline", action="store_true")
    ap.add_argument("--no-drafts", action="store_true")
    ap.add_argument("--apply", action="store_true", help="memolink attach: write the clear attaches (ai:attach)")
    a = ap.parse_args(argv)
    if a.cmd == "memolink":  # before load_corpus: it loads (and reloads) the corpus itself
        from . import memolink as ML
        from .data import VECTORS_DIR, comedy_dir
        return _out(ML.cli(comedy_dir(), Path(a.dir) if a.dir else creative_dir(), VECTORS_DIR,
                           list(a.args) + (["--apply"] if a.apply else [])))
    d = Path(a.dir) if a.dir else creative_dir()
    c = load_corpus()
    x = a.args

    if a.cmd == "connections":
        return _out(C.joke_connections(c, x[0], cap=a.cap))

    if a.cmd == "story":
        bs = S.board_store(d)
        sub = x[0] if x else "list"
        if sub == "list":
            return _out([{"id": b["id"], "title": b["title"], "bits": len(b["joke_ids"]), "source": b["source"]}
                         for b in bs.items.values() if not b.get("deleted")])
        if sub == "show":
            return _out(S.board_view(c, _board(bs, x[1])))
        if sub == "create":
            return _out(S.create(bs, a.title or "Untitled", (a.jokes or "").split(",") if a.jokes else [],
                                 a.target or 900, a.notes or ""))
        if sub == "add":
            if x[2] not in c.jokes:
                sys.exit(f"no such joke: {x[2]}")
            return _out(S.add(bs, x[1], x[2], a.at))
        if sub == "remove":
            return _out(S.remove(bs, x[1], x[2]))
        if sub == "move":
            return _out(S.move(bs, x[1], int(x[2]), int(x[3])))
        if sub == "update":
            return _out(S.update(bs, x[1], title=a.title, target_s=a.target, notes=a.notes))
        if sub == "health":
            return _out(S.health(c, _board(bs, x[1])))
        if sub == "next":
            return _out(S.best_next(c, _board(bs, x[1]), cap=a.cap))
        if sub == "transition":
            return _out(S.transition(c, x[1], x[2]))
        if sub == "bridge":
            return _out(S.bridge_draft(c, x[1], x[2]))
        if sub == "seed":
            return _out([{"id": b["id"], "title": b["title"], "bits": len(b["joke_ids"])} for b in S.seed_from_insights(c, bs)])
        if sub == "export":
            return _out(S.export_setlist(c, _board(bs, x[1])))

    if a.cmd == "ws":
        wss, bs = W.ws_store(d), S.board_store(d)
        sub = x[0] if x else "list"
        if sub == "list":
            return _out([{"id": w["id"], "title": w["title"], "version": w["current"]} for w in wss.items.values()
                         if not w.get("deleted")])
        if sub == "promote":
            return _out(W.promote(c, wss, _board(bs, x[1]), target_s=a.target))
        if sub == "show":
            return _out(_ws(wss, x[1]))
        if sub == "version":
            return _out(W.new_version(wss, x[1], a.note, target_s=a.target))
        if sub == "lock":
            return _out(W.lock_line(c, wss, x[1], x[2], a.line, a.node) if a.line else W.lock_bit(wss, x[1], x[2]))
        if sub == "unlock":
            return _out(W.unlock_line(wss, x[1], x[2], a.line) if a.line else W.unlock_bit(wss, x[1], x[2]))
        if sub == "align":
            w = _ws(wss, x[1])
            return _out(W.align(c, w, a.set) if a.set else W.align_all(c, w))
        if sub == "history":
            return _out(W.history(c, _ws(wss, x[1])))
        if sub == "try":
            return _out(W.try_tonight(c, _ws(wss, x[1])))
        if sub == "punchup":
            return _out(W.punch_up(c, _ws(wss, x[1]), x[2]))
        if sub == "perfected":
            return _out(W.perfected(c, _ws(wss, x[1])))
        if sub == "demo":
            return _out(W.demo(c, bs, wss, with_drafts=not a.no_drafts))

    if a.cmd == "venues":
        return _out(V.build(c, V.Geocoder(d / "geocode_cache.json"), offline=a.offline))

    if a.cmd == "today":
        ws = W.ws_store(d).get(a.ws) if a.ws else None
        return _out(T.build(c, ws, a.date))

    if a.cmd == "merge":
        from . import merge as MG
        return _out(MG.cli(c, d, x, cap=a.cap if a.cap != 5 else 10))

    if a.cmd == "demo-all":
        return _out(demo_all(c, d, drafts=not a.no_drafts, offline=a.offline))

    ap.error(f"unknown command {a.cmd} {' '.join(x)}")


def demo_all(c, d: Path, drafts: bool = True, offline: bool = False) -> dict:
    """Generate every demo output under d/demo (never committed: it is his material)."""
    out = d / "demo"
    bs, wss = S.board_store(d), W.ws_store(d)
    written = []

    def w(name, obj):
        write_json(out / name, obj)
        written.append(str(out / name))

    for j in ("joke:airport-bit", "joke:parking-meter", "joke:gym-membership"):
        w(f"connections_{j.split(':')[1]}.json", C.joke_connections(c, j))
    seeded = S.seed_from_insights(c, bs)
    w("storyboards_seeded.json", [S.board_view(c, b) for b in seeded])
    demo = W.demo(c, bs, wss, with_drafts=drafts)
    w("workingset_demo.json", demo)
    if drafts:
        # the first transition across the seeded boards that needs a bridge and has a link to ground it
        pair = None
        for b in seeded:
            for t in S.health(c, b)["transitions"]:
                if t["needs_bridge"] and (t["via"] or t["shared_topics"]):
                    pair = (t["from"], t["to"])
                    break
            if pair:
                break
        if pair:
            w("bridge_draft.json", S.bridge_draft(c, *pair))
    v = V.build(c, V.Geocoder(d / "geocode_cache.json"), offline=offline)
    w("venues.json", {k: v[k] for k in ("generated_at", "venues", "ambiguous", "geocoder")})
    w("venues.geojson", v["geojson"])
    w("today.json", T.build(c, wss.get("ws:demo-everyday-gripes-run")))
    return {"written": written}


if __name__ == "__main__":
    main()
