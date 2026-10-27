"""Voice memos <-> his bits: vectors for new notes, same-idea suggestions, clear attaches, and "not a joke?" flags.

    python -m creative memolink embed            # vectors for every idea that has no units yet (summary:<joke>)
    python -m creative memolink judge            # + Claude judges the new same-idea candidate pairs (merge_judge)
    python -m creative memolink triage           # + Claude sorts each memo: joke material, a performed set, a
                                                  #   conversation, a work/personal note ... ("not a joke?" flags)
    python -m creative memolink attach [--apply] # + the clear ones attach to their bit (ai:attach; undo = detach)
    python -m creative memolink report           # counts and examples, no model calls

Why (2026-10-27): 144 voice-memo ideas were filed while the embedding install was missing, so they had no vectors
and same-idea matching ran on words alone. Here every summary-only idea gets its bge-small vector
(`summary:<joke>` in <vectors>/extra_summary.json, which creative.data.Vectors and merge.Index read), the merge
judge looks at the new candidate pairs, and the results show up where they already do in the app:
- "Same idea · Merge" groups: GET /api/merge/candidates reads merge_judge.json (his tap merges; nothing automatic);
- an attach only when it is CLEAR: the judge (Claude, offline) says same at >= ATTACH_JUDGE_MIN with one bit only,
  AND the word/meaning matcher (merge.match_text) independently places the note in that same bit. It is an
  `attach` event by `ai:attach` (undoable: POST /api/merge/undo detaches it); his words are never changed;
- "not a joke?": memos the triage judge calls a conversation or a work/personal note (>= NOTJOKE_MIN), listed for
  him to hide in one tap or keep (GET /api/suggestions/not-a-joke). Never hidden or deleted automatically.
- long memos (no idea filed: the transcript is over 4,000 characters): triage says which are performed sets
  (an audience, laughs, crowd work); those are routed to set analysis (worker job `memo_set`).
Court material never reaches a model: every text is checked with the legal rules first and skipped on a hit.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
from pathlib import Path

from . import merge as MG
from .data import Corpus

ATTACH_JUDGE_MIN = 0.90     # the judge's confidence a clear attach needs (the "same" bar for suggestions is 0.80)
NOTJOKE_MIN = 0.75          # triage confidence for a "not a joke?" flag
LONG_CHARS = 4000           # gsync/config.IDEA_MAX: longer memos were kept as notes, not filed as ideas
TRIAGE_KINDS = ("joke", "performed_set", "rehearsal", "conversation", "note", "unclear")
NOT_COMEDY = ("conversation", "note")
EMPTY_CHARS = 60            # an "unclear" memo this short ("Okay, here I am", "Ha ha!") is offered for hiding too
TRIAGE_BATCH = 8
TRIAGE_BATCH_LONG = 3
TRIAGE_SCHEMA = {
    "type": "object",
    "properties": {"items": {"type": "array", "items": {
        "type": "object",
        "properties": {"id": {"type": "string"}, "kind": {"type": "string", "enum": list(TRIAGE_KINDS)},
                       "audience": {"type": "boolean"}, "confidence": {"type": "number"}, "why": {"type": "string"}},
        "required": ["id", "kind", "audience", "confidence", "why"], "additionalProperties": False}}},
    "required": ["items"], "additionalProperties": False}
TRIAGE_SYSTEM = (
    "You sort one stand-up comedian's own voice memos (speech-to-text transcripts, so expect errors). For each memo "
    "pick ONE kind: joke = a joke, bit, premise, tag, story or riff he could use on stage, even rough or half-formed; "
    "performed_set = a recording of him performing to a live audience (audience laughter or reactions, crowd work, "
    "'give it up', 'thank you guys', a host bringing him up); rehearsal = him running material alone, as if on stage, "
    "with no audience; conversation = him talking with other people that is not him performing or working out a bit; "
    "note = a work, tech, business, shopping or personal note, reminder or message to someone, not comedy; unclear = "
    "too short or garbled to tell. A memo that is mostly comedy material is joke even if it wanders. audience = true "
    "only if the transcript itself shows a live audience. confidence is how sure you are (0-1). why is one plain "
    "sentence a comic would understand, quoting a few words when they decide it.")


def sha(text: str) -> str:
    return hashlib.sha256((text or "").encode("utf-8")).hexdigest()[:16]


_RULES: dict = {}


def _legal(text: str) -> bool:
    """gsync/rules.py's content check (loaded from its file once: the gsync package pulls in Drive code)."""
    if "mod" not in _RULES:
        import importlib.util
        spec = importlib.util.spec_from_file_location("pk_legal_rules", Path(__file__).resolve().parent.parent / "gsync" / "rules.py")
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        _RULES["mod"] = mod
    return _RULES["mod"].content_is_legal(text)


