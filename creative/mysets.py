"""Your sets: many sets kept at once (a 5, a 7, a 10, a 15, any name and length), each one opened and worked on.

the comic, 2026-10-26: "we can then have many 'sets' 5 min 7 min 10 min 15 min, whatever the set is, we can open it and
work on it, know whats working, where tough spots are".

A set IS a working set (creative/workingset.py): the same store (`workingsets.jsonl`), the same versions, bit locks and
try-tonight notes. Every working set he already has is one of his sets, so nothing migrates and nothing is lost. What
this module adds, all computed from his tapes at read time (nothing here is stored but his own actions):

- `bit_label`: per bit, what's working in words: killed / solid / mixed / quiet / untested, and over how many nights.
- `tough_spots`: per bit and per set: a quiet bit, a bit that's dropping lately, a long stretch with no laughs, a weak
  transition, a bit that ran long, the set over or short of its length. Each offers actions: swap (a stronger bit from
  his notebook on the same subject), tag (a line he wrote that could tag it), bridge (an AI draft, labelled), move,
  cut, add.
- `trim`: "make a 5 from my 10": keeps the opener and the closer and the strongest bits by stage history.
- `match_recording`: a new set recording lines up with the set it most resembles; that set's tapes and numbers follow.

Words only: no ratios, no rates, no jargon in any `words` / `label_words` / `status_words` / `why` field.
"""
from __future__ import annotations

from . import notebook as N
from . import story as S
from . import workingset as W
from .data import Corpus, venue_short
from .store import Store
from .util import median, mmss, sha8, slug

DEAD_ROOM = 2.0         # a set whose whole room laughed under this (laugh-s/min) barely laughed: its nights don't count
KILL_VR, GOOD_VR, SOME_VR = 1.2, 0.8, 0.5   # vs the room's own laugh rate on that night
LONG_QUIET_S = 40.0     # a stretch this long with no laugh inside one bit is a tough spot ...
LONG_QUIET_SHARE = 0.4  # ... when it's also this share of the bit
RAN_LONG = 1.5          # last night ran this many times its usual length (and 30 s over)
MATCH_MIN = 0.3         # a recording lines up with a set at this score or better (and 2+ shared bits)
LABEL_ORDER = {"killed": 4, "solid": 3, "mixed": 2, "untested": 1, "quiet": 0}
LENGTHS = [300, 420, 600, 900]


# ============================================================== what's working, per bit
def night_outcome(p: dict) -> str | None:
    """One night of one bit: killed | good | some | quiet, or None when the whole room barely laughed (not counted)."""
    rate = p.get("set_laugh_s_per_min")
    if rate is not None and rate < DEAD_ROOM:
        return None
    vr, r = p.get("vs_room"), p.get("ai_rating")
    if vr is None:
        return {"killed": "killed", "worked": "good", "meh": "some"}.get(r, "quiet")
    if r == "died" or vr <= 0:
        return "quiet"
    if vr >= KILL_VR or (r == "killed" and vr >= 1.0):
        return "killed"
    if vr >= GOOD_VR:
        return "good"
    return "some" if vr >= SOME_VR else "quiet"


def _where(p: dict) -> str:
    v = venue_short(p.get("venue"), p.get("date"))
    return "on stage" + (f" '{p['date'][2:4]}" if p.get("date") else "") if v.startswith("Unknown") else f"at {v}"


OUT_WORD = {"killed": "Killed", "good": "Worked", "some": "Some laughs", "quiet": "Quiet"}
OUT_RANK = {"killed": 3, "good": 2, "some": 1, "quiet": 0}


def _of(k: int, n: int) -> str:
    if k == n:
        return "both nights" if n == 2 else "every night" if n > 2 else ""
    return f"{k} of {n} nights"


