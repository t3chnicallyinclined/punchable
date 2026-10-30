"""creative/merge.py on a synthetic corpus (no private material): pair features, the judge cache, groups (no
chaining, his dismissals, the main pick), auto-attach (attach / ambiguous / new) and the merge store."""
from __future__ import annotations

import numpy as np
import pytest

from creative import merge as M
from creative.data import Corpus, Vectors
from creative.store import Store

from .conftest import FakeLLM, HAS_REAL

FILLER = ["toaster", "parade", "glacier", "violin", "cactus", "harbor", "lantern", "meadow", "pretzel", "quartz",
          "saddle", "tunnel", "walrus", "yogurt", "zeppelin", "anchor", "biscuit", "canyon", "dolphin", "ember"]


def _vec(rng, base=None, noise=0.0):
    v = rng.normal(size=384) if base is None else base + rng.normal(size=384) * noise
    return (v / np.linalg.norm(v)).tolist()


def make(planted=True):
    """60 unrelated one-unit jokes plus planted ones: V1/V2 the same Marty bit; C1~C2~C3 a chain (C1-C3 unrelated
    in words); T1/T2 two jots equally close to a jot (for the ambiguity rule)."""
    rng = np.random.default_rng(7)
    nodes, edges, table = [], [], {}

    def joke(k, title, text, stage="idea", date="2020-01-01", vec=None):
        nodes.append({"id": f"joke:{k}", "type": "joke", "title": title, "stage": stage, "summary": "AI summary"})
        nodes.append({"id": f"unit:{k}", "type": "unit", "kind": "note", "text": text, "date": date,
                      "doc_id": f"doc-{k}", "doc_title": title})
        edges.append({"id": f"e:p{k}", "src": f"unit:{k}", "dst": f"joke:{k}", "kind": "part_of",
                      "status": "inferred", "confidence": 0.9})
        table[f"unit:{k}"] = vec if vec is not None else _vec(rng)

    for i in range(60):
        w = [FILLER[(i + n) % len(FILLER)] for n in range(4)]
        joke(f"x{i}", f"Filler {i} {w[0]}", f"Random thought number {i} about the {w[0]} and the {w[1]} near "
                                             f"a {w[2]} with {w[3]} {i}")
    if planted:
        vin = rng.normal(size=384)
        joke("v1", "Marty delivery driver", "My cousin marty sang the wrong song, he was wearing brown, "
                                       "the UPS guy got knocked out cold", stage="testing", date="2017-03-02",
             vec=_vec(rng, vin, 0.15))
        joke("v2", "Karaoke king marty", "Marty always knocks out the wrong guy, he was wearing brown again",
             date="2016-05-01", vec=_vec(rng, vin, 0.15))
        ch = rng.normal(size=384)
        joke("c1", "Gym mirror selfies", "Guys taking gym mirror selfies between every single rep", vec=_vec(rng, ch, 0.2))
        joke("c2", "Gym selfies and protein", "gym mirror selfies then a protein shake selfie, protein powder everywhere",
             vec=_vec(rng, ch, 0.2))
        joke("c3", "Protein shake addicts", "protein powder addicts measuring scoops like a chemist", vec=_vec(rng, ch, 0.2))
        tw = rng.normal(size=384)
        joke("t1", "Dentist small talk", "dentist asks questions with both hands in my mouth, small talk champion",
             vec=_vec(rng, tw, 0.05))
        joke("t2", "Dentist hands in mouth", "the dentist with both hands in my mouth asking about my weekend",
             vec=_vec(rng, tw, 0.05))
        table["_vin"], table["_tw"] = _vec(rng, vin, 0.15), _vec(rng, tw, 0.05)
    c = Corpus({"nodes": nodes, "edges": edges}, {}, vectors=Vectors(table=table))
    return c, table


@pytest.fixture
def syn():
    return make()


def judge_payload(same: dict):
    """A fake judge: `same` maps frozenset({title a, title b}) -> (same, confidence)."""
    def answer(messages):
        import re
        out = []
        for pid, ta, tb in re.findall(r"PAIR (p\d+)\nA \(([^)]*)\):.*?\nB \(([^)]*)\):", messages[1]["content"], re.S):
            s, conf = same.get(frozenset((ta, tb)), (False, 0.9))
            out.append({"id": pid, "same": s, "confidence": conf, "why": f"{ta} vs {tb}"})
        return {"pairs": out}
    return answer


