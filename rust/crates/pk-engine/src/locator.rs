//! Where a candidate's text sits, and its lines. From upstream `gw-ops/src/store.rs` (`line_range`, `first_line`,
//! `numbered`) and `ask.rs` (`span_text`), rust-successor 9dfedad, unchanged. Upstream's units are file spans
//! (`path#L120-135`); Punchable's are records (`unit:…`, `performance:…`) whose lines count from 1, which every
//! function here already handles (a locator without `#L` numbers its own lines).

/// The `(start, end)` of a `path#Lstart-end` locator.
pub fn line_range(locator: &str) -> Option<(usize, usize)> {
    let (a, b) = locator.rsplit_once("#L")?.1.split_once('-')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// The first line of a `path#Lstart-end` locator.
pub fn first_line(locator: &str) -> Option<usize> {
    line_range(locator).map(|r| r.0)
}

/// A text with its line numbers (`120| ...`) when the locator has them, so a reader can cite a line; a record's
/// text is returned as it is.
pub fn numbered(text: &str, locator: &str) -> String {
    match first_line(locator) {
        Some(start) => text
            .lines()
            .enumerate()
            .map(|(i, l)| format!("{}| {l}", start + i))
            .collect::<Vec<_>>()
            .join("\n"),
        None => text.to_owned(),
    }
}

/// Lines `a..=b` (the locator's numbering) of a text, or `None` when outside it. A text without line numbers
/// counts its own lines from 1. The end comes from the locator: a trailing blank line that `lines()` drops is
/// still quotable.
pub fn span_text(text: &str, locator: &str, a: usize, b: usize) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let (start, end) = line_range(locator).unwrap_or((1, lines.len()));
    (a >= start && b >= a && b <= end).then(|| {
        (a..=b)
            .map(|n| lines.get(n - start).copied().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_follow_the_locator_and_records_count_from_one() {
        assert_eq!(span_text("a\nb\nc", "f.md#L10-12", 11, 12).as_deref(), Some("b\nc"));
        assert_eq!(span_text("a\nb\nc", "f.md#L10-12", 9, 10), None);
        assert_eq!(span_text("a\nb", "f.md#L10-12", 12, 12).as_deref(), Some(""));
        assert_eq!(span_text("a\nb", "f.md#L10-12", 13, 13), None);
        assert_eq!(span_text("one\ntwo", "unit:abc", 2, 2).as_deref(), Some("two"));
        assert_eq!(
            numbered("The parser   rejects empty\ninput early.", "f.md#L41-42"),
            "41| The parser   rejects empty\n42| input early."
        );
        assert_eq!(numbered("as is", "unit:abc"), "as is");
    }
}
