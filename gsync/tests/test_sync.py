"""gsync against fakes: new / edit / delete / legal / no loop / conflict / idempotent / gate / boards / ai_draft."""
from __future__ import annotations

import json

import pytest

from gsync.config import ADOPTED_JOKE_DOCS, AI_DRAFT_DOCS, BOARD_DOCS, FOLDER_ID, MIXED_DOCS
from gsync.drive import WriteBlocked
from gsync.store import read_jsonl
from gsync.sync import Sync

from .fakes import ME, OLD, FakeDrive, FakePunchable, clock, make_cfg, read

BOARD_DOC = BOARD_DOCS["sb:robot-vacuum-104b6cc0"]
AI_DOC = next(iter(AI_DRAFT_DOCS))
from .conftest import DENYLISTED, LEGAL_FOLDER


@pytest.fixture
def env(tmp_path):
    cfg = make_cfg(tmp_path)
    d, pk = FakeDrive(), FakePunchable(cfg)
    d.folder(FOLDER_ID, name="Punchable")
    # made by the Claude connector: no appProperties, so only adoption by ID makes them known
    d.user_create("Robot vacuum (board)", "hand-made export", fid=BOARD_DOC, parents=(FOLDER_ID,))
    d.user_create("AI explained step by step (DRAFT walk-through for the AI chunk)", "YOURS: line\nDRAFT: ai line",
                  fid=AI_DOC, parents=(FOLDER_ID,))

    def run(dry=False):
        return Sync(cfg, d, pk, dry_run=dry, clock=clock).run()
    return cfg, d, pk, run


def state(cfg):
    return read(cfg.state / "map.json")


def test_new_doc_becomes_idea(env):
    cfg, d, pk, run = env
    run()  # bootstrap
    fid = d.user_create("Parking app", "I want a parking app\n\nand park badly")
    rec = run()
    assert rec["counts"].get("new") == 1, rec
    jid = state(cfg)["docs"][fid]["joke_id"]
    assert pk.jokes[jid]["summary"] == "I want a parking app\n\nand park badly"  # verbatim, BOM and CRLF gone
    assert pk.jokes[jid]["stage"] == "idea"
    assert pk.jokes[jid]["from"]["gdoc_id"] == fid
    line = pk.lines[-1]
    assert line["by"] == "owner" and line["reason"].startswith("[gdocs]")
    units = read(cfg.state / "units.json")
    u = [x for x in units if x["doc_id"] == fid]
    assert u and u[0]["locator"] == {"para_start": 0, "para_end": 1, "para_rule": "non-blank lines of fileContent, 0-based, inclusive"}
    assert u[0]["text"] == "I want a parking app\nand park badly"


def test_edit_updates_summary_with_undo(env):
    cfg, d, pk, run = env
    run()
    fid = d.user_create("Dating app joke", "v1 text")
    run()
    jid = state(cfg)["docs"][fid]["joke_id"]
    d.user_edit(fid, "v2 text, punched up")
    rec = run()
    assert rec["counts"].get("edit") == 1
    assert pk.jokes[jid]["summary"] == "v2 text, punched up"
    act = [a for a in rec["actions"] if a["op"] == "edit"][0]
    assert act["indexes"], "edits must be undoable"
    runs = read_jsonl(cfg.state / "runs.jsonl")
    assert runs[-1]["run_id"] == rec["run_id"]
    pk.undo(act["indexes"])
    assert pk.jokes[jid]["summary"] == "v1 text"


def test_trashed_doc_is_hidden_never_deleted(env):
    cfg, d, pk, run = env
    run()
    fid = d.user_create("Bad idea", "meh")
    run()
    jid = state(cfg)["docs"][fid]["joke_id"]
    d.user_trash(fid)
    rec = run()
    e = state(cfg)["docs"][fid]
    assert e["hidden"] is True and rec["counts"].get("gone") == 1
    assert pk.jokes[jid]["stage"] == "retired"
    assert not [x for x in pk.lines if x["action"] == "delete"]


