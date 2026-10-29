"""Goldens for pk-embed: a synthetic corpus through the Python it replaces, written for the Rust tests to compare.

Two files in rust/crates/pk-embed/tests/goldens/:
  hash.json   api-dev/search_api.py HashEmbedder over the corpus (no model needed; the Rust port must be equal)
  bge.json    engine/embed.py over the same corpus the way db/embedder.py calls it: the query prefix when `query`,
              `t or " "`, the MAX_CHARS cut; per text the token ids the model saw (unpadded) and the vector.
              Written only when the model is here (PK_BGE_DIR or <PK_CACHE_DIR>/models/bge-small-en-v1.5, from
              tools/kb/fetch_bge.py); the Rust tests skip when it is not.

    PK_BGE_DIR=... python rust/tools/gen_embed_goldens.py        (from the repo root)

Synthetic text only: nothing of the comic's is here.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "api-dev"))
from engine.config import bge_dir_override  # noqa: E402
from engine.embed import MAX_CHARS, QUERY_PREFIX, Embedder  # noqa: E402
from search_api import HashEmbedder  # noqa: E402

OUT = ROOT / "rust" / "crates" / "pk-embed" / "tests" / "goldens"

LONG = ("The parking meter asked for my pronouns before it asked for my card, and I stood there like it was a job "
        "interview. ") * 30          # ~3,300 chars: past MAX_CHARS
DENSE = "a b c d e f g h i j " * 90   # 1,800 chars, ~900 tokens: under MAX_CHARS, over the 512-token truncation

# (text, query): the corpus. ASCII, unicode of several kinds, the long ones, the empty ones, the fingerprint sentence
CORPUS: list[tuple[str, bool]] = [
    ("The quick brown fox jumps over the lazy dog.", False),
    ("Dating apps are a job interview where both people lie about their height.", False),
    ("my cat swiped left", False),
    ("HELLO WORLD, THIS IS ALL CAPS", False),
    ("don't, won't, can't: it's the apostrophes' fault", False),
    ("Call 555-0199 or visit https://example.test/path?q=1&r=2 by 9:30pm", False),
    ("café naïve résumé façade jalapeño", False),
    ("Straße, Größe, weiß — ß stays ß", False),
    ("İstanbul'da bir kedi", False),
    ("東京の猫は左にスワイプした", False),
    ("Привет, мир! Как дела?", False),
    ("مرحبا بالعالم", False),
    ("🐱 swiped left 😂🔥", False),
    ("astral: 𝔘𝔫𝔦𝔠𝔬𝔡𝔢 𝕞𝕒𝕥𝕙 and 🧑‍🚀 zwj", False),
    ("curly ‘quotes’ and “double” ones… with an ellipsis and a non\u00a0breaking space", False),
    ("combining: e\u0301 vs \u00e9, and a\u030a vs \u00e5", False),
    ("tabs\tand\nnewlines\r\nand   runs    of   spaces", False),
    ("[CLS] literal special tokens [SEP] ##piece [PAD] [UNK] [MASK]", False),
    ("control chars \u001c\u001d\u001e\u001f between words", False),
    ("", False),
    ("   ", False),
    ("x", False),
    ("1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20", False),
    ("supercalifragilisticexpialidocious antidisestablishmentarianism", False),
    (LONG, False),
    (DENSE, False),
    ("parking meter pronouns", True),
    ("", True),
    ("what did I say about my cat", True),
    (LONG, True),
    ("The quick brown fox jumps over the lazy dog.", True),
]


def prepared(text: str, query: bool) -> str:
    """What engine/embed.py tokenizes when db/embedder.py calls it: the prefix first, then the MAX_CHARS cut."""
    return ((QUERY_PREFIX if query else "") + (text or " "))[:MAX_CHARS]


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    texts = [t for t, _ in CORPUS]
    hv = HashEmbedder().embed(texts)
    (OUT / "hash.json").write_text(
        json.dumps({"items": [{"text": t, "vec": v} for t, v in zip(texts, hv)]}, ensure_ascii=False), encoding="utf-8")
    print(f"hash.json: {len(texts)} texts")

    d = bge_dir_override()
    if d is None:
        print("bge.json: not written (no model: set PK_BGE_DIR, see tools/kb/fetch_bge.py)")
        return 0
    import onnxruntime
    import tokenizers
    e = Embedder(d)
    # the vectors the way db/embedder.py produces them: one call over the whole corpus, so batching and padding
    # happen as in production; the two query/passage groups are what the API sends in separate calls
    passages = [t for t, q in CORPUS if not q]
    queries = [t for t, q in CORPUS if q]
    pv = iter(e.embed([prepared(t, False) for t in passages]).tolist())
    qv = iter(e.embed([prepared(t, True) for t in queries]).tolist())
    items = []
    for t, q in CORPUS:
        ids = e._tok.encode(prepared(t, q)).ids  # noqa: SLF001  the loaded tokenizer, truncation 512, no batch pad
        items.append({"text": t, "query": q, "ids": ids, "vec": next(qv if q else pv)})
    (OUT / "bge.json").write_text(json.dumps({
        "model": e.name, "onnxruntime": onnxruntime.__version__, "tokenizers": tokenizers.__version__,
        "python": sys.version.split()[0], "items": items}, ensure_ascii=False), encoding="utf-8")
    print(f"bge.json: {len(items)} texts (onnxruntime {onnxruntime.__version__}, tokenizers {tokenizers.__version__})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
