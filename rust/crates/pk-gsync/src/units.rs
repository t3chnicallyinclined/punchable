//! Corpus-format mirror files and units (`gsync/units.py`): the file name, the front matter, one unit per doc cut at
//! markdown headings, and the comparison form of a text (`norm`, `h`).

use std::sync::LazyLock;

use pk_core::{py, text::head};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// The rule the unit locators carry.
pub const PARA_RULE: &str = "non-blank lines of fileContent, 0-based, inclusive";

static NON_SLUG: LazyLock<Regex> = LazyLock::new(|| Regex::new("[^a-z0-9]+").expect("regex"));
static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s{0,3}#{1,6}\s").expect("regex"));

fn rstrip(s: &str) -> &str {
    s.trim_end_matches(py::isspace)
}

/// `triage.py`'s slug.
pub fn slug(t: &str) -> String {
    let lower = t.to_lowercase();
    let s = NON_SLUG.replace_all(&lower, "-");
    let s = s.trim_matches('-');
    let cut = head(s, 50);
    let cut = cut.trim_end_matches('-');
    if cut.is_empty() {
        "untitled".to_string()
    } else {
        cut.to_string()
    }
}

/// `meta.get(key)` when it is a string.
pub fn text_of<'a>(meta: &'a Value, key: &str) -> Option<&'a str> {
    meta.get(key).and_then(Value::as_str)
}

fn id_of(meta: &Value) -> &str {
    text_of(meta, "id").unwrap_or_default()
}

fn or_empty<'a>(meta: &'a Value, key: &str) -> &'a str {
    match meta.get(key) {
        Some(Value::String(s)) => s,
        _ => "",
    }
}

/// `<created-date>_<slug>__<docid8>.md`.
pub fn mirror_name(meta: &Value) -> String {
    format!(
        "{}_{}__{}.md",
        head(or_empty(meta, "createdTime"), 10),
        slug(or_empty(meta, "name")),
        head(id_of(meta), 8)
    )
}

/// The mirror file: front matter, then the export unedited.
pub fn mirror_text(meta: &Value, body: &str) -> String {
    let s = |k: &str| py::str(meta.get(k).unwrap_or(&Value::Null));
    let title = serde_json::to_string(or_empty(meta, "name")).expect("string");
    let fm = [
        "---".to_string(),
        "source: gdoc".to_string(),
        format!("doc_id: {}", id_of(meta)),
        format!("title: {title}"),
        format!("created: {}", s("createdTime")),
        format!("modified: {}", s("modifiedTime")),
        format!("url: https://docs.google.com/document/d/{}/edit", id_of(meta)),
        "---".to_string(),
    ];
    format!("{}\n{body}", fm.join("\n"))
}

/// One unit per doc, cut at markdown headings; an empty body is a title unit.
pub fn split_units(meta: &Value, body: &str, file: &str) -> Vec<Value> {
    let lines: Vec<&str> = body.split('\n').filter(|ln| !py::strip(ln).is_empty()).collect();
    let name = or_empty(meta, "name");
    let mut base = Map::new();
    base.insert("doc_id".into(), json!(id_of(meta)));
    base.insert("doc_title".into(), json!(name));
    base.insert(
        "created".into(),
        meta.get("createdTime").cloned().unwrap_or(Value::Null),
    );
    base.insert(
        "modified".into(),
        meta.get("modifiedTime").cloned().unwrap_or(Value::Null),
    );
    base.insert("file".into(), json!(file));
    base.insert("kind".into(), json!("note"));
    base.insert("parent_hint".into(), json!(name));
    base.insert("topics".into(), json!([]));
    base.insert("devices".into(), json!([]));
    base.insert("structures".into(), json!([]));
    base.insert("form".into(), Value::Null);
    base.insert("attitude".into(), Value::Null);
    base.insert("confidence".into(), json!(0.4));
    base.insert("needs_look".into(), json!(true));
    base.insert("source".into(), json!("gsync"));
    let with = |extra: [(&str, Value); 3]| {
        let mut m = base.clone();
        for (k, v) in extra {
            m.insert(k.to_string(), v);
        }
        Value::Object(m)
    };
    let id8 = head(id_of(meta), 8);
    if lines.is_empty() {
        return vec![with([
            ("unit_id", json!(format!("{id8}-00"))),
            ("locator", json!({"source": "title"})),
            ("text", json!(name)),
        ])];
    }
    let cuts: Vec<usize> = std::iter::once(0)
        .chain((1..lines.len()).filter(|&i| HEADING.is_match(lines[i])))
        .collect();
    let mut out = Vec::new();
    for (n, &s) in cuts.iter().enumerate() {
        let e = cuts.get(n + 1).copied().unwrap_or(lines.len()) - 1;
        out.push(with([
            ("unit_id", json!(format!("{id8}-{n:02}"))),
            (
                "locator",
                json!({"para_start": s, "para_end": e, "para_rule": PARA_RULE}),
            ),
            ("text", json!(lines[s..=e].join("\n"))),
        ]));
    }
    out
}

/// The comparison form of a text: no BOM, `\n` line ends, no trailing spaces, trimmed.
pub fn norm(t: &str) -> String {
    let t = t.replace('\u{feff}', "").replace("\r\n", "\n").replace('\r', "\n");
    let joined = t.split('\n').map(rstrip).collect::<Vec<_>>().join("\n");
    py::strip(&joined).to_string()
}

/// The first 16 hex digits of the SHA-256 of the normalised text.
pub fn h(t: &str) -> String {
    let d = Sha256::digest(norm(t).as_bytes());
    hex::encode(d)[..16].to_string()
}

/// `h(None)`: the hash of the empty text.
pub fn h_opt(t: Option<&str>) -> String {
    h(t.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_and_norm_follow_the_python() {
        assert_eq!(slug("  Hello, World -- 2!  "), "hello-world-2");
        assert_eq!(slug("???"), "untitled");
        assert_eq!(slug(&"a".repeat(60)), "a".repeat(50));
        assert_eq!(norm("\u{feff}a  \r\nb\t\rc \n\n"), "a\nb\nc");
        assert_eq!(h("x"), h("\u{feff}x \r\n"));
        assert_eq!(h("").len(), 16);
    }

    #[test]
    fn units_cut_at_headings_and_empty_is_a_title_unit() {
        let meta = json!({"id": "abcdefgh12345", "name": "T", "createdTime": "2026-10-01T00:00:00.000Z", "modifiedTime": null});
        let u = split_units(&meta, "one\n\n# two\nthree\n   ## four\nfive", "f.md");
        assert_eq!(u.len(), 3);
        assert_eq!(u[0]["locator"]["para_end"], 0);
        assert_eq!(u[1]["text"], "# two\nthree");
        assert_eq!(u[2]["unit_id"], "abcdefgh-02");
        let t = split_units(&meta, "  \n", "f.md");
        assert_eq!(t[0]["locator"], json!({"source": "title"}));
        assert_eq!(t[0]["text"], "T");
        assert!(mirror_text(&meta, "body").contains("modified: None\n"));
        assert_eq!(mirror_name(&meta), "2026-10-01_t__abcdefgh.md");
    }
}
