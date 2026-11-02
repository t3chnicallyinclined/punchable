//! Court material never leaves the worker: `gsync/rules.py` `content_is_legal` (two or more court markers in the
//! lower-cased text) and `db/srcpaths.py` (the only text files a worker may write, so the API's own legal check sees
//! every one). The worker has no Secret with the private rules: the generic markers run here, plus any
//! `content_markers` in a rules file the binary is pointed at; the API re-checks every post with the full rules.

use std::sync::LazyLock;

use regex::Regex;

/// The generic second net (`CONTENT_MARKERS` before the private additions).
pub const GENERIC_MARKERS: [&str; 14] = [
    "supreme court of the state of",
    "court of the state of",
    "appellate division",
    "index no",
    "docket no",
    "file no.",
    "petitioner",
    "respondent",
    "plaintiff",
    "defendant",
    "under penalty of perjury",
    "sworn to before me",
    "order to show cause",
    "affirmation in",
];

/// The markers this worker checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Legal {
    markers: Vec<String>,
}

impl Default for Legal {
    fn default() -> Self {
        Self::new(&[])
    }
}

impl Legal {
    /// The generic markers plus `extra` (a rules file's `content_markers`), lower-cased, without repeats.
    pub fn new(extra: &[String]) -> Self {
        let mut markers: Vec<String> = GENERIC_MARKERS.iter().map(|m| (*m).to_string()).collect();
        for m in extra {
            let m = m.to_lowercase();
            if !markers.contains(&m) {
                markers.push(m);
            }
        }
        Self { markers }
    }

    /// From a rules file's JSON (`legal_rules.json`); anything unreadable is the generic set, as in the Python.
    pub fn from_rules_json(text: &str) -> Self {
        let extra: Vec<String> = serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|v| v.get("content_markers").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        Self::new(&extra)
    }

    /// How many markers this worker checks (logged at start; never the markers themselves).
    pub fn len(&self) -> usize {
        self.markers.len()
    }

    /// Never empty (the generic set is always there).
    pub fn is_empty(&self) -> bool {
        self.markers.is_empty()
    }

    /// `content_is_legal(text)`.
    pub fn is_legal(&self, text: &str) -> bool {
        let t = text.to_lowercase();
        self.markers.iter().filter(|m| t.contains(m.as_str())).count() >= 2
    }
}

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
    fn two_markers_make_court_text() {
        let l = Legal::default();
        assert!(!l.is_legal("The Petitioner walked in."));
        assert!(l.is_legal("The PETITIONER and the respondent walked in."));
        let l = Legal::from_rules_json(r#"{"content_markers": ["Golden County"]}"#);
        assert_eq!(l.len(), GENERIC_MARKERS.len() + 1);
        assert!(l.is_legal("golden county, plaintiff"));
        assert_eq!(Legal::from_rules_json("not json"), Legal::default());
    }

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
