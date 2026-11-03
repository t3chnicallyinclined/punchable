//! The small helpers of `creative/util.py` the write routes mint ids with: [`slug`] (a url-ish piece of a title).
//! `sha8` lives in [`crate::merge`] beside the ids it hashes; `now_iso` is `pk_core::time`.

/// `slug(s, n)`: lower-cased, every run of anything but `a-z0-9` one `-`, trimmed of `-`, the first `n` characters
/// trimmed again, `untitled` when nothing is left. Python's `[^a-z0-9]` is ASCII, so an accented letter is a dash.
pub fn slug(s: &str, n: usize) -> String {
    let lower = s.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut dash = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let head: String = out.trim_matches('-').chars().take(n).collect();
    let cut = head.trim_matches('-');
    if cut.is_empty() {
        "untitled".to_string()
    } else {
        cut.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn slug_is_the_pythons() {
        assert_eq!(slug("New 7 for Friday!", 32), "new-7-for-friday");
        assert_eq!(slug("  --  ", 32), "untitled");
        assert_eq!(slug("", 32), "untitled");
        assert_eq!(slug("a very long title that goes on and on", 10), "a-very-lon");
        assert_eq!(slug("abc-def", 4), "abc", "the cut end is trimmed of dashes again");
        assert_eq!(slug("Café Olé", 32), "caf-ol", "non-ASCII letters are runs of dashes");
    }
}
