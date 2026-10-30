//! The words Ask works with, ported from `creative/notebook.py` and `creative/util.py`: `tokens` (the search
//! words of a text: lower case, a light stemmer, short words out unless listed, stop words out), `short_title`,
//! `month_year`, `mmss`, and the prompt's `_snippet` (a window of a long text around the first search hit).
//! Lengths and positions are code points, as in the Python.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use pk_core::py;

/// `_TOK`: a word is a run of ASCII letters, digits and apostrophes starting with a letter or digit.
static TOK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z0-9][a-z0-9']*").expect("a valid regex"));

/// `STOP` (minus `us`, which the Python removes after building the set).
static STOP: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    "the and that this with you your for are was but not what when they them then there their have has had \
     just like its it's i'm don't can't about from all one get got know out who how why would could should will into \
     been were did does doing very really because also more some any him her his she he our we my me too than \
     only even ever every said says say going gonna want make made thing things people time good bad yeah okay \
     lol im dont cant ill ive youre theyre thats whats wanna gotta kinda sorta well oh uh um guys man way see look"
        .split_whitespace()
        .collect()
});

/// `KEEP_SHORT`: two-letter words that are subjects.
const KEEP_SHORT: [&str; 12] = ["ai", "tv", "dj", "pc", "uk", "la", "ny", "gf", "bf", "ex", "go", "us"];

/// `MONTHS`.
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `_norm(w)`: the light stemmer (possessive off, `ies` to `y`, `es` after a sibilant off, a plural `s` off).
fn norm(w: &str) -> String {
    let mut w = w.trim_matches('\'');
    if let Some(rest) = w.strip_suffix("'s") {
        w = rest;
    }
    let n = w.len(); // ASCII by construction, so bytes are code points
    if n > 4
        && let Some(stem) = w.strip_suffix("ies")
    {
        return format!("{stem}y");
    }
    if n > 4 && (w.ends_with("sses") || w.ends_with("xes") || w.ends_with("ches") || w.ends_with("shes")) {
        return w[..n - 2].to_string();
    }
    if n > 3 && w.ends_with('s') && !w.ends_with("ss") {
        return w[..n - 1].to_string();
    }
    w.to_string()
}

/// `tokens(text)`: the search words of a text, in order, repeats kept.
pub fn tokens(text: &str) -> Vec<String> {
    let low = text.to_lowercase();
    TOK.find_iter(&low)
        .map(|m| norm(m.as_str()))
        .filter(|w| (w.len() >= 3 || KEEP_SHORT.contains(&w.as_str())) && !STOP.contains(w.as_str()))
        .collect()
}

/// `short_title(t)`: the title up to its first ` (`, trimmed.
pub fn short_title(t: Option<&str>) -> String {
    let t = t.unwrap_or("");
    let head = t.split(" (").next().unwrap_or("");
    head.trim_matches(py::isspace).to_string()
}

/// `month_year(date)`: `2015-06-26` to `Jun 2015`; `2015` to `2015`.
pub fn month_year(date: Option<&str>) -> String {
    let d: Vec<char> = date.unwrap_or("").chars().collect();
    let year: String = d.iter().take(4).collect();
    if d.len() >= 7
        && let (Some(tens), Some(ones)) = (d[5].to_digit(10), d[6].to_digit(10))
        && (1..=12).contains(&(tens * 10 + ones))
    {
        return format!("{} {year}", MONTHS[(tens * 10 + ones) as usize - 1]);
    }
    year
}

/// `mmss(seconds)`: `m:ss`, the seconds rounded the way Python's `round` does (ties to even).
pub fn mmss(seconds: f64) -> String {
    let s = seconds.round_ties_even() as i64;
    format!("{}:{:02}", s.div_euclid(60), s.rem_euclid(60))
}

/// The first index in `hay` where `needle` starts (`str.find` over code points).
fn find_chars(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

/// `_snippet(text, terms, n)`: a text at most `n` code points long is itself; a longer one is a window starting a
/// quarter window before the first search term found (moved to the next space) and ending on a space, marked
/// `[...]` where it was cut.
pub fn snippet(text: &str, terms: &[String], n: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if len <= n {
        return text.to_string();
    }
    let low: Vec<char> = text.to_lowercase().chars().collect();
    let mut hits: Vec<usize> = terms
        .iter()
        .filter_map(|t| find_chars(&low, &t.chars().collect::<Vec<_>>()))
        .collect();
    hits.sort_unstable();
    let mut at = hits.first().copied().unwrap_or(0).saturating_sub(n / 4);
    if at != 0
        && let Some(rest) = chars.get(at..)
        && let Some(p) = rest.iter().position(|c| *c == ' ')
    {
        at = at + p + 1;
    }
    let mut end = len.min(at + n);
    if end < len
        && at < end
        && let Some(p) = chars[at..end].iter().rposition(|c| *c == ' ')
        && p > 0
    {
        end = at + p;
    }
    let body: String = chars.get(at.min(end)..end).unwrap_or(&[]).iter().collect();
    let mut out = String::with_capacity(body.len() + 12);
    if at != 0 {
        out.push_str("[...] ");
    }
    out.push_str(&body);
    if end < len {
        out.push_str(" [...]");
    }
    out
}

/// `len(q.split())`: the number of whitespace-separated words (Python's `str.isspace` whitespace).
pub fn py_word_count(s: &str) -> usize {
    s.split(py::isspace).filter(|w| !w.is_empty()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_like_python() {
        assert_eq!(norm("dog's"), "dog");
        assert_eq!(norm("babies"), "baby");
        assert_eq!(
            tokens("babies boxes churches glasses buses ties"),
            ["baby", "box", "church", "glass", "buse", "tie"]
        );
        assert_eq!(
            tokens("The purple monkey's dishwasher"),
            ["purple", "monkey", "dishwasher"]
        );
        assert_eq!(tokens("it's its don't I'm we're"), ["we're"]);
        assert_eq!(tokens("AI TV us US go ex"), ["ai", "tv", "us", "us", "go", "ex"]);
        assert_eq!(tokens("2015 90s 100s 3 42"), ["2015", "90s", "100"]);
    }

    #[test]
    fn dates_and_clock() {
        assert_eq!(month_year(Some("2015-06-26")), "Jun 2015");
        assert_eq!(month_year(Some("2015")), "2015");
        assert_eq!(month_year(Some("2015-13-01")), "2015");
        assert_eq!(month_year(None), "");
        assert_eq!(mmss(70.5), "1:10");
        assert_eq!(mmss(71.5), "1:12");
        assert_eq!(mmss(3599.5), "60:00");
        assert_eq!(short_title(Some("A bit (v2)")), "A bit");
    }

    #[test]
    fn snippet_windows() {
        let t = "aaa bbb ccc ddd eee fff ggg hhh iii jjj KKK lll mmm nnn ooo ppp";
        assert_eq!(
            snippet(t, &["kkk".to_string()], 40),
            "[...] iii jjj KKK lll mmm nnn ooo ppp"
        );
        assert_eq!(snippet(t, &[], 40), "aaa bbb ccc ddd eee fff ggg hhh iii jjj [...]");
        assert_eq!(snippet(t, &[], 1000), t);
    }
}
