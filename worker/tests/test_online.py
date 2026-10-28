"""Online-first worker (worker/online.py): the workspace is a cache of the API, results go back with their actor,
court text never leaves, and no media survives a job, even a failed one. No network, no GPU, no real data.

    python -m pytest worker/tests/test_online.py -q
"""
from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK))
from worker import online  # noqa: E402

LEGAL = "the petitioner and the respondent, index no 4, sworn to before me"


def sha(t: str) -> str:
    return hashlib.sha256(t.encode("utf-8")).hexdigest()


class FakeApi:
    def __init__(self):
        self.files = {"kb/v1/graph.json": json.dumps({"nodes": [], "edges": []}),
                      "corpus/sets/20240101_abc/analysis.json": json.dumps({"set": "2024-01-01_abc", "rows": []}),
                      "kb/gsync/map.json": json.dumps({"docs": {"server": 1}}),
                      "vectors/vectors.part000.json": json.dumps({"cache": "vectors.f32", "ids_file": "vectors_ids.json",
                                                                  "dim": 2, "ids": ["unit:a"], "rows": [[0.5, 0.25]]})}
        self.decisions = [{"target": "node", "id": "joke:a", "action": "add", "by": "owner"}]
        self.posted: list[dict] = []
        self.fetched: list[str] = []
        self.ideas: list[dict] = []

    def call(self, method, path, body=None):
        if path == "/api/worker/files" and method == "GET":
            return {"files": [{"path": p, "sha256": sha(t)} for p, t in self.files.items()]}
        if path.startswith("/api/worker/files/content"):
            from urllib.parse import unquote
            p = unquote(path.split("path=")[1])
            self.fetched.append(p)
            return {"path": p, "text": self.files[p]}
        if path == "/api/worker/files" and method == "POST":
            self.posted.append(body)
            for f in body["files"]:
                assert f["sha256"] == sha(f["text"])
                self.files[f["path"]] = f["text"]
            return {"written": [f["path"] for f in body["files"]]}
        if path.startswith("/api/worker/decisions"):
            after = int(path.split("after=")[1])
            return {"total": len(self.decisions), "lines": [{"decision": d} for d in self.decisions[after:]]}
        if path.startswith("/api/worker/creative"):
            return {"lines": []}
        if path == "/api/worker/profile/voice":
            self.profile_gets = getattr(self, "profile_gets", 0) + 1
            return self.profile if hasattr(self, "profile") else {"kind": "voice", "text": None}
        if path == "/api/worker/ideas":
            self.ideas.append(body)
            return {"ok": True, "id": "joke:new", "attach": {"decision": "new"}}
        raise AssertionError((method, path))


@pytest.fixture
def ws(tmp_path):
    api = FakeApi()
    return api, online.Workspace(api, tmp_path / "ws", legal=lambda t: "petitioner" in t and "respondent" in t)


def test_refresh_fetches_only_what_changed(ws):
    api, w = ws
    r = w.refresh()
    assert r["fetched"] == 4 and r["decisions"] == 1
    assert (w.comedy / "kb" / "v1" / "graph.json").is_file()
    assert json.loads((w.vec / "vectors_ids.json").read_text()) == ["unit:a"] and (w.vec / "vectors.f32").stat().st_size == 8
    assert (w.comedy / "kb" / "v0" / "decisions.jsonl").read_text().count("\n") == 1
    api.fetched.clear()
    assert w.refresh()["fetched"] == 0 and api.fetched == []
    api.files["kb/v1/graph.json"] = json.dumps({"nodes": [{"id": "joke:z"}], "edges": []})
    del api.files["corpus/sets/20240101_abc/analysis.json"]
    r = w.refresh()
    assert api.fetched == ["kb/v1/graph.json"] and r["removed"] == 1
    assert not (w.comedy / "corpus" / "sets" / "20240101_abc" / "analysis.json").exists()


def test_gsync_state_here_is_never_overwritten(ws):
    api, w = ws
    p = w.comedy / "kb" / "gsync" / "map.json"
    p.parent.mkdir(parents=True)
    p.write_text('{"docs": {"local": 1}}', encoding="utf-8")
    w.refresh()
    assert json.loads(p.read_text()) == {"docs": {"local": 1}}
    p.unlink()
    w.refresh()
    assert json.loads(p.read_text()) == {"docs": {"server": 1}}, "a fresh PC gets it back from the database"


def test_push_posts_changed_allowed_files_with_the_actor_and_never_court_text(ws):
    api, w = ws
    w.refresh()
    before = w.snapshot()
    d = w.comedy / "corpus" / "sets" / "20991231_new"
    d.mkdir(parents=True)
    (d / "analysis.json").write_text('{"set": "x"}', encoding="utf-8")
    (d / "transcript.json").write_text(json.dumps({"segments": [{"text": LEGAL}]}), encoding="utf-8")
    (d / "notes.txt").write_text("not a data file", encoding="utf-8")
    (w.comedy / "kb" / "v0" / "decisions.jsonl").write_text("{}\n", encoding="utf-8")  # never posted as a file
    r = w.push(before, "import:youtube")
    assert r["sent"] == ["corpus/sets/20991231_new/analysis.json"]
    assert r["legal_refused"] == ["corpus/sets/20991231_new/transcript.json"]
    assert not (d / "transcript.json").exists(), "court text is deleted here, never sent"
    assert api.posted[-1]["actor"] == "import:youtube"
    n = len(api.posted)
    assert w.push(w.snapshot(), "import:youtube")["sent"] == [] and len(api.posted) == n


