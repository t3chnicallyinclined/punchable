"""bge-small-en-v1.5 embeddings in Python (onnxruntime + tokenizers).

Uses the same Xenova ONNX export and CLS pooling as tools/kb/kb_embed.mjs, so new
vectors are directly comparable with tools/kb/cache/vectors.f32 (checked: cosine
0.99999 on the cached rows). For CANDIDATES and search only, never a verdict.

VectorIndex = the read-only tools/kb cache + an engine-side cache of extra texts
(<kb_dir>/engine_cache/vectors.*) keyed by id and text hash.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

import numpy as np

DIM = 384
QUERY_PREFIX = "Represent this sentence for searching relevant passages: "
MODEL_NAME = "bge-small-en-v1.5"


class Embedder:
    name = MODEL_NAME

    def __init__(self, bge_dir: Path):
        self.bge_dir = Path(bge_dir)
        self._sess = None
        self._tok = None

    def _load(self):
        if self._sess is None:
            import onnxruntime as ort
            from tokenizers import Tokenizer
            tok = Tokenizer.from_file(str(self.bge_dir / "tokenizer.json"))
            tok.enable_truncation(512)
            tok.enable_padding()
            self._tok = tok
            self._sess = ort.InferenceSession(str(self.bge_dir / "onnx" / "model.onnx"),
                                              providers=["CPUExecutionProvider"])

    def embed(self, texts: list[str], batch: int = 8) -> np.ndarray:
        if not texts:
            return np.zeros((0, DIM), np.float32)
        self._load()
        order = sorted(range(len(texts)), key=lambda i: len(texts[i]))
        out = np.zeros((len(texts), DIM), np.float32)
        for s in range(0, len(order), batch):
            ids = order[s:s + batch]
            enc = self._tok.encode_batch([texts[i] or " " for i in ids])
            feed = {"input_ids": np.array([e.ids for e in enc], np.int64),
                    "attention_mask": np.array([e.attention_mask for e in enc], np.int64),
                    "token_type_ids": np.array([e.type_ids for e in enc], np.int64)}
            v = self._sess.run(None, feed)[0][:, 0]
            v = v / np.linalg.norm(v, axis=1, keepdims=True)
            out[ids] = v
        return out

    def query(self, text: str) -> np.ndarray:
        return self.embed([QUERY_PREFIX + text])[0]


class VectorIndex:
    def __init__(self, base_dir: Path | None, cache_dir: Path, embedder):
        self.embedder = embedder
        self.ids: list[str] = []
        mats = []
        if base_dir and (Path(base_dir) / "vectors_ids.json").exists():
            ids = json.load(open(Path(base_dir) / "vectors_ids.json", encoding="utf-8"))
            m = np.fromfile(Path(base_dir) / "vectors.f32", dtype=np.float32).reshape(len(ids), DIM)
            self.ids += ids
            mats.append(m)
        self.cache_dir = Path(cache_dir)
        self._cache_keys: dict[str, int] = {}
        cid, cm = self.cache_dir / "vectors_ids.json", self.cache_dir / "vectors.f32"
        self._cache_ids, self._cache_m = [], np.zeros((0, DIM), np.float32)
        if cid.exists() and cm.exists():
            self._cache_ids = json.load(open(cid, encoding="utf-8"))
            self._cache_m = np.fromfile(cm, dtype=np.float32).reshape(len(self._cache_ids), DIM)
        self.m = np.concatenate(mats + [np.zeros((0, DIM), np.float32)]) if mats else np.zeros((0, DIM), np.float32)
        self.row = {k: i for i, k in enumerate(self.ids)}
        for j, key in enumerate(self._cache_ids):
            self._cache_keys[key] = j

    @staticmethod
    def _key(id_, text):
        return id_ + "#" + hashlib.sha1((text or "").encode("utf-8")).hexdigest()[:10]

    def ensure(self, items: dict[str, str]):
        """Make sure every id -> text has a vector (embeds what is missing, persists it)."""
        missing = [(i, t) for i, t in items.items() if self._key(i, t) not in self._cache_keys and i not in self.row]
        if missing:
            v = self.embedder.embed([t for _, t in missing])
            for (i, t), row in zip(missing, v):
                self._cache_keys[self._key(i, t)] = len(self._cache_ids)
                self._cache_ids.append(self._key(i, t))
            self._cache_m = np.concatenate([self._cache_m, v.astype(np.float32)])
            self.cache_dir.mkdir(parents=True, exist_ok=True)
            tmp = self.cache_dir / "vectors.f32.tmp"
            self._cache_m.astype(np.float32).tofile(tmp)
            os.replace(tmp, self.cache_dir / "vectors.f32")
            (self.cache_dir / "vectors_ids.json").write_text(json.dumps(self._cache_ids), encoding="utf-8")
        out = {}
        for i, t in items.items():
            k = self._key(i, t)
            if k in self._cache_keys:
                out[i] = self._cache_m[self._cache_keys[k]]
            elif i in self.row:
                out[i] = self.m[self.row[i]]
        return out

    def get(self, id_):
        return self.m[self.row[id_]] if id_ in self.row else None

    def base_matrix(self, prefix: str):
        keys = [k for k in self.ids if k.startswith(prefix)]
        return keys, (self.m[[self.row[k] for k in keys]] if keys else np.zeros((0, DIM), np.float32))
