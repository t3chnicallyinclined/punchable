"""Online-first gsync (gsync/punchable.HostedPunchable, gsync/memos.py online): imports carry their actor, nothing is
read from local files, gsync's state goes to the database, and a memo's audio is not kept. No network, no GPU."""
from __future__ import annotations

import json
from dataclasses import replace

import pytest

from gsync import memos as M
from gsync.punchable import ApiDown, ApiError, HostedPunchable

from .fakes import make_cfg
from .test_memos import M4A, FakeApi, FakeWhisper, fake_normalize, fake_probe


class FakeClient:
    """worker.__main__.Client's call(): records every request, answers from small tables, can fail like it."""

    def __init__(self):
        self.calls: list[tuple] = []
        self.files: dict[str, str] = {}
        self.decisions = [{"target": "node", "id": "joke:a", "action": "add", "by": "import:gsync"}]
        self.creative = {"storyboards": [json.dumps({"ts": "2026-10-26T00:00:00+00:00", "op": "create", "id": "sb:x",
                                                     "by": "owner", "data": {"title": "AI", "jokes": [], "target_s": 420}})],
                         "merges": []}
        self.fail = None

    def call(self, method, path, body=None):
        self.calls.append((method, path, body))
        if self.fail:
            raise self.fail
        if path == "/api/health":
            return {"ok": True}
        if path.startswith("/api/worker/decisions"):
            after = int(path.split("after=")[1])
            return {"total": len(self.decisions), "lines": [{"decision": d} for d in self.decisions[after:]]}
        if path.startswith("/api/worker/creative"):
            assert path.endswith("&snapshot=1"), "gsync reads the API's replay, never creative/ code"
            name = path.split("store=")[1].split("&")[0]
            return {"lines": self.creative[name],
                    "items": {json.loads(x)["id"]: json.loads(x)["data"] for x in self.creative[name]}}
        if path.startswith("/api/worker/files?prefix="):
            pre = path.split("prefix=")[1]
            import hashlib
            return {"files": [{"path": p, "sha256": hashlib.sha256(t.encode()).hexdigest()}
                              for p, t in self.files.items() if p.startswith(pre)]}
        if path.startswith("/api/worker/files/content"):
            from urllib.parse import unquote
            p = unquote(path.split("path=")[1])
            return {"path": p, "text": self.files[p]}
        if path == "/api/worker/files":
            for f in body["files"]:
                self.files[f["path"]] = f["text"]
            return {"written": [f["path"] for f in body["files"]]}
        if path == "/api/worker/ideas":
            return {"ok": True, "id": "joke:new", "indexes": [7], "attach": {"decision": "new"}}
        if path == "/api/worker/decision":
            if body.get("after", {}).get("summary") == "same":
                raise RuntimeError('POST /api/worker/decision: HTTP 400 {"error": "nothing changed"}')
            return {"ok": True, "indexes": [8], "decisions": [{"id": "joke:x"}]}
        if path.startswith("/api/worker/jokes/"):
            return {"joke": {"title": "T", "summary": "S", "stage": "idea"}, "versions": [], "yours": True}
        raise AssertionError(path)


@pytest.fixture
def hp(tmp_path):
    cfg = make_cfg(tmp_path)
    cl = FakeClient()
    return cfg, cl, HostedPunchable(cfg, cl)


def test_writes_go_to_worker_routes_with_the_import_actor(hp):
    cfg, cl, pk = hp
    r = pk.add_idea("a doc about airports", "Airports", {"gdoc_id": "d1"}, "[gdocs] new doc")
    assert r == {"id": "joke:new", "indexes": [7]}
    m, path, body = cl.calls[-1]
    assert (m, path, body["actor"]) == ("POST", "/api/worker/ideas", "import:gsync")
    assert pk.edit_joke("joke:x", {"summary": "changed"}, "[gdocs] edited") == {"indexes": [8]}
    assert cl.calls[-1][1] == "/api/worker/decision" and cl.calls[-1][2]["actor"] == "import:gsync"
    assert pk.edit_joke("joke:x", {"summary": "same"}, "[gdocs] edited") is None  # HTTP 400 -> ApiError -> None
    pk.joke("joke:x")
    assert cl.calls[-1][1] == "/api/worker/jokes/joke:x"


