//! The answer check: every rule of upstream's `ask::check` (itself Python `ask/check.py`, rule for
//! rule), as one pure function over a packet's candidates, what those candidates are now, and the answer.
//!
//! Ported from `upstream-ops/src/ask.rs` (Rust branch 9dfedad) with three changes, each in FORK.md:
//! 1. "Word for word" is the one verbatim gate ([`crate::verbatim::verbatim`]), not upstream's whitespace-folded
//!    `contains`: it accepts everything that accepted (whitespace aside) plus typography-only changes, and it is the
//!    gate Punchable's Ask already runs, so a quote of his words passes or fails the same way everywhere.
//! 2. Alignment (a paraphrase placed on the lines it came from, [`crate::verbatim::align`]) is a rule switch,
//!    [`Rules::align`]. Punchable's rules turn it off: a line that claims to be his must be his, word for word.
//! 3. A candidate is a record of his material (`unit:…`, `joke:…`, `performance:…`), so the file-kind grading
//!    (`Candidate::confirms` over paths and commits) is gone; what may confirm a lesson is in [`crate::fact`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::locator::{first_line, line_range, span_text};
use crate::verbatim::{QUOTE_MIN_WORDS, align, nearest_line, normalize, strip_numbers, verbatim};

/// Which matching rules a check runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rules {
    /// A quote that is not word for word may still pass when [`align`] places 80% of its words, in order, on a run
    /// of the candidate's lines (that run's own text then stands as the quote).
    pub align: bool,
    /// A lesson candidate is quotable by its lesson's text as well as by its evidence record. A lesson's text is
    /// written by the engine or a model, not by him, so under Punchable's rules only the record's words count. The
    /// two are never searched as one text (a quote could otherwise run from the end of one into the other).
    pub lesson_text: bool,
}

impl Rules {
    /// Punchable: word for word through the verbatim gate, in his own material only.
    pub const PUNCHABLE: Rules = Rules {
        align: false,
        lesson_text: false,
    };
    /// Upstream as shipped at 9dfedad: alignment on, a fact's text quotable (kept so its test suite proves the
    /// port).
    pub const UPSTREAM: Rules = Rules {
        align: true,
        lesson_text: true,
    };
}

impl Default for Rules {
    fn default() -> Self {
        Rules::PUNCHABLE
    }
}

/// One retrieved candidate as a packet records it: no text, only where it is and what it said (its hash).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// Its number in the packet, from 1.
    pub n: usize,
    /// The record it is (`unit:…`, `joke:…`, `performance:…`).
    pub key: String,
    /// Where its lines are: the key, or a `path#La-b` span.
    pub locator: String,
    /// sha256 of its text when it was retrieved.
    pub hash: String,
    /// What it is.
    pub kind: Option<String>,
    /// Its score in the read, when it had one.
    pub score: Option<f64>,
    /// When the candidate is a recorded lesson: the lesson's text, quotable like its evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fact: Option<String>,
}

/// One quote of an answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteIn {
    /// The candidate number the quote comes from.
    pub n: usize,
    /// The candidate's lines, inclusive; the program copies them, so nothing can be mistyped.
    #[serde(default)]
    pub lines: Option<[usize; 2]>,
    /// Text copied from the candidate, when lines are not given.
    #[serde(default)]
    pub quote: String,
    /// `support` (the answer rests on it) or `note`; `None` is support.
    #[serde(rename = "use", default)]
    pub use_: Option<String>,
    /// Why it was quoted.
    #[serde(default)]
    pub why: Option<String>,
}

/// An answer to check.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerIn {
    /// The answer's prose.
    pub answer: String,
    /// Its quotes.
    #[serde(default)]
    pub quotes: Vec<QuoteIn>,
    /// What the candidates did not settle.
    #[serde(default)]
    pub gaps: Vec<String>,
    /// The candidates do not answer it.
    #[serde(default)]
    pub not_answerable: bool,
}

/// What a candidate's record is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitNow {
    /// sha256 of its text now.
    pub hash: String,
    /// Its text now.
    pub text: String,
    /// Still there (not deleted, not hidden).
    pub present: bool,
}

/// What each candidate's record is now, by key.
pub type Now = BTreeMap<String, UnitNow>;

/// The check's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    /// Every rule held.
    Pass,
    /// A rule failed; [`Report::errors`] says which.
    Fail,
    /// Nothing was read, so nothing could be checked.
    NotExecuted,
}

