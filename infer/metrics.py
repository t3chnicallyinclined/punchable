"""Laugh-time metrics for a set and for each joke in it (contract: INFER.md,
"Laugh metrics"; docs/KB_SCHEMA.md, "Set analysis").

Everything here is pure arithmetic on [start_ms, end_ms) intervals, so the unit
tests can feed it synthetic laughs.

Material window: from the first judged segment's start to the last judged
segment's end (so the MC's intro is out), then
  - start moved past every walk-on event and every intro music region,
  - end moved back to the start of any outro music region,
  - any other (mid-set) music region cut out, because the detector suppresses
    laughs under music and cannot hear them there.
Laughs are counted wherever they fall in that window, including over his
speech: the room laughing over his next line is still the room laughing."""

MERGE_GAP_MS = 1000  # laughs closer than this (gap < 1.0 s) are one rolling laugh


def union(iv):
    """Sorted, non-overlapping cover of the intervals (touching ones join)."""
    out = []
    for a, b in sorted((a, b) for a, b in iv if b > a):
        if out and a <= out[-1][1]:
            out[-1][1] = max(out[-1][1], b)
        else:
            out.append([a, b])
    return [tuple(x) for x in out]


def intersect(iv, window):
    """Parts of `iv` inside the (already unioned) `window` intervals."""
    out = []
    for a, b in iv:
        for wa, wb in window:
            lo, hi = max(a, wa), min(b, wb)
            if hi > lo:
                out.append((lo, hi))
    return union(out)


def subtract(iv, cuts):
    out = []
    for a, b in union(iv):
        cur = [(a, b)]
        for ca, cb in union(cuts):
            nxt = []
            for x, y in cur:
                if cb <= x or ca >= y:
                    nxt.append((x, y))
                else:
                    if ca > x:
                        nxt.append((x, ca))
                    if cb < y:
                        nxt.append((cb, y))
            cur = nxt
        out.extend(cur)
    return union(out)


def length_ms(iv):
    return sum(b - a for a, b in iv)


def rolling(iv, gap_ms=MERGE_GAP_MS):
    """Group laugh intervals into rolling laughs: the next one joins the current
    group when the gap between them is under gap_ms. Returns [(start, end)] spans."""
    out = []
    for a, b in union(iv):
        if out and a - out[-1][1] < gap_ms:
            out[-1][1] = b
        else:
            out.append([a, b])
    return [tuple(x) for x in out]


def longest_gap(window, covered):
    """Longest stretch of `window` with no `covered` interval: (ms, start_ms)."""
    gaps = subtract(window, covered)
    if not gaps:
        return 0, None
    a, b = max(gaps, key=lambda g: (g[1] - g[0], -g[0]))
    return b - a, a


def material_window(seg_start_ms, seg_end_ms, raw):
    """The intervals of the set that count as material (see module doc)."""
    t0, t1 = seg_start_ms, seg_end_ms
    for l in raw.get("laughs", []):
        if l.get("segment") == "walk_on":
            t0 = max(t0, l["end_ms"])
    cuts = []
    for m in raw.get("excluded_music", []):
        if m["where"] == "intro":
            t0 = max(t0, m["end_ms"])
        elif m["where"] == "outro":
            t1 = min(t1, m["start_ms"]) if m["start_ms"] > t0 else t1
        else:
            cuts.append((m["start_ms"], m["end_ms"]))
    return subtract([(t0, t1)], cuts) if t1 > t0 else []


def _s(ms, nd=2):
    return round(ms / 1000, nd)


def per_min(laugh_ms, window_ms):
    return round(laugh_ms / 1000 / (window_ms / 60000), 2) if window_ms > 0 else 0.0


def pct(laugh_ms, window_ms):
    return round(100 * laugh_ms / window_ms, 1) if window_ms > 0 else 0.0


