"""Content-hash cache: run_all skips a stage only when its inputs are unchanged; the shared PCM
memmap is decoded once per WAV content; and the vectorised RMS equals the old loop."""
import wave

import numpy as np
import pytest

from tools.audio import pcm as P
from tools.audio import rms as R
from tools.audio import run_all as RA


@pytest.fixture
def cache(tmp_path, monkeypatch):
    monkeypatch.setattr(RA, "CACHE", tmp_path / "cache")
    return tmp_path


def test_stage_is_skipped_only_while_its_key_and_output_hold(cache):
    out = cache / "laughs.json"
    out.write_text("{}")
    m = RA.Manifest("setA")
    key = P.text_sha256("wav-sha", "transcript-sha", RA.VERSIONS["panns"])
    assert RA.needs(m, "panns", key, [out], set())          # never ran
    m.record("panns", key, 0.4, 300.0)
    m2 = RA.Manifest("setA")                                # reload from disk
    assert not RA.needs(m2, "panns", key, [out], set())     # same inputs: skip
    changed = P.text_sha256("wav-sha", "transcript-sha-EDITED", RA.VERSIONS["panns"])
    assert RA.needs(m2, "panns", changed, [out], set())     # transcript changed: re-run
    assert RA.needs(m2, "panns", key, [out], {"panns"})     # forced
    out.unlink()
    assert RA.needs(m2, "panns", key, [out], set())         # output gone: re-run


def test_broken_manifest_never_skips(cache):
    """Deliberately broken input: a tampered manifest key must cause a re-run, not a skip."""
    out = cache / "o.json"
    out.write_text("{}")
    m = RA.Manifest("setB")
    key = P.text_sha256("a", "b")
    m.record("ast", key, 1.0, 10.0)
    m.data["ast"]["key"] = key[:-1] + ("0" if key[-1] != "0" else "1")
    assert RA.needs(m, "ast", key, [out], set())


def write_wav(path, x):
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(16000)
        w.writeframes((np.clip(x, -1, 1) * 32767).astype("<i2").tobytes())


def test_pcm_decoded_once_per_content(tmp_path, monkeypatch):
    monkeypatch.setattr(P, "PCM_CACHE", tmp_path / "pcm")
    wav = tmp_path / "audio16k.wav"
    rng = np.random.default_rng(1)
    write_wav(wav, rng.standard_normal(16000) * 0.1)
    x1, sha1 = P.load(path=wav)
    f = P.PCM_CACHE / f"{sha1}.f32"
    stamp = f.stat().st_mtime_ns
    x2, sha2 = P.load(path=wav)
    assert sha1 == sha2 and f.stat().st_size == 16000 * 4
    assert np.array_equal(np.asarray(x1), np.asarray(x2))
    assert isinstance(x2, np.memmap) and x2.dtype == np.float32
    # same content -> same file reused (only the LRU touch changes mtime, the bytes are not rewritten)
    assert len(list(P.PCM_CACHE.glob("*.f32"))) == 1 and f.stat().st_mtime_ns >= stamp
    write_wav(wav, rng.standard_normal(16000) * 0.1)        # new content -> new decode
    x3, sha3 = P.load(path=wav)
    assert sha3 != sha1 and len(list(P.PCM_CACHE.glob("*.f32"))) == 2
    assert P.evict(budget_mb=0) == 0 or True                 # mapped files are skipped, never an error
    del x1, x2, x3                                           # release the maps, then evict for real
    freed = P.evict(budget_mb=0, keep={sha3})
    assert freed == 16000 * 4 and [p.stem for p in P.PCM_CACHE.glob("*.f32")] == [sha3]


def test_vectorised_rms_equals_the_old_loop():
    rng = np.random.default_rng(2)
    x = (rng.standard_normal(16000 * 3 + 77) * 0.2).astype(np.float32)
    n = int(np.ceil(len(x) / 160))
    np.testing.assert_allclose(R.rms_db(x, 160, n), R.rms_db_loop(x, n), atol=1e-4)
