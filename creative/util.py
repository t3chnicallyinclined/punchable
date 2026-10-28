"""Small shared helpers: time, atomic JSON writes, rounding, trend math, verbatim slices."""
from __future__ import annotations

import hashlib
import json
import math
import os
import re
from datetime import datetime, timezone
from pathlib import Path

_WORD = re.compile(r"\S+")


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def sha8(obj) -> str:
    blob = json.dumps(obj, sort_keys=True, ensure_ascii=False, separators=(",", ":"))
    return hashlib.sha1(blob.encode("utf-8")).hexdigest()[:8]


def slug(s: str, n: int = 40) -> str:
    s = re.sub(r"[^a-z0-9]+", "-", (s or "").lower()).strip("-")
    return s[:n].strip("-") or "untitled"


def dumps(obj) -> str:
    """Canonical JSON for files the API serves (stable key order is the caller's dict order)."""
    return json.dumps(obj, ensure_ascii=False, indent=1)


def write_json(path: Path, obj) -> None:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(dumps(obj))
    os.replace(tmp, path)


def r2(x):
    return None if x is None else round(float(x), 2)


def r1(x):
    return None if x is None else round(float(x), 1)


def mmss(seconds) -> str:
    if seconds is None:
        return "-"
    s = int(round(seconds))
    return f"{s // 60}:{s % 60:02d}"


def mean(xs):
    xs = [x for x in xs if x is not None]
    return sum(xs) / len(xs) if xs else None


def median(xs):
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return None
    m = len(xs) // 2
    return xs[m] if len(xs) % 2 else (xs[m - 1] + xs[m]) / 2


def cv(xs):
    """Coefficient of variation (sample sd / mean). None with < 2 values or a zero mean."""
    xs = [x for x in xs if x is not None]
    if len(xs) < 2:
        return None
    mu = sum(xs) / len(xs)
    if mu <= 0:
        return None
    var = sum((x - mu) ** 2 for x in xs) / (len(xs) - 1)
    return math.sqrt(var) / mu


def trend(values_by_date: list[tuple[str, float]]):
    """Same formula as infer (KB_SCHEMA 'trend'): one mean value per date, least-squares slope over
    date order x (n-1) / mean. up > +0.20, down < -0.20, else flat; None with one date."""
    by = {}
    for d, v in values_by_date:
        if v is not None:
            by.setdefault(d, []).append(v)
    dates = sorted(by)
    vals = [sum(by[d]) / len(by[d]) for d in dates]
    n = len(vals)
    if n < 2:
        return {"trend": None, "rel_change": None, "dates": n}
    mu = sum(vals) / n
    if mu <= 0:
        return {"trend": "flat", "rel_change": 0.0, "dates": n}
    xm = (n - 1) / 2
    slope = sum((i - xm) * (v - mu) for i, v in enumerate(vals)) / sum((i - xm) ** 2 for i in range(n))
    rel = slope * (n - 1) / mu
    t = "up" if rel > 0.20 else "down" if rel < -0.20 else "flat"
    return {"trend": t, "rel_change": round(rel, 2), "dates": n}


def prefix_words(text: str, n: int = 8) -> str:
    """Verbatim prefix of `text` ending on a word boundary (an exact substring)."""
    ms = list(_WORD.finditer(text or ""))
    if not ms:
        return ""
    end = ms[min(n, len(ms)) - 1].end()
    return text[ms[0].start():end]


def suffix_words(text: str, n: int = 8) -> str:
    ms = list(_WORD.finditer(text or ""))
    if not ms:
        return ""
    return text[ms[max(0, len(ms) - n)].start():ms[-1].end()]


def provider_why(ex: BaseException) -> str:
    """What the screen says when a model call fails: a refusal or a cut answer already carries plain words
    (engine.providers.Refusal / CutShort `.plain`); anything else names the error, short."""
    plain = getattr(ex, "plain", None)
    return plain if isinstance(plain, str) and plain else f"{type(ex).__name__}: {str(ex)[:160]}"


def first_line(text: str, max_chars: int = 160) -> str:
    """First non-empty line of `text`, cut on a word boundary; always an exact substring."""
    for line in (text or "").splitlines():
        s = line.strip()
        if len(s.split()) >= 3:
            if len(s) <= max_chars:
                return s
            cut = s[:max_chars]
            sp = cut.rfind(" ")
            return cut[:sp] if sp > 20 else cut
    return prefix_words(text, 12)
