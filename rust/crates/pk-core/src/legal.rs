//! Court material never reaches a model or a store: `gsync/rules.py` `content_is_legal` (two or more court markers
//! in the lower-cased text), the one rule every service applies to a text on its way in. The generic markers are
//! here; a private rules file (`legal_rules.json`, `content_markers`) adds his own when the process has one.

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

/// The markers a process checks.
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

    /// From a rules file's JSON (`legal_rules.json`); anything unreadable is the generic set, as in the Python
    /// (`_local_rules` returns `{}` on any error).
    pub fn from_rules_json(text: &str) -> Self {
        let extra: Vec<String> = serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|v| v.get("content_markers").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        Self::new(&extra)
    }

    /// How many markers are checked (logged at start; never the markers themselves).
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
}