def bit_label(c: Corpus, jid: str) -> dict:
    """{label, label_words, nights, counted, last_words, outcomes}. Rules (docs/API_CONTRACT.md "Your sets"):
    killed = no quiet night, killed on at least half and a hit on 2 of 3; solid = no quiet night and a hit on 2 of 3; quiet = no hit,
    or quiet on 2 of 3; mixed = the rest; untested = never on tape (or only in a room that barely laughed)."""
    ps = c.perfs.get(jid, [])
    outs = [(p, night_outcome(p)) for p in ps]
    counted = [(p, o) for p, o in outs if o]
    n, skipped = len(counted), len(outs) - len(counted)
    base = {"nights": len(ps), "counted": n, "outcomes": [o for _, o in counted]}
    if not n:
        return {**base, "label": "untested", "last_words": None,
                "label_words": "Not on stage yet" if not ps else "Only on tape in a room that barely laughed"}
    k = sum(1 for _, o in counted if o == "killed")
    hits = k + sum(1 for _, o in counted if o == "good")
    q = sum(1 for _, o in counted if o == "quiet")
    best = max(counted, key=lambda x: (OUT_RANK[x[1]], min(3.0, x[0].get("vs_room") or 0), x[0]["date"]))
    worst = min(counted, key=lambda x: (OUT_RANK[x[1]], min(3.0, x[0].get("vs_room") or 0), x[0]["date"]))
    if q == 0 and k and 2 * k >= n and 3 * hits >= 2 * n:
        label = "killed"
        words = f"Killed {_where(best[0])}" if n == 1 else f"Killed {_of(k, n)}"
    elif q == 0 and 3 * hits >= 2 * n:
        label = "solid"
        words = f"Worked {_where(best[0])}" if n == 1 else f"Worked {_of(hits, n)}" + (f", killed {k}" if k else "")
    elif hits == 0 or 3 * q >= 2 * n:
        label = "quiet"
        if q == 0:
            words = "Some laughs, never a big one" + (f" · {n} nights" if n > 1 else "")
        else:
            words = f"Quiet {_where(worst[0])}" if n == 1 else f"Quiet {_of(q, n)}"
    else:
        label = "mixed"
        words = f"Mixed: {OUT_WORD[best[1]].lower()} {_where(best[0])}, {OUT_WORD[worst[1]].lower()} {_where(worst[0])}"
    if n > 1 and label != "mixed" and "nights" not in words:
        words += f" · {n} nights"
    last_p, last_o = counted[-1]
    return {**base, "label": label, "label_words": words, "skipped_dead_room": skipped,
            "last_words": f"Last time: {OUT_WORD[last_o].lower()} {_where(last_p)}"}


def strength(c: Corpus, jid: str, lab: dict | None = None) -> tuple:
    lab = lab or bit_label(c, jid)
    e, _ = c.energy(jid)
    return (LABEL_ORDER[lab["label"]], e or 0.0, lab["counted"])


# ============================================================== the set as a whole
def ids_of(ws: dict) -> list[str]:
    return [b["joke_id"] for b in W.current(ws)["bits"]]


def runtime(c: Corpus, ids: list[str]) -> tuple[float, int]:
    total, est = 0.0, 0
    for j in ids:
        d, _ = c.duration(j)
        total += d
        est += 0 if c.perfs.get(j) else 1
    return round(total, 1), est


def length_status(total: float, target_s: int) -> tuple[str, str]:
    lo, hi = N.target_band(target_s)
    if not target_s:
        return "none", mmss(total)
    if total < lo:
        return "short", f"{mmss(lo - total)} short"
    if total > hi:
        return "over", f"{mmss(total - hi)} over"
    return "on", "on target"


def length_words(target_s: int) -> str:
    m = target_s / 60
    return f"{int(m)} min" if m == int(m) else f"{m:.1f} min"


TSCORE = {"proven": 3, "link": 2, "weak": 1, "jump": 0}


def _tkind(t: dict) -> str:
    words, weak = N._transition_words(t)
    return "jump" if t["label"] == "jump" else ("weak" if weak else t["label"])


class _T:
    """Transition cache for one request (S.transition is the expensive part of a view)."""

    def __init__(self, c: Corpus):
        self.c, self.m = c, {}

    def __call__(self, a: str, b: str) -> dict:
        k = (a, b)
        if k not in self.m:
            self.m[k] = S.transition(self.c, a, b)
        return self.m[k]

    def score(self, a: str | None, b: str | None) -> int:
        return 2 if a is None or b is None else TSCORE[_tkind(self(a, b))]


