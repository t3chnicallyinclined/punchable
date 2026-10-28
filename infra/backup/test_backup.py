"""Unit tests for infra/backup/backup.py (no network, no SurrealDB).
   python -m pytest infra/backup -q
SigV4 is checked against AWS's published vectors: the aws-sig-v4-test-suite (`get-vanilla`,
`get-vanilla-query-order-key-case`) and the worked S3 examples in "Signature Calculations for the Authorization
Header" (GET Object with Range, ListObjects)."""
import gzip
import importlib.util
import os
import sys
from pathlib import Path

import pytest

_spec = importlib.util.spec_from_file_location("pk_backup", Path(__file__).with_name("backup.py"))
B = importlib.util.module_from_spec(_spec)
sys.modules["pk_backup"] = B
_spec.loader.exec_module(B)

SUITE_AK, SUITE_SK = "AKIDEXAMPLE", "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"
S3_AK, S3_SK = "AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
# AWS's published expected signatures (hex HMAC-SHA256 of the vectors' example keys; not secrets)
SIG_SHA256_GET_VANILLA = "5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
SIG_SHA256_QUERY_ORDER = "b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500"
SIG_SHA256_S3_GET_RANGE = "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
SIG_SHA256_S3_LIST = "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"


# ---------------------------------------------------------------------------------------------------------- SigV4
def test_sigv4_suite_get_vanilla():
    h = {"Host": "example.amazonaws.com", "X-Amz-Date": "20150830T123600Z"}
    got = B.sigv4("GET", "/", {}, h, B.EMPTY_SHA256, SUITE_AK, SUITE_SK, "us-east-1", "service", "20150830T123600Z")
    assert got == ("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, "
                   "SignedHeaders=host;x-amz-date, "
                   "Signature=" + SIG_SHA256_GET_VANILLA)


def test_sigv4_suite_query_order():
    h = {"Host": "example.amazonaws.com", "X-Amz-Date": "20150830T123600Z"}
    got = B.sigv4("GET", "/", {"Param2": "value2", "Param1": "value1"}, h, B.EMPTY_SHA256, SUITE_AK, SUITE_SK,
                  "us-east-1", "service", "20150830T123600Z")
    assert got.endswith("Signature=" + SIG_SHA256_QUERY_ORDER)


def test_sigv4_s3_get_object_range():
    h = {"Host": "examplebucket.s3.amazonaws.com", "Range": "bytes=0-9",
         "x-amz-content-sha256": B.EMPTY_SHA256, "x-amz-date": "20130524T000000Z"}
    got = B.sigv4("GET", "/test.txt", {}, h, B.EMPTY_SHA256, S3_AK, S3_SK, "us-east-1", "s3", "20130524T000000Z")
    assert got == ("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, "
                   "SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, "
                   "Signature=" + SIG_SHA256_S3_GET_RANGE)


def test_sigv4_s3_list_objects():
    h = {"Host": "examplebucket.s3.amazonaws.com", "x-amz-content-sha256": B.EMPTY_SHA256,
         "x-amz-date": "20130524T000000Z"}
    got = B.sigv4("GET", "/", {"max-keys": "2", "prefix": "J"}, h, B.EMPTY_SHA256, S3_AK, S3_SK, "us-east-1", "s3",
                  "20130524T000000Z")
    assert got.endswith("Signature=" + SIG_SHA256_S3_LIST)


# ------------------------------------------------------------------------------------------------------ retention
def _k(pfx, kind, day, hour=4):
    return f"{pfx}{kind}/punchable-main-202610{day:02d}T{hour:02d}1700Z.surql.gz"


def test_retention_keeps_newest_daily_and_weekly():
    pfx = "surreal/"
    daily = [_k(pfx, "daily", d) for d in range(1, 31)]          # 30 nightly backups
    weekly = [_k(pfx, "weekly", d) for d in (4, 11, 18, 25)] + \
             [f"{pfx}weekly/punchable-main-2026{m:02d}01T041700Z.surql.gz" for m in range(1, 9)]  # 12 weekly
    keep, delete = B.select_prune(daily + weekly, pfx, "punchable-main", 14, 8)
    assert sorted(k for k in keep if "/daily/" in k) == sorted(daily[-14:])
    assert sorted(k for k in delete if "/daily/" in k) == sorted(daily[:16])
    newest_w = sorted(weekly, key=lambda k: k.rsplit("-", 1)[1])[-8:]
    assert sorted(k for k in keep if "/weekly/" in k) == sorted(newest_w)
    assert len(delete) == 16 + 4
    assert set(keep).isdisjoint(delete)


