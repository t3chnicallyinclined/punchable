//! The settings: environment variables only, read the way `backup.py` reads `os.environ`.
//!
//! [`Vars`] is a snapshot the binary takes of the process environment (or a test builds by hand). An empty value is
//! *set*, as in Python, where it falls through `get(name, default)` as `""` and then fails the `env()` check.

use std::collections::BTreeMap;

use regex::Regex;

use crate::error::{BackupError, Result, fail};

/// A snapshot of the environment.
#[derive(Debug, Clone, Default)]
pub struct Vars(BTreeMap<String, String>);

impl Vars {
    /// From pairs (tests, and the binary after reading the process environment).
    pub fn from_pairs<K: Into<String>, V: Into<String>>(pairs: impl IntoIterator<Item = (K, V)>) -> Self {
        Self(pairs.into_iter().map(|(k, v)| (k.into(), v.into())).collect())
    }

    /// `os.environ.get(name)`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// `os.environ.get(name, default)`.
    pub fn get_or<'a>(&'a self, name: &str, default: &'a str) -> &'a str {
        self.get(name).unwrap_or(default)
    }

    /// Python truthiness of `os.environ.get(name)`: set and not empty.
    pub fn truthy(&self, name: &str) -> bool {
        self.get(name).is_some_and(|v| !v.is_empty())
    }

    /// `env(name, default)`: the value, or `"<name> is not set"` when it is missing or empty.
    pub fn env(&self, name: &str, default: Option<&str>) -> Result<String> {
        match self.get(name).or(default) {
            Some(v) if !v.is_empty() => Ok(v.to_string()),
            _ => fail(format!("{name} is not set")),
        }
    }

    /// `int_env(name, default, lo)`: an integer of at least `lo`.
    pub fn int(&self, name: &str, default: i64, lo: i64) -> Result<i64> {
        let raw = self.get(name).map_or_else(|| default.to_string(), str::to_string);
        let n: i64 = raw
            .trim()
            .parse()
            .map_err(|_| BackupError(format!("{name} must be an integer")))?;
        if n < lo {
            return fail(format!("{name} must be >= {lo}"));
        }
        Ok(n)
    }
}

/// `BACKUP_PREFIX`: `surreal/` or `surreal-<word>/`; nothing outside it is ever deleted.
pub fn prefix(v: &Vars) -> Result<String> {
    let p = v.get_or("BACKUP_PREFIX", "surreal/");
    if !fullmatch(r"surreal[a-z0-9-]*/", p) {
        return fail(format!(
            "BACKUP_PREFIX must look like 'surreal/' or 'surreal-<word>/' (got {})",
            py_repr(p)
        ));
    }
    Ok(p.to_string())
}

/// `BUGSINK_PREFIX`: `bugsink/` or `bugsink-<word>/`.
pub fn bugsink_prefix(v: &Vars) -> Result<String> {
    let p = v.get_or("BUGSINK_PREFIX", "bugsink/");
    if !fullmatch(r"bugsink[a-z0-9-]*/", p) {
        return fail(format!(
            "BUGSINK_PREFIX must look like 'bugsink/' or 'bugsink-<word>/' (got {})",
            py_repr(p)
        ));
    }
    Ok(p.to_string())
}

/// `re.fullmatch(pattern, text)`.
pub fn fullmatch(pattern: &str, text: &str) -> bool {
    Regex::new(&format!("^(?:{pattern})$")).is_ok_and(|r| r.is_match(text))
}

/// Python's `repr` of a string, as far as the messages need it (single quotes).
fn py_repr(s: &str) -> String {
    if s.contains('\'') && !s.contains('"') {
        format!("\"{s}\"")
    } else {
        format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_env() {
        let v = Vars::from_pairs([("BACKUP_PREFIX", "surreal-selftest/")]);
        assert_eq!(prefix(&v).unwrap(), "surreal-selftest/");
        let v = Vars::from_pairs([("BACKUP_PREFIX", "audio/")]);
        assert!(prefix(&v).is_err());
        assert_eq!(prefix(&Vars::default()).unwrap(), "surreal/");
    }

    #[test]
    fn bugsink_prefix_env() {
        let v = Vars::from_pairs([("BUGSINK_PREFIX", "bugsink-test/")]);
        assert_eq!(bugsink_prefix(&v).unwrap(), "bugsink-test/");
        let v = Vars::from_pairs([("BUGSINK_PREFIX", "surreal/")]);
        assert!(bugsink_prefix(&v).is_err());
    }

    #[test]
    fn an_empty_value_is_set_but_refused() {
        let v = Vars::from_pairs([("R2_BUCKET", "")]);
        assert_eq!(v.env("R2_BUCKET", None).unwrap_err().0, "R2_BUCKET is not set");
        assert_eq!(v.env("R2_REGION", Some("auto")).unwrap(), "auto");
        assert!(
            Vars::from_pairs([("R2_REGION", "")])
                .env("R2_REGION", Some("auto"))
                .is_err()
        );
    }

    #[test]
    fn ints() {
        let v = Vars::from_pairs([("A", "7"), ("B", "x"), ("C", "0"), ("D", " 9 ")]);
        assert_eq!(v.int("A", 1, 1).unwrap(), 7);
        assert_eq!(v.int("B", 1, 1).unwrap_err().0, "B must be an integer");
        assert_eq!(v.int("C", 1, 1).unwrap_err().0, "C must be >= 1");
        assert_eq!(v.int("D", 1, 1).unwrap(), 9);
        assert_eq!(v.int("Z", 14, 1).unwrap(), 14);
    }
}
