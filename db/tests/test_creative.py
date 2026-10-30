"""Creative routes (docs/API_CONTRACT.md): SurrealDB backend vs files backend, plus live push and WebSocket auth.

    python db/tests/test_creative.py

The same discipline as test_baseline.py, on TEMP copies (data, decisions, creative logs) and a TEMP database:
1. Reads: one API on SurrealDB. Every creative GET, default (creative_event rows + decisions from the database) vs
   ?backend=files (the jsonl logs + decisions.jsonl), must be equal.
2. Writes: the same sequence (boards, reorder, add, bridge queue, promote, version, locks, venue pin, errors, the
   worker's claim and result) to that API and to a files-only API. Responses must be equal.
3. Reads again after the writes: within the SurrealDB API (both ways) and across the two APIs.
4. Live: a creative write reaches /api/live as a creative_event change.
5. Auth on /api/live (PK_AUTH=1): no token in the first message -> refused; a good one -> hello.
Allowed differences: timestamps, job ids (they carry the enqueue time), temp paths. Exit 1 on any difference.
"""
from __future__ import annotations

import json
import re
import sys
import time
from collections import OrderedDict
from urllib.parse import quote

from common import DESK, Api, TempDB, TempKB, diff

sys.path.insert(0, str(DESK / "api-dev" / "tests"))
RESULTS: "OrderedDict[str, list]" = OrderedDict()
TS_KEYS = {"ts", "last_seen", "created_at", "updated_at", "generated_at", "done_at", "claimed_at", "updated", "edited_at", "at",
           "received_at", "loaded_at"}


def record(ep, ok, why=""):
    r = RESULTS.setdefault(ep, [0, 0, ""])
    r[0] += 1
    if not ok:
        r[1] += 1
        r[2] = r[2] or why


def norm(x, roots):
    if isinstance(x, dict):
        return {k: ("TS" if k in TS_KEYS else norm(v, roots)) for k, v in x.items()}
    if isinstance(x, list):
        return [norm(v, roots) for v in x]
    if isinstance(x, str):
        for r in roots:
            x = x.replace(r, "<ROOT>").replace(r.replace("\\", "/"), "<ROOT>")
        x = re.sub(r"job:(\w+)-[0-9a-f]{12}", r"job:\1-<ID>", x)
        x = re.sub(r"^try:[0-9a-f]{8}$", "try:<ID>", x)  # his try-note ids carry the time they were written
        return x
    return x


def with_backend(path, b):
    return path + ("&" if "?" in path else "?") + "backend=" + b


def reads(api: Api) -> list[tuple[str, str]]:
    eps = [("boards", "/api/boards"), ("workingsets", "/api/workingsets"), ("venues", "/api/venues"),
           ("geojson", "/api/venues.geojson"), ("today", "/api/today"), ("settings", "/api/settings"),
           ("connections", "/api/connections")]
    _, bs = api.get("/api/boards")
    eps += [("board", f"/api/boards/{quote(b['id'])}") for b in bs["boards"]]
    _, ws = api.get("/api/workingsets")
    for w in ws["workingsets"]:
        wid = quote(w["id"])
        eps += [("workingset", f"/api/workingsets/{wid}"), ("nights", f"/api/workingsets/{wid}/nights")]
        _, d = api.get(f"/api/workingsets/{wid}")
        eps += [("bit", f"/api/workingsets/{wid}/bits/{quote(b['joke_id'])}") for b in d["version"]["bits"]]
    _, js = api.get("/api/jokes?limit=40")
    eps += [("connections", f"/api/jokes/{quote(j['id'])}/connections?cap=3") for j in js["items"]]
    _, sets = api.get("/api/sets")
    eps += [("clips", f"/api/sets/{quote(s['id'])}/clips") for s in sets["sets"]]
    return eps


def compare_within(api: Api, roots, tag):
    for ep, path in reads(api):
        s1, a = api.get(path)
        s2, b = api.get(with_backend(path, "files"))
        d = diff(norm(a, roots), norm(b, roots)) if s1 == s2 else [f"status {s1} != {s2}"]
        record(f"{tag} {ep}", not d, f"{path}: {d[:3]}")


def compare_across(sa: Api, fa: Api, roots, tag):
    for ep, path in reads(sa):
        s1, a = sa.get(path)
        s2, b = fa.get(path)
        d = diff(norm(a, roots), norm(b, roots)) if s1 == s2 else [f"status {s1} != {s2}"]
        record(f"{tag} {ep}", not d, f"{path}: {d[:3]}")


