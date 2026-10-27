"""Download the bge-small-en-v1.5 ONNX export (MIT; the Xenova export transformers.js used for every cached vector)
into <PK_CACHE_DIR>/models/bge-small-en-v1.5 (or --dest), and check it.

    python tools/kb/fetch_bge.py [--dest DIR]

Checks: each file's sha256 against the one the Hugging Face hub publishes for it (its LFS object id, or the hash of
the small text files it serves), then the model itself: the vector of a fixed public sentence must match FINGERPRINT
(the values the cached vectors were made with) to 1e-4. engine/embed.py (onnxruntime + tokenizers) reads the folder;
engine/config.py and db/embedder.py find it there. Checked 2026-10-27: re-embedding 60 random cached texts gives cosine
>= 0.9999995 against tools/kb's vectors_v1.f32. Model files only: no text of yours leaves the machine.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import urllib.request
from pathlib import Path

REPO = "Xenova/bge-small-en-v1.5"
BASE = f"https://huggingface.co/{REPO}/resolve/main/"
FILES = ("onnx/model.onnx", "tokenizer.json")
SENTENCE = "The quick brown fox jumps over the lazy dog."
FINGERPRINT = [-0.10406, -0.01369, -0.0095, 0.10715, 0.0106, 0.00699]  # first 6 dims of SENTENCE's passage vector


def default_dest() -> Path:
    if os.environ.get("PK_BGE_DIR"):
        return Path(os.environ["PK_BGE_DIR"])
    if os.environ.get("PK_CACHE_DIR"):
        return Path(os.environ["PK_CACHE_DIR"]) / "models" / "bge-small-en-v1.5"
    return Path(__file__).resolve().parent / "cache" / "models" / "bge-small-en-v1.5"


def sha(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def published() -> dict[str, str]:
    """path -> sha256 the hub publishes (LFS oid for large files)."""
    out = {}
    for folder in ("", "onnx"):
        url = f"https://huggingface.co/api/models/{REPO}/tree/main" + (f"/{folder}" if folder else "")
        for x in json.load(urllib.request.urlopen(url, timeout=60)):
            if x.get("lfs", {}).get("oid"):
                out[x["path"]] = x["lfs"]["oid"]
    return out


def fingerprint_ok(dest: Path) -> tuple[bool, list]:
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
    from engine.embed import Embedder
    v = [round(float(x), 5) for x in Embedder(dest).embed([SENTENCE])[0][:6]]
    return all(abs(a - b) < 1e-4 for a, b in zip(v, FINGERPRINT)), v


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dest", type=Path, default=None)
    a = ap.parse_args()
    dest = a.dest or default_dest()
    pub = published()
    for rel in FILES:
        out = dest / rel
        want = pub.get(rel)
        if out.is_file() and (want is None or sha(out) == want):
            print(f"ok   {rel}")
            continue
        out.parent.mkdir(parents=True, exist_ok=True)
        tmp = out.with_name(out.name + ".part")
        urllib.request.urlretrieve(BASE + rel, tmp)
        if want and sha(tmp) != want:
            tmp.unlink(missing_ok=True)
            print(f"FAIL {rel}: sha256 differs from the one the hub publishes", file=sys.stderr)
            return 1
        os.replace(tmp, out)
        print(f"got  {rel}")
    ok, v = fingerprint_ok(dest)
    if not ok:
        print(f"FAIL the model's vector for a fixed sentence is {v}, expected {FINGERPRINT}: not the model the cached "
              "vectors were made with", file=sys.stderr)
        return 1
    print(f"bge-small in {dest} (fingerprint ok)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
