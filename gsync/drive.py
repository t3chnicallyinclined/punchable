"""Drive v3 access. DriveBase holds the write gate; GoogleDrive is the real client (backoff + pacing); tests use a fake.

Write gate: gsync writes only to docs it created (appProperties.punchable == "1") or docs adopted by ID (the board
doc). Everything else, every one of his own docs included, raises WriteBlocked before any request is made.

Move gate (merges, gsync/merge.py): one of his docs may be MOVED between folders (its parents change; its text, name
and trashed state never do). `move` re-reads the doc's metadata and refuses a legal doc (rules.legal_reason), a doc
that is not his, a trashed doc and anything that is not a Google Doc, before any request is made.
"""
from __future__ import annotations

import io
import random
import time

from . import rules
from .config import DOC_MIME, FOLDER_MIME

META_FIELDS = "id,name,mimeType,trashed,parents,owners(emailAddress),createdTime,modifiedTime,version,appProperties"
# voice memos (gsync/memos.py): audio files are only listed and downloaded, never written
MEDIA_FIELDS = META_FIELDS + ",size,md5Checksum,fileExtension"
TAG = {"punchable": "1"}


class WriteBlocked(RuntimeError):
    pass


class DriveBase:
    def __init__(self):
        self.writable: set[str] = set()
        self.stats = {"calls": 0, "exports": 0, "writes": 0, "backoffs": 0, "downloads": 0}

    # ---- the gate -----------------------------------------------------------------------------------------------
    def allow(self, fid: str) -> None:
        self.writable.add(fid)

    def assert_writable(self, fid: str) -> None:
        if fid in self.writable:
            return
        meta = self.get(fid)
        if meta and (meta.get("appProperties") or {}).get("punchable") == "1":
            self.writable.add(fid)
            return
        raise WriteBlocked(f"gsync never writes to a doc it did not create: {fid}")

    def create_doc(self, name: str, text: str, parent: str, props: dict) -> dict:
        meta = self._create(name, text, parent, {**props, **TAG})
        self.writable.add(meta["id"])
        self.stats["writes"] += 1
        return meta

    def update_doc(self, fid: str, text: str, name: str | None = None) -> dict:
        self.assert_writable(fid)
        self.stats["writes"] += 1
        return self._update(fid, text, name)

    def trash(self, fid: str) -> None:
        self.assert_writable(fid)
        self.stats["writes"] += 1
        self._trash(fid)

    def export(self, fid: str, mime: str = "text/plain") -> str:
        self.stats["exports"] += 1
        return self._export(fid, mime)

    def create_folder(self, name: str, parent: str, props: dict) -> dict:
        meta = self._create_folder(name, parent, {**props, **TAG})
        self.writable.add(meta["id"])
        self.stats["writes"] += 1
        return meta

    def set_props(self, fid: str, props: dict, name: str | None = None) -> dict:
        """appProperties (a None value removes the key) and optionally the name of a doc gsync made. Never the text."""
        self.assert_writable(fid)
        self.stats["writes"] += 1
        return self._set_props(fid, props, name)

    def move(self, fid: str, add_parent: str, remove_parents: list[str], ancestors: list[str]) -> dict:
        """Move one of his docs into another folder: only its parents change. `ancestors` = every folder above it
        (for the legal folder rule). Raises WriteBlocked for a legal, foreign, trashed or non-Doc file."""
        meta = self.get(fid)
        if meta is None or meta.get("trashed"):
            raise WriteBlocked(f"not moving a missing or trashed doc: {fid}")
        if meta.get("mimeType") != DOC_MIME:
            raise WriteBlocked(f"gsync only moves Google Docs: {fid}")
        why = rules.legal_reason(meta, ancestors)
        if why:
            raise WriteBlocked(f"never touched (legal: {why.split(':')[0]}): {fid}")
        rm = [p for p in remove_parents if p != add_parent]
        self.stats["writes"] += 1
        return self._move(fid, add_parent, rm)

    def download(self, fid: str, dest) -> int:
        """Read-only: copy a file's bytes (an audio file he saved to Drive) to `dest`. Returns the byte count."""
        self.stats["downloads"] += 1
        return self._download(fid, dest)

    # ---- implemented by subclasses ----------------------------------------------------------------------------
    def start_token(self) -> str: ...
    def changes(self, token: str) -> tuple[list[dict], str]: ...
    def list_docs(self) -> list[dict]: ...
    def get(self, fid: str) -> dict | None: ...
    def find_by_prop(self, key: str, value: str) -> dict | None: ...
    def _export(self, fid: str, mime: str) -> str: ...
    def _create(self, name: str, text: str, parent: str, props: dict) -> dict: ...
    def _update(self, fid: str, text: str, name: str | None) -> dict: ...
    def _trash(self, fid: str) -> None: ...
    def _create_folder(self, name: str, parent: str, props: dict) -> dict: ...
    def _set_props(self, fid: str, props: dict, name: str | None) -> dict: ...
    def _move(self, fid: str, add_parent: str, remove_parents: list[str]) -> dict: ...
    def list_folder(self, folder_id: str) -> list[dict]: ...
    def _download(self, fid: str, dest) -> int: ...


