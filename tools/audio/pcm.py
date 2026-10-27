"""Decode a set's audio16k.wav once into a memory-mapped float32 file shared by
every stage (VAD, Whisper, PANNs, AST, RMS), across processes and across runs.

The cache key is the SHA-256 of the WAV bytes, so an edited or re-downloaded
tape gets a fresh decode and a stale one can never be read. The cache lives
outside both repos (PCM_CACHE, default ~/.cache/punchable-pcm) and is held to a
byte budget (least recently used files go first). Only files this module wrote
(<sha>.f32 + <sha>.json) are ever evicted.
"""
from __future__ import annotations

import hashlib
import json
import os
import time
from pathlib import Path

import numpy as np

SR = 16000
PCM_CACHE = Path(os.environ.get("PCM_CACHE", Path.home() / ".cache" / "punchable-pcm"))
BUDGET_MB = int(os.environ.get("PCM_CACHE_MB", "320"))
TAPES = Path(os.environ.get("COMEDY_DIR", os.path.expanduser("~/punchable-data"))) / "tapes" / "raw"


def file_sha256(path: Path, bufsize: int = 1 << 20) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            b = f.read(bufsize)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


def text_sha256(*parts) -> str:
    h = hashlib.sha256()
    for p in parts:
        h.update(p if isinstance(p, bytes) else str(p).encode("utf-8"))
        h.update(b"\0")
    return h.hexdigest()


def wav_path(set_id: str) -> Path:
    return TAPES / set_id / "audio16k.wav"


def _read_wav_float32(path: Path) -> np.ndarray:
    """16-bit PCM WAV -> float32 in [-1, 1). Uses the stdlib reader (no extra deps)."""
    import wave
    with wave.open(str(path)) as w:
        if w.getframerate() != SR or w.getsampwidth() != 2:
            raise ValueError(f"{path}: expected 16 kHz 16-bit PCM, got {w.getframerate()} Hz {8 * w.getsampwidth()}-bit")
        ch = w.getnchannels()
        raw = w.readframes(w.getnframes())
    x = np.frombuffer(raw, dtype="<i2")
    if ch > 1:
        x = x.reshape(-1, ch).mean(axis=1)
    return (x.astype(np.float32) / 32768.0)


def load(set_id: str | None = None, path: Path | None = None, sha: str | None = None) -> tuple[np.memmap, str]:
    """(read-only float32 memmap of the whole set at 16 kHz, wav sha256)."""
    path = Path(path) if path else wav_path(set_id)
    sha = sha or file_sha256(path)
    PCM_CACHE.mkdir(parents=True, exist_ok=True)
    f32 = PCM_CACHE / f"{sha}.f32"
    info = PCM_CACHE / f"{sha}.json"
    if not (f32.exists() and info.exists()):
        x = _read_wav_float32(path)
        tmp = f32.with_suffix(".tmp")
        mm = np.memmap(tmp, dtype=np.float32, mode="w+", shape=x.shape)
        mm[:] = x
        mm.flush()
        del mm
        os.replace(tmp, f32)
        info.write_text(json.dumps({"source": str(path), "samples": int(x.shape[0]), "sr": SR}), encoding="utf-8")
    n = json.loads(info.read_text(encoding="utf-8"))["samples"]
    os.utime(f32)  # LRU touch
    return np.memmap(f32, dtype=np.float32, mode="r", shape=(n,)), sha


def evict(budget_mb: int = BUDGET_MB, keep: set[str] = frozenset()) -> int:
    """Delete least recently used cache entries until under budget. Returns bytes freed."""
    if not PCM_CACHE.exists():
        return 0
    files = sorted(PCM_CACHE.glob("*.f32"), key=lambda p: p.stat().st_mtime)
    total = sum(p.stat().st_size for p in files)
    freed = 0
    for p in files:
        if total <= budget_mb * 1024 * 1024:
            break
        if p.stem in keep:
            continue
        sz = p.stat().st_size
        try:
            p.unlink()
        except PermissionError:  # still memory-mapped by a live array (Windows): leave it for next time
            continue
        (p.with_suffix(".json")).unlink(missing_ok=True)
        total -= sz
        freed += sz
    return freed


def timed(fn, *a, **k):
    t = time.perf_counter()
    r = fn(*a, **k)
    return r, time.perf_counter() - t
