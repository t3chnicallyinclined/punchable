"""Voice memos: Drive > Punchable > Voice memos -> notes inbox -> transcript -> a note (gsync/memos.py).
Fake Drive, fake Whisper, fake API: no network, no GPU. Each test fails if its guard is removed."""
from __future__ import annotations

import json
from dataclasses import replace
from pathlib import Path

import pytest

from gsync import memos as M
from gsync.config import FOLDER_ID, FOLDER_MIME
from gsync.punchable import ApiError
from gsync.sync import Sync

from .fakes import FakeDrive, FakePunchable, clock, make_cfg

M4A = b"\x00\x00\x00\x20ftypM4A " + b"memo-bytes-1" * 50


class FakeApi(FakePunchable):
    """FakePunchable plus POST /api/ideas answering `attach` the way merge_api.after_idea does."""

    def __init__(self, cfg):
        super().__init__(cfg)
        self.attach_to: dict[str, str] = {}  # word in the text -> joke it clearly belongs to
        self.posted: list[dict] = []

    def _req(self, method, path, body=None, timeout=30):
        assert (method, path) == ("POST", "/api/ideas"), path
        if len(body["text"]) > 4000:
            raise ApiError(400, "an idea needs its text (up to 4000 characters)")
        self.posted.append(body)
        r = self.add_idea(body["text"], body["title"] or None, body["from"], body["reason"])
        into = next((j for w, j in self.attach_to.items() if w in body["text"]), None)
        att = {"decision": "attach", "id": "at:x", "into": into, "title": "Airport bit"} if into else {"decision": "new"}
        return {"ok": True, "id": r["id"], "indexes": r["indexes"], "attach": att}


def fake_normalize(src, wav):
    wav.parent.mkdir(parents=True, exist_ok=True)
    wav.write_bytes(b"RIFF" + b"\0" * 64)
    (wav.parent / "audio.opus").write_bytes(b"OggS" + Path(src).read_bytes()[:16])
    return 3.0


class FakeWhisper:
    version = "fake"

    def __init__(self, texts: dict[str, str] | None = None, default="the gate at the airport is a studio apartment",
                 fail: bool = False):
        self.texts, self.default, self.fail, self.calls = texts or {}, default, fail, 0

    def __call__(self, wav, out_dir):
        self.calls += 1
        if self.fail:
            raise RuntimeError("cuda out of memory")
        out_dir.mkdir(parents=True, exist_ok=True)
        key = next((k for k in self.texts if k in str(out_dir)), None)
        text = self.texts.get(key, self.default) if key else self.default
        doc = {"segments": [{"start": 0.0, "end": 2.0, "text": " " + text}]}
        (out_dir / "transcript.json").write_text(json.dumps(doc), encoding="utf-8")
        return doc


def fake_probe(tags=None, dur=42.0):
    return lambda p: {"duration_s": dur, "tags": dict({"date": "2019-03-14T21:03:11Z", "title": "New Recording 12"} if tags is None else tags)}


@pytest.fixture
def env(tmp_path):
    cfg = replace(make_cfg(tmp_path), memos_enabled=True, memo_reserve_mb=0)
    d, pk = FakeDrive(), FakeApi(cfg)
    d.folder(FOLDER_ID, name="Punchable")
    spawned = []

    def run():
        s = Sync(cfg, d, pk, clock=clock)
        import gsync.memos as mm
        orig = mm.MemoPull.__init__

        def init(self, sync, spawn=None, running=None):
            orig(self, sync, spawn=lambda c: spawned.append(1), running=lambda c: False)
        mm.MemoPull.__init__ = init
        try:
            return s.run()
        finally:
            mm.MemoPull.__init__ = orig
    return cfg, d, pk, run, spawned


def store(cfg):
    return M.MemoStore(cfg.memos_state).load()


def memo_folder(d):
    return next(k for k, f in d.files.items() if f.get("mimeType") == FOLDER_MIME and f["name"] == "Voice memos")


def proc(cfg, pk, **kw):
    kw.setdefault("transcriber", FakeWhisper())
    kw.setdefault("normalizer", fake_normalize)
    kw.setdefault("prober", fake_probe())
    return M.MemoProcessor(cfg, pk, log=lambda *a: None, **kw)


