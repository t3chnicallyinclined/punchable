import json
from types import SimpleNamespace

from engine.config import Config
from engine.providers import AnthropicBackend, ClaudeCodeBackend, Router
from engine.tests.conftest import MockBackend

SCHEMA = {"type": "object", "properties": {"search_queries": {"type": "array", "items": {"type": "string"}}},
          "required": ["search_queries"], "additionalProperties": True}


def test_mock_complete_logs_run(env):
    r = Router(env["cfg"])
    c = r.complete("ask_plan", [{"role": "user", "content": "hi"}], schema=SCHEMA, run_id="run-1", fewshot_sha="fs1")
    assert c.data["search_queries"] == ["airport"] and c.provider == "mock" and c.model == "mock-1"
    rec = json.loads(env["cfg"].runs_log.read_text().splitlines()[-1])
    for k in ("run_id", "task", "provider", "model", "prompt_sha", "fewshot_sha", "tokens_in", "tokens_out", "ms"):
        assert k in rec
    assert rec["run_id"] == "run-1" and rec["fewshot_sha"] == "fs1" and rec["tokens_in"] == 100


def test_invalid_json_is_retried(env):
    MockBackend.fail_first_json = True
    c = Router(env["cfg"]).complete("ask_plan", [{"role": "user", "content": "x"}], schema=SCHEMA)
    assert c.data["search_queries"] == ["airport"]
    lines = [json.loads(l) for l in env["cfg"].runs_log.read_text().splitlines()]
    assert [l["ok"] for l in lines] == [False, True]


def test_unavailable_providers_are_skipped(env):
    cfg = Config(env["toml"], overrides={
        "providers": {"broken": {"kind": "openai_compatible", "base_url": "http://127.0.0.1:9/v1"},
                      "anthropic": {"kind": "anthropic"}},
        "tasks": {"ask_plan": {"providers": ["anthropic", "broken", "mock"],
                               "models": {"anthropic": "claude-opus-5-5", "broken": "x", "mock": "mock-1"}}}})
    # no key -> no anthropic; nothing listening on port 9 and no autostart -> skipped
    assert [p for p, _ in Router(cfg).resolve("ask_plan")] == ["mock"]


def anth_cfg(env, secrets=None):
    if secrets is not None:
        (env["tmp"] / "secrets.env").write_text(secrets, encoding="utf-8")
    return Config(env["toml"], overrides={
        "providers": {"anthropic": {"kind": "anthropic", "api_key_env": "ANTHROPIC_API_KEY", "max_tokens": 16000,
                                    "thinking": "adaptive"}},
        "tasks": {"label": {"providers": ["anthropic", "mock"],
                            "models": {"anthropic": "claude-opus-5-5", "mock": "mock-1"}}}})


def test_anthropic_only_with_key(env, monkeypatch):
    assert Router(anth_cfg(env)).resolve("label")[0][0] == "mock"
    assert Router(anth_cfg(env, "ANTHROPIC_API_KEY=sk-test-from-file\n")).resolve("label")[0] == \
        ("anthropic", "claude-opus-5-5")
    (env["tmp"] / "secrets.env").unlink()
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-test-env")
    assert Router(anth_cfg(env)).resolve("label")[0][0] == "anthropic"


class FakeStream:
    """What client.messages.stream(**kw) returns: a context manager whose get_final_message() is the response."""

    def __init__(self, resp):
        self.resp = resp

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False

    def get_final_message(self):
        return self.resp


def fake_client(seen: list, text='{"search_queries": ["dad"]}', usage=None, stop="end_turn"):
    usage = usage or dict(input_tokens=11, output_tokens=7, cache_creation_input_tokens=0, cache_read_input_tokens=0)
    resp = SimpleNamespace(stop_reason=stop, stop_details=None, usage=SimpleNamespace(**usage),
                           content=[SimpleNamespace(type="thinking", thinking="..."), SimpleNamespace(type="text", text=text)])

    class FakeMessages:
        def stream(self, **kw):
            seen.append(kw)
            return FakeStream(resp)

        def create(self, **kw):
            raise AssertionError("the hosted provider streams; create() must not be used")

    return SimpleNamespace(messages=FakeMessages())


def test_anthropic_streams_caches_the_system_prefix_and_never_logs_the_key(env):
    from engine.budget import Budget
    cfg = anth_cfg(env, "ANTHROPIC_API_KEY=sk-secret-123\n")
    seen = []
    AnthropicBackend.client_factory = staticmethod(lambda: fake_client(seen))
    AnthropicBackend.budget = Budget(5.0, env["tmp"] / "spend.json")
    try:
        c = Router(cfg).complete("label", [{"role": "system", "content": "sys"}, {"role": "user", "content": "u"}],
                                 SCHEMA)
    finally:
        AnthropicBackend.client_factory = None
        AnthropicBackend.budget = None
    assert c.provider == "anthropic" and c.data == {"search_queries": ["dad"]}
    assert c.tokens_in == 11 and c.tokens_out == 7
    kw = seen[0]
    assert kw["model"] == "claude-opus-5-5" and kw["thinking"] == {"type": "adaptive"}
    # the stable instruction prefix is a cached block; the material (user turn) comes after the breakpoint
    assert kw["system"] == [{"type": "text", "text": "sys", "cache_control": {"type": "ephemeral"}}]
    assert kw["messages"] == [{"role": "user", "content": "u"}]
    assert kw["output_config"]["format"]["type"] == "json_schema"
    assert "search_queries" in json.dumps(kw["output_config"]["format"]["schema"])
    log = cfg.runs_log.read_text()
    assert "sk-secret-123" not in log
    rec = json.loads(log.splitlines()[-1])
    assert rec["ok"] and rec["usd"] > 0 and rec["cache_read"] == 0  # spend is on the record, per call


