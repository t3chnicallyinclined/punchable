#!/usr/bin/env python3
"""Nightly SurrealDB backup to an S3-compatible bucket (Cloudflare R2). Python 3.13 standard library only: it runs in
the pinned python slim image with no pip (infra/k8s/60-backup.yaml).

  backup.py run                     export -> gzip -> check -> PUT -> HEAD -> (Sunday UTC: weekly copy) -> prune
  backup.py list                    the backups under the prefix, oldest first
  backup.py fetch <key> <out.gz>    download one, check its length and gzip CRC
  backup.py restore-check <file.gz> the same checks `run` makes before an upload, on a local file
  backup.py export <out.gz>         export + checks to a local file, no upload (needs no R2_* settings)

Settings (environment only; nothing secret is ever on argv or printed):
  SURREAL_URL       http://surrealdb:8000          SurrealDB HTTP endpoint (`GET /export`)
  SURREAL_NS / SURREAL_DB     punchable / main
  SURREAL_USER / SURREAL_PASS                      the login used for the export (see infra/README.md "Backups")
  SURREAL_AUTH_LEVEL database (default: a user defined ON DATABASE SURREAL_DB; EDITOR or VIEWER both export the
                    same bytes as root) | root
  R2_ENDPOINT       https://<account>.r2.cloudflarestorage.com
  R2_BUCKET, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY  a token scoped to that one bucket
  R2_REGION         auto
  BACKUP_PREFIX     surreal/        must start with "surreal" and end with "/"; nothing outside it is ever deleted
  BACKUP_KEEP_DAILY 14   BACKUP_KEEP_WEEKLY 8
  BACKUP_WEEKLY     auto (Sunday UTC) | always | never
  BACKUP_MIN_BYTES  4096 (uncompressed export smaller than this is refused)
  BACKUP_TMP        /tmp

Keys: <prefix>daily/<ns>-<db>-<yyyymmddTHHMMSSZ>.surql.gz and <prefix>weekly/<same name>.
Exit status: 0 ok, 1 any failure (one line on stderr, token-like strings redacted)."""
from __future__ import annotations

import base64
import datetime as dt
import gzip
import hashlib
import hmac
import http.client
import os
import re
import sys
import tempfile
import time
import urllib.parse
import xml.etree.ElementTree as ET
import zlib

CHUNK = 1 << 20
S3NS = "{http://s3.amazonaws.com/doc/2006-03-01/}"


class BackupError(Exception):
    pass


# --------------------------------------------------------------------------------------------------------------------
# redaction
# --------------------------------------------------------------------------------------------------------------------
_TOKENISH = re.compile(r"[A-Za-z0-9+/=_\-]{40,}")
_SECRET_ENV = ("SURREAL_PASS", "R2_SECRET_ACCESS_KEY", "R2_ACCESS_KEY_ID", "R2_ACCOUNT_ID", "R2_TOKEN_ID", "R2_ENDPOINT",
               "R2_BUCKET")


def redact(text: str) -> str:
    for k in _SECRET_ENV:
        v = os.environ.get(k, "")
        if len(v) >= 6:
            text = text.replace(v, f"<{k}>")
    return _TOKENISH.sub(lambda m: m.group(0) if _OUR_KEY.fullmatch(m.group(0)) else "<redacted>", text)


# a backup key is not a token: surreal[-x]/daily|weekly/<ns>-<db>-<stamp>(.surql.gz is cut off by the token regex)
_OUR_KEY = re.compile(r"surreal[a-z0-9-]*/(daily|weekly)/[a-z0-9_-]+-\d{8}T\d{6}Z")


# --------------------------------------------------------------------------------------------------------------------
# settings
# --------------------------------------------------------------------------------------------------------------------
def env(name: str, default: str | None = None) -> str:
    v = os.environ.get(name, default)
    if v is None or v == "":
        raise BackupError(f"{name} is not set")
    return v