class GoogleDrive(DriveBase):
    RETRY_STATUS = {429, 500, 502, 503, 504}
    RATE_REASONS = {"rateLimitExceeded", "userRateLimitExceeded"}

    def __init__(self, creds, export_gap_s: float = 0.2, sleep=time.sleep):
        super().__init__()
        from googleapiclient.discovery import build
        self.svc = build("drive", "v3", credentials=creds, cache_discovery=False)
        self.export_gap_s, self.sleep, self._last_export = export_gap_s, sleep, 0.0

    def _x(self, req, tries: int = 8):
        from googleapiclient.errors import HttpError
        for n in range(tries):
            self.stats["calls"] += 1
            try:
                return req.execute()
            except HttpError as e:
                st = int(getattr(e.resp, "status", 0) or 0)
                reason = ""
                try:
                    reason = e.error_details[0].get("reason", "") if e.error_details else ""
                except Exception:  # noqa: BLE001
                    pass
                if (st in self.RETRY_STATUS or (st == 403 and reason in self.RATE_REASONS)) and n < tries - 1:
                    self.stats["backoffs"] += 1
                    self.sleep(min(64.0, 2 ** n) + random.random())
                    continue
                raise

    def start_token(self) -> str:
        return self._x(self.svc.changes().getStartPageToken())["startPageToken"]

    def changes(self, token: str) -> tuple[list[dict], str]:
        out, tok = [], token
        while True:
            r = self._x(self.svc.changes().list(
                pageToken=tok, pageSize=1000, includeRemoved=True, restrictToMyDrive=True, spaces="drive",
                fields=f"nextPageToken,newStartPageToken,changes(fileId,removed,time,file({META_FIELDS}))"))
            out += r.get("changes", [])
            if r.get("nextPageToken"):
                tok = r["nextPageToken"]
                continue
            return out, r["newStartPageToken"]

    def list_docs(self) -> list[dict]:
        out, tok = [], None
        while True:
            r = self._x(self.svc.files().list(
                q=f"'me' in owners and mimeType='{DOC_MIME}' and trashed=false", pageSize=1000, pageToken=tok,
                spaces="drive", fields=f"nextPageToken,files({META_FIELDS})"))
            out += r.get("files", [])
            tok = r.get("nextPageToken")
            if not tok:
                return out

    def get(self, fid: str) -> dict | None:
        from googleapiclient.errors import HttpError
        try:
            return self._x(self.svc.files().get(fileId=fid, fields=META_FIELDS))
        except HttpError as e:
            if int(getattr(e.resp, "status", 0) or 0) == 404:
                return None
            raise

    def find_by_prop(self, key: str, value: str) -> dict | None:
        v = value.replace("\\", "\\\\").replace("'", "\\'")
        r = self._x(self.svc.files().list(
            q=f"appProperties has {{ key='{key}' and value='{v}' }} and trashed=false", pageSize=1, spaces="drive",
            fields=f"files({META_FIELDS})"))
        return (r.get("files") or [None])[0]

    def _export(self, fid: str, mime: str) -> str:
        gap = self.export_gap_s - (time.monotonic() - self._last_export)
        if gap > 0:
            self.sleep(gap)
        self._last_export = time.monotonic()
        data = self._x(self.svc.files().export(fileId=fid, mimeType=mime))
        return data.decode("utf-8") if isinstance(data, (bytes, bytearray)) else str(data)

    def _media(self, text: str):
        from googleapiclient.http import MediaIoBaseUpload
        return MediaIoBaseUpload(io.BytesIO(text.encode("utf-8")), mimetype="text/plain", resumable=False)

    def _create(self, name, text, parent, props):
        body = {"name": name, "mimeType": DOC_MIME, "appProperties": props}
        if parent:
            body["parents"] = [parent]
        return self._x(self.svc.files().create(body=body, media_body=self._media(text), fields=META_FIELDS))

    def _update(self, fid, text, name):
        body = {"name": name} if name else {}
        return self._x(self.svc.files().update(fileId=fid, body=body, media_body=self._media(text), fields=META_FIELDS))

    def _trash(self, fid):
        self._x(self.svc.files().update(fileId=fid, body={"trashed": True}, fields="id"))

    def _create_folder(self, name, parent, props):
        body = {"name": name, "mimeType": FOLDER_MIME, "appProperties": props, "parents": [parent]}
        return self._x(self.svc.files().create(body=body, fields=META_FIELDS))

    def _set_props(self, fid, props, name):
        body = {"appProperties": props, **({"name": name} if name else {})}
        return self._x(self.svc.files().update(fileId=fid, body=body, fields=META_FIELDS))

    def _move(self, fid, add_parent, remove_parents):
        # parents only: no body, no media, so the doc's text and name are untouched
        return self._x(self.svc.files().update(fileId=fid, addParents=add_parent,
                                               removeParents=",".join(remove_parents), fields=META_FIELDS))

    def list_folder(self, folder_id: str) -> list[dict]:
        """Everything directly inside one folder (not trashed), with size and md5Checksum. Read-only."""
        fid = folder_id.replace("\\", "\\\\").replace("'", "\\'")
        out, tok = [], None
        while True:
            r = self._x(self.svc.files().list(
                q=f"'{fid}' in parents and trashed=false", pageSize=1000, pageToken=tok, spaces="drive",
                fields=f"nextPageToken,files({MEDIA_FIELDS})"))
            out += r.get("files", [])
            tok = r.get("nextPageToken")
            if not tok:
                return out

    def _download(self, fid, dest) -> int:
        from googleapiclient.http import MediaIoBaseDownload
        req = self.svc.files().get_media(fileId=fid)
        with open(dest, "wb") as f:
            dl = MediaIoBaseDownload(f, req, chunksize=8 * 1024 * 1024)
            done = False
            while not done:
                self.stats["calls"] += 1
                _, done = dl.next_chunk(num_retries=5)
            return f.tell()
