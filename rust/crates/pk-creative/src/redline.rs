//! The redline round on a bit (`docs/design/SPEC-2026-11-16-bits-sets-tapes.md` §3.2 and Slice 3 as built; B12: the
//! owner's own Redline loop). He selects words in his document (or a whole line) and marks them Change (an optional
//! note), Ask (a question) or Keep; the marks are sent once, in document order, each anchored to the words it quotes,
//! with the whole bit; what comes back is a WHOLE NEW DRAFT plus the answers to his questions (owner, 2026-11-17: "lets
//! make it a whole new draft"). These are the rules the route (`pk-api` `writes::redline`) and the worker kind
//! (`pk-worker` `kinds::redline`) share, with no IO:
//!
//! - [`locate`]: where a quoted mark sits in the text, by its own words (B1: lines have no ids), the occurrence nearest
//!   the phone's hint (a UTF-16 index, the phone's own count) when the words occur more than once;
//! - [`ideas`]: the bit's ideas, each run of lines between blank lines;
//! - [`parts`]: what the model rewrites. A bit of at most [`WHOLE_CAP`] characters is rewritten whole; a longer one
//!   only around its marks (each idea a Change or an Ask is in, and one idea either side), with the whole bit still sent
//!   as context, so the cost of a round follows the marks, not the length of the document. [`assemble`] puts the
//!   rewritten parts back; every word outside them is his, byte for byte, by construction;
//! - the checks on a draft, each of which refuses it (it is retried once, then never shown): [`broken_keeps`] (a Keep's
//!   words must come back byte for byte) and [`dropped_ideas`] (an idea he did not mark must not vanish: fewer than
//!   [`KEPT_SHARE`] of its content words left anywhere in the draft is "dropped"; the lead's number, see below).

use std::collections::HashSet;
use std::ops::Range;

/// Marks in one round, at most.
pub const MAX_MARKS: usize = 20;
/// One mark's quoted words, at most (code points).
pub const MAX_QUOTE_CHARS: usize = 4000;
/// One mark's note or question, at most (code points).
pub const MAX_NOTE_CHARS: usize = 500;
/// A bit up to this many characters is rewritten whole; a longer one only around its marks ([`parts`]). The lead's
/// number (2026-11-17): about 3,000 tokens of draft, where a full 40,000-character version would be about 10,000
/// tokens of Opus output per round, every round.
pub const WHOLE_CAP: usize = 12_000;
/// An idea he did not mark is dropped when fewer than this share of its content words are anywhere in the draft. The
/// lead's number (2026-11-17), not measured on his material: a reworded line keeps most of its long words; an idea
/// that is gone keeps almost none.
pub const KEPT_SHARE: f64 = 0.5;
/// A content word has at least this many letters or digits ("the", "and", "was" are not evidence of anything).
pub const CONTENT_WORD_LEN: usize = 4;
/// An idea with fewer content words than this is too short to judge, and is never called dropped.
pub const MIN_CONTENT_WORDS: usize = 2;

/// What a mark asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// This should change (and, with a note, how).
    Change,
    /// Answer this before revising.
    Ask,
    /// Do not touch this.
    Keep,
}

impl Kind {
    /// The wire name.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Change => "change",
            Kind::Ask => "ask",
            Kind::Keep => "keep",
        }
    }

    /// The kind with that wire name.
    pub fn parse(s: &str) -> Option<Kind> {
        [Kind::Change, Kind::Ask, Kind::Keep]
            .into_iter()
            .find(|k| k.name() == s)
    }

    /// The word the prompt uses (Redline's own: EDIT is CHANGE here, his word on the phone).
    pub fn label(self) -> &'static str {
        match self {
            Kind::Change => "CHANGE",
            Kind::Ask => "QUESTION",
            Kind::Keep => "KEEP",
        }
    }
}

/// How he asked for a Change, when he asked from the selection's own buttons (the one-notebook page,
/// `docs/design/SPEC-2026-11-17-one-notebook.md` §4): a mark with no intent is a plain Change with his note, as before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Intent {
    /// Make these words funnier.
    Funnier,
    /// Say it in fewer words.
    Shorter,
    /// Three takes on these words: one in the draft, all three listed beside it.
    Options,
    /// He passed on the take the draft has here: write another.
    Again,
}

