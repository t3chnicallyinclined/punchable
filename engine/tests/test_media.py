"""Phone-video import: metadata, venue inference, show grouping. Uses tiny synthetic videos made with
ffmpeg (no real recordings)."""
import json
import shutil
import subprocess

import pytest

from engine import media
from engine.audio import normalize
from engine.inbox import Inbox
from engine.tests.conftest import FakeEmbedder, FakeLaughs, FakeTranscriber

pytestmark = pytest.mark.skipif(not shutil.which("ffmpeg"), reason="needs ffmpeg")


def make_video(path, seconds, utc, iso6709=None, apple_local=None, model=None, hevc=False, android=False):
    cmd = ["ffmpeg", "-nostdin", "-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=160x120:rate=10",
           "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-t", str(seconds),
           "-c:v", "libx265" if hevc else "libx264", "-c:a", "aac", "-metadata", f"creation_time={utc}"]
    if hevc:
        cmd += ["-tag:v", "hvc1", "-x265-params", "log-level=error"]
    if android:
        if iso6709:
            cmd += ["-metadata", f"location={iso6709}"]
    else:
        cmd += ["-movflags", "use_metadata_tags"]
        if iso6709:
            cmd += ["-metadata", f"com.apple.quicktime.location.ISO6709={iso6709}"]
        if apple_local:
            cmd += ["-metadata", f"com.apple.quicktime.creationdate={apple_local}"]
        if model:
            cmd += ["-metadata", "com.apple.quicktime.make=Apple", "-metadata", f"com.apple.quicktime.model={model}"]
    subprocess.run(cmd + [str(path)], check=True)
    return path


def test_iso6709():
    assert media.parse_iso6709("+40.7295-073.9965+010.000/") == {"lat": 40.7295, "lon": -73.9965, "alt_m": 10.0}
    assert media.parse_iso6709("+40.7297-073.9968/") == {"lat": 40.7297, "lon": -73.9968}
    assert media.parse_iso6709("garbage") is None


def test_probe_iphone_hevc_and_android(tmp_path):
    a = make_video(tmp_path / "IMG_0001.MOV", 3, "2026-10-24T02:15:00Z", "+40.7295-073.9965+010.000/",
                   "2026-10-23T22:15:00-0400", "iPhone 15 Pro", hevc=True)
    m = media.probe(a)
    assert m["has_video"] and m["video_codec"] == "hevc" and 2.5 < m["duration_s"] < 3.5
    assert m["date"] == "2026-10-23" and m["tz"] == "-0400"  # the show date is LOCAL, not the UTC date
    assert m["gps"]["lat"] == 40.7295 and m["device"] == "Apple iPhone 15 Pro"
    b = make_video(tmp_path / "VID_0002.mp4", 2, "2026-10-24T03:05:00Z", "+40.7297-073.9968/", android=True)
    m2 = media.probe(b)
    assert m2["gps"] == {"lat": 40.7297, "lon": -73.9968} and m2["created_utc"].startswith("2026-10-24T03:05")
    assert media.same_show(m, m2)  # 50 min apart, ~35 m apart
    far = dict(m2, gps={"lat": 40.75, "lon": -73.99})
    assert not media.same_show(m, far)
    late = dict(m2, created_utc="2026-10-24T09:00:00+00:00")
    assert not media.same_show(m, late)


def test_normalize_video_extracts_audio_only(tmp_path):
    v = make_video(tmp_path / "clip.mov", 2, "2026-10-24T02:15:00Z", hevc=True)
    secs = normalize(v, tmp_path / "out" / "audio16k.wav")
    assert 1.5 < secs < 2.5
    assert {p.name for p in (tmp_path / "out").iterdir()} == {"audio16k.wav", "audio.opus"}
    assert v.exists()  # the original is untouched


def test_video_import_venue_and_show_grouping(env):
    root = env["root"]
    # an earlier set at a known venue, with GPS in its meta
    known = root / "corpus" / "sets" / "2026-09-01_aaaa0000"
    known.mkdir(parents=True)
    (known / "meta.json").write_text(json.dumps({"venue": "The Corner Room NYC", "venue_status": "confirmed",
                                                 "gps": {"lat": 40.7296, "lon": -73.9966}}), encoding="utf-8")
    make_video(root / "inbox" / "IMG_0001.MOV", 3, "2026-10-24T02:15:00Z", "+40.7295-073.9965+010.000/",
               "2026-10-23T22:15:00-0400", "iPhone 15 Pro", hevc=True)
    make_video(root / "inbox" / "IMG_0002.MOV", 2, "2026-10-24T02:40:00Z", "+40.7297-073.9968+010.000/",
               "2026-10-23T22:40:00-0400", "iPhone 15 Pro", hevc=True)
    ib = Inbox(env["cfg"], embedder=FakeEmbedder(), transcriber=FakeTranscriber(), laughs=FakeLaughs(),
               log=lambda s: None)  # real ffprobe + ffmpeg, fake whisper/laughs
    r1, r2 = ib.run()
    assert r1["kind"] == r2["kind"] == "clip"  # short, but the room laughed
    metas = {p.parent.name: json.loads(p.read_text()) for p in (root / "corpus" / "sets").glob("2026-10-23_*/meta.json")}
    assert len(metas) == 2
    for m in metas.values():
        assert m["date"] == "2026-10-23" and m["venue"] == "The Corner Room NYC" and m["venue_status"] == "inferred"
        assert m["device"] == "Apple iPhone 15 Pro" and m["show"].startswith("show:")
    assert sorted(m["segment"] for m in metas.values()) == [1, 2]
    g = json.loads((root / "kb" / "graph.json").read_text(encoding="utf-8"))
    shows = [n for n in g["nodes"] if n["type"] == "show"]
    assert len(shows) == 1 and len(shows[0]["segments"]) == 2
    assert len([e for e in g["edges"] if e["dst"] == shows[0]["id"] and e["kind"] == "part_of"]) == 2
    # videos moved, not copied; no video copy anywhere else
    assert not list((root / "inbox").iterdir())
    vids = list(root.rglob("*.MOV"))
    assert len(vids) == 2 and all("processed" in str(v) for v in vids)
