//! Retention: which keys stay and which go. Pure: a list of key names in, two lists out, so it can be compared with
//! the Python's answer over any list of names.

use regex::Regex;

use crate::error::{BackupError, Result, fail};
use crate::settings::fullmatch;

/// The `.surql.gz` extension of `SurrealDB` exports.
pub const SURQL_EXT: &str = ".surql.gz";
/// The `.sqlite3.gz` extension of Bugsink copies.
pub const SQLITE_EXT: &str = ".sqlite3.gz";

fn key_re(pfx: &str, kind: &str, name: &str, ext: &str) -> Regex {
    Regex::new(&format!(
        "^{}{kind}/{}-([0-9]{{8}}T[0-9]{{6}}Z){}$",
        regex::escape(pfx),
        regex::escape(name),
        regex::escape(ext)
    ))
    .expect("key regex")
}

/// `(keep, delete)`. Only keys that are exactly `<pfx>daily|weekly/<name>-<stamp><ext>` (name = `<ns>-<db>`, or
/// `bugsink-db`) are ever candidates; anything else, under or outside the prefix (another database's backups too), is
/// left alone. `delete` lists the oldest of each kind last, as the Python does (newest first within a kind).
pub fn select_prune(
    keys: &[String],
    pfx: &str,
    name: &str,
    keep_daily: i64,
    keep_weekly: i64,
    ext: &str,
) -> Result<(Vec<String>, Vec<String>)> {
    if !fullmatch(r"(surreal|bugsink)[a-z0-9-]*/", pfx) {
        return fail(format!("refusing to prune with prefix {}", py_repr(pfx)));
    }
    if !fullmatch(r"[a-z0-9_]+-[a-z0-9_]+", name) {
        return fail(format!("refusing to prune with name {}", py_repr(name)));
    }
    if ext != SURQL_EXT && ext != SQLITE_EXT {
        return fail(format!("refusing to prune with extension {}", py_repr(ext)));
    }
    if keep_daily < 1 || keep_weekly < 1 {
        return fail("retention must keep at least one daily and one weekly");
    }
    let (mut keep, mut delete) = (Vec::new(), Vec::new());
    for (kind, n) in [("daily", keep_daily), ("weekly", keep_weekly)] {
        let rx = key_re(pfx, kind, name, ext);
        let mut mine: Vec<(String, &String)> = keys
            .iter()
            .filter_map(|k| rx.captures(k).map(|c| (c[1].to_string(), k)))
            .collect();
        mine.sort();
        mine.reverse(); // newest first
        let n = usize::try_from(n).unwrap_or(usize::MAX);
        for (i, (_, k)) in mine.into_iter().enumerate() {
            if i < n {
                keep.push(k.clone());
            } else {
                delete.push(k.clone());
            }
        }
    }
    for k in &delete {
        // belt and braces: the regex already guarantees this
        if !(k.starts_with(pfx) && fullmatch(r"(surreal|bugsink)[a-z0-9-]*/.*", k) && !k.contains("..")) {
            return Err(BackupError(format!("AssertionError: {k}")));
        }
    }
    Ok((keep, delete))
}

