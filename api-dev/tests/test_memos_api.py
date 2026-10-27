"""GET /api/memos* over a temp comedy folder (memos.json as gsync/memos.py writes it). Never reads his real memos.

    python -m pytest api-dev/tests/test_memos_api.py -q
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "db" / "tests"))
sys.path.insert(0, str(DESK))
from common import Api, TempKB  # noqa: E402

OPUS = b"OggS" + bytes(range(256)) * 4


@pytest.fixture(scope="module")
def api():
    t = TempKB("memos", decisions=False)
    root = t.root
    name = "2019-03-14_abcdef12"
    (root / "corpus" / "memos" / name).mkdir(parents=True)
    (root / "corpus" / "memos" / name / "transcript.json").write_text(json.dumps(
        {"segments": [{"start": 0, "end": 2, "text": " the gate at the airport"}, {"start": 2, "end": 4, "text": " is a studio."}]}),
        encoding="utf-8")
    (root / "tapes" / "processed" / name).mkdir(parents=True)
    (root / "tapes" / "processed" / name / "audio.opus").write_bytes(OPUS)
    secret = root.parent / f"{root.name}_secret.opus"  # outside the comedy folder, and it exists
    secret.write_bytes(b"not a memo")
    items = {
        "drv1": {"id": "drv1", "source": "drive", "status": "done", "title": "New Recording 12", "date": "2019-03-14",
                 "recorded_at": "2019-03-14T21:03:11+00:00", "duration_s": 4.0, "audio": f"tapes/processed/{name}/audio.opus",
                 "transcript": f"corpus/memos/{name}/transcript.json", "joke_id": "joke:x",
                 "attach": {"decision": "attach", "id": "at:1", "into": "joke:airport", "title": "Airport bit"}},
        "drv2": {"id": "drv2", "source": "drive", "status": "listed", "name": "New Recording 13.m4a"},
        "drv3": {"id": "drv3", "source": "drive", "status": "skipped_legal", "why": "kw"},
        "drv4": {"id": "drv4", "source": "drive", "status": "duplicate", "name": "copy.m4a", "dup_of": "drv1"},
        "drv5": {"id": "drv5", "source": "drive", "status": "done", "title": "sneaky", "audio": f"../{secret.name}",
                 "transcript": "../../etc/passwd"},
    }
    (root / "kb" / "gsync").mkdir(parents=True)
    (root / "kb" / "gsync" / "memos.json").write_text(json.dumps({"items": items, "paused": None, "processor": {}}),
                                                      encoding="utf-8")
    a = Api(t, "files", extra=["--tapes", str(root / "tapes")])
    yield a
    a.stop()
    secret.unlink(missing_ok=True)
    t.cleanup()


def test_list_progress_and_hidden(api):
    s, r = api.get("/api/memos")
    assert s == 200 and r["available"] is True
    assert r["progress"]["line"] == "2 of 3 voice memos transcribed"
    assert r["progress"]["skipped_legal"] == 1 and r["progress"]["duplicates"] == 1
    ids = [x["id"] for x in r["items"]]
    assert "drv3" not in ids and "drv4" not in ids, "court material and duplicates are only counted"
    one = next(x for x in r["items"] if x["id"] == "drv1")
    assert one["text"] == "the gate at the airport is a studio." and one["date"] == "2019-03-14"
    assert one["attached_to"] == {"joke_id": "joke:airport", "title": "Airport bit"}
    assert one["audio"] == "/api/memos/drv1/audio"
    assert ids[0] == "drv2", "waiting memos first"


def test_paths_outside_the_comedy_folder_are_never_served(api):
    s, r = api.get("/api/memos/drv5")
    assert s == 200 and r["memo"]["audio"] is None and r["memo"]["text"] == ""
    s, _ = api.get("/api/memos/drv5/audio")
    assert s == 404
    s, _ = api.get("/api/memos/drv3")
    assert s == 404


def test_audio_ranges(api):
    s, body, h = api.get("/api/memos/drv1/audio", raw=True, headers={"Range": "bytes=4-9"})
    assert s == 206 and body == OPUS[4:10] and h["Content-Range"] == f"bytes 4-9/{len(OPUS)}"
    s, body, _ = api.get("/api/memos/drv1/audio", raw=True)
    assert s == 200 and body == OPUS


def test_detail_has_segments(api):
    s, r = api.get("/api/memos/drv1")
    assert s == 200 and [x["text"] for x in r["memo"]["segments"]] == ["the gate at the airport", "is a studio."]


def test_same_progress_line_as_gsync():
    sys.path.insert(0, str(DESK / "api-dev"))
    import memos_api
    from gsync import memos as G
    m = {"items": {str(i): {"status": s} for i, s in enumerate(["done", "listed", "duplicate", "skipped_legal", "failed"])},
         "paused": {"reason": "low_disk", "free_mb": 512}}
    assert memos_api.progress(m)["line"] == G.progress(m)["line"] == \
        "1 of 3 voice memos transcribed · paused: the PC is low on disk space (512 MB free)"
