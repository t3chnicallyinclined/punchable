"""Verbatim quote checks.

A quote is kept only if it is an exact substring of its source text. The one
allowance: when a model changed only typography (curly vs straight quotes and
apostrophes, dash style, runs of whitespace, an ellipsis character), the quote is
REPLACED by the exact source substring, so what is stored is still byte-for-byte
from the source. Any change to a word drops the quote.
"""
from __future__ import annotations

import re

_TYPO = str.maketrans({"‘": "'", "’": "'", "‚": "'", "‛": "'", "′": "'",
                       "“": '"', "”": '"', "„": '"', "″": '"',
                       "–": "-", "—": "-", "−": "-", " ": " "})


def _norm_with_map(s: str):
    """Typography-normalised string + index map back into s."""
    out, idx = [], []
    i, n = 0, len(s)
    prev_space = False
    while i < n:
        ch = s[i]
        if ch == "…":
            for c in "...":
                out.append(c)
                idx.append(i)
            prev_space = False
            i += 1
            continue
        c = ch.translate(_TYPO)
        if c.isspace():
            if not prev_space:
                out.append(" ")
                idx.append(i)
            prev_space = True
        else:
            out.append(c)
            idx.append(i)
            prev_space = False
        i += 1
    return "".join(out), idx


def find_verbatim(quote: str, source: str) -> str | None:
    """The exact source substring for `quote`, or None if it is not in the source."""
    if not quote or not source:
        return None
    q = quote.strip()
    if len(q) < 3:
        return None
    if q in source:
        return q
    # strip wrapping quote marks / trailing ellipsis the model added
    q2 = q.strip("\"'“”‘’").strip()
    q2 = re.sub(r"^(\.\.\.|…)\s*|\s*(\.\.\.|…)$", "", q2)
    if len(q2) >= 3 and q2 in source:
        return q2
    ns, idx = _norm_with_map(source)
    nq, _ = _norm_with_map(q2)
    nq = nq.strip()
    if len(nq) < 3:
        return None
    k = ns.find(nq)  # case is not normalised: a case change counts as a word change
    if k < 0:
        return None
    a = idx[k]
    b = idx[k + len(nq) - 1] + 1
    return source[a:b]


def check_quotes(items: list[dict], sources: dict[str, str], *, quote_key="quote", node_key="node"):
    """Split evidence items into (kept, dropped). Kept items carry the exact source text."""
    kept, dropped = [], []
    for it in items:
        src = sources.get(it.get(node_key))
        exact = find_verbatim(it.get(quote_key) or "", src or "")
        if exact is None:
            dropped.append(dict(it, why="not found verbatim in " + str(it.get(node_key))
                                if src is not None else "unknown source " + str(it.get(node_key))))
        else:
            kept.append(dict(it, **{quote_key: exact}))
    return kept, dropped


def strip_unverified_inline_quotes(text: str, sources: list[str], min_words: int = 4):
    """Remove "double-quoted" passages in prose that appear in none of the sources."""
    flags = []

    def repl(m):
        q = m.group(2)
        if len(q.split()) < min_words:
            return m.group(0)
        if any(find_verbatim(q, s) for s in sources):
            return m.group(0)
        flags.append(q)
        return "[quote removed: not verbatim in the material]"

    out = re.sub(r'(")([^"\n]{3,600})(")', repl, text or "")
    out = re.sub(r"(“)([^“”\n]{3,600})(”)", repl, out)
    return out, flags


def normalize_ids(text: str) -> str:
    """Models sometimes print ids with look-alike hyphens (U+2010/2011/2013); map them back."""
    return re.sub(r"[‐‑‒–−]", "-", text or "")
