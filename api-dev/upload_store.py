"""Uploads staged in the database (hosted; db/schema_extra.surql `upload`, `upload_part`).

The hosted pod used to keep an upload in its inbox folder until a transcriber had processed it, which only the
Python could read. Now the file is rows both API servers see (pk-api answers /api/upload, /api/inbox and
/api/worker/jobs/<id>/file too): `upload_part` rows of at most PART bytes keyed by a private `token`, and the
`upload:<file name>` row, made LAST, which names the token and holds the meta.json. A name is reserved by that
CREATE (the unique record id), so two uploads in the same second never share one; a refused CREATE tries the next
suffix. Local development (no jobs_on_upload) keeps the inbox folder and none of this.
"""
from __future__ import annotations

import os
import re
import time

PART = 1024 * 1024


def enabled() -> bool:
    """Uploads in the database (docs/adr/0014, Proposed) are OFF unless PK_UPLOAD_DB=1: off, a hosted upload is a file in
    the pod's inbox folder as before and pk-api refuses to take /api/upload."""
    return os.environ.get("PK_UPLOAD_DB") == "1"
ORPHAN_S = 3600
NAME_RE = re.compile(r"^[A-Za-z0-9._\-]{1,200}$")


def put_part(rpc, token: str, seq: int, data: bytes) -> None:
    """One part and the staging row that counts it, in one transaction."""
    rpc.query("BEGIN; CREATE upload_part CONTENT {token: $k, seq: $s, data: $d} RETURN id; "
              "UPSERT type::record('upload_stage', $k) SET t = $t, n = (n ?? 0) + 1 RETURN id; COMMIT;",
              {"k": token, "s": seq, "t": time.time(), "d": data})


def drop_parts(rpc, token: str) -> None:
    rpc.query("DELETE upload_part WHERE token = $k RETURN NONE; DELETE type::record('upload_stage', $k) RETURN NONE;",
              {"k": token})


def sweep_orphans(rpc) -> None:
    """Parts of an upload that never committed (or whose row was deleted first), found by their staging row's newest-part
    time, never by scanning the megabyte rows. A token that did commit only loses its staging row."""
    rpc.query("LET $stale = (SELECT VALUE record::id(id) FROM upload_stage WHERE t < $old); "
              "LET $kept = (SELECT VALUE token FROM upload WHERE token IN $stale); "
              "FOR $k IN $stale { IF $k NOT IN $kept { DELETE upload_part WHERE token = $k RETURN NONE; }; }; "
              "DELETE upload_stage WHERE t < $old RETURN NONE;", {"old": time.time() - ORPHAN_S})


def commit(rpc, name: str, token: str, meta: dict, nbytes: int, parts: int) -> str:
    """Make the `upload:<name>` row, the next free suffix when the name is taken (`<stem>-2.<ext>`); the name used."""
    counted = rpc.last("SELECT VALUE n FROM ONLY type::record('upload_stage', $k);", {"k": token})
    if counted != parts:
        raise RuntimeError(f"upload {name}: {parts} parts expected, {counted!r} written")
    stem, dot, ext = name.rpartition(".")
    stem, ext = (stem, "." + ext) if dot else (name, "")
    k, use = 2, name
    while True:
        try:
            rpc.write("CREATE type::record('upload', $n) CONTENT {name: $n, token: $k, meta: $m, bytes: $b, parts: $p, t: $t} "
                      "RETURN id;", {"n": use, "k": token, "m": {**meta}, "b": nbytes, "p": parts, "t": time.time()}, expect=1)
            rpc.query("DELETE type::record('upload_stage', $k) RETURN NONE;", {"k": token})
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
    """The upload and its parts: the row first (a stale staging row marks the token for the sweep in the same
    transaction), then the bytes, so a crash between the two leaves orphans the sweep removes, never an inbox row with no
    bytes."""
    row = get(rpc, name)
    if row is None:
        return
    rpc.query("BEGIN; DELETE type::record('upload', $n) RETURN NONE; "
              "UPSERT type::record('upload_stage', $k) SET t = 0.0, n = 0 RETURN id; COMMIT;",
              {"k": row["token"], "n": name})
    drop_parts(rpc, row["token"])
