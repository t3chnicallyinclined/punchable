"""GET /metrics and the crash-report scrubber (api-dev/metrics.py, api-dev/scrub.py, engine/prom.py).

No data, no SurrealDB, no network beyond loopback: the API handler is started with an empty `kbs` (the hosted
"booting" state), which is exactly when `up` and pk_build_info must already answer.

    python -m pytest api-dev/tests/test_metrics.py -q
"""
from __future__ import annotations

import json
import re
import sys
import threading
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "api-dev"))
sys.path.insert(0, str(DESK))
import metrics as M  # noqa: E402
import scrub as SC  # noqa: E402
import server as SV  # noqa: E402
from engine import prom  # noqa: E402

# ---------------------------------------------------------------------------------------------------- engine/prom


def test_registry_renders_the_text_format():
    r = prom.Registry()
    c = r.counter("t_requests_total", "Requests", ["route", "status"])
    c.inc(route="/api/health", status="200")
    c.inc(2, route="/api/health", status="200")
    g = r.gauge("t_live", "Live")
    g.set(3)
    g.dec()
    h = r.histogram("t_seconds", "Latency", ["route"], buckets=(0.1, 1.0))
    h.observe(0.05, route="/x")
    h.observe(5, route="/x")
    txt = r.render()
    assert "# TYPE t_requests_total counter" in txt
    assert 't_requests_total{route="/api/health",status="200"} 3' in txt
    assert "t_live 2" in txt
    assert 't_seconds_bucket{route="/x",le="0.1"} 1' in txt
    assert 't_seconds_bucket{route="/x",le="1"} 1' in txt
    assert 't_seconds_bucket{route="/x",le="+Inf"} 2' in txt
    assert 't_seconds_sum{route="/x"} 5.05' in txt and 't_seconds_count{route="/x"} 2' in txt
    assert txt.endswith("\n")
    # labels are escaped, never raw
    c.inc(route='a"b\\c\n', status="200")
    assert 'route="a\\"b\\\\c\\n"' in r.render()


def test_registry_refuses_bad_names_and_duplicate_metrics():
    r = prom.Registry()
    r.counter("ok_total", "x")
    with pytest.raises(ValueError):
        r.counter("ok_total", "again")
    with pytest.raises(ValueError):
        r.counter("bad-name", "x")
    with pytest.raises(ValueError):
        r.counter("c", "x", ["__reserved"])
    c = r.counter("labelled", "x", ["a"])
    with pytest.raises(ValueError):
        c.inc(b="1")
    with pytest.raises(ValueError):
        c.inc(-1, a="1")


def test_series_per_metric_are_capped_and_collapse_to_other():
    r = prom.Registry()
    c = r.counter("many_total", "x", ["k"])
    for i in range(prom.MAX_SERIES + 50):
        c.inc(k=f"v{i}")
    assert c.series_count() == prom.MAX_SERIES   # the `other` series takes the last slot: never more than the cap
    assert c.value(k="never-seen") == 51   # everything past the cap landed in that one series
    assert f'many_total{{k="{prom.OVERFLOW}"}} 51' in r.render()


def test_gauge_collect_reads_at_scrape_time():
    r = prom.Registry()
    state = {"n": 1}
    r.gauge("q", "x", ["kind"], collect=lambda: {("riff",): state["n"]})
    assert 'q{kind="riff"} 1' in r.render()
    state["n"] = 7
    assert 'q{kind="riff"} 7' in r.render()
    r.gauge("boom", "x", collect=lambda: 1 / 0)
    assert "# TYPE boom gauge" in r.render()   # a failing collector never breaks the scrape


# ------------------------------------------------------------------------------------------------- route patterns