def make_media(w):
    raw = w.comedy / "tapes" / "raw" / "20991231_new"
    raw.mkdir(parents=True)
    (raw / "audio16k.wav").write_bytes(b"RIFF")
    (raw / "audio.opus").write_bytes(b"OggS")
    (w.root / "upload").mkdir(parents=True, exist_ok=True)
    (w.root / "upload" / "phone.m4a").write_bytes(b"m4a")
    (w.cache / "audio" / "x").mkdir(parents=True)
    (w.cache / "audio" / "x" / "manifest.json").write_text("{}")


def test_purge_deletes_every_recording_and_keeps_text(ws):
    api, w = ws
    w.refresh()
    make_media(w)
    n = w.purge_media()
    assert n >= 4
    left = [p for p in w.root.rglob("*") if p.is_file() and p.suffix.lower() in online.MEDIA_EXT]
    assert left == [] and not (w.comedy / "tapes").exists() and not (w.cache / "audio").exists()
    assert (w.comedy / "kb" / "v1" / "graph.json").is_file()


class Boom:
    @staticmethod
    def import_youtube(inp):
        raise RuntimeError("yt-dlp failed")


def test_a_failed_job_still_leaves_no_media(ws, monkeypatch):
    api, w = ws
    w.refresh()
    make_media(w)
    monkeypatch.setattr(w, "refresh", lambda: {"fetched": 0, "files": 0, "decisions": 0})
    with pytest.raises(RuntimeError):
        online.run_job(api, w, {"kind": "import_youtube", "input": {}}, execute=Boom)
    assert not [p for p in w.root.rglob("*") if p.is_file() and p.suffix.lower() in online.MEDIA_EXT]


def test_an_uploaded_note_becomes_an_import_upload_idea(ws, tmp_path):
    api, w = ws
    w.refresh()
    f = tmp_path / "note.m4a"
    f.write_bytes(b"\x00" * 64)

    def norm(src, wav):
        wav.parent.mkdir(parents=True, exist_ok=True)
        wav.write_bytes(b"RIFF")
        (wav.parent / "audio.opus").write_bytes(b"OggS")

    def tx(text):
        def t(wav, out):
            out.mkdir(parents=True, exist_ok=True)
            doc = {"segments": [{"start": 0, "end": 1, "text": text}]}
            (out / "transcript.json").write_text(json.dumps(doc), encoding="utf-8")
            return doc
        return t

    meta = {"kind": "note", "title": "Voice note", "recorded_at": "2026-10-27T01:02:03+00:00", "original_name": "note.m4a"}
    r = online.upload_note(api, w, f, meta, {"duration_s": 3.0, "tags": {}}, transcriber=tx("the bodega cat runs the store"),
                           normalizer=norm)
    assert r["status"] == "processed" and r["joke_id"] == "joke:new"
    assert api.ideas[-1]["actor"] == "import:upload" and "bodega cat" in api.ideas[-1]["text"]
    m = json.loads((w.comedy / "corpus" / "memos" / r["memo"] / "meta.json").read_text())
    assert m["source"] == "app_upload" and m["original_name"] == "note.m4a" and m["date"] == "2026-10-27"
    n = len(api.ideas)
    r = online.upload_note(api, w, f, meta, {"duration_s": 3.0, "tags": {}}, transcriber=tx(LEGAL), normalizer=norm)
    assert r == {"status": "skipped_legal"} and len(api.ideas) == n
    assert not list((w.comedy / "corpus" / "memos").glob("*/transcript.json")) or \
        all("petitioner" not in p.read_text() for p in (w.comedy / "corpus" / "memos").glob("*/transcript.json"))


def test_local_mode_never_asks_for_online_only_kinds():
    import worker.__main__ as WM
    assert "embed" not in WM.default_kinds(False) and "memolink" not in WM.default_kinds(False)
    assert {"embed", "memolink"} <= set(WM.default_kinds(True))


def test_every_job_fetches_the_voice_profile_and_runs_with_it(ws, monkeypatch):
    """online.run_job: GET /api/worker/profile/voice before the job, cached by updated_at, the text never in the log,
    the result saying which profile the job ran with (loaded, updated_at, sha, chars: never the text)."""
    from creative import voice
    api, w = ws
    voice.clear()
    lines = []
    monkeypatch.setattr(online, "log", lambda *a: lines.append(" ".join(str(x) for x in a)))
    seen = []

    class Exec:
        @staticmethod
        def run(job, file=None):
            seen.append(voice.profile())
            return {"status": "ok"}
    try:
        r = online.run_job(api, w, {"kind": "riff", "input": {}}, execute=Exec)
        assert r["online"]["voice"] == {"loaded": False, "updated_at": None, "sha": None, "chars": 0} and seen[-1] is None
        api.profile = {"kind": "voice", "text": "# Voice\n\nHe sits on the punch word; the parrot bit closes.",
                       "updated_at": "2026-10-28T10:00:00+00:00", "by": "owner"}
        r = online.run_job(api, w, {"kind": "riff", "input": {}}, execute=Exec)
        assert r["online"]["voice"]["loaded"] and r["online"]["voice"]["updated_at"] == "2026-10-28T10:00:00+00:00"
        assert seen[-1]["text"].startswith("# Voice") and seen[-1]["source"] == "api"
        n = api.profile_gets
        online.run_job(api, w, {"kind": "punchup", "input": {}}, execute=Exec)
        assert api.profile_gets == n + 1, "fetched per job (cheap), replaced only when updated_at moves"
        blob = "\n".join(lines) + json.dumps(r)
        assert "parrot" not in blob and "punch word" not in blob
        assert any(l.startswith("voice profile: loaded (") for l in lines)
    finally:
        voice.clear()