# ---------------------------------------------------------------------------------------------------------- inputs
def memos(comedy: Path) -> dict[str, dict]:
    """memos.json items that were transcribed (key -> item), with their transcript text."""
    p = Path(comedy) / "kb" / "gsync" / "memos.json"
    try:
        items = json.loads(p.read_text(encoding="utf-8")).get("items") or {}
    except (OSError, ValueError):
        return {}
    out = {}
    for k, it in items.items():
        if it.get("status") != "done" or not it.get("transcript"):
            continue
        tp = Path(comedy) / it["transcript"]
        try:
            segs = json.loads(tp.read_text(encoding="utf-8")).get("segments") or []
        except (OSError, ValueError):
            continue
        text = re.sub(r"\s+", " ", " ".join((s.get("text") or "").strip() for s in segs)).strip()
        if text:
            out[k] = {**it, "text": text}
    return out


def summary_only(c: Corpus) -> dict[str, str]:
    """Jokes with no units yet (an idea from a memo, the app, a Google Doc) -> the text the matcher sees."""
    ix = MG.index(c)
    return {j: " ".join((c.jokes[j].get("summary") or "").split()) for j in ix.jokes
            if not ix.units[j] and (c.jokes[j].get("summary") or "").strip()}


# ---------------------------------------------------------------------------------------------------------- vectors
def embed_summaries(c: Corpus, vec_dir: Path, embedder=None, log=print) -> dict:
    """bge-small vectors for every summary-only idea, keyed `summary:<joke>` (+ the text's hash, so an edited idea
    is embedded again). Written to <vec_dir>/extra_summary.json; returns counts."""
    vec_dir = Path(vec_dir)
    p = vec_dir / "extra_summary.json"
    try:
        doc = json.loads(p.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        doc = {"dim": 384, "model": "bge-small-en-v1.5", "rows": {}, "text_sha": {}}
    rows, tsha = doc.setdefault("rows", {}), doc.setdefault("text_sha", {})
    want = summary_only(c)
    todo = {j: t for j, t in want.items() if tsha.get(f"summary:{j}") != sha(t) and not _legal(t)}
    if todo:
        if embedder is None:
            import sys
            sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "db"))
            from embedder import Embedder
            embedder = Embedder()
            if embedder.engine != "python":
                log("note: the Python bge-small model is not here (python tools/kb/fetch_bge.py); trying node")
        keys = sorted(todo)
        vecs = embedder.embed([todo[j] for j in keys])
        for j, v in zip(keys, vecs):
            rows[f"summary:{j}"] = [float(f"{x:.7g}") for x in v]
            tsha[f"summary:{j}"] = sha(todo[j])
    gone = [k for k in rows if k[len("summary:"):] not in want]
    for k in gone:
        rows.pop(k, None)
        tsha.pop(k, None)
    if todo or gone:
        vec_dir.mkdir(parents=True, exist_ok=True)
        tmp = p.with_name(p.name + ".tmp")
        tmp.write_text(json.dumps(doc, separators=(",", ":")), encoding="utf-8")
        os.replace(tmp, p)
    return {"summary_only": len(want), "embedded": len(todo), "removed": len(gone), "have": len(rows), "file": str(p)}


# ---------------------------------------------------------------------------------------------------------- same idea
def memo_jokes(c: Corpus, ms: dict) -> dict[str, str]:
    """joke id -> memo key, for memo ideas still in the corpus."""
    return {it["joke_id"]: k for k, it in ms.items() if it.get("joke_id") in c.jokes}


def memo_candidates(c: Corpus, mj: dict) -> list[dict]:
    """The merge judge's candidate pairs (merge.judge_candidates) that involve a memo idea."""
    return [f for f in MG.judge_candidates(c) if f["a"] in mj or f["b"] in mj]