def prefix() -> str:
    p = os.environ.get("BACKUP_PREFIX", "surreal/")
    if not re.fullmatch(r"surreal[a-z0-9-]*/", p):
        raise BackupError(f"BACKUP_PREFIX must look like 'surreal/' or 'surreal-<word>/' (got {p!r})")
    return p


def int_env(name: str, default: int, lo: int = 1) -> int:
    raw = os.environ.get(name, str(default))
    try:
        n = int(raw)
    except ValueError:
        raise BackupError(f"{name} must be an integer") from None
    if n < lo:
        raise BackupError(f"{name} must be >= {lo}")
    return n


# --------------------------------------------------------------------------------------------------------------------
# AWS Signature Version 4 (S3 flavour: the path is encoded once, `/` kept)
# --------------------------------------------------------------------------------------------------------------------
EMPTY_SHA256 = hashlib.sha256(b"").hexdigest()


def _q(s: str, safe: str = "-_.~") -> str:
    return urllib.parse.quote(s, safe=safe)


def _hmac(key: bytes, msg: str) -> bytes:
    return hmac.new(key, msg.encode(), hashlib.sha256).digest()


def sigv4(method: str, path: str, query: dict[str, str], headers: dict[str, str], payload_hash: str,
          access_key: str, secret_key: str, region: str, service: str, amz_date: str) -> str:
    """The Authorization header value. `headers` must already hold host and x-amz-date (and x-amz-content-sha256 for
    S3); every header given is signed. `path` is the raw (unencoded) path."""
    canon_path = _q(path, safe="/-_.~")
    canon_query = "&".join(f"{_q(k)}={_q(v)}" for k, v in sorted(query.items()))
    hs = {k.lower().strip(): " ".join(str(v).strip().split()) for k, v in headers.items()}
    signed = ";".join(sorted(hs))
    canon_headers = "".join(f"{k}:{hs[k]}\n" for k in sorted(hs))
    creq = "\n".join([method, canon_path, canon_query, canon_headers, signed, payload_hash])
    scope = f"{amz_date[:8]}/{region}/{service}/aws4_request"
    sts = "\n".join(["AWS4-HMAC-SHA256", amz_date, scope, hashlib.sha256(creq.encode()).hexdigest()])
    k = _hmac(("AWS4" + secret_key).encode(), amz_date[:8])
    k = _hmac(k, region)
    k = _hmac(k, service)
    k = _hmac(k, "aws4_request")
    sig = hmac.new(k, sts.encode(), hashlib.sha256).hexdigest()
    return f"AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed}, Signature={sig}"


