"""The creative layer against contracts/*.json (the rules the app and the server must agree on, T16).

    python -m pytest creative/tests/test_contracts.py -q

`venue_short.json` was produced by running the app's own `venueShort` under node; `venue_label` must give the same
answer on every case. The prose `venue_short` is a different, older rule the pk-creative goldens pin: this file also
pins the five golden venues so a change to either rule is seen here before it reaches a golden.
"""
from __future__ import annotations

import json
from pathlib import Path

import pytest

from creative import mysets as M
from creative.data import RATING_ORDER, UNKNOWN_VENUE, by_kind, venue_label, venue_short
from creative.notebook import RATING_WORD

CONTRACTS = Path(__file__).resolve().parents[2] / "contracts"


def load(name: str) -> dict:
    return json.loads((CONTRACTS / name).read_text(encoding="utf-8"))


VENUES = load("venue_short.json")["cases"]


@pytest.mark.parametrize("case", VENUES, ids=[repr(c["venue"])[:40] for c in VENUES])
def test_venue_label_matches_the_apps_rule_on_every_contract_case(case):
    assert venue_label(case["venue"]) == {"name": case["name"], "guessed": case["guessed"]}


def test_the_contract_covers_every_branch_of_the_apps_rule():
    venues = [c["venue"] or "" for c in VENUES]
    assert len(VENUES) >= 30
    assert any(v == "" for v in venues) and any(v is None for v in (c["venue"] for c in VENUES))
    assert any(v.lower().startswith("unknown") and "maybe " in v.lower() for v in venues)
    assert any(v.lower().startswith("unknown") and "maybe " not in v.lower() for v in venues)
    assert any("likely" in v.lower() for v in venues) and any("(show)" in v.lower() for v in venues)
    assert any(v.lower().startswith("comix at ") for v in venues)
    assert any(v.endswith("Comedy Club") for v in venues) and any(v.endswith("Comedy Show") for v in venues)
    assert any("'" in v for v in venues) and any("," in v for v in venues)
    # "Comix at X" comes out the app's way ("Comix X"), never the old server way ("X")
    comix = next(c for c in VENUES if c["venue"] == "Comix at Riverside")
    assert comix["name"] == "Comix Riverside" and comix["guessed"] is False


def test_venue_label_never_carries_a_year_and_the_unknown_name_is_one_string():
    assert venue_label("Unknown")["name"] == UNKNOWN_VENUE == venue_label(None)["name"] == venue_label("  ")["name"]
    assert "'" not in venue_label("The Corner Room")["name"]


def test_prose_venue_short_is_the_older_rule_the_goldens_pin():
    """rust/tools/gen_creative_goldens.py ASK_SETS: these five values are in the pk-creative goldens as written."""
    assert venue_short("The Corner Room", None) == "The Corner Room"
    assert venue_short("Seaside Comedy Club", None) == "Seaside"
    assert venue_short("Mabel's (upstairs)", None) == "Mabel's"
    assert venue_short("Unknown", None) == "Unknown room"
    assert venue_short("maybe The Loft", None) == "The Loft?"
    assert venue_short("Mabel's (upstairs)", "2023-03-01") == "Mabel's '23"


def test_mysets_where_uses_the_apps_room_name_and_adds_the_year_itself():
    assert M._where({"venue": "Mabel's Comedy Club, Springfield", "date": "2024-07-19"}) == "at Mabel's '24"
    assert M._where({"venue": "Mabel's (upstairs)", "date": "2023-03-01"}) == "at Mabel's (upstairs) '23"
    assert M._where({"venue": "Unknown", "date": "2024-07-19"}) == "on stage '24"
    assert M._where({"venue": None, "date": None}) == "on stage"
    assert M._where({"venue": "Unknown (maybe Seaside)", "date": "2022-01-01"}) == "at Seaside? '22"


def test_ratings_contract_is_the_servers_vocabulary_in_order():
    r = load("ratings.json")
    assert r["order"] == sorted(RATING_ORDER, key=lambda k: -RATING_ORDER[k]) == ["killed", "worked", "meh", "died"]
    assert r["rank"] == RATING_ORDER
    assert r["words"] == RATING_WORD
    assert set(r["order"]) == set(r["rank"]) == set(r["words"])


def test_by_kind_is_the_his_material_rule_in_three_words(monkeypatch):
    import creative.data as D
    monkeypatch.setattr(D, "OWNER_ID", "owner")
    assert by_kind("owner") == "owner"
    assert by_kind("import:gsync") == "import" and by_kind("import:memo") == "import"
    assert by_kind("ai:intent") == "ai" and by_kind("ai:merge") == "ai"
    assert by_kind(None) is None and by_kind("") is None
    for who in ("owner", "import:upload", "ai:x"):
        assert (by_kind(who) in ("owner", "import")) == D.his_material(who)
