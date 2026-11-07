"""upload_store.py: an upload held in the database (hosted, PK_UPLOAD_DB=1; docs/adr/0014). The real rows are proven
against a scratch SurrealDB by rust/tools/diff_api_writes.py (phase U) and the Rust scratch tests; this is the logic
around them, over a fake connection."""
import upload_store as US


class FakeRpc:
    """Just enough of sdb.Rpc: record ids that exist refuse a second CREATE, as the unique record id does; a part
    also counts itself on its staging row."""

    def __init__(self):
        self.uploads = {}
        self.parts = []
        self.stage = {}
        self.sql = []

    def write(self, sql, vars=None, expect=None):
        self.sql.append(sql)
        if sql.startswith("CREATE type::record('upload'"):
            if vars["n"] in self.uploads:
                raise RuntimeError(f"Database record `upload:{vars['n']}` already exists")
            self.uploads[vars["n"]] = vars
            return [{"id": vars["n"]}]
        raise AssertionError(sql)

    def query(self, sql, vars=None):
        self.sql.append(sql)
        if sql.startswith("BEGIN; CREATE upload_part"):
            self.parts.append({"k": vars["k"], "s": vars["s"]})
            self.stage[vars["k"]] = self.stage.get(vars["k"], 0) + 1
        elif sql.startswith(("DELETE upload_part WHERE token = $k", "BEGIN; DELETE upload_part WHERE token = $k")):
            self.parts = [p for p in self.parts if p["k"] != vars["k"]]
            self.stage.pop(vars["k"], None)
        elif sql.startswith("DELETE type::record('upload_stage'"):
            self.stage.pop(vars["k"], None)
        elif sql.startswith("BEGIN; DELETE type::record('upload'"):
            self.uploads.pop(vars["n"], None)
            self.stage[vars["k"]] = 0   # the stale staging row that marks the token for the sweep
        return []

    def last(self, sql, vars=None):
        self.sql.append(sql)
        if sql.startswith("SELECT name, token, bytes, parts, meta FROM ONLY"):
            u = self.uploads.get(vars["n"])
            return {"name": u["n"], "token": u["k"], "bytes": u["b"], "parts": u["p"], "meta": u["m"]} if u else None
        if sql.startswith("SELECT VALUE n FROM ONLY type::record('upload_stage'"):
            return self.stage.get(vars["k"])
        raise AssertionError(sql)


def put(r, token, n):
    for i in range(n):
        US.put_part(r, token, i, b"x")


def test_a_taken_name_gets_the_next_suffix():
    r = FakeRpc()
    names = []
    for i in range(3):
        put(r, f"t{i}", 1)
        names.append(US.commit(r, "2026-11-06_punchable-set_101010_take.m4a", f"t{i}", {}, 1, 1))
    assert names == ["2026-11-06_punchable-set_101010_take.m4a", "2026-11-06_punchable-set_101010_take-2.m4a",
                     "2026-11-06_punchable-set_101010_take-3.m4a"]
    assert r.stage == {}, "a commit takes its staging row with it"


def test_a_commit_counts_the_parts():
    r = FakeRpc()
    put(r, "t1", 2)
    try:
        US.commit(r, "a.m4a", "t1", {}, 2, 3)
    except RuntimeError as e:
        assert "3 parts expected" in str(e)
    else:
        raise AssertionError("an upload with a part missing must not be listed")
    assert r.uploads == {}


def test_another_error_is_not_swallowed():
    class Broken(FakeRpc):
        def write(self, sql, vars=None, expect=None):
            raise RuntimeError("the database is down")

    r = Broken()
    put(r, "t", 1)
    try:
        US.commit(r, "a.m4a", "t", {}, 1, 1)
    except RuntimeError as e:
        assert "down" in str(e)
    else:
        raise AssertionError("a failure other than 'already exists' must surface")


def test_only_a_plain_file_name_is_looked_up():
    r = FakeRpc()
    assert US.get(r, "../etc/passwd") is None and US.get(r, "") is None and US.get(r, "a/b.m4a") is None
    assert r.sql == []


def test_delete_takes_the_row_first_and_the_parts_of_that_token_only():
    r = FakeRpc()
    put(r, "t1", 1)
    put(r, "t2", 1)
    US.commit(r, "x.m4a", "t1", {}, 1, 1)
    r.sql.clear()
    US.delete(r, "x.m4a")
    assert [p["k"] for p in r.parts] == ["t2"]
    row_first = next(i for i, q in enumerate(r.sql) if q.startswith("BEGIN; DELETE type::record('upload'"))
    parts_after = next(i for i, q in enumerate(r.sql) if "DELETE upload_part WHERE token = $k" in q and "type::record('upload'" not in q)
    assert row_first < parts_after, "the row goes before the bytes"
    US.delete(r, "x.m4a")   # again: nothing there, nothing raised


def test_the_flag_is_off_unless_it_is_one(monkeypatch):
    monkeypatch.delenv("PK_UPLOAD_DB", raising=False)
    assert US.enabled() is False
    monkeypatch.setenv("PK_UPLOAD_DB", "0")
    assert US.enabled() is False
    monkeypatch.setenv("PK_UPLOAD_DB", "1")
    assert US.enabled() is True
