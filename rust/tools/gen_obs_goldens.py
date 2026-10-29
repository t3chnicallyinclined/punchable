"""Goldens for pk-obs: what the Python observability modules expose, written from synthetic data.

Runs a scripted sequence against the hosted API's registry (api-dev/metrics.py) and the hosted worker's
(worker/metrics.py), both on engine/prom.py, and dumps the parsed text exposition: for every metric its type, help,
label names, histogram buckets, the series (label values) the script produced, and the values that do not depend on
the clock. The scrub golden runs api-dev/scrub.py over a synthetic crash event (the shape of
api-dev/tests/test_metrics.py `event_with_everything`) plus the small rules (`route_pattern`, `status_class`,
`scrub_text`, `sensitive_key`, the worker's label helpers). The Rust tests (rust/crates/pk-obs/tests/goldens.rs)
replay the same script and compare as parsed JSON. Nothing of the comic's is here.

    python rust/tools/gen_obs_goldens.py        (from the repo root; writes rust/crates/pk-obs/tests/goldens/)
"""
from __future__ import annotations

import importlib.util
import json
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "rust" / "crates" / "pk-obs" / "tests" / "goldens"
SHA = "0123456789abcdef0123"          # PK_BUILD_SHA as the images set it; the label keeps 12
os.environ["PK_BUILD_SHA"] = SHA
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "api-dev"))
import scrub as SC  # noqa: E402
from engine import prom  # noqa: E402


def load(name: str, path: Path):
    """Both metric modules are called `metrics`; load each under its own name."""
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


API = load("pk_api_metrics", ROOT / "api-dev" / "metrics.py")
WORKER = load("pk_worker_metrics", ROOT / "worker" / "metrics.py")

_SAMPLE = re.compile(r'^([a-zA-Z_:][a-zA-Z0-9_:]*)(\{(.*)\})? (\S+)$')
_PAIR = re.compile(r'(\w+)="((?:[^"\\]|\\.)*)"')
# values that depend on the clock or a timer: the series is compared, the value is not
CLOCK = {"pk_jobs_waiting_transcriber_oldest_seconds", "pk_worker_heartbeat_age_seconds"}


def unescape(v: str) -> str:
    return v.replace("\\n", "\n").replace('\\"', '"').replace("\\\\", "\\")


def parse_exposition(text: str) -> dict:
    """{name: {type, help, labels: [sorted names], buckets: [floats], series: [[[k, v], ...]], values: {...},
    counts: {...}}}: the shape both sides are compared on."""
    out: dict[str, dict] = {}
    types: dict[str, str] = {}
    for line in text.splitlines():
        if not line.strip():
            continue
        if line.startswith("# HELP "):
            name, _, help_ = line[7:].partition(" ")
            out.setdefault(name, {"labels": set(), "buckets": set(), "series": set(), "values": {}, "counts": {}})
            out[name]["help"] = unescape(help_)
            continue
        if line.startswith("# TYPE "):
            name, _, kind = line[7:].partition(" ")
            out.setdefault(name, {"labels": set(), "buckets": set(), "series": set(), "values": {}, "counts": {}})
            out[name]["type"] = kind
            types[name] = kind
            continue
        if line.startswith("#"):
            continue
        m = _SAMPLE.match(line)
        assert m, line
        sample, _, labels, value = m.groups()
        base = sample
        suffix = ""
        for s in ("_bucket", "_sum", "_count"):
            if sample.endswith(s) and sample[: -len(s)] in types and types[sample[: -len(s)]] == "histogram":
                base, suffix = sample[: -len(s)], s
                break
        rec = out[base]
        pairs = [(k, unescape(v)) for k, v in _PAIR.findall(labels or "")]
        le = [v for k, v in pairs if k == "le"]
        pairs = sorted((k, v) for k, v in pairs if k != "le")   # the prometheus crate sorts pairs by name; so do we
        rec["labels"] |= {k for k, _ in pairs}
        key = json.dumps(pairs, separators=(",", ":"))   # the same string serde_json writes for Vec<(String, String)>
        rec["series"].add(key)
        if suffix == "_bucket":
            if le and le[0] != "+Inf":
                rec["buckets"].add(float(le[0]))
        elif suffix == "_count":
            rec["counts"][key] = int(value)
        elif suffix == "":
            if base not in CLOCK:
                rec["values"][key] = float(value)
    for rec in out.values():
        rec["labels"] = sorted(rec["labels"])
        rec["buckets"] = sorted(rec["buckets"])
        rec["series"] = sorted(json.loads(s) for s in rec["series"])
        rec["values"] = {k: rec["values"][k] for k in sorted(rec["values"])}
        rec["counts"] = {k: rec["counts"][k] for k in sorted(rec["counts"])}
    return out