def test_planted_pair_scores_and_unrelated_does_not(syn):
    c, _ = syn
    ix = M.index(c)
    f = ix.features("joke:v1", "joke:v2")
    assert "was wearing brown" in f["phrases"] and "marty" in f["title"]
    assert M.pair_score(f) >= M.JUDGE_FLOOR
    g = ix.features("joke:x1", "joke:x30")
    assert M.pair_score(g) < M.JUDGE_FLOOR
    cands = {tuple(sorted((x["a"], x["b"]))) for x in M.judge_candidates(c)}
    assert ("joke:v1", "joke:v2") in cands


def test_groups_need_a_confident_judge(syn):
    c, _ = syn
    fs = M.judge_candidates(c)
    for conf, expect in ((0.95, 1), (0.6, 0)):
        llm = FakeLLM(judge_payload({frozenset(("Marty delivery driver", "Karaoke king marty")): (True, conf)}))
        cache = M.judge(c, fs, {}, llm)
        gs = M.build_groups(c, cache)
        assert len(gs) == expect, (conf, gs)
    assert llm.calls and all(t == "merge_judge" for t, _ in llm.calls)
    # no judge at all (or a provider error): nothing is grouped
    assert M.build_groups(c, {}) == []

    def boom(*a):
        raise RuntimeError("provider down")
    assert M.judge(c, fs, {}, boom) == {}


def test_main_is_the_proven_version_and_reasons_are_plain(syn):
    c, _ = syn
    llm = FakeLLM(judge_payload({frozenset(("Marty delivery driver", "Karaoke king marty")): (True, 0.95)}))
    g = M.build_groups(c, M.judge(c, M.judge_candidates(c), {}, llm))[0]
    assert g["main"] == "joke:v1"          # stage testing beats idea
    assert "furthest along (testing)" in g["main_why"]
    assert [m["joke_id"] for m in g["members"]] == ["joke:v1", "joke:v2"]
    assert g["members"][1]["first_date"] == "2016-05-01" and g["members"][1]["docs"][0]["doc_id"] == "doc-v2"
    assert any("was wearing brown" in r for r in g["reasons"])
    out = M.find_groups(c, M.judge(c, M.judge_candidates(c), {}, llm))
    assert out["counts"]["jokes_in_groups"] == 2 and out["counts"]["jokes_after_merge"] == out["counts"]["jokes"] - 1


def test_no_chaining(syn):
    c, _ = syn
    same = {frozenset(("Gym mirror selfies", "Gym selfies and protein")): (True, 0.9),
            frozenset(("Gym selfies and protein", "Protein shake addicts")): (True, 0.9),
            frozenset(("Gym mirror selfies", "Protein shake addicts")): (False, 0.9)}
    cache = M.judge(c, M.judge_candidates(c), {}, FakeLLM(judge_payload(same)))
    gs = M.build_groups(c, cache)
    for g in gs:
        ids = {m["joke_id"] for m in g["members"]}
        assert not {"joke:c1", "joke:c3"} <= ids, "c1 and c3 were chained together through c2"
        for m in ids - {g["main"]}:
            assert any({p["a"], p["b"]} == {g["main"], m} for p in g["pairs"]), "a member not judged against the main"


def test_dismissed_and_attached_never_grouped(syn):
    c, _ = syn
    llm = FakeLLM(judge_payload({frozenset(("Marty delivery driver", "Karaoke king marty")): (True, 0.95)}))
    cache = M.judge(c, M.judge_candidates(c), {}, llm)
    assert M.build_groups(c, cache, frozenset({frozenset(("joke:v1", "joke:v2"))})) == []
    assert M.build_groups(c, cache, exclude={"joke:v2"}) == []
    r = M.candidates_for(c, "joke:v1", cache)
    assert r["group"]["main"] == "joke:v1"


VIN_GROUP = {"joke:v1": "joke:v1", "joke:v2": "joke:v1"}


