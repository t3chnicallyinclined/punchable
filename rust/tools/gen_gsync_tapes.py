"""Record the Python gsync against fake Google, Keycloak and Punchable servers -> rust/crates/pk-gsync/tests/tapes/*.json.

The tapes are the reference the Rust port (pk-gsync) is held to: every HTTP request the Python made, in order, with the
answer it was given, plus the files it left behind after each CLI call. The Rust test (tests/differential.rs) replays
each tape: it starts from the same files, answers every request from the tape, fails on the first request that is not
the next recorded one (method, path, query, body, auth, range) and on any difference in the files left behind.

Everything is fake: tokens are invented strings, Doc ids and titles are made up, the servers listen on 127.0.0.1 and
nothing leaves the machine. Run it with the gsync venv (google-api-python-client, google-auth):

    ~/.venvs/gsync/Scripts/python.exe rust/tools/gen_gsync_tapes.py            # write the tapes
    ~/.venvs/gsync/Scripts/python.exe rust/tools/gen_gsync_tapes.py --check    # record again, fail when a tape differs

Not part of CI's goldens-current: the tapes are frozen at the Python's last commit (round 9 turns them into fixtures)."""
from __future__ import annotations

import argparse
import base64
import email
import hashlib
import itertools
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import urllib.parse
from datetime import datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
TAPES = REPO / "rust" / "crates" / "pk-gsync" / "tests" / "tapes"
RUNNER = Path(__file__).resolve().parent / "gsync_tape_runner.py"
ME = "owner@example.com"
ROOT = "root0"
OLD = "2026-10-20T10:00:00.000Z"
NOW = "2026-10-26T12:00:00+00:00"
NOW_TS = int(datetime.fromisoformat(NOW).timestamp())
DOC_MIME = "application/vnd.google-apps.document"
FOLDER_MIME = "application/vnd.google-apps.folder"
FOLDER_ID = "sample-folder-punchable-0000000000"
LEGAL_FOLDER = "sample-legal-folder-000000000000000"
DENYLISTED = "sample-denylisted-doc-0000000000000000000000"
BOARD_DOC = "sample-board-doc-robot-vacuum-000000000000"
AI_DOC = "sample-ai-draft-doc-0000000000000000000000"
MIXED_DOC = "sample-mixed-doc-00000000000000000000000000"
ADOPTED_1 = "sample-adopted-doc-1-000000000000000000000"
ADOPTED_2 = "sample-adopted-doc-2-000000000000000000000"
LEGAL_RULES = {"owner": ME, "legal_parents": [LEGAL_FOLDER], "keywords": ["sample person"],
               "content_markers": [], "legal_ids": [DENYLISTED]}
SECRETS = "PK_WORKER_SECRET=fake-worker-secret\n"
KC_REFRESH0 = "fake-kc-refresh-0"
G_REFRESH = "fake-google-refresh-0"


def slug(s):
    return re.sub(r"[^a-z0-9]+", "-", s.lower()).strip("-")[:60] or "x"