/// How many takes an [`Intent::Options`] mark asks for.
pub const OPTION_COUNT: usize = 3;

impl Intent {
    /// The wire name.
    pub fn name(self) -> &'static str {
        match self {
            Intent::Funnier => "funnier",
            Intent::Shorter => "shorter",
            Intent::Options => "options",
            Intent::Again => "again",
        }
    }

    /// The intent with that wire name.
    pub fn parse(s: &str) -> Option<Intent> {
        [Intent::Funnier, Intent::Shorter, Intent::Options, Intent::Again]
            .into_iter()
            .find(|k| k.name() == s)
    }

    /// The line the prompt puts under the quoted words.
    pub fn ask(self) -> &'static str {
        match self {
            Intent::Funnier => "Make this funnier.",
            Intent::Shorter => "Make this shorter: fewer words, the same joke.",
            Intent::Options => {
                "Give 3 options for these words: put the one you would pick in the draft and list all three under `options`."
            }
            Intent::Again => "He passed on this take. Write a different one.",
        }
    }
}

/// One mark, located in the text it was made on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    /// Change, Ask or Keep.
    pub kind: Kind,
    /// His words, exactly as they stand in the text.
    pub text: String,
    /// Where they start in the text (a byte offset on a character boundary).
    pub start: usize,
    /// His note (a Change) or his question (an Ask); empty for a Keep.
    pub note: String,
    /// A Keep from an earlier round on the draft this round marks (round two): sent again so nothing kept is lost.
    pub carried: bool,
    /// How he asked for a Change (the selection's buttons); `None` on a plain Change, an Ask and a Keep.
    pub intent: Option<Intent>,
}

impl Mark {
    /// The byte range of its words.
    pub fn range(&self) -> Range<usize> {
        self.start..self.start + self.text.len()
    }

    /// Whether its words are exactly `text[start..]` (it was located in this text).
    pub fn sits_in(&self, text: &str) -> bool {
        text.get(self.range()) == Some(self.text.as_str())
    }
}

/// The phone's index (UTF-16 code units) of a byte offset. `byte` past the end counts the whole text.
pub fn utf16_of(text: &str, byte: usize) -> usize {
    let mut b = byte.min(text.len());
    while !text.is_char_boundary(b) {
        b -= 1;
    }
    text[..b].encode_utf16().count()
}

/// Where `quote` sits in `text`: of every exact occurrence (overlapping ones included), the one whose UTF-16 index is
/// nearest `hint`, else the first. `None` for an empty quote or one the text does not hold.
pub fn locate(text: &str, quote: &str, hint: Option<usize>) -> Option<usize> {
    if quote.is_empty() {
        return None;
    }
    let mut best: Option<(usize, usize)> = None;
    let (mut from, mut u16_at, mut counted) = (0usize, 0usize, 0usize);
    while let Some(k) = text[from..].find(quote) {
        let at = from + k;
        u16_at += text[counted..at].encode_utf16().count();
        counted = at;
        let Some(h) = hint else { return Some(at) };
        let d = u16_at.abs_diff(h);
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((at, d));
        }
        from = at + text[at..].chars().next().map_or(1, char::len_utf8);
    }
    best.map(|(at, _)| at)
}

/// The bit's ideas: each run of lines with words, as a byte range from its first line's start to its last line's end
/// (the newline after it not included). A blank line (only spaces) separates two ideas; a run of them is one gap.
pub fn ideas(text: &str) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut cur: Option<Range<usize>> = None;
    let mut pos = 0usize;
    for line in text.split('\n') {
        let end = pos + line.len();
        if line.trim().is_empty() {
            if let Some(r) = cur.take() {
                out.push(r);
            }
        } else {
            let body_end = pos + line.trim_end().len();
            cur = Some(cur.map_or(pos..body_end, |r| r.start..body_end));
        }
        pos = end + 1;
    }
    out.extend(cur);
    out
}

/// The ideas a byte range overlaps (indexes into `ideas`).
fn ideas_over(ideas: &[Range<usize>], r: &Range<usize>) -> Vec<usize> {
    ideas
        .iter()
        .enumerate()
        .filter(|(_, x)| x.start < r.end.max(r.start + 1) && r.start < x.end)
        .map(|(i, _)| i)
        .collect()
}

