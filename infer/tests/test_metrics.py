"""Laugh-metric formulas on synthetic laughs (INFER.md "Laugh metrics").
Run: cd punchable && python -m pytest infer/tests -q"""
from infer import laughs as L
from infer import metrics as M
from infer.rollup import laugh_metrics


def lg(a, b, bars=2, seg="set", conf=0.9, overlap=0.0, id_=None):
    return {"id": id_ or f"l{a}", "start_ms": a, "end_ms": b, "bars": bars, "segment": seg,
            "confidence": conf, "detail": {"speech_overlap": overlap}}


# ---- interval primitives -------------------------------------------------

def test_union_and_subtract():
    assert M.union([(5, 8), (0, 3), (2, 4), (8, 9)]) == [(0, 4), (5, 9)]
    assert M.subtract([(0, 100)], [(10, 20), (50, 60)]) == [(0, 10), (20, 50), (60, 100)]
    assert M.intersect([(0, 30), (90, 120)], [(10, 100)]) == [(10, 30), (90, 100)]


# ---- merging -------------------------------------------------------------

def test_rolling_merges_gaps_under_one_second_only():
    iv = [(0, 1000), (1999, 3000), (4000, 5000), (6100, 7000)]
    # gap 999 ms -> merged; gap exactly 1000 ms -> NOT merged (strictly under 1.0 s); gap 1100 -> not
    assert M.rolling(iv) == [(0, 3000), (4000, 5000), (6100, 7000)]


def test_set_merged_count_and_longest_laugh_span():
    window = [(0, 60000)]
    counted = [lg(10000, 11000), lg(11500, 13000), lg(13800, 14000), lg(30000, 31000)]
    m = M.set_metrics(counted, window)
    assert m["laughs_merged"] == 2                      # 3 rolling events + 1
    assert m["laughs_per_min_merged"] == 2.0            # 2 per 1.0 min
    assert m["laugh_s_total"] == 3.7                    # 1.0 + 1.5 + 0.2 + 1.0; gaps are not laughing
    assert m["longest_laugh_s"] == 4.0                  # rolling span 10.0 -> 14.0 s, gaps included
    assert m["_longest"] == (10000, 14000)


# ---- set-level formulas --------------------------------------------------

def test_set_rates_pct_applause_dead_air():
    window = [(0, 120000)]                              # 2.0 min of material
    counted = [lg(20000, 26000), lg(60000, 72000, bars=4)]
    m = M.set_metrics(counted, window)
    assert m["material_s"] == 120.0
    assert m["laugh_s_total"] == 18.0
    assert m["laugh_s_per_min"] == 9.0                  # 18 s / 2 min
    assert m["pct_laughing"] == 15.0                    # 18 / 120
    assert m["applause_s"] == 12.0                      # bars == 4 only
    assert m["dead_air_max_s"] == 48.0                  # 72 s -> 120 s
    assert m["dead_air_max_at_ms"] == 72000


def test_overlapping_events_not_double_counted():
    m = M.set_metrics([lg(0, 5000), lg(3000, 8000)], [(0, 60000)])
    assert m["laugh_s_total"] == 8.0 and m["laughs_merged"] == 1


# ---- overlap with speech -------------------------------------------------

def test_laugh_over_speech_counts_in_full():
    # the room laughing over his next line is still laughing: speech is not subtracted
    over = lg(10000, 14000, overlap=0.8)
    m = M.set_metrics([over], [(0, 60000)])
    assert m["laugh_s_total"] == 4.0


def test_auto_mode_drops_weak_laughs_under_his_voice_only():
    raw = {"laughs": [lg(1000, 2000, conf=0.3, overlap=0.9, id_="weak_speech"),
                      lg(3000, 4000, conf=0.3, overlap=0.1, id_="weak_gap"),
                      lg(5000, 6000, conf=0.9, overlap=0.9, id_="strong_speech")]}
    kept = [l["id"] for l in L.counted(raw, None)]
    assert kept == ["weak_gap", "strong_speech"]
    # with a judge file, only its laugh_drop list removes events
    assert [l["id"] for l in L.counted(raw, {"laugh_drop": {"strong_speech": "his voice"}})] == ["weak_speech", "weak_gap"]


# ---- music and walk-on exclusion -----------------------------------------

def test_material_window_excludes_walk_on_intro_outro_and_mid_music():
    raw = {"laughs": [lg(2000, 9000, bars=4, seg="walk_on")],
           "excluded_music": [{"start_ms": 0, "end_ms": 5000, "where": "intro"},
                              {"start_ms": 100000, "end_ms": 110000, "where": "mid"},
                              {"start_ms": 170000, "end_ms": 190000, "where": "outro"}]}
    w = M.material_window(4000, 180000, raw)
    assert w == [(9000, 100000), (110000, 170000)]      # walk-on end .. outro start, mid cut out
    assert M.length_ms(w) == 151000


