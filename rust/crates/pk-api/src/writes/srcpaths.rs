//! Which text files live in the database (`srcfile` rows) and may be written there by the home worker: `db/srcpaths.py`
//! (`WORKER_PATTERNS`, `clean`, `worker_may_write`, `is_set_input`). One list for every writer, so the bundle, the
//! API's worker route and the worker's upload step can never disagree about what is data.
//!
//! The Python matches with `re.match(...)` and a `$`, which also accepts one final newline: the patterns here end in
//! `\n?\z` so a path with a trailing newline is judged the way the Python judges it.

use std::sync::LazyLock;

use regex::Regex;

/// `MAX_FILE_BYTES`: the largest file the worker route takes.
pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;

const NAME: &str = r"[A-Za-z0-9][A-Za-z0-9_.\-]{0,120}";

static WORKER_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        format!(
            r"\Acorpus/sets/{NAME}/(?:analysis\.json|meta\.json|laughs_auto\.json|transcript\.json|clips\.json)\n?\z"
        ),
        format!(r"\Acorpus/memos/{NAME}/(?:transcript\.json|meta\.json)\n?\z"),
        r"\Akb/gsync/(?:memos\.json|map\.json|units\.json|edits\.jsonl|conflicts\.jsonl|runs\.jsonl)\n?\z".to_string(),
        r"\Akb/v0/(?:inferred\.json|insights\.json|learning\.json|KB_REPORT\.md)\n?\z".to_string(),
        r"\Akb/creative/(?:merge_judge\.json|geocode_cache\.json|venue_addresses\.json|suggest_judge\.json)\n?\z"
            .to_string(),
        r"\Avectors/extra_[a-z_]{1,40}\.json\n?\z".to_string(),
    ]
    .iter()
    .map(|p| Regex::new(p).unwrap_or_else(|e| unreachable!("constant regex: {e}")))
    .collect()
});

/// `clean(rel)`: the canonical form of a relative path (backslashes become slashes), or `None` when it is not a plain
/// relative path (empty, over 300 characters, absolute, a drive letter, an empty / `.` / `..` part).
pub fn clean(rel: &str) -> Option<String> {
    if rel.is_empty() || rel.chars().count() > 300 {
        return None;
    }
    let rel = rel.replace('\\', "/");
    let drive = rel.as_bytes().first().is_some_and(u8::is_ascii_alphabetic) && rel.as_bytes().get(1) == Some(&b':');
    if rel.starts_with('/') || drive || rel.split('/').any(|p| matches!(p, "" | "." | "..")) {
        return None;
    }
    Some(rel)
}

/// `worker_may_write(rel)`.
pub fn worker_may_write(rel: &str) -> bool {
    clean(rel).is_some_and(|r| WORKER_PATTERNS.iter().any(|p| p.is_match(&r)))
}

/// `is_set_input(rel)`: the files `db/load.py` `sync` reads (a change means the graph tables must be re-synced).
pub fn is_set_input(rel: &str) -> bool {
    rel.starts_with("corpus/sets/") || rel.starts_with("kb/v0/") || rel.starts_with("kb/v1/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_clean_or_nothing() {
        assert_eq!(
            clean("corpus\\sets\\x\\analysis.json").as_deref(),
            Some("corpus/sets/x/analysis.json")
        );
        for bad in [
            "",
            "/etc/passwd",
            "C:/x",
            "a/../b",
            "a//b",
            "./a",
            "a/",
            &"x".repeat(301),
        ] {
            assert_eq!(clean(bad), None, "{bad:?}");
        }
        assert!(clean(&"x".repeat(300)).is_some());
    }

    #[test]
    fn the_worker_may_write_data_files_only() {
        for ok in [
            "corpus/sets/20250304_cellar/analysis.json",
            "corpus/sets/a.b-c_d/laughs_auto.json",
            "corpus/memos/m1/transcript.json",
            "kb/gsync/runs.jsonl",
            "kb/v0/inferred.json",
            "kb/creative/merge_judge.json",
            "vectors/extra_unit_vectors.json",
            "kb\\v0\\learning.json",
            "kb/v0/inferred.json\n",
        ] {
            assert!(worker_may_write(ok), "{ok:?}");
        }
        for no in [
            "kb/v0/decisions.jsonl",
            "corpus/sets/x/audio.mp3",
            "corpus/sets/x/y/analysis.json",
            "corpus/sets/-x/analysis.json",
            "kb/v1/graph.json",
            "vectors/extra_UP.json",
            "kb/v0/inferred.json\n\n",
            "../kb/v0/inferred.json",
        ] {
            assert!(!worker_may_write(no), "{no:?}");
        }
    }

    #[test]
    fn set_inputs_are_what_sync_reads() {
        assert!(is_set_input("corpus/sets/x/meta.json"));
        assert!(is_set_input("kb/v0/inferred.json"));
        assert!(!is_set_input("kb/gsync/runs.jsonl"));
        assert!(!is_set_input("vectors/extra_a.json"));
    }
}
