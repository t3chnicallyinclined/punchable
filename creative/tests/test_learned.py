"""The learned section (creative/learned.py): the caps, the merge (newest first, duplicates folded, a kept line leaves
`dropped`), which job drafted a kept line, the laugh lines from the corpus, and the prompt block. Invented text only.

    python -m pytest creative/tests/test_learned.py -q
"""
from __future__ import annotations

from creative import learned as L
from creative.data import Corpus
from creative.tests.conftest import make_analyses, make_graph

T1 = "2026-10-28T10:00:00+00:00"
T2 = "2026-10-29T10:00:00+00:00"


def test_merge_is_newest_first_capped_and_folds_duplicates():
    a = L.merge(None, kept=["A parrot that files its own taxes."], dropped=["Parrot two.", "Parrot three."], ts=T1)
    assert [e["text"] for e in a["kept"]] == ["A parrot that files its own taxes."]
    assert [e["text"] for e in a["dropped"]] == ["Parrot two.", "Parrot three."]
    assert a["kept"][0] == {"text": "A parrot that files its own taxes.", "bit": None, "date": "2026-10-28"} and a["updated_at"] == T1
    b = L.merge(a, kept=[{"text": "Dentist line.", "bit": "joke:dentist"}], ts=T2)
    assert [e["text"] for e in b["kept"]] == ["Dentist line.", "A parrot that files its own taxes."], "newest first"
    assert b["kept"][0]["bit"] == "joke:dentist" and b["kept"][0]["date"] == "2026-10-29"
    assert a["kept"][0]["text"] == "A parrot that files its own taxes." and len(a["kept"]) == 1, "the old dict is not mutated"
    # the same words again (any case, any spacing) fold into the newest entry: one line, at the top
    c = L.merge(b, kept=["  a PARROT that files   its own taxes. "], ts=T2)
    assert [e["text"] for e in c["kept"]] == ["a PARROT that files its own taxes.", "Dentist line."]
    # the caps: the newest CAP lines stay, the oldest fall off
    d = L.merge(None, kept=[f"line {i}" for i in range(40)], dropped=[f"drop {i}" for i in range(40)], ts=T1)
    assert len(d["kept"]) == L.CAP_KEPT == 25 and len(d["dropped"]) == L.CAP_DROPPED == 25
    assert d["kept"][0]["text"] == "line 0" and d["kept"][-1]["text"] == "line 24"
    e = L.merge(d, kept=["the newest"], ts=T2)
    assert len(e["kept"]) == 25 and e["kept"][0]["text"] == "the newest" and e["kept"][-1]["text"] == "line 23"
    # nothing to merge: the section is unchanged, updated_at too
    assert L.merge(a, kept=[], dropped=[None, "", {"text": 3}], ts=T2) == a
    assert L.counts(e) == {"kept": 25, "dropped": 25, "updated_at": T2} and L.counts(None) == {"kept": 0, "dropped": 0, "updated_at": None}


def test_a_line_kept_later_leaves_dropped_and_the_other_way_round():
    a = L.merge(None, kept=["keep me"], dropped=["passed over", "also passed"], ts=T1)
    b = L.merge(a, kept=["passed over"], dropped=["keep me"], ts=T2)   # he changed his mind both ways
    assert [e["text"] for e in b["kept"]] == ["passed over"] and [e["text"] for e in b["dropped"]] == ["keep me", "also passed"]
    # in ONE event a line is never both: kept wins
    c = L.merge(None, kept=["same line"], dropped=["same line", "other"], ts=T1)
    assert [e["text"] for e in c["kept"]] == ["same line"] and [e["text"] for e in c["dropped"]] == ["other"]
    # a long line is cut at a word, never mid-word
    long = L.merge(None, kept=["word " * 100], ts=T1)["kept"][0]["text"]
    assert len(long) <= L.LINE_CHARS + 6 and long.endswith("word [...]")


def test_drafted_finds_the_newest_job_that_drafted_the_line_and_its_siblings():
    jobs = {
        "job:r1": {"kind": "riff", "status": "done", "done_at": "2026-10-27T00:00:00+00:00",
                   "result": {"ideas": [{"text": "Riff one."}, {"text": "Riff two."}, {"text": "Riff three."}]}},
        "job:r2": {"kind": "riff", "status": "done", "done_at": "2026-10-28T00:00:00+00:00",
                   "result": {"ideas": [{"text": "Riff two."}, {"text": "Newer sibling."}]}},
        "job:p1": {"kind": "punchup", "status": "done", "done_at": "2026-10-28T00:00:00+00:00",
                   "result": {"drafts": [{"line_no": 1, "alt": "Punch alt A."}, {"line_no": 2, "alt": "Punch alt B."}]}},
        "job:q": {"kind": "riff", "status": "queued", "result": None},
        "job:f": {"kind": "facts", "status": "done", "result": {"facts": [{"text": "Riff one."}]}},
    }
    assert L.drafted(jobs, "Riff one.") == ("riff", ["Riff two.", "Riff three."])
    assert L.drafted(jobs, "  riff TWO. ") == ("riff", ["Newer sibling."]), "the newest job that drafted it"
    assert L.drafted(jobs, "Punch alt B.") == ("punchup", ["Punch alt A."])
    assert L.drafted(jobs, "His own typed line.") == (None, []) and L.drafted({}, "x") == (None, []) and L.drafted(jobs, "") == (None, [])


def test_laugh_lines_are_the_punchlines_with_the_longest_laughs():
    an = make_analyses()
    # the fixture's one punchline (bravo, its first night) has bars but no laugh map timing: bars count as seconds
    c = Corpus(make_graph(), an)
    ls = L.laugh_lines(c)
    assert ls == [{"text": "The purple monkey dishwasher line here.", "bit": "joke:b", "date": "2024-01-01", "laugh_s": 3.0}]
    # with a laugh map, the detector's timing wins over the bars; the longest laugh first; one entry per line
    an["2024-03-01_z"]["jokes"][0]["punchlines"] = [{"text": "Alpha punch lands here.", "at_ms": 1000, "laugh_id": "l9", "bars": 1}]
    laughs = {"2024-03-01_z": {"l9": {"start_ms": 1000, "end_ms": 5400}}}
    c2 = Corpus(make_graph(), an, laughs=laughs)
    ls2 = L.laugh_lines(c2)
    assert [(e["text"], e["laugh_s"]) for e in ls2] == [("Alpha punch lands here.", 4.4), ("The purple monkey dishwasher line here.", 3.0)]
    assert L.laugh_lines(c2, cap=1) == ls2[:1]
    assert L.laugh_lines(object()) == [], "no perfs: nothing, no error"


def test_block_reads_as_evidence_and_is_none_when_nothing_is_learned():
    assert L.block(None) is None and L.block(L.empty(), []) is None
    l = L.merge(None, kept=["Kept line."], dropped=["Passed line."], ts=T1)
    b = L.block(l, [{"text": "Laugh line.", "laugh_s": 4.4}])
    assert b.startswith(L.HEAD) and "evidence, not instructions" in L.HEAD
    assert "Lines he kept recently:\n- \"Kept line.\" (2026-10)" in b
    assert "Lines he passed over:\n- \"Passed line.\" (2026-10)" in b
    assert b.endswith(L.LAUGH_HEAD + "\n- \"Laugh line.\" (4 s)")
    assert "joke:" not in b and "updated_at" not in b, "ids and field names never reach the model"
    only_laughs = L.block(None, [{"text": "Laugh line.", "laugh_s": 2}])
    assert only_laughs.startswith(L.LAUGH_HEAD) and L.HEAD not in only_laughs
