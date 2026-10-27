"""Test-only legal rules: a sample legal folder, a denylisted doc, the owner and one local keyword.

Real deployments put theirs in ~/.punchable/legal_rules.json (gsync/rules.py); nothing private is in the repo."""
import pytest

from gsync import rules

from .fakes import ME

LEGAL_FOLDER = "sample-legal-folder-000000000000000"
DENYLISTED = "sample-denylisted-doc-0000000000000000000000"


@pytest.fixture(autouse=True)
def sample_legal_rules(monkeypatch):
    monkeypatch.setattr(rules, "LEGAL_PARENTS", {LEGAL_FOLDER})
    monkeypatch.setattr(rules, "LEGAL_IDS", frozenset({DENYLISTED}))
    monkeypatch.setattr(rules, "OWNER", ME)
    monkeypatch.setattr(rules, "KW", rules.KW + ["sample person"])
