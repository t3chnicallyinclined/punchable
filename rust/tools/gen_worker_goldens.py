"""Goldens for pk-worker: what the Python hosted worker (`python -m worker --hosted --once`) puts on the wire for a
scripted job, recorded from the real process, so the Rust worker can be run against the same fixtures and compared
as parsed JSON (ADR 0007). Synthetic material only: nothing of the comic's is here.

Each scenario starts three local servers in this process and runs the Python worker as a child process against them:

  the API        the `/api/worker/*` routes the worker calls (claim, profile/voice, ideas, files, result, and the
                 workspace-mirror reads it makes on every job); every request is recorded with its parsed body
  Anthropic      `POST /v1/messages` answering the scenario's canned answers as an SSE stream, in order (the pinned
                 SDK reads ANTHROPIC_BASE_URL); every request body is recorded
  pages          the fact-check pages: `creative.write.fetch_text` is pointed at this server by a two-line bootstrap
                 (https://host/path -> http://127.0.0.1:<port>/host/path); nothing reaches the network

The child gets a clean environment: no secrets file, no legal rules file (the generic nets only, as on the server),
a temporary home, its own spend ledger. What is written per scenario (crates/pk-worker/tests/goldens/<name>.json):
the scenario (job, voice profile, model answers, pages, budget) and what the Python did (exit code, claim bodies,
the effectful API requests in order, the Anthropic request bodies). Times are normalised (`run_id`); the workspace
mirror requests (`GET files`, `files/content`, `decisions`, `creative`) are kept in `api` with their query strings:
the Rust worker makes the same ones for the kinds that read the notebook, and none for the others
(docs/RUST_WORKSTREAM.md T8). The notebook is rust/tools/notebook_fixture.py's.

Two smaller goldens come from the same modules in-process: `voice.json` (creative/voice.py's blocks and messages
per task, named and unnamed) and `pages.json` (creative/write.py's page text and `supported`).

    python rust/tools/gen_worker_goldens.py        (from the repo root; needs the worker's SDK pins, as gen_llm_goldens)
"""
from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
import notebook_fixture as F  # noqa: E402
OUT = ROOT / "rust" / "crates" / "pk-worker" / "tests" / "goldens"
KEY = "sk-golden-not-a-key"
WORKER_NAME = "golden-worker"
RUN_ID = re.compile(r"settext-\d{8}-\d{6}-")
# the workspace-mirror reads the Python makes on every job (worker/online.py Workspace.refresh); the Rust worker reads
# only what a kind needs, so these are compared by presence in the Python run and absent in the Rust run
MIRROR = ("/api/worker/files", "/api/worker/decisions", "/api/worker/creative")

# ------------------------------------------------------------------ synthetic material
PROFILE = ("VOICE PROFILE (synthetic). Deadpan; short sentences; the turn lands on the last word. Recurring premises: "
           "airports, a parrot that only says legal disclaimers, his landlord's group chat. Moves he names: the flat "
           "repeat, the wrong-size callback, the list of three where the third is a confession. " * 3).strip()
LEARNED = {"kept": [{"text": "The bin gets my belt, my shoes and most of my confidence.", "bit": "joke:airport-bins",
                     "date": "2026-10-20"},
                    {"text": "My parrot read me my rights.", "bit": None, "date": "2026-10-18", "edited": True}],
           "dropped": [{"text": "Airports, am I right?", "bit": "joke:airport-bins", "date": "2026-10-21"}],
           "updated_at": "2026-10-21T10:00:00Z"}
LAUGHS = [{"text": "The parrot settled out of court.", "bit": "joke:parrot", "date": "2026-09-30", "laugh_s": 2.5},
          {"text": "TSA patted down my personality.", "bit": "joke:airport-bins", "date": None, "laugh_s": 3.4999}]


def voice_doc(text=PROFILE, learned=None, laughs=None, updated_at="2026-10-19T08:00:00Z"):
    learned = learned if learned is not None else {"kept": [], "dropped": [], "updated_at": None}
    return {"kind": "voice", "text": text, "updated_at": updated_at if text else None, "by": "owner" if text else None,
            "by_kind": "owner" if text else None, "chars": len(text or ""), "learned": learned,
            "laugh_lines": laughs or []}