/// The check's findings.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// The verdict.
    pub verdict: Verdict,
    /// How many records the read could have reached.
    pub corpus_total: usize,
    /// Candidates in the packet.
    pub retrieved: usize,
    /// Candidates read.
    pub read: usize,
    /// Records not read.
    pub not_read: usize,
    /// Quotes in the answer.
    pub quotes: usize,
    /// Quotes checked.
    pub quotes_checked: usize,
    /// Quotes found.
    pub quotes_found: usize,
    /// Notes on quotes taken from a neighbouring candidate, cut to the candidate, ignored, or aligned.
    pub aligned: Vec<String>,
    /// Every broken rule, in plain words.
    pub errors: Vec<String>,
}

impl Report {
    /// One line: the counts and the verdict.
    pub fn header(&self) -> String {
        format!(
            "corpus {} | retrieved {} | read {} | not read {} | quotes {} (checked {}, found {}) | check {:?}",
            self.corpus_total,
            self.retrieved,
            self.read,
            self.not_read,
            self.quotes,
            self.quotes_checked,
            self.quotes_found,
            self.verdict
        )
    }
}

fn path_of(locator: &str) -> Option<&str> {
    locator.rsplit_once("#L").map(|x| x.0)
}

/// The check, as one pure function: every rule of upstream's `ask::check` against what the records are now.
pub fn check(
    candidates: &[Candidate],
    corpus_total: usize,
    now: &Now,
    read: &[usize],
    answer: &AnswerIn,
    rules: Rules,
) -> Report {
    let mut report = Report {
        verdict: Verdict::Pass,
        corpus_total,
        retrieved: candidates.len(),
        read: 0,
        not_read: corpus_total,
        quotes: answer.quotes.len(),
        quotes_checked: 0,
        quotes_found: 0,
        aligned: Vec::new(),
        errors: Vec::new(),
    };
    if !candidates.is_empty() && read.is_empty() {
        report.verdict = Verdict::NotExecuted;
        report
            .errors
            .push("nothing was read, so nothing can be checked".to_owned());
        return report;
    }
    let by_n: BTreeMap<usize, &Candidate> = candidates.iter().map(|c| (c.n, c)).collect();
    let mut times_read: BTreeMap<usize, usize> = BTreeMap::new();
    for &n in read {
        if !by_n.contains_key(&n) {
            report
                .errors
                .push(format!("read names candidate {n}, which is not in this packet"));
            continue;
        }
        let count = times_read.entry(n).or_default();
        *count += 1;
        if *count == 2 {
            report.errors.push(format!("read names candidate {n} twice"));
        }
    }
    for c in candidates {
        if !times_read.contains_key(&c.n) {
            report
                .errors
                .push(format!("candidate {} ({}) was not read", c.n, c.locator));
        }
    }
    for (i, q) in answer.quotes.iter().enumerate() {
        check_quote(i + 1, q, candidates, &by_n, &times_read, now, rules, &mut report);
    }
    report.read = times_read.len();
    report.not_read = corpus_total.saturating_sub(report.read);
    if !report.errors.is_empty() {
        report.verdict = Verdict::Fail;
    }
    report
}