def set_metrics(counted, window):
    """Set-level laugh metrics. `counted` are the counted detector events
    (dicts with start_ms, end_ms, bars); `window` is material_window(...)."""
    laugh_iv = intersect([(l["start_ms"], l["end_ms"]) for l in counted], window)
    appl_iv = intersect([(l["start_ms"], l["end_ms"]) for l in counted if l["bars"] == 4], window)
    win_ms, laugh_ms = length_ms(window), length_ms(laugh_iv)
    groups = rolling(laugh_iv)
    dead_ms, dead_at = longest_gap(window, laugh_iv)
    longest = max(groups, key=lambda g: (g[1] - g[0], -g[0])) if groups else None
    return {
        "material_s": _s(win_ms, 1),
        "laugh_s_total": _s(laugh_ms, 1),
        "laugh_s_per_min": per_min(laugh_ms, win_ms),
        "pct_laughing": pct(laugh_ms, win_ms),
        "laughs_merged": len(groups),
        "laughs_per_min_merged": round(len(groups) / (win_ms / 60000), 2) if win_ms > 0 else 0.0,
        "applause_s": _s(length_ms(appl_iv), 1),
        "longest_laugh_s": _s(longest[1] - longest[0], 1) if longest else 0.0,
        "_longest": longest,
        "dead_air_max_s": _s(dead_ms, 1),
        "dead_air_max_at_ms": dead_at,
    }


def joke_window(start_ms, end_ms, own_laughs, cap_ms, window):
    """A joke's window: its first word to the later of its last word and the end
    of its own last laugh, capped at the next segment's start (or the material
    end), inside the material window."""
    end = max([end_ms] + [l["end_ms"] for l in own_laughs])
    if cap_ms is not None:
        end = max(end_ms, min(end, cap_ms))
    return intersect([(start_ms, end)], window)


def joke_metrics(start_ms, own_laughs, all_counted, jwin, set_lspm):
    """Per-joke laugh metrics. own_laughs = laughs attributed to this joke by
    punch word; all_counted = every counted laugh (for dead air: any laugh in
    the window, whoever's it is, means the room was not silent)."""
    own_iv = intersect([(l["start_ms"], l["end_ms"]) for l in own_laughs], jwin)
    any_iv = intersect([(l["start_ms"], l["end_ms"]) for l in all_counted], jwin)
    w_ms, l_ms = length_ms(jwin), length_ms(own_iv)
    lspm = per_min(l_ms, w_ms)
    dead_ms, _ = longest_gap(jwin, any_iv)
    first = min((l["start_ms"] for l in own_laughs), default=None)
    return {
        "merged_count": len(rolling(own_iv)),
        "laugh_s": _s(l_ms, 1),
        "window_s": _s(w_ms, 1),
        "laugh_s_per_min": lspm,
        "pct_laughing": pct(l_ms, w_ms),
        "time_to_first_laugh_s": _s(max(0, first - start_ms), 2) if first is not None else None,
        "dead_air_max_s": _s(dead_ms, 1),
        "vs_room": round(lspm / set_lspm, 2) if set_lspm > 0 else None,
    }


def median(xs):
    xs = sorted(xs)
    n = len(xs)
    if not n:
        return None
    return xs[n // 2] if n % 2 else (xs[n // 2 - 1] + xs[n // 2]) / 2


TREND_BAND = 0.20  # relative change over the span needed to call it up or down


def trend(points):
    """points: [(date 'YYYY-MM-DD', value)]. Values on the same date are averaged.
    With n >= 2 dates, fit least squares of value on date order (0..n-1); the
    relative change = slope * (n-1) / mean. up if > +0.20, down if < -0.20, else flat.
    Fewer than 2 dates: None."""
    by = {}
    for d, v in points:
        by.setdefault(d, []).append(v)
    vs = [sum(by[d]) / len(by[d]) for d in sorted(by)]
    n = len(vs)
    if n < 2:
        return None, None
    mx, my = (n - 1) / 2, sum(vs) / n
    slope = sum((i - mx) * (v - my) for i, v in enumerate(vs)) / sum((i - mx) ** 2 for i in range(n))
    if my <= 0:
        return "flat", 0.0
    rel = slope * (n - 1) / my
    return ("up" if rel > TREND_BAND else "down" if rel < -TREND_BAND else "flat"), round(rel, 2)
