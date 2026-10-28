"""The crash-report scrubber: what a Sentry-compatible event may carry out of this API (api/entry.py -> crash.py).

The comic's material never leaves through an error report. Before any event is sent it loses:
  * every request body and query string (`request.data`, `request.query_string`, `request.cookies`),
  * every header but a short allow-list (no Authorization, no Cookie, no X-Title / X-Venue / X-Filename),
  * every field, at any depth, whose name says it holds his words: text, quote, transcript, query, title, body,
    content, prompt, answer, lines, summary, name, venue, filename, token, secret, password, authorization, cookie,
  * breadcrumb `data` and `message` (the route pattern and status survive in `category`/`data.route`),
  * `user` beyond an opaque id, and the local variables Sentry attaches to stack frames.
The exception type, message (route-free, truncated), stack trace (file, line, function) and tags survive: that is
what tells us the API crashed and where. The unit test (api-dev/tests/test_metrics.py) proves the rule and that
breaking it turns the test red.
"""
from __future__ import annotations

import re

SENSITIVE_KEYS = frozenset({
    "text", "quote", "transcript", "query", "q", "title", "body", "content", "prompt", "answer", "lines", "summary",
    "name", "venue", "filename", "original_name", "reason", "note", "notes", "idea", "data", "payload", "vars",
    "token", "secret", "password", "authorization", "cookie", "cookies", "set-cookie", "api_key", "apikey", "key",
    "email", "user", "username", "sub", "ip_address",
})
HEADER_ALLOW = frozenset({"content-type", "content-length", "user-agent", "accept", "host", "x-request-id"})
REDACTED = "[scrubbed]"
MAX_MESSAGE = 300
_QUOTED = re.compile(r"'[^']*'|\"[^\"]*\"")
_KEY_HINT = re.compile(r"(text|quote|transcript|query|title|secret|token|password|cookie|auth)", re.I)


def sensitive_key(k) -> bool:
    s = str(k).strip().lower().replace("-", "_")
    return s in SENSITIVE_KEYS or s.replace("_", "-") in SENSITIVE_KEYS or bool(_KEY_HINT.search(s))


def scrub_text(s) -> str:
    """An exception message or log line: the first line only (SurrealError appends the statement after a newline),
    quoted literals replaced (a KeyError's key, a parser's snippet, a search term in a query), bounded."""
    s = str(s or "").split(chr(10), 1)[0]
    return _QUOTED.sub("'...'", s)[:MAX_MESSAGE]


def scrub_value(v, depth: int = 0):
    """Recursively drop sensitive keys; cut depth and width so a scrubbed event stays small."""
    if depth > 8:
        return REDACTED
    if isinstance(v, dict):
        out = {}
        for i, (k, x) in enumerate(v.items()):
            if i >= 64:
                break
            out[str(k)] = REDACTED if sensitive_key(k) else scrub_value(x, depth + 1)
        return out
    if isinstance(v, (list, tuple)):
        return [scrub_value(x, depth + 1) for x in list(v)[:64]]
    if isinstance(v, str):
        return v if len(v) <= 1000 else v[:1000] + "..."
    return v


def scrub_request(req) -> dict:
    if not isinstance(req, dict):
        return {}
    out = {}
    if isinstance(req.get("method"), str):
        out["method"] = req["method"]
    if isinstance(req.get("url"), str):
        out["url"] = req["url"].split("?", 1)[0]
    hs = req.get("headers")
    if isinstance(hs, dict):
        out["headers"] = {k: v for k, v in hs.items() if str(k).lower() in HEADER_ALLOW}
    # data, query_string, cookies, env: gone
    return out


def scrub_breadcrumb(bc) -> dict:
    if not isinstance(bc, dict):
        return {}
    out = {k: bc[k] for k in ("timestamp", "type", "category", "level") if k in bc}
    d = bc.get("data")
    if isinstance(d, dict):
        keep = {}
        for k in ("route", "status", "status_code", "method", "reason_class", "duration_ms"):
            if k in d and isinstance(d[k], (str, int, float)):
                keep[k] = d[k]
        if keep:
            out["data"] = keep
    return out