ASK_MESSAGES = [
    {"role": "system", "content": "You answer a stand-up comic's questions about his OWN material (synthetic system)."},
    {"role": "user", "content": "SOURCES\n[S1] (bit, Oct 2026) The bin gets my belt and my shoes.\n\nQUESTION: did I "
                                "ever do airport bins?"},
]
ANSWER = json.dumps({"found": True, "answer": "Yes: the bins bit [S1].",
                     "citations": [{"source": "S1", "quote": "The bin gets my belt"}]})

SEGS = ["Give it up for our next comic, he flew in today!", "So airport security wants my belt in a bin.",
        "The bin gets my belt, my shoes and most of my confidence.", "Anyway, quick sip of water.",
        "I bought a parrot that only says legal disclaimers.", "Past results are no guarantee of future crackers."]
SPLIT = json.dumps({"units": [
    {"first": 0, "last": 0, "kind": "intro", "name": "Host intro", "funny": False},
    {"first": 1, "last": 2, "kind": "bit", "name": "  Belt in a bin  ", "funny": True},
    {"first": 3, "last": 3, "kind": "banter", "name": "Water break", "funny": False},
    {"first": 4, "last": 5, "kind": "bit", "name": "Disclaimer parrot", "funny": True}]})

FACTS = json.dumps({"facts": [
    {"fact": "Airport security bins were introduced for laptops and liquids screening.",
     "source_title": "Bins, a history", "source_url": "https://facts.example.org/bins"},
    {"fact": "Parrots can learn hundreds of words.", "source_title": "", "source_url": "http://facts.example.org/parrots"},
    {"fact": "Parrots can mimic human speech with surprising accuracy.", "source_title": "  Parrot speech  ",
     "source_url": "https://facts.example.org/parrot-speech"},
    {"fact": "Belts set off metal detectors because of their buckles.", "source_title": "Buckles",
     "source_url": "https://facts.example.org/missing"},
    {"fact": "Screening lanes average many passengers each hour.", "source_title": "Lanes",
     "source_url": "https://facts.example.org/lanes"},
    {"fact": "Security bins are washed about once a decade.", "source_title": "Bins washed",
     "source_url": "https://facts.example.org/unrelated"},
    {"fact": "Parrot owners report daily disclaimers.", "source_title": "Never fetched",
     "source_url": "https://facts.example.org/never"}]})
PAGES = {
    "https://facts.example.org/bins": "<html><head><style>p{color:red}</style><script>var bins=1;</script></head>"
                                      "<body><p>Airport security <b>bins</b> were introduced for laptops &amp; liquids "
                                      "screening.</p></body></html>",
    "https://facts.example.org/parrot-speech": "<p>Parrots CAN mimic human speech with surprising accuracy &mdash; "
                                               "some &#8220;talk&#8221; daily.</p>",
    "https://facts.example.org/lanes": "<div>Screening lanes process passengers; the average is many each hour."
                                       "</div>",
    "https://facts.example.org/unrelated": "<p>Nothing about it here.</p>",
    "https://facts.example.org/never": "<p>Parrot owners report daily disclaimers.</p>",
}
LEGAL_SEGS = ["Thanks folks.", "My lawyer says the petitioner and the respondent should both try decaf.",
              "That one is for the court reporters."]