@pytest.mark.parametrize("path,want", [
    ("/api/health", "/api/health"),
    ("/api/health?ready=1", "/api/health"),
    ("/api/joke/x:ab12cd", "/api/joke/{id}"),
    ("/api/joke/Something%20Private", "/api/joke/{id}"),
    ("/api/sets/20261027_up0123abcde", "/api/sets/{id}"),
    ("/api/worker/jobs/job:riff-0a1b2c3d4e5f/result", "/api/worker/jobs/{id}/result"),
    ("/api/search?q=my%20secret%20bit", "/api/search"),
    ("/api/audio/ABC", "/api/audio/{id}"),
    ("/api/memos/2026-10-27_deadbeef/transcript", "/api/memos/{id}/transcript"),
    ("/", "/"),
    ("", "/"),
    ("/a/b/c/d/e/f/g/h", "/a/b/c/d/e/f/..."),
])
def test_route_pattern_collapses_ids_and_drops_queries(path, want):
    assert M.route_pattern(path) == want


def test_route_pattern_never_keeps_a_segment_with_digits_or_case():
    for seg in ("x1", "X", "a.b", "a:b", "a" * 33, "%20", "Bit"):
        assert M.route_pattern(f"/api/{seg}") == "/api/{id}", seg


def test_record_bounds_label_cardinality():
    before = M.http_requests.series_count()
    for i in range(500):
        M.record("GET", f"/api/joke/joke:{i:05d}-{'x' * 8}", 200, 0.01)
        M.record("GET", f"/api/search?q=query-number-{i}", 200, 0.01)
    assert M.http_requests.series_count() - before == 2
    assert M.http_requests.value(method="GET", route="/api/joke/{id}", status="200") >= 500
    M.record("BREW", "/api/x", 999, 0.01)
    assert M.http_requests.value(method="OTHER", route="/api/x", status="unknown") == 1


def test_job_stats_from_the_store_shape():
    jobs = [
        {"status": "queued", "kind": "riff", "waiting_for": None, "created_at": "2026-10-27T10:00:00+00:00"},
        {"status": "queued", "kind": "riff", "waiting_for": None, "created_at": "2026-10-27T10:00:00+00:00"},
        {"status": "queued", "kind": "process_set", "waiting_for": "transcriber", "created_at": "2026-10-20T10:00:00+00:00"},
        {"status": "claimed", "kind": "ask_chat"},
        {"status": "done", "kind": "process_set", "waiting_for": None},
        "junk",
    ]
    st = M.job_stats(jobs)
    assert st["queued"] == {"riff": 2, "process_set": 1} and st["waiting"] == 1
    assert st["oldest_waiting_s"] > 24 * 3600
    assert M.job_stats([]) == {"queued": {}, "waiting": 0, "oldest_waiting_s": 0.0}


# -------------------------------------------------------------------------------------------- GET /metrics live


class BootHandler(SV.Handler):
    kbs = {}
    verifier = None
    boot = staticmethod(lambda: {"ok": False, "surreal": False, "loaded": False, "ready": False, "error": "starting"})


@pytest.fixture(scope="module")
def boot_api():
    srv = ThreadingHTTPServer(("127.0.0.1", 0), BootHandler)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    yield f"http://127.0.0.1:{srv.server_address[1]}"
    srv.shutdown()


def fetch(url, method="GET"):
    req = urllib.request.Request(url, method=method)
    try:
        with urllib.request.urlopen(req, timeout=5) as r:
            return r.status, dict(r.headers), r.read()
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers), e.read()


def test_metrics_route_answers_while_booting_with_no_token(boot_api):
    api = boot_api
    code, hs, body = fetch(api + "/metrics")
    assert code == 200 and hs["Content-Type"].startswith("text/plain; version=0.0.4")
    txt = body.decode()
    assert "# TYPE pk_http_requests_total counter" in txt
    assert re.search(r'^pk_build_info\{version="[0-9.]+",sha="[0-9a-z]+"\} 1$', txt, re.M)
    assert "pk_http_requests_in_flight 0" in txt
    assert "pk_live_clients 0" in txt
    assert "# TYPE pk_db_query_duration_seconds histogram" in txt
    assert "pk_jobs_waiting_transcriber 0" in txt
    # the rest of the API is still 503 (booting): the scrape path is the exception on purpose
    assert fetch(api + "/api/jobs")[0] == 503


