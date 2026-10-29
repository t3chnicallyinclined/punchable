"""The Bits list's sorts (api-dev/server.py sort_jokes): `recent` is what he touched last, first. Pure, no data.

    python -m pytest api-dev/tests/test_sorts.py -q
"""
from __future__ import annotations

import sys
from pathlib import Path

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK / "api-dev"))
sys.path.insert(0, str(DESK))
import server as SV  # noqa: E402


def test_recent_puts_the_last_touched_first_whatever_touched_it():
    items = [
        {"id": "joke:old", "title": "Old", "first_seen": "2023-01-01", "last_seen": "2023-01-01"},
        {"id": "joke:typed", "title": "Typed today", "first_seen": "2026-10-29", "last_seen": "2026-10-29", "touched": "2026-10-29T10:00:00+00:00"},
        {"id": "joke:edited", "title": "Edited later today", "first_seen": "2024-05-05", "last_seen": "2024-05-05", "touched": "2026-10-29T18:30:00+00:00"},
        {"id": "joke:staged", "title": "On stage last week", "first_seen": "2022-01-01", "last_seen": "2022-01-01", "last_performed": "2026-10-22"},
        {"id": "joke:written", "title": "New version this month", "first_seen": "2021-01-01", "last_seen": "2026-10-15"},
    ]
    out = [x["id"] for x in SV.sort_jokes(items, "recent")]
    assert out == ["joke:edited", "joke:typed", "joke:staged", "joke:written", "joke:old"]
    assert "recent" in SV.JOKE_SORTS
    # a date-only value and a same-day timestamp: the timestamp wins (it is the later of the two)
    assert SV.recent_key({"last_seen": "2026-10-29", "touched": "2026-10-29T01:00:00"}) == "2026-10-29T01:00:00"
    assert SV.recent_key({}) == ""


def test_other_sorts_still_hold():
    items = [{"id": "a", "title": "A", "last_seen": "2025-01-01"}, {"id": "b", "title": "B", "last_seen": "2026-01-01"}]
    assert [x["id"] for x in SV.sort_jokes(items, "written")] == ["b", "a"]
    assert [x["id"] for x in SV.sort_jokes(items, "untouched")] == ["a", "b"]
