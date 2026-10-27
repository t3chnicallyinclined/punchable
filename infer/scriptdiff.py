"""Script diff: the written version unit vs what he said on stage.

on_script_pct = share of the written version's words he said, in order
(difflib alignment on normalised words, runs of >= 2 words only).
dropped = written lines (verbatim from the unit) of which he said < 40%.
added   = stage pieces (verbatim from the transcript) of >= 4 words with < 25%
          of their words from the written version."""
import re
from difflib import SequenceMatcher

NUM = {"0": "zero", "1": "one", "2": "two", "3": "three", "4": "four", "5": "five", "6": "six",
       "7": "seven", "8": "eight", "9": "nine", "10": "ten", "20": "twenty", "30": "thirty"}
_W = re.compile(r"[A-Za-z0-9’']+")


def ntok(t):
    t = t.lower().replace("’", "'")
    t = re.sub(r"[^a-z0-9']", "", t).strip("'")
    return NUM.get(t, t)


def unit_tokens(text):
    """[(norm, c0, c1)] for each word of the unit text."""
    return [(ntok(m.group()), m.start(), m.end()) for m in _W.finditer(text) if ntok(m.group())]


def unit_lines(text):
    """Verbatim written lines: split on newlines and sentence ends."""
    out = []
    for m in re.finditer(r"[^\n.!?]+[.!?]*", text):
        s = m.group().strip()
        if len(_W.findall(s)) >= 3:
            out.append((s, m.start() + m.group().find(s)))
    return out


def align(utoks, stoks):
    sm = SequenceMatcher(None, [t for t, *_ in utoks], stoks, autojunk=False)
    um, sm_ = set(), set()
    for a, b, n in sm.get_matching_blocks():
        if n >= 2:
            um.update(range(a, a + n))
            sm_.update(range(b, b + n))
    return um, sm_


def diff(unit_text, tr, a, b):
    utoks = unit_tokens(unit_text)
    sidx = list(range(a, b))
    stoks = [ntok(tr.words[i]["w"]) for i in sidx]
    um, smatch = align(utoks, stoks)
    pct = round(100 * len(um) / len(utoks)) if utoks else 0
    dropped = []
    for line, off in unit_lines(unit_text):
        idx = [k for k, (_, c0, c1) in enumerate(utoks) if c0 >= off and c1 <= off + len(line)]
        if idx and sum(1 for k in idx if k in um) / len(idx) < 0.4:
            dropped.append(line)
    added = []
    for pa, pb in tr.pieces(a, b):
        ks = [i - a for i in range(pa, pb)]
        if len(ks) >= 4 and sum(1 for k in ks if k in smatch) / len(ks) < 0.25:
            added.append(tr.span(pa, pb))
    return {"on_script_pct": pct, "dropped": dropped[:8], "added": added[:8]}, (len(um), len(utoks), len(smatch), len(stoks))


def dice(unit_text, tr, a, b):
    utoks = unit_tokens(unit_text)
    stoks = [ntok(tr.words[i]["w"]) for i in range(a, b)]
    if not utoks or not stoks:
        return 0.0, 0.0
    um, smatch = align(utoks, stoks)
    return 2 * len(um) / (len(utoks) + len(stoks)), len(um) / len(utoks)