def test_attach_clear_match(syn):
    c, t = syn
    jot = "Marty knocked out the wrong guy again, this one was wearing brown too"
    # v1 and v2 are two close jokes: until they are known to be one bit, that is a tie and the jot stays new
    assert M.match_text(c, jot, t["_vin"])["decision"] == "ambiguous"
    r = M.match_text(c, jot, t["_vin"], group_of=VIN_GROUP)
    assert r["decision"] == "attach" and r["joke_id"] == "joke:v1", r     # the group's main
    assert r["reasons"]


def test_attach_ambiguous_stays_new(syn):
    c, t = syn
    r = M.match_text(c, "the dentist with both hands in my mouth doing small talk", t["_tw"])
    assert r["decision"] == "ambiguous", r     # t1 and t2 are equally close: never guess


def test_attach_short_or_unrelated_stays_new(syn):
    c, t = syn
    assert M.match_text(c, "Marty brown", t["_vin"])["decision"] == "new"            # too short to place
    rng = np.random.default_rng(99)
    r = M.match_text(c, "completely different thought about airline peanuts and tray tables", _vec(rng))
    assert r["decision"] == "new", r
    # same words but far in meaning: words alone never attach when a vector says otherwise
    r = M.match_text(c, "Marty knocked out the wrong guy, he was wearing brown", _vec(rng))
    assert r["decision"] != "attach", r


def test_attach_without_vector_needs_unmistakable_words(syn):
    c, _ = syn
    assert M.match_text(c, "Marty knocked out the wrong guy, he was wearing brown, the UPS guy got knocked out "
                           "cold", group_of=VIN_GROUP)["decision"] == "attach"
    assert M.match_text(c, "a guy wearing brown at the gym")["decision"] != "attach"


def test_store_ops_and_replay(tmp_path):
    st = Store(tmp_path, "merges", M.store_apply)
    st.append("merge", "mg:1", {"main": "joke:a", "others": ["joke:b"], "indexes": [5, 6], "snapshot": {"joke:b": {}}})
    with pytest.raises(ValueError):
        st.append("merge", "mg:2", {"main": "joke:a", "others": ["joke:a"]})
    with pytest.raises(ValueError):
        st.append("merge", "mg:3", {"main": "joke:a", "others": []})
    st.append("undo", "mg:1", {"indexes": [8, 7]})
    with pytest.raises(ValueError):
        st.append("undo", "mg:1", {})
    st.append("attach", "at:1", {"joke": "joke:n", "into": "joke:a", "text": "jot"}, by="ai:attach")
    assert M.attached(st.items) == {"joke:n": st.items["at:1"]}
    st.append("detach", "at:1", {})
    assert M.attached(st.items) == {}
    st.append("dismiss", "dm:1", {"jokes": ["joke:x", "joke:y", "joke:z"]})
    assert frozenset(("joke:x", "joke:z")) in M.dismissed_pairs(st.items)
    with pytest.raises(ValueError):
        st.append("explode", "mg:1", {})
    again = Store(tmp_path, "merges", M.store_apply)
    assert again.items == st.items and again.items["mg:1"]["status"] == "undone"
    assert again.items["at:1"]["by"] == "ai:attach" and again.items["mg:1"]["by"] == "owner"
    h = M.history(again.items, "joke:a")
    assert [m["id"] for m in h["merges"]] == ["mg:1"] and h["notes"] == []


@pytest.mark.skipif(not HAS_REAL, reason="real KB not on this machine")
def test_real_groups_are_consistent(real):
    from creative.data import creative_dir
    j = M.load_judgements(creative_dir())
    out = M.find_groups(real, j)
    n = out["counts"]
    assert n["jokes_after_merge"] == n["jokes"] - n["jokes_in_groups"] + n["groups"]
    seen = set()
    for g in out["groups"]:
        ids = [m["joke_id"] for m in g["members"]]
        assert g["main"] == ids[0] and len(ids) >= 2 and not (set(ids) & seen)
        seen |= set(ids)
        for m in ids[1:]:
            assert any({p["a"], p["b"]} == {g["main"], m} for p in g["pairs"])
    assert M.find_groups(real, j)["groups"] == out["groups"]   # deterministic
