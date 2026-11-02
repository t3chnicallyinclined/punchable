//! The verbatim gate: a quote is kept only as an exact substring of its source, typography forgiven (curly quotes,
//! dash style, whitespace runs, an ellipsis character), words and case never.
//!
//! One copy for every caller: the implementation lives in `pk_engine::verbatim` since the upstream fork
//! (docs/adr/0013-fork-the-upstream-engine.md), where the engine's answer check uses it too. This module re-exports
//! it so Ask ([`crate::ask`]) and this crate's goldens (`tests/goldens.rs`, against both Python copies) and property
//! tests (`tests/verbatim_props.rs`) keep proving the same function.
//!
//! Next to it, the two helpers of `engine/verify.py` the one-shot Ask ([`crate::oneshot`]) runs after the gate:
//! [`strip_unverified_inline_quotes`] and [`normalize_ids`]. They call the gate; they never fork it.

use std::sync::LazyLock;

use regex::{Captures, Regex};

pub use pk_engine::verbatim::{QUOTE_MIN, verbatim};

/// What replaces a "double-quoted" passage of the prose that is in none of the sources.
pub const QUOTE_REMOVED: &str = "[quote removed: not verbatim in the material]";

static STRAIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(")([^"\n]{3,600})(")"#).expect("regex"));
static CURLY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("(\u{201c})([^\u{201c}\u{201d}\n]{3,600})(\u{201d})").expect("regex"));
static LOOKALIKE_HYPHENS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("[\u{2010}\u{2011}\u{2012}\u{2013}\u{2212}]").expect("regex"));

/// `strip_unverified_inline_quotes(text, sources, min_words=4)`: every straight-quoted, then every curly-quoted,
/// passage of 3 to 600 characters on one line, of at least `min_words` words (Python `str.split()`), that the gate
/// finds in none of `sources` is replaced by [`QUOTE_REMOVED`]; the passages removed come back in order.
pub fn strip_unverified_inline_quotes<S: AsRef<str>>(
    text: &str,
    sources: &[S],
    min_words: usize,
) -> (String, Vec<String>) {
    let mut flags = Vec::new();
    let mut repl = |m: &Captures<'_>| -> String {
        let q = &m[2];
        if q.split(pk_core::py::isspace).filter(|w| !w.is_empty()).count() < min_words
            || sources.iter().any(|s| verbatim(q, s.as_ref()).is_some())
        {
            return m[0].to_string();
        }
        flags.push(q.to_string());
        QUOTE_REMOVED.to_string()
    };
    let out = STRAIGHT.replace_all(text, &mut repl).into_owned();
    let out = CURLY.replace_all(&out, &mut repl).into_owned();
    (out, flags)
}

/// `normalize_ids(text)`: the look-alike hyphens a model prints in ids (U+2010, U+2011, U+2012, U+2013, U+2212)
/// mapped back to `-`.
pub fn normalize_ids(text: &str) -> String {
    LOOKALIKE_HYPHENS.replace_all(text, "-").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_in_no_source_is_cut_and_a_short_or_true_one_kept() {
        let src = ["My landlord started a group chat for the building."];
        let (out, flags) = strip_unverified_inline_quotes(
            "He said \"started a group chat for\" and \u{201c}a thing he never said at all\u{201d} and \"hi there\".",
            &src,
            4,
        );
        assert_eq!(
            out,
            format!("He said \"started a group chat for\" and {QUOTE_REMOVED} and \"hi there\".")
        );
        assert_eq!(flags, ["a thing he never said at all"]);
        assert_eq!(
            normalize_ids("unit:a\u{2011}b perf:c\u{2212}d e\u{2014}f"),
            "unit:a-b perf:c-d e\u{2014}f"
        );
    }
}