# ----------------------------------------------------------------------------------------------------- the fake worlds
class DriveWorld:
    """Google Drive v3 as a REST fake (the logic of gsync/tests/fakes.py FakeDrive, behind HTTP)."""

    def __init__(self):
        self.files = {ROOT: {"id": ROOT, "name": "My Drive", "mimeType": FOLDER_MIME, "parents": []}}
        self.log: list[str] = []
        self.ids = itertools.count(1)
        self.fail: list[tuple] = []     # (matcher, status, reason) one-shot failures, first match wins

    def folder(self, fid, parents=(ROOT,), name="folder"):
        self.files[fid] = {"id": fid, "name": name, "mimeType": FOLDER_MIME, "parents": list(parents)}
        return fid

    def user_create(self, name, text, fid=None, parents=(ROOT,), owner=ME, modified=OLD, props=None):
        fid = fid or f"doc{next(self.ids):04d}xxxxxxxx"
        self.files[fid] = {"id": fid, "name": name, "mimeType": DOC_MIME, "parents": list(parents),
                           "owners": [{"emailAddress": owner}], "createdTime": modified, "modifiedTime": modified,
                           "version": "1", "trashed": False, "appProperties": dict(props or {}), "_text": text}
        self.log.append(fid)
        return fid

    def user_edit(self, fid, text=None, name=None, modified="2026-10-26T09:00:00.000Z"):
        f = self.files[fid]
        if text is not None:
            f["_text"] = text
        if name is not None:
            f["name"] = name
        f["version"] = str(int(f["version"]) + 1)
        f["modifiedTime"] = modified
        self.log.append(fid)

    def user_trash(self, fid):
        self.files[fid]["trashed"] = True
        self.files[fid]["version"] = str(int(self.files[fid]["version"]) + 1)
        self.log.append(fid)

    def user_upload(self, name, data: bytes, parents, mime="audio/x-m4a", created=OLD, owner=ME, fid=None):
        fid = fid or f"aud{next(self.ids):04d}xxxxxxxx"
        self.files[fid] = {"id": fid, "name": name, "mimeType": mime, "parents": list(parents),
                           "owners": [{"emailAddress": owner}], "createdTime": created, "modifiedTime": created,
                           "version": "1", "trashed": False, "appProperties": {}, "size": str(len(data)),
                           "md5Checksum": hashlib.md5(data).hexdigest(), "_bytes": data}
        self.log.append(fid)
        return fid

    def text(self, fid):
        return self.files[fid]["_text"]

    def meta(self, fid):
        return {k: v for k, v in self.files[fid].items() if not k.startswith("_")}

    # ---- REST ----
    def handle(self, method, path, q, body, headers):
        """-> (status, content_type, payload bytes, extra headers)"""
        for i, (match, status, msg, *rest) in enumerate(self.fail):
            if match(method, path, q):
                del self.fail[i]
                return self.err(status, msg, rest[0] if rest else None)
        p = path
        if p == "/drive/v3/changes/startPageToken":
            return self.ok({"startPageToken": str(len(self.log))})
        if p == "/drive/v3/changes" and method == "GET":
            n = int(q["pageToken"][0])
            out = [{"fileId": fid, "removed": fid not in self.files, "time": "2026-10-26T09:00:00.000Z",
                    "file": self.meta(fid) if fid in self.files else None} for fid in self.log[n:]]
            return self.ok({"changes": out, "newStartPageToken": str(len(self.log))})
        if p == "/drive/v3/files" and method == "GET":
            return self.list(q)
        if p == "/drive/v3/files" and method == "POST":
            meta = json.loads(body)
            return self.ok(self.create(meta, None))
        if p == "/upload/drive/v3/files" and method == "POST":
            meta, media = self.multipart(body, headers)
            return self.ok(self.create(meta, media))
        m = re.fullmatch(r"(/upload)?/drive/v3/files/([^/]+)(/export)?", p)
        if not m:
            return self.err(404, "no route " + p)
        fid = urllib.parse.unquote(m.group(2))
        if m.group(3):
            if fid not in self.files:
                return self.err(404, f"File not found: {fid}.", "notFound")
            self.exported = getattr(self, "exported", []) + [fid]
            mime = q["mimeType"][0]
            t = self.files[fid]["_text"]
            if mime == "text/plain":
                t = "﻿" + t.replace("\n", "\r\n")
            else:
                t = "﻿" + t.replace("\n", "\r\n")
            return 200, mime, t.encode("utf-8"), {}
        if method == "GET":
            if fid not in self.files:
                return self.err(404, f"File not found: {fid}.", "notFound")
            if q.get("alt") == ["media"]:
                data = self.files[fid]["_bytes"]
                rng = headers.get("range")
                if rng:
                    a, b = re.fullmatch(r"bytes=(\d+)-(\d+)", rng).groups()
                    a, b = int(a), min(int(b), len(data) - 1)
                    return 206, "audio/x-m4a", data[a:b + 1], {"Content-Range": f"bytes {a}-{b}/{len(data)}"}
                return 200, "audio/x-m4a", data, {}
            return self.ok(self.meta(fid))
        if method == "PATCH":
            if fid not in self.files:
                return self.err(404, f"File not found: {fid}.", "notFound")
            f = self.files[fid]
            if m.group(1):
                meta, media = self.multipart(body, headers)
                if media is not None:
                    f["_text"] = media
                self.apply_meta(f, meta)
            else:
                self.apply_meta(f, json.loads(body) if body else {})
                if q.get("addParents"):
                    rm = [x for x in q.get("removeParents", [""])[0].split(",") if x]
                    ps = [x for x in f["parents"] if x not in rm]
                    if q["addParents"][0] not in ps:
                        ps.append(q["addParents"][0])
                    f["parents"] = ps
            f["version"] = str(int(f["version"]) + 1)
            self.log.append(fid)
            return self.ok(self.meta(fid))
        return self.err(405, "method")

    def apply_meta(self, f, meta):
        if meta.get("name"):
            f["name"] = meta["name"]
        if "trashed" in meta:
            f["trashed"] = meta["trashed"]
        if "appProperties" in meta:
            ap = dict(f.get("appProperties") or {})
            for k, v in meta["appProperties"].items():
                if v is None:
                    ap.pop(k, None)
                else:
                    ap[k] = v
            f["appProperties"] = ap

    def multipart(self, body, headers):
        ct = headers["content-type"]
        msg = email.message_from_bytes(b"Content-Type: " + ct.encode() + b"\r\nMIME-Version: 1.0\r\n\r\n" + body)
        parts = msg.get_payload()
        meta = json.loads(parts[0].get_payload(decode=True).decode("utf-8"))
        media = parts[1].get_payload(decode=True).decode("utf-8") if len(parts) > 1 else None
        return meta, media

    def create(self, meta, media):
        is_folder = meta.get("mimeType") == FOLDER_MIME
        fid = f"{'fold' if is_folder else 'new'}{next(self.ids):04d}xxxxxxxx"
        f = {"id": fid, "name": meta["name"], "mimeType": meta["mimeType"], "parents": meta.get("parents") or [ROOT],
             "owners": [{"emailAddress": ME}], "createdTime": OLD, "version": "1", "trashed": False,
             "appProperties": dict(meta.get("appProperties") or {})}
        if not is_folder:
            f["modifiedTime"] = OLD
            f["_text"] = media or ""
        self.files[fid] = f
        self.log.append(fid)
        return self.meta(fid)

    def list(self, q):
        qs = q["q"][0]
        if "mimeType='" + DOC_MIME + "'" in qs and "'me' in owners" in qs:
            files = [self.meta(k) for k, f in self.files.items() if f["mimeType"] == DOC_MIME and not f.get("trashed")
                     and (f.get("owners") or [{}])[0].get("emailAddress") == ME]
        elif qs.startswith("appProperties has"):
            m = re.fullmatch(r"appProperties has \{ key='(.*)' and value='(.*)' \} and trashed=false", qs)
            k, v = m.group(1), m.group(2).replace("\\'", "'").replace("\\\\", "\\")
            files = [self.meta(i) for i, f in self.files.items() if (f.get("appProperties") or {}).get(k) == v
                     and not f.get("trashed")][:1]
        else:
            m = re.fullmatch(r"'(.*)' in parents and trashed=false", qs)
            fid = m.group(1).replace("\\'", "'").replace("\\\\", "\\")
            files = [self.meta(i) for i, f in self.files.items() if fid in (f.get("parents") or [])
                     and not f.get("trashed")]
        return self.ok({"files": files})

    def ok(self, obj):
        return 200, "application/json", json.dumps(obj).encode("utf-8"), {}

    def err(self, status, msg, reason=None):
        e = {"code": status, "message": msg}
        if reason:
            e["errors"] = [{"message": msg, "domain": "global", "reason": reason}]
        return status, "application/json", json.dumps({"error": e}).encode("utf-8"), {}


class PkWorld:
    """The hosted API's worker routes (the logic of gsync/tests/fakes.py FakePunchable, behind HTTP)."""

    def __init__(self):
        self.lines: list[dict] = []
        self.jokes: dict[str, dict] = {}
        self.stores: dict[str, dict] = {"storyboards": {}, "merges": {}}
        self.graph_docs: dict = {}
        self.files: dict[str, str] = {}
        self.page = 2
        self.down = False
        self.fail: list[tuple] = []

    def line(self, **d):
        self.lines.append({"ts": "t", "by": "owner", **d})
        return len(self.lines) - 1

    def app_idea(self, title, text):
        jid = "joke:" + slug(title)
        self.jokes[jid] = {"id": jid, "title": title, "summary": text, "stage": "idea", "lead_text": None, "hidden": False}
        self.line(target="node", id=jid, action="add", after={"type": "joke", "title": title, "summary": text},
                  reason="saved from review")
        return jid

    def app_edit(self, jid, text):
        self.jokes[jid]["summary"] = text
        self.line(target="node", id=jid, action="edit", after={"summary": text}, reason="")

    def handle(self, method, path, q, body):
        for i, (match, status, msg) in enumerate(self.fail):
            if match(method, path, q):
                del self.fail[i]
                return status, {"error": msg}
        if path == "/api/health":
            return (503, {"error": "down"}) if self.down else (200, {"ok": True})
        if path == "/api/worker/ideas" and method == "POST":
            text, title = body["text"], body.get("title") or ""
            t = title or " ".join(text.split()[:7])
            jid, k = "joke:" + slug(t), 2
            while jid in self.jokes:
                jid, k = f"joke:{slug(t)}-{k}", k + 1
            self.jokes[jid] = {"id": jid, "title": t, "summary": text.strip(), "stage": "idea", "lead_text": None,
                               "hidden": False}
            i = self.line(target="node", id=jid, action="add", after={"type": "joke", "title": t, "summary": text},
                          reason=body["reason"], by=body["actor"])
            return 200, {"ok": True, "id": jid, "indexes": [i], "attach": None}
        if path == "/api/worker/decision" and method == "POST":
            if body["action"] == "add":
                a = body["after"]
                jid = "joke:" + slug(a["title"])
                self.jokes[jid] = {"id": jid, "title": a["title"], "summary": a["summary"], "stage": a["stage"],
                                   "lead_text": None, "hidden": False}
                i = self.line(target="node", id=jid, action="add", after=a, reason=body["reason"], by=body["actor"])
                return 200, {"ok": True, "indexes": [i], "decisions": [{"id": jid}]}
            j = self.jokes.get(body["id"])
            if j is None:
                return 404, {"error": "no such node"}
            ch = {k: (v.strip() if isinstance(v, str) else v) for k, v in body["after"].items() if j.get(k) != v}
            if not ch:
                return 400, {"error": "nothing changed"}
            before = {k: j.get(k) for k in ch}
            j.update(ch)
            i = self.line(target="node", id=body["id"], action="edit", before=before, after=ch, reason=body["reason"],
                          by=body["actor"])
            return 200, {"ok": True, "indexes": [i], "decisions": [{"id": body["id"]}]}
        if path in ("/api/worker/undo", "/api/undo") and method == "POST":
            for i in body["indexes"]:
                if i >= len(self.lines):
                    return 400, {"error": f"no decision line {i}"}
                d = self.lines[i]
                if d["action"] == "edit":
                    self.jokes[d["id"]].update(d["before"])
                self.line(target="node", id=d["id"], action="undo", undo_of=i, by=body["actor"])
            return 200, {"ok": True}
        m = re.fullmatch(r"/api/worker/jokes/(.*)", path)
        if m:
            jid = urllib.parse.unquote(m.group(1))
            j = self.jokes.get(jid)
            if j is None:
                return 404, {"error": "no such joke"}
            lead = j.get("lead_text")
            return 200, {"joke": {"title": j["title"], "summary": j["summary"], "stage": j["stage"],
                                  "hidden": j["hidden"], "lead_unit": "u1" if lead else None},
                         "versions": [{"units": [{"id": "u1", "text": lead}]}] if lead else [], "yours": True}
        if path == "/api/worker/decisions":
            a = int(q["after"][0])
            chunk = self.lines[a:a + self.page]
            return 200, {"total": len(self.lines), "lines": [{"decision": d} for d in chunk]}
        if path == "/api/worker/creative":
            return 200, {"items": self.stores[q["store"][0]]}
        if path == "/api/worker/files" and method == "GET":
            pre = q["prefix"][0]
            return 200, {"files": [{"path": p, "sha256": hashlib.sha256(t.encode()).hexdigest()}
                                   for p, t in sorted(self.files.items()) if p.startswith(pre)]}
        if path == "/api/worker/files/content":
            p = q["path"][0]
            if p not in self.files:
                return 404, {"error": "no such file"}
            return 200, {"path": p, "text": self.files[p]}
        if path == "/api/worker/files" and method == "POST":
            for f in body["files"]:
                self.files[f["path"]] = f["text"]
            return 200, {"written": [f["path"] for f in body["files"]]}
        return 404, {"error": "no route " + path}