def sse_events(text: str, *, model: str, input_tokens: int = 1200, output_tokens: int = 90, cache_read: int = 0,
               cache_write: int = 0, stop_reason: str = "end_turn", stop_details: dict | None = None) -> list[dict]:
    usage0 = {"input_tokens": input_tokens, "output_tokens": 1, "cache_creation_input_tokens": cache_write,
              "cache_read_input_tokens": cache_read}
    ev = [{"type": "message_start", "message": {"id": "msg_golden", "type": "message", "role": "assistant",
                                                 "model": model, "content": [], "stop_reason": None,
                                                 "stop_sequence": None, "usage": usage0}}]
    if text:
        a, b = text[: len(text) // 2], text[len(text) // 2:]
        ev += [{"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}},
               {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": a}},
               {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": b}},
               {"type": "content_block_stop", "index": 0}]
    delta = {"stop_reason": stop_reason, "stop_sequence": None}
    if stop_details:
        delta["stop_details"] = stop_details
    ev += [{"type": "message_delta", "delta": delta, "usage": {"output_tokens": output_tokens}},
           {"type": "message_stop"}]
    return ev


def answer(text: str, model: str, **kw) -> dict:
    """One canned answer as the scenario stores it: the SSE events are rebuilt from it on both sides."""
    return {"text": text, "model": model, **kw}


def job(kind: str, inp, n: int = 1) -> dict:
    return {"id": f"job:{kind}-golden{n:06d}", "kind": kind, "input": inp, "key": None, "status": "claimed",
            "created_at": "2026-10-29T12:00:00Z", "by": "owner", "attempts": 1, "claimed_at": "2026-10-29T12:00:01Z",
            "worker": WORKER_NAME, "done_at": None, "result": None, "error": None, "waiting_for": None}


def set_text_input(segs, *, timed=False, title="Open mic", recorded_at="2026-10-20T21:00:00-04:00", venue="a room",
                   duration_s=None, engine="speech"):
    out = []
    t = 0.0
    for s in segs:
        if timed:
            out.append({"text": s, "start": t, "end": t + 2.25})
            t += 2.5
        else:
            out.append({"text": s})
    return {"segments": out, "title": title, "recorded_at": recorded_at, "venue": venue, "duration_s": duration_s,
            "received_at": "2026-10-20T21:30:00Z", "engine": engine}


OPUS, SONNET = "claude-opus-5-5", "claude-sonnet-5-5"
RIFF = json.dumps({"ideas": [
    {"line": " The bin took my belt; TSA kept the receipt. ", "built_on": ["B1", "riff", "B9"], "move": "the flat repeat"},
    {"line": "   ", "built_on": ["B1"], "move": "x"},
    {"line": "My parrot billed me by the hour.", "built_on": [], "move": ""},
    {"line": "x" * 450, "built_on": ["riff"], "move": "m" * 100},
    {"line": "Over the cap.", "built_on": [], "move": "none"}]})
NAMED = {"PK_COMIC_NAME": "Pat Example", "PK_COMIC_FIRST": "Pat"}

SCENARIOS = [
    {"name": "ask_chat_named", "env": NAMED, "job": job("ask_chat", {"messages": ASK_MESSAGES}),
     "voice": voice_doc(learned=LEARNED, laughs=LAUGHS), "model": [answer(ANSWER, SONNET, cache_write=900)]},
    {"name": "ask_chat_no_profile", "env": {}, "job": job("ask_chat", {"messages": ASK_MESSAGES}),
     "voice": voice_doc(text=None), "model": [answer(ANSWER, SONNET)]},
    {"name": "ask_chat_bad_input", "env": {}, "job": job("ask_chat", {"messages": []}),
     "voice": voice_doc(), "model": []},
    {"name": "ask_chat_invalid_twice", "env": {}, "job": job("ask_chat", {"messages": ASK_MESSAGES}),
     "voice": voice_doc(), "model": [answer("not json at all", SONNET), answer('{"found": "yes"}', SONNET)]},
    {"name": "ask_chat_refusal", "env": {}, "job": job("ask_chat", {"messages": ASK_MESSAGES}),
     "voice": voice_doc(), "model": [answer("", SONNET, stop_reason="refusal",
                                            stop_details={"type": "refusal", "category": "cyber",
                                                          "explanation": "Not this one."})]},
    {"name": "facts_checked", "env": {}, "job": job("facts", {"text": "  airport security bins and parrots  "}),
     "voice": voice_doc(), "model": [answer(FACTS, SONNET)], "pages": PAGES},
    {"name": "facts_empty", "env": {}, "job": job("facts", {"text": "   "}), "voice": voice_doc(), "model": []},
    {"name": "facts_refused", "env": {}, "job": job("facts", {"text": "parrots"}), "voice": voice_doc(),
     "model": [answer("", SONNET, stop_reason="refusal")]},
    {"name": "set_text_synthetic", "env": {}, "job": job("set_text", set_text_input(SEGS)),
     "voice": voice_doc(learned=LEARNED), "model": [answer(SPLIT, OPUS)]},
    {"name": "set_text_device_named", "env": NAMED,
     "job": job("set_text", set_text_input(SEGS, timed=True, title="", venue=None, duration_s=0, engine="")),
     "voice": voice_doc(), "model": [answer(SPLIT, OPUS)]},
    {"name": "set_text_legal", "env": {}, "job": job("set_text", set_text_input(LEGAL_SEGS)), "voice": voice_doc(),
     "model": [answer(json.dumps({"units": [{"first": 0, "last": 2, "kind": "bit", "name": "Decaf", "funny": True}]}),
                      OPUS)]},
    {"name": "set_text_budget_mid_job", "env": {}, "daily_usd": 1.0,
     "job": job("set_text", set_text_input([f"Line {i} about luggage and the carousel." for i in range(230)])),
     "voice": voice_doc(),
     "model": [answer(json.dumps({"units": [{"first": 0, "last": 219, "kind": "bit", "name": "Luggage",
                                             "funny": True}]}), OPUS, input_tokens=1_000_000)]},
    {"name": "riff_named_learned", "env": NAMED, "notebook": True,
     "job": job("riff", {"idea": "  Airport bins again ", "lines": ["the bin gets everything", "  ", "my belt"]}),
     "voice": voice_doc(learned=LEARNED, laughs=LAUGHS), "model": [answer(RIFF, OPUS, cache_write=1400)]},
    {"name": "riff_unnamed_profile_only", "env": {}, "notebook": True,
     "job": job("riff", {"idea": "parrot lawyer", "lines": []}), "voice": voice_doc(),
     "model": [answer(RIFF, OPUS, cache_read=1400)]},
    {"name": "riff_no_grounding", "env": {}, "notebook": True,
     "job": job("riff", {"idea": "zebra quantum", "lines": []}), "voice": voice_doc(), "model": []},
    {"name": "riff_refused_then_fallback", "env": {}, "notebook": True,
     "job": job("riff", {"idea": "landlord rent", "lines": ["my landlord"]}), "voice": voice_doc(learned=LEARNED),
     "model": [answer("", OPUS, stop_reason="refusal"), answer("", SONNET, stop_reason="refusal")]},
    {"name": "result_refused", "env": {}, "job": job("ask_chat", {"messages": ASK_MESSAGES}), "voice": voice_doc(),
     "model": [answer(ANSWER, SONNET)], "refuse_ok_result": True},
    {"name": "budget_paused", "env": {}, "daily_usd": 1.0, "spent_usd": 1.2,
     "job": job("riff", {"idea": "x", "lines": []}), "voice": voice_doc(), "model": []},
    {"name": "transcriber_handed_back", "env": {}, "job": job("process_set", {"file": "x.m4a", "meta": {}}),
     "voice": voice_doc(), "model": []},
]


# ------------------------------------------------------------------ the servers
class Recorder:
    def __init__(self, scenario: dict):
        self.sc = scenario
        self.api: list[dict] = []
        self.anthropic: list[dict] = []
        self.handed = False
        self.ideas = 0
        self.answers = list(scenario.get("model") or [])
        self.lock = threading.Lock()


def make_handler(rec: Recorder, which: str):
    class H(BaseHTTPRequestHandler):
        def log_message(self, *a):
            return

        def _body(self):
            n = int(self.headers.get("Content-Length") or 0)
            raw = self.rfile.read(n) if n else b""
            return json.loads(raw) if raw else None

        def _send(self, status: int, body=None, ctype="application/json", raw: bytes | None = None):
            data = raw if raw is not None else (b"" if body is None else json.dumps(body).encode("utf-8"))
            self.send_response(status)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            self._route("GET", None)

        def do_POST(self):
            self._route("POST", self._body())

        def _route(self, method: str, body):
            u = urllib.parse.urlsplit(self.path)
            if which == "pages":
                url = "https://" + u.path.lstrip("/")
                page = (rec.sc.get("pages") or {}).get(url)
                if page is None:
                    return self._send(404, {"error": "no such page"})
                return self._send(200, ctype="text/html; charset=utf-8", raw=page.encode("utf-8"))
            if which == "anthropic":
                with rec.lock:
                    rec.anthropic.append(body)
                    a = rec.answers.pop(0) if rec.answers else answer("{}", SONNET)
                ev = sse_events(a["text"], **{k: v for k, v in a.items() if k != "text"})
                text = "".join(f"event: {e['type']}\ndata: {json.dumps(e)}\n\n" for e in ev)
                return self._send(200, ctype="text/event-stream", raw=text.encode("utf-8"))
            with rec.lock:
                rec.api.append({"method": method, "path": u.path, "query": u.query, "body": body})
            p = u.path
            if p == "/api/worker/claim":
                if not rec.handed and not (body or {}).get("paused"):
                    rec.handed = True
                    return self._send(200, {"job": rec.sc["job"]})
                return self._send(204)
            if p == "/api/worker/profile/voice":
                return self._send(200, rec.sc["voice"])
            nb = notebook_files(rec.sc)
            if p == "/api/worker/files" and method == "GET":
                return self._send(200, {"files": [{"path": k, "sha256": hashlib.sha256(v.encode()).hexdigest(),
                                                   "bytes": len(v.encode())} for k, v in sorted(nb.items())],
                                        "files_rev": 3 if nb else None, "mode": "db"})
            if p == "/api/worker/files/content":
                rel = urllib.parse.parse_qs(u.query).get("path", [""])[0]
                if rel not in nb:
                    return self._send(404, {"error": "no such file"})
                return self._send(200, {"path": rel, "sha256": hashlib.sha256(nb[rel].encode()).hexdigest(),
                                        "text": nb[rel]})
            if p == "/api/worker/decisions":
                ds = notebook_decisions(rec.sc)
                after = int(urllib.parse.parse_qs(u.query).get("after", ["0"])[0])
                return self._send(200, {"lines": [{"decision": d, "index": after + i} for i, d in enumerate(ds[after:])],
                                        "total": len(ds)})
            if p == "/api/worker/creative":
                return self._send(200, {"lines": []})
            if p == "/api/worker/ideas":
                rec.ideas += 1
                return self._send(200, ideas_reply(rec.ideas))
            if p == "/api/worker/files" and method == "POST":
                return self._send(200, files_reply(body))
            if p.startswith("/api/worker/jobs/") and p.endswith("/result"):
                if rec.sc.get("refuse_ok_result") and (body or {}).get("ok"):
                    return self._send(413, raw=REFUSED_RESULT)
                return self._send(200, {"ok": True, "job": {"id": rec.sc["job"]["id"], "kind": rec.sc["job"]["kind"],
                                                            "status": "done", "created_at": None, "done_at": None,
                                                            "attempts": 1, "error": None}})
            return self._send(404, {"error": f"no route {method} {p}"})

    return H


# the API's answer to a result it will not take (`refuse_ok_result`: the success post is refused, the failure taken)
REFUSED_RESULT = b'{"error": "body too large"}'


def notebook_files(sc: dict) -> dict:
    return F.files() if sc.get("notebook") else {}


def notebook_decisions(sc: dict) -> list:
    return F.decisions() if sc.get("notebook") else []


def ideas_reply(n: int) -> dict:
    """The API's answer to the n-th idea post (the Rust test answers the same)."""
    return {"ok": True, "indexes": [100 + n], "id": f"joke:golden-idea-{n}", "decisions": [],
            "attach": {"decision": "new" if n % 2 else "attached", "into": None}}


def files_reply(body: dict) -> dict:
    files = (body or {}).get("files") or []
    return {"ok": True, "written": [f["path"] for f in files], "removed": [], "unchanged": 0, "synced": None,
            "files_rev": 7, "actor": (body or {}).get("actor")}


def serve(rec: Recorder, which: str) -> ThreadingHTTPServer:
    srv = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(rec, which))
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


BOOT = """import sys
sys.path.insert(0, {root!r})
import creative.write as W
_orig = W.fetch_text
W.fetch_text = lambda url, timeout=8.0: _orig(url.replace("https://", {pages!r} + "/", 1), timeout)
import worker.__main__ as M
sys.exit(M.main(["--hosted", "--once", "--no-auth", "--api", {api!r}]))
"""


def run_python(sc: dict) -> dict:
    rec = Recorder(sc)
    api, anth, pages = serve(rec, "api"), serve(rec, "anthropic"), serve(rec, "pages")
    base = lambda s: f"http://127.0.0.1:{s.server_address[1]}"  # noqa: E731
    with tempfile.TemporaryDirectory(prefix="pk-worker-golden-") as td:
        tmp = Path(td)
        spend = tmp / "ai_spend.json"
        if sc.get("spent_usd"):
            today = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d")
            spend.write_text(json.dumps({today: {"usd": sc["spent_usd"], "calls": 1,
                                                 "tasks": {"creative_riff": sc["spent_usd"]}}}), encoding="utf-8")
        env = {k: os.environ[k] for k in ("PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "TEMP", "TMP", "APPDATA")
               if k in os.environ}
        env.update({
            "HOME": str(tmp), "USERPROFILE": str(tmp), "PYTHONUTF8": "1", "PYTHONHASHSEED": "0",
            "PUNCH_SECRETS": str(tmp / "no-secrets.env"), "PUNCHABLE_HOME": str(tmp),
            "PUNCHABLE_LEGAL_RULES": str(tmp / "no-legal-rules.json"),
            "ANTHROPIC_API_KEY": KEY, "ANTHROPIC_BASE_URL": base(anth),
            "ENGINE_CONFIG": str(ROOT / "engine" / "config.hosted.toml"),
            "PK_WORKSPACE": str(tmp / "ws"), "PK_CACHE_DIR": str(tmp / "cache"), "PK_BGE_DIR": str(tmp / "no-bge"),
            "PK_HEARTBEAT_FILE": str(tmp / "heartbeat"), "PK_AI_SPEND_FILE": str(spend),
            "PK_AI_DAILY_USD": str(sc.get("daily_usd", 5.0)), "PK_METRICS_PORT": "0", "PK_WORKER_NAME": WORKER_NAME,
        })
        env.update(sc.get("env") or {})
        boot = BOOT.format(root=str(ROOT), pages=base(pages), api=base(api))
        p = subprocess.run([sys.executable, "-c", boot], cwd=str(tmp), env=env, capture_output=True, text=True,
                           encoding="utf-8", errors="replace", timeout=300)
    for s in (api, anth, pages):
        s.shutdown()
    if p.returncode not in (0, 1):
        raise SystemExit(f"{sc['name']}: the Python worker exited {p.returncode}\n{p.stdout}\n{p.stderr}")
    claims = [r["body"] for r in rec.api if r["path"] == "/api/worker/claim"]
    effect = [r for r in rec.api if r["path"] != "/api/worker/claim"]
    return {"exit": p.returncode, "claims": claims,
            "api": [normalise({"method": r["method"], "path": r["path"], "query": r["query"], "body": r["body"]})
                    for r in effect],
            "anthropic": rec.anthropic}


def normalise(x):
    """Times out of a recorded request: the set-analysis run id (and the file text it is written into, whose sha256
    is recomputed over the normalised text so the Rust side can compare its own bytes)."""
    if isinstance(x, dict):
        out = {k: normalise(v) for k, v in x.items()}
        if isinstance(x.get("text"), str) and isinstance(x.get("sha256"), str):
            # the Python hashes the file's bytes; analyze_transcript writes with write_text, which is CRLF on
            # Windows, so only on POSIX (the server, CI) are the bytes the text: there the posted sha is proven equal
            if os.name != "nt" and not RUN_ID.search(x["text"]):
                assert x["sha256"] == hashlib.sha256(x["text"].encode("utf-8")).hexdigest(), x["path"]
            out["sha256"] = hashlib.sha256(out["text"].encode("utf-8")).hexdigest()
        return out
    if isinstance(x, list):
        return [normalise(v) for v in x]
    if isinstance(x, str):
        return RUN_ID.sub("settext-<time>-", x)
    return x


# ------------------------------------------------------------------ the in-process goldens
def voice_golden() -> dict:
    sys.path.insert(0, str(ROOT))
    from creative import voice as V

    class Cl:
        def __init__(self, d):
            self.d = d

        def call(self, method, path, body=None):
            assert (method, path) == ("GET", "/api/worker/profile/voice")
            return self.d

    cases = []
    for env in ({}, NAMED, {"PK_COMIC_NAME": "  Pat Example  "}):
        for name, doc in (("profile_learned_laughs", voice_doc(learned=LEARNED, laughs=LAUGHS)),
                          ("profile_only", voice_doc()), ("learned_only", voice_doc(text=None, learned=LEARNED)),
                          ("nothing", voice_doc(text=None)), ("blank_profile", voice_doc(text="  \n ")),
                          ("short_profile", voice_doc(text="Deadpan.", learned=LEARNED))):
            for k in ("PK_COMIC_NAME", "PK_COMIC_FIRST"):
                os.environ.pop(k, None)
            os.environ.update(env)
            V.clear()
            V.fetch(Cl(doc))
            per_task = {}
            for task in ("creative_riff", "creative_punchup", "creative_bridge", "ask_chat"):
                per_task[task] = {"system_block": V.system_block(task), "learned_block": V.learned_block(task),
                                  "cacheable_learned": V.cacheable_learned(task),
                                  "messages": V.messages(task, "TASK LINE", {"role": "user", "content": "USER TURN"})}
            st = V.status()
            cases.append({"env": env, "doc": name, "voice": doc, "tasks": per_task,
                          "status": {k: st[k] for k in ("loaded", "updated_at", "sha", "chars")},
                          "names": {"whose_idea": V.whose_idea(), "he_could_say": V.he_could_say(),
                                    "material_owner": V.material_owner()}})
    for k in ("PK_COMIC_NAME", "PK_COMIC_FIRST"):
        os.environ.pop(k, None)
    V.clear()
    return {"cases": cases}


HTML = [
    "<p>Plain words.</p>",
    "<html><head><STYLE type='text/css'>p{}</STYLE><Script>alert('x')</Script></head><body>Visible</body></html>",
    "<script>a</style>b</script>tail",
    "<style>unterminated <p>text</p>",
    "Fish &amp; chips &lt;b&gt; &quot;quoted&quot; &#39;single&#39; &#x41;&#66; caf&eacute; &mdash; &nbsp;end",
    "<a href='x'>link</a> and <br/> line<br>break",
    "UPPER Case ÄÖÜ İstanbul ΣΑΣ",
    "<scripts>not a script tag</scripts> kept?",
    "a < b > c",
    "<p>Parrots &copy; 2026</p>",
    "Fish &amp chips &copy 2026 &notit",
]
# Python's html.unescape also decodes the HTML5 legacy entities written without ";" (`&amp`, `&copy`, `&not`); the
# html-escape crate the Rust uses decodes only `;`-terminated ones. Listed so the Rust test pins the difference (the
# page text only feeds a word-presence check on words of 4+ letters, so a fact's support does not move on it).
LEGACY_ENTITIES = {"Fish &amp chips &copy 2026 &notit"}
SUPPORTED = [
    ("Airport security bins were introduced for laptops.", "airport security bins were introduced for laptops."),
    ("Airport security bins were introduced for laptops.", "airport only"),
    ("Cats nap.", "cats nap a lot"),
    ("Parrots mimic speech, parrots mimic speech.", "parrots mimic"),
    ("The parrot's owners' disclaimers", "parrot owner disclaimer"),
    ("Screening lanes average many passengers each hour.", "screening lanes process passengers; the average is many"),
    ("", "anything"),
    ("Words words words", ""),
]


def pages_golden() -> dict:
    sys.path.insert(0, str(ROOT))
    from creative import write as W
    sc = {"pages": {f"https://golden.example.org/{i}": h for i, h in enumerate(HTML)}}
    rec = Recorder(sc)
    srv = serve(rec, "pages")
    base = f"http://127.0.0.1:{srv.server_address[1]}"
    try:
        texts = [{"html": h, "text": W.fetch_text(f"{base}/golden.example.org/{i}"), "legacy_entities": h in LEGACY_ENTITIES}
                 for i, h in enumerate(HTML)]
        missing = W.fetch_text(f"{base}/golden.example.org/missing")
    finally:
        srv.shutdown()
    return {"texts": texts, "missing": missing,
            "supported": [{"fact": f, "page": p, "ok": W.supported(f, p or None)} for f, p in SUPPORTED],
            "url_ok": [{"url": u, "ok": bool(W.URL_RE.match(u))} for u in (
                "https://facts.example.org/bins", "http://facts.example.org/x", "https://localhost/x",
                "https://a.b.co", "https://a.b.c0/x", "https://x.example.org/with space", "https://x.example.org/ü",
                "https://ex-ample.org/path?q=1#f", "https://.org", "ftp://x.example.org")]}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for sc in SCENARIOS:
        got = run_python(sc)
        doc = {k: v for k, v in sc.items()}
        if sc.get("notebook"):   # the Rust test serves the same notebook
            doc["notebook_files"], doc["notebook_decisions"] = notebook_files(sc), notebook_decisions(sc)
        doc["expect"] = got
        (OUT / f"scenario_{sc['name']}.json").write_text(json.dumps(doc, indent=1, ensure_ascii=False) + "\n",
                                                        encoding="utf-8", newline="\n")
        print(f"{sc['name']}: exit {got['exit']}, {len(got['claims'])} claim(s), {len(got['api'])} request(s), "
              f"{len(got['anthropic'])} model call(s)")
    (OUT / "voice.json").write_text(json.dumps(voice_golden(), indent=1, ensure_ascii=False) + "\n", encoding="utf-8",
                                    newline="\n")
    (OUT / "pages.json").write_text(json.dumps(pages_golden(), indent=1, ensure_ascii=False) + "\n", encoding="utf-8",
                                    newline="\n")
    print(f"goldens in {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
