"""Goldens for pk-llm: what the Python Anthropic provider (engine/providers.py AnthropicBackend + Router) puts on the
wire through the pinned SDK, what it makes of a streamed answer, the daily budget ledger (engine/budget.py) after a
scripted sequence of charges, and the plain-words error texts. The Rust tests build the same requests and compare as
parsed JSON. Synthetic prompts and schemas only: nothing of the comic's is here. Never calls the real API: the SDK
client is given an httpx MockTransport that records the request and answers a canned SSE stream.

    python rust/tools/gen_llm_goldens.py        (from the repo root; writes rust/crates/pk-llm/tests/goldens/)

Needs the pinned SDK (infra/worker/requirements.txt): pip install anthropic==0.104.1 jsonschema==4.26.0 httpx==0.28.1
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

import anthropic
import httpx

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from engine import budget as B  # noqa: E402
from engine.config import ENGINE_DIR, Config  # noqa: E402
from engine.providers import (  # noqa: E402
    AnthropicBackend, CutShort, NoProvider, Refusal, Router, parse_json, schema_instructions,
)
from anthropic.lib._parse._transform import transform_schema  # noqa: E402

OUT = ROOT / "rust" / "crates" / "pk-llm" / "tests" / "goldens"
KEY = "sk-golden-not-a-key"
HOSTED = ENGINE_DIR / "config.hosted.toml"

# ------------------------------------------------------------------ synthetic prompts
PROFILE = ("VOICE PROFILE (synthetic). Deadpan, short sentences, the turn lands on the last word. "
           "Never explain the joke. " * 4).strip()
LEARNED = "WHAT HE KEPT (synthetic): 'the meter asked for my pronouns'; 'my cat swiped left'."
MATERIAL = "Bit: parking meters now take cards, apps, and apparently my dignity. Give me three turns on it."

# the schemas exercise every branch of the SDK's transform_schema (anthropic/lib/_parse/_transform.py)
SCHEMA_PLAN = {"type": "object", "properties": {"search_queries": {"type": "array", "items": {"type": "string"}}},
               "required": ["search_queries"], "additionalProperties": True}
SCHEMA_RICH = {
    "type": "object", "title": "Riff", "description": "Three turns on the bit",
    "$defs": {"Turn": {"type": "object", "properties": {"text": {"type": "string", "minLength": 1, "maxLength": 280},
                                                        "kind": {"type": "string", "enum": ["tag", "act_out", "callback"]},
                                                        "score": {"type": "number", "minimum": 0, "maximum": 1, "default": 0.5}},
                       "required": ["text", "kind"], "additionalProperties": False}},
    "properties": {
        "turns": {"type": "array", "items": {"$ref": "#/$defs/Turn"}, "minItems": 1, "maxItems": 3},
        "tags": {"type": "array", "items": {"type": "string", "pattern": "^[a-z-]+$"}, "minItems": 2},
        "single": {"type": "array", "items": {"type": "integer"}, "minItems": 0},
        "email": {"type": "string", "format": "email"},
        "slug": {"type": "string", "format": "slug", "description": "lowercase"},
        "when": {"type": "string", "format": "date-time"},
        "count": {"type": "integer", "minimum": 1, "maximum": 10, "description": "A number"},
        "keep": {"type": "boolean", "default": True},
        "nothing": {"type": "null"},
        "maybe": {"anyOf": [{"type": "string"}, {"type": "null"}], "description": "optional words"},
        "either": {"oneOf": [{"type": "integer"}, {"type": "boolean"}]},
        "both": {"allOf": [{"type": "object", "properties": {"a": {"type": "string"}}}]},
        "extra": {"type": "object", "properties": {}, "x-extra": {"k": 1, "list": ["a", "b"]}, "const": "fixed",
                  "examples": ["one", "two"], "nullable": None, "ratio": 1.5},
    },
    "required": ["turns"],
}
SCHEMA_JUDGE = {"type": "object", "properties": {"same": {"type": "boolean"}, "why": {"type": "string"}},
                "required": ["same", "why"], "additionalProperties": False}
SCHEMAS = {"plan": SCHEMA_PLAN, "rich": SCHEMA_RICH, "judge": SCHEMA_JUDGE}


# ------------------------------------------------------------------ the fake wire
def sse(events: list[dict]) -> str:
    return "".join(f"event: {e['type']}\ndata: {json.dumps(e)}\n\n" for e in events)


def stream_events(text_blocks: list[str], *, stop_reason="end_turn", model="claude-sonnet-5-5", input_tokens=11,
                  output_tokens=7, cache_read=0, cache_write=0, stop_details=None, thinking=True) -> list[dict]:
    """A canned Messages stream the way the API sends one: message_start carries the input usage, each text block
    arrives in two deltas, message_delta carries the stop reason and the final output_tokens."""
    usage0 = {"input_tokens": input_tokens, "output_tokens": 1, "cache_creation_input_tokens": cache_write,
              "cache_read_input_tokens": cache_read, "service_tier": "standard"}
    ev = [{"type": "message_start", "message": {"id": "msg_golden", "type": "message", "role": "assistant", "model": model,
                                                 "content": [], "stop_reason": None, "stop_sequence": None, "usage": usage0}}]
    i = 0
    if thinking:
        ev += [{"type": "content_block_start", "index": i, "content_block": {"type": "thinking", "thinking": "", "signature": ""}},
               {"type": "content_block_delta", "index": i, "delta": {"type": "thinking_delta", "thinking": "hmm"}},
               {"type": "content_block_delta", "index": i, "delta": {"type": "signature_delta", "signature": "sig=="}},
               {"type": "content_block_stop", "index": i}]
        i += 1
    for t in text_blocks:
        a, b = t[: len(t) // 2], t[len(t) // 2:]
        ev += [{"type": "content_block_start", "index": i, "content_block": {"type": "text", "text": ""}},
               {"type": "content_block_delta", "index": i, "delta": {"type": "text_delta", "text": a}},
               {"type": "content_block_delta", "index": i, "delta": {"type": "text_delta", "text": b}},
               {"type": "content_block_stop", "index": i}]
        i += 1
    delta = {"stop_reason": stop_reason, "stop_sequence": None}
    if stop_details:
        delta["stop_details"] = stop_details
    ev += [{"type": "message_delta", "delta": delta, "usage": {"output_tokens": output_tokens}},
           {"type": "message_stop"}]
    return ev


class Wire:
    """An httpx MockTransport handler: records every request (headers + parsed body), answers from a script."""

    def __init__(self, script: list[str]):
        self.script, self.seen = list(script), []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        hdr = {k.lower(): v for k, v in request.headers.items()
               if k.lower() in ("anthropic-version", "accept", "content-type")}
        self.seen.append({"method": request.method, "path": request.url.path, "headers": hdr,
                          "body": json.loads(request.content)})
        body = self.script.pop(0) if self.script else sse(stream_events(['{"ok": true}']))
        return httpx.Response(200, headers={"content-type": "text/event-stream"}, text=body)

    def client(self):
        return anthropic.Anthropic(api_key=KEY, max_retries=0,
                                   http_client=httpx.Client(transport=httpx.MockTransport(self)))


def hosted_cfg(tmp: Path) -> Config:
    return Config(HOSTED, overrides={"paths": {"kb_dir": (tmp / "kb").as_posix(), "secrets_env": "/nonexistent"}})


def messages(n_system: int) -> list[dict]:
    """creative/voice.py messages(): profile, learned, the task line, then the user turn; fewer blocks when the
    profile or the learned section is missing."""
    sys_blocks = [PROFILE, LEARNED, "TASK (synthetic): three turns, one line each."][:3]
    if n_system == 2:
        sys_blocks = [PROFILE, sys_blocks[2]]
    elif n_system == 1:
        sys_blocks = [sys_blocks[2]]
    elif n_system == 0:
        sys_blocks = []
    return [{"role": "system", "content": s} for s in sys_blocks] + [{"role": "user", "content": MATERIAL}]


# ------------------------------------------------------------------ goldens
def gen_requests(tmp: Path) -> dict:
    cfg = hosted_cfg(tmp)
    tasks = sorted(cfg.data["tasks"])
    cases = []
    for task in tasks:
        for n_sys, sname in ((3, "rich"), (2, None), (1, "judge")):
            msgs = messages(n_sys)
            schema = SCHEMAS[sname] if sname else None
            wire = Wire([sse(stream_events(['{"turns": []}'] if sname == "rich" else ['{"same": true, "why": "x"}']
                                           if sname else ["plain words"]))])
            AnthropicBackend.client_factory = staticmethod(wire.client)
            AnthropicBackend.budget = B.Budget(50.0, tmp / "spend-req.json")
            try:
                if schema:  # the canned answer must validate or the router retries: a schema-shaped one per schema
                    answer = {"rich": {"turns": [{"text": "t", "kind": "tag"}]}, "judge": {"same": True, "why": "x"},
                              "plan": {"search_queries": ["q"]}}[sname]
                    wire.script = [sse(stream_events([json.dumps(answer)]))]
                c = Router(cfg).complete(task, msgs, schema)
            finally:
                AnthropicBackend.client_factory = None
                AnthropicBackend.budget = None
            assert len(wire.seen) == 1, (task, wire.seen)
            req = wire.seen[0]
            cases.append({"name": f"{task}/{n_sys}sys/{sname or 'text'}", "task": task, "messages": msgs,
                          "schema": schema, "model": c.model, "request": req})
    # no system block at all, and a model that does not think (haiku) with a schema
    for task, n_sys, sname in (("ask_plan", 0, "plan"), ("merge_judge", 0, None)):
        msgs = messages(n_sys)
        schema = SCHEMAS[sname] if sname else None
        wire = Wire([sse(stream_events([json.dumps({"search_queries": ["q"]}) if schema else "words"]))])
        AnthropicBackend.client_factory = staticmethod(wire.client)
        AnthropicBackend.budget = B.Budget(50.0, tmp / "spend-req.json")
        try:
            c = Router(cfg).complete(task, msgs, schema)
        finally:
            AnthropicBackend.client_factory = None
            AnthropicBackend.budget = None
        cases.append({"name": f"{task}/{n_sys}sys/{sname or 'text'}", "task": task, "messages": msgs,
                      "schema": schema, "model": c.model, "request": wire.seen[0]})
    # the retry on invalid JSON: the second request carries the bad answer and the correction
    retries = []
    for bad in ("not json at all", '{"search_queries": "x"}', '```json\n{"search_queries": ["fenced"]}\n```'):
        wire = Wire([sse(stream_events([bad])), sse(stream_events(['{"search_queries": ["q"]}']))])
        AnthropicBackend.client_factory = staticmethod(wire.client)
        AnthropicBackend.budget = B.Budget(50.0, tmp / "spend-req.json")
        try:
            c = Router(cfg).complete("ask_plan", messages(2), SCHEMA_PLAN)
        finally:
            AnthropicBackend.client_factory = None
            AnthropicBackend.budget = None
        retries.append({"bad_answer": bad, "requests": wire.seen, "data": c.data, "attempts": c.attempts})
    # the fallback: creative_riff refused on Opus is retried once on the default task's model
    wire = Wire([sse(stream_events([], stop_reason="refusal", model="claude-opus-5-5",
                                   stop_details={"type": "refusal", "category": "general_harms",
                                                 "explanation": "the request names a real person"}, thinking=False)),
                 sse(stream_events(["second try"]))])
    AnthropicBackend.client_factory = staticmethod(wire.client)
    AnthropicBackend.budget = B.Budget(50.0, tmp / "spend-req.json")
    try:
        c = Router(cfg).complete("creative_riff", messages(3))
    finally:
        AnthropicBackend.client_factory = None
        AnthropicBackend.budget = None
    fallback = {"models": [r["body"]["model"] for r in wire.seen], "requests": wire.seen, "text": c.text,
                "model": c.model, "attempts": c.attempts}
    # the task table as the router resolves it: provider, model, the merged task config fields the backend reads
    routes = {t: {"chain": Router(cfg).resolve(t), "models_for": Router(cfg).models_for(cfg.task(t), "anthropic",
                                                                                           cfg.task(t)["models"]["anthropic"]),
                  "effort": cfg.task(t).get("effort"), "offline": bool(cfg.task(t).get("offline", False)),
                  "fallbacks": cfg.task(t).get("fallbacks") or []} for t in tasks}
    return {"cases": cases, "retries": retries, "fallback": fallback, "routes": routes,
            "schema_instructions": {k: schema_instructions(v) for k, v in SCHEMAS.items()}}


def gen_schemas() -> dict:
    out = {k: transform_schema(v) for k, v in SCHEMAS.items()}
    out["ref_only"] = transform_schema({"$ref": "#/$defs/X", "description": "dropped"})
    out["string_plain"] = transform_schema({"type": "string"})
    return out


def gen_streams(tmp: Path) -> list[dict]:
    """Canned streams through AnthropicBackend.call(): the text, the tokens, the cache numbers and the USD charged,
    or the plain-words error. The Rust client is given the same bytes by wiremock."""
    cfg = hosted_cfg(tmp)
    scripts = {
        "normal": stream_events(["Three turns: ", "one, two, three."], input_tokens=1200, output_tokens=90),
        "cache_hit": stream_events(['{"same": true, "why": "x"}'], model="claude-opus-5-5", input_tokens=40,
                                   output_tokens=12, cache_read=3000, cache_write=700, thinking=False),
        "refusal": stream_events([], stop_reason="refusal", thinking=False, input_tokens=5, output_tokens=1,
                                 stop_details={"type": "refusal", "category": "general_harms",
                                               "explanation": "the request names a real person"}),
        "refusal_bare": stream_events([], stop_reason="refusal", thinking=False, input_tokens=5, output_tokens=1,
                                      stop_details={"type": "refusal", "category": "cyber", "explanation": None}),
        "max_tokens": stream_events(["and then the meter said"], stop_reason="max_tokens", output_tokens=16000),
    }
    out = []
    for name, events in scripts.items():
        model = events[0]["message"]["model"]
        wire = Wire([sse(events)])
        b = AnthropicBackend("anthropic", cfg.providers["anthropic"], cfg)
        b.client_factory = staticmethod(wire.client)
        b.budget = B.Budget(5.0, tmp / f"spend-{name}.json")
        tcfg = cfg.task("ask_chat")
        tcfg["_task"] = "ask_chat"
        rec = {"name": name, "model": model, "sse": sse(events)}
        try:
            text, tin, tout, extra = b.call(model, messages(2), None, tcfg)
            rec.update({"ok": True, "text": text, "tokens_in": tin, "tokens_out": tout, **extra})
        except (Refusal, CutShort) as e:
            rec.update({"ok": False, "error": type(e).__name__, "plain": e.plain})
        spent = json.loads((tmp / f"spend-{name}.json").read_text())
        rec["charged_usd"] = next(iter(spent.values()))["usd"]  # charged even when refused: the tokens were spent
        out.append(rec)
    return out


def gen_budget(tmp: Path) -> dict:
    """A scripted ledger: 16 days of small charges (the 14-day trim), then one day with every model and the cap."""
    path = tmp / "ledger.json"
    b = B.Budget(5.0, path)
    days = [f"2026-09-{d:02d}" for d in range(14, 31)] + ["2026-10-01"]   # 18 days before today
    today = "2026-10-29"
    steps = []
    U = lambda i, o, w=0, r=0: {"input_tokens": i, "output_tokens": o, "cache_creation_input_tokens": w,  # noqa: E731
                                 "cache_read_input_tokens": r}
    for d in days:
        B.today = lambda d=d: d
        usd = b.charge("claude-haiku-5-5", U(1000, 100), task="memo_triage")
        steps.append({"day": d, "model": "claude-haiku-5-5", "usage": U(1000, 100), "task": "memo_triage", "usd": usd})
    B.today = lambda: today
    charges = [("claude-opus-5-5", U(1000, 500, 2000, 30000), "creative_riff"),
               ("claude-sonnet-5-5", U(800, 200, 0, 12000), "ask_chat"),
               ("claude-haiku-5-5", U(300, 50), "merge_judge"),
               ("claude-sonnet-5-5-20260101", U(100, 10), "ask_plan"),      # longest prefix wins: sonnet-5-5
               ("claude-sonnet-5", U(100, 10), "ask_plan"),                 # the older sonnet row
               ("claude-fable-5-1", U(100, 10), "ask_answer"),              # not in the table: priced as Opus
               ("claude-opus-5-5", U(1_000_000, 0), "creative_punchup")]    # $4.00: over the cap
    for model, usage, task in charges:
        before = b.status()
        usd = b.charge(model, usage, task=task)
        steps.append({"day": today, "model": model, "usage": usage, "task": task, "usd": usd,
                      "remaining_before": round(before["daily_usd"] - before["spent_usd"], 6)})
    try:
        b.check()
        exceeded = None
    except B.BudgetExceeded as e:
        exceeded = str(e)
    prices = {m: list(B.price_of(m)) for m in ("claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5", "claude-opus-5",
                                               "claude-sonnet-5", "claude-haiku-4-5", "claude-sonnet-5-5-20260101",
                                               "CLAUDE-OPUS-5-5", "claude-fable-5-1", "", "gpt-x")}
    zero = B.Budget(0.0, tmp / "zero.json")
    try:
        zero.check()
        zero_exceeded = None
    except B.BudgetExceeded as e:
        zero_exceeded = str(e)
    return {"daily_usd": 5.0, "today": today, "steps": steps, "ledger": json.loads(path.read_text()),
            "status": b.status(), "remaining": b.remaining(), "spent_today": b.spent_today(), "reason": b.reason(),
            "exceeded": exceeded, "prices": prices, "zero_cap_exceeded": zero_exceeded,
            "fresh_status": B.Budget(2.0, tmp / "fresh.json").status()}


def gen_errors() -> dict:
    refusals = [(None, None), ("general_harms", None), ("general_harms", "the request names a real person"),
                ("cyber", "  trailing dot.  "), (None, "no category"), ("x", ""), ("x", "   ")]
    return {"refusal": [{"category": c, "explanation": e, "plain": Refusal(c, e).plain} for c, e in refusals],
            "cut_short": [{"max_tokens": n, "plain": CutShort(n).plain} for n in (16000, 4096)],
            "no_provider": [{"task": t, "message": str(NoProvider(f"no available provider for task {t!r}"))}
                            for t in ("creative_riff", "it's")],
            "parse_json": [parse_probe(t) for t in
                           ('{"a": 1}', '  {"a": 1}  ', '```json\n{"a": [1, 2]}\n```', '```\n{"a": "b"}```',
                            'Sure! Here it is: {"a": {"b": 2}} hope that helps', '{"a": 1} trailing {"b": 2}',
                            'no braces here', '```json\nnot json```')]}


def parse_probe(text: str) -> dict:
    try:
        return {"text": text, "ok": True, "data": parse_json(text)}
    except ValueError:
        return {"text": text, "ok": False}


def main():
    os.environ["ANTHROPIC_API_KEY"] = KEY   # the router only lists the provider when a key is present; never real
    os.environ.pop("PK_AI_DAILY_USD", None)
    OUT.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        files = {"requests.json": gen_requests(tmp), "schemas.json": gen_schemas(), "streams.json": gen_streams(tmp),
                 "budget.json": gen_budget(tmp), "errors.json": gen_errors()}
    for name, data in files.items():
        (OUT / name).write_text(json.dumps(data, indent=1, ensure_ascii=False, sort_keys=False) + "\n", encoding="utf-8")
        print(f"wrote {OUT / name}")


if __name__ == "__main__":
    main()