/// One quote's rules (the body of upstream's loop, split out so the loop reads as the rule list). The short
/// names are upstream's (k the quote's number, q the quote, c its candidate, a..b its lines), kept for the diff.
#[allow(clippy::too_many_arguments, clippy::many_single_char_names)]
fn check_quote(
    k: usize,
    q: &QuoteIn,
    candidates: &[Candidate],
    by_n: &BTreeMap<usize, &Candidate>,
    times_read: &BTreeMap<usize, usize>,
    now: &Now,
    rules: Rules,
    report: &mut Report,
) {
    let use_ = q.use_.as_deref().unwrap_or("support");
    if use_ != "support" && use_ != "note" {
        report
            .errors
            .push(format!("quote {k} use must be support or note, not {use_:?}"));
    }
    let Some(c) = by_n.get(&q.n).copied() else {
        report
            .errors
            .push(format!("quote {k} cites candidate {}, outside this packet", q.n));
        return;
    };
    if !times_read.contains_key(&q.n) {
        report
            .errors
            .push(format!("quote {k} cites candidate {}, which was not read", q.n));
    }
    // Lines named for the wrong candidate of the same source (the span next door) are taken from the candidate
    // that holds them, and the report says so.
    let c = match (q.lines, now.get(&c.key)) {
        (Some([a, b]), Some(unit)) if span_text(&unit.text, &c.locator, a, b).is_none() => {
            let path = path_of(&c.locator);
            let holder = candidates.iter().find(|o| {
                o.n != c.n
                    && path.is_some()
                    && path_of(&o.locator) == path
                    && now
                        .get(&o.key)
                        .is_some_and(|u| span_text(&u.text, &o.locator, a, b).is_some())
            });
            match holder {
                Some(o) => {
                    report.aligned.push(format!(
                        "quote {k}: lines {a}-{b} are in candidate {} ({}), not {}; taken from there",
                        o.n, o.locator, c.n
                    ));
                    o
                }
                None => c,
            }
        }
        _ => c,
    };
    // A quote object with neither lines nor text carries nothing: noted and skipped, not refused.
    if q.lines.is_none() && q.quote.trim().is_empty() {
        report
            .aligned
            .push(format!("quote {k} had neither lines nor text and was ignored"));
        return;
    }
    let text = match (q.lines, now.get(&c.key)) {
        (Some([a, b]), Some(unit)) => {
            if let Some(t) = span_text(&unit.text, &c.locator, a, b) {
                normalize(&t)
            } else {
                // A range that starts inside the candidate and runs past its end is cut to the candidate's
                // lines, and the report says so; one wholly outside is refused.
                let end = line_range(&c.locator).map_or(unit.text.lines().count(), |r| r.1);
                match span_text(&unit.text, &c.locator, a, end.min(b)) {
                    Some(t) if a <= end => {
                        report.aligned.push(format!(
                            "quote {k}: lines {}-{b} are past the end of candidate {} ({}); took lines {a}-{end}",
                            end + 1,
                            q.n,
                            c.locator
                        ));
                        normalize(&t)
                    }
                    _ => {
                        report.errors.push(format!(
                            "quote {k}: lines {a}-{b} are not inside candidate {} ({})",
                            q.n, c.locator
                        ));
                        return;
                    }
                }
            }
        }
        _ => normalize(&strip_numbers(&q.quote)),
    };
    // A quote under QUOTE_MIN_WORDS (a number, a name) counts only word for word: too short to align, not too
    // short to be exact.
    let short = text.split(' ').filter(|w| !w.is_empty()).count() < QUOTE_MIN_WORDS;
    if short && text.len() < 4 {
        report
            .errors
            .push(format!("quote {k} is too short to mean anything ({text:?})"));
        return;
    }
    report.quotes_checked += 1;
    let Some(unit) = now.get(&c.key) else {
        report
            .errors
            .push(format!("quote {k}: candidate {} ({}) no longer exists", q.n, c.locator));
        return;
    };
    if unit.hash != c.hash {
        report.errors.push(format!(
            "quote {k}: candidate {} ({}) changed since it was retrieved; ask again",
            q.n, c.locator
        ));
        return;
    }
    // The record's words; a lesson's own text only where the rules allow it, and never joined to the record's (a
    // quote must not run from the end of one into the other). Changed from upstream, which searched
    // `fact + "\n" + unit` as one text (FORK.md).
    let in_record = verbatim(&text, &unit.text).is_some();
    let in_lesson = rules.lesson_text && c.fact.as_deref().is_some_and(|f| verbatim(&text, f).is_some());
    if in_record || in_lesson {
        report.quotes_found += 1;
    } else if c.fact.as_deref().is_some_and(|f| verbatim(&text, f).is_some()) {
        report.errors.push(format!(
            "quote {k} is the lesson's text, not his words: quote candidate {}'s own record",
            q.n
        ));
    } else if short {
        report.errors.push(format!(
            "quote {k} ({text:?}) is under {QUOTE_MIN_WORDS} words and not in candidate {} word for word",
            q.n
        ));
    } else if let Some(a) = align(&unit.text, &text).filter(|_| rules.align) {
        // Models find the passage, not the exact span (upstream measured 40% of typed quotes failing word for
        // word). The span's own text stands as the quote. Off under Punchable's rules.
        report.quotes_found += 1;
        let start = first_line(&c.locator).unwrap_or(1);
        report.aligned.push(format!(
            "quote {k} aligned to candidate {} lines {}-{} ({:.0}% of its words, in order)",
            q.n,
            start + a.first,
            start + a.last,
            a.score * 100.0
        ));
    } else {
        report.errors.push(format!(
            "quote {k} not found in candidate {} ({}){}",
            q.n,
            c.locator,
            nearest_line(&unit.text, &text).map_or(String::new(), |l| format!("; its nearest line reads: {l:?}"))
        ));
    }
    if use_ == "support" && !unit.present {
        report.errors.push(format!(
            "quote {k} supports from candidate {} ({}), which is no longer present",
            q.n, c.locator
        ));
    }
}