def test_requests_are_counted_by_pattern_and_status_with_no_ids(boot_api):
    api = boot_api
    n0 = M.http_requests.value(method="GET", route="/api/joke/{id}", status="503")
    d0 = M.http_duration.count(method="GET", route="/api/joke/{id}")
    h0 = M.http_requests.value(method="HEAD", route="/api/health", status="503")
    for jid in ("joke:abc123", "joke:def456", "joke:Private%20Title"):
        fetch(api + f"/api/joke/{jid}")
    fetch(api + "/api/health")
    fetch(api + "/api/health", method="HEAD")
    code, _, body = fetch(api + "/metrics")
    txt = body.decode()
    assert M.http_requests.value(method="GET", route="/api/joke/{id}", status="503") == n0 + 3
    assert M.http_requests.value(method="HEAD", route="/api/health", status="503") == h0 + 1   # once, not twice
    assert M.http_duration.count(method="GET", route="/api/joke/{id}") == d0 + 3
    assert 'pk_http_requests_total{method="GET",route="/api/joke/{id}",status="503"}' in txt
    assert M.http_requests.value(method="GET", route="/metrics", status="200") == 0   # the scrape is not traffic
    for word in ("abc123", "def456", "Private", "joke:"):
        assert word not in txt, word
    # every label value on every line is a route pattern, a method, a status or a build label
    for line in txt.splitlines():
        for k, v in re.findall(r'(\w+)="((?:[^"\\]|\\.)*)"', line):
            assert k in ("method", "route", "status", "version", "sha", "le", "kind"), (k, v)
            if k == "route":
                assert re.fullmatch(r"(/[a-z_\-]+|/\{id\}|/\.\.\.)+|/", v), v


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


def event_with_everything():
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


def _values_of(x, key):
    if isinstance(x, dict):
        for k, v in x.items():
            if k == key:
                yield v
            yield from _values_of(v, key)
    elif isinstance(x, list):
        for v in x:
            yield from _values_of(v, key)


def test_scrub_event_strips_every_private_field_and_keeps_what_locates_the_crash():
    out = SC.scrub_event(event_with_everything(), {})
    blob = json.dumps(out)
    for k, v in PRIVATE.items():
        assert v not in blob, (k, v)
    assert "owner@example.test" not in blob and "203.0.113.9" not in blob and "session=abc" not in blob
    keys = SC.flatten_keys(out)
    for bad in ("query_string", "cookies", "env", "vars", "context_line", "pre_context", "Authorization", "Cookie",
                "X-Title", "email", "ip_address", "body"):
        assert bad not in keys, bad
    assert "data" not in SC.flatten_keys(out["request"])   # the request body is gone; a breadcrumb's data is route+status only
    # a sensitive key may survive only as a REDACTED placeholder, never with a value
    for k in ("text", "quote", "transcript", "query", "title"):
        assert all(v == SC.REDACTED for v in _values_of(out, k)), k
    # what survives: the type, a bounded message, the frames (file/function/line), the route tag, the request method+path
    assert out["exception"]["values"][0]["type"] == "SurrealError"
    assert out["exception"]["values"][0]["value"] == "statement 0 failed: '...'"
    assert out["message"].startswith("boom: '...' xxx")
    assert out["exception"]["values"][0]["stacktrace"]["frames"] == [{"filename": "server.py", "function": "do_POST", "lineno": 1}]
    assert out["request"] == {"method": "POST", "url": "https://example.test/api/decision", "headers": {"Content-Type": "application/json"}}
    assert out["tags"] == {"route": "/api/decision"}
    assert out["user"] == {"id": "owner"}
    assert out["breadcrumbs"]["values"] == [{"category": "http", "data": {"route": "/api/joke/{id}", "status": 500}}]
    assert out["extra"] == {"nested": {"deep": {"quote": SC.REDACTED, "n": 3}}, "QUERY_STRING": SC.REDACTED, "ok": "fine",
                            "job": {"kind": "riff", "text": SC.REDACTED}}
    assert len(out["message"]) <= SC.MAX_MESSAGE and out["release"] == "punchable-api@0.1.0+abc"
    assert out["threads"]["values"][0]["stacktrace"]["frames"] == [{"function": "f"}]
    assert "custom" not in out["contexts"]