class S3:
    """Path-style requests to one bucket: https://<endpoint>/<bucket>/<key>."""

    def __init__(self):
        ep = urllib.parse.urlsplit(env("R2_ENDPOINT"))
        if ep.scheme != "https" or not ep.hostname:
            raise BackupError("R2_ENDPOINT must be an https:// URL")
        self.host = ep.hostname
        self.port = ep.port or 443
        self.bucket = env("R2_BUCKET")
        self.ak = env("R2_ACCESS_KEY_ID")
        self.sk = env("R2_SECRET_ACCESS_KEY")
        self.region = os.environ.get("R2_REGION", "auto")

    def request(self, method: str, key: str = "", query: dict | None = None, body=None, length: int | None = None,
                payload_hash: str = EMPTY_SHA256, extra: dict | None = None, out=None, timeout: float = 300):
        """One signed request. Returns (status, headers, body bytes) -- or, with `out`, streams the body into it and
        returns its length as the body. Retries network errors and 5xx three times (not with a body stream that has
        been consumed: the caller passes a callable for that)."""
        query = query or {}
        path = f"/{self.bucket}/{key}" if key else f"/{self.bucket}"
        last = None
        for attempt in range(3):
            amz_date = dt.datetime.now(dt.UTC).strftime("%Y%m%dT%H%M%SZ")
            headers = {"host": self.host if self.port == 443 else f"{self.host}:{self.port}", "x-amz-date": amz_date,
                       "x-amz-content-sha256": payload_hash, **(extra or {})}
            if length is not None:
                headers["content-length"] = str(length)
            headers["Authorization"] = sigv4(method, path, query, headers, payload_hash, self.ak, self.sk,
                                             self.region, "s3", amz_date)
            url = _q(path, safe="/-_.~") + ("?" + "&".join(f"{_q(k)}={_q(v)}" for k, v in sorted(query.items()))
                                             if query else "")
            conn = http.client.HTTPSConnection(self.host, self.port, timeout=timeout)
            data = body() if callable(body) else body
            try:
                conn.request(method, url, body=data, headers=headers)
                r = conn.getresponse()
                if r.status >= 500:
                    last = BackupError(f"{method} {key or '/'}: HTTP {r.status}")
                    r.read()
                    time.sleep(2 * (attempt + 1))
                    continue
                if out is not None and r.status == 200:
                    n = 0
                    while chunk := r.read(CHUNK):
                        out.write(chunk)
                        n += len(chunk)
                    return r.status, {k.lower(): v for k, v in r.getheaders()}, n
                return r.status, {k.lower(): v for k, v in r.getheaders()}, r.read()
            except (OSError, http.client.HTTPException) as e:
                last = BackupError(f"{method} {key or '/'}: {type(e).__name__}: {e}")
                if out is not None:
                    raise last from None   # a partial download is not retried into the same file
                time.sleep(2 * (attempt + 1))
            finally:
                conn.close()
                if callable(body) and hasattr(data, "close"):
                    data.close()
        raise last  # type: ignore[misc]

    def put_file(self, key: str, path: str, sha256: str) -> None:
        size = os.path.getsize(path)

        def opener():
            return open(path, "rb")
        st, _, body = self.request("PUT", key, body=opener, length=size, payload_hash=sha256,
                                   extra={"content-type": "application/gzip"}, timeout=900)
        if st != 200:
            raise BackupError(f"PUT {key}: HTTP {st} {_s3_error(body)}")

    def copy(self, src_key: str, dst_key: str) -> None:
        st, _, body = self.request("PUT", dst_key, extra={"x-amz-copy-source": _q(f"/{self.bucket}/{src_key}",
                                                                                   safe="/-_.~")})
        if st != 200 or b"<Error>" in body:
            raise BackupError(f"COPY -> {dst_key}: HTTP {st} {_s3_error(body)}")

    def head(self, key: str) -> int | None:
        st, h, _ = self.request("HEAD", key)
        if st == 404:
            return None
        if st != 200:
            raise BackupError(f"HEAD {key}: HTTP {st}")
        return int(h.get("content-length", "-1"))

    def delete(self, key: str) -> None:
        st, _, body = self.request("DELETE", key)
        if st not in (200, 204):
            raise BackupError(f"DELETE {key}: HTTP {st} {_s3_error(body)}")

    def list(self, pfx: str) -> list[tuple[str, int]]:
        out, token = [], None
        while True:
            q = {"list-type": "2", "prefix": pfx, "max-keys": "1000"}
            if token:
                q["continuation-token"] = token
            st, _, body = self.request("GET", "", query=q)
            if st != 200:
                raise BackupError(f"LIST {pfx}: HTTP {st} {_s3_error(body)}")
            root = ET.fromstring(body)
            for c in root.iter(f"{S3NS}Contents"):
                out.append((c.findtext(f"{S3NS}Key"), int(c.findtext(f"{S3NS}Size") or 0)))
            if (root.findtext(f"{S3NS}IsTruncated") or "").lower() != "true":
                return out
            token = root.findtext(f"{S3NS}NextContinuationToken")
            if not token:
                raise BackupError("LIST: truncated listing without a continuation token")


def _s3_error(body: bytes) -> str:
    m = re.search(rb"<Code>([^<]{1,80})</Code>", body or b"")
    return m.group(1).decode() if m else ""


