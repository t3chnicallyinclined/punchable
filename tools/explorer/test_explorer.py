"""Desk explorer tests: replay logic, decisions file, and every HTTP endpoint.

    python tools/explorer/test_explorer.py

Works on a TEMP copy of the fixture. Never touches comedy/kb.
"""
import hashlib
import json
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import kb as K  # noqa: E402

FIX = HERE / "fixtures"
FAILS = []


def check(cond, msg):
    print(("PASS " if cond else "FAIL ") + msg)
    if not cond:
        FAILS.append(msg)


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def tmpkb():
    d = Path(tempfile.mkdtemp(prefix="explorer_t_"))
    shutil.copy(FIX / "graph.sample.json", d / "graph.json")
    shutil.copy(FIX / "KB_REPORT.sample.md", d / "KB_REPORT.md")
    return d, K.KB(d / "graph.json", d / "decisions.jsonl", FIX / "corpus", d / "KB_REPORT.md")


def lines(p):
    return [json.loads(x) for x in Path(p).read_text(encoding="utf-8").splitlines() if x.strip()]


def logic_tests():
    d, kb = tmpkb()
    g0 = sha(d / "graph.json")
    v = kb.view()
    check(len(v.jokes) == 6 and len(v.edges) == 37, "fixture loads: 6 jokes, 37 edges")
    check(v.report[:2] == K.parse_report(FIX / "KB_REPORT.sample.md") and len(v.report) == 2, "KB_REPORT: only IDs under the 'first 50' heading (2)")
    items, total = v.queue()
    check(items[0]["id"] == v.report[0] and items[1]["id"] == v.report[1], "queue: KB_REPORT order first")
    rest = [i["score"] for i in items[2:]]
    check(rest == sorted(rest, reverse=True), "queue: then by learning value (score desc)")

    belt_lab = K.edge_id("joke:airport-belt", "has_label", "label:device:specificity")
    r = kb.decide({"target": "edge", "id": belt_lab, "action": "confirm", "reason": "yes, the belt"})
    ln = lines(d / "decisions.jsonl")
    check(len(ln) == 1 and set(ln[0]) == {"ts", "target", "id", "action", "before", "after", "reason", "by"} and ln[0]["by"] == "owner",
          "confirm appends exactly one line with ts,target,id,action,before,after,reason,by")
    check(kb.view().edges[belt_lab]["status"] == "confirmed", "confirm applied in the view")
    rej = K.edge_id("joke:plant-dad", "same_premise", "joke:gym-mirror")
    kb.decide({"target": "edge", "id": rej, "action": "reject"})
    v = kb.view()
    check(all(i["id"] != rej for i in v.queue()[0]) and any(i["id"] == rej for i in v.queue("rejected")[0]),
          "rejected: gone from queue, listed under rejected")
    ed = K.edge_id("joke:gps-voice", "same_shape", "joke:dating-40")
    kb.decide({"target": "edge", "id": ed, "action": "edit", "after": {"kind": "custom", "custom_kind": "same_target", "dst": "joke:old-phone"}, "reason": "both are about being judged"})
    e = kb.view().edges[ed]
    check(e["kind"] == "custom" and e["custom_kind"] == "same_target" and e["dst"] == "joke:old-phone" and e["status"] == "confirmed",
          "edit: kind/custom kind/target changed and confirmed")
    kb.decide({"target": "node", "action": "add", "after": {"type": "label", "name": "Self own", "definition": "the punch is on me"}})
    check("label:custom:self-own" in kb.view().nodes, "add custom label")
    a = kb.decide({"target": "edge", "action": "add", "after": {"src": "joke:gym-mirror", "dst": "label:custom:self-own", "kind": "has_label"}, "reason": "I am the joke"})
    check(kb.view().edges[a["decision"]["id"]]["status"] == "confirmed", "add connection (confirmed, yours)")
    for bad, why in [({"target": "edge", "action": "add", "after": {"src": "joke:gym-mirror", "dst": "joke:gps-voice", "kind": "segue"}}, "add without reason"),
                     ({"target": "edge", "action": "add", "after": {"src": "joke:gym-mirror", "dst": "label:custom:self-own", "kind": "has_label"}, "reason": "x"}, "duplicate add"),
                     ({"target": "edge", "action": "add", "after": {"src": "joke:gym-mirror", "dst": "joke:nope", "kind": "segue"}, "reason": "x"}, "unknown node"),
                     ({"target": "edge", "id": rej, "action": "delete"}, "delete a pipeline edge"),
                     ({"target": "node", "id": "joke:gym-mirror", "action": "edit", "after": {"stage": "famous"}}, "bad stage"),
                     ({"target": "graph", "action": "add"}, "bad target")]:
        n0 = len(lines(d / "decisions.jsonl"))
        try:
            kb.decide(bad)
            check(False, f"rejects {why}")
        except K.DecisionError:
            check(len(lines(d / "decisions.jsonl")) == n0, f"rejects {why}, writes nothing")

    # undo: last (the added edge) -> delete
    kb.undo(None)
    v = kb.view()
    check(a["decision"]["id"] not in v.edges, "undo of add = delete line; edge gone")
    ln = lines(d / "decisions.jsonl")
    check(ln[-1]["action"] == "delete" and ln[-1]["undo_of"] == 4, "undo appended an inverse line (delete, undo_of 4)")
    kb.undo(2)  # the edit
    e = kb.view().edges[ed]
    check(e["kind"] == "same_shape" and e["dst"] == "joke:dating-40" and e["status"] == "proposed" and e.get("custom_kind") is None,
          "undo of edit restores kind, target and status")
    try:
        kb.undo(2)
        check(False, "double undo refused")
    except K.DecisionError:
        check(True, "double undo refused")
    kb.decide({"target": "node", "id": "label:device:repetition", "action": "edit", "after": {"merged_into": "label:device:analogy"}})
    v = kb.view()
    labs = {x["id"]: x for x in v.label_index()}
    check("label:device:repetition" not in labs and "label:device:repetition" in labs["label:device:analogy"]["merged_from"],
          "merge: label folded into the other, recorded as one line")
    check(sum(1 for c in labs["label:device:analogy"]["carriers"] if c["joke"]["id"] == "joke:gps-voice") == 1,
          "merge: duplicate has_label after merge is shown once")
    kb.undo(None)
    check("label:device:repetition" in {x["id"] for x in kb.view().label_index()}, "undo merge")
    kb.decide({"target": "node", "id": "joke:gym-mirror", "action": "edit", "after": {"stage": "to_try"}})

    # reload persistence: a fresh KB on the same files gives the same view
    v1 = kb.view()
    kb2 = K.KB(d / "graph.json", d / "decisions.jsonl", FIX / "corpus", d / "KB_REPORT.md")
    v2 = kb2.view()
    same = all(K.public(v1.edges[k]) == K.public(v2.edges.get(k, {})) for k in v1.edges) and v1.nodes["joke:gym-mirror"]["stage"] == v2.nodes["joke:gym-mirror"]["stage"] == "to_try"
    check(same and len(v1.edges) == len(v2.edges), "reload: fresh process replays to the identical view")
    st = v2.stats()
    check(st["by_kind"].get("has_label", {}).get("confirm") == 1 and st["by_kind"].get("same_premise", {}).get("reject") == 1
          and st["by_kind"].get("same_shape") is None and len(st["added_labels"]) == 1 and st["added_edges"] == [],
          "stats: per-kind confirm/reject, undone items excluded, custom label listed")

    # torn last line + garbage line
    with open(d / "decisions.jsonl", "ab") as f:
        f.write(b'{"ts":"x","target":"edge"')
    kb.decide({"target": "edge", "id": rej, "action": "confirm"})
    raw = (d / "decisions.jsonl").read_text(encoding="utf-8").splitlines()
    check(json.loads(raw[-1])["id"] == rej and kb.bad_lines == 1, "torn last line: new decision lands on its own line; bad line counted, skipped")

    # pipeline re-run drops an edge the comic confirmed -> it survives from the decision's before snapshot
    g = json.loads((d / "graph.json").read_text(encoding="utf-8"))
    g["edges"] = [x for x in g["edges"] if x["id"] != belt_lab]
    (d / "graph.json").write_text(json.dumps(g), encoding="utf-8")
    check(kb.view().edges.get(belt_lab, {}).get("status") == "confirmed", "regenerated graph without a confirmed edge: decision keeps it")

    # graph missing -> appears later, no restart
    d2 = Path(tempfile.mkdtemp(prefix="explorer_m_"))
    kb3 = K.KB(d2 / "graph.json", d2 / "decisions.jsonl", None, None)
    check(kb3.view() and kb3.graph_state == "missing" and not kb3.view().jokes, "graph missing: empty view, state=missing")
    (d2 / "graph.json").write_text('{"nodes": [', encoding="utf-8")
    kb3.view()
    check(kb3.graph_state == "unreadable", "half-written graph: state=unreadable, no crash")
    shutil.copy(FIX / "graph.sample.json", d2 / "graph.json")
    check(kb3.graph_state and len(kb3.view().jokes) == 6 and kb3.graph_state == "ok", "graph appears: loaded with no restart")
    try:
        K.KB(d / "graph.json", d / "graph.json", None, None)
        check(False, "refuses decisions == graph.json")
    except SystemExit:
        check(True, "refuses decisions == graph.json")
    check(sha(FIX / "graph.sample.json") == sha(d2 / "graph.json"), "fixture graph never modified")
    del g0


