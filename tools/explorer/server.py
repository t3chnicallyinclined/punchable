"""Desk explorer: browse, confirm, change and extend the joke knowledge graph.

    python tools/explorer/server.py                       # http://127.0.0.1:8795/
    python tools/explorer/server.py --graph tools/explorer/fixtures/graph.sample.json --decisions %TEMP%/d.jsonl

Binds to 127.0.0.1 only. No network calls, no CDN. Reads graph.json (never
writes it). Every action appends one line to decisions.jsonl (fsynced).
Standard library only.
"""
from __future__ import annotations

import argparse
import json
import mimetypes
import os
import re
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlparse

sys.path.insert(0, str(Path(__file__).resolve().parent))
import kb as K  # noqa: E402

HERE = Path(__file__).resolve().parent
STATIC = HERE / "static"
_COMEDY = Path(os.path.expanduser(os.environ.get("COMEDY_DIR", "~/punchable-data")))
DEFAULT_KB = _COMEDY / "kb" / "v0"
DEFAULT_CORPUS = _COMEDY / "corpus"
DEFAULT_PORT = 8795
MAX_BODY = 256 * 1024
NODE_ID_RE = re.compile(r"^[A-Za-z0-9:_@.+\-]{1,300}$")

mimetypes.add_type("text/javascript", ".js")
mimetypes.add_type("text/css", ".css")


