"""Goldens for pk-creative's `merge` and `memolink` (the Rust port of creative/merge.py and creative/memolink.py's
same-idea and memo halves): a synthetic corpus built so every path fires (near-duplicate unit vectors above the
candidate line, his and inferred same-premise edges, shared titles, phrases and rare words, memo ideas with summary
vectors, a dismissed pair, a live attach, a memo heard as a performed set), run through the real Python, dumped as
JSON. Both sides read the identical vectors, so the logic is compared exactly; the similarity matrix itself is a BLAS
product in the Python and is compared to float32 precision. Nothing here is anyone's material.

    python rust/tools/gen_merge_goldens.py        (from the repo root; writes crates/pk-creative/tests/goldens/merge/)
"""
from __future__ import annotations

import pin_blas  # noqa: F401  (first: the BLAS kernel, so the numbers are the same on every machine)

import hashlib
import json
import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
OUT = ROOT / "rust" / "crates" / "pk-creative" / "tests" / "goldens" / "merge"

TOPICS = ["parrot lawyer", "airport bins", "landlord chat", "laundromat date", "gym mirror", "carousel luggage",
          "subway pigeon", "ticket machine", "dentist chatter", "self checkout", "smart fridge", "group photo",
          "wedding toast", "hotel breakfast", "rental car", "phone update", "jury duty", "neighbor drums", "dog park",
          "bank music"]


def rnd(seed: str):
    import numpy as np
    out, k = [], 0
    while len(out) < 384:
        out += [(x - 128) / 128 for x in hashlib.sha256(f"{seed}/{k}".encode()).digest()]
        k += 1
    return np.asarray(out[:384], dtype=np.float32)


def corpus_parts():
    """The graph, the decisions and the vector table."""
    import numpy as np
    nodes, edges, table, decisions = [], [], {}, []
    for n, topic in enumerate(TOPICS * 2):
        jid, uid = f"joke:m{n:02d}", f"unit:m{n:02d}"
        a, b = topic.split()
        # pairs (0, 20), (1, 21) ... are versions of one bit: the same words and nearly the same vector
        text = (f"So the {a} {b} thing, the {a} keeps doing the {b} routine every single morning. "
                f"Version {'two' if n >= 20 else 'one'} of the {a} bit with a {['tag', 'twist'][n % 2]}.")
        title = f"{topic.title()} {'again' if n >= 20 else ''}".strip()
        nodes.append({"id": jid, "type": "joke", "title": title, "stage": ["working", "idea", "testing"][n % 3],
                      "first_seen": f"2024-0{1 + n % 9}-01"})
        nodes.append({"id": uid, "type": "unit", "text": text, "date": f"2024-{1 + n % 12:02d}-{1 + n % 27:02d}",
                      **({"doc_id": f"doc-{n % 7}", "doc_title": f"Doc {n % 7}"} if n % 3 == 0 else {})})
        edges.append({"id": f"e:mp{n}", "kind": "part_of", "src": uid, "dst": jid, "confidence": 0.9})
        base = rnd(f"bit-{n % 20}")
        table[uid] = [float(x) for x in (base + (0.12 if n >= 20 else 0.0) * rnd(f"v-{n}")).astype(np.float32)]
    # same-premise edges: his (by the owner), and an inferred one
    edges.append({"id": "e:prem-his", "kind": "same_premise", "src": "joke:m06", "dst": "unit:m07", "by": "owner",
                  "confidence": 0.9, "reason": "both the subway"})
    edges.append({"id": "e:prem-ai", "kind": "same_premise", "src": "joke:m08", "dst": "joke:m09", "status": "inferred",
                  "confidence": 0.7, "reason": "maybe"})
    # memo ideas (summary only, added as imports of his own memos), with summary vectors
    memos = [("memo-a", "the parrot lawyer thing again, the parrot keeps doing the lawyer routine", "bit-0", 0.1),
             ("memo-b", "a long rant about the airport bins and the landlord chat at once", "bit-1", 0.9),
             ("memo-c", "totally new premise about volcano insurance and my cousin", "fresh", 0.0),
             ("memo-d", "the subway pigeon walks into the subway car like a commuter", "bit-6", 0.2)]
    for k, summary, seed, noise in memos:
        decisions.append({"by": "import:memo", "target": "node", "id": f"joke:{k}", "action": "add",
                          "after": {"type": "joke", "title": f"Memo {k}", "summary": summary, "date": "2024-08-01"}})
        v = rnd(seed) + noise * rnd(f"noise-{k}") if seed != "fresh" else rnd("fresh")
        table[f"summary:joke:{k}"] = [float(x) for x in v.astype(np.float32)]
    return {"nodes": nodes, "edges": edges}, decisions, table


def files(tmp: Path) -> dict:
    """The memo files the worker mirrors: kb/gsync/memos.json and each memo's transcript."""
    items = {}
    out = {}
    for k, text, joke in [("memo-a", "the parrot lawyer thing again", "joke:memo-a"),
                          ("memo-b", "airport bins and landlord chat " * 300, "joke:memo-b"),
                          ("memo-c", "volcano insurance", "joke:memo-c"),
                          ("memo-d", "subway pigeon commuter", "joke:memo-d"),
                          ("memo-x", "no transcript yet", None)]:
        items[k] = {"status": "done" if joke else "queued", "transcript": f"corpus/memos/{k}/transcript.json",
                    "joke_id": joke, "title": f"Memo {k}", "duration_s": 42.5, "date": "2024-08-01"}
        out[f"corpus/memos/{k}/transcript.json"] = json.dumps({"segments": [{"text": " " + t + " "} for t in text.split(". ")]})
    out["kb/gsync/memos.json"] = json.dumps({"items": items})
    return out


