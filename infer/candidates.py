"""Review aid for the judge: nearest catalog units for each segment of a set.
Candidates only (bge-small); the match itself is a judgment in judge/<set>.json.
Usage: python -m infer.candidates <set_dir> [k]"""
import sys

import numpy as np

from .catalog import Catalog
from .embed import Embedder
from .segments import load_judgment, resolve
from .transcript import Transcript


def unit_matrix(cat, emb):
    uids = [u for u in cat.units]
    return uids, np.stack([emb.unit(u, cat.units[u]["text"]) for u in uids])


def main():
    set_dir = sys.argv[1]
    k = int(sys.argv[2]) if len(sys.argv) > 2 else 8
    cat, emb = Catalog(), Embedder()
    uids, U = unit_matrix(cat, emb)
    tr = Transcript(set_dir)
    for s, a, b in resolve(tr, load_judgment(set_dir)):
        text = tr.span(a, b)
        q = emb.texts([text])[0]
        sc = U @ q
        top = np.argsort(-sc)[:k]
        print(f"\n## {s['title']} [{s['status']}] {tr.words[a]['s_ms']/1000:.0f}s :: {text[:110]!r}")
        for j in top:
            u = cat.units[uids[j]]
            jk = cat.unit_joke.get(uids[j], "-")
            print(f"  {sc[j]:.3f} {uids[j]} {jk} '{u['doc_title']}' {u['created'][:10]} :: {u['text'][:110]!r}")


if __name__ == "__main__":
    main()