# ---------------------------------------------------------------------------------------------------------- Drive side
def test_folder_made_once_in_punchable_folder_and_his_memo_downloaded_read_only(env):
    cfg, d, pk, run, spawned = env
    rec = run()
    assert not rec["errors"], rec["errors"]
    fid = memo_folder(d)
    assert d.files[fid]["parents"] == [FOLDER_ID]
    assert d.files[fid]["appProperties"] == {"punchable_kind": "voice_memos", "punchable": "1"}
    aid = d.user_upload("New Recording 12.m4a", M4A, parents=(fid,))
    before = json.dumps(d.files[aid], default=str, sort_keys=True)
    rec = run()
    assert not rec["errors"], rec["errors"]
    it = store(cfg)["items"][aid]
    assert it["status"] == "downloaded"
    assert (cfg.notes_inbox / it["file"]).read_bytes() == M4A
    assert json.dumps(d.files[aid], default=str, sort_keys=True) == before  # his file: untouched
    assert [w for w in d.written if w[1] == aid] == []
    assert sum(1 for w in d.written if w[0] == "create_folder") == 1
    assert spawned == [1]
    run()
    assert sum(1 for w in d.written if w[0] == "create_folder") == 1, "the folder is made once"
    assert d.downloaded == [aid], "a memo is downloaded once"


def test_hand_made_folder_is_adopted_and_subfolders_walked(env):
    cfg, d, pk, run, _ = env
    mine = d.folder("handmadefolder", parents=(FOLDER_ID,), name="voice Memos ")
    sub = d.folder("sub2019", parents=(mine,), name="2019")
    aid = d.user_upload("airport tag.m4a", M4A, parents=(sub,))
    run()
    assert not [w for w in d.written if w[0] == "create_folder"]
    assert store(cfg)["folder_id"] == mine
    assert store(cfg)["items"][aid]["status"] == "downloaded"


def test_court_memo_is_never_downloaded_and_its_name_is_not_kept(env):
    cfg, d, pk, run, _ = env
    run()
    fid = memo_folder(d)
    aid = d.user_upload("Lawsuit hearing notes.m4a", M4A + b"x", parents=(fid,))
    other = d.user_upload("Shared by someone.m4a", M4A + b"y", parents=(fid,), owner="someone@else.com")
    run()
    for k in (aid, other):
        it = store(cfg)["items"][k]
        assert it["status"] == "skipped_legal"
        assert "name" not in it and "md5" not in it
    assert getattr(d, "downloaded", []) == []
    assert not list(cfg.notes_inbox.glob("*"))


def test_same_bytes_twice_downloaded_once(env):
    cfg, d, pk, run, _ = env
    run()
    fid = memo_folder(d)
    a = d.user_upload("New Recording 3.m4a", M4A, parents=(fid,), created="2026-10-20T10:00:00.000Z")
    b = d.user_upload("New Recording 3 copy.m4a", M4A, parents=(fid,), created="2026-10-20T10:00:01.000Z")
    run()
    st = store(cfg)["items"]
    assert st[a]["status"] == "downloaded"
    assert st[b]["status"] == "duplicate" and st[b]["dup_of"] == a
    assert d.downloaded == [a]


def test_low_disk_pauses_downloads(env):
    cfg, d, pk, run, _ = env
    run()
    fid = memo_folder(d)
    aid = d.user_upload("New Recording 4.m4a", M4A, parents=(fid,))
    cfg.memo_reserve_mb = 10 ** 9  # more than any disk has
    rec = run()
    m = store(cfg)
    assert m["items"][aid]["status"] == "listed"
    assert m["paused"]["reason"] == "low_disk"
    assert "paused" in M.progress(m)["line"]
    assert rec["counts"].get("memo_paused_low_disk") == 1
    cfg.memo_reserve_mb = 0
    run()
    m = store(cfg)
    assert m["items"][aid]["status"] == "downloaded" and m["paused"] is None


def test_staging_cap_downloads_in_batches(env):
    cfg, d, pk, run, _ = env
    cfg.memo_stage_mb = 0  # one file at a time on disk
    run()
    fid = memo_folder(d)
    ids = [d.user_upload(f"New Recording {i}.m4a", M4A + bytes([i]), parents=(fid,)) for i in range(3)]
    run()
    assert [store(cfg)["items"][i]["status"] for i in ids].count("downloaded") == 1
    run()
    assert [store(cfg)["items"][i]["status"] for i in ids].count("downloaded") == 1, "waits for the processor"