# ============================================================== actions
def _card(c, jid, **extra):
    lab = bit_label(c, jid)
    return N.card(c, jid, None, label=lab["label"], label_words=lab["label_words"], **extra)


def swap_options(c: Corpus, ids: list[str], jid: str, cap: int = 2) -> list[dict]:
    """Stronger bits from his notebook that could take this bit's place: proven on tape (killed or solid), on the
    same subject as the bit or its neighbours first, and of a similar length."""
    i = ids.index(jid)
    near = [x for x in (ids[i - 1] if i else None, jid, ids[i + 1] if i + 1 < len(ids) else None) if x]
    d0, _ = c.duration(jid)
    mine = strength(c, jid)
    out = []
    for o in c.perfs:
        if o in ids or o not in c.jokes or c.stage(o)[0] == "retired":
            continue
        lab = bit_label(c, o)
        if lab["label"] not in ("killed", "solid"):
            continue
        st = strength(c, o, lab)
        if st <= mine:
            continue
        d, _ = c.duration(o)
        if not (0.4 * d0 <= d <= 2.0 * d0 + 30):
            continue
        sc, words, _ = S.same_subject(c, near, o)
        out.append((sc >= 0.35, st, -abs(d - d0), o, words))
    out.sort(key=lambda x: (not x[0], -x[1][0], -x[1][1], -x[2], x[3]))
    res = []
    for subj, st, _, o, words in out[:cap]:
        lab = bit_label(c, o)
        why = lab["label_words"] + (f"; same subject ({', '.join(words[:2])})" if subj and words else "")
        res.append({"kind": "swap", "joke_id": jid, "with": _card(c, o, why=why), "words": f"Swap in {N.short_title(c.title(o))}"})
    return res


def tag_option(c: Corpus, jid: str) -> dict | None:
    """A line he already wrote that could tag this bit (the joke map's "Could tag it")."""
    try:
        g = next((g for g in N.joke_map(c, jid, cap=3)["groups"] if g["key"] == "tags"), None)
    except Exception:  # noqa: BLE001  the map is a bonus; the spot still shows
        return None
    if not g or not g["items"]:
        return None
    it = g["items"][0]
    return {"kind": "tag", "joke_id": jid, "tag": it, "words": f"Tag it with {it['title']}"}


def move_option(c: Corpus, ids: list[str], jid: str, tr: _T, keep_ends: bool = True) -> dict | None:
    """The spot in the set where this bit links best to what's around it (better than where it is now)."""
    i = ids.index(jid)
    rest = [x for x in ids if x != jid]

    def score(seq, k):
        return tr.score(seq[k - 1] if k else None, seq[k]) + tr.score(seq[k], seq[k + 1] if k + 1 < len(seq) else None)

    now = score(ids, i) - (tr.score(ids[i - 1], ids[i + 1]) if 0 < i < len(ids) - 1 else 0)
    best = None
    lo, hi = (1, len(rest)) if keep_ends and len(rest) >= 2 else (0, len(rest) + 1)
    for k in range(lo, hi):
        if k == i:
            continue
        seq = rest[:k] + [jid] + rest[k:]
        gain = score(seq, k) - (tr.score(rest[k - 1], rest[k]) if 0 < k < len(rest) else 0)
        if best is None or gain > best[0]:
            best = (gain, k, seq)
    if not best or best[0] <= now:
        return None
    _, k, seq = best
    prev = seq[k - 1] if k else None
    t = tr(prev, jid) if prev else None
    tw = N._transition_words(t)[0] if t else None
    words = (f"Move it after {N.short_title(c.title(prev))}" if prev else "Open with it") + (f" ({tw.lower()})" if tw else "")
    return {"kind": "move", "joke_id": jid, "to": k, "words": words}


# ============================================================== tough spots
def _spot(kind, words, joke_id=None, actions=None, **extra):
    return {"kind": kind, "joke_id": joke_id, "words": words, "actions": actions or [], **extra}


