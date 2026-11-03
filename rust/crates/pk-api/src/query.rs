//! The query string the way the Python reads it (`urllib.parse.parse_qs`, blank values dropped) and the two helpers
//! every route uses: `arg(k, default)` (the first value) and Python's `int()` over it, whose `ValueError` text the
//! route answers with a 400.

use crate::error::ApiError;

/// The decoded `key=value` pairs, in order, without the ones whose value is blank (`parse_qs` default).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pairs: Vec<(String, String)>,
}

impl Query {
    /// Parse a raw query string (no leading `?`): split on `&` only, `+` is a space, percent-decoding replaces
    /// invalid UTF-8, a piece without `=` or with an empty value is skipped.
    pub fn parse(raw: Option<&str>) -> Self {
        let pairs = raw
            .map(|r| {
                form_urlencoded::parse(r.as_bytes())
                    .filter(|(_, v)| !v.is_empty())
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect()
            })
            .unwrap_or_default();
        Self { pairs }
    }

    /// The first value of `key` (`q.get(k, [None])[0]`).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// Every value of `key`, in order (`q.get(k, [])`).
    pub fn all(&self, key: &str) -> Vec<&str> {
        self.pairs
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// `int(arg(key, default))`: Python's `int()` of the first value, or of `default` when there is none. A literal
    /// past `i64` (Python's `int` has no bound) saturates (`pk_core::py::int_saturating`): every caller clamps it or
    /// slices with it, as the Python's `min` / `max` / slices do with the big value; only the Bits list's `offset`
    /// echo can differ (the differential's ALLOW says so).
    pub fn int(&self, key: &str, default: &str) -> Result<i64, ApiError> {
        Ok(pk_core::py::int_saturating(self.get(key).unwrap_or(default))?)
    }

    /// `?include_hidden=1|true|yes`.
    pub fn include_hidden(&self) -> bool {
        matches!(self.get("include_hidden"), Some("1" | "true" | "yes"))
    }
}

/// Python's slice bounds for `xs[start:stop]` over a list of `len` items, with negative indexes counted from the
/// end and both clamped (`slice.indices`).
pub fn py_slice(len: usize, start: Option<i64>, stop: Option<i64>) -> std::ops::Range<usize> {
    let n = i64::try_from(len).unwrap_or(i64::MAX);
    let fix = |i: i64| -> usize {
        let j = if i < 0 { (i + n).max(0) } else { i.min(n) };
        usize::try_from(j).unwrap_or(0)
    };
    let a = start.map_or(0, fix);
    let b = stop.map_or(len, fix);
    a..b.max(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_values_and_bare_names_are_dropped_as_parse_qs_does() {
        let q = Query::parse(Some("a=&b&c=1&c=2&d=x+y%20z&backend="));
        assert_eq!(q.get("a"), None);
        assert_eq!(q.get("b"), None);
        assert_eq!(q.get("backend"), None);
        assert_eq!(q.all("c"), vec!["1", "2"]);
        assert_eq!(q.get("d"), Some("x y z"));
        assert_eq!(Query::parse(Some("x=1;y=2")).get("x"), Some("1;y=2"));
    }

    #[test]
    fn int_answers_with_the_python_value_error() {
        let q = Query::parse(Some("limit=abc&n=%2012%20&u=1_0"));
        assert_eq!(
            q.int("limit", "30").unwrap_err().msg,
            "invalid literal for int() with base 10: 'abc'"
        );
        assert_eq!(q.int("n", "0").unwrap(), 12);
        assert_eq!(q.int("u", "0").unwrap(), 10);
        assert_eq!(q.int("missing", "30").unwrap(), 30);
        let big = Query::parse(Some(
            "a=99999999999999999999&b=-1_000_000_000_000_000_000_000&c=9x99999999999999999999",
        ));
        assert_eq!(
            big.int("a", "0").unwrap(),
            i64::MAX,
            "Python's int() has no bound: saturate"
        );
        assert_eq!(big.int("b", "0").unwrap(), i64::MIN);
        assert_eq!(
            big.int("c", "0").unwrap_err().msg,
            "invalid literal for int() with base 10: '9x99999999999999999999'"
        );
    }

    #[test]
    fn slices_follow_python() {
        assert_eq!(py_slice(10, None, Some(3)), 0..3);
        assert_eq!(py_slice(10, None, Some(-3)), 0..7);
        assert_eq!(py_slice(10, Some(-3), None), 7..10);
        assert_eq!(py_slice(10, Some(0), None), 0..10);
        assert_eq!(py_slice(10, Some(5), None), 5..10);
        assert_eq!(py_slice(2, None, Some(-5)), 0..0);
        assert_eq!(py_slice(2, Some(-50), None), 0..2);
    }
}