# ---------------------------------------------------------------------------------------------------------- processor
def test_memo_becomes_a_dated_note_with_audio_and_attaches(env):
    cfg, d, pk, run, _ = env
    run()
    aid = d.user_upload("New Recording 12.m4a", M4A, parents=(memo_folder(d),))
    run()
    pk.attach_to = {"airport": "joke:airport-bit"}
    staged = cfg.notes_inbox / store(cfg)["items"][aid]["file"]
    r = proc(cfg, pk).run()
    assert r["done"] == 1
    it = store(cfg)["items"][aid]
    assert it["status"] == "done"
    assert it["recorded_at"].startswith("2019-03-14") and "©day" in it["date_source"]
    assert it["attach"] == {"decision": "attach", "id": "at:x", "into": "joke:airport-bit", "title": "Airport bit"}
    jid = it["joke_id"]
    assert pk.jokes[jid]["summary"] == "the gate at the airport is a studio apartment"  # verbatim transcript
    body = pk.posted[-1]
    assert body["title"] == ""  # "New Recording 12" is not his title: the API titles it from the words
    assert body["reason"].startswith("[gdocs] voice memo 'New Recording 12'") and f"(memo {aid}," in body["reason"]
    assert pk.app_jokes() == [], "memo ideas are never pushed back to Drive as new Google Docs"
    adir = cfg.comedy / Path(it["audio"]).parent
    assert (cfg.comedy / it["audio"]).is_file() and not (adir / "audio16k.wav").exists()
    assert (cfg.comedy / it["transcript"]).is_file()
    assert not staged.exists(), "our downloaded copy is removed (Drive keeps his original)"
    assert list(store(cfg)["items"]) == [aid] and not list(cfg.comedy.rglob("*.m4a")), "and never re-queued as a drop"
    assert d.files[aid]["_bytes"] == M4A and not d.files[aid]["trashed"]
    assert M.progress(store(cfg))["line"] == "1 of 1 voice memo transcribed"


def test_court_words_in_the_audio_keep_nothing(env):
    cfg, d, pk, run, _ = env
    run()
    aid = d.user_upload("New Recording 7.m4a", M4A, parents=(memo_folder(d),))
    run()
    w = FakeWhisper(default="the petitioner and the respondent in the superior court of the state of example")
    proc(cfg, pk, transcriber=w).run()
    it = store(cfg)["items"][aid]
    assert it["status"] == "skipped_legal" and it["why"] == "content"
    assert "title" not in it and "name" not in it and "transcript" not in it
    assert pk.posted == []
    assert not list((cfg.memo_corpus).glob("*")) and not list(cfg.memo_audio.glob("*"))
    assert M.progress(store(cfg))["total"] == 0


def test_rerun_and_crash_never_file_twice(env):
    cfg, d, pk, run, _ = env
    run()
    aid = d.user_upload("airport bit tag 2.m4a", M4A, parents=(memo_folder(d),))
    run()
    staged = cfg.notes_inbox / store(cfg)["items"][aid]["file"]
    keep = staged.read_bytes()
    named = fake_probe(tags={"date": "2019-03-14T21:03:11Z", "title": "airport bit tag 2"})
    proc(cfg, pk, prober=named).run()
    assert pk.posted[-1]["title"] == "airport bit tag"  # his own memo name is the title
    n = len(pk.posted)
    proc(cfg, pk, prober=named).run()
    assert len(pk.posted) == n
    # crash after the idea was written but before memos.json said done: the file is back, the status is downloaded
    staged.write_bytes(keep)
    M.MemoStore(cfg.memos_state).set_item(aid, status="downloaded", file=staged.name, sha256=None)
    proc(cfg, pk, prober=named).run()
    assert len(pk.posted) == n, "the decision line is found; no second idea"
    assert store(cfg)["items"][aid]["status"] == "done"


