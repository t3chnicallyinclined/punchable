"""upload_store.py: an upload held in the database (hosted). The real rows are proven against a scratch SurrealDB by
rust/tools/diff_api_writes.py (phase U); this is the logic around them, over a fake connection."""
import upload_store as US


class FakeRpc:
    """Just enough of sdb.Rpc: record ids that exist refuse a second CREATE, as the unique record id does."""

    def __init__(self):
        self.uploads = {}
        self.parts = []
        self.sql = []

    def write(self, sql, vars=None, expect=None):
        self.sql.append(sql)
        if sql.startswith("CREATE type::record('upload'"):
            if vars["n"] in self.uploads:
                raise RuntimeError(f"Database record `upload:{vars['n']}` already exists")
            self.uploads[vars["n"]] = vars
            return [{"id": vars["n"]}]
        if sql.startswith("CREATE upload_part"):
            self.parts.append(vars)
            return [{"id": len(self.parts)}]
        raise AssertionError(sql)

    def query(self, sql, vars=None):
        self.sql.append(sql)
        if sql.startswith("DELETE upload_part WHERE token = $k"):
            self.parts = [p for p in self.parts if p["k"] != vars["k"]]
        return []

    def last(self, sql, vars=None):
        self.sql.append(sql)
        if sql.startswith("SELECT name, token, bytes, parts, meta FROM ONLY"):
            u = self.uploads.get(vars["n"])
            return {"name": u["n"], "token": u["k"], "bytes": u["b"], "parts": u["p"], "meta": u["m"]} if u else None
        raise AssertionError(sql)


def test_a_taken_name_gets_the_next_suffix():
    r = FakeRpc()
    names = [US.commit(r, "2026-11-06_punchable-set_101010_take.m4a", f"t{i}", {}, 1, 1) for i in range(3)]
    assert names == ["2026-11-06_punchable-set_101010_take.m4a", "2026-11-06_punchable-set_101010_take-2.m4a",
                     "2026-11-06_punchable-set_101010_take-3.m4a"]


def test_another_error_is_not_swallowed():
    class Broken(FakeRpc):
        def write(self, sql, vars=None, expect=None):
            raise RuntimeError("the database is down")

    try:
        US.commit(Broken(), "a.m4a", "t", {}, 1, 1)
    except RuntimeError as e:
        assert "down" in str(e)
    else:
        raise AssertionError("a failure other than 'already exists' must surface")


def test_only_a_plain_file_name_is_looked_up():
    r = FakeRpc()
    assert US.get(r, "../etc/passwd") is None and US.get(r, "") is None and US.get(r, "a/b.m4a") is None
    assert r.sql == []


def test_delete_takes_the_parts_of_that_token_only():
    r = FakeRpc()
    US.put_part(r, "t1", 0, b"a")
    US.put_part(r, "t2", 0, b"b")
    US.commit(r, "x.m4a", "t1", {}, 1, 1)
    US.delete(r, "x.m4a")
    assert [p["k"] for p in r.parts] == ["t2"]
    US.delete(r, "x.m4a")   # again: nothing there, nothing raised
