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


def test_anthropic_request_shape_and_key_not_logged(env):
    cfg = anth_cfg(env, "ANTHROPIC_API_KEY=sk-secret-123\n")
    seen = {}

    class FakeMessages:
        def create(self, **kw):
            seen.update(kw)
            return SimpleNamespace(stop_reason="end_turn", usage=SimpleNamespace(input_tokens=11, output_tokens=7),
                                   content=[SimpleNamespace(type="thinking", thinking="..."),
                                            SimpleNamespace(type="text", text='{"search_queries": ["dad"]}')])

    AnthropicBackend.client_factory = staticmethod(lambda: SimpleNamespace(messages=FakeMessages()))
    try:
        c = Router(cfg).complete("label", [{"role": "system", "content": "sys"}, {"role": "user", "content": "u"}],
                                 SCHEMA)
    finally:
        AnthropicBackend.client_factory = None
    assert c.provider == "anthropic" and c.data == {"search_queries": ["dad"]}
    assert c.tokens_in == 11 and c.tokens_out == 7
    assert seen["model"] == "claude-opus-5-5" and seen["thinking"] == {"type": "adaptive"}
    assert seen["system"] == "sys" and seen["messages"] == [{"role": "user", "content": "u"}]
    assert seen["output_config"]["format"]["type"] == "json_schema"
    assert "search_queries" in json.dumps(seen["output_config"]["format"]["schema"])
    assert "sk-secret-123" not in cfg.runs_log.read_text()


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