# --------------------------------------------------------------------------------------------------------------------
# the dump: export (streamed) -> gzip -> checks
# --------------------------------------------------------------------------------------------------------------------
def export_to_gzip(dest: str) -> int:
    """Stream SurrealDB's `GET /export` for SURREAL_NS/SURREAL_DB into a gzip file. Returns uncompressed bytes."""
    u = urllib.parse.urlsplit(env("SURREAL_URL", "http://surrealdb:8000"))
    if u.scheme not in ("http", "https") or not u.hostname:
        raise BackupError("SURREAL_URL must be http(s)://host:port")
    auth = base64.b64encode(f"{env('SURREAL_USER')}:{env('SURREAL_PASS')}".encode()).decode()
    ns, db = env("SURREAL_NS", "punchable"), env("SURREAL_DB", "main")
    headers = {"Authorization": "Basic " + auth, "Surreal-NS": ns, "Surreal-DB": db, "Accept": "text/plain"}
    level = os.environ.get("SURREAL_AUTH_LEVEL", "database")
    if level == "database":
        # SurrealDB 3.x checks Basic credentials against a DATABASE user only when these two headers name it;
        # without them it tries root and answers 401.
        headers |= {"Surreal-Auth-NS": ns, "Surreal-Auth-DB": db}
    elif level != "root":
        raise BackupError("SURREAL_AUTH_LEVEL must be database or root")
    C = http.client.HTTPSConnection if u.scheme == "https" else http.client.HTTPConnection
    conn = C(u.hostname, u.port or (443 if u.scheme == "https" else 80), timeout=600)
    n = 0
    try:
        conn.request("GET", (u.path.rstrip("/") or "") + "/export", headers=headers)
        r = conn.getresponse()
        if r.status != 200:
            raise BackupError(f"export: HTTP {r.status} {r.read(300).decode('utf-8', 'replace').strip()}")
        with open(dest, "wb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", compresslevel=6, mtime=0) as gz:
            while chunk := r.read(CHUNK):   # a dropped chunked stream raises IncompleteRead here
                gz.write(chunk)
                n += len(chunk)
    except (OSError, http.client.HTTPException) as e:
        raise BackupError(f"export: {type(e).__name__}: {e}") from None
    finally:
        conn.close()
    return n


def check_dump(path: str, min_bytes: int | None = None) -> dict:
    """Read the whole gzip (CRC and length are checked by the gzip reader at EOF) and look at the SurrealQL inside.
    Raises BackupError when the file is not a complete, plausible export."""
    min_bytes = int_env("BACKUP_MIN_BYTES", 4096) if min_bytes is None else min_bytes
    if not os.path.isfile(path):
        raise BackupError(f"{path}: no such file")
    sha, n, tables, tail, head = hashlib.sha256(), 0, 0, b"", b""
    carry = b""
    try:
        with open(path, "rb") as f:
            while chunk := f.read(CHUNK):
                sha.update(chunk)
        with gzip.open(path, "rb") as g:
            while chunk := g.read(CHUNK):
                if len(head) < 4096:
                    head = (head + chunk)[:4096]
                buf = carry + chunk   # count line-start matches once, also across a chunk boundary
                tables += buf.count(b"\nDEFINE TABLE ") - carry.count(b"\nDEFINE TABLE ")
                carry = buf[-16:]
                tail = (tail + chunk)[-256:]
                n += len(chunk)
    except (EOFError, OSError, zlib.error) as e:
        # gzip.BadGzipFile is an OSError; a truncated file is EOFError; a flipped bit fails the CRC (BadGzipFile)
        raise BackupError(f"{os.path.basename(path)}: not a complete gzip ({type(e).__name__}: {e})") from None
    if n < min_bytes:
        raise BackupError(f"{os.path.basename(path)}: export is only {n} bytes (BACKUP_MIN_BYTES={min_bytes})")
    if b"OPTION IMPORT;" not in head:
        raise BackupError(f"{os.path.basename(path)}: does not start like a SurrealDB export (no OPTION IMPORT)")
    if tables < 1:
        raise BackupError(f"{os.path.basename(path)}: no DEFINE TABLE in the export")
    if not tail.rstrip().endswith(b";"):
        raise BackupError(f"{os.path.basename(path)}: export does not end with a complete statement")
    return {"bytes_gz": os.path.getsize(path), "bytes_sql": n, "tables": tables, "sha256": sha.hexdigest()}