def http_tests():
    d = Path(tempfile.mkdtemp(prefix="explorer_h_"))
    shutil.copy(FIX / "graph.sample.json", d / "graph.json")
    shutil.copy(FIX / "KB_REPORT.sample.md", d / "KB_REPORT.md")
    g0 = sha(d / "graph.json")
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    srv = subprocess.Popen([sys.executable, str(HERE / "server.py"), "--port", str(port), "--graph", str(d / "graph.json"),
                            "--corpus", str(FIX / "corpus")], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    base = f"http://127.0.0.1:{port}"

    def req(path, body=None, method=None):
        data = json.dumps(body).encode() if body is not None else None
        r = urllib.request.Request(base + path, data=data, method=method or ("POST" if data is not None else "GET"),
                                   headers={"Content-Type": "application/json"} if data else {})
        try:
            with urllib.request.urlopen(r, timeout=10) as resp:
                raw = resp.read()
                return resp.status, (json.loads(raw) if resp.headers.get("Content-Type", "").startswith("application/json") else raw)
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read() or b"{}")

    try:
        for _ in range(100):
            try:
                if req("/api/health")[0] == 200:
                    break
            except OSError:
                time.sleep(0.1)
        st, h = req("/api/health")
        check(st == 200 and h["graph_state"] == "ok" and h["decisions"].endswith("decisions.jsonl") and Path(h["decisions"]).parent == d.resolve(),
              "GET /api/health (decisions default next to --graph)")
        for p in ("/", "/static/app.js", "/static/app.css", "/static/graph.js"):
            st, b = req(p)
            check(st == 200 and len(b) > 500, f"GET {p}")
        check(req("/static/../server.py")[0] == 404 and req("/nope")[0] == 404, "404 for unknown paths and traversal")
        st, m = req("/api/meta")
        check(st == 200 and m["counts"]["jokes"] == 6 and "has_label" in m["edge_kinds"], "GET /api/meta")
        st, j = req("/api/jokes")
        check(st == 200 and len(j["jokes"]) == 6 and all("counts" in x for x in j["jokes"]), "GET /api/jokes (counts per joke)")
        check([x["id"] for x in req("/api/jokes?q=sniffed")[1]["jokes"]] == ["joke:airport-belt"], "GET /api/jokes?q= searches unit text")
        check([x["id"] for x in req("/api/jokes?q=misdirect")[1]["jokes"]] == ["joke:dating-40"], "GET /api/jokes?q= searches labels")
        check({x["id"] for x in req("/api/jokes?stage=testing")[1]["jokes"]} == {"joke:airport-belt", "joke:dating-40"}, "GET /api/jokes?stage=")
        check({x["id"] for x in req("/api/jokes?label=label:device:analogy")[1]["jokes"]} == {"joke:gps-voice", "joke:dating-40"}, "GET /api/jokes?label=")
        st, jd = req("/api/joke/joke:airport-belt")
        perf = jd["performances"][0] if jd.get("performances") else {}
        check(st == 200 and len(jd["unit_groups"]) == 2 and perf.get("laughs", {}).get("max_bars") == 4 and perf["laughs"]["hits"] == 3,
              "GET /api/joke/<id>: units grouped by date, laughs in window (3, best 4 bars)")
        check(req("/api/joke/joke:nope")[0] == 404, "GET /api/joke/<missing> = 404")
        st, lb = req("/api/labels")
        check(st == 200 and len(lb["labels"]) == 12, "GET /api/labels")
        st, q = req("/api/queue")
        check(st == 200 and q["total"] == 37 and q["items"][0]["id"] == q["report_ids"][0], "GET /api/queue")
        check(req("/api/queue?status=bogus")[0] == 400, "GET /api/queue bad status = 400")
        st, g = req("/api/graph?id=joke:gps-voice&hops=2")
        st1, g1 = req("/api/graph?id=joke:gps-voice&hops=1&kinds=has_label")
        check(st == 200 and len(g["nodes"]) > len(g1["nodes"]) and all(e["kind"] == "has_label" for e in g1["edges"]), "GET /api/graph hops + kind filter")
        check(req("/api/graph?id=joke:nope")[0] == 404, "GET /api/graph missing = 404")
        check(req("/api/stats")[0] == 200 and req("/api/decisions")[0] == 200, "GET /api/stats, /api/decisions")
        eid = q["items"][0]["id"]
        st, r = req("/api/decision", {"target": "edge", "id": eid, "action": "confirm"})
        check(st == 200 and r["index"] == 0, "POST /api/decision confirm")
        check(req("/api/decision", {"target": "edge", "id": "e:000000000000", "action": "confirm"})[0] == 404, "POST unknown edge = 404")
        check(req("/api/decision", {"target": "edge", "action": "add", "after": {"src": "joke:gps-voice", "dst": "label:device:analogy", "kind": "has_label"}, "reason": "x"})[0] == 409,
              "POST duplicate add = 409")
        check(req("/api/decision", {"target": "edge", "id": "bad id with spaces", "action": "confirm"})[0] == 400, "POST bad id = 400")
        st, u = req("/api/undo", {})
        check(st == 200 and u["decision"]["undo_of"] == 0 and u["decision"]["after"] == {"status": "proposed"}, "POST /api/undo")
        check(req("/api/undo", {"index": 0})[0] == 409, "POST /api/undo twice = 409")
        st, dl = req("/api/decisions?limit=5")
        check(st == 200 and dl["total"] == 2 and dl["decisions"][0]["undo_of"] == 0 and dl["decisions"][1]["undone"], "GET /api/decisions marks undone")
    finally:
        srv.terminate()
        out = srv.communicate(timeout=10)[0].decode(errors="replace")
    check("Traceback" not in out and "ERROR" not in out, "server log clean")
    check(sha(d / "graph.json") == g0, "graph.json byte-identical after all writes")


if __name__ == "__main__":
    logic_tests()
    http_tests()
    print(f"\n{len(FAILS)} FAILED" if FAILS else "\nALL PASS")
    sys.exit(1 if FAILS else 0)
