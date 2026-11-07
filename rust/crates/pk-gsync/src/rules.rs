//! Legal exclusion and comedy triage for Google Docs (`gsync/rules.py`). Court and legal documents are never
//! exported. The generic rules always apply; his own list (folder ids, doc ids, extra keywords, his Google account)
//! lives in a private JSON file that is read and never printed. The private file can only exclude MORE, with one
//! narrow exception (`own_keywords_only_ids`, kept as in the Python).

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const GENERIC_KW: [&str; 9] = [
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
const FILING_KW: [&str; 10] = [
    "summation",
    "affirmation",
    "affadavit",
    "memorandum of law",
    "motion to",
    "order to show cause",
    "otsc",
    "subpoena",
    "q&a prep",
    "cover letter for osc",
];
const CONTENT_MARKERS: [&str; 14] = [
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

static FILING_WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bosc\b").expect("regex"));
static NOT_COMEDY_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(resume|résumé|cover ?letter|coverletter|app idea|business (plan|idea|model)|start a business|blockchain|crypto|dao|nft|kubernetes|landing page|take-home|sow|10dlc|test 1|copy of test)\b",
    )
    .expect("regex")
});

/// The legal rules in force.
#[derive(Debug, Clone)]
pub struct Rules {
    legal_parents: BTreeSet<String>,
    legal_folders_extra: BTreeSet<String>,
    kw: Vec<String>,
    own_kw: Vec<String>,
    own_kw_only_ids: BTreeSet<String>,
    owner: Option<String>,
    content_markers: Vec<String>,
    legal_ids: BTreeSet<String>,
    not_comedy_ids: BTreeSet<String>,
}

