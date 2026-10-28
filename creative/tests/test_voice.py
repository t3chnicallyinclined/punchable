"""The voice profile block (creative/voice.py): first system block, stable across tasks, long enough to cache, never
in a log; the worker's fetch cached by updated_at; the prompts' naming when PK_COMIC_NAME is unset. Invented text only.

    python -m pytest creative/tests/test_voice.py -q
"""
from __future__ import annotations

import json

import pytest

from creative import voice

# an invented profile of a realistic length (about 1,100 words): the real one is private and never in this repo
FAKE_PROFILE = "\n\n".join(
    f"## Section {i}\nHe talks fast and sits on the punch word. Example {i}: a line about a parrot that files its own "
    f"taxes, delivered flat, then one more beat where he polls the room about parrots. Repeat the word. Never explain."
    for i in range(1, 41))


@pytest.fixture(autouse=True)
def clean():
    voice.clear()
    yield
    voice.clear()


def test_no_profile_means_the_prompt_is_what_it_was():
    assert voice.system_block("creative_riff") is None and not voice.loaded()
    assert voice.messages("creative_riff", "task line", {"role": "user", "content": "u"}) == [
        {"role": "system", "content": "task line"}, {"role": "user", "content": "u"}]
    assert voice.status() == {"loaded": False, "chars": 0, "sha": None, "updated_at": None, "by": None, "source": None}


def test_profile_is_the_first_block_stable_across_tasks_and_over_the_cache_minimum():
    voice.set_profile(FAKE_PROFILE, "2026-10-28T10:00:00+00:00", "owner")
    riff = voice.messages("creative_riff", "riff task", {"role": "user", "content": "idea"})
    punch = voice.messages("creative_punchup", "punch task", {"role": "user", "content": "lines"})
    bridge = voice.messages("creative_bridge", "bridge task", {"role": "user", "content": "x"})
    ask = voice.messages("ask_chat", "ask task", {"role": "user", "content": "q"})
    for m in (riff, punch, bridge):
        assert [x["role"] for x in m] == ["system", "system", "user"]
        assert m[0]["content"].startswith(voice.HEAD) and FAKE_PROFILE in m[0]["content"]
        assert m[0]["content"].endswith(voice.CRAFT), "the craft rules are in the same (cached) block"
    # byte-identical first block across the three writing tasks: one cache entry serves them all
    assert riff[0]["content"] == punch[0]["content"] == bridge[0]["content"]
    assert riff[1]["content"] == "riff task" and punch[1]["content"] == "punch task"
    # Ask gets the profile too, with the reading note instead of the craft rules (it answers, it does not write)
    assert ask[0]["content"].startswith(voice.HEAD) and ask[0]["content"].endswith(voice.READING)
    assert "never quote it" in voice.READING
    # over the model minimum (512 tokens on Opus 5.5 / Sonnet 5.5), by a margin, under the pessimistic estimate
    assert voice.est_tokens(riff[0]["content"]) >= voice.MIN_CACHE_TOKENS * 2
    assert voice.cacheable("creative_riff") and voice.cacheable("ask_chat")
    # a short profile alone would not reach the minimum: cacheable() says so (the worker logs it, nothing breaks)
    voice.set_profile("He is dry.", "t", "owner")
    assert voice.loaded() and not voice.cacheable("creative_riff")


def test_craft_rules_name_the_rules_the_audit_asked_for_and_none_of_his_moves():
    c = voice.CRAFT.lower()
    assert "punch word lands last" in c and "no explaining the joke" in c and "`move`" in c
    assert "first person, spoken, short" in c
    # the profile's own words are the only place his moves are named: the public rules only point at it
    assert "the profile above names" in c


class FakeClient:
    def __init__(self, doc):
        self.doc, self.calls = doc, 0

    def call(self, method, path, body=None):
        assert (method, path) == ("GET", "/api/worker/profile/voice")
        self.calls += 1
        if isinstance(self.doc, Exception):
            raise self.doc
        return self.doc


def test_fetch_caches_by_updated_at_and_never_logs_the_text():
    lines = []
    log = lambda *a: lines.append(" ".join(str(x) for x in a))  # noqa: E731
    cl = FakeClient({"kind": "voice", "text": FAKE_PROFILE, "updated_at": "2026-10-28T10:00:00+00:00", "by": "owner"})
    r = voice.fetch(cl, log)
    assert r["loaded"] and r["cached"] is False and r["chars"] == len(FAKE_PROFILE) and r["updated_at"] == "2026-10-28T10:00:00+00:00"
    assert voice.loaded() and voice.profile()["text"] == FAKE_PROFILE and voice.profile()["source"] == "api"
    r2 = voice.fetch(cl, log)
    assert r2["cached"] is True and cl.calls == 2, "fetched per job, but the same updated_at replaces nothing"
    # a new version replaces the copy
    cl.doc = {**cl.doc, "text": FAKE_PROFILE + "\n\nNew paragraph.", "updated_at": "2026-10-29T00:00:00+00:00"}
    r3 = voice.fetch(cl, log)
    assert r3["cached"] is False and voice.profile()["text"].endswith("New paragraph.")
    # the API down: the last copy stays, the job goes on, the log says so
    down = FakeClient(RuntimeError("GET /api/worker/profile/voice: HTTP 503 down"))
    r4 = voice.fetch(down, log)
    assert r4["loaded"] and r4["error"] == "RuntimeError" and voice.profile()["text"].endswith("New paragraph.")
    # removed on the server: cleared, said once
    r5 = voice.fetch(FakeClient({"kind": "voice", "text": None}), log)
    assert not r5["loaded"] and not voice.loaded()
    blob = "\n".join(lines) + json.dumps([r, r2, r3, r4, r5])
    assert "parrot" not in blob and "Section 1" not in blob, "the profile text is never logged or reported"
    assert any(l.startswith("voice profile: loaded (") and "sha " in l for l in lines)
    assert any("not fetched (RuntimeError" in l and "using the copy from" in l for l in lines)
    assert any("removed on the server" in l for l in lines)


def test_naming_without_pk_comic_name_reads_as_a_sentence(monkeypatch):
    monkeypatch.delenv("PK_COMIC_NAME", raising=False)
    monkeypatch.delenv("PK_COMIC_FIRST", raising=False)
    assert voice.comic() == "the comic" and not voice.named()
    assert voice.material_owner() == "a stand-up comic's" and voice.whose_idea() == "His idea" and voice.he_could_say() == "he could say"
    monkeypatch.setenv("PK_COMIC_NAME", "Pat Example")
    monkeypatch.setenv("PK_COMIC_FIRST", "Pat")
    assert voice.named() and voice.material_owner() == "stand-up comic Pat Example's"
    assert voice.whose_idea() == "Pat's idea" and voice.he_could_say() == "Pat could say"
    monkeypatch.setenv("PK_COMIC_NAME", "the comic")  # the manifest default: the same as unset
    assert not voice.named()
