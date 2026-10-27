"""AI rating of one joke against its room. Any laugh is a hit.

killed  a laugh, and (a 4-bar applause break, or a 3-bar laugh at >= 1.0x the room's
        laugh-time rate, or >= 1.75x the room's rate)
worked  a laugh, and (>= 0.6x the room's rate or a 2+ bar laugh)
meh     only chuckles, below 0.6x the room's rate
died    no laugh detected
A room is SPARSE when the detector found under 1.5 laughs/min all set; then
'died' is weak evidence and the basis says so."""

ORDER = {"killed": 3, "worked": 2, "meh": 1, "died": 0}
SPARSE_LPM = 1.5


def mmss(ms):
    s = int(round(ms / 1000))
    return f"{s // 60}:{s % 60:02d}"


def rate(seg_laughs, dur_ms, rm):
    n = len(seg_laughs)
    mx = max((l["bars"] for l in seg_laughs), default=0)
    ls = sum((l["end_ms"] - l["start_ms"]) / 1000 for l in seg_laughs)
    dens = ls / max(dur_ms / 60000, 0.25)
    rel = dens / rm["laugh_s_per_min"] if rm["laugh_s_per_min"] > 0 else 0
    sparse = rm["laughs_per_min"] < SPARSE_LPM
    if n == 0:
        r = "died"
    elif mx >= 4 or (mx >= 3 and rel >= 1.0) or rel >= 1.75:
        r = "killed"
    elif rel >= 0.6 or mx >= 2:
        r = "worked"
    else:
        r = "meh"
    room_s = f"room: {rm['laughs']} laughs in {mmss(rm['minutes'] * 60000)}"
    if n == 0:
        basis = f"No laugh detected in {mmss(dur_ms)} ({room_s})"
        if sparse:
            basis += "; the detector heard very little all set, so this is weak evidence"
    else:
        basis = (f"{n} laugh{'s' if n > 1 else ''} (best {mx} bar{'s' if mx != 1 else ''}, {ls:.1f} s) in {mmss(dur_ms)}"
                 f" = {rel:.1f}x the room's laugh-time rate ({room_s})")
    return r, basis, {"count": n, "max_bars": mx, "total_s": round(ls, 1)}, rel
