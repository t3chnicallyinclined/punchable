"""The formatted values every list row carries (T16, docs/ARCHITECTURE.md section 4): `venue_short` +
`venue_guessed` (the app's room rule, no year), `when_label` ("Mar 2025"), `at_label` ("3:07"), `short_name`,
`by_kind` (owner | import | ai). Over a synthetic graph in a temp folder: no data, no SurrealDB, never his files.

Additive only: the keys the shipped TestFlight build reads (the `SetCard`, `Moment` and tonight types in
app/src/lib/api.ts and searchRows.ts) are asserted present with the same type, so a rename or a type change here
fails before it reaches a phone.

    python -m pytest api-dev/tests/test_list_rows.py -q
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "tools" / "explorer"))  # kb.py; api-dev must come first: the explorer has a server.py too
sys.path.insert(0, str(DESK / "api-dev"))
sys.path.insert(0, str(DESK))
import profile_api as PF  # noqa: E402
import search_api as SR  # noqa: E402
import server as SV  # noqa: E402

GRAPH = {
    "version": "test", "generated_at": "2026-10-29", "runs": [],
    "nodes": [
        {"id": "joke:j1", "type": "joke", "title": "Umbrella at the airport (draft)", "stage": "working",
         "summary": "Airports treat an umbrella like a carry-on", "short_name": "Umbrella"},
        {"id": "unit:u1", "type": "unit", "text": "They gate-check my umbrella like it is a stroller", "date": "2025-01-02"},
        {"id": "joke:j2", "type": "joke", "title": "Haunted dishwasher saga of the century", "stage": "testing",
         "summary": "The dishwasher is haunted"},
        {"id": "set:2025-03-04_abc", "type": "set", "title": "Tuesday at the Cellar", "date": "2025-03-04",
         "venue": "The Cellar Comedy Club, Springfield", "duration_s": 600},
        {"id": "set:2024-07-19_def", "type": "set", "title": "Open mic", "date": "2024-07-19",
         "venue": "Unknown (title 'Beach jokes'; maybe Seaside)", "duration_s": 300},
        {"id": "perf:p1", "type": "performance", "set": "set:2025-03-04_abc", "start_ms": 187000, "end_ms": 220000,
         "text": "they gate-checked my umbrella again"},
        {"id": "perf:p2", "type": "performance", "set": "set:2024-07-19_def", "start_ms": 0, "end_ms": 30000,
         "text": "the dishwasher started humming again", "kind": "crowd_work", "name": "Front row"},
    ],
    "edges": [
        {"id": "e:a1", "kind": "part_of", "src": "unit:u1", "dst": "joke:j1", "status": "confirmed"},
        {"id": "e:a3", "kind": "performed_as", "src": "perf:p1", "dst": "joke:j1", "status": "confirmed"},
    ],
}

# What the shipped build reads (app/src/lib/api.ts SetCard + jokes[]; searchRows.ts Moment): key -> accepted types.
SHIPPED_SET_CARD = {"id": str, "title": str, "date": str, "venue": str, "duration_s": (int, float), "performances": int,
                    "laughs": int, "has_audio": bool, "jokes": list, "rateable": int, "rated": int, "reviewed": bool}
SHIPPED_SET_JOKE = {"perf": str, "title": str, "joke": (str, type(None)), "kind": (str, type(None)),
                    "rating": (str, type(None)), "rating_by": (str, type(None)), "start_ms": int}
SHIPPED_MOMENT = {"id": str, "set": str, "set_title": str, "venue": str, "date": str, "start_ms": int, "end_ms": int,
                  "text": str, "joke": (str, type(None)), "joke_title": (str, type(None))}


@pytest.fixture(scope="module")
def kb(tmp_path_factory):
    root = tmp_path_factory.mktemp("rows")
    for d in ("kb", "corpus/sets", "tapes/inbox"):
        (root / d).mkdir(parents=True)
    (root / "kb" / "graph.json").write_text(json.dumps(GRAPH), encoding="utf-8")
    (root / "kb" / "decisions.jsonl").write_text("", encoding="utf-8")
    return SV.AppKB(root / "kb", root / "corpus", root / "tapes", root / "tapes" / "inbox", None)


def typed(row: dict, shipped: dict):
    for k, t in shipped.items():
        assert k in row, f"shipped key {k} missing"
        assert isinstance(row[k], t), (k, row[k])


def test_set_cards_carry_the_room_the_month_and_each_rows_name_and_place(kb):
    a = kb.app()
    cards = {c["id"]: c for c in (SV.set_card(kb, a, s) for s in a.sets)}
    c = cards["set:2025-03-04_abc"]
    typed(c, SHIPPED_SET_CARD)
    assert (c["venue_short"], c["venue_guessed"], c["when_label"]) == ("The Cellar", False, "Mar 2025")
    assert c["venue"] == "The Cellar Comedy Club, Springfield", "the raw venue is untouched"
    j = c["jokes"][0]
    typed(j, SHIPPED_SET_JOKE)
    assert (j["short_name"], j["at_label"]) == ("Umbrella", "3:07")   # his own short name wins; 187000 ms -> 3:07
    g = cards["set:2024-07-19_def"]
    assert (g["venue_short"], g["venue_guessed"], g["when_label"]) == ("Seaside?", True, "Jul 2024")
    assert g["jokes"][0]["short_name"] == "Front row" and g["jokes"][0]["at_label"] == "0:00"


def test_moments_carry_the_same_values_and_the_bits_short_name(kb):
    a = kb.app()
    m = SR.moment_item(a.v, "perf:p1")
    typed(m, SHIPPED_MOMENT)
    assert (m["venue_short"], m["venue_guessed"], m["when_label"], m["at_label"]) == ("The Cellar", False, "Mar 2025", "3:07")
    assert m["short_name"] == "Umbrella"
    m2 = SR.moment_item(a.v, "perf:p2")
    assert (m2["venue_short"], m2["venue_guessed"], m2["at_label"], m2["short_name"]) == ("Seaside?", True, "0:00", None)


def test_joke_rows_and_tonight_items_carry_short_name(kb):
    a = kb.app()
    rows = {x["id"]: x for x in a.summaries()}
    assert rows["joke:j1"]["short_name"] == "Umbrella"
    assert rows["joke:j2"]["short_name"] == "Haunted dishwasher"      # the title cut at a word break inside 18
    assert SV.brief_joke(a, "joke:j2")["short_name"] == "Haunted dishwasher"
    items = SV.tonight(a)["items"]
    assert items and all(it["short_name"] for it in items)


def test_when_and_at_labels_are_empty_not_wrong_without_a_value():
    assert SV.when_label(None) == "" and SV.when_label("") == "" and SV.when_label("2015") == "2015"
    assert SV.at_label(None) is None and SV.at_label(True) is None and SV.at_label(0) == "0:00"
    assert SV.at_label(59_500) == "1:00" and SV.at_label(60_499) == "1:00" and SV.at_label(3_600_000) == "60:00"
    assert SV.venue_fields(None) == {"venue_short": "Unknown venue", "venue_guessed": True}


def test_the_profile_view_says_who_in_three_words():
    doc = {"id": "profile:voice", "kind": "voice", "text": "my voice", "updated_at": "2026-10-29T00:00:00+00:00",
           "by": "owner", "previous": {"text": "older", "updated_at": "2026-10-01T00:00:00+00:00", "by": "ai:voice"}}
    v = PF.view(doc)
    assert v["by"] == "owner" and v["by_kind"] == "owner"
    assert v["previous"]["by"] == "ai:voice" and v["previous"]["by_kind"] == "ai"
    assert PF.view(None)["by_kind"] is None
    w = PF.view({**doc, "by": "import:gsync"}, worker=True)
    assert w["by_kind"] == "import" and "previous" not in w


def test_decision_rows_carry_by_kind():
    assert SV.by_kind("owner") == "owner" and SV.by_kind("import:upload") == "import" and SV.by_kind("ai:merge") == "ai"
