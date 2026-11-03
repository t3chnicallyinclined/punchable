//! Court material never leaves the worker: `gsync/rules.py` `content_is_legal` (`pk_core::legal::Legal`, the rule
//! the API applies too) and `db/srcpaths.py` (the only text files a worker may write, so the API's own legal check sees
//! every one). The worker has no Secret with the private rules: the generic markers run here, plus any
//! `content_markers` in a rules file the binary is pointed at; the API re-checks every post with the full rules.

use std::sync::LazyLock;

use regex::Regex;

pub use pk_core::legal::{GENERIC_MARKERS, Legal};

const NAME: &str = r"[A-Za-z0-9][A-Za-z0-9_.\-]{0,120}";

static WORKER_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        format!(r"^corpus/sets/{NAME}/(?:analysis\.json|meta\.json|laughs_auto\.json|transcript\.json|clips\.json)$"),
        format!(r"^corpus/memos/{NAME}/(?:transcript\.json|meta\.json)$"),
        r"^kb/gsync/(?:memos\.json|map\.json|units\.json|edits\.jsonl|conflicts\.jsonl|runs\.jsonl)$".to_string(),
        r"^kb/v0/(?:inferred\.json|insights\.json|learning\.json|KB_REPORT\.md)$".to_string(),
        r"^kb/creative/(?:merge_judge\.json|geocode_cache\.json|venue_addresses\.json|suggest_judge\.json)$"
            .to_string(),
        r"^vectors/extra_[a-z_]{1,40}\.json$".to_string(),
    ]
    .iter()
    .map(|p| Regex::new(p).expect("a valid pattern"))
    .collect()
});

/// `srcpaths.clean`: the canonical relative path, or `None` for an absolute path, a drive, `.` or `..`.
pub fn clean(rel: &str) -> Option<String> {
    if rel.is_empty() || rel.chars().count() > 300 {
        return None;
    }
    let rel = rel.replace('\\', "/");
    let drive = rel.len() >= 2 && rel.as_bytes()[0].is_ascii_alphabetic() && rel.as_bytes()[1] == b':';
    if rel.starts_with('/') || drive || rel.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
        return None;
    }
    Some(rel)
}

/// `srcpaths.worker_may_write`.
pub fn worker_may_write(rel: &str) -> bool {
    clean(rel).is_some_and(|r| WORKER_PATTERNS.iter().any(|p| p.is_match(&r)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_whitelisted_paths() {
        assert!(worker_may_write("corpus/sets/20261020_phabc/analysis.json"));
        assert!(worker_may_write("vectors/extra_summary.json"));
        assert!(!worker_may_write("corpus/sets/../x/analysis.json"));
        assert!(!worker_may_write("/corpus/sets/a/analysis.json"));
        assert!(!worker_may_write("kb/v0/decisions.jsonl"));
        assert!(!worker_may_write("corpus/sets/a/audio.wav"));
    }
}
