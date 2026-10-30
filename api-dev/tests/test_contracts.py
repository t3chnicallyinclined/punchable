"""The API against contracts/*.json (the rules the app and the server must agree on, T16): the say classifier's
filler words and the crash scrubber's key list. Pure, no data.

    python -m pytest api-dev/tests/test_contracts.py -q
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "tools" / "explorer"))
sys.path.insert(0, str(DESK / "api-dev"))
sys.path.insert(0, str(DESK))
import say_api as SY  # noqa: E402
import scrub as SC  # noqa: E402

CONTRACTS = DESK / "contracts"


def load(name: str) -> dict:
    return json.loads((CONTRACTS / name).read_text(encoding="utf-8"))


def test_say_fillers_are_the_contracts_list_in_order():
    assert list(SY.FILLERS) == load("say_fillers.json")["fillers"]
    assert SY.classify("oh and so um what are my untested bits")[0] == "question"  # the fillers fall off the front


def test_sensitive_keys_contract_pins_what_the_scrubber_drops():
    c = load("sensitive_keys.json")
    assert c["keys"] == sorted(c["keys"]), "sorted, so a diff reads"
    assert set(c["keys"]) == set(SC.CONTRACT_KEYS)
    assert SC.APP_KEYS == {"marks", "segments", "punch", "evidence"} and SC.APP_KEYS <= SC.CONTRACT_KEYS
    assert SC.SENSITIVE_KEYS | SC.APP_KEYS == SC.CONTRACT_KEYS
    assert SC._KEY_HINT.pattern == "(" + "|".join(c["hint_words"]) + ")"
    for k in c["keys"]:
        assert SC.sensitive_key(k) and SC.sensitive_key(k.upper()) and SC.sensitive_key(k.replace("_", "-")), k


def test_the_apps_four_keys_are_scrubbed_here_too():
    out = SC.scrub_value({"marks": [1, 2], "segments": [{"text": "x"}], "punch": "a line", "evidence": [{"quote": "q"}],
                          "status": 200})
    assert out == {"marks": SC.REDACTED, "segments": SC.REDACTED, "punch": SC.REDACTED, "evidence": SC.REDACTED,
                   "status": 200}


def test_every_contract_file_is_listed_in_the_readme_and_has_a_rule_name():
    readme = (CONTRACTS / "README.md").read_text(encoding="utf-8")
    files = sorted(p.name for p in CONTRACTS.glob("*.json"))
    assert files == ["ratings.json", "say_fillers.json", "sensitive_keys.json", "venue_short.json"]
    for f in files:
        assert f"`{f}`" in readme, f
        assert load(f)["rule"] == f[:-5]