def tough_spots(c: Corpus, ws: dict, with_actions: bool = True, tr: _T | None = None) -> dict:
    """{bits: {joke_id: [spot]}, set: [spot]}. A locked bit is perfected: it gets no spots of its own."""
    tr = tr or _T(c)
    ver = W.current(ws)
    ids = [b["joke_id"] for b in ver["bits"]]
    per = {j: [] for j in ids}
    set_spots = []
    for i, j in enumerate(ids):
        locked = W.bit_locked(ws, j)
        if i:
            t = tr(ids[i - 1], j)
            words, weak = N._transition_words(t)
            if weak:
                acts = []
                if with_actions:
                    acts.append({"kind": "bridge", "from": ids[i - 1], "to": j, "words": "Get a bridge idea (draft)"})
                    if not locked:
                        m = move_option(c, ids, j, tr)
                        if m:
                            acts.append(m)
                per[j].append(_spot("transition", f"From {N.short_title(c.title(ids[i - 1]))}: {words.lower()}", j, acts,
                                    **{"from": ids[i - 1], "to": j}))
        if locked:
            continue
        lab = bit_label(c, j)
        ps = [p for p in c.perfs.get(j, []) if night_outcome(p)]
        acts = lambda: ((swap_options(c, ids, j) + [x for x in [tag_option(c, j)] if x]) if with_actions else [])  # noqa: E731
        if lab["label"] == "quiet":
            per[j].append(_spot("quiet", f"Hasn't been landing: {lab['label_words']}", j, acts()))
        elif len(ps) >= 2 and night_outcome(ps[-1]) in ("quiet", "some") and any(night_outcome(p) in ("killed", "good") for p in ps[:-1]):
            hit = max(ps[:-1], key=lambda p: (min(3.0, p.get("vs_room") or 0), p["date"]))
            per[j].append(_spot("dropping", f"Dropping lately: {OUT_WORD[night_outcome(ps[-1])].lower()} {_where(ps[-1])} "
                                            f"after {OUT_WORD[night_outcome(hit)].lower()} {_where(hit)}", j, acts()))
        if ps:
            last = ps[-1]
            da = last.get("dead_air_max_s") or 0
            if da >= LONG_QUIET_S and da >= LONG_QUIET_SHARE * (last.get("duration_s") or 1) and lab["label"] != "quiet":
                t = tag_option(c, j) if with_actions else None
                per[j].append(_spot("stretch", f"A {int(da)}-second stretch with no laughs {_where(last)}", j, [x for x in [t] if x]))
            usual = median([p["duration_s"] for p in ps])
            if len(ps) >= 2 and last["duration_s"] > RAN_LONG * usual and last["duration_s"] - usual > 30:
                per[j].append(_spot("long", f"Ran {mmss(last['duration_s'])} {_where(last)}, usually {mmss(usual)}: trim the setup", j))
    # the set as a whole
    total, _ = runtime(c, ids)
    st, words = length_status(total, ver["target_s"])
    lo, hi = N.target_band(ver["target_s"])
    if st == "over" and len(ids) > 2:
        acts = []
        if with_actions:
            cands = [j for j in ids[1:-1] if not W.bit_locked(ws, j)]
            cands.sort(key=lambda j: (strength(c, j), j))
            for j in cands[:2]:
                d, _ = c.duration(j)
                acts.append({"kind": "cut", "joke_id": j, "words": f"Cut {N.short_title(c.title(j))} ({mmss(d)}, {bit_label(c, j)['label_words'][0].lower()}{bit_label(c, j)['label_words'][1:]})"})
        set_spots.append(_spot("over", f"{words} your {length_words(ver['target_s'])}", None, acts))
    elif st == "short" and ids:
        acts = []
        if with_actions:
            for x in S.best_next(c, {"id": ws["id"], "joke_ids": ids, "target_s": hi}, cap=2):
                acts.append({"kind": "add", "joke_id": x["joke_id"], "card": _card(c, x["joke_id"], why=N._plain_best_next(x["why"], N.short_title(c.title(ids[-1])))),
                             "words": f"Add {N.short_title(c.title(x['joke_id']))}"})
        set_spots.append(_spot("short", f"{words} of {length_words(lo)}", None, acts))
    if len(ids) >= 3 and not W.bit_locked(ws, ids[-1]):
        last = bit_label(c, ids[-1])
        if last["label"] in ("quiet", "mixed"):
            better = [j for j in ids[:-1] if bit_label(c, j)["label"] == "killed"]
            sug = max(better, key=lambda j: (strength(c, j), j)) if better else None
            acts = [{"kind": "move", "joke_id": sug, "to": len(ids) - 1, "words": f"Close on {N.short_title(c.title(sug))}"}] if sug and with_actions else []
            set_spots.append(_spot("closer", "The ending could be stronger" + (f": {N.short_title(c.title(sug))} kills more" if sug else ""), ids[-1], acts))
    return {"bits": per, "set": set_spots}


