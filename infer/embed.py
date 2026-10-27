"""bge-small embeddings (candidates only). Unit vectors are reused read-only from
tools/kb/cache when present; anything else is embedded via embed.mjs and cached
in infer/cache/emb.npz keyed by sha1(text)."""
import hashlib
import json
import os
import subprocess
import tempfile

import numpy as np

from .paths import CACHE, HERE, KB_VECTORS

DIM = 384


def _h(t):
    return hashlib.sha1(t.encode("utf-8")).hexdigest()


class Embedder:
    def __init__(self):
        os.makedirs(CACHE, exist_ok=True)
        self.path = os.path.join(CACHE, "emb.npz")
        self.mem = {}
        if os.path.exists(self.path):
            z = np.load(self.path)
            self.mem = {k: z[k] for k in z.files}
        self.kb = None
        ids_p = os.path.join(KB_VECTORS, "vectors_ids.json")
        if os.path.exists(ids_p):
            ids = json.load(open(ids_p, encoding="utf-8"))
            m = np.fromfile(os.path.join(KB_VECTORS, "vectors.f32"), dtype=np.float32).reshape(len(ids), DIM)
            self.kb = {k: m[i] for i, k in enumerate(ids)}

    def unit(self, uid, text):
        """Vector for a gdocs unit (reuses the KB cache if it has this unit)."""
        if self.kb is not None and f"unit:{uid}" in self.kb:
            return self.kb[f"unit:{uid}"]
        return self.texts([text])[0]

    def texts(self, texts):
        missing = [t for t in dict.fromkeys(texts) if _h(t) not in self.mem]
        if missing:
            with tempfile.TemporaryDirectory() as td:
                inp, out = os.path.join(td, "in.json"), os.path.join(td, "out.f32")
                json.dump([{"id": str(i), "text": t[:2000]} for i, t in enumerate(missing)], open(inp, "w", encoding="utf-8"))
                subprocess.run(["node", os.path.join(HERE, "embed.mjs"), inp, out], check=True, capture_output=True)
                m = np.fromfile(out, dtype=np.float32).reshape(len(missing), DIM)
            for t, v in zip(missing, m):
                self.mem[_h(t)] = v
            np.savez(self.path, **self.mem)
        return np.stack([self.mem[_h(t)] for t in texts]) if texts else np.zeros((0, DIM), np.float32)