def test_retention_never_touches_other_keys():
    pfx = "surreal/"
    foreign = [
        "audio/set-1/tape.m4a",                                     # the audio bucket's own objects
        "punchable-main-20260101T000000Z.surql.gz",                 # right name, no prefix
        "surreal-selftest/daily/punchable-main-20260101T000000Z.surql.gz",   # another prefix
        "x/surreal/daily/punchable-main-20260101T000000Z.surql.gz",
        "surreal/daily/notes.txt",                                  # under the prefix but not a backup
        "surreal/daily/punchable-main-20260101T000000Z.surql.gz.part",
        "surreal/daily/../audio/punchable-main-20260101T000000Z.surql.gz",
        "surreal/monthly/punchable-main-20260101T000000Z.surql.gz",
        "surreal/daily/punchable-main_old-20200101T000000Z.surql.gz",       # another database's backups
        "surreal/daily/other-main-20200101T000000Z.surql.gz",
    ]
    ours = [_k(pfx, "daily", d) for d in range(1, 4)]
    keep, delete = B.select_prune(foreign + ours, pfx, "punchable-main", 1, 1)
    assert delete == [_k(pfx, "daily", 2), _k(pfx, "daily", 1)]
    assert keep == [_k(pfx, "daily", 3)]
    assert not set(foreign) & set(keep + delete)


def test_retention_refuses_bad_prefix_and_zero_keep():
    for bad in ("", "/", "audio/", "surreal", "Surreal/", "surreal/../"):
        with pytest.raises(B.BackupError):
            B.select_prune(["audio/x"], bad, "punchable-main", 14, 8)
    with pytest.raises(B.BackupError):
        B.select_prune([], "surreal/", "punchable-main", 0, 8)
    with pytest.raises(B.BackupError):
        B.select_prune([], "surreal/", "../x", 14, 8)


def test_prefix_env(monkeypatch):
    monkeypatch.setenv("BACKUP_PREFIX", "surreal-selftest/")
    assert B.prefix() == "surreal-selftest/"
    monkeypatch.setenv("BACKUP_PREFIX", "audio/")
    with pytest.raises(B.BackupError):
        B.prefix()


# ------------------------------------------------------------------------------------------------- dump checks
SQL = (b"-- ------------------------------\n-- OPTION\n-- ------------------------------\n\nOPTION IMPORT;\n\n"
       b"DEFINE DATABASE OVERWRITE $session.db STRICT;\n"
       b"DEFINE TABLE joke TYPE NORMAL SCHEMAFULL CHANGEFEED 30d INCLUDE ORIGINAL PERMISSIONS NONE;\n"
       b"DEFINE TABLE unit TYPE NORMAL SCHEMAFULL PERMISSIONS NONE;\n"
       + b"".join(b"INSERT [ { id: unit:u%d, text: 'line %d' } ];\n" % (i, i) for i in range(400)))


def _write(tmp_path, data: bytes, name="d.surql.gz") -> str:
    p = tmp_path / name
    p.write_bytes(gzip.compress(data))
    return str(p)


def test_check_dump_accepts_a_good_export(tmp_path):
    info = B.check_dump(_write(tmp_path, SQL), min_bytes=1024)
    assert info["tables"] == 2 and info["bytes_sql"] == len(SQL)


def test_check_dump_counts_tables_across_chunk_boundary(tmp_path, monkeypatch):
    monkeypatch.setattr(B, "CHUNK", 7)
    assert B.check_dump(_write(tmp_path, SQL), min_bytes=1024)["tables"] == 2