def health(c: Corpus, ws: dict, spots: dict | None = None) -> dict:
    ids = ids_of(ws)
    spots = spots or tough_spots(c, ws, with_actions=False)
    n = sum(len(v) for v in spots["bits"].values()) + len(spots["set"])
    if not ids:
        word = "empty"
    elif n:
        word = f"{n} tough spot{'s' if n != 1 else ''}"
    elif all(bit_label(c, j)["label"] == "untested" for j in ids):
        word = "untested"
    else:
        word = "solid"
    return {"word": word, "tough_n": n}


# ============================================================== recordings line up with the set they resemble
def performed_in(c: Corpus, set_key: str) -> list[str]:
    out = []
    for x in c.segments.get(set_key, []):
        if x.get("joke") and x.get("status") in ("known", "new") and x["joke"] not in out:
            out.append(x["joke"])
    return out


def match_score(plan: list[str], performed: list[str]) -> tuple[float, int]:
    shared = [j for j in plan if j in performed]
    if not plan or not performed or len(shared) < min(2, len(plan)):
        return 0.0, len(shared)
    rec, prec = len(shared) / len(plan), len(shared) / len(performed)
    f1 = 2 * rec * prec / (rec + prec)
    order = len(W._lcs(shared, [j for j in performed if j in shared])) / len(shared)
    return round(f1 * (0.8 + 0.2 * order), 3), len(shared)


def match_recording(c: Corpus, sets: list[dict], set_key: str) -> dict:
    """The set a recording most resembles. His own pick at import (`align_working_set` in the recording's meta.json,
    from the app's X-Align-Working-Set) wins over the score."""
    perf = performed_in(c, set_key)
    live = [w for w in sets if not w.get("deleted")]
    ranked = []
    for w in live:
        sc, n = match_score(ids_of(w), perf)
        ranked.append({"id": w["id"], "title": w["title"], "score": sc, "shared": n, "planned": len(ids_of(w))})
    ranked.sort(key=lambda r: (-r["score"], -r["shared"], r["id"]))
    pick = (c.metas.get(set_key) or {}).get("align_working_set")
    chosen = next((r for r in ranked if r["id"] == pick), None)
    how = "his pick" if chosen else None
    if not chosen and ranked and ranked[0]["score"] >= MATCH_MIN:
        chosen, how = ranked[0], "closest"
    s = c.sets.get(set_key) or {}
    out = {"set_key": set_key, "date": s.get("date"), "performed": len(perf), "match": None, "candidates": ranked[:3]}
    if chosen:
        out["match"] = {**chosen, "how": how,
                        "words": f"Lined up with {chosen['title']}: {chosen['shared']} of its {chosen['planned']} bits"}
    return out


def tapes_for(c: Corpus, sets: list[dict], ws: dict) -> list[str]:
    """Recordings whose closest set (or his pick) is this one, oldest first."""
    return [k for k in c.set_keys() if ((match_recording(c, sets, k)["match"] or {}).get("id") == ws["id"])]


