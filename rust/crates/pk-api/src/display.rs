//! The formatted values a list row carries, ported from `api-dev/server.py` (`short_title`, `punch_line`, `r2`) and
//! `creative/notebook.py` (`short_name_of`), plus the two pieces of `CPython` arithmetic the averages need to come out
//! bit-equal (`sum` over floats, `round(x, 2)`). The single home these move to is `pk_creative::display`
//! (docs/ARCHITECTURE.md section 4) once lane T8 has merged; kept in this crate until then so the two lanes do not edit one file.

use std::sync::LazyLock;

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
    pk_core::py::sum(xs.iter().copied())
}

/// `round(x, 2)`: the decimal correctly rounded from the exact binary value (ties to even), read back.
pub fn r2(x: f64) -> f64 {
    format!("{x:.2}").parse().unwrap_or(x)
}

/// `server.py` `suggest_rating`: the rating the laughs suggest.
#[expect(clippy::float_cmp, reason = "the Python compares the bar count exactly")]
pub fn suggest_rating(max_bars: f64) -> &'static str {
    if max_bars >= 3.0 {
        "killed"
    } else if max_bars == 2.0 {
        "worked"
    } else if max_bars == 1.0 {
        "meh"
    } else {
        "died"
    }
}

/// `server.py` `last_sentence_split`: `(setup, reveal)`, the reveal being the last sentence (the last four words of
/// a long run-on text). Whitespace runs become one space first.
pub fn last_sentence_split(text: &str) -> (String, String) {
    let t = pk_core::py::strip(text)
        .split(isspace)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    // `re.split(r"(?<=[.!?])\s+(?=\S)", t)`: t has single spaces and no ends, so every space after [.!?] splits
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    for c in t.chars() {
        if c == ' ' && matches!(prev, Some('.' | '!' | '?')) {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
        prev = Some(c);
    }
    parts.push(cur);
    parts.retain(|p| !p.is_empty());
    if parts.len() <= 1 {
        let words: Vec<&str> = t.split(' ').collect();
        if words.len() > 8 {
            let k = words.len() - 4;
            return (words[..k].join(" "), words[k..].join(" "));
        }
        return (String::new(), t);
    }
    let reveal = parts.pop().unwrap_or_default();
    (parts.join(" "), reveal)
}

/// `UNKNOWN_VENUE` (`creative/data.py`).
pub const UNKNOWN_VENUE: &str = "Unknown venue";

fn re(p: &str) -> regex::Regex {
    regex::Regex::new(p).unwrap_or_else(|e| unreachable!("constant regex {p}: {e}"))
}

static MAYBE: LazyLock<regex::Regex> = LazyLock::new(|| re(r"(?i)maybe ([^)';]+)"));
static UNKNOWN: LazyLock<regex::Regex> = LazyLock::new(|| re(r"(?i)\Aunknown"));
// `re.A`: the word boundary is ASCII, as in JavaScript
static GUESSED: LazyLock<regex::Regex> = LazyLock::new(|| re(r"(?i)(?-u:\b)(likely|maybe|\(show\))"));
static SHOW: LazyLock<regex::Regex> = LazyLock::new(|| re(r"(?i)\(show\)"));
static LIKELY_TAIL: LazyLock<regex::Regex> = LazyLock::new(|| re(r"(?i)(?-u:\b)likely(?-u:\b).*\z"));
static COMIX_AT: LazyLock<regex::Regex> = LazyLock::new(|| re(r"(?i)\AComix at\s+"));
static CLUB_TAIL: LazyLock<regex::Regex> = LazyLock::new(|| re(r"(?i)\s+(Comedy Club|Comedy Show)\z"));

/// `creative/data.py` `venue_label`: the short room name a list row shows, and whether it is a guess (pinned by
/// `contracts/venue_short.json`).
pub fn venue_label(venue: Option<&str>) -> (String, bool) {
    let r = pk_core::py::strip(venue.unwrap_or(""));
    if r.is_empty() {
        return (UNKNOWN_VENUE.to_string(), true);
    }
    let maybe = MAYBE.captures(r);
    if UNKNOWN.is_match(r) {
        return (
            maybe.map_or_else(
                || UNKNOWN_VENUE.to_string(),
                |m| format!("{}?", pk_core::py::strip(&m[1])),
            ),
            true,
        );
    }
    let guessed = GUESSED.is_match(r);
    let first = r.split(',').next().unwrap_or("");
    let name = SHOW.replacen(first, 1, "");
    let name = LIKELY_TAIL.replacen(&name, 1, "");
    let name = pk_core::py::strip(&name).to_string();
    let name = COMIX_AT.replacen(&name, 1, "Comix ");
    let name = CLUB_TAIL.replacen(&name, 1, "");
    let name = pk_core::py::strip(&name);
    (if guessed { format!("{name}?") } else { name.to_string() }, guessed)
}

/// `server.py` `when_label`: `2015-06-26` -> `Jun 2015`; `""` when there is no date.
pub fn when_label(date: Option<&serde_json::Value>) -> String {
    match date.and_then(serde_json::Value::as_str) {
        Some(d) if !d.is_empty() => pk_creative::ask::words::month_year(Some(d)),
        _ => String::new(),
    }
}

/// `server.py` `at_label`: a place on the tape as `m:ss`; `None` when it is not a number.
pub fn at_label(start_ms: Option<&serde_json::Value>) -> serde_json::Value {
    match start_ms {
        Some(serde_json::Value::Number(n)) => n.as_f64().map_or(serde_json::Value::Null, |ms| {
            pk_creative::ask::words::mmss(ms / 1000.0).into()
        }),
        _ => serde_json::Value::Null,
    }
}

static VENUE_MAYBE: LazyLock<regex::Regex> = LazyLock::new(|| re(r#"(?i)maybe ([^)'"]+)"#));

/// `creative/venues.py` `parse_venue(raw)["name"]`.
pub fn venue_name(raw: &str) -> String {
    let v = pk_core::py::strip(raw);
    let mut name = if let Some(m) = VENUE_MAYBE.captures(v) {
        pk_core::py::strip(&m[1]).to_string()
    } else {
        let base = pk_core::py::strip(v.split('(').next().unwrap_or(""));
        base.split(',')
            .map(pk_core::py::strip)
            .find(|p| !p.is_empty())
            .unwrap_or(v)
            .to_string()
    };
    if name.to_lowercase().starts_with("comix at ") {
        name = name.chars().skip(9).collect();
    }
    name
}

/// `creative/util.py` `slug(s, n)`.
pub fn slug(s: &str, n: usize) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c);
        } else {
            dash = true;
        }
    }
    let cut: String = out.chars().take(n).collect();
    let cut = cut.trim_matches('-');
    if cut.is_empty() {
        "untitled".into()
    } else {
        cut.to_string()
    }
}

