"""The Google Docs side of a merge, against the fake Drive only: one consolidated doc, the old docs moved (never edited,
never trashed), legal docs never touched, shared docs left in place, undo moves everything back, no loop."""
from __future__ import annotations

import pytest

from gsync.config import FOLDER_ID
from gsync.drive import WriteBlocked
from gsync.merge import ARCHIVE_NAME, merge_text
from gsync.sync import Sync

from .fakes import OLD, ROOT, FakeDrive, FakePunchable, clock, make_cfg, read

from .conftest import LEGAL_FOLDER


def seed(cfg, fid, title, body):
    (cfg.corpus_gdocs / f"x__{fid[:8]}.md").write_text(
        f"---\nsource: gdoc\ndoc_id: {fid}\ntitle: \"{title}\"\ncreated: {OLD}\nmodified: {OLD}\nurl: u\n---\n{body}",
        encoding="utf-8")


@pytest.fixture
def env(tmp_path):
    cfg = make_cfg(tmp_path)
    d, pk = FakeDrive(), FakePunchable(cfg)
    d.folder(FOLDER_ID, name="Punchable")
    d.folder("jokes2016", name="Jokes 2016")
    a = d.user_create("Marty delivery", "My cousin marty sang the wrong song.\nHe was wearing brown.", parents=("jokes2016",))
    b = d.user_create("Karaoke king marty", "Always picks the wrong song", parents=(ROOT,))
    shared = d.user_create("Ideas dump", "Marty tag\nand some other joke")
    for fid, t in ((a, "Marty delivery"), (b, "Karaoke king marty"), (shared, "Ideas dump")):
        seed(cfg, fid, t, d.text(fid))
    pk.graph_docs = {a: ["joke:marty-delivery-driver"], b: ["joke:karaoke-king-marty"],
                     shared: ["joke:karaoke-king-marty", "joke:some-other-joke"]}

    def run(dry=False):
        return Sync(cfg, d, pk, dry_run=dry, clock=clock).run()
    run()  # bootstrap
    return cfg, d, pk, run, {"a": a, "b": b, "shared": shared}


def merge_rec(docs, status="merged", extra_units=()):
    return {"id": "mg:test1", "kind": "merge", "status": status, "main": "joke:marty-delivery-driver",
            "others": ["joke:karaoke-king-marty"], "created_at": "2026-10-26T12:00:00+00:00",
            "snapshot": {
                "joke:marty-delivery-driver": {"title": "Marty delivery driver", "summary": "AI CATALOG SUMMARY", "summary_is_his": False,
                                          "units": [{"id": "unit:a-00", "date": "2017-03-02", "doc_id": docs["a"],
                                                     "doc_title": "Marty delivery",
                                                     "text": "My cousin marty sang the wrong song.\nHe was wearing brown."}]},
                "joke:karaoke-king-marty": {"title": "Karaoke king marty", "summary": None, "summary_is_his": False,
                                             "units": [{"id": "unit:b-00", "date": "2016-05-01", "doc_id": docs["b"],
                                                        "doc_title": "Karaoke king marty",
                                                        "text": "Always picks the wrong song"},
                                                       {"id": "unit:s-00", "date": "2016-06-01", "doc_id": docs["shared"],
                                                        "doc_title": "Ideas dump", "text": "Marty tag"},
                                                       *extra_units]}}}


def state(cfg):
    return read(cfg.state / "map.json")


def his_docs_untouched(d, docs, before):
    for fid in docs.values():
        f = d.files[fid]
        assert f["_text"] == before[fid]["_text"] and f["name"] == before[fid]["name"], "his doc was edited"
        assert not f.get("trashed"), "his doc was trashed"
    assert not [w for w in d.written if w[0] in ("update", "trash") and w[1] in docs.values()]
    assert not [w for w in d.written if w[0] == "trash"], "a merge never trashes anything"