def tapes_summary(c: Corpus, sets: list[dict], ws: dict) -> dict:
    keys = tapes_for(c, sets, ws)
    if not keys:
        return {"n": 0, "words": "No tape of this set yet. Import a recording and it lines up here.", "last": None}
    a = W.align(c, ws, keys[-1])
    t = a["totals"]
    last = {"set": keys[-1], "date": a["date"], "venue_label": a["venue_label"], "bits_done": t["bits_performed"],
            "bits_planned": t["bits_planned"], "dropped": a["dropped"], "same_order": a["order"]["same_order"],
            "run_s": t["performed_planned_s"] + t["added_s"]}
    where = _where({"venue": a["venue"], "date": a["date"]})
    words = (f"Last tape {where}: {t['bits_performed']} of {t['bits_planned']} bits"
             + ("" if a["order"]["same_order"] else ", in a different order"))
    return {"n": len(keys), "words": words, "last": last, "keys": keys}


# ============================================================== "make a 5 from my 10"
def trim(c: Corpus, ws: dict, target_s: int) -> dict:
    """Keep the opener and the closer, then the strongest bits by stage history (locked bits first), in the set's own
    order, until the length is reached."""
    ids = ids_of(ws)
    lo, hi = N.target_band(target_s)
    dur = {j: c.duration(j)[0] for j in ids}
    if len(ids) <= 2:
        keep = list(ids)
    else:
        ends = [ids[0], ids[-1]]
        budget = hi - sum(dur[j] for j in ends)
        mid = sorted(ids[1:-1], key=lambda j: (not W.bit_locked(ws, j), tuple(-x for x in strength(c, j)), j))
        chosen, have = set(), sum(dur[j] for j in ends)
        for j in mid:
            weak = bit_label(c, j)["label"] in ("quiet", "untested") and not W.bit_locked(ws, j)
            if dur[j] <= budget and not (weak and have >= lo):  # a quiet bit only to reach the length
                chosen.add(j)
                budget -= dur[j]
                have += dur[j]
        keep = [j for j in ids if j in ends or j in chosen]
    total = round(sum(dur[j] for j in keep), 1)
    st, words = length_status(total, target_s)
    cut = [j for j in ids if j not in keep]
    return {"from": ws["id"], "title": f"{length_words(target_s)} from {ws['title']}", "target_s": target_s,
            "runtime_s": total, "runtime": mmss(total), "status": st, "status_words": words,
            "keep": [_card(c, j, position=i) for i, j in enumerate(keep)],
            "cut": [_card(c, j) for j in cut],
            "why": "Keeps your opener and your closer, then the bits that do best on stage, in the same order."}


# ============================================================== writes (his actions, into the working-set store)
def create(c: Corpus, store: Store, title: str, target_s: int, joke_ids=(), note: str | None = None,
           storyboard_id: str | None = None, by: str = "owner", ts: str | None = None, id_: str | None = None) -> dict:
    bits = []
    for j in joke_ids:
        d, basis = c.duration(j)
        bits.append({"joke_id": j, "title": c.title(j), "target_s": int(round(d / 5.0) * 5), "target_basis": basis})
    id_ = id_ or f"ws:{slug(title, 28)}-{sha8([title, ts or '', len(store.items), list(joke_ids)])}"
    return store.append("promote", id_, {"title": title, "storyboard_id": storyboard_id, "bits": bits, "target_s": int(target_s),
                                         "note": note or "v1: a new set"}, by=by, ts=ts)


def change_note(c: Corpus, old: list[str], new: list[str], old_t: int, new_t: int) -> str:
    """What changed, in words, for the version history."""
    t = lambda j: N.short_title(c.title(j))  # noqa: E731
    parts = []
    added = [j for j in new if j not in old]
    gone = [j for j in old if j not in new]
    if len(added) == 1 and len(gone) == 1 and new.index(added[0]) == old.index(gone[0]):
        parts.append(f"Swapped {t(gone[0])} for {t(added[0])}")
        added, gone = [], []
    if added:
        parts.append("Added " + ", ".join(t(j) for j in added))
    if gone:
        parts.append("Cut " + ", ".join(t(j) for j in gone))
    kept_old = [j for j in old if j in new]
    kept_new = [j for j in new if j in old]
    if kept_old != kept_new:
        moved = [j for j in kept_new if kept_new.index(j) != kept_old.index(j)]
        # the one bit that moved the furthest is the one he dragged
        j = max(moved, key=lambda j: (abs(kept_new.index(j) - kept_old.index(j)), j)) if moved else None
        if j:
            parts.append(f"Moved {t(j)} to #{new.index(j) + 1}")
    if old_t != new_t:
        parts.append(f"Length {length_words(new_t)} (was {length_words(old_t)})")
    return "; ".join(parts) or "No change"