# ---------------------------------------------------------------------------------------------- the api script
JOBS = [
    {"status": "queued", "kind": "riff", "waiting_for": None, "created_at": "2026-10-27T10:00:00+00:00"},
    {"status": "queued", "kind": "riff", "waiting_for": None, "created_at": "2026-10-27T10:00:00+00:00"},
    {"status": "queued", "kind": "process_set", "waiting_for": "transcriber", "created_at": "2026-10-20T10:00:00+00:00"},
    {"status": "queued", "waiting_for": "transcriber", "created_at": "not a date"},
    {"status": "claimed", "kind": "ask_chat"},
    {"status": "done", "kind": "process_set", "waiting_for": None},
    "junk",
    7,
]
API_REQUESTS = [
    # method, path, status, seconds
    ["GET", "/api/health", 200, 0.01],
    ["GET", "/api/health?ready=1", 200, 0.012],
    ["GET", "/api/joke/joke:abc123", 200, 0.02],
    ["GET", "/api/joke/joke:def456", 404, 0.004],
    ["POST", "/api/decision", 201, 0.1],
    ["HEAD", "/api/health", 503, 0.0],
    ["BREW", "/api/x", 999, 0.01],
    ["GET", "/api/worker/jobs/job:riff-0a1b2c3d4e5f/result", 200, 0.3],
    ["DELETE", "/a/b/c/d/e/f/g/h", 204, -1.0],
    ["GET", "/api/search?q=words", "200", 0.05],
]


def api_script() -> str:
    for method, path, status, seconds in API_REQUESTS:
        API.record(method, path, status, seconds)
    API.in_flight.inc()
    API.in_flight.inc()
    API.in_flight.dec()
    API.live_clients.set(2)
    API.db_duration.observe(0.003)
    API.db_duration.observe(0.3)
    API.set_jobs_source(lambda: JOBS)
    return API.registry.render()


# ------------------------------------------------------------------------------------------- the worker script
class FakeBudget:
    listeners: list = []

    def status(self):
        return {"daily_usd": 5.0, "spent_usd": 1.25, "calls": 3, "tasks": {"riff": 1.25}, "over": False}


AI_CHARGES = [
    # model, usage, usd, task
    ["claude-sonnet-4-5", {"input_tokens": 100, "output_tokens": 20, "cache_read_input_tokens": 5, "cache_creation_input_tokens": 0}, 0.0123, "riff"],
    ["Claude-Sonnet-4-5 ", {"input_tokens": 10, "output_tokens": "7", "cache_creation_input_tokens": 300}, 0.002, "ask_chat"],
    ["bad model/id", {"input_tokens": None}, -1.0, ""],
    [None, {}, 0.5, "t" * 41],
]


def worker_script() -> str:
    WORKER.heartbeat()
    t = WORKER.JobTimer("riff")
    t.done()
    t = WORKER.JobTimer("ask_chat")
    t.failed(retry=True)
    t = WORKER.JobTimer("weird kind!")
    t.failed()
    t = WORKER.JobTimer(None)
    t.done()
    for model, usage, usd, task in AI_CHARGES:
        WORKER.on_charge(model, usage, usd, task)
    WORKER.attach_budget(FakeBudget())
    return WORKER.registry.render()


# ---------------------------------------------------------------------------------------------------- scrubber
PRIVATE = {
    "text": "sample bit text about pigeons",
    "quote": "sample punchline text",
    "transcript": "sample transcript line",
    "query": "sample search words",
    "title": "Sample Title",
    "Authorization": "Bearer eyJhbGciOi.secret.sig",
    "Cookie": "session=abc",
}