def test_merge_writes_one_doc_and_moves_old_docs_to_archive(env):
    cfg, d, pk, run, docs = env
    before = {k: dict(v) for k, v in d.files.items()}
    pk.merge_items = {"mg:test1": merge_rec(docs)}
    rec = run()
    assert not rec["errors"], rec["errors"]
    st = state(cfg)["merges"]["mg:test1"]
    assert st["state"] == "applied"
    arch = st["archive_folder"]
    assert d.files[arch]["name"] == ARCHIVE_NAME and d.files[arch]["parents"] == [FOLDER_ID]
    # one consolidated doc, in the Punchable folder, every original line verbatim with its date; no AI summary
    made = [w[1] for w in d.written if w[0] == "create"]
    assert made == [st["doc_id"]]
    t = d.text(st["doc_id"])
    assert d.files[st["doc_id"]]["parents"] == [FOLDER_ID]
    for line in ("My cousin marty sang the wrong song.\nHe was wearing brown.", "Always picks the wrong song",
                 "[2017-03-02 · from \"Marty delivery\"]", "[2016-05-01 · from \"Karaoke king marty\"]", "Marty tag"):
        assert line in t, line
    assert "AI CATALOG SUMMARY" not in t
    assert d.files[st["doc_id"]]["appProperties"]["punchable_merge"] == "mg:test1"
    # the old docs: moved (parents only), never edited or trashed; the shared dump holds another joke: left alone
    assert d.files[docs["a"]]["parents"] == [arch] and d.files[docs["b"]]["parents"] == [arch]
    assert d.files[docs["shared"]]["parents"] == [ROOT] and docs["shared"] in st["left"]
    assert st["moved"][docs["a"]]["from"] == ["jokes2016"] and st["moved"][docs["b"]]["from"] == [ROOT]
    his_docs_untouched(d, docs, before)


def test_merge_is_idempotent_and_does_not_loop(env):
    cfg, d, pk, run, docs = env
    pk.merge_items = {"mg:test1": merge_rec(docs)}
    run()
    n_w, n_e, n_lines = len(d.written), len(d.exported), len(pk.lines)
    run()   # the Changes feed now holds our moves and our create
    run()
    assert len(d.written) == n_w, "a second pass wrote again"
    assert len(d.exported) == n_e, "our own moves came back as edits (exported)"
    assert len(pk.lines) == n_lines, "our own merged doc came back as an idea"


def test_legal_doc_in_a_merge_is_never_moved_or_read(env):
    cfg, d, pk, run, docs = env
    d.folder(LEGAL_FOLDER)
    legal = d.user_create("notes", "COURT TEXT", parents=(LEGAL_FOLDER,))
    lawsuit = d.user_create("Lawsuit timeline", "COURT TEXT 2")
    extra = [{"id": "unit:l-00", "date": "2020-01-01", "doc_id": legal, "doc_title": "notes", "text": "x"},
             {"id": "unit:c-00", "date": "2020-01-02", "doc_id": lawsuit, "doc_title": "Lawsuit timeline", "text": "y"}]
    pk.merge_items = {"mg:test1": merge_rec(docs, extra_units=extra)}
    rec = run()
    st = state(cfg)["merges"]["mg:test1"]
    for fid, parents in ((legal, [LEGAL_FOLDER]), (lawsuit, [ROOT])):
        assert d.files[fid]["parents"] == parents, "a legal doc was moved"
        assert fid not in d.exported
        assert fid in st["skipped_legal"]
        assert not [w for w in d.written if w[1] == fid]
    assert st["state"] == "applied" and not rec["errors"]