def hosted_cfg(env, secrets="ANTHROPIC_API_KEY=sk-hosted\n"):
    """engine/config.hosted.toml itself (the file the server worker runs), with the key in the temp secrets file."""
    from engine.config import ENGINE_DIR
    (env["tmp"] / "secrets.env").write_text(secrets, encoding="utf-8")
    return Config(ENGINE_DIR / "config.hosted.toml",
                  overrides={"paths": {"kb_dir": (env["root"] / "kb").as_posix(),
                                       "secrets_env": (env["tmp"] / "secrets.env").as_posix()}})


OPUS = ("creative_riff", "creative_punchup", "split", "match_judge", "propose_joke", "label", "link_judge")
SONNET = ("ask_chat", "creative_bridge", "creative_facts", "clips_rank", "ask_plan", "ask_answer")
HAIKU = ("merge_judge", "memo_triage")


def test_hosted_config_routes_every_task_to_the_api_with_the_decided_model(env):
    r = Router(hosted_cfg(env))
    for t in OPUS + SONNET + HAIKU:
        chain = r.resolve(t)
        assert chain and [p for p, _ in chain] == ["anthropic"], t  # never claude_code, never ollama
    assert {r.resolve(t)[0][1] for t in OPUS} == {"claude-opus-5-5"}
    assert {r.resolve(t)[0][1] for t in SONNET} == {"claude-sonnet-5-5"}
    assert {r.resolve(t)[0][1] for t in HAIKU} == {"claude-haiku-5-5"}
    assert "claude_code" not in r.backends and "ollama" not in r.backends
    assert all(r.cfg.task(t).get("offline") for t in OPUS + HAIKU + ("ask_chat", "creative_bridge", "creative_facts"))


def test_hosted_requests_thinking_effort_and_model_per_task(env):
    from engine.budget import Budget
    cfg = hosted_cfg(env)
    seen = []
    AnthropicBackend.client_factory = staticmethod(lambda: fake_client(seen, text='{"ok": true}'))
    AnthropicBackend.budget = Budget(5.0, env["tmp"] / "spend.json")
    try:
        r = Router(cfg)
        for t in ("creative_riff", "ask_chat", "merge_judge"):
            r.complete(t, [{"role": "system", "content": "voice profile"}, {"role": "user", "content": "x"}])
    finally:
        AnthropicBackend.client_factory = None
        AnthropicBackend.budget = None
    riff, ask, judge = seen
    assert riff["model"] == "claude-opus-5-5" and riff["thinking"] == {"type": "adaptive"}
    assert riff["output_config"] == {"effort": "high"}
    assert ask["model"] == "claude-sonnet-5-5" and ask["thinking"] == {"type": "adaptive"}
    assert ask["output_config"] == {"effort": "medium"}
    assert judge["model"] == "claude-haiku-5-5" and "thinking" not in judge  # Haiku: the parameter is omitted
    assert judge["output_config"] == {"effort": "low"}
    for kw in seen:  # every call caches the same stable prefix
        assert kw["system"][-1]["cache_control"] == {"type": "ephemeral"}
    spend = json.loads((env["tmp"] / "spend.json").read_text())
    day = next(iter(spend.values()))
    assert day["calls"] == 3 and set(day["tasks"]) == {"creative_riff", "ask_chat", "merge_judge"}
    assert day["tasks"]["creative_riff"] > day["tasks"]["merge_judge"]  # Opus costs more than Haiku per token