def test_check_dump_rejects_truncated_gzip(tmp_path):
    p = _write(tmp_path, SQL)
    raw = Path(p).read_bytes()
    Path(p).write_bytes(raw[: len(raw) // 2])
    with pytest.raises(B.BackupError, match="not a complete gzip"):
        B.check_dump(p, min_bytes=1024)


def test_check_dump_rejects_bad_crc(tmp_path):
    p = _write(tmp_path, SQL)
    raw = bytearray(Path(p).read_bytes())
    raw[-8] ^= 0xFF           # the CRC32 field of the gzip trailer
    Path(p).write_bytes(bytes(raw))
    with pytest.raises(B.BackupError, match="not a complete gzip"):
        B.check_dump(p, min_bytes=1024)


def test_check_dump_rejects_tiny_export(tmp_path):
    with pytest.raises(B.BackupError, match="only"):
        B.check_dump(_write(tmp_path, SQL[:300]), min_bytes=1024)


def test_check_dump_rejects_export_without_tables(tmp_path):
    with pytest.raises(B.BackupError, match="DEFINE TABLE"):
        B.check_dump(_write(tmp_path, SQL.replace(b"DEFINE TABLE", b"DEFINE FIELD")), min_bytes=1024)


def test_check_dump_rejects_cut_statement(tmp_path):
    with pytest.raises(B.BackupError, match="complete statement"):
        B.check_dump(_write(tmp_path, SQL[:-20]), min_bytes=1024)


def test_check_dump_rejects_non_export(tmp_path):
    with pytest.raises(B.BackupError, match="OPTION IMPORT"):
        B.check_dump(_write(tmp_path, b"hello;\n" * 2000), min_bytes=1024)


# ------------------------------------------------------------------------------------------------------ redaction
def test_redact_hides_tokens_and_env_secrets(monkeypatch):
    monkeypatch.setenv("SURREAL_PASS", "hunter2hunter2")
    msg = B.redact("auth failed for hunter2hunter2 sig=" + "a" * 64 + " key AKIAIOSFODNN7EXAMPLE")
    assert "hunter2" not in msg and "a" * 40 not in msg and "<redacted>" in msg


def test_redact_keeps_backup_key_names():
    k = "surreal-selftest/daily/punchable-main-20261027T021247Z.surql.gz"
    assert B.redact(f"uploaded {k}") == f"uploaded {k}"
    assert "<redacted>" in B.redact("x/surreal/daily/" + "q" * 45)


def test_main_prints_one_line_without_secrets(monkeypatch, capsys):
    monkeypatch.setenv("BACKUP_PREFIX", "audio/")
    assert B.main(["list"]) == 1
    err = capsys.readouterr().err.strip().splitlines()
    assert len(err) == 1 and err[0].startswith("backup: FAIL")


# ----------------------------------------------------------------------------------------- the second file: Bugsink
def _sqlite(tmp_path, rows=200):
    import sqlite3
    p = tmp_path / "db.sqlite3"
    c = sqlite3.connect(p)
    c.execute("PRAGMA journal_mode=WAL")
    c.execute("CREATE TABLE event (id INTEGER PRIMARY KEY, body TEXT)")
    c.executemany("INSERT INTO event (body) VALUES (?)", [("x" * 100,)] * rows)
    c.commit()
    return c, p


def test_sqlite_backup_is_a_consistent_copy_that_passes_integrity_check(tmp_path):
    c, p = _sqlite(tmp_path)
    out = str(tmp_path / "copy.sqlite3.gz")
    got = B.sqlite_backup_to_gzip(str(p), out)
    info = B.check_sqlite_gz(out, min_bytes=1024)
    assert got["bytes_db"] == info["bytes_db"] and info["tables"] == 1 and info["bytes_gz"] < info["bytes_db"]
    assert len(info["sha256"]) == 64
    assert not (tmp_path / "copy.sqlite3.gz.db").exists() and not (tmp_path / "copy.sqlite3.gz.check").exists()
    c.close()


def test_sqlite_backup_works_while_a_writer_holds_the_database_open(tmp_path):
    c, p = _sqlite(tmp_path)
    c.execute("INSERT INTO event (body) VALUES ('open transaction')")   # uncommitted: the copy is of the last commit
    out = str(tmp_path / "copy.sqlite3.gz")
    B.sqlite_backup_to_gzip(str(p), out)
    import gzip, sqlite3
    raw = tmp_path / "copy.sqlite3"
    raw.write_bytes(gzip.open(out).read())
    n = sqlite3.connect(raw).execute("SELECT count(*) FROM event").fetchone()[0]
    assert n == 200
    c.rollback()
    c.close()


def test_check_sqlite_gz_rejects_a_truncated_or_foreign_file(tmp_path):
    c, p = _sqlite(tmp_path)
    c.close()
    out = str(tmp_path / "copy.sqlite3.gz")
    B.sqlite_backup_to_gzip(str(p), out)
    data = open(out, "rb").read()
    cut = tmp_path / "cut.sqlite3.gz"
    cut.write_bytes(data[: len(data) // 2])
    with pytest.raises(B.BackupError, match="not a complete gzip"):
        B.check_sqlite_gz(str(cut), min_bytes=1024)
    other = tmp_path / "other.sqlite3.gz"
    other.write_bytes(gzip.compress(b"OPTION IMPORT;\n" + b"x" * 5000))
    with pytest.raises(B.BackupError, match="not a SQLite file"):
        B.check_sqlite_gz(str(other), min_bytes=1024)
    with pytest.raises(B.BackupError, match="no such file"):
        B.sqlite_backup_to_gzip(str(tmp_path / "missing.sqlite3"), out)


def test_bugsink_retention_uses_its_own_prefix_and_extension():
    pfx = "bugsink/"
    keys = [f"bugsink/daily/bugsink-db-2026101{d}T041700Z.sqlite3.gz" for d in range(0, 10)]
    keys += ["bugsink/weekly/bugsink-db-20261005T041700Z.sqlite3.gz", "surreal/daily/punchable-main-20261010T041700Z.surql.gz",
             "bugsink/daily/bugsink-db-20261010T041700Z.surql.gz"]   # wrong extension: never a candidate
    keep, delete = B.select_prune(keys, pfx, "bugsink-db", 3, 8, ext=".sqlite3.gz")
    assert len(keep) == 4 and len(delete) == 7
    assert all(k.startswith("bugsink/daily/") and k.endswith(".sqlite3.gz") for k in delete)
    assert "surreal/daily/punchable-main-20261010T041700Z.surql.gz" not in delete
    assert "bugsink/daily/bugsink-db-20261010T041700Z.surql.gz" not in delete
    with pytest.raises(B.BackupError):
        B.select_prune(keys, "bugsink/", "bugsink-db", 3, 8, ext=".tar")
    with pytest.raises(B.BackupError):
        B.select_prune(keys, "audio/", "bugsink-db", 3, 8, ext=".sqlite3.gz")


def test_bugsink_prefix_env(monkeypatch):
    monkeypatch.setenv("BUGSINK_PREFIX", "bugsink-test/")
    assert B.bugsink_prefix() == "bugsink-test/"
    monkeypatch.setenv("BUGSINK_PREFIX", "surreal/")
    with pytest.raises(B.BackupError):
        B.bugsink_prefix()
    assert B.redact("bugsink/daily/bugsink-db-20261010T041700Z.sqlite3.gz") == "bugsink/daily/bugsink-db-20261010T041700Z.sqlite3.gz"


class _FakeS3:
    def __init__(self):
        self.objects: dict[str, int] = {}
        self.deleted: list[str] = []

    def put_file(self, key, path, sha256):
        self.objects[key] = os.path.getsize(path)

    def head(self, key):
        return self.objects.get(key)

    def copy(self, src, dst):
        self.objects[dst] = self.objects[src]

    def delete(self, key):
        self.deleted.append(key)
        self.objects.pop(key, None)

    def list(self, pfx):
        return [(k, v) for k, v in self.objects.items() if k.startswith(pfx)]


def test_run_sqlite_uploads_verifies_and_prunes(tmp_path, monkeypatch):
    import datetime as dt
    c, p = _sqlite(tmp_path)
    c.close()
    monkeypatch.setenv("BUGSINK_DB_PATH", str(p))
    monkeypatch.setenv("BACKUP_TMP", str(tmp_path))
    monkeypatch.setenv("BACKUP_WEEKLY", "always")
    monkeypatch.setattr(B, "check_sqlite_gz", lambda path, min_bytes=8192: B.__dict__["check_sqlite_gz"].__wrapped__(path, 1024)
                        if hasattr(B.check_sqlite_gz, "__wrapped__") else _orig_check(path, 1024))
    s3 = _FakeS3()
    for i in range(1, 16):   # fifteen old dailies already there
        s3.objects[f"bugsink/daily/bugsink-db-202609{i:02d}T041700Z.sqlite3.gz"] = 10
    now = dt.datetime(2026, 10, 28, 4, 17, tzinfo=dt.UTC)
    B.run_sqlite(s3, now, 14, 8)
    assert "bugsink/daily/bugsink-db-20261028T041700Z.sqlite3.gz" in s3.objects
    assert "bugsink/weekly/bugsink-db-20261028T041700Z.sqlite3.gz" in s3.objects
    assert len([k for k in s3.objects if k.startswith("bugsink/daily/")]) == 14
    assert set(s3.deleted) == {"bugsink/daily/bugsink-db-20260901T041700Z.sqlite3.gz", "bugsink/daily/bugsink-db-20260902T041700Z.sqlite3.gz"}
    assert not list(tmp_path.glob("bugsink-*"))   # the temp copy is gone


_orig_check = B.check_sqlite_gz