class KcWorld:
    def __init__(self):
        self.refresh = KC_REFRESH0
        self.n = 0
        self.g = 0
        self.kc_down = False
        self.broker_status = 200
        self.google_dead: set[str] = set()
        self.broker_rt = "fake-google-refresh-broker"
        self.device_polls = 0

    def handle(self, method, path, form):
        if path == "/kc/realms/punchable/protocol/openid-connect/token":
            if form.get("grant_type") == "refresh_token":
                if self.kc_down or form.get("refresh_token") != self.refresh:
                    return 400, {"error": "invalid_grant"}
                self.n += 1
                self.refresh = f"fake-kc-refresh-{self.n}"
                return 200, {"access_token": f"fake-kc-access-{self.n}", "refresh_token": self.refresh, "expires_in": 300}
            if form.get("grant_type", "").endswith("device_code"):
                self.device_polls += 1
                if self.device_polls < 2:
                    return 400, {"error": "authorization_pending"}
                self.refresh = "fake-kc-refresh-device"
                return 200, {"access_token": "fake-kc-access-device", "refresh_token": self.refresh, "expires_in": 300}
        if path == "/kc/realms/punchable/protocol/openid-connect/auth/device":
            return 200, {"device_code": "fake-device-code", "user_code": "ABCD-EFGH", "interval": 1, "expires_in": 600,
                         "verification_uri": "https://auth.example.com/device",
                         "verification_uri_complete": "https://auth.example.com/device?user_code=ABCD-EFGH"}
        if path == "/kc/realms/punchable/broker/google/token":
            if self.broker_status != 200:
                return self.broker_status, {}
            return 200, {"access_token": "fake-broker-access", "refresh_token": self.broker_rt, "expires_in": 3599}
        if path == "/google/token":
            if form.get("refresh_token") in self.google_dead:
                return 400, {"error": "invalid_grant", "error_description": "Token has been expired or revoked."}
            self.g += 1
            return 200, {"access_token": f"fake-g-access-{self.g}", "expires_in": 3599, "token_type": "Bearer"}
        if path == "/wtoken":
            return 200, {"access_token": "fake-worker-access", "expires_in": 300}
        return 404, {"error": "no route"}


class World:
    def __init__(self):
        self.drive, self.pk, self.kc = DriveWorld(), PkWorld(), KcWorld()
        self.ex: list[dict] = []
        self.lock = threading.Lock()
        self.base = ""

    def start(self):
        world = self

        class H(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *a):
                pass

            def do(self):
                u = urllib.parse.urlsplit(self.path)
                n = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(n) if n else b""
                hdr = {k.lower(): v for k, v in self.headers.items()}
                with world.lock:
                    status, ctype, payload, extra = world.route(self.command, u.path, urllib.parse.parse_qs(
                        u.query, keep_blank_values=True), body, hdr)
                self.send_response(status)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(payload)))
                for k, v in extra.items():
                    self.send_header(k, v)
                self.end_headers()
                self.wfile.write(payload)

            do_GET = do_POST = do_PATCH = do_PUT = do_DELETE = do

        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.base = f"http://127.0.0.1:{self.srv.server_address[1]}"
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()

    def route(self, method, path, q, body, hdr):
        ctype = (hdr.get("content-type") or "").split(";")[0].strip()
        svc, parsed = "pk", None
        if path.startswith(("/drive/", "/upload/drive/")):
            svc = "drive"
        elif path.startswith(("/kc/", "/google/", "/wtoken")):
            svc = "auth"
        elif path == "/_spawn":
            svc = "spawn"
        if ctype == "application/json" and body:
            parsed = json.loads(body)
        elif ctype == "application/x-www-form-urlencoded":
            parsed = {k: v[0] for k, v in urllib.parse.parse_qs(body.decode()).items()}
        elif ctype == "multipart/related":
            parsed = {}
            meta, media = self.drive.multipart(body, hdr)
            parsed = {"meta": meta, "media": media}
        rec = {"svc": svc, "method": method, "path": path, "query": {k: v for k, v in sorted(q.items())},
               "auth": hdr.get("authorization"), "ctype": ctype or None, "range": hdr.get("range"), "body": parsed}
        if svc == "drive":
            status, ct, payload, extra = self.drive.handle(method, path, q, body, hdr)
        elif svc == "auth":
            status, obj = self.kc.handle(method, path, parsed or {})
            ct, payload, extra = "application/json", json.dumps(obj).encode(), {}
        elif svc == "spawn":
            status, ct, payload, extra = 200, "application/json", b"{}", {}
        else:
            status, obj = self.pk.handle(method, path, q, parsed)
            ct, payload, extra = "application/json", json.dumps(obj).encode(), {}
        resp = {"status": status, "ctype": ct, "extra": extra}
        if ct == "application/json" and svc != "drive" or (svc == "drive" and ct == "application/json"):
            resp["json"] = json.loads(payload)
        elif ct.startswith(("text/", "application/")) and not ct.startswith("audio"):
            resp["text"] = payload.decode("utf-8")
        else:
            resp["b64"] = base64.b64encode(payload).decode()
        rec["resp"] = resp
        self.ex.append(rec)
        return status, ct, payload, extra