def test_move_gate_refuses_legal_docs_directly(env):
    cfg, d, pk, run, docs = env
    d.folder(LEGAL_FOLDER)
    legal = d.user_create("notes", "COURT TEXT", parents=(LEGAL_FOLDER,))
    with pytest.raises(WriteBlocked):
        d.move(legal, FOLDER_ID, [LEGAL_FOLDER], [])
    kw = d.user_create("Hearing notes for Tuesday", "COURT TEXT")
    with pytest.raises(WriteBlocked):
        d.move(kw, FOLDER_ID, [ROOT], [])
    foreign = d.user_create("shared", "x", owner="someone@else.com")
    with pytest.raises(WriteBlocked):
        d.move(foreign, FOLDER_ID, [ROOT], [])
    assert d.files[legal]["parents"] == [LEGAL_FOLDER] and d.files[kw]["parents"] == [ROOT]


def test_undo_moves_docs_back_and_releases_the_merged_doc(env):
    cfg, d, pk, run, docs = env
    before = {k: dict(v) for k, v in d.files.items()}
    pk.merge_items = {"mg:test1": merge_rec(docs)}
    run()
    mdoc = state(cfg)["merges"]["mg:test1"]["doc_id"]
    pk.merge_items = {"mg:test1": merge_rec(docs, status="undone")}
    rec = run()
    assert not rec["errors"], rec["errors"]
    st = state(cfg)["merges"]["mg:test1"]
    assert st["state"] == "undone"
    assert d.files[docs["a"]]["parents"] == ["jokes2016"] and d.files[docs["b"]]["parents"] == [ROOT]
    f = d.files[mdoc]
    assert not f.get("trashed"), "the merged doc is never trashed"
    assert "punchable_merge" not in f["appProperties"] and "punchable_joke" not in f["appProperties"]
    assert f["name"].startswith("(merge undone)") and f["parents"] == [st["archive_folder"]]
    his_docs_untouched(d, docs, before)
    n = len(d.written)
    run()
    assert len(d.written) == n


def test_undo_leaves_a_doc_he_moved_himself(env):
    cfg, d, pk, run, docs = env
    pk.merge_items = {"mg:test1": merge_rec(docs)}
    run()
    d.folder("elsewhere")
    d.files[docs["a"]]["parents"] = ["elsewhere"]
    pk.merge_items = {"mg:test1": merge_rec(docs, status="undone")}
    run()
    st = state(cfg)["merges"]["mg:test1"]
    assert d.files[docs["a"]]["parents"] == ["elsewhere"]
    assert "you moved it" in st["moved_back"][docs["a"]]


def test_dry_run_moves_nothing(env):
    cfg, d, pk, run, docs = env
    pk.merge_items = {"mg:test1": merge_rec(docs)}
    n = len(d.written)
    rec = run(dry=True)
    assert len(d.written) == n and rec["counts"].get("merge_apply") == 1
    assert "mg:test1" not in state(cfg).get("merges", {})


def test_his_edit_to_the_merged_doc_is_kept_never_an_idea(env):
    cfg, d, pk, run, docs = env
    pk.merge_items = {"mg:test1": merge_rec(docs)}
    run()
    mdoc = state(cfg)["merges"]["mg:test1"]["doc_id"]
    n_lines = len(pk.lines)
    d.user_edit(mdoc, d.text(mdoc) + "\nNEW TAG: brown is the new black", modified="2026-10-26T09:00:00.000Z")
    run()
    edits = [x for x in (cfg.state / "edits.jsonl").read_text(encoding="utf-8").splitlines() if "merged_doc_edit" in x]
    assert edits and "NEW TAG: brown is the new black" in edits[-1]
    assert len(pk.lines) == n_lines


def test_merge_text_keeps_his_typed_idea_and_skips_ai_summary():
    rec = merge_rec({"a": "A", "b": "B", "shared": "S"})
    rec["others"].append("joke:typed")
    rec["snapshot"]["joke:typed"] = {"title": "typed idea", "summary": "his own typed words", "summary_is_his": True,
                                     "added_at": "2026-10-20", "units": []}
    t = merge_text(rec)
    assert "his own typed words" in t and "[2026-10-20 · typed in Punchable]" in t
    assert "AI CATALOG SUMMARY" not in t
    assert t.index("MAIN: Marty delivery driver") < t.index("Karaoke king marty")