def event_with_everything() -> dict:
    """api-dev/tests/test_metrics.py `event_with_everything`, as written there."""
    return {
        "event_id": "e1", "timestamp": 1.0, "platform": "python", "level": "error", "release": "punchable-api@0.1.0+abc",
        "environment": "production", "logger": "x",
        "message": "boom: '" + PRIVATE["text"] + "' " + "x" * 400,
        "request": {"method": "POST", "url": "https://example.test/api/decision?q=" + PRIVATE["query"],
                    "query_string": "q=" + PRIVATE["query"], "data": {"text": PRIVATE["text"], "after": {"title": PRIVATE["title"]}},
                    "headers": {"Authorization": PRIVATE["Authorization"], "Cookie": PRIVATE["Cookie"],
                                "Content-Type": "application/json", "X-Title": PRIVATE["title"]},
                    "cookies": {"session": "abc"}, "env": {"REMOTE_ADDR": "203.0.113.9"}},
        "exception": {"values": [{"type": "SurrealError", "value": "statement 0 failed: \"" + PRIVATE["quote"] + "\"\n"
                                  "--- SELECT * FROM unit WHERE text @@ '" + PRIVATE["query"] + "'",
                                  "stacktrace": {"frames": [
            {"filename": "server.py", "function": "do_POST", "lineno": 1, "context_line": PRIVATE["transcript"],
             "pre_context": [PRIVATE["text"]], "vars": {"b": {"text": PRIVATE["text"]}, "transcript": PRIVATE["transcript"]}}]}}]},
        "breadcrumbs": {"values": [{"category": "http", "message": PRIVATE["text"], "data": {"route": "/api/joke/{id}", "status": 500,
                                                                                            "body": PRIVATE["text"], "query": PRIVATE["query"]}}]},
        "tags": {"route": "/api/decision", "transcript": PRIVATE["transcript"]},
        "extra": {"nested": {"deep": {"quote": PRIVATE["quote"], "n": 3}}, "QUERY_STRING": PRIVATE["query"], "ok": "fine",
                  "job": {"kind": "riff", "text": PRIVATE["text"]}},
        "user": {"id": "owner", "email": "owner@example.test", "ip_address": "203.0.113.9", "username": "x"},
        "contexts": {"runtime": {"name": "CPython"}, "custom": {"text": PRIVATE["text"]}},
        "threads": {"values": [{"id": 1, "crashed": True, "stacktrace": {"frames": [{"function": "f", "vars": {"text": PRIVATE["text"]}}]}}]},
    }


def deep(n: int, leaf):
    return leaf if n == 0 else {"d": deep(n - 1, leaf)}


def event_edge_cases() -> dict:
    """The branches `event_with_everything` does not reach: message as a dict, breadcrumbs as a list, a mechanism,
    non-dict items, a user without an id, a thread without a stacktrace, depth / width / length limits, bools in
    breadcrumb data, a null exception value, falsy messages."""
    return {
        "event_id": "e2", "timestamp": 2, "type": "event", "sdk": {"name": "sentry.python", "version": "2.0"},
        "dist": "d", "server_name": "api-0", "transaction": "/api/decision",
        "message": {"formatted": "KeyError: 'the bit'\n--- SELECT 1", "message": "KeyError: %s", "params": ["the bit"]},
        "logentry": {"message": "warn \"quoted\" and 'single'", "params": []},
        "exception": {"values": [
            {"type": "ValueError", "value": None, "mechanism": {"type": "generic", "handled": False, "data": {"text": "x"}},
             "stacktrace": {"frames": [{"filename": f"f{i}.py", "lineno": i, "module": "m", "abs_path": "/x/f.py", "in_app": True, "vars": {"q": i}} for i in range(60)]}},
            "not a dict",
            {"type": 42, "value": 7.5},
            {"type": "T" * 120, "value": "", "stacktrace": "not a dict"},
        ] + [{"type": f"Over{i}", "value": "x"} for i in range(8)]},
        "breadcrumbs": [
            {"timestamp": 1.5, "type": "default", "category": "http", "level": "info", "message": "words",
             "data": {"route": "/api/x", "status_code": 500, "method": "GET", "reason_class": "Timeout", "duration_ms": 12.5,
                      "ok": True, "body": "words", "nested": {"route": "x"}}},
            {"category": "db", "data": {"statement": "SELECT 1"}},
            {"category": "none", "data": "not a dict"},
            "junk",
        ] + [{"category": f"c{i}"} for i in range(40)],
        "tags": {"kind": "riff", "retry": "False", "n": 3, "cookie": "x", "api_key": "k", "set-cookie": "y", "nested": {"text": "t", "ok": [1, 2]}},
        "extra": {
            "deep": deep(12, {"text": "x", "n": 1}),
            "wide": {f"k{i:03d}": i for i in range(70)},
            "long": "y" * 1200,
            "list": list(range(80)) + ["z" * 1001],
            "joke_text": "x", "HTTP_AUTHORIZATION": "x", "x-title": "x", "search_query": "x", "ip-address": "x",
            "route": "/api/x", "lineno": 3, "function": "f", "none": None, "bool": True, "float": 1.5,
        },
        "user": {"email": "owner@example.test"},
        "contexts": {"runtime": {"name": "CPython", "text": "x"}, "os": {"name": "Linux"}, "device": "not a dict",
                     "app": {"app_name": "api"}, "trace": {"trace_id": "t"}, "custom": {"ok": 1}},
        "threads": {"values": [{"id": 1, "crashed": False}, {"crashed": True, "stacktrace": {"frames": "not a list"}},
                               "junk", {"id": "main", "stacktrace": {"frames": [{"function": "g", "context_line": "x"}, "junk"]}}]},
        "modules": {"x": "1"}, "culprit": "c", "fingerprint": ["x"], "stacktrace": {"frames": []},
    }