/// The idea a mark starts in, 1-based, for the prompt's "idea 2"; `None` when it starts in a gap.
pub fn idea_no(ideas: &[Range<usize>], m: &Mark) -> Option<usize> {
    ideas_over(ideas, &m.range()).first().map(|i| i + 1)
}

/// The ideas a Change or an Ask is in: the ones a revision may rewrite.
pub fn touched(ideas: &[Range<usize>], marks: &[Mark]) -> HashSet<usize> {
    marks
        .iter()
        .filter(|m| m.kind != Kind::Keep)
        .flat_map(|m| ideas_over(ideas, &m.range()))
        .collect()
}

/// Content words: the lower-cased runs of letters and digits of at least [`CONTENT_WORD_LEN`].
pub fn content_words(s: &str) -> HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= CONTENT_WORD_LEN)
        .map(str::to_lowercase)
        .collect()
}

/// The ideas of `base` he did not mark (no Change or Ask in them) that the draft lost wholesale: fewer than
/// [`KEPT_SHARE`] of their content words are anywhere in `draft`. Indexes into [`ideas`]`(base)`.
pub fn dropped_ideas(base: &str, draft: &str, marks: &[Mark]) -> Vec<usize> {
    dropped_ideas_at(base, draft, marks, KEPT_SHARE)
}

/// [`dropped_ideas`] with its share as an argument (the tests prove the check, and its threshold, can fail).
pub fn dropped_ideas_at(base: &str, draft: &str, marks: &[Mark], share: f64) -> Vec<usize> {
    let all = ideas(base);
    let hit = touched(&all, marks);
    let have = content_words(draft);
    all.iter()
        .enumerate()
        .filter(|(i, _)| !hit.contains(i))
        .filter_map(|(i, r)| {
            let words = content_words(&base[r.clone()]);
            if words.len() < MIN_CONTENT_WORDS {
                return None;
            }
            let kept = words.iter().filter(|w| have.contains(*w)).count();
            #[allow(clippy::cast_precision_loss)] // word counts of one idea
            let gone = (kept as f64) < share * words.len() as f64;
            gone.then_some(i)
        })
        .collect()
}

/// The Keep marks whose words are not in the draft byte for byte (indexes into `marks`).
pub fn broken_keeps(draft: &str, marks: &[Mark]) -> Vec<usize> {
    marks
        .iter()
        .enumerate()
        .filter(|(_, m)| m.kind == Kind::Keep && !draft.contains(m.text.as_str()))
        .map(|(i, _)| i)
        .collect()
}

/// A Keep and a Change or an Ask on overlapping words (indexes into `marks`): he cannot both keep words and have them
/// changed.
pub fn conflict(marks: &[Mark]) -> Option<(usize, usize)> {
    for (i, a) in marks.iter().enumerate() {
        for (j, b) in marks.iter().enumerate().skip(i + 1) {
            let keep_and_other = (a.kind == Kind::Keep) != (b.kind == Kind::Keep);
            let a_r = a.range();
            let b_r = b.range();
            if keep_and_other
                && a_r.start < b_r.end
                && b_r.start < a_r.end
                && b.kind != Kind::Ask
                && a.kind != Kind::Ask
            {
                return Some((i, j));
            }
        }
    }
    None
}

/// Whether the bit is rewritten whole (at most [`WHOLE_CAP`] characters).
pub fn is_whole(base: &str) -> bool {
    base.chars().count() <= WHOLE_CAP
}

/// What the model rewrites, as byte ranges of `base` in order: the whole text when [`is_whole`], else each idea a
/// Change or an Ask is in with one idea either side, neighbouring windows merged. A long bit whose marks are all Keeps
/// has no part (the route refuses such a round: at least one Change or Ask).
pub fn parts(base: &str, marks: &[Mark]) -> Vec<Range<usize>> {
    if is_whole(base) {
        return std::iter::once(0..base.len()).collect();
    }
    let all = ideas(base);
    let mut hit: Vec<usize> = touched(&all, marks).into_iter().collect();
    hit.sort_unstable();
    let mut windows: Vec<(usize, usize)> = Vec::new();
    for i in hit {
        let (lo, hi) = (i.saturating_sub(1), (i + 1).min(all.len().saturating_sub(1)));
        match windows.last_mut() {
            Some(w) if lo <= w.1 + 1 => w.1 = w.1.max(hi),
            _ => windows.push((lo, hi)),
        }
    }
    windows.into_iter().map(|(lo, hi)| all[lo].start..all[hi].end).collect()
}

