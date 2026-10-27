"""In-memory Drive and Punchable for tests. FakeDrive keeps the write gate (it subclasses DriveBase) and records every
export and write, so tests can prove a legal doc was never exported and his docs were never written."""
from __future__ import annotations

import itertools
import json
import re
from datetime import datetime, timezone
from pathlib import Path

from gsync.config import DOC_MIME, FOLDER_MIME, Config
from gsync.drive import DriveBase
from gsync.punchable import Punchable

ME = "owner@example.com"
ROOT = "root0"
OLD = "2026-10-20T10:00:00.000Z"


class FakeDrive(DriveBase):
    def __init__(self):
        super().__init__()
        self.files: dict[str, dict] = {ROOT: {"id": ROOT, "name": "My Drive", "mimeType": FOLDER_MIME, "parents": []}}
        self.log: list[tuple[str, bool]] = []
        self.exported: list[str] = []
        self.written: list[tuple[str, str]] = []
        self._ids = itertools.count(1)

    # ---- what the comic does in Drive -----------------------------------------------------------------------------------
    def folder(self, fid, parents=(ROOT,), name="folder"):
        self.files[fid] = {"id": fid, "name": name, "mimeType": FOLDER_MIME, "parents": list(parents)}
        return fid

    def user_create(self, name, text, fid=None, parents=(ROOT,), owner=ME, modified=OLD, props=None):
        fid = fid or f"doc{next(self._ids):04d}xxxxxxxx"
        self.files[fid] = {"id": fid, "name": name, "mimeType": DOC_MIME, "parents": list(parents),
                           "owners": [{"emailAddress": owner}], "createdTime": modified, "modifiedTime": modified,
                           "version": "1", "trashed": False, "appProperties": dict(props or {}), "_text": text}
        self.log.append((fid, False))
        return fid

    def user_edit(self, fid, text=None, name=None, modified="2026-10-26T09:00:00.000Z"):
        f = self.files[fid]
        if text is not None:
            f["_text"] = text
        if name is not None:
            f["name"] = name
        f["version"] = str(int(f["version"]) + 1)
        f["modifiedTime"] = modified
        self.log.append((fid, False))

    def user_trash(self, fid):
        self.files[fid]["trashed"] = True
        self.files[fid]["version"] = str(int(self.files[fid]["version"]) + 1)
        self.log.append((fid, False))

    def text(self, fid):
        return self.files[fid]["_text"]

    # ---- the Drive API surface -----------------------------------------------------------------------------------------
    def _meta(self, fid):
        return {k: v for k, v in self.files[fid].items() if not k.startswith("_")}

    def start_token(self):
        return str(len(self.log))

    def changes(self, token):
        out = [{"fileId": fid, "removed": fid not in self.files, "file": self._meta(fid) if fid in self.files else None}
               for fid, _ in self.log[int(token):]]
        return out, str(len(self.log))

    def list_docs(self):
        return [self._meta(k) for k, f in self.files.items() if f["mimeType"] == DOC_MIME and not f.get("trashed")
                and (f.get("owners") or [{}])[0].get("emailAddress") == ME]

    def get(self, fid):
        return self._meta(fid) if fid in self.files else None

    def find_by_prop(self, key, value):
        for k, f in self.files.items():
            if (f.get("appProperties") or {}).get(key) == value and not f.get("trashed"):
                return self._meta(k)
        return None

    def _export(self, fid, mime):
        self.exported.append(fid)
        return "\ufeff" + self.files[fid]["_text"].replace("\n", "\r\n")  # what Drive's text/plain export looks like

    def _create(self, name, text, parent, props):
        fid = f"new{next(self._ids):04d}xxxxxxxx"
        self.files[fid] = {"id": fid, "name": name, "mimeType": DOC_MIME, "parents": [parent] if parent else [ROOT],
                           "owners": [{"emailAddress": ME}], "createdTime": OLD, "modifiedTime": OLD, "version": "1",
                           "trashed": False, "appProperties": dict(props), "_text": text}
        self.log.append((fid, False))
        self.written.append(("create", fid))
        return self._meta(fid)

    def _update(self, fid, text, name):
        f = self.files[fid]
        f["_text"] = text
        if name:
            f["name"] = name
        f["version"] = str(int(f["version"]) + 1)
        self.log.append((fid, False))
        self.written.append(("update", fid))
        return self._meta(fid)

    def _trash(self, fid):
        self.files[fid]["trashed"] = True
        self.log.append((fid, False))
        self.written.append(("trash", fid))

    def _create_folder(self, name, parent, props):
        fid = f"fold{next(self._ids):04d}xxxxxxxx"
        self.files[fid] = {"id": fid, "name": name, "mimeType": FOLDER_MIME, "parents": [parent] if parent else [ROOT],
                           "owners": [{"emailAddress": ME}], "version": "1", "trashed": False,
                           "appProperties": dict(props)}
        self.log.append((fid, False))
        self.written.append(("create_folder", fid))
        return self._meta(fid)

    def _set_props(self, fid, props, name):
        f = self.files[fid]
        ap = dict(f.get("appProperties") or {})
        for k, v in props.items():
            if v is None:
                ap.pop(k, None)
            else:
                ap[k] = v
        f["appProperties"] = ap
        if name:
            f["name"] = name
        f["version"] = str(int(f["version"]) + 1)
        self.log.append((fid, False))
        self.written.append(("props", fid))
        return self._meta(fid)

    def _move(self, fid, add_parent, remove_parents):
        f = self.files[fid]
        ps = [p for p in f["parents"] if p not in remove_parents]
        if add_parent not in ps:
            ps.append(add_parent)
        f["parents"] = ps
        f["version"] = str(int(f["version"]) + 1)  # Drive bumps the version on a move; modifiedTime stays
        self.log.append((fid, False))
        self.written.append(("move", fid))
        return self._meta(fid)


    # ---- voice memos (read-only: list a folder, download a file's bytes) --------------------------------------------
    def user_upload(self, name, data: bytes, parents, mime="audio/x-m4a", created=OLD, owner=ME, fid=None):
        import hashlib
        fid = fid or f"aud{next(self._ids):04d}xxxxxxxx"
        self.files[fid] = {"id": fid, "name": name, "mimeType": mime, "parents": list(parents),
                           "owners": [{"emailAddress": owner}], "createdTime": created, "modifiedTime": created,
                           "version": "1", "trashed": False, "appProperties": {}, "size": str(len(data)),
                           "md5Checksum": hashlib.md5(data).hexdigest(), "_bytes": data}
        self.log.append((fid, False))
        return fid

    def list_folder(self, folder_id):
        return [self._meta(k) for k, f in self.files.items() if folder_id in (f.get("parents") or []) and not f.get("trashed")]

    def _download(self, fid, dest):
        self.downloaded = getattr(self, "downloaded", []) + [fid]
        data = self.files[fid]["_bytes"]
        Path(dest).write_bytes(data)
        return len(data)