def scrub_exception(exc) -> dict:
    if not isinstance(exc, dict):
        return {}
    vals = []
    for v in (exc.get("values") or [])[:8]:
        if not isinstance(v, dict):
            continue
        item = {"type": str(v.get("type") or "")[:100], "value": scrub_text(v.get("value"))}
        st = v.get("stacktrace")
        if isinstance(st, dict):
            frames = []
            for f in (st.get("frames") or [])[-50:]:
                if isinstance(f, dict):
                    frames.append({k: f[k] for k in ("filename", "abs_path", "function", "lineno", "module", "in_app")
                                   if k in f})   # no `vars`, no `context_line`, no `pre_context`/`post_context`
            item["stacktrace"] = {"frames": frames}
        if isinstance(v.get("mechanism"), dict):
            item["mechanism"] = {k: v["mechanism"][k] for k in ("type", "handled") if k in v["mechanism"]}
        vals.append(item)
    return {"values": vals}


def scrub_event(event, hint=None):
    """sentry_sdk before_send. Returns the event to send (never None: a crash is always reported, just bare)."""
    if not isinstance(event, dict):
        return event
    out = {}
    for k in ("event_id", "timestamp", "platform", "level", "logger", "release", "environment", "server_name",
              "transaction", "sdk", "dist", "type"):
        if k in event:
            out[k] = event[k]
    if "message" in event:
        m = event["message"]
        out["message"] = scrub_text(m if isinstance(m, str) else (m or {}).get("formatted", ""))
    if "logentry" in event and isinstance(event["logentry"], dict):
        out["logentry"] = {"message": scrub_text(event["logentry"].get("message"))}
    if "exception" in event:
        out["exception"] = scrub_exception(event["exception"])
    if "request" in event:
        out["request"] = scrub_request(event["request"])
    if isinstance(event.get("breadcrumbs"), dict):
        out["breadcrumbs"] = {"values": [scrub_breadcrumb(b) for b in (event["breadcrumbs"].get("values") or [])[-30:]]}
    elif isinstance(event.get("breadcrumbs"), list):
        out["breadcrumbs"] = [scrub_breadcrumb(b) for b in event["breadcrumbs"][-30:]]
    if isinstance(event.get("tags"), dict):
        out["tags"] = {str(k): scrub_value(v) for k, v in event["tags"].items() if not sensitive_key(k)}
    if isinstance(event.get("contexts"), dict):
        ctx = {}
        for k in ("runtime", "os", "device", "app", "trace"):
            if isinstance(event["contexts"].get(k), dict):
                ctx[k] = scrub_value(event["contexts"][k])
        out["contexts"] = ctx
    if isinstance(event.get("extra"), dict):
        out["extra"] = {str(k): v for k, v in scrub_value(event["extra"]).items()}
    if isinstance(event.get("user"), dict) and event["user"].get("id") is not None:
        out["user"] = {"id": str(event["user"]["id"])[:64]}
    # threads carry stack traces with the same frame rule
    if isinstance(event.get("threads"), dict):
        out["threads"] = {"values": [{"id": t.get("id"), "crashed": t.get("crashed"),
                                      "stacktrace": scrub_exception({"values": [{"stacktrace": t.get("stacktrace")}]})
                                      ["values"][0].get("stacktrace")}
                                     for t in (event["threads"].get("values") or [])[:16] if isinstance(t, dict)]}
    return out


def flatten_keys(x, prefix: str = "") -> set[str]:
    """Every key path in a (scrubbed) event; the test asserts no sensitive key survives."""
    keys: set[str] = set()
    if isinstance(x, dict):
        for k, v in x.items():
            keys.add(str(k))
            keys |= flatten_keys(v, prefix + str(k) + ".")
    elif isinstance(x, (list, tuple)):
        for v in x:
            keys |= flatten_keys(v, prefix)
    return keys
