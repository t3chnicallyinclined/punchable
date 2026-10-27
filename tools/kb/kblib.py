"""Shared helpers for the KB v0 pipeline (tools/kb)."""
import hashlib
import json
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))
COMEDY = os.environ.get("COMEDY_DIR", os.path.expanduser("~/punchable-data"))
CORPUS = os.path.join(COMEDY, "corpus")
CACHE = os.path.join(os.environ["PK_CACHE_DIR"], "kb") if os.environ.get("PK_CACHE_DIR") else os.path.join(HERE, "cache")
OUT_DIR = os.path.join(COMEDY, "kb", "v0")


def load_units():
    return json.load(open(os.path.join(CORPUS, "gdocs_units.json"), encoding="utf-8"))


def load_spans():
    return json.load(open(os.path.join(CACHE, "perf_spans.json"), encoding="utf-8"))


_SENT = re.compile(r"(?<=[.!?\n])\s+")


def sentences(text):
    """Split into sentence-ish pieces; every piece is a verbatim substring."""
    parts = []
    for p in _SENT.split(text):
        p = p.strip()
        if p:
            parts.append(p)
    return parts or [text.strip()]


def reveal(text, max_chars=220):
    """The last line or two of a unit: where the punch usually is."""
    sents = sentences(text)
    out = sents[-1]
    i = len(sents) - 2
    while len(out) < 60 and i >= 0:
        out = sents[i] + " " + out if (sents[i] + " " + out) in text else out
        if (sents[i] + " " + out) not in text:
            break
        i -= 1
    if len(out) > max_chars:
        out = out[-max_chars:]
        sp = out.find(" ")
        if 0 <= sp < 40:
            out = out[sp + 1:]
    return out if out in text else sents[-1]


def excerpt(text, n=160):
    """A verbatim excerpt of at most ~n chars from the start, cut at a word."""
    t = text.strip()
    if len(t) <= n:
        return t
    cut = t[:n]
    sp = cut.rfind(" ")
    return cut[:sp] if sp > 40 else cut


def clip_quote(q, n=200):
    q = q.strip()
    if len(q) <= n:
        return q
    cut = q[:n]
    sp = cut.rfind(" ")
    return cut[:sp] if sp > 40 else cut


def edge_id(src, kind, dst):
    return "e:" + hashlib.sha1(f"{src}|{kind}|{dst}".encode("utf-8")).hexdigest()[:12]


def slugify(s, maxlen=48):
    s = s.lower().replace("\u2019", "'").replace("&", " and ")
    s = re.sub(r"[^a-z0-9]+", "-", s).strip("-")
    return (s[:maxlen].rstrip("-")) or "untitled"