def test_reads_come_from_the_api_not_local_files(hp, tmp_path):
    cfg, cl, pk = hp
    (cfg.comedy / "kb" / "v0").mkdir(parents=True)
    (cfg.comedy / "kb" / "v0" / "decisions.jsonl").write_text('{"target":"node","id":"joke:LOCAL","action":"add"}\n')
    assert [d["id"] for d in pk.decisions()] == ["joke:a"]
    assert list(pk.boards()) == ["sb:x"]
    assert pk.merges() == {}


def test_api_errors_and_outages_map_like_the_local_client(hp):
    cfg, cl, pk = hp
    cl.fail = RuntimeError('POST /api/worker/ideas: HTTP 422 {"error": "refused by the legal rules"}')
    with pytest.raises(ApiError) as e:
        pk.add_idea("x", None, {}, "r")
    assert e.value.status == 422
    cl.fail = ConnectionRefusedError("down")
    with pytest.raises(ApiDown):
        pk.add_idea("x", None, {}, "r")
    assert pk.health() is False


def test_state_is_pushed_when_it_changes_and_pulled_only_when_missing(hp):
    cfg, cl, pk = hp
    g = cfg.comedy / "kb" / "gsync"
    g.mkdir(parents=True)
    (g / "map.json").write_text('{"docs": {}}', encoding="utf-8")
    pk.push_state(force_runs=True)
    assert set(cl.files) == {"kb/gsync/map.json"} and cl.calls[-1][2]["actor"] == "import:gsync"
    n = len(cl.calls)
    pk.push_state()
    assert len(cl.calls) == n, "nothing changed: nothing posted"
    (g / "map.json").write_text('{"docs": {"d": {}}}', encoding="utf-8")
    pk.push_state()
    assert json.loads(cl.files["kb/gsync/map.json"]) == {"docs": {"d": {}}}
    # a fresh PC: memos.json is in the database, not here -> downloaded; map.json is here -> never overwritten
    cl.files["kb/gsync/memos.json"] = '{"items": {}}'
    cl.files["kb/gsync/map.json"] = '{"docs": "SERVER COPY"}'
    got = pk.pull_state()
    assert got == ["kb/gsync/memos.json"] and (g / "memos.json").read_text() == '{"items": {}}'
    assert json.loads((g / "map.json").read_text()) == {"docs": {"d": {}}}


def _one_memo(cfg):
    cfg.notes_inbox.mkdir(parents=True, exist_ok=True)
    (cfg.notes_inbox / "aaa_memo.m4a").write_bytes(M4A)
    M.MemoStore(cfg.memos_state).set_item("drive1", source="drive", drive_id="drive1", status="downloaded",
                                          file="aaa_memo.m4a", name="New Recording 9.m4a", size=len(M4A))


def test_memo_ideas_are_import_memo(tmp_path):
    cfg = replace(make_cfg(tmp_path), memo_reserve_mb=0)
    pk = FakeApi(cfg)
    _one_memo(cfg)
    M.MemoProcessor(cfg, pk, transcriber=FakeWhisper(), normalizer=fake_normalize, prober=fake_probe(),
                    log=lambda *a: None, online=False).run()
    assert pk.posted and pk.posted[0]["actor"] == "import:memo"
    it = M.MemoStore(cfg.memos_state).load()["items"]["drive1"]
    assert it["status"] == "done" and it["audio"], "local mode (before the cutover) keeps the Opus copy as before"


class PushingApi(FakeApi):
    def __init__(self, cfg):
        super().__init__(cfg)
        self.pushed: list[list] = []

    def push_state(self, paths=None, force_runs=False):
        self.pushed.append(list(paths or []))
        return {"written": paths}


def test_online_memo_keeps_no_audio_and_posts_its_transcript(tmp_path):
    cfg = replace(make_cfg(tmp_path), memo_reserve_mb=0)
    pk = PushingApi(cfg)
    _one_memo(cfg)
    M.MemoProcessor(cfg, pk, transcriber=FakeWhisper(), normalizer=fake_normalize, prober=fake_probe(),
                    log=lambda *a: None, online=True).run()
    it = M.MemoStore(cfg.memos_state).load()["items"]["drive1"]
    assert it["status"] == "done" and it.get("audio") is None and it["drive_id"] == "drive1"
    assert not list(cfg.memo_audio.rglob("*.opus")) and not list(cfg.memo_audio.rglob("*.wav")), "no audio kept"
    assert not list(cfg.notes_inbox.glob("*.m4a")), "the downloaded copy is gone (Drive keeps the original)"
    assert pk.pushed and pk.pushed[-1] == [f"corpus/memos/{it['name_dir']}/transcript.json",
                                           f"corpus/memos/{it['name_dir']}/meta.json"]
