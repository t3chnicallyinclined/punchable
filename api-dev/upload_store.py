"""Uploads staged in the database (hosted; db/schema_extra.surql `upload`, `upload_part`).

The hosted pod used to keep an upload in its inbox folder until a transcriber had processed it, which only the
Python could read. Now the file is rows both API servers see (pk-api answers /api/upload, /api/inbox and
/api/worker/jobs/<id>/file too): `upload_part` rows of at most PART bytes keyed by a private `token`, and the
`upload:<file name>` row, made LAST, which names the token and holds the meta.json. A name is reserved by that
CREATE (the unique record id), so two uploads in the same second never share one; a refused CREATE tries the next
suffix. Local development (no jobs_on_upload) keeps the inbox folder and none of this.
"""
from __future__ import annotations

import re
import time

PART = 1024 * 1024
ORPHAN_S = 3600
NAME_RE = re.compile(r"^[A-Za-z0-9._\-]{1,200}$")


def put_part(rpc, token: str, seq: int, data: bytes) -> None:
    rpc.write("CREATE upload_part CONTENT {token: $k, seq: $s, t: $t, data: $d} RETURN id;",
              {"k": token, "s": seq, "t": time.time(), "d": data}, expect=1)


def drop_parts(rpc, token: str) -> None:
    rpc.query("DELETE upload_part WHERE token = $k RETURN NONE;", {"k": token})


def sweep_orphans(rpc) -> None:
    """Parts of an upload that never committed (the pod died mid-body): gone after an hour. Cheap, per upload."""
    rpc.query("DELETE upload_part WHERE t < $old AND token NOT IN (SELECT VALUE token FROM upload) RETURN NONE;",
              {"old": time.time() - ORPHAN_S})


def commit(rpc, name: str, token: str, meta: dict, nbytes: int, parts: int) -> str:
    """Make the `upload:<name>` row, the next free suffix when the name is taken (`<stem>-2.<ext>`); the name used."""
    stem, dot, ext = name.rpartition(".")
    stem, ext = (stem, "." + ext) if dot else (name, "")
    k, use = 2, name
    while True:
        try:
            rpc.write("CREATE type::record('upload', $n) CONTENT {name: $n, token: $k, meta: $m, bytes: $b, parts: $p, t: $t} "
                      "RETURN id;", {"n": use, "k": token, "m": {**meta}, "b": nbytes, "p": parts, "t": time.time()}, expect=1)
            return use
        except Exception as e:  # noqa: BLE001
            if "already exists" not in str(e).lower():
                raise
            use = f"{stem}-{k}{ext}"
            k += 1


def rows(rpc) -> list[dict]:
    """Every upload, newest first (the inbox folder sorted by mtime, newest first): name, bytes, meta, t."""
    return rpc.last("SELECT name, bytes, meta, t FROM upload ORDER BY t DESC, name DESC;")


def count(rpc) -> int:
    return int(rpc.last("RETURN count(SELECT id FROM upload);"))


def get(rpc, name: str) -> dict | None:
    if not NAME_RE.match(name or ""):
        return None
    r = rpc.last("SELECT name, token, bytes, parts, meta FROM ONLY type::record('upload', $n);", {"n": name})
    return r if isinstance(r, dict) else None


def part(rpc, token: str, seq: int) -> bytes:
    r = rpc.last("SELECT VALUE data FROM upload_part WHERE token = $k AND seq = $s LIMIT 1;", {"k": token, "s": seq})
    if not r:
        raise LookupError(f"upload part {seq} is missing")
    return bytes(r[0])


def delete(rpc, name: str) -> None:
    """The upload and its parts (the transcriber's result landed, or a duplicate is thrown away)."""
    row = get(rpc, name)
    if row is None:
        return
    rpc.query("DELETE upload_part WHERE token = $k RETURN NONE; DELETE type::record('upload', $n) RETURN NONE;",
              {"k": row["token"], "n": name})
