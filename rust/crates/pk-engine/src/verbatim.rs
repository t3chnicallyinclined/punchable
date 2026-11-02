//! The one verbatim gate, and the one place a quote is matched against its source.
//!
//! [`verbatim`] is the gate every line that claims to be his passes: a quote is kept only as an exact substring of
//! its source. The one allowance is typography: when a model changed only quote marks, apostrophes, dash style, runs
//! of whitespace or an ellipsis character, the quote is replaced by the exact source slice. Any change to a word, or
//! to case, drops it. Moved here unchanged from `pk-creative::verbatim` (ported from `creative/askchat.py::verbatim`,
//! golden against it in `pk-creative`'s tests, which still run through the re-export), so `pk-creative` (Ask) and the
//! engine's answer check ([`crate::check`]) use one implementation. Whitespace is Python's `str.isspace`
//! (`pk_core::py::isspace`) so the differential against the Python stays exact; positions are code points there and
//! bytes here, which the index map hides.
//!
//! [`align`] comes from upstream (`upstream-ops/src/ask.rs`, Rust branch 9dfedad): it LOCATES the run of lines a
//! paraphrased quote came from (80% of its words, in order). It is never a gate for his words: the engine's check
//! uses it only under [`crate::check::Rules::UPSTREAM`], and even there the line's own text, not the model's
//! words, stands as the quote. Punchable's rules ([`crate::check::Rules::PUNCHABLE`]) leave it off.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use pk_core::py;
use regex::Regex;

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
        if py::isspace(c) {
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
        s = rest.trim_start_matches(py::isspace);
    }
    if let Some(rest) = s.strip_suffix("...").or_else(|| s.strip_suffix('\u{2026}')) {
        s = rest.trim_end_matches(py::isspace);
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
    let q = py::strip(quote);
    if py::len(q) >= QUOTE_MIN
        && let Some(found) = slice_of(source, q)
    {
        return Some(found);
    }
    let q = py::strip(q.trim_matches(|c| matches!(c, '"' | '\'' | '\u{201c}' | '\u{201d}' | '\u{2018}' | '\u{2019}')));
    let q = strip_ellipsis(q);
    if py::len(q) < QUOTE_MIN {
        return None;
    }
    if let Some(found) = slice_of(source, q) {
        return Some(found);
    }
    let (ns, idx) = norm_map(source);
    let (nq, _) = norm_map(q);
    let nq = py::strip(&nq);
    let nq_len = py::len(nq);
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

// ---- from upstream (upstream-ops/src/ask.rs, 9dfedad) ---------------------------------------------------------------

/// Upstream's `labels.quote_min_words`: under this many words a quote counts only word for word.
pub const QUOTE_MIN_WORDS: usize = 3;
/// A quote's words must appear, in order, at this share of their count inside a span for [`align`] to place it.
pub const ALIGN_MIN: f64 = 0.8;

/// Runs of whitespace become one space, ends trimmed (upstream's `normalize`, Python's `_norm`).
pub fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A quote as a reader copies it from a numbered candidate: line numbers (`120| `) are not the text.
pub fn strip_numbers(quote: &str) -> String {
    static NUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|\s)\d{1,7}\| ").expect("a valid pattern"));
    NUM.replace_all(quote, " ").into_owned()
}

/// Where a typed quote sits in a text: the run of lines [`align`] found and the share of the quote's words it holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aligned {
    /// The first line of the run, from 0.
    pub first: usize,
    /// The last line of the run, from 0.
    pub last: usize,
    /// The share of the quote's words found in order.
    pub score: f64,
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Length of the longest common subsequence of two word lists.
fn lcs(a: &[String], b: &[String]) -> usize {
    let mut prev = vec![0usize; b.len() + 1];
    let mut cur = vec![0usize; b.len() + 1];
    for x in a {
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = if x == y { prev[j] + 1 } else { prev[j + 1].max(cur[j]) };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// The smallest run of lines (at most 6) of `text` holding, in order, at least [`ALIGN_MIN`] of `wanted`'s words;
/// punctuation and case do not count, word order does. A locator, never a gate (module doc).
pub fn align(text: &str, wanted: &str) -> Option<Aligned> {
    let want = words(wanted);
    if want.len() < QUOTE_MIN_WORDS {
        return None;
    }
    let lines: Vec<Vec<String>> = text.lines().map(words).collect();
    let mut best: Option<Aligned> = None;
    for width in 1..=6usize {
        for first in 0..lines.len() {
            let last = first + width - 1;
            if last >= lines.len() {
                break;
            }
            let window: Vec<String> = lines[first..=last].iter().flatten().cloned().collect();
            if window.len() > want.len() * 6 + 40 {
                continue;
            }
            let score = lcs(&want, &window) as f64 / want.len() as f64;
            if score >= ALIGN_MIN && best.is_none_or(|b| score > b.score) {
                best = Some(Aligned { first, last, score });
            }
        }
        if best.is_some() {
            break;
        }
    }
    best
}

/// The line of `text` sharing the most words with `wanted`, so a refused quote can be fixed from the refusal.
pub fn nearest_line(text: &str, wanted: &str) -> Option<String> {
    let want: BTreeSet<String> = wanted.split(' ').map(str::to_lowercase).collect();
    text.lines()
        .map(|l| {
            let have: BTreeSet<String> = normalize(l).split(' ').map(str::to_lowercase).collect();
            (want.intersection(&have).count(), l)
        })
        .filter(|(n, _)| *n > 0)
        .max_by_key(|(n, _)| *n)
        .map(|(_, l)| normalize(l))
}

#[cfg(test)]
mod tests {
    use super::*;

    // From pk-creative's verbatim.rs (moved with the gate).
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

    // From upstream's ask.rs tests.
    #[test]
    fn whitespace_is_normalised_like_the_python() {
        assert_eq!(normalize("  a \n\t b  c "), "a b c");
    }

    #[test]
    fn align_places_a_near_miss_and_not_a_stranger() {
        let a = align("one two three\nfour five six seven\neight", "four FIVE six, seven").expect("aligned");
        assert_eq!((a.first, a.last), (1, 1));
        assert!(align("one two three", "nine ten eleven").is_none());
        assert!(
            align("one two three", "one two").is_none(),
            "under three words never aligns"
        );
    }

    #[test]
    fn line_numbers_are_not_text() {
        assert_eq!(
            normalize(&strip_numbers("41| parser rejects empty 42| input")),
            "parser rejects empty input"
        );
        assert_eq!(
            strip_numbers("rejects 3|empty"),
            "rejects 3|empty",
            "a number inside the text is text"
        );
    }

    #[test]
    fn the_gate_is_at_least_as_strict_on_words_as_a_normalised_contains() {
        // Every quote the upstream word-for-word rule accepted, the gate accepts (whitespace aside), and the gate
        // still refuses a changed word or case that the alignment would have placed.
        let unit = "The parser   rejects empty\ninput early.";
        assert!(verbatim("parser rejects empty input", unit).is_some());
        assert!(verbatim("Parser rejects empty input", unit).is_none());
        assert!(align(unit, "Parser rejects empty input").is_some());
    }
}