# ------------------------------------------------------------------------------------------------------------ scenarios
class Scenario:
    def __init__(self, name, world, env=None, files=None):
        self.name, self.w = name, world
        self.env = env or {}
        self.files = {}
        self.steps = []
        self.before: dict = {"write": {}, "delete": []}
        self.setup_files(files or {})

    def setup_files(self, extra):
        self.files = {"home/legal_rules.json": json.dumps(LEGAL_RULES), "home/secrets.env": SECRETS,
                      "home/google-oauth.env": "GOOGLE_CLIENT_ID=fake-client-id\nGOOGLE_CLIENT_SECRET=fake-client-secret\n",
                      "home/keycloak_gsync_token.json": json.dumps({"refresh_token": KC_REFRESH0, "issuer": "x",
                                                                     "client_id": "pk-gsync", "saved_at": 1}),
                      "home/google_token.json": json.dumps({"refresh_token": G_REFRESH, "source": "keycloak-broker",
                                                            "kc_checked_at": NOW_TS - 100}),
                      "ws/comedy/corpus/gdocs/.keep": ""}
        self.files.update(extra)

    def write(self, root: Path, rel: str, text: str):
        """Change a file between two CLI calls; the tape records it so the Rust starts the next call from the same files."""
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8", newline="\n")
        self.before["write"][rel] = text.replace(self.w.base, "{BASE}").replace("docs.google.com", "{GDOCS_HOST}")

    def delete(self, root: Path, rel: str):
        (root / rel).unlink()
        self.before["delete"].append(rel)

    def snapshot(self, root: Path) -> dict:
        out = {}
        for p in sorted(root.rglob("*")):
            if not p.is_file() or p.name.endswith((".lock", ".tmp", ".part")):
                continue
            rel = p.relative_to(root).as_posix()
            data = p.read_bytes()
            try:
                out[rel] = re.sub(r'"expiry": "[^"]*"', '"expiry": "<expiry>"', data.decode("utf-8").replace("\r\n", "\n"))
            except UnicodeDecodeError:
                out[rel] = f"bin:{len(data)}:{hashlib.sha256(data).hexdigest()[:16]}"
        return out

    def sub(self, s: str, root: Path) -> str:
        # docs.google.com stays out of the committed tapes (the commit guard refuses Doc URLs, fake ids or not)
        return (s.replace(self.w.base, "{BASE}").replace(str(root).replace("\\", "/"), "{ROOT}").replace(str(root), "{ROOT}")
                .replace("docs.google.com", "{GDOCS_HOST}"))

    def run(self, root: Path, argv, now=NOW, free_mb=50000, note=None, env_extra=None):
        base = self.w.base
        env = {k: os.environ[k] for k in ("PATH", "SYSTEMROOT", "USERNAME", "TEMP", "TMP", "USERPROFILE", "HOME",
                                          "COMSPEC", "WINDIR") if k in os.environ}
        env.update({"PYTHONUTF8": "1", "PYTHONDONTWRITEBYTECODE": "1", "NO_PROXY": "127.0.0.1", "no_proxy": "127.0.0.1",
                    "PUNCHABLE_HOME": str(root / "home"), "PK_ONLINE_FIRST": "1", "PK_WORKSPACE": str(root / "ws"),
                    "PUNCH_SECRETS": str(root / "home" / "secrets.env"), "PK_API": base, "PK_TOKEN_URL": base + "/wtoken",
                    "PK_AUTH_URL": base + "/kc/realms/punchable", "PK_GSYNC_MEMOS": "1",
                    "PUNCHABLE_API": base})
        env.update({k: v.replace("{BASE}", base).replace("{ROOT}", str(root)) for k, v in self.env.items()})
        env.update(env_extra or {})
        env["GSYNC_TAPE"] = json.dumps({"base": base, "now": now, "argv": argv, "free_mb": free_mb})
        n0 = len(self.w.ex)
        pyexe = os.environ.get("GSYNC_PY") or sys.executable
        r = subprocess.run([pyexe, str(RUNNER)], env=env, cwd=str(REPO), capture_output=True, text=True, encoding="utf-8",
                           timeout=300)
        ex = json.loads(self.sub(json.dumps(self.w.ex[n0:]), root))
        step = {"argv": argv, "now": now, "free_mb": free_mb, "before": self.before, "exchanges": ex, "exit": r.returncode,
                "stdout": self.sub(r.stdout.replace("\r\n", "\n"), root).replace("\\", "/"),
                "after": json.loads(self.sub(json.dumps(self.snapshot(root)), root))}
        if note:
            step["note"] = note
        if r.returncode not in (0, 2) and "Traceback" in r.stderr:
            sys.stderr.write(r.stderr[-1500:])
        step["stderr_has_traceback"] = "Traceback" in r.stderr
        self.before = {"write": {}, "delete": []}
        self.steps.append(step)
        return step


def build_root(sc: Scenario) -> Path:
    root = Path(tempfile.mkdtemp(prefix="gsync-tape-"))
    for rel, text in sc.files.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text.replace("{BASE}", sc.w.base), encoding="utf-8", newline="\n")
    return root


def stable_hashes(steps: list) -> None:
    """A posted file whose text names the fake server (a run's error with its URL) was hashed with this recording's
    random port, so its hash would differ on every run. Hash it with port 0 instead, in the post and in every later
    listing of the stored files. The Rust replay hashes such texts again with its own port (tests/differential.rs
    `replace_base`), so the recorded value only has to be the same each time."""
    named: dict[str, str] = {}
    for step in steps:
        for ex in step["exchanges"]:
            if ex["path"] != "/api/worker/files":
                continue
            if ex["method"] == "POST":
                for f in (ex["body"] or {}).get("files", []):
                    if "{BASE}" in f["text"]:
                        named[f["path"]] = f["sha256"] = hashlib.sha256(
                            f["text"].replace("{BASE}", "http://127.0.0.1:0").encode("utf-8")).hexdigest()
                    else:
                        named.pop(f["path"], None)
            else:
                for f in ex["resp"].get("json", {}).get("files", []):
                    if f["path"] in named:
                        f["sha256"] = named[f["path"]]


def finish(sc: Scenario, root: Path, initial: dict) -> dict:
    stable_hashes(sc.steps)
    return {"name": sc.name, "env": {k: v for k, v in sc.env.items()}, "files": initial, "steps": sc.steps}


def scenarios():
    yield bootstrap_and_pull
    yield push_and_boards
    yield merges
    yield voice_memos
    yield failures
    yield auth_chain
    yield auth_device_and_commands
    yield hosted_state
    yield dry_run
    yield adopted_docs
    yield narrow_scopes
    yield bootstrap_error


# ---------------------------------------------------------------------------------------------------------------------
def with_folder(w: World):
    w.drive.folder(FOLDER_ID, name="Punchable")
    w.drive.folder(LEGAL_FOLDER, name="Legal stuff")