def assert_legal_never_ingested(cfg, d, pk, run):
    """The legal test. Kept as a helper so test_rules can prove it fails on a broken rule."""
    run()
    sub = d.folder("subfolder-of-legal", parents=(LEGAL_FOLDER,))
    d.folder(LEGAL_FOLDER)
    legal = {
        "kw": d.user_create("Lawsuit timeline notes", "COURT TEXT A"),
        "kw2": d.user_create("Hearing notes for Tuesday", "COURT TEXT B"),
        "folder": d.user_create("notes", "COURT TEXT C", parents=(LEGAL_FOLDER,)),
        "subfolder": d.user_create("draft", "COURT TEXT D", parents=(sub,)),
        "owner": d.user_create("shared thing", "COURT TEXT E", owner="someone@else.com"),
        "denylist": d.user_create("Untitled document", "COURT TEXT F", fid=DENYLISTED),
        "filing": d.user_create("Order to show cause", "COURT TEXT G"),
        "localkw": d.user_create("Notes about Sample Person", "COURT TEXT H"),
    }
    caption = d.user_create("Funny stuff", "SUPERIOR COURT OF THE STATE OF EXAMPLE\nIndex No. 123\nPetitioner")
    joke = d.user_create("Judge tag", "the judge looked at me like an exhibit")  # a joke about a courtroom: kept
    run()
    m = state(cfg)
    for k, fid in legal.items():
        assert fid not in d.exported, f"{k}: a legal doc was exported"
        assert fid in m["excluded"], k
        assert fid not in m["docs"], k
    assert caption in m["excluded"] and m["excluded"][caption] == "content"
    blob = json.dumps(pk.lines) + (cfg.state / "map.json").read_text(encoding="utf-8")
    blob += (cfg.state / "units.json").read_text(encoding="utf-8") if (cfg.state / "units.json").exists() else ""
    blob += "".join(p.read_text(encoding="utf-8") for p in (cfg.state / "mirror").glob("*")) if (cfg.state / "mirror").exists() else ""
    blob += (cfg.state / "runs.jsonl").read_text(encoding="utf-8")
    for t in ["COURT TEXT", "Index No. 123", "Lawsuit timeline", "Order to show cause", "Sample Person"]:
        assert t not in blob, f"legal content or title leaked: {t}"
    assert m["docs"][joke]["joke_id"], "a joke about a courtroom is part of the act and must sync"


def test_legal_docs_skipped_and_never_exported(env):
    assert_legal_never_ingested(*env)


def test_push_then_no_loop_and_idempotent(env):
    cfg, d, pk, run = env
    jid = pk.app_idea("Dice bit", "DICE CLAY THE COMEDIAN\n\nTAG: can the dice be fixed?")
    run()
    docs = [k for k, e in state(cfg)["docs"].items() if e.get("joke_id") == jid]
    assert len(docs) == 1
    did = docs[0]
    assert d.text(did) == "DICE CLAY THE COMEDIAN\n\nTAG: can the dice be fixed?"  # verbatim
    assert d.files[did]["parents"] == [FOLDER_ID] and d.files[did]["name"] == "Dice bit"
    assert d.files[did]["appProperties"]["punchable"] == "1"
    n_lines, n_written, n_exp = len(pk.lines), len(d.written), len(d.exported)
    rec = run()  # the Changes feed now holds our own create: it must not come back as an idea
    assert len(pk.lines) == n_lines, "push came back as a pull: loop"
    rec = run()
    assert len(d.written) == n_written and len(d.exported) == n_exp and len(pk.lines) == n_lines
    assert not rec["errors"]
    assert set(rec["counts"]) <= {"settling"}, rec["counts"]


