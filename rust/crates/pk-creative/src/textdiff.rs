//! Word normalisation and wording comparison between stage transcripts (`infer/scriptdiff.py::ntok`,
//! `creative/textdiff.py`): `ntok` (lower case, straight apostrophe, ASCII letters, digits and apostrophes only,
//! small numbers spelled out), `toks`, `phrases` (verbatim clauses), and `present` / `coverage` over difflib's
//! `SequenceMatcher` matching blocks (no junk, no autojunk), ported as-is so the same words count.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

static W: LazyLock<Regex> = LazyLock::new(|| Regex::new("[A-Za-z0-9\u{2019}']+").expect("a valid regex"));
static CLAUSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^.!?,;]+[.!?]?").expect("a valid regex"));

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

/// `toks(text)`: the comparison words of a text.
pub fn toks(text: &str) -> Vec<String> {
    W.find_iter(text)
        .map(|m| ntok(m.as_str()))
        .filter(|t| !t.is_empty())
        .collect()
}

/// `SequenceMatcher(None, a, b, autojunk=False).get_matching_blocks()`: (i, j, size), adjacent blocks merged, the
/// `(len a, len b, 0)` sentinel last.
#[allow(clippy::similar_names, clippy::many_single_char_names)] // difflib's own names, kept to be checked against it
pub fn matching_blocks(a: &[String], b: &[String]) -> Vec<(usize, usize, usize)> {
    let mut b2j: HashMap<&str, Vec<usize>> = HashMap::new();
    for (j, x) in b.iter().enumerate() {
        b2j.entry(x.as_str()).or_default().push(j);
    }
    let longest = |alo: usize, ahi: usize, blo: usize, bhi: usize| -> (usize, usize, usize) {
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0usize);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        for (i, ai) in a.iter().enumerate().take(ahi).skip(alo) {
            let mut next: HashMap<usize, usize> = HashMap::new();
            for &j in b2j.get(ai.as_str()).map_or(&[][..], Vec::as_slice) {
                if j < blo {
                    continue;
                }
                if j >= bhi {
                    break;
                }
                let k = j.checked_sub(1).and_then(|p| j2len.get(&p)).copied().unwrap_or(0) + 1;
                next.insert(j, k);
                if k > bestsize {
                    (besti, bestj, bestsize) = (i + 1 - k, j + 1 - k, k);
                }
            }
            j2len = next;
        }
        while besti > alo && bestj > blo && a[besti - 1] == b[bestj - 1] {
            (besti, bestj, bestsize) = (besti - 1, bestj - 1, bestsize + 1);
        }
        while besti + bestsize < ahi && bestj + bestsize < bhi && a[besti + bestsize] == b[bestj + bestsize] {
            bestsize += 1;
        }
        (besti, bestj, bestsize)
    };
    let mut queue = vec![(0, a.len(), 0, b.len())];
    let mut blocks = Vec::new();
    while let Some((alo, ahi, blo, bhi)) = queue.pop() {
        let (i, j, k) = longest(alo, ahi, blo, bhi);
        if k > 0 {
            blocks.push((i, j, k));
            if alo < i && blo < j {
                queue.push((alo, i, blo, j));
            }
            if i + k < ahi && j + k < bhi {
                queue.push((i + k, ahi, j + k, bhi));
            }
        }
    }
    blocks.sort_unstable();
    let mut out = Vec::new();
    let (mut i1, mut j1, mut k1) = (0, 0, 0);
    for (i2, j2, k2) in blocks {
        if i1 + k1 == i2 && j1 + k1 == j2 {
            k1 += k2;
        } else {
            if k1 > 0 {
                out.push((i1, j1, k1));
            }
            (i1, j1, k1) = (i2, j2, k2);
        }
    }
    if k1 > 0 {
        out.push((i1, j1, k1));
    }
    out.push((a.len(), b.len(), 0));
    out
}

/// `coverage(phrase, text)`: the share of the phrase's words said in order, in runs of two or more.
pub fn coverage(phrase: &str, text: &str) -> f64 {
    let (p, t) = (toks(phrase), toks(text));
    if p.is_empty() || t.is_empty() {
        return 0.0;
    }
    let hit: usize = matching_blocks(&p, &t).iter().map(|b| b.2).filter(|n| *n >= 2).sum();
    hit as f64 / p.len() as f64
}

/// `present(phrase, text, thr)`.
pub fn present(phrase: &str, text: &str, thr: f64) -> bool {
    coverage(phrase, text) >= thr
}

/// `phrases(text)`: verbatim clauses of 5 to 14 words (longer ones cut into 14-word windows), repeats out.
pub fn phrases(text: &str) -> Vec<String> {
    const MIN: usize = 5;
    const MAX: usize = 14;
    let mut out: Vec<&str> = Vec::new();
    for m in CLAUSE.find_iter(text) {
        let seg = m.as_str();
        // Python's `\S+`: runs of characters `str.isspace` does not count as space
        let words = crate::corpus::word_spans(seg);
        if words.len() < MIN {
            continue;
        }
        if words.len() <= MAX {
            out.push(&seg[words[0].0..words[words.len() - 1].1]);
            continue;
        }
        for w in words.chunks(MAX) {
            if w.len() >= MIN {
                out.push(&seg[w[0].0..w[w.len() - 1].1]);
            }
        }
    }
    let mut seen: HashSet<String> = HashSet::new();
    out.into_iter()
        .filter(|p| {
            let k = toks(p).join(" ");
            !k.is_empty() && seen.insert(k)
        })
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_python() {
        assert_eq!(ntok("Won\u{2019}t"), "won't");
        assert_eq!(ntok("10"), "ten");
        assert_eq!(ntok("'Hello!'"), "hello");
        assert_eq!(ntok("\u{130}"), "i");
        assert_eq!(ntok("\u{df}eta"), "eta");
        assert_eq!(ntok("—"), "");
    }

    #[test]
    fn matching_blocks_like_difflib() {
        let w = |s: &str| s.split(' ').map(str::to_string).collect::<Vec<_>>();
        // difflib: SequenceMatcher(None, "a b c d".split(), "a x c d".split()).get_matching_blocks()
        assert_eq!(
            matching_blocks(&w("a b c d"), &w("a x c d")),
            [(0, 0, 1), (2, 2, 2), (4, 4, 0)]
        );
        assert_eq!(matching_blocks(&w("a b"), &w("c")), [(2, 1, 0)]);
    }
}