def judge_memos(c: Corpus, creative_dir: Path, mj: dict, complete, log=print) -> dict:
    """Claude (task merge_judge, offline, cached in merge_judge.json like `merge judge`) on the memo pairs."""
    from .util import write_json
    p = Path(creative_dir) / "merge_judge.json"
    cache = MG.load_judgements(creative_dir)
    before = len(cache)
    fs = memo_candidates(c, mj)
    for i in range(0, len(fs), MG.JUDGE_BATCH):
        MG.judge(c, fs[i:i + MG.JUDGE_BATCH], cache, complete, log)
        write_json(p, {"version": 1, "task": "merge_judge", "pairs": cache})
    MG.second_opinion(c, cache, complete, log)
    write_json(p, {"version": 1, "task": "merge_judge", "pairs": cache})
    return {"memo_pairs": len(fs), "newly_judged": len(cache) - before, "cache": len(cache)}


def set_recordings(creative_dir: Path) -> set[str]:
    """Joke ids of memo "ideas" the triage judge heard as a recording of a performed set (an audience). They hold
    several bits each, so they are not merged into one bit or attached to one: they are routed to set analysis
    (tools/ingest/memo_sets.py), which lines each performance up with his bits. Found 2026-10-27: 117 of the 148
    memo ideas were short set recordings (an opener, crowd work, several bits), and the same-idea judge grouped
    recordings of the same night together, which a merge would have collapsed into one joke."""
    try:
        items = json.loads((Path(creative_dir) / "suggest_judge.json").read_text(encoding="utf-8")).get("items") or {}
    except (OSError, ValueError):
        return set()
    return {v["joke_id"] for v in items.values() if v.get("kind") == "performed_set" and v.get("joke_id")}


def suggestions(c: Corpus, creative_dir: Path, mj: dict, items: dict | None = None) -> list[dict]:
    """The "Same idea · Merge" groups that include a memo idea (what GET /api/merge/candidates will show).
    Set recordings are left out (see set_recordings)."""
    items = items or {}
    g = MG.find_groups(c, MG.load_judgements(creative_dir), MG.dismissed_pairs(items),
                       exclude=set(MG.attached(items)) | set_recordings(creative_dir))
    return [x for x in g["groups"] if any(m["joke_id"] in mj for m in x["members"])]


def clear_attaches(c: Corpus, creative_dir: Path, mj: dict, items: dict, vectors=None) -> list[dict]:
    """Memo ideas that clearly ARE one existing bit: the judge (first look, or a confident second look) says same at
    >= ATTACH_JUDGE_MIN with exactly one bit (a joke with units, not another note), and match_text, on its own
    evidence, places the memo in that same bit (or its same-idea group). Anything less stays a suggestion."""
    ix = MG.index(c)
    judg = MG.load_judgements(creative_dir)
    attached, merged = MG.attached(items), {o for m in items.values() if m.get("kind") == "merge"
                                              and m.get("status") == "merged" for o in m.get("others", [])}
    dismissed = MG.dismissed_pairs(items)
    recordings = set_recordings(creative_dir)
    groups = MG.find_groups(c, judg, dismissed, exclude=set(attached) | recordings)["groups"]
    group_of = {m["joke_id"]: g["main"] for g in groups for m in g["members"]}
    sure: dict[str, list] = {}
    for a, b in sorted(ix.candidates()):
        if not ((a in mj) ^ (b in mj)):
            continue
        memo, bit = (a, b) if a in mj else (b, a)
        if not ix.units.get(bit) or frozenset((a, b)) in dismissed:
            continue
        f = ix.features(a, b)
        v = judg.get(MG.judge_key(c, ix, f))
        if not v or not v.get("same"):
            continue
        conf = v.get("confidence", 0)
        sec = v.get("second") or {}
        if sec.get("same"):
            conf = max(conf, sec.get("confidence", 0))
        if conf >= ATTACH_JUDGE_MIN:
            sure.setdefault(memo, []).append((bit, conf, v.get("why")))
    out = []
    for memo, hits in sorted(sure.items()):
        if memo in attached or memo in merged or memo in recordings:
            continue
        homes = {group_of.get(b, b) for b, _, _ in hits}
        if len(homes) != 1:
            continue  # the judge sees two different bits: a suggestion, not a clear attach
        home = homes.pop()
        text = " ".join((c.jokes[memo].get("summary") or "").split())
        vec = c.vectors.get(f"summary:{memo}")
        r = MG.match_text(c, text, vec, exclude={memo}, group_of=group_of)
        if r["decision"] != "attach" or group_of.get(r["joke_id"], r["joke_id"]) != home:
            continue
        bit, conf, why = max(hits, key=lambda h: h[1])
        out.append({"memo_joke": memo, "into": home, "into_title": c.title(home), "memo_title": c.title(memo),
                    "judge_confidence": round(conf, 2), "judge_why": why, "matcher": r["reasons"], "text": text,
                    "score": r.get("score")})
    return out


