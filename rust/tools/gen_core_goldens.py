"""Goldens for pk-core: what CPython prints for the values `pk_core::py` can be handed, from synthetic data.

`repr`, `str`, `json.dumps` (plain and `sort_keys`), `float.__repr__`, `int()`, `str.isspace`, `str.strip`, `len`,
`list`, `sorted(set(...))`, truthiness and the type name, over floats at every exponent range, the escapes `repr`
writes, and non-ASCII text. The Rust test (rust/crates/pk-core/tests/goldens.rs) runs the same values through pk-core
and compares the texts. Nothing of the comic's is here. Deterministic: no sets are iterated, no clock is read.

    python rust/tools/gen_core_goldens.py        (from the repo root; writes rust/crates/pk-core/tests/goldens/)
"""
from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "rust" / "crates" / "pk-core" / "tests" / "goldens"

FLOATS = [
    0.0, -0.0, 0.5, 1.0, 1.5, 280.0, 100.0, 0.1, 0.1 + 0.2, 2.5e-5, 9.999e-5, 1e-4, 1e-5, 1.5e-7, 1e-10,
    1e15, 1e16, 1e20, 1e100, 123456.789, 1234567890123456.0, 12345678901234567.0, -2.25, -1e-7,
    5e-324, 1.7976931348623157e308, 3.141592653589793, 2.0 ** 53, 1e22, 1e21, 1e-100,
]
INTS = [0, 1, -1, 7, 42, -1000, 2 ** 31, 2 ** 53, 2 ** 63 - 1, -(2 ** 63), 2 ** 64 - 1]
STRINGS = [
    "", "plain", "joke:x", "it's", 'say "hi"', "it's \"both\"", "a\\b", "line\nbreak", "tab\there", "cr\rlf",
    "bell\x07", "esc\x1b", "del\x7f", "sep\x1c\x1d\x1e\x1f", "é", "日本語", "emoji 😀", "nbsp\xa0here",
    "  padded  ", "\x1c lead", "quote 'in' \"both\"",
]
NESTED = [
    None, True, False, [], {}, [1, 2.5, "x", None, True], {"a": 1, "b": None, "c": [1.0, "y"]},
    {"z": 1, "a": {"d": 1e-5, "c": [{"k": "v"}, 1e20]}}, [[[]]], {"": ""}, {"k": "it's"}, ["é", "日本語", "😀"],
    {"n": -0.0, "big": 2 ** 63 - 1, "u": 2 ** 64 - 1},
]
INT_INPUTS = [
    " 12 ", "3.9", "1_0", "1__0", "_1", "1_", "+7", "-7", "", "abc", "0x10", "\x1c5\x1f", 3.9, -3.9, True, False, 7,
    None, [], {},
]
# no mixed str/int set: which pair `sorted` compares first follows the set's order, which follows PYTHONHASHSEED
SORTED_SET_INPUTS = [
    [], ["b", "a", "b"], [2, 1.5, 2], [[1]], [{"k": 1}], ["x"], [-1, 3, -1.5, 3],
]


def values():
    return [*NESTED, *FLOATS, *INTS, *STRINGS]


def int_case(v):
    try:
        return {"ok": int(v)}
    except (ValueError, TypeError) as e:
        return {"err": type(e).__name__, "msg": str(e)}


def sorted_set_case(xs):
    try:
        return {"ok": sorted(set(xs))}
    except TypeError as e:
        return {"err": "TypeError", "msg": str(e)}


def list_case(v):
    try:
        return {"ok": list(v)}
    except TypeError as e:
        return {"err": "TypeError", "msg": str(e)}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    golden = {
        "repr": [[v, repr(v)] for v in values()],
        "str": [[v, str(v)] for v in values()],
        "float_repr": [[f, repr(f)] for f in FLOATS],
        "dumps": [[v, json.dumps(v, ensure_ascii=False)] for v in values()],
        "dumps_sorted": [[v, json.dumps(v, sort_keys=True, ensure_ascii=False)] for v in NESTED],
        "int": [[v, int_case(v)] for v in INT_INPUTS],
        "isspace": [[c, c.isspace()] for c in " \t\n\r\x0b\x0c\x1c\x1d\x1e\x1f\xa0 　a0_\x1b"],
        "strip": [[s, s.strip()] for s in STRINGS],
        "len": [[s, len(s)] for s in STRINGS],
        "list": [[v, list_case(v)] for v in [[], [1, "a"], "héllo", {"b": 1, "a": 2}, 1, None, 1.5, True]],
        "sorted_set": [[xs, sorted_set_case(xs)] for xs in SORTED_SET_INPUTS],
        "truthy": [[v, bool(v)] for v in values()],
        "type_name": [[v, type(v).__name__] for v in values()],
    }
    (OUT / "py.json").write_text(json.dumps(golden, ensure_ascii=False, indent=1), encoding="utf-8")
    print("wrote", OUT, "values", len(values()), "floats", len(FLOATS), "ints", len(INT_INPUTS))


if __name__ == "__main__":
    main()
