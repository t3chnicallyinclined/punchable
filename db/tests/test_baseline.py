"""Baseline diff: every API endpoint, SurrealDB backend vs files backend, on the real data (temp copies).

    python db/tests/test_baseline.py

1. Reads: one API with the SurrealDB backend over a TEMP database loaded from TEMP copies of the files. Every GET
   endpoint is fetched twice from it, default (SurrealDB) and ?backend=files (the same temp files replayed in
   Python), and the two answers must be equal. Every joke page, every set page, every query variant.
2. Writes: the same sequence of POSTs (all write endpoints, every set-row op, undo, errors) goes to that API and to
   a second, files-only API on its own temp copy. Responses must be equal (timestamps and temp paths normalised).
3. Reads again after the writes, both ways (SurrealDB vs files inside one API; SurrealDB API vs files-only API).

Both API processes run with PYTHONHASHSEED=0: kb.View.joke_scope is a Python set, so the order of equal-confidence
labels (and top_labels ties) follows per-process string hashing in either backend. Pinning the seed makes two
processes comparable; it is not a SurrealDB difference (inside one process the two backends always agree).
Allowed differences (documented in db/DB.md): /api/health `rev` (a DB generation vs a file signature),
/api/backend (describes the backend), ?mode=hybrid (SurrealDB-only search), and floats within 1e-9.
Prints a table per endpoint. Exit 1 on any difference.
"""
from __future__ import annotations

import re
import sys
import time
from collections import OrderedDict
from urllib.parse import quote

from common import DESK, Api, TempDB, TempKB, diff

RESULTS: "OrderedDict[str, list]" = OrderedDict()   # endpoint -> [checked, failed, first failure]


def record(ep: str, ok: bool, why: str = ""):
    r = RESULTS.setdefault(ep, [0, 0, ""])
    r[0] += 1
    if not ok:
        r[1] += 1
        r[2] = r[2] or why


def norm(x, roots: list[str]):
    if isinstance(x, dict):
        return {k: ("TS" if k in ("ts", "received_at", "generated_at_api") else norm(v, roots)) for k, v in x.items()}
    if isinstance(x, list):
        return [norm(v, roots) for v in x]
    if isinstance(x, str):
        for i, r in enumerate(roots):
            x = x.replace(r, "<ROOT>").replace(r.replace("\\", "/"), "<ROOT>")
        x = re.sub(r"_\d{6}_", "_HHMMSS_", x) if "punchable-" in x else x
        return x
    return x


def endpoints(api: Api) -> list[tuple[str, str]]:
    """(group, path) for every GET endpoint and its variants, from the API's own data."""
    eps = [("health", "/api/health"), ("meta", "/api/meta"), ("meta", "/api/meta?labels=40"),
           ("meta", "/api/meta?include_hidden=1"), ("today", "/api/today"), ("pipeline", "/api/pipeline"),
           ("pipeline", "/api/pipeline?limit=500"), ("sets", "/api/sets"), ("labels", "/api/labels"),
           ("inbox", "/api/inbox"), ("notes", "/api/notes"), ("decisions", "/api/decisions"),
           ("decisions", "/api/decisions?limit=500")]
    _, meta = api.get("/api/meta")
    _, allj = api.get("/api/jokes?limit=500&offset=0")
    total = allj["total"]
    eps += [("jokes", f"/api/jokes?limit=500&offset={o}") for o in range(0, total + 1, 500)]
    eps += [("jokes", "/api/jokes"), ("jokes", "/api/jokes?include_hidden=1&limit=500")]
    for st in meta["stages"]:
        eps.append(("jokes", f"/api/jokes?stage={st}&limit=500"))
    for lab in meta["top_labels"][:12]:
        eps.append(("jokes", f"/api/jokes?label={quote(lab['id'])}&limit=500"))
    for w in ("ai", "confirmed", "edited"):
        eps.append(("jokes", f"/api/jokes?who={w}&limit=500"))
    for q in ("airport", "dating", "my dad", "airport", "the", "zzqx", "date", "Parent panic"):
        eps.append(("jokes", f"/api/jokes?q={quote(q)}&limit=500"))
    # API_CONTRACT "Requested by app": Desk sorts, AND-ed labels, pages above 500, learned, export
    for srt in ("performed", "strongest", "written", "untouched"):
        eps += [("jokes", f"/api/jokes?sort={srt}&limit=5000"), ("jokes", f"/api/jokes?sort={srt}&limit=500&offset=500")]
    l1, l2 = meta["top_labels"][0]["id"], meta["top_labels"][1]["id"]
    eps += [("jokes", f"/api/jokes?label={quote(l1)}&label={quote(l2)}&limit=500&sort=strongest"),
            ("jokes", "/api/jokes?sort=nope"), ("learned", "/api/learned"), ("export", "/api/export")]
    ids = []
    for o in range(0, total, 500):
        _, page = api.get(f"/api/jokes?limit=500&offset={o}&include_hidden=1")
        ids += [x["id"] for x in page["items"]]
    for jid in ids:
        eps.append(("joke", f"/api/jokes/{quote(jid, safe=':')}"))
    _, hidden = api.get("/api/jokes?include_hidden=1&limit=500&stage=")
    eps.append(("joke", "/api/jokes/joke:does-not-exist"))
    _, sets = api.get("/api/sets")
    for s in sets["sets"]:
        sid = quote(s["id"], safe=":")
        eps += [("set review", f"/api/sets/{sid}/review"), ("set analysis", f"/api/sets/{sid}/analysis")]
    eps += [("set review", "/api/sets/set:nope/review"), ("set analysis", "/api/sets/set:nope/analysis"),
            ("not found", "/api/nope")]
    return eps