def slug(s):
    return re.sub(r"[^a-z0-9]+", "-", s.lower()).strip("-")[:60] or "x"


class FakePunchable(Punchable):
    """Same reads/writes as Punchable, backed by a list of decision lines (app_jokes() is the real code)."""

    def __init__(self, cfg: Config):
        super().__init__(cfg)
        self.lines: list[dict] = []
        self.jokes: dict[str, dict] = {}
        self.board_items: dict = {}
        self.graph_docs: dict = {}
        self.merge_items: dict = {}
        self.up = True

    def health(self):
        return self.up

    def decisions(self):
        return self.lines

    def _line(self, **d):
        self.lines.append({"ts": "t", "by": "owner", **d})
        return len(self.lines) - 1

    def app_idea(self, title, text):  # the comic typing an idea in the app
        jid = "joke:" + slug(title)
        self.jokes[jid] = {"id": jid, "title": title, "summary": text, "stage": "idea", "lead_text": None, "hidden": False}
        self._line(target="node", id=jid, action="add", after={"type": "joke", "title": title, "summary": text},
                   reason="saved from review")
        return jid

    def app_edit(self, jid, text):
        self.jokes[jid]["summary"] = text
        self._line(target="node", id=jid, action="edit", after={"summary": text}, reason="")

    def add_idea(self, text, title, frm, reason):
        t = title or " ".join(text.split()[:7])
        jid, k = "joke:" + slug(t), 2
        while jid in self.jokes:
            jid, k = f"joke:{slug(t)}-{k}", k + 1
        self.jokes[jid] = {"id": jid, "title": t, "summary": text.strip(), "stage": "idea", "lead_text": None,
                           "hidden": False, "from": frm}
        i = self._line(target="node", id=jid, action="add", after={"type": "joke", "title": t, "summary": text}, reason=reason)
        return {"id": jid, "indexes": [i]}

    def edit_joke(self, jid, after, reason):
        j = self.jokes[jid]
        ch = {k: (v.strip() if isinstance(v, str) else v) for k, v in after.items() if j.get(k) != v}
        if not ch:
            return None
        before = {k: j.get(k) for k in ch}
        j.update(ch)
        return {"indexes": [self._line(target="node", id=jid, action="edit", before=before, after=ch, reason=reason)]}

    def undo(self, indexes):
        for i in indexes:
            d = self.lines[i]
            if d["action"] == "edit":
                self.jokes[d["id"]].update(d["before"])
            self._line(target="node", id=d["id"], action="undo", undo_of=i)
        return {"ok": True}

    def joke(self, jid):
        j = self.jokes.get(jid)
        return dict(j) if j else None

    def boards(self):
        return self.board_items

    def doc_jokes(self):
        return self.graph_docs

    def merges(self):
        return self.merge_items


def make_cfg(tmp: Path) -> Config:
    c = Config(comedy=tmp / "comedy", api="http://127.0.0.1:1", home=tmp / "home", memos_enabled=False)
    (c.comedy / "corpus" / "gdocs").mkdir(parents=True)
    return c


def clock():
    return datetime(2026, 10, 26, 12, 0, tzinfo=timezone.utc)


def read(p: Path):
    return json.loads(p.read_text(encoding="utf-8"))
