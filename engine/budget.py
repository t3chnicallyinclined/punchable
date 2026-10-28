"""A per-day spend cap for the hosted Anthropic provider (the cost guard of the server worker).

    from engine.budget import Budget
    b = Budget()                       # cap from PK_AI_DAILY_USD (default 2.00), ledger from PK_AI_SPEND_FILE
    b.check()                          # raises BudgetExceeded when today's spend has reached the cap
    b.charge("claude-opus-5-5", usage, task="creative_riff")   # usage = the SDK's response.usage -> USD, recorded

The ledger is one small JSON file keyed by UTC day: {"2026-10-27": {"usd": 0.41, "tasks": {"creative_riff": 0.3, ...},
"calls": 7}}. Days older than 14 are dropped. Prices are first-party Claude API list prices per million tokens
(cached 2026-10-06): input, output, cache write (1.25x input), cache read. A model not in the table is priced as
Opus so a mistake can only over-count. Nothing here ever sees a key.
"""
from __future__ import annotations

import json
import os
import threading
import time
from pathlib import Path

DEFAULT_DAILY_USD = 2.00
# model id prefix -> (input $/MTok, output $/MTok, cache write $/MTok, cache read $/MTok)
PRICES = {
    "claude-opus-5-5": (4.00, 20.00, 5.00, 0.20),
    "claude-sonnet-5-5": (2.00, 10.00, 2.50, 0.20),
    "claude-haiku-5-5": (0.10, 0.50, 0.125, 0.01),
    "claude-opus-5": (5.00, 25.00, 6.25, 0.50),
    "claude-sonnet-5": (2.00, 10.00, 2.50, 0.20),
    "claude-haiku-4-5": (1.00, 5.00, 1.25, 0.10),
}
FALLBACK_PRICE = PRICES["claude-opus-5-5"]


class BudgetExceeded(RuntimeError):
    """Today's cap is spent: the caller keeps its job queued and says why (never silently drops it)."""


def price_of(model: str) -> tuple[float, float, float, float]:
    m = (model or "").lower()
    best = None
    for k, v in PRICES.items():
        if m.startswith(k) and (best is None or len(k) > len(best[0])):
            best = (k, v)
    return best[1] if best else FALLBACK_PRICE


def usage_usd(model: str, usage) -> float:
    """USD for one response from the SDK's usage numbers (missing fields count as 0)."""
    def n(name):
        v = getattr(usage, name, None) if not isinstance(usage, dict) else usage.get(name)
        return int(v or 0)
    pi, po, pw, pr = price_of(model)
    return (n("input_tokens") * pi + n("output_tokens") * po + n("cache_creation_input_tokens") * pw
            + n("cache_read_input_tokens") * pr) / 1_000_000


def today() -> str:
    return time.strftime("%Y-%m-%d", time.gmtime())


class Budget:
    def __init__(self, daily_usd: float | None = None, path: str | Path | None = None):
        env = os.environ.get("PK_AI_DAILY_USD")
        self.daily_usd = float(daily_usd if daily_usd is not None else (env if env else DEFAULT_DAILY_USD))
        p = path or os.environ.get("PK_AI_SPEND_FILE")
        if not p:
            base = os.environ.get("PK_CACHE_DIR") or os.environ.get("PK_WORKSPACE") or str(Path.home() / ".punchable-worker")
            p = Path(base) / "ai_spend.json"
        self.path = Path(p)
        self._lock = threading.Lock()
        # called after every charge with (model, usage, usd, task); worker/metrics.py counts tokens and USD there.
        self.listeners: list = []

    # ------------------------------------------------------------------ ledger
    def _read(self) -> dict:
        try:
            d = json.loads(self.path.read_text(encoding="utf-8"))
            return d if isinstance(d, dict) else {}
        except (OSError, ValueError):
            return {}

    def _write(self, d: dict):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        tmp = self.path.with_name(self.path.name + ".tmp")
        tmp.write_text(json.dumps(d, indent=0, sort_keys=True), encoding="utf-8")
        os.replace(tmp, self.path)

    def spent_today(self) -> float:
        return float((self._read().get(today()) or {}).get("usd") or 0.0)

    def remaining(self) -> float:
        return self.daily_usd - self.spent_today()

    def status(self) -> dict:
        d = self._read().get(today()) or {}
        return {"daily_usd": self.daily_usd, "spent_usd": round(float(d.get("usd") or 0.0), 4),
                "calls": int(d.get("calls") or 0), "tasks": {k: round(v, 4) for k, v in (d.get("tasks") or {}).items()},
                "over": self.remaining() <= 0}

    def reason(self) -> str:
        return (f"over today's AI budget (${self.spent_today():.2f} of ${self.daily_usd:.2f}); "
                "queued jobs run again after midnight UTC or a higher PK_AI_DAILY_USD")

    # ------------------------------------------------------------------ guard
    def check(self):
        if self.daily_usd <= 0 or self.remaining() <= 0:
            raise BudgetExceeded(self.reason())

    def charge(self, model: str, usage, task: str = "?") -> float:
        usd = usage_usd(model, usage)
        with self._lock:
            d = self._read()
            day = d.setdefault(today(), {"usd": 0.0, "calls": 0, "tasks": {}})
            day["usd"] = float(day.get("usd") or 0.0) + usd
            day["calls"] = int(day.get("calls") or 0) + 1
            tasks = day.setdefault("tasks", {})
            tasks[task] = float(tasks.get(task) or 0.0) + usd
            for k in sorted(d):
                if len(d) > 14 and k != today():
                    d.pop(k)
                    if len(d) <= 14:
                        break
            self._write(d)
        for fn in list(self.listeners):
            try:
                fn(model, usage, usd, task)
            except Exception:  # noqa: BLE001  a metrics hook never fails a job
                pass
        return usd


_default: Budget | None = None


def default_budget() -> Budget:
    global _default
    if _default is None:
        _default = Budget()
    return _default
