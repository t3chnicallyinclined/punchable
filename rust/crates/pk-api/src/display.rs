//! The formatted values a list row carries, ported from `api-dev/server.py` (`short_title`, `punch_line`, `r2`) and
//! `creative/notebook.py` (`short_name_of`), plus the two pieces of `CPython` arithmetic the averages need to come out
//! bit-equal (`sum` over floats, `round(x, 2)`). The single home these move to is `pk_creative::display`
//! (docs/ARCHITECTURE.md section 4) once lane T8 has merged; kept in this crate until then so the two lanes do not edit one file.

use pk_core::py::isspace;

/// `SHORT_NAME_MAX` in `creative/notebook.py`.
const SHORT_NAME_MAX: usize = 18;

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

/// `str.rstrip(set)` over the given characters.
fn rstrip_set<'a>(s: &'a str, set: &str) -> &'a str {
    s.trim_end_matches(|c| set.contains(c))
}

/// `server.py` `short_title`: the title cut at its first ` (`, at most 28 characters (27 and `…`).
pub fn short_title(title: &str) -> String {
    let first = pk_core::py::strip(title.split(" (").next().unwrap_or(""));
    let t = if first.is_empty() {
        pk_core::py::strip(title)
    } else {
        first
    };
    let cs = chars(t);
    if cs.len() <= 28 {
        t.to_string()
    } else {
        let head: String = cs[..27].iter().collect();
        format!("{}…", head.trim_end_matches(isspace))
    }
}

/// `creative/notebook.py` `short_name_of`: his own short name when he set one, else the title cut at its first
/// parenthesis and at a word break inside 18 characters.
pub fn short_name_of(title: &str, custom: Option<&str>) -> String {
    if let Some(c) = custom.map(pk_core::py::strip).filter(|c| !c.is_empty()) {
        return c.to_string();
    }
    let st = pk_core::py::strip(title.split(" (").next().unwrap_or(""));
    let t = if st.is_empty() { pk_core::py::strip(title) } else { st };
    let cs = chars(t);
    if cs.len() <= SHORT_NAME_MAX {
        return t.to_string();
    }
    let cut: Vec<char> = cs[..=SHORT_NAME_MAX].to_vec();
    let sp = cut.iter().rposition(|c| *c == ' ');
    let base: String = match sp {
        Some(i) if i >= 4 => cut[..i].iter().collect(),
        _ => cs[..SHORT_NAME_MAX].iter().collect(),
    };
    let out = rstrip_set(&base, " ,.;:!?-");
    if out.is_empty() {
        cs[..SHORT_NAME_MAX].iter().collect()
    } else {
        out.to_string()
    }
}

const SENT_END: [char; 4] = ['.', '!', '?', '…'];
const CLOSERS: [char; 6] = ['"', '\'', '”', '’', ')', ']'];

/// The end (char index) of the last match of `[.!?…]["'”’)\]]*\s+(?=\S)` in `t`, scanning as `re.finditer` does.
fn last_sentence_end(t: &[char]) -> Option<usize> {
    let mut last = None;
    let mut i = 0;
    while i < t.len() {
        if SENT_END.contains(&t[i]) {
            let mut j = i + 1;
            while j < t.len() && CLOSERS.contains(&t[j]) {
                j += 1;
            }
            let ws = j;
            while j < t.len() && isspace(t[j]) {
                j += 1;
            }
            if j > ws && j < t.len() {
                last = Some(j);
                i = j;
                continue;
            }
        }
        i += 1;
    }
    last
}

/// `server.py` `punch_line`: the last sentence of `text` (the last 14 words of a long one), always an exact
/// substring; `None` when there is nothing to show.
pub fn punch_line(text: &str) -> Option<String> {
    let t = text.trim_end_matches(isspace);
    if pk_core::py::strip(t).is_empty() {
        return None;
    }
    let tc = chars(t);
    let mut p: Vec<char> = match last_sentence_end(&tc) {
        Some(e) => tc[e..].to_vec(),
        None => tc,
    };
    let words = p.split(|c| isspace(*c)).filter(|w| !w.is_empty()).count();
    if words > 16 || p.len() > 140 {
        // `(?:\S+\s+){0,13}\S+\s*$`, leftmost: from the start of the 14th word from the end (or the first word)
        let starts: Vec<usize> = (0..p.len())
            .filter(|&k| !isspace(p[k]) && (k == 0 || isspace(p[k - 1])))
            .collect();
        if let Some(&s) = starts.len().checked_sub(14).map_or(starts.first(), |k| starts.get(k)) {
            p = p[s..].to_vec();
        }
    }
    let out: String = p.into_iter().collect();
    let out = pk_core::py::strip(&out);
    (!out.is_empty() && text.contains(out)).then(|| out.to_string())
}

/// `CPython`'s `sum()` over floats (3.12+: Neumaier compensated summation).
pub fn py_sum(xs: &[f64]) -> f64 {
    let (mut f, mut c) = (0.0_f64, 0.0_f64);
    for &x in xs {
        let t = f + x;
        if f.abs() >= x.abs() {
            c += (f - t) + x;
        } else {
            c += (x - t) + f;
        }
        f = t;
    }
    if c != 0.0 && c.is_finite() { f + c } else { f }
}

/// `round(x, 2)`: the decimal correctly rounded from the exact binary value (ties to even), read back.
pub fn r2(x: f64) -> f64 {
    format!("{x:.2}").parse().unwrap_or(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_follow_the_notebook() {
        assert_eq!(short_name_of("Haunted dishwasher saga", None), "Haunted dishwasher");
        assert_eq!(short_name_of("Bravo bit (the dog one)", None), "Bravo bit");
        assert_eq!(short_name_of("x", Some("  Mine ")), "Mine");
        assert_eq!(short_name_of("x", Some("   ")), "x");
        assert_eq!(short_name_of("Abcdefghijklmnopqrstuvwxyz", None), "Abcdefghijklmnopqr");
        // values from CPython (creative.notebook.short_name_of)
        assert_eq!(short_name_of("Abc defghijklmnopqrstuvwxyz", None), "Abc defghijklmnopq");
        assert_eq!(short_name_of("Hello, world, again and again", None), "Hello, world");
    }

    #[test]
    fn short_titles_cut_at_28() {
        assert_eq!(short_title("Bravo bit (the dog one)"), "Bravo bit");
        assert_eq!(short_title(" (x)"), "(x)");
        let long = "a".repeat(40);
        assert_eq!(short_title(&long), format!("{}…", "a".repeat(27)));
    }

    #[test]
    fn punch_lines_are_the_last_sentence() {
        assert_eq!(punch_line("One. Two three."), Some("Two three.".into()));
        assert_eq!(punch_line("One!\" Two"), Some("Two".into()));
        assert_eq!(punch_line("No end"), Some("No end".into()));
        assert_eq!(punch_line("   "), None);
        assert_eq!(punch_line("Trailing. "), Some("Trailing.".into()));
        let long: Vec<String> = (0..20).map(|i| format!("w{i}")).collect();
        let s = long.join(" ");
        assert_eq!(punch_line(&s), Some(long[6..].join(" ")));
    }

    #[test]
    #[allow(clippy::float_cmp)] // bit-equal to CPython is the point
    fn cpython_arithmetic() {
        assert_eq!(r2(0.125), 0.12);
        assert_eq!(r2(0.375), 0.38);
        assert_eq!(r2(2.675), 2.67);
        assert_eq!(py_sum(&[0.1, 0.2, 0.3]), 0.6);
        assert_eq!(py_sum(&[1e16, 1.0, -1e16]), 1.0);
    }
}