def test_app_edit_updates_doc_when_doc_unchanged(env):
    cfg, d, pk, run = env
    jid = pk.app_idea("Closer", "v1")
    run()
    did = [k for k, e in state(cfg)["docs"].items() if e.get("joke_id") == jid][0]
    pk.app_edit(jid, "v1\n\nLINE: new line")
    rec = run()
    assert rec["counts"].get("push_edit") == 1
    assert d.text(did) == "v1\n\nLINE: new line"
    n = len(pk.lines)
    run()
    assert len(pk.lines) == n, "our own doc update came back as an edit"


def test_conflict_doc_wins_app_text_kept_doc_not_overwritten(env):
    cfg, d, pk, run = env
    jid = pk.app_idea("Room model", "v1")
    run()
    did = [k for k, e in state(cfg)["docs"].items() if e.get("joke_id") == jid][0]
    pk.app_edit(jid, "app version")
    d.user_edit(did, "doc version")
    rec = run()
    assert d.text(did) == "doc version", "the doc was overwritten"
    assert pk.jokes[jid]["summary"] == "doc version"
    c = read_jsonl(cfg.state / "conflicts.jsonl")
    assert c and c[-1]["kind"] == "both_edited" and c[-1]["app_text"] == "app version"
    assert rec["counts"].get("conflict") == 1
    assert ("update", did) not in d.written


def test_conflict_when_doc_changed_but_feed_lags(env):
    cfg, d, pk, run = env
    jid = pk.app_idea("Room model", "v1")
    run()
    run()  # the feed has delivered our own create
    did = [k for k, e in state(cfg)["docs"].items() if e.get("joke_id") == jid][0]
    pk.app_edit(jid, "app version")
    d.files[did]["_text"], d.files[did]["version"] = "doc version", "9"  # edited, not yet in the Changes feed
    rec = run()
    assert d.text(did) == "doc version"
    assert read_jsonl(cfg.state / "conflicts.jsonl")[-1]["kind"] == "doc_changed_since_sync"
    run()  # same state: no duplicate conflict note
    assert len(read_jsonl(cfg.state / "conflicts.jsonl")) == 1


def test_app_edit_to_idea_from_his_own_doc_never_writes_his_doc(env):
    cfg, d, pk, run = env
    run()
    fid = d.user_create("Old joke", "his words")
    run()
    jid = state(cfg)["docs"][fid]["joke_id"]
    pk.jokes[jid]["summary"] = "changed in app"
    run()
    assert d.text(fid) == "his words" and ("update", fid) not in d.written
    assert read_jsonl(cfg.state / "conflicts.jsonl")[-1]["kind"] == "own_doc_not_written"


def test_write_gate_blocks_his_docs(env):
    cfg, d, pk, run = env
    his = d.user_create("Airport bit", "his words")
    run()
    with pytest.raises(WriteBlocked):
        d.update_doc(his, "overwritten")
    with pytest.raises(WriteBlocked):
        d.update_doc(AI_DOC, "overwritten")
    with pytest.raises(WriteBlocked):
        d.trash(his)
    allowed = {k for k, e in state(cfg)["docs"].items() if e.get("origin") in ("app", "board")}
    assert {fid for _, fid in d.written} <= allowed


def test_ai_draft_doc_adopted_and_edits_are_his_not_ideas(env):
    cfg, d, pk, run = env
    run()
    e = state(cfg)["docs"][AI_DOC]
    assert e["origin"] == "ai_draft" and e["source"] == "ai_draft" and e["contains_drafts"] is True
    n = len(pk.lines)
    d.user_edit(AI_DOC, "YOURS: line\nDRAFT: ai line\nYOURS: his new line")
    run()
    assert len(pk.lines) == n, "the AI-draft doc must never become an idea"
    ed = read_jsonl(cfg.state / "edits.jsonl")
    hit = [x for x in ed if x["doc_id"] == AI_DOC]
    assert hit and hit[-1]["by"] == "owner" and hit[-1]["contains_drafts"] is True
    units = read(cfg.state / "units.json") if (cfg.state / "units.json").exists() else []
    assert not [u for u in units if u["doc_id"] == AI_DOC]
    assert AI_DOC not in {fid for _, fid in d.written}


