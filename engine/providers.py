"""One model-provider interface for every engine task.

    from engine.providers import complete
    c = complete("label", [{"role": "system", "content": "..."}, {"role": "user", "content": "..."}],
                 schema={...}, run_id="auto-...", fewshot_sha="...")
    c.data   # parsed + schema-validated JSON when a schema was given
    c.text   # raw text

Routing is per task in config.toml ([tasks.<name>].providers, first available
wins, falling through on errors). Backends:
  openai_compatible  Ollama (default), LM Studio, vLLM, any custom endpoint
  anthropic          Claude via the official SDK; only when ANTHROPIC_API_KEY is in
                     the environment or in paths.secrets_env. Streamed, structured
                     output via output_config.format, adaptive thinking, the system
                     prefix cached, a daily USD cap (engine/budget.py). The hosted
                     worker's only provider (engine/config.hosted.toml, ENGINE_CONFIG).
  claude_code        the official Claude Code CLI in headless mode (`claude -p
                     --output-format json`) on the comic's own PC. Off by default,
                     opt-in per task, offline/batch tasks only, concurrency 1-2,
                     rate limits back off then fall through to the next provider.

Every call (success or failure) is appended to <kb_dir>/runs.jsonl with run_id,
task, provider, model, prompt_sha, fewshot_sha, tokens_in/out and ms. Keys are
never logged.
"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import threading
import time
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path

from .config import Config


class ProviderError(RuntimeError):
    pass


class RateLimited(ProviderError):
    pass


class Refusal(ProviderError):
    """The model declined (stop_reason "refusal"). The message IS the plain words the comic sees; `category` and
    `explanation` are the API's stop_details. Never silent: the router tries the task's `fallbacks` models once each,
    then raises this (not a generic "failed on all providers") so the words survive to the screen."""

    def __init__(self, category: str | None = None, explanation: str | None = None):
        self.category, self.explanation = category, explanation
        why = (explanation or "").strip().rstrip(".")
        tail = f": {why}" if why else (f" ({category})" if category else "")
        super().__init__(f"Claude declined this one{tail}. Try rewording.")

    @property
    def plain(self) -> str:
        return str(self)


class CutShort(ProviderError):
    """stop_reason "max_tokens": the answer was cut off. Plain words, same handling as a refusal."""

    def __init__(self, max_tokens: int):
        self.max_tokens = max_tokens
        super().__init__(f"Claude ran out of room on this one (max_tokens {max_tokens}): the answer was cut short. "
                         "Try a shorter idea.")

    @property
    def plain(self) -> str:
        return str(self)


class NoProvider(ProviderError):
    pass


@dataclass
class Completion:
    text: str
    data: object = None
    task: str = ""
    provider: str = ""
    model: str = ""
    tokens_in: int = 0
    tokens_out: int = 0
    ms: int = 0
    call_id: str = ""
    prompt_sha: str = ""
    attempts: list = field(default_factory=list)


def sha(obj) -> str:
    s = obj if isinstance(obj, str) else json.dumps(obj, sort_keys=True, ensure_ascii=False)
    return hashlib.sha256(s.encode("utf-8")).hexdigest()[:16]


def read_secret(cfg: Config, name: str) -> str | None:
    """Env var first, then NAME=value in paths.secrets_env. Never logged."""
    v = os.environ.get(name)
    if v and v.strip():
        return v.strip()
    p = cfg.paths.get("secrets_env")
    try:
        if p and p.is_file():
            for line in p.read_text(encoding="utf-8").splitlines():
                line = line.strip()
                if line.startswith("export "):
                    line = line[7:]
                if "=" in line and not line.startswith("#"):
                    k, val = line.split("=", 1)
                    if k.strip() == name and val.strip().strip('"').strip("'"):
                        return val.strip().strip('"').strip("'")
    except OSError:
        pass
    return None


def parse_json(text: str):
    """Parse a JSON object from model text (tolerates ```json fences and leading prose)."""
    t = (text or "").strip()
    if t.startswith("```"):
        t = t.split("\n", 1)[1] if "\n" in t else t
        if t.rstrip().endswith("```"):
            t = t.rstrip()[:-3]
    try:
        return json.loads(t)
    except ValueError:
        a, b = t.find("{"), t.rfind("}")
        if a >= 0 and b > a:
            return json.loads(t[a:b + 1])
        raise


def validate(data, schema):
    import jsonschema
    jsonschema.validate(data, schema)


def schema_instructions(schema: dict) -> str:
    return ("Reply with ONE JSON object only, no prose and no code fences, matching this JSON Schema:\n"
            + json.dumps(schema, ensure_ascii=False))


# ---------------------------------------------------------------------------
# backends
# ---------------------------------------------------------------------------
class Backend:
    kind = "?"

    def __init__(self, name: str, pcfg: dict, cfg: Config):
        self.name, self.pcfg, self.cfg = name, pcfg, cfg

    def available(self, task_cfg: dict, request_path: bool) -> bool:
        return True

    def call(self, model: str, messages: list[dict], schema: dict | None, task_cfg: dict):
        """-> (text, tokens_in, tokens_out)"""
        raise NotImplementedError


class OpenAICompatBackend(Backend):
    kind = "openai_compatible"
    _started: set = set()

    def _base_url(self):
        return self.pcfg.get("base_url") or (os.environ.get(self.pcfg["base_url_env"]) if self.pcfg.get("base_url_env") else None)

    def _key(self):
        if self.pcfg.get("api_key_env"):
            k = read_secret(self.cfg, self.pcfg["api_key_env"])
            if k:
                return k
        return self.pcfg.get("api_key") or "none"

    def _alive(self, url) -> bool:
        import httpx
        try:
            return httpx.get(url.rstrip("/") + "/models", timeout=3).status_code < 500
        except Exception:
            return False

    def available(self, task_cfg, request_path):
        url = self._base_url()
        if not url:
            return False
        if self._alive(url):
            return True
        cmd = self.pcfg.get("autostart")
        if cmd and self.name not in OpenAICompatBackend._started:
            OpenAICompatBackend._started.add(self.name)
            flags = getattr(subprocess, "CREATE_NO_WINDOW", 0) | getattr(subprocess, "DETACHED_PROCESS", 0)
            try:
                subprocess.Popen(cmd.split(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=flags)
            except OSError:
                return False
            for _ in range(30):
                time.sleep(1)
                if self._alive(url):
                    return True
        return False

    def call(self, model, messages, schema, task_cfg):
        from openai import OpenAI, RateLimitError
        client = OpenAI(base_url=self._base_url(), api_key=self._key(), timeout=task_cfg.get("timeout_s", 900))
        kw = {"model": model, "messages": messages, "temperature": task_cfg.get("temperature", 0)}
        if schema:
            kw["response_format"] = {"type": "json_schema",
                                     "json_schema": {"name": "out", "schema": schema, "strict": True}}
        if task_cfg.get("reasoning_effort"):
            kw["reasoning_effort"] = task_cfg["reasoning_effort"]
        try:
            r = client.chat.completions.create(**kw)
        except RateLimitError as e:
            raise RateLimited(str(e)) from e
        text = r.choices[0].message.content or ""
        u = r.usage
        return text, (u.prompt_tokens if u else 0), (u.completion_tokens if u else 0)


EFFORTS = ("low", "medium", "high", "xhigh", "max")


class AnthropicBackend(Backend):
    """Claude through the official SDK (the hosted worker's only provider, engine/config.hosted.toml).

    - streams every request (`messages.stream` + `get_final_message()`): long drafts never hit the HTTP timeout;
    - the system messages become text blocks with `cache_control` on the FIRST one (the voice profile + craft rules,
      creative/voice.py: stable across every task, long enough to cache), on the last one (the task's own line) and,
      when there are three, on the middle one (the learned section, creative/learned.py: the lines he kept, cached
      as the prefix behind the profile), so the prefix is served from the prompt cache on the next call; the volatile
      material is in the user turn, after the breakpoints. The prefix must reach the model's minimum (512 tokens on
      Opus 5.5 / Sonnet 5.5) or the cache silently never hits: a system block on its own does not; the profile block
      does;
    - a refusal (stop_reason "refusal") raises Refusal, a cut answer (max_tokens) CutShort: plain words the comic can
      read; the router retries each once on the task's `fallbacks` models (config: `fallbacks = ["default"]`, i.e.
      the default task's model, Sonnet on the hosted config) before giving up, and never swallows them;
    - `thinking: {type: "adaptive"}` on Opus and Sonnet; Haiku is sent without the parameter (it thinks adaptively by
      default), and a task may set `thinking = "none"` to omit it anywhere;
    - `effort` per task (`[tasks.x] effort = "low"|"medium"|"high"|"xhigh"|"max"`) -> output_config.effort;
    - the daily cost guard (engine/budget.py): refused before the call when today's cap is spent, charged after it
      from the SDK's usage numbers, per task.
    """
    kind = "anthropic"
    client_factory = None  # tests inject a fake client here
    budget = None          # tests inject a Budget; else engine.budget.default_budget() (PK_AI_DAILY_USD)

    def _key(self):
        return read_secret(self.cfg, self.pcfg.get("api_key_env", "ANTHROPIC_API_KEY"))

    def available(self, task_cfg, request_path):
        return bool(self._key())

    def _budget(self):
        if self.budget is not None:
            return self.budget
        if self.pcfg.get("budget", True) is False:
            return None
        from .budget import default_budget
        return default_budget()

    @staticmethod
    def thinks(model: str, task_cfg: dict | None, pcfg: dict) -> bool:
        if (task_cfg or {}).get("thinking", pcfg.get("thinking", "adaptive")) != "adaptive":
            return False
        return not model.lower().startswith("claude-haiku")

    def build_request(self, model, messages, schema, task_cfg: dict | None = None):
        task_cfg = task_cfg or {}
        sys_blocks = [{"type": "text", "text": m["content"]} for m in messages if m["role"] == "system" and m["content"]]
        msgs = [{"role": m["role"], "content": m["content"]} for m in messages if m["role"] != "system"]
        kw = {"model": model, "max_tokens": int(task_cfg.get("max_tokens") or self.pcfg.get("max_tokens", 16000)),
              "messages": msgs}
        if sys_blocks:
            if self.pcfg.get("cache", True):
                # the first block is the stable prefix shared by every task (the voice profile); the last is the
                # task's own line: two breakpoints (of the four allowed), so a different task still reuses the first.
                # With three blocks the middle one is the learned section (creative/learned.py: changes when he keeps
                # a line): its own breakpoint, so riff and punch-up share the [profile + learned] prefix too
                sys_blocks[0]["cache_control"] = {"type": "ephemeral"}
                sys_blocks[-1]["cache_control"] = {"type": "ephemeral"}
                if len(sys_blocks) >= 3:
                    sys_blocks[1]["cache_control"] = {"type": "ephemeral"}
            kw["system"] = sys_blocks
        if self.thinks(model, task_cfg, self.pcfg):
            kw["thinking"] = {"type": "adaptive"}
        effort = task_cfg.get("effort")
        if effort:
            if effort not in EFFORTS:
                raise ProviderError(f"effort must be one of {EFFORTS}, not {effort!r}")
            kw["output_config"] = {"effort": effort}
        if schema:
            from anthropic.lib._parse._transform import transform_schema
            kw.setdefault("output_config", {})["format"] = {"type": "json_schema", "schema": transform_schema(schema)}
        return kw

    def call(self, model, messages, schema, task_cfg):
        import anthropic
        budget = self._budget()
        if budget is not None:
            budget.check()  # BudgetExceeded: the worker keeps the job queued and says why
        client = self.client_factory() if self.client_factory else anthropic.Anthropic(api_key=self._key())
        kw = self.build_request(model, messages, schema, task_cfg)
        try:
            if self.pcfg.get("stream", True):
                with client.messages.stream(**kw) as stream:
                    r = stream.get_final_message()
            else:
                r = client.messages.create(**kw)
        except anthropic.RateLimitError as e:
            raise RateLimited(str(e)) from e
        u = r.usage
        extra = {"cache_read": int(getattr(u, "cache_read_input_tokens", 0) or 0),
                 "cache_write": int(getattr(u, "cache_creation_input_tokens", 0) or 0)}
        if budget is not None:
            extra["usd"] = round(budget.charge(model, u, task=task_cfg.get("_task", "?")), 6)
        if getattr(r, "stop_reason", None) == "refusal":
            det = getattr(r, "stop_details", None)
            raise Refusal(getattr(det, "category", None) if det is not None else None,
                          getattr(det, "explanation", None) if det is not None else None)
        if getattr(r, "stop_reason", None) == "max_tokens":
            raise CutShort(int(kw["max_tokens"]))
        text = "".join(b.text for b in r.content if getattr(b, "type", None) == "text")
        return text, u.input_tokens, u.output_tokens, extra


class ClaudeCodeBackend(Backend):
    """Personal use on the comic's own machine via the official CLI; not for serving other users."""
    kind = "claude_code"
    _sems: dict = {}
    runner = None  # tests inject: callable(argv, stdin_text, timeout) -> (returncode, stdout, stderr)

    def _sem(self):
        n = max(1, min(2, int(self.pcfg.get("concurrency", 1))))
        return ClaudeCodeBackend._sems.setdefault(self.name, threading.BoundedSemaphore(n))

    def available(self, task_cfg, request_path):
        if not self.pcfg.get("enabled", False):
            return False
        if request_path or not task_cfg.get("offline", False):
            return False  # batch/offline tasks only, never the API request path
        return bool(self.runner or shutil.which(self.pcfg.get("command", "claude")))

    def argv(self, model):
        a = [shutil.which(self.pcfg.get("command", "claude")) or self.pcfg.get("command", "claude"),
             "-p", "--output-format", "json", "--tools", "", "--no-session-persistence", "--strict-mcp-config"]
        if model:
            a += ["--model", model]
        return a

    @staticmethod
    def prompt(messages, schema):
        parts = []
        for m in messages:
            tag = {"system": "INSTRUCTIONS", "user": "INPUT", "assistant": "PREVIOUS ANSWER"}.get(m["role"], m["role"])
            parts.append(f"## {tag}\n{m['content']}")
        if schema:
            parts.append("## OUTPUT\n" + schema_instructions(schema))
        return "\n\n".join(parts)

    def _run(self, argv, text, timeout):
        if self.runner:
            return self.runner(argv, text, timeout)
        p = subprocess.run(argv, input=text, capture_output=True, text=True, encoding="utf-8", timeout=timeout)
        return p.returncode, p.stdout, p.stderr

    def call(self, model, messages, schema, task_cfg):
        text = self.prompt(messages, schema)
        waits = list(self.pcfg.get("backoff_s", [30, 120]))
        while True:
            with self._sem():
                rc, out, err = self._run(self.argv(model), text, int(self.pcfg.get("timeout_s", 600)))
            try:
                doc = json.loads(out)
            except ValueError:
                doc = {"is_error": True, "result": (out or "")[-300:] + (err or "")[-300:]}
            blob = (str(doc.get("result", "")) + " " + str(doc.get("api_error_status", "")) + " " + (err or "")).lower()
            limited = doc.get("api_error_status") in (429, "429") or "rate limit" in blob or "usage limit" in blob \
                or "limit reached" in blob
            if rc == 0 and not doc.get("is_error"):
                u = doc.get("usage") or {}
                tin = int(u.get("input_tokens", 0)) + int(u.get("cache_creation_input_tokens", 0)) + \
                    int(u.get("cache_read_input_tokens", 0))
                return doc.get("result", ""), tin, int(u.get("output_tokens", 0))
            if limited and waits:
                time.sleep(waits.pop(0))
                continue
            if limited:
                raise RateLimited("claude_code rate limited")
            raise ProviderError(f"claude_code failed rc={rc}: {blob[:200]}")


BACKEND_KINDS = {"openai_compatible": OpenAICompatBackend, "anthropic": AnthropicBackend,
                 "claude_code": ClaudeCodeBackend}


def register_kind(kind: str, cls):
    BACKEND_KINDS[kind] = cls


# ---------------------------------------------------------------------------
# router
# ---------------------------------------------------------------------------
class Router:
    def __init__(self, cfg: Config | None = None, log_path: Path | None = None):
        self.cfg = cfg or Config()
        self.log_path = Path(log_path) if log_path else self.cfg.runs_log
        self._lock = threading.Lock()
        self.backends: dict[str, Backend] = {}
        for name, pcfg in self.cfg.providers.items():
            cls = BACKEND_KINDS.get(pcfg.get("kind"))
            if cls:
                self.backends[name] = cls(name, pcfg, self.cfg)
        self.calls = []  # in-process record of this router's calls (for stage accounting)

    def resolve(self, task: str, request_path: bool = False) -> list[tuple[str, str]]:
        """[(provider, model)] in preference order, available ones only."""
        tcfg = self.cfg.task(task)
        out = []
        for name in tcfg.get("providers", []):
            b = self.backends.get(name)
            model = (tcfg.get("models") or {}).get(name)
            if b and model and b.available(tcfg, request_path):
                out.append((name, model))
        return out

    def describe(self, task: str) -> str:
        r = self.resolve(task)
        return f"{r[0][0]}:{r[0][1]}" if r else "none"

    def complete(self, task: str, messages: list[dict], schema: dict | None = None, *, run_id: str | None = None,
                 fewshot_sha: str | None = None, request_path: bool = False) -> Completion:
        tcfg = self.cfg.task(task)
        tcfg["_task"] = task  # backends that account spend per task read it (AnthropicBackend)
        prompt_sha = sha({"messages": messages, "schema": schema})
        chain = self.resolve(task, request_path)
        if not chain:
            raise NoProvider(f"no available provider for task {task!r}")
        attempts, last = [], None
        for name, model in chain:
            b = self.backends[name]
            for m in self.models_for(tcfg, name, model):
                got = self._attempt(b, name, m, task, messages, schema, tcfg, run_id, fewshot_sha, prompt_sha, attempts)
                if isinstance(got, Completion):
                    return got
                last = got
                if not isinstance(got, (Refusal, CutShort)):
                    break  # provider down, rate limited, bad JSON twice: the next provider, not the next model
        if isinstance(last, (Refusal, CutShort)):
            raise last  # the plain words survive to the caller (and to the screen)
        raise ProviderError(f"task {task!r} failed on all providers: {attempts}; last: {last}")

    def models_for(self, tcfg: dict, provider: str, model: str) -> list[str]:
        """The model, then the task's `fallbacks` (task names whose model on this provider is tried once each after a
        refusal or a cut answer; `["default"]` = the default task's model). Duplicates and the model itself skipped."""
        out = [model]
        for t in tcfg.get("fallbacks") or []:
            fm = (self.cfg.task(str(t)).get("models") or {}).get(provider)
            if fm and fm not in out:
                out.append(fm)
        return out

    def _attempt(self, b: Backend, name: str, model: str, task: str, messages: list[dict], schema, tcfg: dict,
                 run_id, fewshot_sha, prompt_sha, attempts: list):
        """One model on one provider, with one retry on unparseable / invalid JSON. Returns the Completion, or the
        exception that ended it (BudgetExceeded is raised: the caller keeps the job queued)."""
        from .budget import BudgetExceeded
        msgs = list(messages)
        last = None
        for attempt in range(2):
            t0 = time.perf_counter()
            call_id = uuid.uuid4().hex[:12]
            extra = None
            try:
                got = b.call(model, msgs, schema, tcfg)
                text, tin, tout = got[0], got[1], got[2]
                extra = got[3] if len(got) > 3 else None  # {"usd", "cache_read", "cache_write"}: the SDK's usage
            except BudgetExceeded as e:  # the day's cap: no other provider is tried, the caller keeps the job
                ms = int((time.perf_counter() - t0) * 1000)
                self._log(call_id, run_id, task, name, model, prompt_sha, fewshot_sha, 0, 0, ms, False,
                          f"BudgetExceeded: {str(e)[:200]}")
                raise
            except Exception as e:  # provider down, rate limited, a refusal, a cut answer
                ms = int((time.perf_counter() - t0) * 1000)
                self._log(call_id, run_id, task, name, model, prompt_sha, fewshot_sha, 0, 0, ms, False,
                          f"{type(e).__name__}: {str(e)[:200]}")
                attempts.append((name, type(e).__name__))
                return e
            ms = int((time.perf_counter() - t0) * 1000)
            data, err = None, None
            if schema:
                try:
                    data = parse_json(text)
                    validate(data, schema)
                except Exception as e:
                    err = f"invalid JSON: {str(e)[:200]}"
            self._log(call_id, run_id, task, name, model, prompt_sha, fewshot_sha, tin, tout, ms, err is None, err, extra)
            if err is None:
                return Completion(text=text, data=data, task=task, provider=name, model=model, tokens_in=tin,
                                  tokens_out=tout, ms=ms, call_id=call_id, prompt_sha=prompt_sha, attempts=attempts)
            attempts.append((name, "invalid_json"))
            last = ProviderError(err)
            msgs = list(messages) + [{"role": "assistant", "content": text[:4000]},
                                     {"role": "user", "content": f"That was not valid: {err}. "
                                                                 + schema_instructions(schema)}]
        return last

    def _log(self, call_id, run_id, task, provider, model, prompt_sha, fewshot_sha, tin, tout, ms, ok, error,
             extra: dict | None = None):
        rec = {"ts": datetime.now(timezone.utc).isoformat(timespec="seconds"), "call_id": call_id, "run_id": run_id,
               "task": task, "provider": provider, "model": model, "prompt_sha": prompt_sha,
               "fewshot_sha": fewshot_sha, "tokens_in": tin, "tokens_out": tout, "ms": ms, "ok": ok}
        if extra:
            rec.update({k: v for k, v in extra.items() if k in ("usd", "cache_read", "cache_write")})
        if error:
            rec["error"] = error
        with self._lock:
            self.calls.append(rec)
            self.log_path.parent.mkdir(parents=True, exist_ok=True)
            with open(self.log_path, "a", encoding="utf-8") as f:
                f.write(json.dumps(rec, ensure_ascii=False) + "\n")


_default: Router | None = None


def default_router() -> Router:
    global _default
    if _default is None:
        _default = Router()
    return _default


def complete(task: str, messages: list[dict], schema: dict | None = None, **kw) -> Completion:
    return default_router().complete(task, messages, schema, **kw)