#[cfg(test)]
mod tests {
    //! Upstream's check tests (ask.rs, 9dfedad), ported with `Rules::UPSTREAM`, plus Punchable's own.
    use super::*;
    use crate::locator::numbered;

    fn cand(n: usize, key: &str) -> Candidate {
        Candidate {
            n,
            key: key.into(),
            locator: format!("f.md#L{n}-{n}"),
            hash: format!("h{key}"),
            kind: Some("unit".into()),
            score: Some(1.0),
            fact: None,
        }
    }

    fn unit(hash: &str, text: &str, present: bool) -> UnitNow {
        UnitNow {
            hash: hash.into(),
            text: text.into(),
            present,
        }
    }

    fn now() -> Now {
        let mut m = Now::new();
        m.insert("a".into(), unit("ha", "The parser   rejects empty\ninput early.", true));
        m.insert("b".into(), unit("hb", "Rollback resimulates confirmed frames.", true));
        m
    }

    fn ans(quotes: Vec<(usize, &str, &str)>) -> AnswerIn {
        AnswerIn {
            answer: "a".into(),
            quotes: quotes
                .into_iter()
                .map(|(n, q, u)| QuoteIn {
                    n,
                    quote: q.into(),
                    lines: None,
                    use_: Some(u.into()),
                    why: None,
                })
                .collect(),
            gaps: vec![],
            not_answerable: false,
        }
    }

    const GW: Rules = Rules::UPSTREAM;