def bootstrap_and_pull(w: World):
    sc = Scenario("bootstrap_and_pull", w, files={
        "ws/comedy/corpus/gdocs/2026-09-01_old-bit__corpus01.md":
            '---\nsource: gdoc\ndoc_id: corpus-doc-0001\ntitle: "Old bit"\nmodified: 2026-09-01T10:00:00.000Z\n---\nold text\n'})
    d, pk = w.drive, w.pk
    with_folder(w)
    # in the Punchable folder: comedy, an untitled empty doc, legal by title / folder / content / owner / denylist, not comedy
    a = d.user_create("Airport security bit", "Taking off my belt\n\nand my dignity", fid="doc-airport-0000000000", parents=(FOLDER_ID,))
    d.user_create("Untitled document", "", fid="doc-untitled-000000000", parents=(FOLDER_ID,))
    d.user_create("Petition draft v2", "should never be read", fid="doc-petition-000000000", parents=(FOLDER_ID,))
    d.user_create("Fine title", "words", fid="doc-in-legal-folder-0000", parents=(LEGAL_FOLDER,))
    d.user_create("Quiet notes", "In the Supreme Court of the State of nowhere, the petitioner said hello.",
                  fid="doc-content-legal-0000", parents=(FOLDER_ID,))
    d.user_create("Somebody else's doc", "theirs", fid="doc-foreign-0000000000", owner="other@example.com")
    d.user_create("Denylisted", "x", fid=DENYLISTED)
    d.user_create("My resume", "jobs", fid="doc-resume-00000000000", parents=(FOLDER_ID,))
    d.user_create("Zebra crossing", "stripes # not a heading\n# Heading two\nmore\n## three\nlast",
                  fid="doc-zebra-0000000000000", parents=(FOLDER_ID,))
    d.user_create("Fresh edit", "typed a second ago", fid="doc-fresh-000000000000", modified="2026-10-26T11:59:30.000Z")
    d.user_create("Sample person story", "a story", fid="doc-kw-0000000000000", parents=(FOLDER_ID,))
    # a doc the corpus ingest already holds, unchanged since: its version is remembered, nothing exported
    d.user_create("Old bit", "old text", fid="corpus-doc-0001", parents=(FOLDER_ID,), modified="2026-09-01T10:00:00.000Z")
    root = build_root(sc)
    initial = dict(sc.files)
    sc.run(root, ["run"], note="bootstrap")
    sc.run(root, ["run"], note="nothing changed")
    # the comic edits one doc, creates one, and edits the app idea behind the first
    d.user_edit(a, text="Taking off my belt\n\nand my dignity\n\nand my shoes")
    d.user_create("Late night thought", "Why is it called a building if it is already built", fid="doc-late-000000000000", parents=(FOLDER_ID,))
    sc.run(root, ["run"], note="edit + new")
    jid = json.loads((root / "ws/comedy/kb/gsync/map.json").read_text())["docs"][a]["joke_id"]
    # both edited: the app text changes, then the doc changes
    pk.app_edit(jid, "app wording of the belt bit")
    d.user_edit(a, text="doc wording of the belt bit", name="Airport security bit (v2)", modified="2026-10-26T10:00:00.000Z")
    sc.run(root, ["run"], note="both edited -> conflict")
    d.user_trash(a)
    sc.run(root, ["run"], note="trashed -> retired")
    # a long doc becomes a node add, a doc that only just changed is parked, a settled one is picked up
    d.user_create("Long one", "word " * 900, fid="doc-long-0000000000000", parents=(FOLDER_ID,))
    d.user_edit("doc-fresh-000000000000", text="typed a bit more", modified="2026-10-26T11:59:40.000Z")
    sc.run(root, ["run"], note="long text + settling")
    sc.run(root, ["run"], now="2026-10-26T12:10:00+00:00", note="settled")
    # an edit to a corpus doc is kept as a mirror file and an edit record, never an idea
    d.user_edit("corpus-doc-0001", text="old text, edited", modified="2026-10-26T09:00:00.000Z")
    sc.run(root, ["run"], now="2026-10-26T12:20:00+00:00", note="corpus doc edited")
    return finish(sc, root, initial)


def push_and_boards(w: World):
    sc = Scenario("push_and_boards", w)
    d, pk = w.drive, w.pk
    with_folder(w)
    d.user_create("A made-up doc", "hello", fid="doc-plain-00000000000", parents=(FOLDER_ID,))
    root = build_root(sc)
    initial = dict(sc.files)
    sc.run(root, ["run"], note="bootstrap")
    j1 = pk.app_idea("Parking app", "I want a parking app\nand park badly")
    j2 = pk.app_idea("Elevator small talk", "Nobody talks in elevators")
    pk.jokes[j2]["lead_text"] = "lead words of the elevator bit"
    sc.run(root, ["run"], note="app ideas -> new docs")
    pk.app_edit(j1, "I want a parking app\nand park worse")
    sc.run(root, ["run"], note="app edit -> push_edit")
    d.user_edit(next(k for k, f in d.files.items() if f.get("appProperties", {}).get("punchable_joke") == j1),
                text="the doc was edited too", modified="2026-10-26T10:00:00.000Z")
    pk.app_edit(j1, "app edited again")
    sc.run(root, ["run"], note="doc changed since sync -> conflict, not overwritten")
    pk.stores["storyboards"]["sb:set-one"] = {"id": "sb:set-one", "title": "Set one", "by": "owner",
                                              "joke_ids": [j1, j2], "target_s": 425, "updated_at": "2026-10-25T00:00:00+00:00",
                                              "notes": "my notes"}
    pk.stores["storyboards"]["sb:ai-seed"] = {"id": "sb:ai-seed", "title": "AI seed", "by": "ai", "joke_ids": [j1]}
    sc.run(root, ["run"], note="board doc created")
    pk.stores["storyboards"]["sb:set-one"]["joke_ids"] = [j2, j1]
    sc.run(root, ["run"], note="board regenerated")
    pk.stores["storyboards"]["sb:set-one"]["deleted"] = True
    sc.run(root, ["run"], note="board deleted -> trash")
    # he removed the joke in the app, then edits its doc: the doc is detached, no idea, no edit
    plain = next(k for k, f in d.files.items() if f["name"] == "A made-up doc")
    pk.app_idea("Placeholder", "p")
    jid = json.loads((root / "ws/comedy/kb/gsync/map.json").read_text(encoding="utf-8"))["docs"][plain]["joke_id"]
    del pk.jokes[jid]
    d.user_edit(plain, text="edited after the joke was removed", modified="2026-10-26T09:00:00.000Z")
    sc.run(root, ["run"], note="joke removed in the app, doc edited: detached")
    return finish(sc, root, initial)


