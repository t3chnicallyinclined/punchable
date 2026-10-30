"""Wording comparison between stage transcripts (no model; difflib on normalised words).

phrases(text)    -> verbatim clauses of a transcript (exact substrings)
present(p, text) -> was this phrase said (>= 60% of its words in order, runs of >= 2 words)
"""
from __future__ import annotations

import re
from difflib import SequenceMatcher

from infer.scriptdiff import ntok  # same word normalisation as the infer script diff

_W = re.compile(r"[A-Za-z0-9’']+")
_CLAUSE = re.compile(r"[^.!?,;]+[.!?]?")


def toks(text: str) -> list[str]:
    return [t for t in (ntok(m.group()) for m in _W.finditer(text or "")) if t]


def coverage(phrase: str, text: str) -> float:
    p, t = toks(phrase), toks(text)
    if not p or not t:
        return 0.0
    sm = SequenceMatcher(None, p, t, autojunk=False)
    hit = sum(n for _, _, n in sm.get_matching_blocks() if n >= 2)
    return hit / len(p)


def present(phrase: str, text: str, thr: float = 0.6) -> bool:
    return coverage(phrase, text) >= thr


def phrases(text: str, min_words: int = 5, max_words: int = 14) -> list[str]:
    """Verbatim clauses: split on punctuation; long clauses are cut into word windows."""
    out = []
    for m in _CLAUSE.finditer(text or ""):
        seg = m.group()
        words = list(re.finditer(r"\S+", seg))
        if len(words) < min_words:
            continue
        if len(words) <= max_words:
            out.append(seg[words[0].start():words[-1].end()])
            continue
        for i in range(0, len(words), max_words):
            w = words[i:i + max_words]
            if len(w) >= min_words:
                out.append(seg[w[0].start():w[-1].end()])
    seen, uniq = set(), []
    for p in out:
        k = " ".join(toks(p))
        if k and k not in seen:
            seen.add(k)
            uniq.append(p)
    return uniq


def similar(a: str, b: str) -> float:
    ta, tb = toks(a), toks(b)
    if not ta or not tb:
        return 0.0
    return SequenceMatcher(None, ta, tb, autojunk=False).ratio()
