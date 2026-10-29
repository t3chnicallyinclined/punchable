//! The verbatim gate by property (docs/RUST.md "What is tested by property, not equality"): anything returned is a
//! slice of the source; a substring of three or more characters comes back as itself; a quote that differs only in
//! typography maps back to the exact source slice; a changed word, or a changed case, returns nothing.

use pk_creative::verbatim;
use proptest::prelude::*;

/// Source text the quote is cut from: letters a–m, digits, the typography the gate forgives, spaces.
fn body() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            8 => prop::char::range('a', 'm'),
            2 => prop::char::range('0', '9'),
            3 => Just(' '),
            1 => Just('\''),
            1 => Just('\u{2019}'),
            1 => Just('"'),
            1 => Just('\u{201c}'),
            1 => Just('-'),
            1 => Just('\u{2014}'),
            1 => Just('\u{2026}'),
            1 => Just('.'),
            1 => Just('\u{a0}'),
            1 => Just('\u{1F600}'),
        ],
        1..40,
    )
    .prop_map(|cs| cs.into_iter().collect())
}

/// Padding around the quote, from an alphabet disjoint from the body's letters (so the quote occurs once).
fn padding() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![6 => prop::char::range('n', 'z'), 2 => Just(' '), 1 => Just('\u{1F60A}'), 1 => Just(',')],
        0..20,
    )
    .prop_map(|cs| cs.into_iter().collect())
}

/// A body that starts and ends with a letter (so stripping touches nothing) and has at least three characters.
fn quote_body() -> impl Strategy<Value = String> {
    (prop::char::range('a', 'm'), body(), prop::char::range('a', 'm')).prop_map(|(a, mid, z)| format!("{a}{mid}{z}"))
}

/// The typography a model might change: curly to straight and back, dashes, an ellipsis, doubled spaces, NBSP.
fn perturb(s: &str, seed: u64) -> String {
    let mut out = String::new();
    let mut i = 0_u64;
    for c in s.chars() {
        i += 1;
        let flip = (seed >> (i % 60)) & 1 == 1;
        match c {
            '\'' if flip => out.push('\u{2019}'),
            '\u{2019}' if flip => out.push('\''),
            '"' if flip => out.push('\u{201c}'),
            '\u{201c}' if flip => out.push('"'),
            '-' if flip => out.push('\u{2013}'),
            '\u{2014}' if flip => out.push('-'),
            '\u{2026}' if flip => out.push_str("..."),
            ' ' if flip => out.push_str("  "),
            '\u{a0}' if flip => out.push(' '),
            ' ' => out.push('\u{a0}'),
            c => out.push(c),
        }
    }
    out
}

proptest! {
    #[test]
    fn anything_returned_is_a_slice_of_the_source(q in "\\PC{0,30}", s in "\\PC{0,60}") {
        if let Some(found) = verbatim(&q, &s) {
            prop_assert!(s.contains(found));
            prop_assert!(found.chars().count() >= 1);
        }
    }

    #[test]
    fn a_substring_of_three_or_more_characters_returns_itself(pre in padding(), sub in quote_body(), post in padding()) {
        let source = format!("{pre}{sub}{post}");
        prop_assert_eq!(verbatim(&sub, &source), Some(sub.as_str()));
    }

    #[test]
    fn typography_maps_back_to_the_exact_source_slice(pre in padding(), sub in quote_body(), post in padding(), seed in any::<u64>()) {
        let source = format!("{pre}{sub}{post}");
        let quote = perturb(&sub, seed);
        prop_assert_eq!(verbatim(&quote, &source), Some(sub.as_str()), "quote {:?}", quote);
    }

    #[test]
    fn a_changed_word_or_case_returns_none(pre in padding(), sub in quote_body(), post in padding(), at in 0usize..40, upper in any::<bool>()) {
        let source = format!("{pre}{sub}{post}");
        let n = sub.chars().count();
        let at = at % n;
        let changed: String = sub.chars().enumerate().map(|(i, c)| {
            if i != at { c } else if upper && c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { 'q' }
        }).collect();
        prop_assert_eq!(verbatim(&changed, &source), None, "quote {:?}", changed);
    }
}
