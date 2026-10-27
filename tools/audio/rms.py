"""RMS energy curve from the shared PCM (replaces the earlier clip tool prosody.rs's ffmpeg
`astats` pass and detect_laughs.py's per-frame Python loop).

Two resolutions, both in dBFS (10*log10(mean square + 1e-12), same as before):
  * frames: 10 ms (160 samples), what detect_laughs uses for event loudness
  * windows: 1 s, what the earlier clip tool's astats(length=1) curve gives, for clip scoring
"""
from __future__ import annotations

import numpy as np

SR = 16000


def rms_db(pcm: np.ndarray, hop: int = 160, n_frames: int | None = None) -> np.ndarray:
    """Vectorised. Frame i = samples [i*hop, (i+1)*hop); missing frames are -120 dB."""
    n_full = len(pcm) // hop
    if n_frames is None:
        n_frames = int(np.ceil(len(pcm) / hop))
    out = np.full(n_frames, -120.0, dtype=np.float32)
    x = np.asarray(pcm[:n_full * hop], dtype=np.float64).reshape(n_full, hop)
    k = min(n_full, n_frames)
    out[:k] = 10 * np.log10(np.mean(x[:k] ** 2, axis=1) + 1e-12)
    if n_frames > n_full and len(pcm) > n_full * hop:
        tail = np.asarray(pcm[n_full * hop:], dtype=np.float64)
        out[n_full] = 10 * np.log10(np.mean(tail ** 2) + 1e-12)
    return out


def rms_db_loop(pcm: np.ndarray, n_frames: int, hop: int = 160) -> np.ndarray:
    """The old detect_laughs.rms_db, kept only as the reference for the tests."""
    out = np.full(n_frames, -120.0, dtype=np.float32)
    for i in range(n_frames):
        seg = pcm[i * hop:(i + 1) * hop]
        if len(seg):
            out[i] = 10 * np.log10(np.mean(seg.astype(np.float64) ** 2) + 1e-12)
    return out


def curve(pcm: np.ndarray) -> dict:
    """{'hop_s', 'db_10ms', 'db_1s'} as float32 arrays."""
    return {"hop_s": 0.01, "db_10ms": rms_db(pcm, 160), "db_1s": rms_db(pcm, SR)}