fn py_repr(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(pfx: &str, kind: &str, day: u32, hour: u32) -> String {
        format!("{pfx}{kind}/punchable-main-202610{day:02}T{hour:02}1700Z.surql.gz")
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn keeps_newest_daily_and_weekly() {
        let pfx = "surreal/";
        let daily: Vec<String> = (1..=30).map(|d| k(pfx, "daily", d, 4)).collect();
        let mut weekly: Vec<String> = [4, 11, 18, 25].iter().map(|d| k(pfx, "weekly", *d, 4)).collect();
        weekly.extend((1..=8).map(|m| format!("{pfx}weekly/punchable-main-2026{m:02}01T041700Z.surql.gz")));
        let all: Vec<String> = daily.iter().chain(&weekly).cloned().collect();
        let (keep, delete) = select_prune(&all, pfx, "punchable-main", 14, 8, SURQL_EXT).unwrap();
        let mut kd: Vec<_> = keep.iter().filter(|x| x.contains("/daily/")).cloned().collect();
        kd.sort();
        assert_eq!(kd, daily[16..].to_vec());
        let mut dd: Vec<_> = delete.iter().filter(|x| x.contains("/daily/")).cloned().collect();
        dd.sort();
        assert_eq!(dd, daily[..16].to_vec());
        let mut newest_w = weekly.clone();
        newest_w.sort_by_key(|x| x.rsplit_once('-').map(|(_, t)| t.to_string()));
        let mut kw: Vec<_> = keep.iter().filter(|x| x.contains("/weekly/")).cloned().collect();
        kw.sort();
        let mut want = newest_w[4..].to_vec();
        want.sort();
        assert_eq!(kw, want);
        assert_eq!(delete.len(), 16 + 4);
        assert!(keep.iter().all(|x| !delete.contains(x)));
    }

    #[test]
    fn never_touches_other_keys() {
        let pfx = "surreal/";
        let foreign = s(&[
            "audio/set-1/tape.m4a",
            "punchable-main-20260101T000000Z.surql.gz",
            "surreal-selftest/daily/punchable-main-20260101T000000Z.surql.gz",
            "x/surreal/daily/punchable-main-20260101T000000Z.surql.gz",
            "surreal/daily/notes.txt",
            "surreal/daily/punchable-main-20260101T000000Z.surql.gz.part",
            "surreal/daily/../audio/punchable-main-20260101T000000Z.surql.gz",
            "surreal/monthly/punchable-main-20260101T000000Z.surql.gz",
            "surreal/daily/punchable-main_old-20200101T000000Z.surql.gz",
            "surreal/daily/other-main-20200101T000000Z.surql.gz",
        ]);
        let ours: Vec<String> = (1..=3).map(|d| k(pfx, "daily", d, 4)).collect();
        let all: Vec<String> = foreign.iter().chain(&ours).cloned().collect();
        let (keep, delete) = select_prune(&all, pfx, "punchable-main", 1, 1, SURQL_EXT).unwrap();
        assert_eq!(delete, vec![k(pfx, "daily", 2, 4), k(pfx, "daily", 1, 4)]);
        assert_eq!(keep, vec![k(pfx, "daily", 3, 4)]);
        assert!(foreign.iter().all(|f| !keep.contains(f) && !delete.contains(f)));
    }

    #[test]
    fn refuses_bad_prefix_and_zero_keep() {
        for bad in ["", "/", "audio/", "surreal", "Surreal/", "surreal/../"] {
            assert!(
                select_prune(&s(&["audio/x"]), bad, "punchable-main", 14, 8, SURQL_EXT).is_err(),
                "{bad}"
            );
        }
        assert!(select_prune(&[], "surreal/", "punchable-main", 0, 8, SURQL_EXT).is_err());
        assert!(select_prune(&[], "surreal/", "../x", 14, 8, SURQL_EXT).is_err());
    }

    #[test]
    fn bugsink_uses_its_own_prefix_and_extension() {
        let mut keys: Vec<String> = (0..10)
            .map(|d| format!("bugsink/daily/bugsink-db-2026101{d}T041700Z.sqlite3.gz"))
            .collect();
        keys.extend(s(&[
            "bugsink/weekly/bugsink-db-20261005T041700Z.sqlite3.gz",
            "surreal/daily/punchable-main-20261010T041700Z.surql.gz",
            "bugsink/daily/bugsink-db-20261010T041700Z.surql.gz",
        ]));
        let (keep, delete) = select_prune(&keys, "bugsink/", "bugsink-db", 3, 8, SQLITE_EXT).unwrap();
        assert_eq!((keep.len(), delete.len()), (4, 7));
        assert!(
            delete
                .iter()
                .all(|x| x.starts_with("bugsink/daily/") && x.ends_with(".sqlite3.gz"))
        );
        assert!(!delete.contains(&"surreal/daily/punchable-main-20261010T041700Z.surql.gz".to_string()));
        assert!(!delete.contains(&"bugsink/daily/bugsink-db-20261010T041700Z.surql.gz".to_string()));
        assert!(select_prune(&keys, "bugsink/", "bugsink-db", 3, 8, ".tar").is_err());
        assert!(select_prune(&keys, "audio/", "bugsink-db", 3, 8, SQLITE_EXT).is_err());
    }
}