def merges(w: World):
    sc = Scenario("merges", w, files={
        "ws/comedy/kb/gsync/map.json": json.dumps({"version": 1, "changes_token": "0", "bootstrapped_at": "2026-10-25T00:00:00+00:00",
                                                   "folder_id": FOLDER_ID, "docs": {}, "boards": {}, "excluded": {},
                                                   "skipped": {}, "pending": {}, "folders": {}})})
    d, pk = w.drive, w.pk
    with_folder(w)
    d1 = d.user_create("Belt bit take one", "belt one", fid="doc-belt-1-0000000000", parents=(FOLDER_ID,))
    d2 = d.user_create("Belt bit take two", "belt two", fid="doc-belt-2-0000000000", parents=(FOLDER_ID,))
    d3 = d.user_create("Shared doc", "belt and a different one", fid="doc-shared-000000000", parents=(FOLDER_ID,))
    d4 = d.user_create("Gone doc", "x", fid="doc-gone-00000000000", parents=(FOLDER_ID,))
    d.user_trash(d4)
    d5 = d.user_create("Petition notes", "x", fid="doc-legal-merge-00000", parents=(FOLDER_ID,))
    root = build_root(sc)
    # doc 3 also holds a joke outside the merge
    pk.graph_docs = {}
    pk.files["kb/v1/graph.json"] = json.dumps({"nodes": [{"id": "u1", "type": "unit", "doc_id": d1}, {"id": "u2", "type": "unit", "doc_id": d2},
                                                          {"id": "u3", "type": "unit", "doc_id": d3}, {"id": "u4", "type": "unit", "doc_id": d3}],
                                               "edges": [{"kind": "part_of", "src": "u1", "dst": "joke:belt"},
                                                         {"kind": "part_of", "src": "u2", "dst": "joke:belt-two"},
                                                         {"kind": "part_of", "src": "u3", "dst": "joke:belt"},
                                                         {"kind": "part_of", "src": "u4", "dst": "joke:other"}]})
    for j, t in (("joke:belt", "Belt"), ("joke:belt-two", "Belt two"), ("joke:other", "Other")):
        pk.jokes[j] = {"id": j, "title": t, "summary": "s", "stage": "idea", "lead_text": None, "hidden": False}
    initial = dict(sc.files)
    pk.stores["merges"]["m1"] = {"kind": "merge", "status": "merged", "main": "joke:belt", "others": ["joke:belt-two"],
                                 "created_at": "2026-10-25T08:00:00+00:00",
                                 "snapshot": {"joke:belt": {"title": "Belt", "units": [{"doc_id": d1, "doc_title": "Belt bit take one", "date": "2026-09-01", "text": "belt one"},
                                                                                       {"doc_id": d3, "doc_title": "Shared doc", "date": "2026-09-02", "text": "belt and"},
                                                                                       {"doc_id": d4, "doc_title": "Gone doc", "date": None, "text": "g"},
                                                                                       {"doc_id": d5, "doc_title": "Petition notes", "date": None, "text": "p"}],
                                              "summary": "typed words", "summary_is_his": True, "added_at": "2026-09-03"},
                                              "joke:belt-two": {"title": "Belt two", "units": [{"doc_id": d2, "doc_title": "Belt bit take two", "date": "2026-09-05", "text": "belt two"}]}}}
    sc.run(root, ["run"], note="merge applied")
    sc.run(root, ["run"], note="merge idempotent")
    # the joke behind doc 2 is gone from the API (merged away): his later edit of that doc is kept, not turned into an idea
    d2_entry = json.loads((root / "ws/comedy/kb/gsync/map.json").read_text(encoding="utf-8"))["docs"][d2]
    del pk.jokes["joke:belt-two"]
    d.user_edit(d2, text="belt two, edited later", modified="2026-10-26T09:00:00.000Z")
    sc.write(root, "ws/comedy/kb/gsync/map.json", json.dumps({**json.loads((root / "ws/comedy/kb/gsync/map.json").read_text(encoding="utf-8")),
             "docs": {**json.loads((root / "ws/comedy/kb/gsync/map.json").read_text(encoding="utf-8"))["docs"],
                      d2: {**d2_entry, "origin": "gdocs", "joke_id": "joke:belt-two", "version": "2", "doc_hash": "0000000000000000"}}},
             indent=1, sort_keys=True))
    sc.run(root, ["run"], note="a merged-away joke's doc edited: kept verbatim")
    pk.jokes["joke:belt-two"] = {"id": "joke:belt-two", "title": "Belt two", "summary": "s", "stage": "idea", "lead_text": None, "hidden": False}
    pk.stores["merges"]["m1"]["status"] = "undone"
    sc.run(root, ["run"], note="merge undone")
    pk.stores["merges"]["m2"] = {"kind": "merge", "status": "undone", "main": "joke:other", "others": []}
    sc.run(root, ["run"], note="undone before applied")
    return finish(sc, root, initial)


def voice_memos(w: World):
    sc = Scenario("voice_memos", w)
    d, pk = w.drive, w.pk
    with_folder(w)
    mf = d.folder("memos-folder-0000000000", parents=(FOLDER_ID,), name="Voice memos")
    sub = d.folder("memos-2019-00000000000", parents=(mf,), name="2019")
    d.files[mf]["appProperties"] = {}
    a = d.user_upload("New Recording 7.m4a", b"AAAA" * 100, [mf], created="2026-10-01T10:00:00.000Z", fid="aud-a-0000000000")
    d.user_upload("airport tag.m4a", b"BBBB" * 100, [mf], created="2026-10-02T10:00:00.000Z", fid="aud-b-0000000000")
    d.user_upload("copy of airport tag.m4a", b"BBBB" * 100, [mf], created="2026-10-03T10:00:00.000Z", fid="aud-c-0000000000")
    d.user_upload("Sample person call.m4a", b"CCCC" * 100, [mf], created="2026-10-04T10:00:00.000Z", fid="aud-d-0000000000")
    d.user_upload("old one.mp3", b"DDDD" * 100, [sub], mime="audio/mpeg", created="2019-03-14T21:03:11.000Z", fid="aud-e-0000000000")
    d.user_upload("notes.txt", b"not audio", [mf], mime="text/plain", fid="txt-f-0000000000")
    root = build_root(sc)
    initial = dict(sc.files)
    sc.run(root, ["run", "--dry-run"], note="dry run lists, downloads nothing")
    sc.run(root, ["run"], note="bootstrap + list + download + spawn")
    sc.run(root, ["run"], note="nothing new, already downloaded")
    sc.run(root, ["run"], free_mb=300, note="low disk pauses downloads")
    # a download that keeps failing: six tries each pass (the first, then five retries), three passes, then it is given up
    d.user_upload("later.m4a", b"HHHH" * 50, [mf], created="2026-10-06T10:00:00.000Z", fid="aud-h-0000000000")
    for _ in range(18):
        d.fail.append((lambda m, p, q: q.get("alt") == ["media"] and "aud-h" in p, 500, "download failed"))
    sc.run(root, ["run"], note="download fails: back to listed, tries 1")
    sc.run(root, ["run"], note="download fails: tries 2")
    sc.run(root, ["run"], note="download fails: tries 3, failed")
    sc.run(root, ["run"], note="a failed memo is left alone")
    return finish(sc, root, initial)