def test_board_adopted_regenerated_verbatim_only(env):
    cfg, d, pk, run = env
    a = pk.app_idea("AI guesses what comes next", "mary had a little ...")
    pk.jokes["joke:phone-autocorrect"] = {"id": "joke:phone-autocorrect", "title": "Phone autocorrect",
                                                  "summary": "AI CATALOG SUMMARY", "stage": "idea",
                                                  "lead_text": "why are blue whales...", "hidden": False}
    pk.jokes["joke:no-unit"] = {"id": "joke:no-unit", "title": "No unit", "summary": "ANOTHER AI SUMMARY", "stage": "idea",
                                "lead_text": None, "hidden": False}
    pk.board_items = {
        "sb:robot-vacuum-104b6cc0": {"id": "sb:robot-vacuum-104b6cc0", "title": "Robot vacuum", "by": "owner",
                                        "source": "his", "joke_ids": ["joke:phone-autocorrect", a, "joke:no-unit"],
                                        "target_s": 420, "notes": "his note", "updated_at": "u1", "deleted": False},
        "sb:seed-x": {"id": "sb:seed-x", "title": "seed", "by": "ai:seed", "source": "inferred", "joke_ids": [a],
                      "notes": "AI note", "updated_at": "u1", "deleted": False}}
    run()
    t = d.text(BOARD_DOC)
    assert t.startswith("Robot vacuum (board)") and "GENERATED BY PUNCHABLE" in t
    assert "why are blue whales..." in t and "mary had a little ..." in t and "his note" in t
    assert "AI CATALOG SUMMARY" not in t and "ANOTHER AI SUMMARY" not in t
    assert not [f for f in d.files.values() if f.get("name") == "seed (board)"], "AI seed boards are not written"
    ed = [x for x in read_jsonl(cfg.state / "edits.jsonl") if x["doc_id"] == BOARD_DOC]
    assert ed and ed[0]["text"] == "hand-made export", "the adopted doc's prior text is kept before regenerating"
    w = len(d.written)
    run()
    assert len(d.written) == w, "unchanged board rewritten"
    pk.board_items["sb:robot-vacuum-104b6cc0"]["joke_ids"].reverse()
    pk.board_items["sb:robot-vacuum-104b6cc0"]["updated_at"] = "u2"
    run()
    assert d.text(BOARD_DOC).index("No unit") < d.text(BOARD_DOC).index("Phone autocorrect")


def test_bootstrap_seeds_corpus_and_exports_only_changed(env, tmp_path):
    cfg, d, pk, run = env
    same = d.user_create("airport security", "old body", modified=OLD)
    changed = d.user_create("internet bit", "new body", modified="2026-10-26T08:00:00.000Z")
    for fid, mod, body in [(same, OLD, "old body"), (changed, "2026-10-24T08:00:00.000Z", "old internet")]:
        (cfg.corpus_gdocs / f"x__{fid[:8]}.md").write_text(
            f"---\nsource: gdoc\ndoc_id: {fid}\ntitle: \"t\"\ncreated: {mod}\nmodified: {mod}\nurl: u\n---\n{body}",
            encoding="utf-8")
    pk.graph_docs = {same: ["joke:airport-bit"], changed: ["joke:internet-bit"]}
    rec = run()
    assert rec["mode"] == "bootstrap"
    assert same not in d.exported
    assert changed in d.exported and rec["counts"].get("edit_units") == 1
    assert not [x for x in pk.lines if x.get("id") == "joke:internet-bit"], "catalog summaries are not overwritten"
    assert state(cfg)["docs"][same]["jokes"] == ["joke:airport-bit"]


def test_settling_doc_waits_then_syncs(env):
    cfg, d, pk, run = env
    run()
    fid = d.user_create("typing now", "half a joke", modified="2026-10-26T11:59:30.000Z")
    rec = run()
    assert rec["counts"].get("settling") == 1 and fid in state(cfg)["pending"]
    d.files[fid]["modifiedTime"] = "2026-10-26T11:50:00.000Z"
    run()
    assert state(cfg)["docs"][fid]["joke_id"] and fid not in state(cfg)["pending"]


