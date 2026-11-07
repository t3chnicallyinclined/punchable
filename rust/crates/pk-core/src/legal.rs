//! Court material never reaches a model or a store: `gsync/rules.py` `content_is_legal` (two or more court markers
//! in the lower-cased text), the one rule every service applies to a text on its way in. The generic markers are
//! here; a private rules file (`legal_rules.json`, `content_markers`) adds his own when the process has one.

use std::collections::BTreeSet;

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

/// The generic title keywords (`GENERIC_KW`), part of the rules' fingerprint.
pub const GENERIC_KW: [&str; 9] = [
    "petition",
    "court",
    "cross examination",
    "exhibit",
    "affidavit",
    "deposition",
    "lawsuit",
    "hearing notes",
    "legal",
];

/// The private rules the fingerprint covers besides the markers (`gsync/rules.py` constants).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Rules {
    parents: BTreeSet<String>,
    folders_extra: BTreeSet<String>,
    own_kw: Vec<String>,
    owner: Option<String>,
    ids: BTreeSet<String>,
    own_kw_only: BTreeSet<String>,
}

/// The markers a process checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Legal {
    markers: Vec<String>,
    rules: Rules,
}

impl Default for Legal {
    fn default() -> Self {
        Self::new(&[])
    }
}

/// A rules file's list of strings under `key` (anything else is none, as an unusable file is `{}`).
fn strings(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| serde_json::from_value(x.clone()).ok())
        .unwrap_or_default()
}

impl Legal {
    /// The generic markers plus `extra` (a rules file's `content_markers`), lower-cased. As in the Python, an own
    /// marker is dropped only when it is one of the generic ones: two equal own markers both stay, and each counts.
    pub fn new(extra: &[String]) -> Self {
        let mut markers: Vec<String> = GENERIC_MARKERS.iter().map(|m| (*m).to_string()).collect();
        let own: Vec<String> = extra
            .iter()
            .map(|m| m.to_lowercase())
            .filter(|m| !GENERIC_MARKERS.contains(&m.as_str()))
            .collect();
        markers.extend(own);
        Self {
            markers,
            rules: Rules::default(),
        }
    }

    /// From a rules file's JSON (`legal_rules.json`); anything unreadable is the generic set, as in the Python
    /// (`_local_rules` returns `{}` on any error).
    pub fn from_rules_json(text: &str) -> Self {
        Self::from_rules(text, None)
    }

    /// [`Legal::from_rules`] that refuses a file that is not a JSON object (the lenient one reads it as no rules at
    /// all, the Python's `_local_rules`): for a process that was told to use the file and must not run without it.
    pub fn try_from_rules(text: &str, owner_env: Option<&str>) -> Result<Self, String> {
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(v) if v.is_object() => Ok(Self::from_rules(text, owner_env)),
            Ok(_) => Err("not a JSON object".to_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// [`Legal::from_rules_json`] with `PUNCHABLE_OWNER_EMAIL` (`owner`, when the file names none).
    pub fn from_rules(text: &str, owner_env: Option<&str>) -> Self {
        let v: serde_json::Value = serde_json::from_str(text)
            .ok()
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        let mut me = Self::new(&strings(&v, "content_markers"));
        me.rules = Rules {
            parents: strings(&v, "legal_parents").into_iter().collect(),
            folders_extra: strings(&v, "legal_folders_extra").into_iter().collect(),
            own_kw: strings(&v, "keywords").iter().map(|k| k.to_lowercase()).collect(),
            owner: v
                .get("owner")
                .and_then(serde_json::Value::as_str)
                .filter(|o| !o.is_empty())
                .or(owner_env)
                .map(str::to_string),
            ids: strings(&v, "legal_ids").into_iter().collect(),
            own_kw_only: strings(&v, "own_keywords_only_ids").into_iter().collect(),
        };
        me
    }

    /// The text `gsync/rules.py` `fingerprint()` hashes: the legal constants in a canonical form.
    pub fn canonical(&self) -> String {
        let mut kw: Vec<String> = GENERIC_KW.iter().map(|k| (*k).to_string()).collect();
        kw.extend(
            self.rules
                .own_kw
                .iter()
                .filter(|k| !GENERIC_KW.contains(&k.as_str()))
                .cloned(),
        );
        let r = &self.rules;
        crate::py::dumps_sorted(&serde_json::json!({
            "parents": r.parents, "extra": r.folders_extra, "kw": kw, "owner": r.owner,
            "ids": r.ids, "markers": self.markers, "own_kw_only": r.own_kw_only,
        }))
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