# --------------------------------------------------------------------------------------------------------------------
# retention
# --------------------------------------------------------------------------------------------------------------------
def key_re(pfx: str, kind: str, name: str) -> re.Pattern:
    return re.compile(re.escape(pfx) + kind + "/" + re.escape(name) + r"-(\d{8}T\d{6}Z)\.surql\.gz")


def select_prune(keys: list[str], pfx: str, name: str, keep_daily: int, keep_weekly: int
                 ) -> tuple[list[str], list[str]]:
    """(keep, delete). Only keys that are exactly <pfx>daily|weekly/<name>-<stamp>.surql.gz (name = <ns>-<db>) are
    ever candidates; anything else, under or outside the prefix (another database's backups too), is left alone."""
    if not re.fullmatch(r"surreal[a-z0-9-]*/", pfx):
        raise BackupError(f"refusing to prune with prefix {pfx!r}")
    if not re.fullmatch(r"[a-z0-9_]+-[a-z0-9_]+", name):
        raise BackupError(f"refusing to prune with name {name!r}")
    if keep_daily < 1 or keep_weekly < 1:
        raise BackupError("retention must keep at least one daily and one weekly")
    keep, delete = [], []
    for kind, n in (("daily", keep_daily), ("weekly", keep_weekly)):
        rx = key_re(pfx, kind, name)
        mine = sorted((m.group(1), k) for k in keys if (m := rx.fullmatch(k)))
        mine.reverse()   # newest first
        keep += [k for _, k in mine[:n]]
        delete += [k for _, k in mine[n:]]
    for k in delete:   # belt and braces: the regex already guarantees this
        assert k.startswith(pfx) and k.startswith("surreal") and ".." not in k, k
    return keep, delete


# --------------------------------------------------------------------------------------------------------------------
# commands
# --------------------------------------------------------------------------------------------------------------------
def log(msg: str) -> None:
    print(redact(msg), flush=True)


def want_weekly(now: dt.datetime) -> bool:
    mode = os.environ.get("BACKUP_WEEKLY", "auto")
    if mode not in ("auto", "always", "never"):
        raise BackupError("BACKUP_WEEKLY must be auto, always or never")
    return mode == "always" or (mode == "auto" and now.weekday() == 6)


