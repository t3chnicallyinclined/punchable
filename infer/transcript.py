"""Word stream of a set. The canonical text of a set is every Whisper word,
stripped, joined by single spaces. Every verbatim stage quote is a substring of it."""
import json
import os
import re

from .paths import SETS


def norm(tok):
    return re.sub(r"[^a-z0-9']", "", tok.lower().replace("\u2019", "'"))


class Transcript:
    def __init__(self, set_dir):
        tr = json.load(open(os.path.join(SETS, set_dir, "transcript.json"), encoding="utf-8"))
        self.audio_sec = tr.get("audio_sec")
        self.words = []  # dicts: w, s_ms, e_ms, seg, c0, c1 (char offsets in .text)
        parts, pos = [], 0
        for si, seg in enumerate(tr["segments"]):
            for w in seg.get("words", []):
                t = w["w"].strip()
                if not t:
                    continue
                if parts:
                    pos += 1
                self.words.append({"w": t, "s_ms": int(round(w["start"] * 1000)), "e_ms": int(round(w["end"] * 1000)),
                                   "seg": si, "c0": pos, "c1": pos + len(t)})
                parts.append(t)
                pos += len(t)
        self.text = " ".join(parts)
        assert all(self.text[w["c0"]:w["c1"]] == w["w"] for w in self.words)

    def span(self, a, b):
        """Verbatim text of words [a, b)."""
        if b <= a:
            return ""
        return self.text[self.words[a]["c0"]:self.words[b - 1]["c1"]]

    def find(self, anchor, from_seg=0, from_word=0):
        """Index of the word where `anchor` starts (normalised char match), searching
        from segment `from_seg` / word `from_word` onward."""
        target = "".join(norm(t) for t in anchor.split())
        start = next((i for i, w in enumerate(self.words) if w["seg"] >= from_seg and i >= from_word), None)
        if start is None or not target:
            raise ValueError(f"anchor not found: {anchor!r}")
        owner, stream = [], []
        for i in range(start, len(self.words)):
            t = norm(self.words[i]["w"])
            stream.append(t)
            owner.extend([i] * len(t))
        pos = "".join(stream).find(target)
        if pos < 0:
            raise ValueError(f"anchor not found from seg {from_seg}: {anchor!r}")
        # the anchor's end word, too
        return owner[pos], owner[pos + len(target) - 1] + 1

    def word_at_or_before(self, ms):
        """Index of the last word that starts before ms."""
        idx = None
        for i, w in enumerate(self.words):
            if w["s_ms"] < ms:
                idx = i
            else:
                break
        return idx

    def pieces(self, a, b, max_words=18):
        """Split words [a, b) into clause-ish verbatim pieces (for script diff):
        break at sentence punctuation, at segment boundaries, at pauses >= 0.6 s,
        and every max_words words."""
        out, cur = [], a
        for i in range(a, b):
            w = self.words[i]
            nxt = self.words[i + 1] if i + 1 < b else None
            brk = (nxt is None or w["w"][-1:] in ".?!" or nxt["seg"] != w["seg"]
                   or nxt["s_ms"] - w["e_ms"] >= 600 or (i + 1 - cur) >= max_words
                   or (w["w"][-1:] == "," and i + 1 - cur >= 8))
            if brk:
                out.append((cur, i + 1))
                cur = i + 1
        return out