SCRUB_TEXT_CASES = [
    "KeyError: 'the bit'\n--- SELECT 1",
    "statement 0 failed: \"quoted words\"",
    "a 'b' c \"d\" e 'unterminated",
    "",
    None,
    0,
    False,
    12.5,
    "x" * 400,
    "'" + "y" * 500 + "' tail",
    "ünïcödé 'wörds' and emoji \U0001F600 " + "z" * 300,
    "line one 'a'\nline two 'b'",
]
SENSITIVE_KEY_CASES = [
    "text", "TEXT", "Transcript", "set-cookie", "set_cookie", "joke_text", "search_query", "x-title", "HTTP_AUTHORIZATION",
    "route", "status", "method", "lineno", "release", "function", "ip-address", "ip_address", "api-key", "apikey", "key",
    "keys", "context", "textual", "q", "Q", " name ", "user", "users", "sub", "subject", "authentic", "token_count", "", "-", "_",
    "original-name", "original_name", "set-cookie-x",
]
ROUTE_CASES = [
    "/api/health", "/api/health?ready=1", "/api/joke/x:ab12cd", "/api/joke/Something%20Private", "/api/sets/20261027_up0123abcde",
    "/api/worker/jobs/job:riff-0a1b2c3d4e5f/result", "/api/search?q=my%20secret%20bit", "/api/audio/ABC",
    "/api/memos/2026-10-27_deadbeef/transcript", "/", "", "/a/b/c/d/e/f/g/h", "/a/b/c/d/e/f", "/a/b/c/d/e/f/g",
    "/api/x1", "/api/X", "/api/a.b", "/api/a:b", "/api/" + "zq" * 16 + "z", "/api/" + "zq" * 16, "/api/%20", "/api/Bit",
    "//api//health//", "/api/health#frag", "/api/health?q=1#frag", "/api/a-b_c", "/api/-x", "/api/_x", "/api/ünï", "api/health",
    "/api/health/", "?q=1", "/api/joke/x:ab12cd/edit",
]
STATUS_CASES = [200, "200", 201, 404, 503, 99, 100, 599, 600, 0, -1, "abc", None, "", "  404 ", 200.0, 200.9, True, "0x1F"]
KIND_CASES = ["riff", "ask_chat", "process_set", "weird kind!", None, "", "zq" * 16, "zq" * 16 + "z", "Riff", "ri-ff", "r1ff", "_", "ünï"]
MODEL_CASES = ["claude-sonnet-4-5", "Claude-Sonnet-4-5 ", "bad model/id", None, "", "zq" * 32, "zq" * 32 + "z", "m:1.0_x", "ünï", "m+1"]
TASK_CASES = ["riff", "", None, "t" * 40, "t" * 41, "ask chat"]
USAGE_CASES = [
    {"input_tokens": 100, "output_tokens": 20, "cache_read_input_tokens": 5, "cache_creation_input_tokens": 0},
    {"input_tokens": "7", "output_tokens": None, "cache_read_input_tokens": "x", "cache_creation_input_tokens": 2.9},
    {},
]


