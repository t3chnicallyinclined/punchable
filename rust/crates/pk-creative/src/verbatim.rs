//! The verbatim gate: a quote is kept only as an exact substring of its source. The one allowance is typography:
//! when a model changed only quote marks, apostrophes, dash style, runs of whitespace or an ellipsis character, the
//! quote is replaced by the exact source slice. Any change to a word, or to case, drops it.
//!
//! One copy, ported from `creative/askchat.py::verbatim` (the one the hosted API runs; `engine/verify.py` is the
//! same rule with one difference in the length gate, see `rust/tools/gen_creative_goldens.py`). Whitespace is
//! Python's `str.isspace` ([`py_isspace`]) so the differential against the Python is exact; positions are code
//! points there and bytes here, which the index map hides.

use crate::pyish::{py_isspace, py_len, py_strip};

/// The shortest quote the gate will look for, in code points (`QUOTE_MIN`).
pub const QUOTE_MIN: usize = 3;

/// The typography the gate forgives: curly to straight, every dash to a hyphen, NBSP to a space (`_TYPO`).
fn typo(c: char) -> char {
    match c {
        '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}' | '\u{2032}' => '\'',
        '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{2033}' => '"',
        '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
        '\u{a0}' => ' ',
        c => c,
    }
}

/// The normalised text and, per normalised character, the byte offset of the source character it came from.
fn norm_map(s: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(s.len());
    let mut idx = Vec::with_capacity(s.len());
    let mut prev_space = false;
    for (i, ch) in s.char_indices() {
        if ch == '\u{2026}' {
            out.push_str("...");
            idx.extend([i, i, i]);
            prev_space = false;
            continue;
        }
        let c = typo(ch);
        if py_isspace(c) {
            if !prev_space {
                out.push(' ');
                idx.push(i);
            }
            prev_space = true;
        } else {
            out.push(c);
            idx.push(i);
            prev_space = false;
        }
    }
    (out, idx)
}

/// `re.sub(r"^(\.\.\.|…)\s*|\s*(\.\.\.|…)$", "", q)`: one leading and one trailing ellipsis the model added.
fn strip_ellipsis(q: &str) -> &str {
    let mut s = q;
    if let Some(rest) = s.strip_prefix("...").or_else(|| s.strip_prefix('\u{2026}')) {
        s = rest.trim_start_matches(py_isspace);
    }
    if let Some(rest) = s.strip_suffix("...").or_else(|| s.strip_suffix('\u{2026}')) {
        s = rest.trim_end_matches(py_isspace);
    }
    s
}

/// The first occurrence of `needle` in `hay`, as a slice of `hay`.
fn slice_of<'a>(hay: &'a str, needle: &str) -> Option<&'a str> {
    hay.find(needle).map(|k| &hay[k..k + needle.len()])
}

/// The exact source substring for `quote`, or `None` when it is not in the source. The returned text is always a
/// slice of `source`; only typography may differ between the quote and what is returned; case and words must match.
pub fn verbatim<'a>(quote: &str, source: &'a str) -> Option<&'a str> {
    if quote.is_empty() || source.is_empty() {
        return None;
    }
    let q = py_strip(quote);
    if py_len(q) >= QUOTE_MIN
        && let Some(found) = slice_of(source, q)
    {
        return Some(found);
    }
    let q = py_strip(q.trim_matches(|c| matches!(c, '"' | '\'' | '\u{201c}' | '\u{201d}' | '\u{2018}' | '\u{2019}')));
    let q = strip_ellipsis(q);
    if py_len(q) < QUOTE_MIN {
        return None;
    }
    if let Some(found) = slice_of(source, q) {
        return Some(found);
    }
    let (ns, idx) = norm_map(source);
    let (nq, _) = norm_map(q);
    let nq = py_strip(&nq);
    let nq_len = py_len(nq);
    if nq_len < QUOTE_MIN {
        return None;
    }
    let k = ns.find(nq)?;
    let kc = ns[..k].chars().count();
    let a = idx[kc];
    let last = idx[kc + nq_len - 1];
    let b = last + source[last..].chars().next().map_or(0, char::len_utf8);
    Some(&source[a..b])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_typography_and_refusals() {
        let src = "He said \u{201c}don\u{2019}t park there\u{201d} \u{2014} twice\u{2026} really";
        assert_eq!(verbatim("don't park there", src), Some("don\u{2019}t park there"));
        assert_eq!(verbatim("\"don't park there\"", src), Some("don\u{2019}t park there"));
        assert_eq!(
            verbatim("there\" - twice...", src),
            Some("there\u{201d} \u{2014} twice")
        );
        assert_eq!(verbatim("there - twice...", src), None);
        assert_eq!(verbatim("Don't park there", src), None);
        assert_eq!(verbatim("don't park here", src), None);
        assert_eq!(verbatim("he", src), None);
        assert_eq!(verbatim("", src), None);
        assert_eq!(verbatim("twice", ""), None);
    }

    #[test]
    fn astral_and_separator_whitespace() {
        let src = "\u{1F600}\u{1F600} a  b\u{1c}c";
        assert_eq!(verbatim("a b c", src), Some("a  b\u{1c}c"));
        assert_eq!(verbatim("\u{1c}a b c\u{1f}", src), Some("a  b\u{1c}c"));
    }

    #[test]
    fn ellipsis_strip_matches_the_regex() {
        assert_eq!(strip_ellipsis("... a ..."), "a");
        assert_eq!(strip_ellipsis("...."), ".");
        assert_eq!(strip_ellipsis("......"), "");
        assert_eq!(strip_ellipsis("\u{2026} \u{2026}"), "");
        assert_eq!(strip_ellipsis("a\u{2026}\u{2026}"), "a\u{2026}");
    }
}