def failures(w: World):
    sc = Scenario("failures", w)
    d, pk = w.drive, w.pk
    with_folder(w)
    root = build_root(sc)
    initial = dict(sc.files)
    pk.down = True
    sc.run(root, ["run"], note="api down")
    pk.down = False
    d.user_create("First bit", "one", fid="doc-one-00000000000", parents=(FOLDER_ID,))
    d.user_create("Second bit", "two", fid="doc-two-00000000000", parents=(FOLDER_ID,))
    d.fail.append((lambda m, p, q: p.endswith("/export") and "doc-one" in p, 503, "Backend Error"))
    d.fail.append((lambda m, p, q: p.endswith("/export") and "doc-one" in p, 429, "slow down"))
    sc.run(root, ["run"], note="export retried after 503 and 429")
    d.fail.append((lambda m, p, q: p.endswith("/export") and "doc-two" in p, 403, "Rate Limit Exceeded", "rateLimitExceeded"))
    d.fail.append((lambda m, p, q: p.endswith("/export") and "doc-two" in p, 403, "Forbidden", "forbidden"))
    d.user_edit("doc-two-00000000000", text="two again", modified="2026-10-26T09:00:00.000Z")
    d.user_edit("doc-one-00000000000", text="one again", modified="2026-10-26T09:00:00.000Z")
    for _ in range(8):
        d.fail.append((lambda m, p, q: p.endswith("/export") and "doc-one" in p, 500, "boom"))
    sc.run(root, ["run"], note="a doc fails 8 times -> error, pending, run goes on")
    pk.fail.append((lambda m, p, q: m == "POST" and p == "/api/worker/decision", 400, "bad line"))
    sc.run(root, ["run"], note="retry the failed doc; the edit is refused by the API")
    pk.fail.append((lambda m, p, q: m == "POST" and p == "/api/worker/decision", 500, "server error"))
    d.user_edit("doc-two-00000000000", text="two a third time", modified="2026-10-26T09:00:00.000Z")
    sc.run(root, ["run"], note="API 500 on an edit")
    d.fail.append((lambda m, p, q: p == "/drive/v3/changes", 401, "Invalid Credentials"))
    sc.run(root, ["run"], note="changes feed refused")
    d.user_edit("doc-two-00000000000", text="two a fourth time", modified="2026-10-26T09:00:00.000Z")
    pk.fail.append((lambda m, p, q: p.startswith("/api/worker/jokes/"), 500, "joke read failed"))
    sc.run(root, ["run"], note="joke read fails mid-run")
    return finish(sc, root, initial)


def auth_chain(w: World):
    sc = Scenario("auth_chain", w)
    d = w.drive
    with_folder(w)
    d.user_create("A bit", "words", fid="doc-a-000000000000", parents=(FOLDER_ID,))
    root = build_root(sc)
    initial = dict(sc.files)
    sc.run(root, ["run"], note="cached google token refreshed, keycloak not touched")
    # a day later: keycloak is asked (rotation), the broker token used
    sc.run(root, ["run"], now="2026-10-27T12:30:00+00:00", note="keycloak daily check via the broker")
    w.kc.google_dead.add("fake-google-refresh-broker")
    w.kc.broker_rt = "fake-google-refresh-broker2"
    sc.write(root, "home/google_token.json", json.dumps({"refresh_token": "fake-dead", "source": "x", "kc_checked_at": NOW_TS - 10}))
    w.kc.google_dead.add("fake-dead")
    sc.run(root, ["run"], note="google refuses the cached token -> keycloak broker gives a fresh one")
    w.kc.broker_status = 400
    sc.delete(root, "home/google_token.json")
    sc.run(root, ["run"], note="broker 400: not linked")
    w.kc.broker_status = 403
    sc.run(root, ["run"], note="broker 403")
    w.kc.broker_status = 200
    w.kc.kc_down = True
    sc.run(root, ["run"], note="keycloak expired")
    w.kc.kc_down = False
    sc.delete(root, "home/keycloak_gsync_token.json")
    sc.write(root, "home/google_token.json", json.dumps({"refresh_token": "fake-desktop-refresh", "client_id": "fake-desktop-client",
                                                         "client_secret": "fake-desktop-secret", "token_uri": w.base + "/google/token",
                                                         "scopes": ["https://www.googleapis.com/auth/drive"]}))
    sc.run(root, ["run"], note="desktop fallback token")
    sc.delete(root, "home/google_token.json")
    sc.run(root, ["run"], note="nothing set up: no_auth")
    sc.write(root, "home/keycloak_gsync_token.json", json.dumps({"refresh_token": w.kc.refresh}))
    sc.write(root, "home/google-oauth.env", "GOOGLE_CLIENT_ID=fake-client-id\n")
    sc.run(root, ["run"], note="google client secret missing")
    return finish(sc, root, initial)


def auth_device_and_commands(w: World):
    sc = Scenario("auth_device_and_commands", w)
    d, pk = w.drive, w.pk
    with_folder(w)
    d.user_create("Status bit", "words", fid="doc-status-0000000000", parents=(FOLDER_ID,))
    root = build_root(sc)
    initial = dict(sc.files)
    sc.run(root, ["status"], note="status before the first run")
    sc.run(root, ["run"], note="first run")
    j = pk.app_idea("An app joke", "words of the app joke")
    d.user_edit("doc-status-0000000000", text="changed words", modified="2026-10-26T09:00:00.000Z")
    sc.run(root, ["run"], note="a run with actions")
    pk.fail.append((lambda m, p, q: False, 0, ""))
    runs = [json.loads(x) for x in (root / "ws/comedy/kb/gsync/runs.jsonl").read_text().splitlines()]
    rid = next(r["run_id"] for r in runs if r.get("actions") and any(a.get("indexes") for a in r["actions"]))
    sc.run(root, ["status"], note="status after runs")
    sc.run(root, ["undo", rid], note="undo the run's changes")
    sc.run(root, ["undo", "gs-00000000-000000"], note="undo unknown run")
    # undo with a line the API refuses
    runs_path = root / "ws/comedy/kb/gsync/runs.jsonl"
    sc.write(root, "ws/comedy/kb/gsync/runs.jsonl", runs_path.read_text(encoding="utf-8") +
             json.dumps({"run_id": "gs-fake-bad", "actions": [{"op": "edit", "indexes": [999, 0]}]}) + "\n")
    sc.run(root, ["undo", "gs-fake-bad"], note="undo with a refused line")
    # a fresh sign-in
    sc.delete(root, "home/keycloak_gsync_token.json")
    sc.run(root, ["auth"], note="device sign-in")
    sc.run(root, ["run"], note="run after sign-in")
    return finish(sc, root, initial)


def hosted_state(w: World):
    sc = Scenario("hosted_state", w)
    d, pk = w.drive, w.pk
    with_folder(w)
    d.user_create("State bit", "words", fid="doc-state-00000000000", parents=(FOLDER_ID,))
    # a fresh PC: gsync's state is in the database already
    pk.files["kb/gsync/map.json"] = json.dumps({"version": 1, "changes_token": "0", "folder_id": FOLDER_ID, "docs": {}, "boards": {},
                                                "excluded": {}, "skipped": {}, "pending": {}, "folders": {}, "bootstrapped_at": "x"})
    pk.files["kb/gsync/conflicts.jsonl"] = '{"ts":"t","kind":"old"}\n'
    pk.files["kb/gsync/memos.json"] = json.dumps({"version": 1, "items": {}, "paused": None, "processor": {}})
    pk.files["kb/v1/graph.json"] = json.dumps({"nodes": [], "edges": []})
    root = build_root(sc)
    initial = dict(sc.files)
    sc.run(root, ["run"], note="pull_state on a fresh PC, then a pull run, then push_state")
    pk.fail.append((lambda m, p, q: m == "GET" and p == "/api/worker/files" and q.get("prefix") == ["kb/gsync/"], 500, "files down"))
    sc.run(root, ["run"], note="pull_state fails and is ignored")
    pk.fail.append((lambda m, p, q: m == "POST" and p == "/api/worker/files", 500, "files down"))
    d.user_edit("doc-state-00000000000", text="more words", modified="2026-10-26T09:00:00.000Z")
    sc.run(root, ["run"], note="push_state fails: recorded in the run")
    sc.run(root, ["run"], now="2026-10-26T12:20:00+00:00", note="runs.jsonl pushed again after ten minutes")
    return finish(sc, root, initial)