def cmd_run() -> int:
    pfx = prefix()
    keep_d, keep_w = int_env("BACKUP_KEEP_DAILY", 14), int_env("BACKUP_KEEP_WEEKLY", 8)
    ns, db = env("SURREAL_NS", "punchable"), env("SURREAL_DB", "main")
    if not re.fullmatch(r"[a-z0-9_]+", ns) or not re.fullmatch(r"[a-z0-9_]+", db):
        raise BackupError("SURREAL_NS / SURREAL_DB must be [a-z0-9_]+")
    s3 = S3()
    want_weekly(dt.datetime.now(dt.UTC))   # validate before the export
    now = dt.datetime.now(dt.UTC)
    name = f"{ns}-{db}-{now.strftime('%Y%m%dT%H%M%SZ')}.surql.gz"
    tmpdir = os.environ.get("BACKUP_TMP", tempfile.gettempdir())
    fd, tmp = tempfile.mkstemp(prefix="surreal-export-", suffix=".surql.gz", dir=tmpdir)
    os.close(fd)
    try:
        t0 = time.monotonic()
        n = export_to_gzip(tmp)
        info = check_dump(tmp)
        if info["bytes_sql"] != n:
            raise BackupError("gzip re-read length differs from the export length")
        log(f"export ok: {ns}/{db} {n} bytes -> {info['bytes_gz']} gz, {info['tables']} tables, "
            f"{time.monotonic() - t0:.1f}s")
        daily = f"{pfx}daily/{name}"
        s3.put_file(daily, tmp, info["sha256"])
        got = s3.head(daily)
        if got != info["bytes_gz"]:
            raise BackupError(f"HEAD {daily}: size {got}, expected {info['bytes_gz']}")
        log(f"uploaded {daily} ({got} bytes, verified)")
        if want_weekly(now):
            weekly = f"{pfx}weekly/{name}"
            s3.copy(daily, weekly)
            got = s3.head(weekly)
            if got != info["bytes_gz"]:
                raise BackupError(f"HEAD {weekly}: size {got}, expected {info['bytes_gz']}")
            log(f"weekly copy {weekly} (verified)")
    finally:
        try:
            os.remove(tmp)
        except OSError:
            pass
    # prune: only reached after a verified upload
    keys = [k for k, _ in s3.list(pfx)]
    keep, delete = select_prune(keys, pfx, f"{ns}-{db}", keep_d, keep_w)
    if daily not in keep:
        raise BackupError(f"prune refused: the new backup {daily} is not among the newest {keep_d} (clock skew?)")
    for k in delete:
        s3.delete(k)
    log(f"retention: kept {len(keep)} ({keep_d} daily / {keep_w} weekly max), deleted {len(delete)}")
    return 0


def cmd_list() -> int:
    for k, size in sorted(S3().list(prefix())):
        print(f"{size:>12}  {k}")
    return 0


def cmd_fetch(key: str, out: str) -> int:
    pfx = prefix()
    if not key.startswith(pfx):
        raise BackupError(f"key must be under {pfx}")
    s3 = S3()
    expected = s3.head(key)
    if expected is None:
        raise BackupError(f"{key}: not found")
    part = out + ".part"
    try:
        with open(part, "wb") as f:
            st, _, n = s3.request("GET", key, out=f, timeout=900)
        if st != 200:
            raise BackupError(f"GET {key}: HTTP {st}")
        if n != expected:
            raise BackupError(f"GET {key}: got {n} bytes, expected {expected}")
        info = check_dump(part)
        os.replace(part, out)
    finally:
        if os.path.exists(part):
            os.remove(part)
    log(f"fetched {key} -> {out}: {info['bytes_gz']} gz, {info['bytes_sql']} sql bytes, {info['tables']} tables, "
        f"sha256 {info['sha256'][:16]}")
    return 0


def cmd_export(out: str) -> int:
    n = export_to_gzip(out)
    try:
        info = check_dump(out)
    except BackupError:
        os.remove(out)
        raise
    log(f"export ok: {n} bytes -> {out} ({info['bytes_gz']} gz, {info['tables']} tables)")
    return 0


def cmd_restore_check(path: str) -> int:
    info = check_dump(path)
    log(f"ok: {info['bytes_gz']} gz, {info['bytes_sql']} sql bytes, {info['tables']} tables, "
        f"sha256 {info['sha256'][:16]}")
    return 0


def main(argv: list[str]) -> int:
    try:
        match argv:
            case ["run"]:
                return cmd_run()
            case ["list"]:
                return cmd_list()
            case ["fetch", key, out]:
                return cmd_fetch(key, out)
            case ["restore-check", path]:
                return cmd_restore_check(path)
            case ["export", out]:
                return cmd_export(out)
            case _:
                print("usage: backup.py run | list | fetch <key> <out.gz> | restore-check <file.gz> | export <out.gz>", file=sys.stderr)
                return 2
    except BackupError as e:
        print(redact(f"backup: FAIL {e}"), file=sys.stderr)
        return 1
    except Exception as e:  # noqa: BLE001 -- one line, never a traceback with request headers in it
        print(redact(f"backup: FAIL {type(e).__name__}: {e}"), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