def compare_inside(api: Api, phase: str):
    """SurrealDB (default) vs ?backend=files, same API, same temp files."""
    eps = endpoints(api)
    for group, path in eps:
        sep = "&" if "?" in path else "?"
        s1, a = api.get(path)
        s2, b = api.get(path + sep + "backend=files")
        if group == "export":
            a, b = {k: x for k, x in a.items() if k != "exported_at"}, {k: x for k, x in b.items() if k != "exported_at"}
        if group == "health":
            a, b = dict(a), dict(b)
            a.pop("rev", None)
            b.pop("rev", None)
        d = ([f"status {s1} != {s2}"] if s1 != s2 else []) + diff(a, b)
        record(f"{phase} GET {group}", not d, f"{path}: {d[:3]}")
    # audio: bytes, status and headers (identical code path, checked anyway)
    _, sets = api.get("/api/sets")
    for s in sets["sets"][:3]:
        for fmt, rng in (("", None), ("", "bytes=0-1023"), ("?fmt=wav", "bytes=1000-"), ("", "bytes=999999999-")):
            h = {"Range": rng} if rng else {}
            path = f"/api/audio/{quote(s['id'], safe=':')}{fmt}"
            sa, ba, ha = api.get(path, raw=True, headers=h)
            sb, bb, hb = api.get(path + ("&" if "?" in path else "?") + "backend=files", raw=True, headers=h)
            ok = sa == sb and ba == bb and ha.get("Content-Range") == hb.get("Content-Range")
            record(f"{phase} GET audio", ok, f"{path} {rng}: {sa} vs {sb}")
    return len(eps)