def scrub_golden() -> dict:
    ev1, ev2 = event_with_everything(), event_edge_cases()
    return {
        "private": PRIVATE,
        "redacted": SC.REDACTED,
        "max_message": SC.MAX_MESSAGE,
        "sensitive_keys": sorted(SC.SENSITIVE_KEYS),
        "header_allow": sorted(SC.HEADER_ALLOW),
        "key_hint": SC._KEY_HINT.pattern,
        "events": [
            {"name": "everything", "event": ev1, "scrubbed": SC.scrub_event(ev1, {}), "scrub_value": SC.scrub_value(ev1),
             "flat_keys": sorted(SC.flatten_keys(SC.scrub_event(ev1, {})))},
            {"name": "edge_cases", "event": ev2, "scrubbed": SC.scrub_event(ev2, None), "scrub_value": SC.scrub_value(ev2),
             "flat_keys": sorted(SC.flatten_keys(SC.scrub_event(ev2, None)))},
        ],
        "scrub_event_passthrough": [[x, SC.scrub_event(x, None)] for x in ("not a dict", 3, None, [1, 2], {})],
        "scrub_text": [[s, SC.scrub_text(s)] for s in SCRUB_TEXT_CASES],
        "sensitive_key": [[k, SC.sensitive_key(k)] for k in SENSITIVE_KEY_CASES],
        "scrub_request": [[r, SC.scrub_request(r)] for r in (
            {"method": "GET", "url": "https://x/api?q=1", "headers": {"Content-Type": "a", "content-length": "1", "X-Title": "t",
                                                                      "USER-AGENT": "ua", "Host": "h", "x-request-id": "r", "Accept": "*/*"}},
            {"method": 1, "url": None, "headers": "not a dict", "data": "x"}, "not a dict", {})],
        "scrub_breadcrumb": [[b, SC.scrub_breadcrumb(b)] for b in (
            {"timestamp": 1, "type": "http", "category": "c", "level": "info", "message": "m", "data": {"route": "/x", "status": 200}},
            {"data": {"route": ["not", "scalar"], "status": None, "method": True, "duration_ms": 1.5}}, {"data": {}}, {}, "junk", None)],
    }


def labels_golden() -> dict:
    return {
        "route_pattern": [[p, API.route_pattern(p)] for p in ROUTE_CASES],
        "status_class": [[c, API.status_class(c)] for c in STATUS_CASES],
        "methods": list(API.METHODS),
        "kind_label": [[k, WORKER.kind_label(k)] for k in KIND_CASES],
        "model_label": [[m, WORKER.model_label(m)] for m in MODEL_CASES],
        "job_stats": [[JOBS, {k: v for k, v in API.job_stats(JOBS).items() if k != "oldest_waiting_s"}],
                      [[], API.job_stats([])], [None, API.job_stats(None)]],
        "job_buckets": list(WORKER.JOB_BUCKETS),
        "default_buckets": list(prom.DEFAULT_BUCKETS),
        "max_series": prom.MAX_SERIES,
        "overflow": prom.OVERFLOW,
        "content_type": prom.CONTENT_TYPE,
    }


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    api_text = api_script()
    worker_text = worker_script()
    exposition = {
        "sha": SHA, "version": API.VERSION, "worker_version": WORKER.VERSION,
        "api": {"requests": API_REQUESTS, "jobs": JOBS, "metrics": parse_exposition(api_text)},
        "worker": {"charges": AI_CHARGES, "budget": FakeBudget().status(), "metrics": parse_exposition(worker_text)},
    }
    (OUT / "exposition.json").write_text(json.dumps(exposition, ensure_ascii=False, indent=1), encoding="utf-8")
    (OUT / "scrub.json").write_text(json.dumps(scrub_golden(), ensure_ascii=False, indent=1), encoding="utf-8")
    (OUT / "labels.json").write_text(json.dumps(labels_golden(), ensure_ascii=False, indent=1), encoding="utf-8")
    print("wrote", OUT, "api metrics", len(exposition["api"]["metrics"]), "worker metrics", len(exposition["worker"]["metrics"]))


if __name__ == "__main__":
    main()