fn strings(local: &Value, key: &str) -> Vec<String> {
    local
        .get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

impl Rules {
    /// The rules from his private file's parsed text (`{}` when there is none) and `PUNCHABLE_OWNER_EMAIL`.
    pub fn new(local: &Value, owner_env: Option<&str>) -> Self {
        let own_kw: Vec<String> = strings(local, "keywords").iter().map(|k| k.to_lowercase()).collect();
        let mut kw: Vec<String> = GENERIC_KW.iter().map(ToString::to_string).collect();
        for k in &own_kw {
            if !kw.contains(k) {
                kw.push(k.clone());
            }
        }
        let mut markers: Vec<String> = CONTENT_MARKERS.iter().map(ToString::to_string).collect();
        for m in strings(local, "content_markers") {
            let m = m.to_lowercase();
            if !markers.contains(&m) {
                markers.push(m);
            }
        }
        let owner = local
            .get("owner")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or(owner_env.filter(|s| !s.is_empty()))
            .map(str::to_string);
        Self {
            legal_parents: strings(local, "legal_parents").into_iter().collect(),
            legal_folders_extra: strings(local, "legal_folders_extra").into_iter().collect(),
            kw,
            own_kw,
            own_kw_only_ids: strings(local, "own_keywords_only_ids").into_iter().collect(),
            owner,
            content_markers: markers,
            legal_ids: strings(local, "legal_ids").into_iter().collect(),
            not_comedy_ids: strings(local, "not_comedy_ids").into_iter().collect(),
        }
    }

    /// Read `legal_rules.json` from `path`; a missing or unreadable file is an empty one (the generic rules only).
    pub fn load(path: &Path, owner_env: Option<&str>) -> Self {
        let local = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .filter(Value::is_object)
            .unwrap_or(Value::Object(Map::new()));
        Self::new(&local, owner_env)
    }

    /// Metadata-only check: why a doc is legal, or `None`. Never needs the doc's text. `ancestors` is every folder
    /// above the doc; `extra_ids` the ids already excluded.
    pub fn legal_reason(&self, meta: &Value, ancestors: &[String], extra_ids: &Map<String, Value>) -> Option<String> {
        let did = meta.get("id").and_then(Value::as_str);
        if let Some(d) = did
            && (self.legal_ids.contains(d) || extra_ids.contains_key(d))
        {
            return Some("denylist".into());
        }
        let parents = meta
            .get("parents")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for p in parents.iter().chain(ancestors) {
            if self.legal_parents.contains(p) || self.legal_folders_extra.contains(p) {
                return Some("folder".into());
            }
        }
        let first_owner = meta
            .get("owners")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .map(|o| o.get("emailAddress").and_then(Value::as_str));
        match first_owner {
            None => return Some("owner".into()),
            Some(email) => {
                if let Some(owner) = &self.owner
                    && email != Some(owner.as_str())
                {
                    return Some("owner".into());
                }
            }
        }
        let t = meta
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase();
        let own_only = did.is_some_and(|d| self.own_kw_only_ids.contains(d)) && !self.own_kw.is_empty();
        let list = if own_only { &self.own_kw } else { &self.kw };
        let hit: Vec<&str> = list
            .iter()
            .filter(|k| t.contains(k.as_str()))
            .map(String::as_str)
            .collect();
        if !hit.is_empty() {
            return Some(format!("kw:{}", hit.join(",")));
        }
        let filing: Vec<&str> = FILING_KW.iter().copied().filter(|k| t.contains(k)).collect();
        if !filing.is_empty() || FILING_WORDS.is_match(&t) {
            let names = if filing.is_empty() { vec!["osc"] } else { filing };
            return Some(format!("filing:{}", names.join(",")));
        }
        None
    }

    /// The second net, on exported text: two or more markers and the doc is dropped at once.
    pub fn content_is_legal(&self, text: &str) -> bool {
        let t = text.to_lowercase();
        self.content_markers.iter().filter(|m| t.contains(m.as_str())).count() >= 2
    }

    /// `'comedy'` or `'not_comedy'`, for a doc `legal_reason` passed.
    pub fn triage(&self, meta: &Value) -> &'static str {
        if meta
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|d| self.not_comedy_ids.contains(d))
        {
            return "not_comedy";
        }
        if NOT_COMEDY_TITLE.is_match(meta.get("name").and_then(Value::as_str).unwrap_or_default()) {
            return "not_comedy";
        }
        "comedy"
    }

    /// SHA-256 over the legal constants in a canonical form, so a change is visible in the logs.
    pub fn fingerprint(&self) -> String {
        let canon = serde_json::json!({
            "parents": self.legal_parents, "extra": self.legal_folders_extra, "kw": self.kw,
            "owner": self.owner, "ids": self.legal_ids, "markers": self.content_markers,
            "own_kw_only": self.own_kw_only_ids,
        });
        hex::encode(Sha256::digest(canon.to_string().as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rules() -> Rules {
        Rules::new(
            &json!({"owner": "me@example.com", "legal_parents": ["lf"], "keywords": ["Sample Person"],
                    "content_markers": ["Sample County"], "legal_ids": ["bad"], "own_keywords_only_ids": ["ill"]}),
            None,
        )
    }

    fn doc(id: &str, name: &str, owner: &str, parents: &[&str]) -> Value {
        json!({"id": id, "name": name, "owners": [{"emailAddress": owner}], "parents": parents})
    }

    #[test]
    fn each_net_names_its_reason() {
        let r = rules();
        let none = Map::new();
        let why = |m: &Value| r.legal_reason(m, &[], &none);
        assert_eq!(
            why(&doc("bad", "x", "me@example.com", &[])).as_deref(),
            Some("denylist")
        );
        assert_eq!(
            why(&doc("a", "x", "me@example.com", &["lf"])).as_deref(),
            Some("folder")
        );
        assert_eq!(why(&doc("a", "x", "you@example.com", &[])).as_deref(), Some("owner"));
        assert_eq!(why(&json!({"id": "a", "name": "x"})).as_deref(), Some("owner"));
        assert_eq!(
            why(&doc("a", "My Petition", "me@example.com", &[])).as_deref(),
            Some("kw:petition")
        );
        assert_eq!(
            why(&doc("a", "sample person bit", "me@example.com", &[])).as_deref(),
            Some("kw:sample person")
        );
        assert_eq!(
            why(&doc("a", "Q&A prep", "me@example.com", &[])).as_deref(),
            Some("filing:q&a prep")
        );
        assert_eq!(
            why(&doc("a", "the OSC", "me@example.com", &[])).as_deref(),
            Some("filing:osc")
        );
        assert_eq!(why(&doc("a", "Airport", "me@example.com", &[])), None);
        // a doc listed in own_keywords_only_ids is checked against his keywords alone
        assert_eq!(why(&doc("ill", "illegal parking", "me@example.com", &[])), None);
        assert!(
            r.legal_reason(&doc("a", "x", "me@example.com", &["up"]), &["lf".into()], &none)
                .is_some()
        );
        let mut ex = Map::new();
        ex.insert("a".into(), json!("content"));
        assert_eq!(
            r.legal_reason(&doc("a", "x", "me@example.com", &[]), &[], &ex)
                .as_deref(),
            Some("denylist")
        );
    }

    #[test]
    fn content_needs_two_markers_and_triage_defaults_to_comedy() {
        let r = rules();
        assert!(!r.content_is_legal("the petitioner said hello"));
        assert!(r.content_is_legal("The Petitioner and the RESPONDENT"));
        assert!(r.content_is_legal("petitioner in Sample County"));
        assert_eq!(r.triage(&json!({"id": "a", "name": "My Résumé"})), "not_comedy");
        assert_eq!(r.triage(&json!({"id": "a", "name": "Airport"})), "comedy");
        assert_eq!(r.fingerprint().len(), 64);
    }
}