def write_sequence(api: Api) -> list[tuple[str, dict]]:
    """Every write endpoint and op, aimed at real items picked from the data."""
    _, today = api.get("/api/today")
    _, sets = api.get("/api/sets")
    _, page = api.get("/api/jokes?limit=60")
    jokes = page["items"]
    _, j0 = api.get(f"/api/jokes/{quote(jokes[0]['id'], safe=':')}")
    lab_edges = [e for e in j0["labels"]]
    link_edges = [e for e in j0["links"]]
    perfs = []
    for s in sets["sets"]:
        perfs += [x for x in s["jokes"]]
    p_known = [p for p in perfs if p["joke"]]
    p_any = perfs
    other = next(j["id"] for j in jokes if j["id"] not in {p["joke"] for p in perfs})
    seq = [
        ("/api/decision", {"target": "edge", "action": "confirm", "id": lab_edges[0]["id"]}),
        ("/api/decision", {"target": "edge", "action": "reject", "id": lab_edges[1]["id"], "reason": "not this one"}),
        ("/api/decision", {"target": "edge", "action": "edit", "id": (link_edges or lab_edges)[-1]["id"], "reason": "fine but say why"}),
        ("/api/decision", {"target": "node", "action": "edit", "id": jokes[1]["id"], "after": {"stage": "retired"}}),
        ("/api/decision", {"target": "node", "action": "confirm", "id": jokes[2]["id"]}),
        ("/api/decision", {"target": "node", "action": "add", "after": {"type": "label", "kind": "custom", "name": "Baseline test label", "definition": "t"}}),
        ("/api/decision", {"target": "edge", "action": "add", "after": {"src": jokes[3]["id"], "dst": "label:custom:baseline-test-label", "kind": "has_label"}, "reason": "it fits"}),
        ("/api/decision", {"target": "edge", "action": "add", "after": {"src": jokes[3]["id"], "dst": jokes[4]["id"], "kind": "custom", "custom_kind": "same crowd"}, "reason": "both kill in AC"}),
        ("/api/decision", {"target": "node", "action": "edit", "id": "label:custom:baseline-test-label", "after": {"name": "Renamed label"}}),
        ("/api/decision", {"target": "edge", "action": "add", "after": {"src": jokes[3]["id"], "dst": jokes[4]["id"], "kind": "custom", "custom_kind": "same crowd"}, "reason": "dup"}),  # 409
        ("/api/decision", {"target": "node", "action": "edit", "id": jokes[1]["id"], "after": {"bogus": 1}}),  # 400
        ("/api/review/rate", {"perf": p_known[0]["perf"], "rating": "killed"}),
        ("/api/review/rate", {"perf": p_known[0]["perf"], "rating": "killed"}),  # 409 already
        ("/api/review/rate", {"perf": p_known[1]["perf"], "rating": "died", "confirm_match": False}),
        ("/api/set-row", {"perf": p_known[2]["perf"], "op": "match", "joke": other}),
        ("/api/set-row", {"perf": p_known[2]["perf"], "op": "match", "joke": other}),
        ("/api/set-row", {"perf": p_known[3]["perf"], "op": "confirm"}),
        ("/api/set-row", {"perf": p_known[4]["perf"], "op": "crowd"}),
        ("/api/set-row", {"perf": p_known[4]["perf"], "op": "new"}),
        ("/api/set-row", {"perf": p_known[5]["perf"], "op": "bit"}),
        ("/api/set-row", {"perf": p_known[6]["perf"], "op": "rate", "rating": "worked"}),
        ("/api/set-row", {"perf": p_known[7]["perf"], "op": "new", "reason": "brand new tag"}),
        ("/api/set-row", {"perf": p_any[-1]["perf"], "op": "bogus"}),
        ("/api/review/reject-match", {"perf": p_known[8]["perf"], "reason": "wrong joke"}),
        ("/api/ideas", {"text": "What if the airport had wifi", "from": {"set": sets["sets"][0]["id"], "perf": p_any[0]["perf"], "start_ms": 1000, "end_ms": 5000}}),
        ("/api/ideas", {"text": ""}),
        ("/api/tonight", {"items": [{"joke": jokes[0]["id"], "done": False}, {"joke": jokes[5]["id"], "done": True}]}),
        ("/api/tonight", {"items": [{"joke": jokes[0]["id"], "done": True}]}),
        ("/api/undo", {}),
        ("/api/undo", {"indexes": None}),
        ("/api/undo", {"index": 0}),
        ("/api/decision", {"target": "edge", "action": "delete", "id": "e:doesnotexist"}),
        # his set edits (kb.py NODE_EDIT_FIELDS["set"]) and banter on a performance
        ("/api/decision", {"target": "node", "action": "edit", "id": sets["sets"][0]["id"], "after": {"show_date": "2025-10-09"}}),
        ("/api/decision", {"target": "node", "action": "edit", "id": sets["sets"][0]["id"], "after": {"venue": "Borgata"}}),
        ("/api/decision", {"target": "node", "action": "edit", "id": sets["sets"][1]["id"], "after": {"audio_unreliable": True}}),
        ("/api/decision", {"target": "node", "action": "edit", "id": sets["sets"][1]["id"], "after": {"show_date": "13/7/24"}}),  # 400
        ("/api/decision", {"target": "node", "action": "edit", "id": p_known[9]["perf"], "after": {"kind": "banter"}}),
        ("/api/set-row", {"perf": p_known[10]["perf"], "op": "banter"}),
        ("/api/undo", {}),
    ]
    return seq