def writes(api: Api) -> list:
    """The write sequence; returns [(name, status, body)]."""
    out = []

    def w(name, method, path, body):
        st, b = api.post(path, body, method=method)
        out.append((name, st, b))
        return st, b

    _, js = api.get("/api/jokes?stage=working&limit=6")
    ids = [j["id"] for j in js["items"]]
    st, b = w("board create", "POST", "/api/boards", {"title": "Baseline board", "joke_ids": ids[:3], "target_s": 420, "notes": "n"})
    bid = b["board"]["id"]
    w("board patch", "PATCH", f"/api/boards/{quote(bid)}", {"title": "Baseline board 2", "move": {"from": 0, "to": 2}})
    w("board patch add", "PATCH", f"/api/boards/{quote(bid)}", {"add": {"joke_id": ids[3], "at": 1}})
    w("board patch bad (all or nothing)", "PATCH", f"/api/boards/{quote(bid)}", {"title": "x", "remove": "joke:not-there"})
    w("board item", "POST", f"/api/boards/{quote(bid)}/items", {"joke_id": ids[4], "after": ids[0]})
    w("board item dup", "POST", f"/api/boards/{quote(bid)}/items", {"joke_id": ids[4]})
    w("bridge queued", "POST", f"/api/boards/{quote(bid)}/bridge", {"from": ids[0], "to": ids[1]})
    w("bridge again (same job)", "POST", f"/api/boards/{quote(bid)}/bridge", {"from": ids[0], "to": ids[1]})
    st, b = w("promote", "POST", "/api/workingsets", {"board_id": bid, "target_s": 400})
    wid = b["working_set"]["id"]
    w("version", "POST", f"/api/workingsets/{quote(wid)}/versions", {"note": "closer last", "joke_ids": ids[:4][::-1]})
    w("version without note", "POST", f"/api/workingsets/{quote(wid)}/versions", {"joke_ids": ids[:2]})
    w("lock bit", "POST", f"/api/workingsets/{quote(wid)}/lock", {"joke_id": ids[0]})
    w("punch-up of a locked bit (no job)", "POST", f"/api/workingsets/{quote(wid)}/bits/{quote(ids[0])}/punchup", {})
    w("unlock bit", "POST", f"/api/workingsets/{quote(wid)}/lock", {"joke_id": ids[0], "unlock": True})
    w("lock line not verbatim", "POST", f"/api/workingsets/{quote(wid)}/lock", {"joke_id": ids[1], "line": "words he never said"})
    _, bh = api.get(f"/api/workingsets/{quote(wid)}/bits/{quote(ids[1])}")
    phrase = next((c["examples"][0] for c in (bh.get("wording") or {}).get("effects", []) if c.get("examples")), None)
    if phrase:
        w("lock line", "POST", f"/api/workingsets/{quote(wid)}/lock", {"joke_id": ids[1], "line": phrase["quote"]})
    w("lock line null", "POST", f"/api/workingsets/{quote(wid)}/lock", {"joke_id": ids[1], "line": None})
    w("punch-up queued", "POST", f"/api/workingsets/{quote(wid)}/bits/{quote(ids[1])}/punchup", {})
    _, vs = api.get("/api/venues")
    vid = vs["venues"][0]["venue_id"]
    w("venue pin", "PATCH", f"/api/venues/{quote(vid)}", {"lat": 40.7291, "lon": -73.9965, "name": "The room"})
    w("venue bad", "PATCH", f"/api/venues/{quote(vid)}", {"lat": 400, "lon": 0})
    w("venue unknown", "PATCH", "/api/venues/venue:nowhere", {"name": "x"})
    w("board unknown", "PATCH", "/api/boards/sb:nope", {"title": "x"})
    w("ask queued", "POST", "/api/ask", {"question": "every version of the airport bit"})
    st, b = w("worker claim", "POST", "/api/worker/claim", {"kinds": ["bridge"], "worker": "test"})
    if st == 200:
        w("worker result", "POST", f"/api/worker/jobs/{quote(b['job']['id'])}/result",
          {"ok": True, "result": {"status": "ok", "text": "a bridge", "draft": True, "label": "DRAFT"}})
    w("bridge cached", "POST", f"/api/boards/{quote(bid)}/bridge", {"from": ids[0], "to": ids[1]})
    # API_CONTRACT "Requested by app": punch-up of any bit, try notes, YouTube import, settings
    w("joke punch-up queued", "POST", f"/api/jokes/{quote(ids[2])}/punchup", {})
    w("joke punch-up again (same job)", "POST", f"/api/jokes/{quote(ids[2])}/punchup", {})
    w("lock bit for joke punch-up", "POST", f"/api/workingsets/{quote(wid)}/lock", {"joke_id": ids[3]})
    w("joke punch-up of a locked bit", "POST", f"/api/jokes/{quote(ids[3])}/punchup", {})
    st, b = w("try note", "POST", f"/api/workingsets/{quote(wid)}/try", {"joke_id": ids[0], "text": "say it slower"})
    if st == 201:
        w("try tick", "PATCH", f"/api/workingsets/{quote(wid)}/try/{quote(b['note']['id'])}", {"done": True})
    w("try tick AI note", "PATCH", f"/api/workingsets/{quote(wid)}/try/{quote(ids[0] + '|dropped', safe='')}", {"done": True})
    w("try note bad", "POST", f"/api/workingsets/{quote(wid)}/try", {"joke_id": ids[0]})
    w("youtube queued", "POST", "/api/import/youtube", {"url": "https://youtu.be/dQw4w9WgXcQ"})
    w("youtube bad", "POST", "/api/import/youtube", {"url": "https://vimeo.com/1"})
    w("settings", "PATCH", "/api/settings", {"provider": "key", "training": True})
    w("settings bad", "PATCH", "/api/settings", {"provider": "openai"})
    return out


