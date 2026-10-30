"""The sync engine: bootstrap (first run) / pull (Changes API) / push (app jokes) / boards. See GSYNC.md."""
from __future__ import annotations

import os
import re
import traceback
from datetime import datetime, timezone

from . import rules
from .config import DOC_MIME, FOLDER_MIME, Config
from .drive import DriveBase, WriteBlocked
from .punchable import GDOCS, ApiDown, Punchable
from .store import Store, now_iso
from .units import h, mirror_name, mirror_text, norm, split_units

OWNER_ID = os.environ.get("PK_OWNER_ID", "owner")  # the actor id on your own edits (`by`); PK_OWNER_ID


def _ts(s: str | None) -> datetime | None:
    if not s:
        return None
    return datetime.fromisoformat(s.replace("Z", "+00:00"))


def _untitled(name: str | None) -> bool:
    return (name or "").strip().lower().startswith("untitled document") or not (name or "").strip()


class Sync:
    def __init__(self, cfg: Config, drive: DriveBase, pk: Punchable, dry_run: bool = False, clock=None, store=None):
        self.cfg, self.drive, self.pk, self.dry = cfg, drive, pk, dry_run
        self.clock = clock or (lambda: datetime.now(timezone.utc))
        self.store = store or Store(cfg.state)
        self.m = self.store.m
        for k in ("docs", "boards", "excluded", "skipped", "pending", "folders"):
            self.m.setdefault(k, {})
        self.rec: dict = {}

    # ---- bookkeeping ------------------------------------------------------------------------------------------------
    def act(self, op: str, **kw) -> None:
        self.rec["actions"].append({"op": op, **{k: v for k, v in kw.items() if v is not None}})
        self.rec["counts"][op] = self.rec["counts"].get(op, 0) + 1

    def _conflict(self, **kw) -> None:
        if not self.dry:
            self.store.log("conflicts.jsonl", {"ts": now_iso(), "run_id": self.rec["run_id"], **kw})
        self.act("conflict", doc=kw.get("doc_id"), joke=kw.get("joke_id"), kind=kw.get("kind"))

    def _save(self) -> None:
        if not self.dry:
            self.store.save()

    # ---- entry point ------------------------------------------------------------------------------------------------
    def run(self) -> dict:
        t0 = self.clock()
        self.rec = {"run_id": "gs-" + t0.strftime("%Y%m%d-%H%M%S"), "ts": now_iso(), "dry_run": self.dry,
                    "mode": None, "actions": [], "counts": {}, "errors": []}
        try:
            if not self.pk.health():
                self.rec["mode"] = "api_down"
                return self._finish()
            self.adopt()
            if self.m.get("changes_token") is None:
                self.rec["mode"] = "bootstrap"
                self.bootstrap()
            else:
                self.rec["mode"] = "pull"
                self.pull()
            self.push_jokes()
            self.push_boards()
            self.push_merges()
            self.pull_memos()
        except ApiDown as e:
            self.rec["errors"].append({"where": "api", "error": f"api_down: {e}"})
        except Exception as e:  # noqa: BLE001
            self.rec["errors"].append({"where": "run", "error": f"{type(e).__name__}: {e}",
                                       "trace": traceback.format_exc(limit=4)})
        return self._finish()

    def _finish(self) -> dict:
        self.rec["drive"] = dict(self.drive.stats)
        self.rec["api_calls"] = getattr(self.pk, "calls", None)
        self.rec["ts_end"] = now_iso()
        self._save()
        if not self.dry:
            self.store.log("runs.jsonl", self.rec)
        return self.rec

    # ---- adoption ---------------------------------------------------------------------------------------------------
    def adopt(self) -> None:
        m = self.m
        if self.cfg.scopes_mode != "narrow":
            m.setdefault("folder_id", self.cfg.folder_id)
            for sb, did in self.cfg.board_docs.items():
                if sb not in m["boards"]:
                    m["boards"][sb] = {"doc_id": did, "hash": None, "version": None, "adopted": True}
                    m["docs"][did] = {"origin": "board", "board": sb}
                    self.act("adopt", doc=did, kind="board")
        else:
            m.setdefault("folder_id", None)
        for did, title in self.cfg.ai_draft_docs.items():
            if did not in m["docs"]:
                m["docs"][did] = {"origin": "ai_draft", "source": "ai_draft", "contains_drafts": True, "title": title,
                                  "joke_id": None, "version": None, "doc_hash": None}
                self.act("adopt", doc=did, kind="ai_draft")
        for did, jid in self.cfg.mixed_docs.items():
            if did not in m["docs"]:
                m["docs"][did] = {"origin": "mixed", "source": "mixed", "contains_drafts": True, "joke_id": jid,
                                  "version": None, "doc_hash": None, "app_hash": None}
                self.act("adopt", doc=did, joke=jid, kind="mixed")
        for did, jid in self.cfg.adopted_joke_docs.items():
            if did not in m["docs"]:
                m["docs"][did] = {"origin": "app", "adopted": True, "joke_id": jid, "version": None, "doc_hash": None,
                                  "app_hash": None}
                self.act("adopt", doc=did, joke=jid, kind="joke")
        for b in m["boards"].values():
            if b.get("doc_id") and (self.cfg.scopes_mode != "narrow" or not b.get("adopted")):
                self.drive.allow(b["doc_id"])
        for did, e in m["docs"].items():
            if e.get("origin") == "app":
                self.drive.allow(did)

    # ---- bootstrap --------------------------------------------------------------------------------------------------
    def seed_corpus(self) -> int:
        dj, n = self.pk.doc_jokes(), 0
        for f in sorted(self.cfg.corpus_gdocs.glob("*.md")) if self.cfg.corpus_gdocs.exists() else []:
            head = f.read_text(encoding="utf-8").split("\n---", 1)[0]
            fm = dict(re.findall(r"^(\w+): (.*)$", head, re.M))
            did = fm.get("doc_id")
            if not did or did in self.m["docs"]:
                continue
            self.m["docs"][did] = {"origin": "corpus", "jokes": dj.get(did, []), "modified": fm.get("modified"),
                                   "title": fm.get("title", "").strip('"'), "file": f"corpus/gdocs/{f.name}",
                                   "version": None, "doc_hash": None}
            n += 1
        return n

    def bootstrap(self) -> None:
        token = self.drive.start_token()  # before listing, so nothing changed during the listing is missed
        self.act("seed_corpus", n=self.seed_corpus())
        listed = self.drive.list_docs()
        ids = {x["id"] for x in listed}
        for meta in listed:
            self._safe(meta, bootstrap=True)
        for did, e in list(self.m["docs"].items()):
            if e.get("origin") in ("corpus", "gdocs") and did not in ids and not e.get("hidden") and not e.get("excluded"):
                self.gone(did)
        self.m["bootstrapped_at"] = now_iso()
        if not self.dry and not self.rec["errors"]:
            self.m["changes_token"] = token

    # ---- pull -------------------------------------------------------------------------------------------------------
    def pull(self) -> None:
        changes, new_token = self.drive.changes(self.m["changes_token"])
        last: dict[str, dict] = {}
        for c in changes:
            last[c["fileId"]] = c
        for fid, c in last.items():
            f = c.get("file") or {}
            if c.get("removed") or f.get("trashed"):
                if fid in self.m["docs"]:
                    self.gone(fid)
                self.m["pending"].pop(fid, None)
                continue
            if f.get("mimeType") == FOLDER_MIME:
                self.m["folders"][fid] = list(f.get("parents") or [])
                continue
            if f.get("mimeType") == DOC_MIME:
                self._safe(f)
        for fid in [k for k in list(self.m["pending"]) if k not in last]:
            meta = self.drive.get(fid)
            if meta is None or meta.get("trashed"):
                self.m["pending"].pop(fid, None)
                if fid in self.m["docs"]:
                    self.gone(fid)
            else:
                self._safe(meta)
        if not self.dry:
            self.m["changes_token"] = new_token

    def _safe(self, meta: dict, bootstrap: bool = False) -> None:
        try:
            self.handle(meta, bootstrap)
        except ApiDown:
            raise
        except Exception as e:  # noqa: BLE001  one bad doc never stops the run; it is retried next run
            self.m["pending"][meta["id"]] = "error"
            self.rec["errors"].append({"doc": meta["id"], "error": f"{type(e).__name__}: {e}"})

    def ancestors(self, meta: dict) -> list[str]:
        cache, out, seen = self.m["folders"], [], set()
        todo = list(meta.get("parents") or [])
        while todo:
            p = todo.pop()
            if p in seen:
                continue
            seen.add(p)
            out.append(p)
            if p not in cache:
                fm = self.drive.get(p)
                cache[p] = list((fm or {}).get("parents") or [])
            todo += cache[p]
        return out

    def handle(self, meta: dict, bootstrap: bool = False) -> None:
        m, fid = self.m, meta["id"]
        # 1. legal: metadata only, before anything is exported
        why = rules.legal_reason(meta, self.ancestors(meta), extra_ids=frozenset(m["excluded"]))
        if why:
            m["excluded"].setdefault(fid, why)
            if fid in m["docs"]:
                m["docs"][fid]["excluded"] = True
            m["pending"].pop(fid, None)
            self.act("skip_legal", doc=fid, why=why.split(":")[0])
            return
        e = m["docs"].get(fid)
        props = meta.get("appProperties") or {}
        # 2. ours
        if (e and e.get("origin") == "board") or props.get("punchable_kind") == "board":
            return
        if e and e.get("origin") == "ai_draft":
            return self.ai_draft(meta, e)
        if e and e.get("origin") == "mixed":
            return self.mixed(meta, e)
        if e and e.get("origin") in ("merged", "merged_undone"):
            return self.merged_doc(meta, e)
        if e and (e.get("excluded") or e.get("detached")):
            return
        if e and e.get("origin") == "app" and e.get("version") is None:
            # adopted by id (made through the connector, no appProperties): today's state is the baseline
            cur = self.pk.joke(e["joke_id"]) if e.get("joke_id") else None
            e.update(version=meta.get("version"), modified=meta.get("modifiedTime"), title=meta.get("name"),
                     doc_hash=h(self.drive.export(fid, "text/plain")), app_hash=h(cur["summary"]) if cur else None)
            self.m["pending"].pop(fid, None)
            self.act("baseline", doc=fid, joke=e.get("joke_id"), kind="joke")
            return
        if e and e.get("version") is not None and str(meta.get("version")) == str(e["version"]):
            return  # our own write, or nothing new: the loop guard
        if e and e.get("origin") == "corpus" and e.get("version") is None \
                and (meta.get("modifiedTime") or "") <= (e.get("modified") or ""):
            e["version"] = meta.get("version")  # unchanged since the ingest: remember its version, export nothing
            return
        if not e and fid in m["skipped"] and bootstrap:
            return
        # 3. settling
        mod = _ts(meta.get("modifiedTime"))
        if mod and (self.clock() - mod).total_seconds() < self.cfg.settle_s:
            m["pending"][fid] = "settling"
            self.act("settling", doc=fid)
            return
        # 4. comedy triage (new docs only; a mapped doc is already his material)
        if not e:
            t = rules.triage(meta)
            if t != "comedy":
                m["skipped"][fid] = t
                m["pending"].pop(fid, None)
                self.act("skip_not_comedy", doc=fid)
                return
        # 5. export, then the content net
        plain = norm(self.drive.export(fid, "text/plain"))
        if rules.content_is_legal(plain):
            plain = None
            m["excluded"][fid] = "content"
            if e:
                e["excluded"] = True
            m["pending"].pop(fid, None)
            self.act("skip_legal", doc=fid, why="content")
            return
        if not e and not plain and _untitled(meta.get("name")):
            m["pending"].pop(fid, None)
            self.act("wait_empty", doc=fid)  # it comes back through the Changes feed once he types
            return
        md = self.drive.export(fid, "text/markdown").replace("\ufeff", "").replace("\r\n", "\n")
        if e:
            self.edited(meta, e, plain, md)
        else:
            self.new(meta, plain, md)
        m["pending"].pop(fid, None)

    def _mirror(self, meta: dict, md: str) -> str | None:
        if self.dry:
            return None
        name = mirror_name(meta)
        rel = self.store.put_mirror(name, mirror_text(meta, md))
        self.store.put_units(meta["id"], split_units(meta, md, rel))
        return rel

    def _joke_for_doc(self, fid: str) -> str | None:
        """Crash guard: a gsync idea for this doc that reached decisions.jsonl but not map.json."""
        ds = self.pk.decisions()
        undone = {d["undo_of"] for d in ds if isinstance(d.get("undo_of"), int)}
        for i, d in enumerate(ds):
            if d.get("action") == "add" and i not in undone and (d.get("reason") or "").startswith(GDOCS) \
                    and f"({fid}," in (d.get("reason") or ""):
                return d.get("id")
        return None

    def new(self, meta: dict, plain: str, md: str) -> None:
        fid, name = meta["id"], meta.get("name") or ""
        text = plain or name  # an empty body: the title is the premise (as in the ingest)
        title = None if _untitled(name) else name[:120]
        if self.dry:
            self.act("new", doc=fid, title=name)
            return
        jid = self._joke_for_doc(fid)
        idx = None
        if not jid:
            r = self.pk.add_idea(text, title, {"gdoc_id": fid, "title": name, "modified": meta.get("modifiedTime"),
                                               "note": f"Google Doc: {name}"},
                                 f"{GDOCS} new Google Doc '{name}' ({fid}, modified {meta.get('modifiedTime')})")
            jid, idx = r["id"], r["indexes"]
        self.m["docs"][fid] = {"origin": "gdocs", "joke_id": jid, "title": name, "version": meta.get("version"),
                               "modified": meta.get("modifiedTime"), "doc_hash": h(plain), "app_hash": h(text),
                               "file": self._mirror(meta, md)}
        self._save()  # every write is recorded before the next one
        self.act("new", doc=fid, joke=jid, indexes=idx, title=name)

    def edited(self, meta: dict, e: dict, plain: str, md: str) -> None:
        fid, name = meta["id"], meta.get("name") or ""
        dh = h(plain)
        if e.get("doc_hash") is None and e.get("origin") == "corpus" and e.get("file"):
            p = self.cfg.comedy / e["file"]
            if p.exists() and norm(p.read_text(encoding="utf-8").split("\n---\n", 1)[-1]) == norm(md):
                e["doc_hash"] = dh
        if dh == e.get("doc_hash"):
            e.update(version=meta.get("version"), modified=meta.get("modifiedTime"))
            return
        if e.get("origin") == "corpus":
            if not self.dry:
                e["file_live"] = self._mirror(meta, md)
                self.store.log("edits.jsonl", {"ts": now_iso(), "by": OWNER_ID, "source": "gdocs", "doc_id": fid,
                                               "kind": "corpus_doc_edit", "jokes": e.get("jokes"),
                                               "modified": meta.get("modifiedTime"), "file": e["file_live"]})
                e.update(version=meta.get("version"), modified=meta.get("modifiedTime"), doc_hash=dh)
            self.act("edit_units", doc=fid, jokes=e.get("jokes"))
            return
        jid = e.get("joke_id")
        cur = self.pk.joke(jid) if jid else None
        if cur is None and jid and self._merged_into(jid):
            # the joke was merged into another one (hidden): his edit is kept, verbatim, and the doc stays synced
            if not self.dry:
                self.store.log("edits.jsonl", {"ts": now_iso(), "by": OWNER_ID, "source": "gdocs", "doc_id": fid,
                                               "kind": "merged_member_doc_edit", "joke_id": jid,
                                               "merged_into": self._merged_into(jid), "title": name,
                                               "modified": meta.get("modifiedTime"), "text": plain})
                e.update(version=meta.get("version"), modified=meta.get("modifiedTime"), doc_hash=dh)
            self.act("merged_member_edit", doc=fid, joke=jid)
            return
        if cur is None:
            e["detached"] = True  # he undid or removed the joke; the doc stays his and is no longer synced
            self.act("detached", doc=fid, joke=jid)
            return
        new_text = plain or name
        after = {"summary": new_text}
        if e.get("origin") in ("gdocs", "app") and not _untitled(name) and name != e.get("title") and name[:120] != cur["title"]:
            after["title"] = name[:120]
        if h(cur["summary"]) != e.get("app_hash"):
            self._conflict(kind="both_edited", doc_id=fid, joke_id=jid, title=cur["title"], app_text=cur["summary"],
                           doc_text=new_text, resolution="the doc's text was applied (the doc is the source of truth); "
                           "the app text is kept here verbatim, and undoing this run restores it")
        if self.dry:
            self.act("edit", doc=fid, joke=jid)
            return
        r = self.pk.edit_joke(jid, after, f"{GDOCS} edited in Google Docs '{name}' ({fid}, modified {meta.get('modifiedTime')})")
        e.update(version=meta.get("version"), modified=meta.get("modifiedTime"), doc_hash=dh, app_hash=h(new_text),
                 title=name, file=self._mirror(meta, md))
        self._save()
        self.act("edit", doc=fid, joke=jid, indexes=(r or {}).get("indexes"))

    def ai_draft(self, meta: dict, e: dict) -> None:
        fid = meta["id"]
        if e.get("version") is None:
            e.update(version=meta.get("version"), modified=meta.get("modifiedTime"))
            self.act("baseline", doc=fid, kind="ai_draft")
            return
        if str(meta.get("version")) == str(e["version"]):
            return
        mod = _ts(meta.get("modifiedTime"))
        if mod and (self.clock() - mod).total_seconds() < self.cfg.settle_s:
            self.m["pending"][fid] = "settling"
            return
        plain = norm(self.drive.export(fid, "text/plain"))
        if h(plain) != e.get("doc_hash") and not self.dry:
            # his edit to a doc that holds AI drafts: stored as his edit, never as an idea or a his-words unit
            self.store.log("edits.jsonl", {"ts": now_iso(), "by": OWNER_ID, "source": "gdocs", "doc_id": fid,
                                           "kind": "ai_draft_doc_edit", "contains_drafts": True,
                                           "title": meta.get("name"), "modified": meta.get("modifiedTime"), "text": plain})
            self.act("ai_draft_edit", doc=fid)
        if not self.dry:
            e.update(version=meta.get("version"), modified=meta.get("modifiedTime"), doc_hash=h(plain), contains_drafts=True)
        self.m["pending"].pop(fid, None)

    HR = re.compile(r"^\s*([-*_])(\s*\1){2,}\s*$")

    def mixed(self, meta: dict, e: dict) -> None:
        """A Punchable-made doc: his verbatim words above the first horizontal rule, AI notes (ai_draft) below.
        Only the part above the rule is his text; gsync never writes this doc."""
        fid, jid = meta["id"], e.get("joke_id")
        cur = self.pk.joke(jid) if jid else None
        if e.get("version") is None:
            e.update(version=meta.get("version"), modified=meta.get("modifiedTime"),
                     app_hash=h(cur["summary"]) if cur else None)
            self.act("baseline", doc=fid, joke=jid, kind="mixed")
            return
        if str(meta.get("version")) == str(e["version"]):
            return
        mod = _ts(meta.get("modifiedTime"))
        if mod and (self.clock() - mod).total_seconds() < self.cfg.settle_s:
            self.m["pending"][fid] = "settling"
            return
        md = self.drive.export(fid, "text/markdown").replace("\ufeff", "").replace("\r\n", "\n")
        if rules.content_is_legal(md):
            self.m["excluded"][fid] = "content"
            e["excluded"] = True
            self.act("skip_legal", doc=fid, why="content")
            return
        lines = md.split("\n")
        cut = next((i for i, ln in enumerate(lines) if self.HR.match(ln)), None)
        self.m["pending"].pop(fid, None)
        if cut is None:  # the rule is gone: can't tell his words from the notes, so nothing is applied
            if not self.dry:
                self.store.log("edits.jsonl", {"ts": now_iso(), "by": OWNER_ID, "source": "gdocs", "doc_id": fid,
                                               "kind": "mixed_doc_edit_unsplit", "contains_drafts": True, "text": md})
                e.update(version=meta.get("version"), modified=meta.get("modifiedTime"))
            self.act("mixed_unsplit", doc=fid)
            return
        his = norm("\n".join(lines[:cut]))
        if cur is None or h(his) == h(cur["summary"]) or h(his) == e.get("doc_hash"):
            if not self.dry:
                e.update(version=meta.get("version"), modified=meta.get("modifiedTime"), doc_hash=h(his))
            return
        if e.get("app_hash") and h(cur["summary"]) != e["app_hash"]:
            self._conflict(kind="both_edited", doc_id=fid, joke_id=jid, title=cur["title"], app_text=cur["summary"],
                           doc_text=his, resolution="the doc's text (above the line) was applied; the app text is kept here")
        if self.dry:
            self.act("edit", doc=fid, joke=jid)
            return
        r = self.pk.edit_joke(jid, {"summary": his}, f"{GDOCS} edited in Google Docs (above the line) ({fid}, "
                                                     f"modified {meta.get('modifiedTime')})")
        self.store.log("edits.jsonl", {"ts": now_iso(), "by": OWNER_ID, "source": "gdocs", "doc_id": fid,
                                       "kind": "mixed_doc_edit", "contains_drafts": True, "joke_id": jid,
                                       "modified": meta.get("modifiedTime")})
        e.update(version=meta.get("version"), modified=meta.get("modifiedTime"), doc_hash=h(his), app_hash=h(his),
                 contains_drafts=True)
        self._save()
        self.act("edit", doc=fid, joke=jid, indexes=(r or {}).get("indexes"))

    def _merged_into(self, jid: str) -> str | None:
        for m in self.pk.merges().values():
            if m.get("kind") == "merge" and m.get("status") == "merged" and jid in (m.get("others") or []):
                return m.get("main")
        return None

    def merged_doc(self, meta: dict, e: dict) -> None:
        """A consolidated doc gsync wrote for a merge. Our own write is skipped by version; his later edits are kept
        verbatim in edits.jsonl (by: owner) and never turned into an idea."""
        fid = meta["id"]
        if str(meta.get("version")) == str(e.get("version")):
            return
        mod = _ts(meta.get("modifiedTime"))
        if mod and (self.clock() - mod).total_seconds() < self.cfg.settle_s:
            self.m["pending"][fid] = "settling"
            return
        plain = norm(self.drive.export(fid, "text/plain"))
        if h(plain) != e.get("doc_hash") and not self.dry:
            self.store.log("edits.jsonl", {"ts": now_iso(), "by": OWNER_ID, "source": "gdocs", "doc_id": fid,
                                           "kind": "merged_doc_edit", "merge_id": e.get("merge_id"),
                                           "joke_id": e.get("merged_joke"), "title": meta.get("name"),
                                           "modified": meta.get("modifiedTime"), "text": plain})
            self.act("merged_doc_edit", doc=fid, joke=e.get("merged_joke"))
        if not self.dry:
            e.update(version=meta.get("version"), modified=meta.get("modifiedTime"), doc_hash=h(plain))
        self.m["pending"].pop(fid, None)

    def push_merges(self) -> None:
        """Merges the comic made in the app -> one consolidated doc + the old docs moved to the archive (gsync/merge.py)."""
        from .merge import MergeSync
        MergeSync(self).run()

    def pull_memos(self) -> None:
        """iPhone voice memos saved to Drive > Punchable > Voice memos -> downloaded for transcription (gsync/memos.py)."""
        if self.cfg.memos_enabled:
            from .memos import MemoPull
            MemoPull(self).run()

    def gone(self, fid: str) -> None:
        e = self.m["docs"].get(fid)
        if not e or e.get("hidden"):
            return
        if e.get("origin") == "board":
            b = self.m["boards"].get(e.get("board"), {})
            b["doc_trashed"] = True  # he trashed a generated board doc: respect it, don't regenerate
            e["hidden"] = True
            self.act("gone", doc=fid, kind="board")
            return
        e.update(hidden=True, gone_at=now_iso())
        jid, idx = e.get("joke_id"), None
        if jid and e.get("origin") in ("gdocs", "app") and not self.dry:
            cur = self.pk.joke(jid)
            if cur and cur.get("stage") == "idea":
                r = self.pk.edit_joke(jid, {"stage": "retired"}, f"{GDOCS} Google Doc moved to trash ({fid}); hidden")
                idx = (r or {}).get("indexes")
        self._save()
        self.act("gone", doc=fid, joke=jid, indexes=idx)

    # ---- push: app jokes -> docs -------------------------------------------------------------------------------------
    def push_jokes(self) -> None:
        by_joke = {e["joke_id"]: (did, e) for did, e in self.m["docs"].items() if e.get("joke_id")}
        parent = self.m.get("folder_id")
        for jid in self.pk.app_jokes():
            cur = self.pk.joke(jid)
            if not cur or cur.get("hidden"):
                continue
            text = cur["summary"] or cur["title"]
            hit = by_joke.get(jid)
            if not hit:
                if self.dry:
                    self.act("push_new", joke=jid, title=cur["title"])
                    continue
                found = self.drive.find_by_prop("punchable_joke", jid)  # crash guard: a doc made but not recorded
                meta = found or self.drive.create_doc(cur["title"], text, parent,
                                                      {"punchable_kind": "joke", "punchable_joke": jid})
                self.drive.allow(meta["id"])
                self.m["docs"][meta["id"]] = {"origin": "app", "joke_id": jid, "title": cur["title"],
                                              "version": meta.get("version"), "modified": meta.get("modifiedTime"),
                                              "doc_hash": h(self.drive.export(meta["id"], "text/plain")),
                                              "app_hash": h(text)}
                self._save()
                self.act("push_new", joke=jid, doc=meta["id"], title=cur["title"])
                continue
            did, e = hit
            if e.get("version") is None:
                continue  # adopted, not seen in Drive yet: baseline first, never a second doc
            if e.get("hidden") or e.get("excluded") or e.get("detached") or h(text) == e.get("app_hash"):
                continue
            if e.get("origin") != "app":
                self._own_doc(did, e, cur, text)
                continue
            meta = self.drive.get(did)
            if meta is None or meta.get("trashed"):
                continue
            if str(meta.get("version")) != str(e.get("version")):
                if e.get("conflict_key") != [h(text), meta.get("version")]:
                    self._conflict(kind="doc_changed_since_sync", doc_id=did, joke_id=jid, title=cur["title"],
                                   app_text=text, resolution="the doc was not overwritten; its text is pulled next run")
                    e["conflict_key"] = [h(text), meta.get("version")]
                continue
            if self.dry:
                self.act("push_edit", joke=jid, doc=did)
                continue
            new = self.drive.update_doc(did, text, name=cur["title"] if cur["title"] != e.get("title") else None)
            e.update(version=new.get("version"), modified=new.get("modifiedTime"), app_hash=h(text), title=cur["title"],
                     doc_hash=h(self.drive.export(did, "text/plain")))
            self._save()
            self.act("push_edit", joke=jid, doc=did)
        # ideas that came from his own docs and were then edited in the app: noted, his doc is never written (v1)
        for did, e in list(self.m["docs"].items()):
            if e.get("origin") == "gdocs" and e.get("joke_id") and not (e.get("hidden") or e.get("excluded") or e.get("detached")):
                cur = self.pk.joke(e["joke_id"])
                if cur and h(cur["summary"]) != e.get("app_hash"):
                    self._own_doc(did, e, cur, cur["summary"])

    def _own_doc(self, did: str, e: dict, cur: dict, text: str) -> None:
        if e.get("noted_app_hash") == h(text):
            return
        if e.get("origin") == "mixed":
            self._conflict(kind="mixed_doc_not_written", doc_id=did, joke_id=cur["id"], title=cur["title"], app_text=text,
                           resolution="edited in the app; this doc mixes your words with Punchable notes, so it is never "
                           "rewritten; copy the change into the doc above the line")
        else:
            self._conflict(kind="own_doc_not_written", doc_id=did, joke_id=cur["id"], title=cur["title"], app_text=text,
                           resolution="edited in the app; your own Google Doc is never written in v1")
        e["noted_app_hash"] = h(text)

    # ---- push: storyboards -> board docs -----------------------------------------------------------------------------
    def board_text(self, b: dict) -> str:
        his = set(self.pk.app_jokes())
        doc_of = {e["joke_id"]: did for did, e in self.m["docs"].items() if e.get("joke_id") and not e.get("hidden")}
        lines = [f"{b['title']} (board)", "",
                 "GENERATED BY PUNCHABLE from your storyboard. This doc is rewritten whenever the board changes, so edit "
                 "the board in the app, not here. Every bit's text below is yours, verbatim; nothing here is AI-written.",
                 f"Board last changed: {b.get('updated_at')}"]
        if b.get("target_s"):
            lines.append(f"Target: {int(b['target_s']) // 60}:{int(b['target_s']) % 60:02d} · {len(b.get('joke_ids') or [])} bits")
        if b.get("notes") and b.get("by") == OWNER_ID:
            lines += ["", "Your notes:", b["notes"]]
        for n, jid in enumerate(b.get("joke_ids") or [], 1):
            j = self.pk.joke(jid) or {"title": jid, "summary": "", "lead_text": None}
            body = j.get("lead_text") or (j.get("summary") if (jid in his or jid in doc_of) else None)
            lines += ["", f"{n}. {j.get('title') or jid}"]
            if body:
                lines.append(body)
            if jid in doc_of:
                lines.append(f"Doc: https://docs.google.com/document/d/{doc_of[jid]}/edit")
        return "\n".join(lines) + "\n"

    def push_boards(self) -> None:
        parent = self.m.get("folder_id")
        for sb, b in sorted(self.pk.boards().items()):
            if not (b.get("by") == OWNER_ID or b.get("source") == "his"):
                continue  # untouched AI seed boards are drafts: never written to his Drive
            be = self.m["boards"].get(sb)
            if b.get("deleted"):
                if be and be.get("doc_id") and not be.get("trashed") and not be.get("doc_trashed") and not self.dry:
                    self.drive.trash(be["doc_id"])
                    be["trashed"] = True
                    self.act("board_trash", board=sb, doc=be["doc_id"])
                continue
            if be and be.get("doc_trashed"):
                continue
            text = self.board_text(b)
            if be and be.get("hash") == h(text):
                continue
            if self.dry:
                self.act("board", board=sb, doc=(be or {}).get("doc_id"))
                continue
            name = f"{b['title']} (board)"
            new = None
            if be and be.get("doc_id"):
                meta = self.drive.get(be["doc_id"])
                if meta is None or meta.get("trashed"):
                    be["doc_trashed"] = True
                    self.act("gone", doc=be["doc_id"], kind="board")
                    continue
                if be.get("version") is None or str(meta.get("version")) != str(be.get("version")):
                    prior = norm(self.drive.export(be["doc_id"], "text/plain"))
                    if h(prior) != be.get("doc_hash"):
                        self.store.log("edits.jsonl", {"ts": now_iso(), "by": OWNER_ID if be.get("version") else "adopted",
                                                       "source": "gdocs", "doc_id": be["doc_id"], "kind": "board_doc_before_regen",
                                                       "board": sb, "text": prior})
                try:
                    new = self.drive.update_doc(be["doc_id"], text, name=name)
                except WriteBlocked:
                    raise
                except Exception as ex:  # noqa: BLE001  adopted doc not writable (narrow scopes): make our own
                    if not be.get("adopted"):
                        raise
                    self.rec["errors"].append({"board": sb, "error": f"adopted board doc not writable: {ex}"})
            if new is None:
                new = self.drive.create_doc(name, text, parent, {"punchable_kind": "board", "punchable_board": sb})
                be = {"doc_id": new["id"]}
                self.m["docs"][new["id"]] = {"origin": "board", "board": sb}
            be.update(hash=h(text), version=new.get("version"), doc_hash=h(self.drive.export(be["doc_id"], "text/plain")))
            self.m["boards"][sb] = be
            self._save()
            self.act("board", board=sb, doc=be["doc_id"])