def main():
    t0 = time.time()
    ta, tb = TempKB("base_files"), TempKB("base_surreal")
    db = TempDB("base")
    a = b = None
    try:
        info = db.load(tb)
        print(f"temp db {db.ns}/{db.db}: graph {info['graph_version']} {info['counts']}")
        b = Api(tb, "surreal", db)
        a = Api(ta, "files")
        n = compare_inside(b, "before writes")
        print(f"before writes: {n} GET endpoint variants compared")
        # same writes to both APIs, compare responses
        roots = [str(ta.root), str(tb.root)]
        for path, body in write_sequence(b):
            sa, ra = a.post(path, body)
            sb, rb = b.post(path, body)
            d = ([f"status {sa} != {sb}"] if sa != sb else []) + diff(norm(ra, roots), norm(rb, roots))
            record(f"POST {path}", not d, f"{body}: {d[:3]}")
        # upload: no decision line, same response shape
        import urllib.request
        ups = []
        for api in (a, b):
            req = urllib.request.Request(api.url + "/api/upload", data=b"RIFF0000WAVE", method="POST",
                                         headers={"Content-Type": "audio/wav", "X-Kind": "note", "X-Filename": "test.wav", "X-Title": "t"})
            with urllib.request.urlopen(req) as r:
                ups.append(__import__("json").loads(r.read()))
        record("POST /api/upload", not diff(norm(ups[0], roots), norm(ups[1], roots)), str(ups))
        n2 = compare_inside(b, "after writes")
        # cross-API: SurrealDB API vs files-only API, every GET, after the same writes
        for group, path in endpoints(b):
            s1, x = a.get(path)
            s2, y = b.get(path)
            if group == "health":
                x.pop("rev", None)
                y.pop("rev", None)
            if group in ("inbox", "notes"):
                continue  # each API has its own temp inbox with its own upload
            if group == "export":
                x, y = ({k: v for k, v in z.items() if k not in ("exported_at", "inbox")} for z in (x, y))
            d = ([f"status {s1} != {s2}"] if s1 != s2 else []) + diff(norm(x, roots), norm(y, roots))
            record(f"files API vs surreal API GET {group}", not d, f"{path}: {d[:3]}")
        # 4. new pipeline output and another writer: the API's file watcher runs db/load.py's incremental sync,
        #    and the result must again equal the files backend everywhere.
        import shutil
        fx = DESK / "api-dev" / "fixtures" / "sets" / "20230428_FakeSet0004" / "analysis.json"
        if fx.is_file():
            shutil.copy(fx, tb.corpus / "sets" / "20230428_FakeSet0004" / "analysis.json")
        _, before = b.get("/api/backend")
        n_sync = len(before["backends"]["surreal"]["file_syncs"])
        _, jj = b.get("/api/jokes?limit=3")
        s_, _ = b.post("/api/decision?backend=files", {"target": "node", "action": "edit", "id": jj["items"][2]["id"],
                                                        "after": {"stage": "idea"}, "reason": "written by another writer"})
        record("POST ?backend=files (another writer)", s_ == 200, str(s_))
        t1 = time.time()
        while time.time() - t1 < 60:
            _, st = b.get("/api/backend")
            sv = st["backends"]["surreal"]
            if len(sv["file_syncs"]) > n_sync and not sv["loading"]:
                _, hf = b.get("/api/health?backend=files")
                _, hs = b.get("/api/health")
                if hf["decision_lines"] == hs["decision_lines"]:
                    break
            time.sleep(0.5)
        _, st = b.get("/api/backend")
        print(f"watcher sync: {st['backends']['surreal']['file_syncs'][n_sync:]} error={st['backends']['surreal']['sync_error']}")
        record("watcher sync ran", len(st["backends"]["surreal"]["file_syncs"]) > n_sync, str(st["backends"]["surreal"]))
        compare_inside(b, "after watcher sync")
        _, h = b.get("/api/backend")
        _, hh = b.get("/api/health")
        print(f"after writes: {n2} GET variants compared; db gen {h['backends']['surreal']['gen']}, "
              f"decision lines {hh['decision_lines']}")
    finally:
        for x in (a, b):
            if x:
                x.stop()
        db.drop()
        ta.cleanup()
        tb.cleanup()
    width = max(len(k) for k in RESULTS)
    print(f"\n{'endpoint'.ljust(width)}  checked  failed")
    bad = 0
    for k, (n, f, why) in RESULTS.items():
        print(f"{k.ljust(width)}  {n:7d}  {f:6d}" + (f"   first: {why[:300]}" if f else ""))
        bad += f
    print(f"\n{'PASS' if not bad else 'FAIL'}: {sum(v[0] for v in RESULTS.values())} comparisons, {bad} different, "
          f"{time.time() - t0:.0f}s")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