def attach_event(x: dict, memo_key: str | None) -> tuple[str, dict]:
    from .util import now_iso, sha8
    aid = "at:" + sha8([x["memo_joke"], x["into"]])
    return aid, {"joke": x["memo_joke"], "into": x["into"], "text": x["text"], "date": now_iso()[:10],
                 "score": x.get("score"), "source": "memo",
                 "reasons": [f"Claude: the same bit ({x['judge_confidence']:.2f}): {x['judge_why']}"] + list(x["matcher"])[:2]}


# ---------------------------------------------------------------------------------------------------------- triage
def _excerpt_long(t: str) -> str:
    if len(t) <= 3200:
        return t
    m = len(t) // 2
    return t[:1600] + " [...] " + t[m - 400:m + 400] + " [...] " + t[-800:]


def triage(ms: dict, cache_path: Path, complete, log=print, keys: list[str] | None = None) -> dict:
    """Claude (task memo_triage, offline) sorts each memo once; cached by the transcript's hash in
    kb/creative/suggest_judge.json. Court material is never sent (legal rules first)."""
    try:
        doc = json.loads(Path(cache_path).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        doc = {"version": 1, "task": "memo_triage", "items": {}}
    cache = doc.setdefault("items", {})
    keys = keys if keys is not None else sorted(ms)
    todo = [k for k in keys if k in ms and (cache.get(k) or {}).get("text_sha") != sha(ms[k]["text"])]
    skipped = [k for k in todo if _legal(ms[k]["text"])]
    todo = [k for k in todo if k not in skipped]
    short = [k for k in todo if len(ms[k]["text"]) <= LONG_CHARS]
    long_ = [k for k in todo if len(ms[k]["text"]) > LONG_CHARS]
    batches = [short[i:i + TRIAGE_BATCH] for i in range(0, len(short), TRIAGE_BATCH)] + \
              [long_[i:i + TRIAGE_BATCH_LONG] for i in range(0, len(long_), TRIAGE_BATCH_LONG)]
    done = 0
    for b in batches:
        lines = []
        for n, k in enumerate(b):
            it = ms[k]
            t = ms[k]["text"]
            lines.append(f"MEMO m{n + 1} (title '{it.get('title') or ''}', {int(it.get('duration_s') or 0)} s, "
                         f"{len(t)} characters{', excerpts' if len(t) > 3200 else ''}):\n{_excerpt_long(t) if len(t) > LONG_CHARS else t[:3200]}")
        try:
            r = complete("memo_triage", [{"role": "system", "content": TRIAGE_SYSTEM},
                                         {"role": "user", "content": "Sort each memo.\n\n" + "\n\n".join(lines)}],
                         TRIAGE_SCHEMA)
        except Exception as e:  # noqa: BLE001  never guess: an unanswered memo stays unsorted
            log(f"triage batch: {type(e).__name__}: {str(e)[:160]}")
            continue
        got = {x.get("id"): x for x in (r.data or {}).get("items") or []}
        for n, k in enumerate(b):
            x = got.get(f"m{n + 1}")
            if not x or x.get("kind") not in TRIAGE_KINDS:
                continue
            cache[k] = {"kind": x["kind"], "audience": bool(x.get("audience")), "confidence": float(x.get("confidence") or 0),
                        "why": str(x.get("why") or "")[:300], "text_sha": sha(ms[k]["text"]), "chars": len(ms[k]["text"]),
                        "joke_id": ms[k].get("joke_id"), "model": getattr(r, "model", None)}
            done += 1
        tmp = Path(cache_path).with_name(Path(cache_path).name + ".tmp")
        tmp.write_text(json.dumps(doc, ensure_ascii=False, indent=1), encoding="utf-8")
        os.replace(tmp, cache_path)
        log(f"triaged {done}/{len(todo)}")
    return {"asked": len(todo), "sorted": done, "legal_skipped": len(skipped), "cached": len(cache)}


def flagged(v: dict) -> bool:
    """A triage verdict that makes a "not a joke?" suggestion: a conversation or a note, or an "unclear" memo with
    almost nothing in it, at >= NOTJOKE_MIN confidence."""
    if v.get("confidence", 0) < NOTJOKE_MIN:
        return False
    return v.get("kind") in NOT_COMEDY or (v.get("kind") == "unclear" and (v.get("chars") or 0) <= EMPTY_CHARS)


def not_a_joke(cache: dict, ms: dict) -> list[dict]:
    """Memo ideas the triage judge confidently calls not comedy (a conversation, a work or personal note, or empty)."""
    out = []
    for k, v in cache.items():
        it = ms.get(k)
        if not it or not it.get("joke_id") or not flagged(v):
            continue
        if v.get("text_sha") != sha(it["text"]):
            continue
        out.append({"memo": k, "joke_id": it["joke_id"], "title": it.get("title"), "kind": v["kind"],
                    "confidence": v["confidence"], "why": v.get("why") or "", "date": it.get("date")})
    return sorted(out, key=lambda x: (x["date"] or "", x["memo"]))


def performed_sets(cache: dict, ms: dict) -> list[dict]:
    """Long memos (kept as notes, no idea filed) that the triage judge reads as a live set with an audience."""
    out = []
    for k, it in ms.items():
        v = cache.get(k) or {}
        if len(it["text"]) > LONG_CHARS and v.get("kind") == "performed_set" and v.get("audience"):
            out.append({"memo": k, "title": it.get("title"), "date": it.get("date"), "duration_s": it.get("duration_s"),
                        "confidence": v.get("confidence"), "why": v.get("why"), "name_dir": it.get("name_dir")})
    return sorted(out, key=lambda x: (x["date"] or "", x["memo"]))


# ---------------------------------------------------------------------------------------------------------- CLI
def cli(comedy: Path, creative_dir: Path, vec_dir: Path, args: list[str]) -> dict:
    from .data import load_corpus, Corpus as _C
    from .store import Store
    sub = args[0] if args else "report"
    apply = "--apply" in args
    ms = memos(comedy)
    out: dict = {"memos_transcribed": len(ms), "long_memos": sum(1 for it in ms.values() if len(it["text"]) > LONG_CHARS)}
    c = load_corpus(comedy)
    if sub in ("embed", "judge", "attach", "triage", "all"):
        out["embed"] = embed_summaries(c, vec_dir)
        c = _C.load(comedy)  # fresh: the new vectors
    mj = memo_jokes(c, ms)
    out["memo_ideas_in_corpus"] = len(mj)
    out["memo_ideas_with_vectors"] = sum(1 for j in mj if c.vectors.get(f"summary:{j}") is not None)
    complete = None
    if sub in ("judge", "all", "triage"):
        from .story import _creative_complete
        complete = _creative_complete()
    if sub in ("judge", "all"):
        out["judge"] = judge_memos(c, creative_dir, mj, complete)
    cache_path = Path(creative_dir) / "suggest_judge.json"
    if sub in ("triage", "all"):
        out["triage"] = triage(ms, cache_path, complete)
    st = Store(creative_dir, "merges", MG.store_apply)
    out["memo_candidate_pairs"] = len(memo_candidates(c, mj))
    sg = suggestions(c, creative_dir, mj, st.items)
    out["same_idea_groups_with_a_memo"] = len(sg)
    out["same_idea_examples"] = [{"main": g["main_title"], "members": [m["title"] for m in g["members"]],
                                  "confidence": g["confidence"], "why": (g["reasons"] or [""])[0]} for g in sg[:12]]
    clear = clear_attaches(c, creative_dir, mj, st.items)
    out["clear_attaches"] = [{k: x[k] for k in ("memo_title", "into_title", "judge_confidence", "judge_why")} for x in clear]
    if sub in ("attach", "all") and apply:
        done = []
        for x in clear:
            aid, data = attach_event(x, mj.get(x["memo_joke"]))
            if aid in st.items and st.items[aid].get("status") == "attached":
                continue
            st.append("attach", aid, data, by="ai:attach")
            done.append(aid)
        out["attached_now"] = done
    try:
        tc = json.loads(cache_path.read_text(encoding="utf-8")).get("items") or {}
    except (OSError, ValueError):
        tc = {}
    from collections import Counter
    out["triage_kinds"] = dict(Counter(v.get("kind") for k, v in tc.items() if k in ms))
    nj = not_a_joke(tc, ms)
    out["not_a_joke"] = len(nj)
    out["not_a_joke_examples"] = [{k: x[k] for k in ("title", "kind", "confidence", "why")} for x in nj[:10]]
    ps = performed_sets(tc, ms)
    out["long_memos_performed_sets"] = len(ps)
    out["performed_set_examples"] = [{k: x[k] for k in ("title", "date", "duration_s", "why")} for x in ps[:10]]
    return out