def test_budget_guard_keeps_the_cap_and_goes_red_when_broken(env):
    """The guard is consulted BEFORE the call and charged AFTER it. Over the cap -> BudgetExceeded, no request made,
    and the Router does not slide to another provider. Breaking the guard (a cap that is never reached) is red."""
    from engine.budget import Budget, BudgetExceeded, usage_usd
    cfg = anth_cfg(env, "ANTHROPIC_API_KEY=sk-x\n")
    big = dict(input_tokens=1_000_000, output_tokens=0, cache_creation_input_tokens=0, cache_read_input_tokens=0)
    assert usage_usd("claude-opus-5-5", big) == 4.0 and usage_usd("claude-haiku-5-5", big) == 0.1
    seen = []
    AnthropicBackend.client_factory = staticmethod(lambda: fake_client(seen, usage=big))
    AnthropicBackend.budget = Budget(5.0, env["tmp"] / "spend.json")
    try:
        r = Router(cfg)
        r.complete("label", [{"role": "user", "content": "1"}], SCHEMA)   # $4.00 of $5.00
        r.complete("label", [{"role": "user", "content": "2"}], SCHEMA)   # $8.00: allowed (the cap is checked before)
        assert len(seen) == 2 and AnthropicBackend.budget.status()["over"] is True
        try:
            r.complete("label", [{"role": "user", "content": "3"}], SCHEMA)
        except BudgetExceeded as e:
            assert "over today's AI budget" in str(e) and "$8.00 of $5.00" in str(e)
        else:
            raise AssertionError("the third call must be refused")
        assert len(seen) == 2  # no request went out; the mock provider was NOT tried instead
        # broken guard: a cap of 0 means "no spend allowed", not "unlimited"
        AnthropicBackend.budget = Budget(0.0, env["tmp"] / "spend0.json")
        try:
            r.complete("label", [{"role": "user", "content": "4"}], SCHEMA)
        except BudgetExceeded:
            pass
        else:
            raise AssertionError("a zero cap must refuse")
        assert len(seen) == 2
    finally:
        AnthropicBackend.client_factory = None
        AnthropicBackend.budget = None


def test_budget_default_and_env(env, monkeypatch):
    from engine.budget import Budget
    monkeypatch.delenv("PK_AI_DAILY_USD", raising=False)
    assert Budget(path=env["tmp"] / "s.json").daily_usd == 2.0
    monkeypatch.setenv("PK_AI_DAILY_USD", "0.75")
    b = Budget(path=env["tmp"] / "s.json")
    assert b.daily_usd == 0.75 and b.remaining() == 0.75 and b.status()["over"] is False


def test_anthropic_refusal_and_cut_answers_fail_loudly(env):
    from engine.budget import Budget
    cfg = anth_cfg(env, "ANTHROPIC_API_KEY=sk-x\n")
    seen = []
    AnthropicBackend.budget = Budget(5.0, env["tmp"] / "spend.json")
    try:
        for stop in ("refusal", "max_tokens"):
            AnthropicBackend.client_factory = staticmethod(lambda stop=stop: fake_client(seen, stop=stop))
            c = Router(cfg).complete("label", [{"role": "user", "content": "q"}], SCHEMA)
            assert c.provider == "mock"  # fell through to the next provider, nothing invented
    finally:
        AnthropicBackend.client_factory = None
        AnthropicBackend.budget = None
    errs = [json.loads(l).get("error", "") for l in cfg.runs_log.read_text().splitlines()]
    assert any("refusal" in e for e in errs) and any("max_tokens" in e for e in errs)


def cc_cfg(env, enabled=True, offline=True):
    return Config(env["toml"], overrides={
        "providers": {"claude_code": {"kind": "claude_code", "command": "claude", "enabled": enabled,
                                      "concurrency": 1, "backoff_s": [0]}},
        "tasks": {"label": {"providers": ["claude_code", "mock"], "offline": offline,
                            "models": {"claude_code": "claude-opus-5-5", "mock": "mock-1"}}}})


def test_claude_code_off_by_default_and_never_in_request_path(env):
    ClaudeCodeBackend.runner = staticmethod(lambda a, t, to: (0, "{}", ""))
    try:
        assert Router(cc_cfg(env, enabled=False)).resolve("label")[0][0] == "mock"
        assert Router(cc_cfg(env, offline=False)).resolve("label")[0][0] == "mock"
        assert Router(cc_cfg(env)).resolve("label", request_path=True)[0][0] == "mock"
        assert Router(cc_cfg(env)).resolve("label")[0][0] == "claude_code"
    finally:
        ClaudeCodeBackend.runner = None


def test_claude_code_json_and_rate_limit_fallback(env):
    calls = []

    def ok(argv, text, timeout):
        calls.append((argv, text))
        return 0, json.dumps({"type": "result", "is_error": False, "result": '{"search_queries": ["x"]}',
                              "usage": {"input_tokens": 3, "cache_read_input_tokens": 10, "output_tokens": 4}}), ""

    ClaudeCodeBackend.runner = staticmethod(ok)
    try:
        c = Router(cc_cfg(env)).complete("label", [{"role": "user", "content": "q"}], SCHEMA)
    finally:
        ClaudeCodeBackend.runner = None
    assert c.provider == "claude_code" and c.tokens_in == 13 and c.tokens_out == 4
    argv, text = calls[0]
    assert argv[1:4] == ["-p", "--output-format", "json"] and "JSON Schema" in text

    def limited(argv, text, timeout):
        calls.append("limited")
        return 1, json.dumps({"is_error": True, "result": "Claude usage limit reached", "api_error_status": 429}), ""

    calls.clear()
    ClaudeCodeBackend.runner = staticmethod(limited)
    try:
        c = Router(cc_cfg(env)).complete("label", [{"role": "user", "content": "q"}], SCHEMA)
    finally:
        ClaudeCodeBackend.runner = None
    assert calls == ["limited", "limited"]  # one back-off retry, then fall back
    assert c.provider == "mock"