    #[test]
    fn a_fact_candidate_is_quotable_by_its_fact_text_and_a_record_by_its_own_lines() {
        let mut c = cand(1, "b");
        c.fact = Some("Rollback keeps only confirmed frames in the save set.".into());
        let r = check(
            &[c],
            10,
            &now(),
            &[1],
            &ans(vec![(1, "keeps only confirmed frames", "support")]),
            GW,
        );
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.errors);
        assert_eq!(
            span_text("subject\n\n2026-10-01\nfiles:\na.rs", "commit:abc", 4, 5).as_deref(),
            Some("files:\na.rs")
        );
        let r = check(&[cand(1, "a")], 10, &now(), &[1], &ans(vec![(1, "", "support")]), GW);
        assert_eq!(
            r.verdict,
            Verdict::Pass,
            "an empty quote object is ignored: {:?}",
            r.errors
        );
        assert!(r.aligned[0].contains("neither lines nor text"), "{:?}", r.aligned);
        let r = check(
            &[cand(1, "a")],
            10,
            &now(),
            &[1],
            &ans(vec![(1, "empty input", "note")]),
            GW,
        );
        assert_eq!((r.verdict, r.quotes_found), (Verdict::Pass, 1), "{:?}", r.errors);
        let r = check(
            &[cand(1, "a")],
            10,
            &now(),
            &[1],
            &ans(vec![(1, "empty output", "note")]),
            GW,
        );
        assert_eq!(r.verdict, Verdict::Fail);
        assert!(
            r.errors[0].contains("not in candidate 1 word for word"),
            "{:?}",
            r.errors
        );
        let mut over = ans(vec![]);
        over.quotes.push(QuoteIn {
            n: 1,
            lines: Some([1, 3]),
            ..QuoteIn::default()
        });
        let r = check(&[cand(1, "a")], 10, &now(), &[1], &over, GW);
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.errors);
        assert!(r.aligned.iter().any(|a| a.contains("past the end")), "{:?}", r.aligned);
        over.quotes[0].lines = Some([5, 6]);
        assert_eq!(
            check(&[cand(1, "a")], 10, &now(), &[1], &over, GW).verdict,
            Verdict::Fail
        );
    }

    #[test]
    fn a_quote_copied_with_its_line_numbers_still_matches() {
        let c = [cand(1, "a")];
        assert_eq!(
            numbered("The parser   rejects empty\ninput early.", "f.md#L41-42"),
            "41| The parser   rejects empty\n42| input early."
        );
        let r = check(
            &c,
            10,
            &now(),
            &[1],
            &ans(vec![(1, "41| parser rejects empty 42| input", "support")]),
            GW,
        );
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.errors);
        // A stray number inside the text is tolerated by alignment (4 of 5 words, in order), and said so.
        let r = check(
            &c,
            10,
            &now(),
            &[1],
            &ans(vec![(1, "parser rejects 3|empty input", "support")]),
            GW,
        );
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.errors);
        assert_eq!(r.aligned.len(), 1);
    }

    #[test]
    fn a_whole_read_with_true_quotes_passes_and_reports_what_was_not_read() {
        let c = [cand(1, "a"), cand(2, "b")];
        let r = check(
            &c,
            500,
            &now(),
            &[1, 2],
            &ans(vec![(1, "parser rejects empty input", "support")]),
            GW,
        );
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.errors);
        assert_eq!((r.read, r.not_read, r.quotes_found), (2, 498, 1));
    }

    #[test]
    fn every_rule_refuses() {
        let c = [cand(1, "a"), cand(2, "b")];
        let n = now();
        let errs = |read: &[usize], a: AnswerIn| check(&c, 10, &n, read, &a, GW).errors.join("\n");
        assert!(errs(&[1], ans(vec![])).contains("candidate 2 (f.md#L2-2) was not read"));
        assert!(errs(&[1, 2, 2], ans(vec![])).contains("candidate 2 twice"));
        assert!(errs(&[1, 2, 9], ans(vec![])).contains("candidate 9, which is not in this packet"));
        assert!(
            errs(&[1, 2], ans(vec![(1, "the parser accepts input", "support")])).contains("not found in candidate")
        );
        assert!(
            errs(&[1, 2], ans(vec![(1, "rejects nothing", "support")]))
                .contains("under 3 words and not in candidate 1")
        );
        assert!(errs(&[1, 2], ans(vec![(7, "parser rejects empty", "support")])).contains("outside this packet"));
        assert!(errs(&[1, 2], ans(vec![(1, "parser rejects empty", "maybe")])).contains("support or note"));
        assert_eq!(check(&c, 10, &n, &[], &ans(vec![]), GW).verdict, Verdict::NotExecuted);
    }

    #[test]
    fn a_record_that_changed_or_left_cannot_carry_the_answer() {
        let c = [cand(1, "a")];
        let mut changed = now();
        changed.insert("a".into(), unit("other", "The parser rejects empty input early.", true));
        let r = check(
            &c,
            10,
            &changed,
            &[1],
            &ans(vec![(1, "parser rejects empty input", "support")]),
            GW,
        );
        assert!(
            r.errors.join(" ").contains("changed since it was retrieved"),
            "{:?}",
            r.errors
        );
        let mut gone = now();
        gone.insert("a".into(), unit("ha", "The parser rejects empty input early.", false));
        let r = check(
            &c,
            10,
            &gone,
            &[1],
            &ans(vec![(1, "parser rejects empty input", "support")]),
            GW,
        );
        assert!(r.errors.join(" ").contains("no longer present"));
        let r = check(
            &c,
            10,
            &gone,
            &[1],
            &ans(vec![(1, "parser rejects empty input", "note")]),
            GW,
        );
        assert_eq!(
            r.verdict,
            Verdict::Pass,
            "a note may cite a record that left: {:?}",
            r.errors
        );
    }

    #[test]
    fn a_quote_given_as_lines_is_copied_by_the_program_and_checked_for_range() {
        let c = [cand(1, "a")];
        let mut by_lines = ans(vec![]);
        by_lines.quotes.push(QuoteIn {
            n: 1,
            lines: Some([1, 1]),
            ..QuoteIn::default()
        });
        let r = check(&c, 10, &now(), &[1], &by_lines, GW);
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.errors);
        by_lines.quotes[0].lines = Some([2, 5]);
        let r = check(&c, 10, &now(), &[1], &by_lines, GW);
        assert!(r.errors.join(" ").contains("not inside candidate"), "{:?}", r.errors);
    }

    #[test]
    fn a_near_miss_quote_is_aligned_to_its_span_and_a_paraphrase_is_not() {
        let c = [cand(1, "b")];
        let r = check(
            &c,
            10,
            &now(),
            &[1],
            &ans(vec![(1, "rollback resimulates CONFIRMED frames!", "support")]),
            GW,
        );
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.errors);
        assert_eq!(r.quotes_found, 1);
        assert!(
            r.aligned[0].contains("aligned to candidate 1 lines 1-1"),
            "{:?}",
            r.aligned
        );
        let r = check(
            &c,
            10,
            &now(),
            &[1],
            &ans(vec![(1, "frames that were confirmed get resimulated by it", "support")]),
            GW,
        );
        assert_eq!(r.verdict, Verdict::Fail, "{:?}", r.aligned);
    }

    // ---- Punchable's rules ----------------------------------------------------------------------------------

    #[test]
    fn under_punchables_rules_a_line_that_claims_to_be_his_must_be_word_for_word() {
        let c = [cand(1, "b")];
        // Upstream places this near miss; Punchable refuses it: case and a dropped word are not his line.
        let near = ans(vec![(1, "rollback resimulates CONFIRMED frames!", "support")]);
        assert_eq!(check(&c, 10, &now(), &[1], &near, GW).verdict, Verdict::Pass);
        let r = check(&c, 10, &now(), &[1], &near, Rules::PUNCHABLE);
        assert_eq!(r.verdict, Verdict::Fail, "{:?}", r.aligned);
        assert!(r.errors[0].contains("not found in candidate 1"), "{:?}", r.errors);
        // The stray line number of the upstream test does not pass either.
        let a = [cand(1, "a")];
        let stray = ans(vec![(1, "parser rejects 3|empty input", "support")]);
        assert_eq!(
            check(&a, 10, &now(), &[1], &stray, Rules::PUNCHABLE).verdict,
            Verdict::Fail
        );
        assert_eq!(Rules::default(), Rules::PUNCHABLE);
    }

    #[test]
    fn a_lessons_text_is_not_his_words_and_no_quote_runs_across_the_boundary() {
        // A passed-over line the model wrote, stored as a lesson, resting on a record of his.
        let mut c = cand(1, "b");
        c.fact = Some("He passed over this line: my cat files my taxes".into());
        let only_lesson = ans(vec![(1, "my cat files my taxes", "support")]);
        let r = check(
            std::slice::from_ref(&c),
            10,
            &now(),
            &[1],
            &only_lesson,
            Rules::PUNCHABLE,
        );
        assert_eq!(r.verdict, Verdict::Fail, "{:?}", r.errors);
        assert!(
            r.errors[0].contains("the lesson's text, not his words"),
            "{:?}",
            r.errors
        );
        // Upstream's rules keep its behaviour: a fact's text is quotable.
        assert_eq!(
            check(std::slice::from_ref(&c), 10, &now(), &[1], &only_lesson, GW).verdict,
            Verdict::Pass
        );
        // The end of the lesson and the start of the record are two texts, under either rules.
        let stitched = ans(vec![(1, "my taxes Rollback resimulates", "support")]);
        for rules in [Rules::PUNCHABLE, GW] {
            let r = check(std::slice::from_ref(&c), 10, &now(), &[1], &stitched, rules);
            assert_eq!(r.verdict, Verdict::Fail, "{rules:?}: {:?}", r.errors);
        }
        // His record's own words still pass.
        let his = ans(vec![(1, "Rollback resimulates confirmed frames", "support")]);
        assert_eq!(
            check(&[c], 10, &now(), &[1], &his, Rules::PUNCHABLE).verdict,
            Verdict::Pass
        );
    }

    #[test]
    fn typography_only_changes_pass_under_both_rules_through_the_one_gate() {
        let mut n = Now::new();
        n.insert(
            "u".into(),
            unit("hu", "My landlord\u{2019}s dog \u{2014} he owns me.", true),
        );
        let c = [Candidate {
            n: 1,
            key: "u".into(),
            locator: "unit:u".into(),
            hash: "hu".into(),
            kind: Some("unit".into()),
            score: None,
            fact: None,
        }];
        let q = ans(vec![(1, "\"My landlord's dog - he owns me.\"", "support")]);
        for rules in [Rules::PUNCHABLE, Rules::UPSTREAM] {
            let r = check(&c, 1, &n, &[1], &q, rules);
            assert_eq!(r.verdict, Verdict::Pass, "{rules:?}: {:?}", r.errors);
        }
    }
}