def test_local_drop_is_queued_deduped_and_never_deleted(env):
    cfg, d, pk, run, _ = env
    cfg.notes_inbox.mkdir(parents=True)
    (cfg.notes_inbox / "20180502 101500.m4a").write_bytes(M4A)
    (cfg.notes_inbox / "copy.m4a").write_bytes(M4A)
    (cfg.notes_inbox / "notes.txt").write_text("written notes stay put")
    r = proc(cfg, pk, prober=fake_probe(tags={})).run()
    assert r["done"] == 1 and r["duplicate"] == 1
    items = store(cfg)["items"]
    done = next(x for x in items.values() if x["status"] == "done")
    assert done["date"] == "2018-05-02" and done["date_source"] == "the file name"
    assert (cfg.notes_inbox / "notes.txt").exists()
    kept = list(cfg.memo_audio.rglob("*.m4a"))
    assert len(kept) == 2, "his own files are moved next to the audio, never deleted"


def test_long_recording_kept_as_a_note_not_one_giant_idea(env):
    cfg, d, pk, run, _ = env
    cfg.notes_inbox.mkdir(parents=True)
    (cfg.notes_inbox / "set.m4a").write_bytes(M4A)
    proc(cfg, pk, transcriber=FakeWhisper(default="word " * 1200)).run()
    it = next(iter(store(cfg)["items"].values()))
    assert it["status"] == "done" and it["long"] is True and "joke_id" not in it
    assert pk.posted == []


def test_failures_retry_then_stop(env):
    cfg, d, pk, run, _ = env
    cfg.notes_inbox.mkdir(parents=True)
    (cfg.notes_inbox / "bad.m4a").write_bytes(M4A)
    w = FakeWhisper(fail=True)
    proc(cfg, pk, transcriber=w).run()
    it = next(iter(store(cfg)["items"].values()))
    assert it["status"] == "failed" and it["tries"] == 3 and "cuda" in it["error"]
    assert w.calls == 3
    assert (cfg.notes_inbox / "bad.m4a").exists()


def test_api_down_stops_without_losing_the_memo(env):
    cfg, d, pk, run, _ = env
    cfg.notes_inbox.mkdir(parents=True)
    (cfg.notes_inbox / "a.m4a").write_bytes(M4A)
    pk.up = False
    r = proc(cfg, pk).run()
    assert r.get("api_down") and next(iter(store(cfg)["items"].values()))["status"] == "downloaded"


def test_one_processor_at_a_time(env):
    cfg, d, pk, run, _ = env
    lk = M.ProcLock(M.proc_lock_path(cfg))
    assert lk.acquire()
    try:
        assert M.processor_running(cfg)
        assert proc(cfg, pk).run() == {"status": "busy"}
    finally:
        lk.release()
    assert not M.processor_running(cfg)


# ---------------------------------------------------------------------------------------------------------- the file
@pytest.mark.parametrize("tags,name,src", [
    ({"date": "2019-03-14T21:03:11Z", "creation_time": "2026-10-26T10:00:00.000000Z"}, "x.m4a", "©day"),
    ({"creation_time": "2021-06-01T01:02:03.000000Z"}, "x.m4a", "creation_time"),
    ({"date": "2019"}, "20170102 030405.m4a", "file name"),
    ({}, "Brooklyn, NY 3.m4a", "Google Drive"),
])
def test_recorded_date_precedence(tags, name, src):
    at, date, source = M.recorded(tags, name, "2026-10-26T12:00:00Z", None)
    assert src in source, (at, date, source)
    assert at and date


@pytest.mark.parametrize("t,want", [("New Recording 12", None), ("Brooklyn, NY 3", None), ("123 Main St", None),
                                    ("20190314 210311", None), ("airport bit tag", "airport bit tag"),
                                    ("Parking app 2", "Parking app")])
def test_idea_title(t, want):
    assert M.idea_title(t) == want


def test_processor_scanning_mid_download_never_takes_a_download_for_a_drop(env, monkeypatch):
    cfg, d, pk, run, _ = env
    run()
    aid = d.user_upload("New Recording 9.m4a", M4A, parents=(memo_folder(d),))
    p = proc(cfg, pk)
    real = M.os.replace

    def replace(a, b):  # the processor looks at the inbox the moment the download lands
        real(a, b)
        if str(b).endswith(".m4a"):
            p.register_local()
    monkeypatch.setattr(M.os, "replace", replace)
    run()
    monkeypatch.setattr(M.os, "replace", real)
    assert list(store(cfg)["items"]) == [aid]