def test_scrub_event_never_drops_the_event_itself():
    assert SC.scrub_event({"exception": {"values": [{"type": "X", "value": "y"}]}}, None)["exception"]["values"][0]["type"] == "X"
    assert SC.scrub_text("KeyError: 'the bit'" + chr(10) + "--- SELECT 1") == "KeyError: '...'"
    assert SC.scrub_event("not a dict", None) == "not a dict"
    assert SC.scrub_event({}, None) == {}


def test_sensitive_key_rule_is_case_and_separator_insensitive():
    for k in ("text", "TEXT", "Transcript", "set-cookie", "joke_text", "search_query", "x-title", "HTTP_AUTHORIZATION"):
        assert SC.sensitive_key(k), k
    for k in ("route", "status", "method", "lineno", "release", "function"):
        assert not SC.sensitive_key(k), k


def test_the_scrubber_test_goes_red_when_the_rule_is_broken(monkeypatch):
    """The proof the previous test is load-bearing: with `text` taken off the sensitive list, his words leak and the
    same assertion fails. (This is the "break the guard once" check, kept as a permanent test.)"""
    monkeypatch.setattr(SC, "CONTRACT_KEYS", frozenset(SC.CONTRACT_KEYS - {"text"}))  # the set sensitive_key() reads
    monkeypatch.setattr(SC, "_KEY_HINT", re.compile(r"(quote|transcript|query|title|secret|token|password|cookie|auth)", re.I))
    out = SC.scrub_event(event_with_everything(), {})
    assert PRIVATE["text"] in json.dumps(out)   # the leak the real rule prevents (extra.nested / contexts / vars)


# ---------------------------------------------------------------------------------------------- crash.py no-DSN


def test_crash_module_is_a_no_op_without_a_dsn(monkeypatch):
    import crash as CRASH
    monkeypatch.delenv("SENTRY_DSN", raising=False)
    assert CRASH.init(log=lambda *a: None) is False and CRASH.enabled() is False
    CRASH.report(RuntimeError("x"), route="/api/x", status=500)   # must not raise
    CRASH.breadcrumb("/api/x", "GET", 500)
    CRASH.flush()
    assert CRASH.release().startswith("punchable-api@")
    assert CRASH.release("punchable-worker").startswith("punchable-worker@")   # the worker's name for the same module


def test_crash_report_sets_the_route_status_and_tags_on_the_scope(monkeypatch):
    """report() hands the SDK the exception inside a scope carrying route, status and the caller's tags (the worker's
    job kind), every value a bounded string."""
    import crash as CRASH

    class Scope:
        def __init__(self):
            self.tags = {}

        def set_tag(self, k, v):
            self.tags[k] = v

    class Sdk:
        def __init__(self):
            self.scope, self.captured = Scope(), []

        def new_scope(self):
            sdk = self

            class Cm:
                def __enter__(self_):
                    return sdk.scope

                def __exit__(self_, *a):
                    return False
            return Cm()

        def capture_exception(self, exc):
            self.captured.append(exc)

    fake = Sdk()
    monkeypatch.setattr(CRASH, "_sdk", fake)
    err = ValueError("x")
    CRASH.report(err, route="/api/x", status=500, tags={"kind": "riff", "retry": False})
    assert fake.captured == [err]
    assert fake.scope.tags == {"route": "/api/x", "status": "500", "kind": "riff", "retry": "False"}
    assert all(isinstance(v, str) and len(v) <= 64 for v in fake.scope.tags.values())
