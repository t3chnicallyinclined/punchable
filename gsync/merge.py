"""The Google Docs side of a merge (the comic, 2026-10-26: "no reason to always have 1000's of google docs").

Read-only input: comedy/kb/creative/merges.json (the API's merge store snapshot; api-dev/merge_api.py writes it when
the comic taps Merge). For every merge, once:
- ONE consolidated Google Doc in the Punchable folder: the main joke's title, then every original's words verbatim, each
  under its date and source doc. gsync made it (appProperties punchable=1, punchable_kind=merged, punchable_merge=<id>).
- the source docs are MOVED into "Merged (archive)" inside the Punchable folder. Moving changes a doc's folder only:
  its text and name are never edited, nothing is trashed or deleted. A legal doc is never touched (the same metadata
  rule as the pull, checked again inside Drive.move). A doc that also holds a joke outside the merge stays where it is.
For an undone merge: each moved doc goes back to the folders it came from (if it is still in the archive), and the
merged doc releases its claim (punchable_merge/punchable_joke removed, renamed "(merge undone) ...", moved into the
archive). It is never trashed.
State lives in map.json `merges` {merge_id: {state, doc_id, moved:{doc:{from:[parents], pending?}}, left, ...}}.
Every move is recorded (pending) before it is made, so a crash between the two can always be undone.
"""
from __future__ import annotations

import json

from . import rules
from .drive import WriteBlocked
from .store import now_iso
from .units import h

ARCHIVE_NAME = "Merged (archive)"
NOT_MOVED_ORIGINS = ("board", "ai_draft", "mixed", "merged", "merged_undone")


def merge_text(rec: dict) -> str:
    """The consolidated doc: his words only, verbatim, dated. Catalog summaries (AI-written) are never written."""
    snap = rec.get("snapshot") or {}
    main = rec["main"]
    order = [main] + [j for j in rec.get("others") or [] if j != main]
    title = (snap.get(main) or {}).get("title") or main
    lines = [f"{title}", "",
             f"MERGED BY PUNCHABLE on {(rec.get('created_at') or '')[:10]} from {len(order)} versions of this bit. "
             f"Every line below is yours, verbatim, under its date and the doc it came from. The original docs are in "
             f"the \"{ARCHIVE_NAME}\" folder, unchanged."]
    for n, j in enumerate(order):
        s = snap.get(j) or {}
        head = f"{'MAIN: ' if n == 0 else ''}{s.get('title') or j}"
        lines += ["", "=" * 8 + " " + head + " " + "=" * 8]
        wrote = False
        for u in s.get("units") or []:
            src = f" · from \"{u['doc_title']}\"" if u.get("doc_title") else ""
            lines += ["", f"[{u.get('date') or 'undated'}{src}]", u.get("text") or ""]
            wrote = True
        if s.get("summary_is_his") and s.get("summary"):
            lines += ["", f"[{s.get('added_at') or s.get('first_seen') or 'undated'} · typed in Punchable]", s["summary"]]
            wrote = True
        if not wrote:
            lines += ["", "(no written text of yours: this one is known from tape only)"]
    return "\n".join(lines) + "\n"


def source_docs(sync, rec: dict) -> list[str]:
    snap = rec.get("snapshot") or {}
    group = {rec["main"], *(rec.get("others") or [])}
    out = []
    for j in [rec["main"]] + list(rec.get("others") or []):
        for u in (snap.get(j) or {}).get("units") or []:
            if u.get("doc_id"):
                out.append(u["doc_id"])
    for did, e in sync.m["docs"].items():
        if e.get("joke_id") in group and e.get("origin") not in NOT_MOVED_ORIGINS:
            out.append(did)
    return list(dict.fromkeys(out))