class Handler(BaseHTTPRequestHandler):
    kb: K.KB = None
    server_version = "desk-explorer/0"

    def handle(self):
        try:
            super().handle()
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            pass

    def log_message(self, fmt, *args):
        sys.stderr.write("%s %s\n" % (self.log_date_time_string(), fmt % args))

    def send_json(self, obj, status=200):
        b = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(b)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(b)

    def err(self, status, msg):
        self.send_json({"error": msg}, status)

    def send_file(self, p: Path):
        b = p.read_bytes()
        self.send_response(200)
        self.send_header("Content-Type", (mimetypes.guess_type(p.name)[0] or "application/octet-stream") + "; charset=utf-8")
        self.send_header("Content-Length", str(len(b)))
        self.send_header("Cache-Control", "no-cache")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(b)

    def body_json(self):
        n = int(self.headers.get("Content-Length") or 0)
        if n <= 0 or n > MAX_BODY:
            raise K.DecisionError("body missing or too large")
        try:
            b = json.loads(self.rfile.read(n).decode("utf-8"))
        except ValueError:
            raise K.DecisionError("body is not JSON")
        if not isinstance(b, dict):
            raise K.DecisionError("body must be an object")
        return b

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        u = urlparse(self.path)
        path, q = u.path, parse_qs(u.query)
        arg = lambda k, d=None: q.get(k, [d])[0]
        kb = self.kb
        try:
            if path in ("/", "/index.html"):
                return self.send_file(STATIC / "index.html")
            if path.startswith("/static/"):
                name = path[len("/static/"):]
                p = STATIC / name
                if not re.match(r"^[A-Za-z0-9_.-]+$", name) or not p.is_file():
                    return self.err(404, "not found")
                return self.send_file(p)
            if path == "/api/health":
                v = kb.view()
                return self.send_json({"ok": True, "graph": str(kb.graph_path), "graph_state": kb.graph_state,
                                       "graph_error": kb.graph_error, "decisions": str(kb.decisions_path),
                                       "decision_lines": len(v.decisions), "bad_lines": kb.bad_lines,
                                       "orphan_decisions": v.orphans, "report": str(kb.report_path) if kb.report_path else None,
                                       "report_ids": len(v.report), "corpus": str(kb.corpus) if kb.corpus else None})
            v = kb.view()
            if path == "/api/meta":
                return self.send_json({
                    "graph": v.graph_meta, "graph_state": kb.graph_state, "graph_path": str(kb.graph_path),
                    "counts": {"jokes": len(v.jokes), "labels": len(v.labels), "edges": len(v.edges),
                               "nodes": len(v.nodes), "decisions": len(v.decisions)},
                    "edge_kinds": K.EDGE_KINDS, "stages": K.STAGES, "label_kinds": K.LABEL_KINDS,
                    "custom_kinds": sorted({e["custom_kind"] for e in v.edges.values() if e.get("custom_kind")}),
                    "jokes": [{"id": j["id"], "title": j.get("title") or j["id"], "stage": j.get("stage")} for j in v.jokes],
                    "labels": [{"id": l["id"], "name": l.get("name") or l["id"], "kind": l.get("kind")} for l in v.labels],
                })
            if path == "/api/jokes":
                items = v.joke_summaries()
                qs = (arg("q") or "").strip()
                if qs:
                    hits = set(v.search(qs))
                    items = [x for x in items if x["id"] in hits]
                if arg("stage"):
                    items = [x for x in items if x["stage"] == arg("stage")]
                if arg("label"):
                    items = [x for x in items if any(l["id"] == arg("label") for l in x["labels"])]
                return self.send_json({"jokes": items, "total": len(v.jokes)})
            m = re.match(r"^/api/joke/(.+)$", path)
            if m:
                d = v.joke_detail(unquote(m.group(1)))
                return self.send_json(d) if d else self.err(404, "no such joke")
            if path == "/api/labels":
                return self.send_json({"labels": v.label_index(),
                                       "merged": [{"id": k, "into": x, "name": v.nodes[k].get("name")} for k, x in v.alias.items()
                                                  if v.ntype(k) == "label"]})
            if path == "/api/queue":
                status = arg("status", "proposed")
                if status not in ("proposed", "rejected", "confirmed"):
                    return self.err(400, "status must be proposed|rejected|confirmed")
                items, total = v.queue(status, min(int(arg("limit", "200")), 2000))
                return self.send_json({"items": items, "total": total, "report_ids": v.report})
            if path == "/api/graph":
                center = arg("id") or ""
                hops = 2 if arg("hops") == "2" else 1
                kinds = {k for k in (arg("kinds") or "").split(",") if k} or None
                types = {"joke", "label"} | ({"unit", "performance"} if arg("all") == "1" else set())
                g = v.neighborhood(center, hops, kinds, types, arg("rejected") == "1")
                return self.send_json(g) if g else self.err(404, "no such node")
            if path == "/api/stats":
                return self.send_json(v.stats())
            if path == "/api/decisions":
                n = min(int(arg("limit", "50")), 1000)
                ds = [dict(d, index=i, undone=i in v.undone) for i, d in enumerate(v.decisions)][-n:][::-1]
                for d in ds:
                    d["title"] = v.brief(d.get("id")).get("title") if d.get("target") == "node" else None
                    if d.get("target") == "edge":
                        e = v.edges.get(d.get("id")) or d.get("before") or d.get("after") or {}
                        d["title"] = f"{v.brief(e.get('src')).get('title')} → {K.ekind(e)} → {v.brief(e.get('dst')).get('title')}"
                return self.send_json({"decisions": ds, "total": len(v.decisions)})
            return self.err(404, "not found")
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            pass
        except ValueError as e:
            return self.err(400, str(e))
        except Exception as e:  # noqa: BLE001
            self.log_message("ERROR %r", e)
            return self.err(500, f"{type(e).__name__}: {e}")

    def do_POST(self):
        path = urlparse(self.path).path
        try:
            if path == "/api/decision":
                b = self.body_json()
                if b.get("id") is not None and (not isinstance(b["id"], str) or not NODE_ID_RE.match(b["id"])):
                    raise K.DecisionError("bad id")
                return self.send_json(self.kb.decide(b))
            if path == "/api/undo":
                b = self.body_json() if int(self.headers.get("Content-Length") or 0) else {}
                idx = b.get("index")
                if idx is not None and not isinstance(idx, int):
                    raise K.DecisionError("index must be an integer")
                return self.send_json(self.kb.undo(idx))
            return self.err(404, "not found")
        except K.DecisionError as e:
            return self.err(e.status, str(e))
        except Exception as e:  # noqa: BLE001
            self.log_message("ERROR %r", e)
            return self.err(500, f"{type(e).__name__}: {e}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--port", type=int, default=DEFAULT_PORT)
    ap.add_argument("--graph", type=Path, default=DEFAULT_KB / "graph.json", help="graph.json to read (never written)")
    ap.add_argument("--decisions", type=Path, default=None, help="default: decisions.jsonl next to --graph")
    ap.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS, help="for sets/<set>/laughs_auto.json")
    ap.add_argument("--report", type=Path, default=None, help="default: KB_REPORT.md next to --graph")
    a = ap.parse_args()
    graph = a.graph.resolve()
    decisions = (a.decisions or graph.parent / "decisions.jsonl").resolve()
    report = (a.report or graph.parent / "KB_REPORT.md").resolve()
    Handler.kb = K.KB(graph, decisions, a.corpus, report)
    v = Handler.kb.view()
    srv = ThreadingHTTPServer(("127.0.0.1", a.port), Handler)
    srv.daemon_threads = True
    print(f"desk explorer on http://127.0.0.1:{a.port}/\n  graph     {graph} ({Handler.kb.graph_state}, "
          f"{len(v.jokes)} jokes, {len(v.edges)} edges)\n  decisions {decisions} ({len(v.decisions)} lines)", flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