def dry_run(w: World):
    sc = Scenario("dry_run", w, files={
        "ws/comedy/corpus/gdocs/2026-09-01_old-bit__corpus01.md":
            '---\nsource: gdoc\ndoc_id: corpus-doc-0001\ntitle: "Old bit"\nmodified: 2026-09-01T10:00:00.000Z\n---\nold text\n'})
    d, pk = w.drive, w.pk
    with_folder(w)
    d.user_create("Dry run bit", "words\nmore words", fid="doc-dry-000000000000", parents=(FOLDER_ID,))
    d.user_create("Petition x", "never", fid="doc-dry-legal-0000000", parents=(FOLDER_ID,))
    pk.app_idea("An app idea", "app words")
    pk.stores["storyboards"]["sb:dry"] = {"id": "sb:dry", "title": "Dry set", "by": "owner", "joke_ids": ["joke:an-app-idea"]}
    root = build_root(sc)
    initial = dict(sc.files)
    sc.run(root, ["run", "--dry-run"], note="dry bootstrap: reads only")
    sc.run(root, ["run"], note="real bootstrap")
    d.user_edit("doc-dry-000000000000", text="edited words", modified="2026-10-26T09:00:00.000Z")
    pk.app_idea("Second app idea", "more app words")
    sc.run(root, ["run", "--dry-run"], note="dry pull: reads only")
    return finish(sc, root, initial)


def adopted_docs(w: World):
    ids = {"folder_id": FOLDER_ID, "board_docs": {"sb:set-two": BOARD_DOC}, "ai_draft_docs": {AI_DOC: "AI walk-through (DRAFT)"},
           "mixed_docs": {MIXED_DOC: "joke:good-data"}, "adopted_joke_docs": {ADOPTED_1: "joke:adopted-one", ADOPTED_2: "joke:adopted-two"}}
    sc = Scenario("adopted_docs", w, files={"home/gsync_docs.json": json.dumps(ids)})
    d, pk = w.drive, w.pk
    with_folder(w)
    d.user_create("Set two (board)", "hand-made export", fid=BOARD_DOC, parents=(FOLDER_ID,))
    d.user_create("AI walk-through (DRAFT)", "YOURS: line\nDRAFT: ai line", fid=AI_DOC, parents=(FOLDER_ID,))
    d.user_create("Good data", "his words here\n\n---\n\nPunchable notes: ai text", fid=MIXED_DOC, parents=(FOLDER_ID,))
    d.user_create("Adopted one", "adopted text one", fid=ADOPTED_1, parents=(FOLDER_ID,))
    d.user_create("Adopted two", "adopted text two", fid=ADOPTED_2, parents=(FOLDER_ID,))
    for j, t in (("joke:good-data", "Good data"), ("joke:adopted-one", "Adopted one"), ("joke:adopted-two", "Adopted two")):
        pk.jokes[j] = {"id": j, "title": t, "summary": "app text " + t, "stage": "idea", "lead_text": None, "hidden": False}
    pk.line(target="node", id="joke:good-data", action="add", after={"type": "joke", "title": "Good data"}, reason="saved")
    pk.line(target="node", id="joke:adopted-one", action="add", after={"type": "joke", "title": "Adopted one"}, reason="saved")
    pk.line(target="node", id="joke:adopted-two", action="add", after={"type": "joke", "title": "Adopted two"}, reason="saved")
    root = build_root(sc)
    initial = dict(sc.files)
    sc.run(root, ["run"], note="adoption + baselines")
    d.user_edit(AI_DOC, text="YOURS: line\nDRAFT: ai line\nYOURS: edited", modified="2026-10-26T09:00:00.000Z")
    d.user_edit(MIXED_DOC, text="his fresh phrasing\n\n---\n\nnotes", modified="2026-10-26T09:00:00.000Z")
    d.user_edit(ADOPTED_1, text="adopted text one, edited", modified="2026-10-26T09:00:00.000Z")
    d.user_edit(BOARD_DOC, text="board doc changed by hand", modified="2026-10-26T09:00:00.000Z")
    sc.run(root, ["run"], note="edits to adopted docs")
    pk.app_edit("joke:good-data", "app side edit of the mixed doc")
    pk.app_edit("joke:adopted-two", "app side edit of an adopted doc")
    d.user_edit(MIXED_DOC, text="no rule here at all", modified="2026-10-26T09:10:00.000Z")
    sc.run(root, ["run"], note="mixed doc loses its rule; app edits of own docs are noted, not written")
    d.user_edit(MIXED_DOC, text="In the Supreme Court of the State of nowhere\n---\nthe petitioner", modified="2026-10-26T09:20:00.000Z")
    sc.run(root, ["run"], note="mixed doc turns legal")
    d.user_trash(BOARD_DOC)
    d.user_trash(AI_DOC)
    sc.run(root, ["run"], note="a board doc and an AI doc trashed")
    return finish(sc, root, initial)


def narrow_scopes(w: World):
    sc = Scenario("narrow_scopes", w, env={"GSYNC_SCOPES": "narrow"})
    d, pk = w.drive, w.pk
    with_folder(w)
    d.user_create("Narrow bit", "words", fid="doc-narrow-0000000000", parents=(FOLDER_ID,))
    pk.app_idea("Narrow app idea", "app words")
    root = build_root(sc)
    initial = dict(sc.files)
    sc.run(root, ["run"], note="narrow scopes: no folder, no board adoption, no memos")
    sc.run(root, ["run"], note="and again")
    return finish(sc, root, initial)


def bootstrap_error(w: World):
    sc = Scenario("bootstrap_error", w)
    d, pk = w.drive, w.pk
    with_folder(w)
    d.user_create("Fine bit", "words", fid="doc-fine-00000000000", parents=(FOLDER_ID,))
    d.user_create("Broken bit", "words", fid="doc-broken-0000000000", parents=(FOLDER_ID,))
    root = build_root(sc)
    initial = dict(sc.files)
    for _ in range(8):
        d.fail.append((lambda m, p, q: p.endswith("/export") and "doc-broken" in p, 500, "boom"))
    sc.run(root, ["run"], note="a doc fails during the bootstrap: the changes token is not kept")
    sc.run(root, ["run"], note="the bootstrap runs again and succeeds")
    sc.run(root, ["run"], note="then pull")
    return finish(sc, root, initial)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    ap.add_argument("only", nargs="*")
    a = ap.parse_args()
    TAPES.mkdir(parents=True, exist_ok=True)
    bad = 0
    for build in scenarios():
        if a.only and build.__name__ not in a.only:
            continue
        w = World()
        w.start()
        tape = build(w)
        w.srv.shutdown()
        text = json.dumps(tape, ensure_ascii=False, indent=1) + "\n"
        p = TAPES / f"{tape['name']}.json"
        if a.check:
            if not p.exists() or p.read_text(encoding="utf-8") != text:
                print("DIFFERS", p.name)
                bad += 1
            continue
        p.write_text(text, encoding="utf-8", newline="\n")
        n = sum(len(s["exchanges"]) for s in tape["steps"])
        print(f"{p.name}: {len(tape['steps'])} steps, {n} exchanges")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
