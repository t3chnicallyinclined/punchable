//! Word normalisation shared by the stage-vs-script comparisons (`infer/scriptdiff.py::ntok`, re-exported by
//! `creative/textdiff.py`): lower case, straight apostrophe, ASCII letters, digits and apostrophes only, small
//! numbers spelled out.

/// `ntok(t)`: the comparison form of one word; empty when nothing of it survives.
pub fn ntok(t: &str) -> String {
    let lower = t.to_lowercase().replace('\u{2019}', "'");
    let kept: String = lower
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '\'')
        .collect();
    let s = kept.trim_matches('\'');
    match s {
        "0" => "zero",
        "1" => "one",
        "2" => "two",
        "3" => "three",
        "4" => "four",
        "5" => "five",
        "6" => "six",
        "7" => "seven",
        "8" => "eight",
        "9" => "nine",
        "10" => "ten",
        "20" => "twenty",
        "30" => "thirty",
        other => other,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::ntok;

    #[test]
    fn like_python() {
        assert_eq!(ntok("Won\u{2019}t"), "won't");
        assert_eq!(ntok("10"), "ten");
        assert_eq!(ntok("'Hello!'"), "hello");
        assert_eq!(ntok("\u{130}"), "i");
        assert_eq!(ntok("\u{df}eta"), "eta");
        assert_eq!(ntok("—"), "");
    }
}