def test_dry_run_writes_nothing(env):
    cfg, d, pk, run = env
    pk.app_idea("Idea", "text")
    d.user_create("New doc", "text2")
    rec = run(dry=True)
    assert rec["dry_run"] and not d.written and len(pk.lines) == 1
    assert not (cfg.state / "map.json").exists()


def test_api_down_changes_nothing(env):
    cfg, d, pk, run = env
    pk.up = False
    rec = run()
    assert rec["mode"] == "api_down" and not d.exported and not d.written


def test_crash_between_idea_and_map_does_not_duplicate(env):
    cfg, d, pk, run = env
    run()
    fid = d.user_create("Crashy", "text")
    pk.add_idea("text", "Crashy", {}, f"[gdocs] new Google Doc 'Crashy' ({fid}, modified {OLD})")  # written, map lost
    run()
    assert len([x for x in pk.lines if x["action"] == "add"]) == 1


def test_adopted_joke_docs_are_mapped_not_new_not_pushed_twice(env):
    cfg, d, pk, run = env
    for did, jid in ADOPTED_JOKE_DOCS.items():
        title = jid.split(":", 1)[1].replace("-", " ")
        pk.app_idea(title, f"{title}\n\nSETUP: s\nTAG: t")
        assert jid in pk.jokes
        d.user_create(title, f"{title}\n\nSETUP: s\nTAG: t", fid=did, parents=(FOLDER_ID,))  # connector-made: no tag
    n = len(pk.lines)
    run()
    assert len(pk.lines) == n, "an adopted doc came back as a new idea"
    pushed = [f for op, f in d.written if op == "create" and d.files[f]["appProperties"].get("punchable_kind") == "joke"]
    assert not pushed, "a duplicate doc was pushed"
    m = state(cfg)
    for did, jid in ADOPTED_JOKE_DOCS.items():
        assert m["docs"][did]["joke_id"] == jid and m["docs"][did]["version"] == "1"
    run()
    assert len(pk.lines) == n and not [f for _, f in d.written if f in ADOPTED_JOKE_DOCS]
    did, jid = next(iter(ADOPTED_JOKE_DOCS.items()))
    d.user_edit(did, "his edit in the doc")
    run()
    assert pk.jokes[jid]["summary"] == "his edit in the doc"
    pk.app_edit(jid, "his edit in the app")
    run()
    assert d.text(did) == "his edit in the app", "adopted docs are Punchable docs: app edits go to them"


def test_mixed_doc_only_text_above_the_line_is_his(env):
    cfg, d, pk, run = env
    did, jid = next(iter(MIXED_DOCS.items()))
    assert jid == "joke:good-data-the-soup-label"
    pk.app_idea("Good data: the soup label", "his soup words")
    d.user_create("Good data: the soup label",
                  "his soup words\n\n---\n\nPunchable notes (not your words)\nAI NOTE TEXT", fid=did, parents=(FOLDER_ID,))
    n = len(pk.lines)
    run()
    assert len(pk.lines) == n
    assert not [f for op, f in d.written if op == "create" and d.files[f]["appProperties"].get("punchable_joke") == jid]
    e = state(cfg)["docs"][did]
    assert e["origin"] == "mixed" and e["contains_drafts"] is True
    d.user_edit(did, "his NEW soup words\n\n---\n\nPunchable notes (not your words)\nAI NOTE TEXT")
    run()
    assert pk.jokes[jid]["summary"] == "his NEW soup words"
    assert "AI NOTE TEXT" not in json.dumps(pk.lines)
    pk.app_edit(jid, "app change")
    run()
    assert "AI NOTE TEXT" in d.text(did) and did not in {f for _, f in d.written}
    assert read_jsonl(cfg.state / "conflicts.jsonl")[-1]["kind"] == "mixed_doc_not_written"
