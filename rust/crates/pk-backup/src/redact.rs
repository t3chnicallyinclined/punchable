//! What may reach stderr or stdout: the settings that are secrets are replaced by their names, and anything token-like
//! (40 or more characters of base64 / hex alphabet) by `<redacted>`, except a backup key, which is a name, not a token.

use std::sync::LazyLock;

use regex::Regex;

use crate::settings::Vars;

/// The settings whose values are replaced by `<NAME>` wherever they appear.
const SECRET_ENV: [&str; 7] = [
    "SURREAL_PASS",
    "R2_SECRET_ACCESS_KEY",
    "R2_ACCESS_KEY_ID",
    "R2_ACCOUNT_ID",
    "R2_TOKEN_ID",
    "R2_ENDPOINT",
    "R2_BUCKET",
];

static TOKENISH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z0-9+/=_\-]{40,}").expect("regex"));
// a backup key is not a token: surreal[-x]/daily|weekly/<ns>-<db>-<stamp> (".surql.gz" is cut off by the token regex)
static OUR_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(?:surreal|bugsink)[a-z0-9-]*/(?:daily|weekly)/[a-z0-9_-]+-[0-9]{8}T[0-9]{6}Z)$").expect("regex")
});

/// `redact(text)`.
pub fn redact(text: &str, vars: &Vars) -> String {
    let mut text = text.to_string();
    for k in SECRET_ENV {
        if let Some(v) = vars.get(k)
            && v.chars().count() >= 6
        {
            text = text.replace(v, &format!("<{k}>"));
        }
    }
    TOKENISH
        .replace_all(&text, |m: &regex::Captures<'_>| {
            let s = &m[0];
            if OUR_KEY.is_match(s) {
                s.to_string()
            } else {
                "<redacted>".to_string()
            }
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_tokens_and_env_secrets() {
        let v = Vars::from_pairs([("SURREAL_PASS", "pw-for-the-test-only")]);
        let msg = redact(
            &format!(
                "auth failed for pw-for-the-test-only sig={} key AKIAIOSFODNN7EXAMPLE",
                "a".repeat(64)
            ),
            &v,
        );
        assert!(!msg.contains("pw-for") && !msg.contains(&"a".repeat(40)) && msg.contains("<redacted>"));
        assert!(msg.contains("<SURREAL_PASS>"));
    }

    #[test]
    fn keeps_backup_key_names() {
        let v = Vars::default();
        let k = "surreal-selftest/daily/punchable-main-20261027T021247Z.surql.gz";
        assert_eq!(redact(&format!("uploaded {k}"), &v), format!("uploaded {k}"));
        assert!(redact(&format!("x/surreal/daily/{}", "q".repeat(45)), &v).contains("<redacted>"));
        let b = "bugsink/daily/bugsink-db-20261010T041700Z.sqlite3.gz";
        assert_eq!(redact(b, &v), b);
    }
}