/// `server.py` `venue_id_of`: the room id `/api/venues` gives the same room.
pub fn venue_id_of(raw: &str) -> String {
    format!("venue:{}", slug(&venue_name(raw), 40))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn venue_labels_follow_the_contract() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../contracts/venue_short.json");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let cases = doc["cases"].as_array().unwrap();
        assert!(cases.len() >= 30);
        for c in cases {
            let got = venue_label(c["venue"].as_str());
            assert_eq!(
                (got.0.as_str(), got.1),
                (c["name"].as_str().unwrap(), c["guessed"].as_bool().unwrap()),
                "{c}"
            );
        }
    }

    #[test]
    fn rooms_and_splits_as_the_python() {
        assert_eq!(
            venue_id_of("Mabel's Comedy Club, Springfield"),
            "venue:mabel-s-comedy-club"
        );
        assert_eq!(venue_id_of("Unknown (maybe The Loft)"), "venue:the-loft");
        assert_eq!(venue_id_of("Comix at Riverside"), "venue:riverside");
        assert_eq!(venue_id_of(""), "venue:untitled");
        assert_eq!(slug("--A  b--", 40), "a-b");
        assert_eq!(
            last_sentence_split("One.  Two!\nThree"),
            ("One. Two!".into(), "Three".into())
        );
        assert_eq!(
            last_sentence_split("a b c d e f g h i j"),
            ("a b c d e f".into(), "g h i j".into())
        );
        assert_eq!(last_sentence_split("short one"), (String::new(), "short one".into()));
        assert_eq!(suggest_rating(2.0), "worked");
        assert_eq!(at_label(Some(&serde_json::json!(187_000))), serde_json::json!("3:07"));
        assert_eq!(at_label(Some(&serde_json::json!(null))), serde_json::Value::Null);
    }

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