# ============================================================== views
def summary(c: Corpus, ws: dict) -> dict:
    ver = W.current(ws)
    ids = [b["joke_id"] for b in ver["bits"]]
    total, est = runtime(c, ids)
    st, words = length_status(total, ver["target_s"])
    h = health(c, ws)
    return {"id": ws["id"], "title": ws["title"], "target_s": ver["target_s"], "length_words": length_words(ver["target_s"]),
            "n": len(ids), "runtime_s": total, "runtime": mmss(total), "estimated_n": est, "status": st,
            "status_words": words, "health": h["word"], "tough_n": h["tough_n"], "version": ver["v"],
            "storyboard_id": ws.get("storyboard_id"), "updated": ws["updated_at"], "source": ws.get("source"),
            "made_by": "you" if ws.get("by") == "owner" else "Punchable"}


def view(c: Corpus, ws: dict, sets: list[dict] | None = None, with_actions: bool = True) -> dict:
    """Everything the set screen needs: the bits in order with run time vs the length, what's working per bit, every
    tough spot with its actions, locks, the version history and the last tape that lined up with it."""
    tr = _T(c)
    ver = W.current(ws)
    ids = [b["joke_id"] for b in ver["bits"]]
    spots = tough_spots(c, ws, with_actions=with_actions, tr=tr)
    total, est = runtime(c, ids)
    st, words = length_status(total, ver["target_s"])
    lo, hi = N.target_band(ver["target_s"])
    bits = []
    for i, j in enumerate(ids):
        lab = bit_label(c, j)
        tin = None
        if i:
            t = tr(ids[i - 1], j)
            tw, weak = N._transition_words(t)
            tin = {"words": tw, "weak": weak, "kind": t["label"], "from": ids[i - 1], "to": j}
        bits.append(N.card(c, j, None, position=i, label=lab["label"], label_words=lab["label_words"],
                           last_words=lab["last_words"], locked=W.bit_locked(ws, j), transition_in=tin,
                           tough=spots["bits"].get(j, [])))
    h = health(c, ws, spots)
    sets = sets if sets is not None else [ws]
    nxt = []
    if ids:
        nxt = [_card(c, x["joke_id"], why=N._plain_best_next(x["why"], N.short_title(c.title(ids[-1]))))
               for x in S.best_next(c, {"id": ws["id"], "joke_ids": ids, "target_s": hi}, cap=3)]
    return {
        "set": {"id": ws["id"], "title": ws["title"], "target_s": hi, "target_lo_s": lo,
                "length_words": length_words(ver["target_s"]), "version": ver["v"], "storyboard_id": ws.get("storyboard_id"),
                "created_at": ws.get("created_at"), "updated_at": ws.get("updated_at"),
                "made_by": "you" if ws.get("by") == "owner" else "Punchable"},
        "runtime_s": total, "runtime": mmss(total), "estimated_n": est, "status": st, "status_words": words,
        "health": h["word"], "tough_n": h["tough_n"],
        "bits": bits, "tough": spots["set"], "next": nxt,
        "tapes": tapes_summary(c, sets, ws),
        "versions": [{"v": v["v"], "created_at": v["created_at"], "note": v["note"], "n": len(v["bits"]),
                      "target_s": v["target_s"], "by": "you" if v.get("by") == "owner" else "Punchable",
                      "titles": [N.short_title(b["title"]) for b in v["bits"]]} for v in reversed(ws["versions"])],
    }
