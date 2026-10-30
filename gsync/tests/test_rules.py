"""The legal rule: generic defaults, a private local file that can only add, and a legal test that can go red."""
from __future__ import annotations

import json

import pytest

from gsync import rules
from gsync.sync import Sync

from .fakes import FakeDrive, FakePunchable, clock, make_cfg
from .test_sync import FOLDER_ID, assert_legal_never_ingested


def test_generic_defaults_hold_nothing_personal():
    assert {'petition', 'court', 'affidavit', 'exhibit'} <= set(rules.KW)
    assert 'order to show cause' in rules.FILING_KW and 'index no' in rules.CONTENT_MARKERS
    assert len(rules.fingerprint()) == 64


def test_local_rules_file_only_adds(tmp_path, monkeypatch):
    f = tmp_path / "legal_rules.json"
    f.write_text(json.dumps({"owner": "me@example.com", "legal_parents": ["fold1"], "keywords": ["Sample Person"],
                             "legal_ids": ["doc1"]}), encoding="utf-8")
    monkeypatch.setenv("PUNCHABLE_LEGAL_RULES", str(f))
    loaded = rules._local_rules()
    assert loaded["legal_parents"] == ["fold1"] and loaded["keywords"] == ["Sample Person"]
    monkeypatch.setenv("PUNCHABLE_LEGAL_RULES", str(tmp_path / "missing.json"))
    assert rules._local_rules() == {}


def test_examples():
    me = [{"emailAddress": "owner@example.com"}]
    assert rules.legal_reason({"id": "a", "name": "Notes about Sample Person", "owners": me}) == "kw:sample person"
    assert rules.legal_reason({"id": "a", "name": "MEMORANDUM OF LAW", "owners": me}).startswith("filing")
    assert rules.legal_reason({"id": "a", "name": "Cover letter for OSC", "owners": me}).startswith("filing")
    assert rules.legal_reason({"id": "a", "name": "Rex tag", "owners": me}) is None
    assert rules.legal_reason({"id": "a", "name": "Judge joke", "owners": me}) is None
    assert rules.legal_reason({"id": "a", "name": "x", "owners": []}) == "owner"
    assert rules.content_is_legal("the judge said") is False
    assert rules.content_is_legal("SUPREME COURT OF THE STATE OF EXAMPLE\nDocket No. 1") is True
    assert rules.triage({"id": "a", "name": "the comic Resume"}) == "not_comedy"
    assert rules.triage({"id": "a", "name": "explaining wifi to my uncle"}) == "comedy"
    assert rules.triage({"id": "a", "name": "Untitled document"}) == "comedy"


@pytest.mark.parametrize("break_it", [
    lambda mp: mp.setattr(rules, "KW", [k for k in rules.KW if k != "lawsuit"]),
    lambda mp: mp.setattr(rules, "LEGAL_PARENTS", set()),
    lambda mp: mp.setattr(rules, "LEGAL_IDS", frozenset()),
    lambda mp: mp.setattr(rules, "legal_reason", lambda *a, **k: None),
], ids=["drop-keyword", "drop-folder", "drop-denylist", "rule-off"])
def test_broken_legal_rule_makes_the_legal_test_fail(tmp_path, monkeypatch, break_it):
    break_it(monkeypatch)
    cfg = make_cfg(tmp_path)
    d, pk = FakeDrive(), FakePunchable(cfg)
    d.folder(FOLDER_ID)

    def run(dry=False):
        return Sync(cfg, d, pk, dry_run=dry, clock=clock).run()
    with pytest.raises(AssertionError, match="legal doc was exported|leaked"):
        assert_legal_never_ingested(cfg, d, pk, run)


def test_own_keywords_only_ids_skip_only_the_generic_title_keywords(monkeypatch):
    me = [{"emailAddress": "owner@example.com"}]
    monkeypatch.setattr(rules, "OWN_KW", ["sample matter"])
    monkeypatch.setattr(rules, "OWN_KW_ONLY_IDS", frozenset({"bit1"}))
    # a bit whose title trips a generic keyword is checked against your own list only...
    assert rules.legal_reason({"id": "bit1", "name": "Illegal parking bit", "owners": me}) is None
    assert rules.legal_reason({"id": "other", "name": "Illegal parking bit", "owners": me}) == "kw:legal"
    # ...but your own keywords, the denylist, folders, owner and filing names still apply to it
    assert rules.legal_reason({"id": "bit1", "name": "sample matter notes", "owners": me}) == "kw:sample matter"
    assert rules.legal_reason({"id": "bit1", "name": "Memorandum of law", "owners": me}).startswith("filing")
    assert rules.legal_reason({"id": "bit1", "name": "x", "owners": []}) == "owner"
    monkeypatch.setattr(rules, "LEGAL_IDS", frozenset({"bit1"}))
    assert rules.legal_reason({"id": "bit1", "name": "Illegal parking bit", "owners": me}) == "denylist"


def test_own_keywords_only_ids_need_your_own_keywords(monkeypatch):
    me = [{"emailAddress": "owner@example.com"}]
    monkeypatch.setattr(rules, "OWN_KW", [])
    monkeypatch.setattr(rules, "OWN_KW_ONLY_IDS", frozenset({"bit1"}))
    assert rules.legal_reason({"id": "bit1", "name": "Legal notes", "owners": me}) == "kw:legal"


def test_local_content_markers_add_to_the_text_net(tmp_path, monkeypatch):
    f = tmp_path / "legal_rules.json"
    f.write_text(json.dumps({"content_markers": ["Sample County"]}), encoding="utf-8")
    monkeypatch.setenv("PUNCHABLE_LEGAL_RULES", str(f))
    import importlib
    fresh = importlib.reload(rules)
    try:
        assert "sample county" in fresh.CONTENT_MARKERS
        assert fresh.content_is_legal("SAMPLE COUNTY ... Docket No. 7") is True
        assert fresh.content_is_legal("a bit set in sample county") is False
    finally:
        monkeypatch.delenv("PUNCHABLE_LEGAL_RULES")
        importlib.reload(rules)
