//! The differential against `CPython`: `rust/tools/gen_core_goldens.py` prints synthetic values with `repr`, `str`,
//! `json.dumps`, `float.__repr__`, `int()`, `str.isspace`, `str.strip`, `len`, `list`, `sorted(set(...))`, `bool` and
//! `type(...).__name__`; these tests hand the same values to `pk_core::py` and compare the texts.

use std::path::PathBuf;

use pk_core::py::{self, PyError};
use serde_json::{Value, json};

fn golden() -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/goldens/py.json");
    let text = std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("{}: {e} (run python rust/tools/gen_core_goldens.py)", p.display()));
    serde_json::from_str(&text).expect("golden is JSON")
}

fn rows<'a>(g: &'a Value, key: &str) -> impl Iterator<Item = (&'a Value, &'a Value)> {
    g[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key}: rows"))
        .iter()
        .map(|row| (&row[0], &row[1]))
}

fn text(v: &Value) -> &str {
    v.as_str().expect("a text")
}

/// What the Python wrote for a raised exception: `{"err": class, "msg": text}` becomes the matching [`PyError`].
fn expected_error(v: &Value) -> Option<PyError> {
    let msg = v.get("msg")?.as_str()?.to_string();
    match v["err"].as_str()? {
        "ValueError" => Some(PyError::Value(msg)),
        "TypeError" => Some(PyError::Type(msg)),
        other => panic!("unexpected exception class {other}"),
    }
}

#[test]
fn repr_and_str_match_cpython() {
    let g = golden();
    for (v, want) in rows(&g, "repr") {
        assert_eq!(py::repr(v), text(want), "repr of {v}");
    }
    for (v, want) in rows(&g, "str") {
        assert_eq!(py::str(v), text(want), "str of {v}");
    }
    for (v, want) in rows(&g, "type_name") {
        assert_eq!(py::type_name(v), text(want), "type of {v}");
    }
    for (v, want) in rows(&g, "truthy") {
        assert_eq!(py::truthy(Some(v)), want.as_bool().expect("bool"), "bool of {v}");
    }
}

#[test]
fn floats_print_as_python_does() {
    let g = golden();
    let floats = g["float_repr"].as_array().expect("rows");
    assert!(floats.len() >= 30, "the golden covers every exponent range");
    for (f, want) in rows(&g, "float_repr") {
        assert_eq!(py::float_repr(f.as_f64().expect("a float")), text(want), "repr of {f}");
    }
}

#[test]
fn dumps_matches_json_dumps() {
    let g = golden();
    for (v, want) in rows(&g, "dumps") {
        assert_eq!(py::dumps(v), text(want), "dumps of {v}");
    }
    for (v, want) in rows(&g, "dumps_sorted") {
        assert_eq!(py::dumps_sorted(v), text(want), "sorted dumps of {v}");
    }
}

#[test]
fn int_matches_cpython_including_its_errors() {
    let g = golden();
    for (v, want) in rows(&g, "int") {
        let got = py::int(Some(v));
        match (got, want.get("ok"), expected_error(want)) {
            (Ok(n), Some(ok), _) => assert_eq!(json!(n), *ok, "int of {v}"),
            (Err(e), None, Some(want_e)) => assert_eq!(e, want_e, "int of {v}"),
            (got, _, _) => panic!("int of {v}: rust {got:?}, python {want}"),
        }
    }
}

#[test]
fn strings_match_cpython() {
    let g = golden();
    for (c, want) in rows(&g, "isspace") {
        let c = text(c).chars().next().expect("one char");
        assert_eq!(py::isspace(c), want.as_bool().expect("bool"), "isspace of {c:?}");
    }
    for (s, want) in rows(&g, "strip") {
        assert_eq!(py::strip(text(s)), text(want), "strip of {s}");
    }
    for (s, want) in rows(&g, "len") {
        assert_eq!(py::len(text(s)), want.as_u64().expect("len") as usize, "len of {s}");
    }
}

#[test]
fn list_and_sorted_set_match_cpython() {
    let g = golden();
    for (v, want) in rows(&g, "list") {
        match (py::list(v), want.get("ok"), expected_error(want)) {
            (Ok(xs), Some(ok), _) => assert_eq!(json!(xs), *ok, "list of {v}"),
            (Err(e), None, Some(want_e)) => assert_eq!(e, want_e, "list of {v}"),
            (got, _, _) => panic!("list of {v}: rust {got:?}, python {want}"),
        }
    }
    for (xs, want) in rows(&g, "sorted_set") {
        let xs = xs.as_array().expect("a list");
        match (py::sorted_set(xs), want.get("ok"), expected_error(want)) {
            (Ok(out), Some(ok), _) => assert_eq!(json!(out), *ok, "sorted set of {xs:?}"),
            (Err(e), None, Some(want_e)) => assert_eq!(e, want_e, "sorted set of {xs:?}"),
            (got, _, _) => panic!("sorted set of {xs:?}: rust {got:?}, python {want}"),
        }
    }
}