class Said:
    def __init__(self, data, model="claude-haiku-5-5"):
        self.data, self.model, self.provider = data, model, "anthropic"


def main():
    from creative import data as D
    from creative import memolink as ML
    from creative import merge as MG
    graph, decisions, table = corpus_parts()
    c = D.Corpus(graph, decisions=decisions, vectors=D.Vectors(table=table))
    ix = MG.index(c)
    cands = sorted(ix.candidates())
    feats = {f"{a}|{b}": ix.features(a, b) for a, b in cands}
    jc = MG.judge_candidates(c)
    # the judge: every candidate answered, a spread of verdicts (same sure, same borderline, different)
    asked, cache = [], {}

    def judge(task, msgs, schema):
        asked.append(msgs[-1]["content"])
        n = msgs[-1]["content"].count("PAIR p")
        return Said({"pairs": [{"id": f"p{k + 1}", "same": k % 3 != 2, "confidence": [0.95, 0.7, 0.9][k % 3],
                                "why": f"verdict {k}"} for k in range(n)]})

    MG.judge(c, jc, cache, judge)
    second_asked = []

    def second(task, msgs, schema):
        second_asked.append(msgs[-1]["content"])
        n = msgs[-1]["content"].count("PAIR p")
        return Said({"pairs": [{"id": f"p{k + 1}", "same": True, "confidence": 0.85, "why": "on a closer look"}
                               for k in range(n)]})

    second_todo = [k for k, v in sorted(cache.items()) if v.get("same") and MG.SECOND_LO <= v.get("confidence", 0) < MG.SAME_MIN_CONF
                   and "second" not in v]
    MG.second_opinion(c, cache, second)
    items = {"d1": {"id": "d1", "kind": "dismiss", "status": "dismissed", "jokes": ["joke:m02", "joke:m22"]},
             "a1": {"id": "a1", "kind": "attach", "status": "attached", "joke": "joke:memo-d", "into": "joke:m06"}}
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        fs = files(tmp)
        for rel, text in fs.items():
            (tmp / rel).parent.mkdir(parents=True, exist_ok=True)
            (tmp / rel).write_text(text, encoding="utf-8")
        ms = ML.memos(tmp)
        mj = ML.memo_jokes(c, ms)
        cdir = tmp / "kb" / "creative"
        cdir.mkdir(parents=True, exist_ok=True)
        (cdir / "merge_judge.json").write_text(json.dumps({"version": 1, "task": "merge_judge", "pairs": cache}),
                                               encoding="utf-8")
        triage_asked = []

        def triage(task, msgs, schema):
            triage_asked.append(msgs[-1]["content"])
            n = msgs[-1]["content"].count("MEMO m")
            return Said({"items": [{"id": f"m{k + 1}", "kind": ["joke", "performed_set", "note", "bogus"][k % 4],
                                    "audience": k % 2 == 1, "confidence": 0.8, "why": f"memo {k}"} for k in range(n)]})

        tri = ML.triage(ms, cdir / "suggest_judge.json", triage, log=lambda *a: None)
        triage_doc = json.loads((cdir / "suggest_judge.json").read_text(encoding="utf-8"))
        sugg = ML.suggestions(c, cdir, mj, items)
        clear = ML.clear_attaches(c, cdir, mj, items)
        events = [ML.attach_event(x, mj.get(x["memo_joke"])) for x in clear]
    groups = MG.build_groups(c, cache, MG.dismissed_pairs(items), exclude=set(MG.attached(items)))
    texts = ["the parrot lawyer thing again, the parrot keeps doing the lawyer routine every single morning",
             "So the airport bins thing, the airport keeps doing the bins routine every single morning.",
             "short one", "a completely new bit about volcano insurance and a cousin who sells it"]
    matches = []
    for t in texts:
        for vk in (None, "summary:joke:memo-a", "summary:joke:memo-b"):
            vec = table.get(vk) if vk else None
            matches.append({"text": t, "vec": vk, "result": MG.match_text(c, t, vec)})
    OUT.mkdir(parents=True, exist_ok=True)
    doc = {"graph": graph, "decisions": decisions, "vectors": table, "memo_files": fs,
           "index": {"jokes": ix.jokes, "units": ix.units, "vec_units": ix.vec_units, "p99": [float(x) for x in ix.p99]},
           "candidates": [list(p) for p in cands], "features": feats,
           "pair_score": {k: MG.pair_score(f) for k, f in feats.items()},
           "judge_keys": {k: MG.judge_key(c, ix, f) for k, f in feats.items()},
           "judge_candidates": jc, "judge_asked": asked, "second_todo": second_todo, "second_asked": second_asked,
           "cache": cache, "items": items, "groups": groups, "matches": matches,
           "memos": ms, "memo_jokes": mj, "triage": tri, "triage_asked": triage_asked, "triage_doc": triage_doc,
           "suggestions": [g["id"] for g in sugg], "clear": clear,
           "events": [[aid, {**d, "date": "<today>"}] for aid, d in events],
           "full": {j: MG._full(c, ix, j) for j in ix.jokes},
           "verdicts": {k: MG.verdict(v) for k, v in cache.items()}}
    (OUT / "merge.json").write_text(json.dumps(doc, indent=1, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n")
    print(f"{len(cands)} candidates, {len(jc)} judged, {len(groups)} groups, {len(clear)} clear attaches, "
          f"{len(matches)} matches -> {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    os.environ.pop("PK_OWNER_ID", None)
    main()