def live_check(api: Api) -> bool:
    import websocket
    ws = websocket.create_connection(api.url.replace("http", "ws") + "/api/live", timeout=10)
    try:
        hello = json.loads(ws.recv())
        assert hello["type"] == "hello", hello
        api.post("/api/boards", {"title": "Live board", "joke_ids": []})
        t = time.time()
        while time.time() - t < 5:
            m = json.loads(ws.recv())
            if m.get("type") == "change" and any(c.get("table") == "creative_event" for c in m.get("changes", [])):
                print(f"  live: creative_event pushed in {(time.time() - t) * 1000:.0f} ms")
                return True
        return False
    finally:
        ws.close()


def ws_auth_check(t: TempKB, db: TempDB) -> list[str]:
    import websocket
    from jwt_kit import JWKSServer, Key, auth_env, token, worker_token
    key = Key("kw", seed=11)
    jw = JWKSServer([key])
    api = Api(t, "surreal", db, env=auth_env(jw.url))
    fails = []
    try:
        url = api.url.replace("http", "ws") + "/api/live"
        for name, first, want in (("no auth message", {"type": "hello"}, "error"),
                                  ("bad token", {"type": "auth", "token": token(Key("kw", seed=12))}, "error"),
                                  ("worker token", {"type": "auth", "token": worker_token(key)}, "error"),
                                  ("good token", {"type": "auth", "token": token(key)}, "hello")):
            ws = websocket.create_connection(url, timeout=10)
            try:
                ws.send(json.dumps(first))
                got = json.loads(ws.recv()).get("type")
            finally:
                ws.close()
            if got != want:
                fails.append(f"{name}: got {got}, want {want}")
            print(f"  ws auth: {name} -> {got}")
        st, _ = api.get("/api/boards")
        if st != 401:
            fails.append(f"/api/boards without a token: {st}")
    finally:
        api.stop()
        jw.close()
    return fails


def main():
    t0 = time.time()
    ts, tf = TempKB("cr_s"), TempKB("cr_f")
    db = TempDB("creative")
    roots = [str(ts.root), str(tf.root)]
    fails = []
    sa = fa = None
    try:
        db.load(ts)
        sa = Api(ts, "surreal", db)
        fa = Api(tf, "files")
        compare_within(sa, roots, "1 within")
        compare_across(sa, fa, roots, "1 across")
        ws, wf = writes(sa), writes(fa)
        for (name, s1, b1), (_, s2, b2) in zip(ws, wf):
            d = diff(norm(b1, roots), norm(b2, roots)) if s1 == s2 else [f"status {s1} != {s2}"]
            record(f"2 write {name}", not d, str(d[:3]))
            print(f"  {name:36s} {s1}")
        compare_within(sa, roots, "3 within")
        compare_across(sa, fa, roots, "3 across")
        if not live_check(sa):
            fails.append("live: no creative_event change within 5 s")
        sa.stop()
        sa = None
        fails += ws_auth_check(ts, db)
    finally:
        for a in (sa, fa):
            if a:
                a.stop()
        db.drop()
        ts.cleanup()
        tf.cleanup()
    total = sum(v[0] for v in RESULTS.values())
    bad = sum(v[1] for v in RESULTS.values())
    print(f"\n{'check':44s} {'n':>5s} {'diff':>5s}")
    groups: "OrderedDict[str, list]" = OrderedDict()
    for k, v in RESULTS.items():
        g = groups.setdefault(" ".join(k.split()[:2]) if not k.startswith("2 ") else k, [0, 0, ""])
        g[0] += v[0]
        g[1] += v[1]
        g[2] = g[2] or v[2]
    for k, v in groups.items():
        print(f"{k[:44]:44s} {v[0]:5d} {v[1]:5d}" + (f"   {v[2][:160]}" if v[1] else ""))
    print(f"\n{total} comparisons, {bad} different; {len(fails)} other failures {fails}; {time.time() - t0:.0f}s")
    sys.exit(1 if bad or fails else 0)


if __name__ == "__main__":
    main()
