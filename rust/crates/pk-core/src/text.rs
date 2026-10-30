//! Text by code points, the way a Python `str` slices: `s[:n]` is the first `n` characters, never bytes.

/// `s[:n]`: the first `n` code points of `s`.
pub fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::head;

    #[test]
    fn head_counts_code_points() {
        assert_eq!(head("héllo", 2), "hé");
        assert_eq!(head("ab", 5), "ab");
        assert_eq!(head("", 3), "");
        assert_eq!(head("日本語", 2), "日本");
    }
}