/// The draft: `base` with each part replaced by its new text, the blank lines at a new text's ends dropped (the gaps
/// between ideas outside the parts are his and stay). `texts` pairs with `parts`; a missing text leaves its part as
/// it was.
pub fn assemble(base: &str, parts: &[Range<usize>], texts: &[String]) -> String {
    let mut out = String::with_capacity(base.len());
    let mut at = 0usize;
    for (k, r) in parts.iter().enumerate() {
        out.push_str(&base[at..r.start]);
        match texts.get(k) {
            Some(t) => out.push_str(t.trim_matches(|c| c == '\n' || c == '\r')),
            None => out.push_str(&base[r.clone()]),
        }
        at = r.end;
    }
    out.push_str(&base[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIT: &str = "My fridge has an app now.\nIt texts me when the milk is sad.\n\n\
The doorbell has a camera too.\nNow my front door judges my outfits.\n\n\
My parrot only says legal disclaimers.\nHe will not confirm or deny the crackers.";

    #[test]
    fn an_intent_has_one_wire_name_and_an_unknown_one_is_none() {
        for i in [Intent::Funnier, Intent::Shorter, Intent::Options, Intent::Again] {
            assert_eq!(Intent::parse(i.name()), Some(i));
            assert!(!i.ask().is_empty());
        }
        assert_eq!(Intent::parse("louder"), None);
        assert_eq!(Intent::parse(""), None);
    }

    fn mark(kind: Kind, text: &str, note: &str) -> Mark {
        let start = locate(BIT, text, None).expect("in the bit");
        Mark {
            kind,
            text: text.into(),
            start,
            note: note.into(),
            carried: false,
            intent: None,
        }
    }

    #[test]
    fn a_quote_is_found_by_its_words_nearest_the_hint() {
        let t = "the car, the cart, the car";
        assert_eq!(locate(t, "the car", None), Some(0));
        assert_eq!(locate(t, "the car", Some(20)), Some(19), "the last one is nearest");
        assert_eq!(
            locate(t, "the car", Some(9)),
            Some(9),
            "inside 'the cart' counts: the words are exact"
        );
        assert_eq!(locate(t, "the bus", Some(0)), None);
        assert_eq!(locate(t, "", Some(0)), None);
        // the hint is the phone's count (UTF-16): an emoji before is two units, four bytes
        let e = "\u{1F600} ok \u{1F600} ok";
        assert_eq!(locate(e, "ok", Some(8)), Some(e.rfind("ok").unwrap()));
        assert_eq!(utf16_of(e, e.rfind("ok").unwrap()), 9);
        assert_eq!(
            locate(e, "ok", Some(6)),
            Some(e.find("ok").unwrap()),
            "a tie goes to the first"
        );
        // overlapping occurrences are all candidates
        assert_eq!(locate("aaa", "aa", Some(1)), Some(1));
    }

    #[test]
    fn ideas_are_runs_of_lines_between_blank_lines() {
        let r = ideas(BIT);
        assert_eq!(r.len(), 3);
        assert_eq!(
            &BIT[r[0].clone()],
            "My fridge has an app now.\nIt texts me when the milk is sad."
        );
        assert!(BIT[r[2].clone()].ends_with("crackers."));
        assert_eq!(
            ideas("\n\n  \na\n \n\nb  \n").len(),
            2,
            "a run of blank lines is one gap; spaces are blank"
        );
        assert_eq!(
            ideas("a  \n\nb"),
            vec![0..1, 5..6],
            "trailing spaces are not the idea's"
        );
    }

    #[test]
    fn an_unmarked_idea_that_is_gone_is_caught_and_a_reworded_one_is_not() {
        let marks = [mark(Kind::Change, "the milk is sad", "funnier")];
        let kept = BIT.replace("the milk is sad", "the milk has given up");
        assert_eq!(dropped_ideas(BIT, &kept, &marks), Vec::<usize>::new());
        // the parrot idea reworded a little: most long words stay
        let reworded = kept.replace("He will not confirm", "He won't confirm");
        assert_eq!(dropped_ideas(BIT, &reworded, &marks), Vec::<usize>::new());
        // the doorbell idea gone wholesale
        let gone = kept.replace(
            "The doorbell has a camera too.\nNow my front door judges my outfits.\n\n",
            "",
        );
        assert_eq!(dropped_ideas(BIT, &gone, &marks), vec![1]);
        // the marked idea may change as much as the mark asks
        let rewritten = gone.replace(
            "My fridge has an app now.\nIt texts me when the milk has given up.",
            "Smart fridge.",
        );
        assert!(!dropped_ideas(BIT, &rewritten, &marks).contains(&0));
        // the red arm: with no share required, nothing is ever called dropped
        assert_eq!(dropped_ideas_at(BIT, &gone, &marks, 0.0), Vec::<usize>::new());
    }

    #[test]
    fn a_keep_must_come_back_byte_for_byte() {
        let marks = [
            mark(Kind::Keep, "My parrot only says legal disclaimers.", ""),
            mark(Kind::Change, "judges my outfits", ""),
        ];
        assert_eq!(broken_keeps(BIT, &marks), Vec::<usize>::new());
        let curly = BIT.replace("legal disclaimers.", "legal disclaimers!");
        assert_eq!(broken_keeps(&curly, &marks), vec![0]);
        let spaced = BIT.replace("only says", "only  says");
        assert_eq!(broken_keeps(&spaced, &marks), vec![0], "two spaces are not his words");
    }

    #[test]
    fn a_keep_and_a_change_on_the_same_words_conflict() {
        let a = mark(Kind::Keep, "Now my front door judges my outfits.", "");
        let b = mark(Kind::Change, "judges my outfits", "");
        let c = mark(Kind::Ask, "judges", "why?");
        assert_eq!(conflict(&[a.clone(), b.clone()]), Some((0, 1)));
        assert_eq!(conflict(&[a.clone(), c]), None, "a question on kept words is fine");
        assert_eq!(conflict(&[b, mark(Kind::Keep, "My fridge has an app now.", "")]), None);
    }

    #[test]
    fn a_long_bit_is_rewritten_around_its_marks_and_the_rest_is_his_byte_for_byte() {
        // twenty ideas, well over the cap
        let ideas_text: Vec<String> = (0..20)
            .map(|i| {
                format!(
                    "Idea number {i} has a premise about thing {i}.\n{}",
                    "and a long middle line. ".repeat(40)
                )
            })
            .collect();
        let base = ideas_text.join("\n\n");
        assert!(!is_whole(&base));
        let q = "Idea number 7 has a premise";
        let m = Mark {
            kind: Kind::Change,
            text: q.into(),
            start: locate(&base, q, None).unwrap(),
            note: String::new(),
            carried: false,
            intent: None,
        };
        let p = parts(&base, std::slice::from_ref(&m));
        assert_eq!(p.len(), 1);
        let all = ideas(&base);
        assert_eq!(p[0], all[6].start..all[8].end, "the idea and one either side");
        let draft = assemble(&base, &p, &["\nNEW PART\n\n".to_string()]);
        assert!(draft.starts_with(&base[..all[6].start]) && draft.ends_with(&base[all[8].end..]));
        assert!(
            draft.contains("\n\nNEW PART \n\n"),
            "the gaps (and his trailing space) outside the part are his"
        );
        // two marks far apart: two parts; two close together: one
        let far = Mark {
            start: locate(&base, "Idea number 15", None).unwrap(),
            text: "Idea number 15".into(),
            ..m.clone()
        };
        assert_eq!(parts(&base, &[m.clone(), far]).len(), 2);
        let near = Mark {
            start: locate(&base, "Idea number 9", None).unwrap(),
            text: "Idea number 9".into(),
            ..m.clone()
        };
        assert_eq!(parts(&base, &[m, near]).len(), 1);
        // a short bit is one part, the whole text
        let whole = parts(BIT, &[]);
        assert_eq!((whole.len(), whole.first().cloned()), (1, Some(0..BIT.len())));
    }
}