def test_music_is_not_dead_air_and_laughs_in_music_do_not_count():
    raw = {"laughs": [], "excluded_music": [{"start_ms": 30000, "end_ms": 50000, "where": "mid"}]}
    w = M.material_window(0, 80000, raw)                # 0-30 s and 50-80 s
    m = M.set_metrics([lg(40000, 45000), lg(10000, 12000)], w)
    assert m["laugh_s_total"] == 2.0                    # the laugh inside the music is cut
    assert m["dead_air_max_s"] == 30.0                  # 50 -> 80 s; the music span is not dead air
    assert m["pct_laughing"] == round(100 * 2 / 60, 1)


def test_outro_before_start_is_ignored():
    raw = {"laughs": [], "excluded_music": [{"start_ms": 1000, "end_ms": 2000, "where": "outro"}]}
    assert M.material_window(5000, 60000, raw) == [(5000, 60000)]


# ---- per joke --------------------------------------------------------------

def test_joke_window_extends_to_own_laugh_but_caps_at_next_joke():
    window = [(0, 300000)]
    own = [lg(58000, 66000)]
    assert M.joke_window(30000, 60000, own, 63000, window) == [(30000, 63000)]   # capped at next start
    assert M.joke_window(30000, 60000, own, 90000, window) == [(30000, 66000)]   # runs to laugh end
    assert M.joke_window(30000, 60000, [], 90000, window) == [(30000, 60000)]


def test_joke_metrics():
    jwin = [(0, 60000)]
    own = [lg(12000, 14000), lg(14500, 16000), lg(40000, 44000)]
    neighbour = lg(0, 3000)                             # previous joke's laugh spilling in
    jm = M.joke_metrics(0, own, own + [neighbour], jwin, set_lspm=5.0)
    assert jm["merged_count"] == 2
    assert jm["laugh_s"] == 7.5
    assert jm["laugh_s_per_min"] == 7.5
    assert jm["pct_laughing"] == 12.5
    assert jm["time_to_first_laugh_s"] == 12.0
    assert jm["dead_air_max_s"] == 24.0                 # 16 s -> 40 s
    assert jm["vs_room"] == 1.5


def test_joke_with_no_laugh():
    jm = M.joke_metrics(5000, [], [], [(5000, 35000)], set_lspm=0)
    assert jm["time_to_first_laugh_s"] is None and jm["dead_air_max_s"] == 30.0
    assert jm["pct_laughing"] == 0.0 and jm["vs_room"] is None


# ---- cross-set ---------------------------------------------------------------

def test_median_and_trend():
    assert M.median([3, 1, 2]) == 2 and M.median([4, 1, 2, 3]) == 2.5
    assert M.trend([("2020-01-01", 5.0)]) == (None, None)
    assert M.trend([("2020-01-01", 4.0), ("2021-01-01", 6.0)]) == ("up", 0.4)       # +2 / mean 5
    assert M.trend([("2020-01-01", 6.0), ("2021-01-01", 4.0)]) == ("down", -0.4)
    assert M.trend([("2020-01-01", 5.0), ("2021-01-01", 5.5)])[0] == "flat"          # +0.5 / 5.25 = 0.095
    # same-date performances average before fitting
    assert M.trend([("2020-01-01", 2.0), ("2020-01-01", 6.0), ("2021-01-01", 4.0)]) == ("flat", 0.0)
    assert M.trend([("2020-01-01", 0.0), ("2021-01-01", 0.0)]) == ("flat", 0.0)


def test_rollup_laugh_metrics():
    ps = [{"set": "a", "date": "2020-01-01", "laugh_s_per_min": 4.0, "pct_laughing": 6.7},
          {"set": "b", "date": "2022-01-01", "laugh_s_per_min": 8.0, "pct_laughing": 13.3}]
    m = laugh_metrics(ps)
    assert m["avg_laugh_s_per_min"] == 6.0 and m["best_laugh_s_per_min"] == 8.0 and m["best_set"] == "b"
    assert [r["pct_laughing"] for r in m["pct_laughing_by_set"]] == [6.7, 13.3]
    assert m["trend"] == "up"


def test_room_heat(monkeypatch, tmp_path):
    from infer import sets as S
    for d in ("x", "y", "z"):
        (tmp_path / d).mkdir()
    monkeypatch.setattr(S, "SETS", str(tmp_path))
    monkeypatch.setattr(S, "set_dirs", lambda: ["x", "y", "z"])
    fresh = {d: {"summary": {"laugh_s_per_min": v}} for d, v in (("x", 2.0), ("y", 4.0), ("z", 10.0))}
    out = S.room_heat(fresh)
    assert [out[d]["summary"]["room_heat"] for d in "xyz"] == [0.5, 1.0, 2.5]
    assert out["x"]["summary"]["room_heat_basis"] == {"median_laugh_s_per_min": 4.0, "sets": 3}