class MergeSync:
    def __init__(self, sync):
        self.s = sync
        self.mm = sync.m.setdefault("merges", {})

    # ---- entry ----------------------------------------------------------------------------------------------------
    def run(self) -> None:
        for mid, rec in sorted(self.s.pk.merges().items()):
            if rec.get("kind") != "merge":
                continue
            st = self.mm.get(mid)
            try:
                if rec.get("status") == "merged" and (not st or st.get("state") in ("applying", "error")):
                    self.apply(mid, rec)
                elif rec.get("status") == "undone" and st and st.get("state") in ("applied", "applying", "undoing",
                                                                                    "error"):
                    self.undo(mid, rec)
                elif rec.get("status") == "undone" and not st:
                    if not self.s.dry:
                        self.mm[mid] = {"state": "undone_before_applied", "undone_at": now_iso()}
            except WriteBlocked:
                raise
            except Exception as e:  # noqa: BLE001  one bad merge never stops the run; it is retried next run
                if mid in self.mm and not self.s.dry:
                    self.mm[mid]["error"] = f"{type(e).__name__}: {e}"[:300]
                self.s.rec["errors"].append({"merge": mid, "error": f"{type(e).__name__}: {e}"})
            self.s._save()

    # ---- the archive folder ---------------------------------------------------------------------------------------
    def archive(self) -> str:
        m = self.s.m
        fid = m.get("archive_folder_id")
        if fid:
            meta = self.s.drive.get(fid)
            if meta and not meta.get("trashed"):
                return fid
        found = self.s.drive.find_by_prop("punchable_kind", "merge_archive")  # crash guard: made but not recorded
        meta = found or self.s.drive.create_folder(ARCHIVE_NAME, m.get("folder_id"), {"punchable_kind": "merge_archive"})
        m["archive_folder_id"] = meta["id"]
        m["folders"][meta["id"]] = list(meta.get("parents") or [m.get("folder_id")])
        self.s._save()
        self.s.act("merge_archive_folder", doc=meta["id"])
        return meta["id"]

    # ---- apply ----------------------------------------------------------------------------------------------------
    def apply(self, mid: str, rec: dict) -> None:
        s = self.s
        docs = source_docs(s, rec)
        if s.dry:
            s.act("merge_apply", merge=mid, docs=len(docs))
            return
        st = self.mm.setdefault(mid, {"state": "applying", "moved": {}, "left": {}, "skipped_legal": []})
        st.update(state="applying", main=rec["main"], error=None)
        arch = self.archive()
        st["archive_folder"] = arch
        # 1. the one consolidated doc (found again by its tag after a crash, never made twice)
        if not st.get("doc_id"):
            text = merge_text(rec)
            title = ((rec.get("snapshot") or {}).get(rec["main"]) or {}).get("title") or rec["main"]
            found = s.drive.find_by_prop("punchable_merge", mid)
            meta = found or s.drive.create_doc(f"{title} (merged)", text, s.m.get("folder_id"),
                                               {"punchable_kind": "merged", "punchable_merge": mid,
                                                "punchable_joke": rec["main"]})
            s.drive.allow(meta["id"])
            st.update(doc_id=meta["id"], doc_url=f"https://docs.google.com/document/d/{meta['id']}/edit")
            s.m["docs"][meta["id"]] = {"origin": "merged", "merge_id": mid, "merged_joke": rec["main"],
                                       "title": meta.get("name"), "version": meta.get("version"),
                                       "modified": meta.get("modifiedTime"), "doc_hash": h(text)}
            s._save()
            s.act("merge_doc", merge=mid, doc=meta["id"])
        # 2. move every source doc into the archive (recorded before each move)
        group = {rec["main"], *(rec.get("others") or [])}
        doc_jokes = s.pk.doc_jokes()
        for did in docs:
            if did in st["moved"] and not st["moved"][did].get("pending"):
                continue
            meta = s.drive.get(did)
            if meta is None or meta.get("trashed"):
                st["left"][did] = "gone from Drive"
                continue
            anc = s.ancestors(meta)
            if did in s.m["excluded"] or rules.legal_reason(meta, anc, extra_ids=frozenset(s.m["excluded"])):
                if did not in st["skipped_legal"]:
                    st["skipped_legal"].append(did)  # the ID only; never read, never moved
                continue
            parents = list(meta.get("parents") or [])
            if arch in parents:
                st["moved"].setdefault(did, {"from": [p for p in parents if p != arch]})
                st["moved"][did].pop("pending", None)
                continue
            outside = sorted(set(doc_jokes.get(did) or []) - group)
            if outside:
                st["left"][did] = f"also holds {len(outside)} other joke{'s' if len(outside) > 1 else ''}; left in place"
                continue
            st["moved"][did] = {"from": parents, "pending": True}
            s._save()
            new = s.drive.move(did, arch, parents, anc)
            st["moved"][did].pop("pending", None)
            e = s.m["docs"].get(did)
            if e is not None:  # our own move must not come back through the Changes feed as an edit
                e["version"] = new.get("version", e.get("version"))
                e["archived_by_merge"] = mid
            s._save()
            s.act("merge_move", merge=mid, doc=did)
        st.update(state="applied", applied_at=now_iso())

    # ---- undo -----------------------------------------------------------------------------------------------------
    def undo(self, mid: str, rec: dict) -> None:
        s = self.s
        st = self.mm[mid]
        if s.dry:
            s.act("merge_undo", merge=mid, docs=len(st.get("moved") or {}))
            return
        st["state"] = "undoing"
        arch = st.get("archive_folder") or s.m.get("archive_folder_id")
        back = st.setdefault("moved_back", {})
        for did, mv in sorted((st.get("moved") or {}).items()):
            if did in back:
                continue
            meta = s.drive.get(did)
            if meta is None or meta.get("trashed"):
                back[did] = "gone from Drive"
                continue
            parents = list(meta.get("parents") or [])
            if arch not in parents:
                back[did] = "not in the archive any more (you moved it): left where it is"
                continue
            dest = [p for p in mv.get("from") or [] if p != arch] or [s.m.get("folder_id")]
            new = s.drive.move(did, dest[0], [arch], s.ancestors(meta))
            for extra in dest[1:]:  # a doc that lived in several folders goes back to all of them
                new = s.drive.move(did, extra, [], s.ancestors(meta))
            back[did] = "moved back"
            e = s.m["docs"].get(did)
            if e is not None:
                e["version"] = new.get("version", e.get("version"))
                e.pop("archived_by_merge", None)
            s._save()
            s.act("merge_move_back", merge=mid, doc=did)
        did = st.get("doc_id")
        if did and not st.get("released"):
            meta = s.drive.get(did)
            if meta and not meta.get("trashed"):
                s.drive.allow(did)
                new = s.drive.set_props(did, {"punchable_kind": "merge_undone", "punchable_merge": None,
                                              "punchable_joke": None},
                                        name=f"(merge undone) {meta.get('name') or ''}".strip())
                if arch and arch not in (meta.get("parents") or []):
                    new = s.drive.move(did, arch, list(meta.get("parents") or []), s.ancestors(meta))
                e = s.m["docs"].get(did) or {}
                e.update(origin="merged_undone", version=new.get("version"), hidden=True)
                s.m["docs"][did] = e
            st["released"] = True
            s.act("merge_release", merge=mid, doc=did)
        st.update(state="undone", undone_at=now_iso())


def merges_from(path) -> dict:
    """merges.json items (the creative store snapshot), read-only."""
    try:
        items = json.loads(path.read_text(encoding="utf-8")).get("items") or {}
    except (OSError, ValueError):
        return {}
    return items if isinstance(items, dict) else {}
