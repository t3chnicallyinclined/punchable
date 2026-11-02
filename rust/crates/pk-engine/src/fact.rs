//! Lessons: what the engine has learned about his material and his taste, each with its source and its evidence,
//! and the evidence rule that decides how sure each one is.
//!
//! The rule, from upstream (`upstream-ops/src/store.rs` `needs_evidence`, `confirm_cap`, `Quote::verify`;
//! Rust branch 9dfedad): a lesson may be CONFIRMED only with evidence, and the evidence is checked, not trusted.
//! Punchable keeps two kinds of evidence and drops one (FORK.md):
//! - a **quote** of his material (a record and words from it): found through the one verbatim gate
//!   ([`crate::verbatim::verbatim`]), and the exact slice of the record is what is kept, never the writer's copy;
//! - an **owner statement**: his own action in the app (he kept a line, he passed one over, he said so). It confirms
//!   only when his own signed-in session wrote it (`signed`), as upstream's owner login does;
//! - a run receipt (upstream's `run` / `gate` proof that a command ran) has no meaning here and is gone.
//!
//! The grade (CONFIRMED / INFERRED) is internal: the app never shows it ([`crate::edge`]).

use serde::{Deserialize, Serialize};

use crate::error::{EngineError, EngineResult};
use crate::verbatim::{QUOTE_MIN_WORDS, verbatim};

/// What a lesson is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Something true about his material or his taste.
    Fact,
    /// Something he decided.
    Decision,
    /// Something ruled out: shown as ruled out, never as a fact (upstream's `fact_heading`).
    Eliminated,
}

/// How sure a lesson is. Internal: never reaches the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Grade {
    /// Not known.
    Unknown,
    /// Inferred: a model's or the engine's reading, not proven.
    Inferred,
    /// Confirmed by checked evidence.
    Confirmed,
}

/// Words a lesson rests on: a record of his material and the words in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteOf {
    /// The record (`unit:…`, `joke:…`, `performance:…`).
    pub record: String,
    /// The words, as the writer gave them.
    pub text: String,
}

/// A lesson as a writer proposes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactIn {
    /// What it is.
    pub kind: Kind,
    /// The lesson, in plain words.
    pub text: String,
    /// How sure the writer says it is.
    pub confidence: Grade,
    /// Where it came from (a packet, a draft, a set).
    pub source: String,
    /// Who wrote it (`app`, `worker`, `owner`).
    pub by: String,
    /// His own statement or action, when that is the evidence.
    #[serde(default)]
    pub owner_statement: Option<String>,
    /// Words of his material, when that is the evidence.
    #[serde(default)]
    pub quote: Option<QuoteOf>,
}

/// A lesson that passed the rule, ready to store. Only [`check_fact`] makes one (private fields, no serde), so a
/// caller cannot hand the store a CONFIRMED lesson the rule never saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    kind: Kind,
    text: String,
    grade: Grade,
    capped: Option<String>,
    source: String,
    by: String,
    owner_statement: Option<String>,
    signed: bool,
    evidence: Option<QuoteOf>,
}

impl Checked {
    /// What it is.
    pub fn kind(&self) -> Kind {
        self.kind
    }
    /// The lesson, trimmed (at most [`TEXT_MAX`] characters).
    pub fn text(&self) -> &str {
        &self.text
    }
    /// The grade the rule allows (internal: never for the app).
    pub fn grade(&self) -> Grade {
        self.grade
    }
    /// Why it is not CONFIRMED when the writer asked for that.
    pub fn capped(&self) -> Option<&str> {
        self.capped.as_deref()
    }
    /// Where it came from.
    pub fn source(&self) -> &str {
        &self.source
    }
    /// Who wrote it.
    pub fn by(&self) -> &str {
        &self.by
    }
    /// His statement, when given.
    pub fn owner_statement(&self) -> Option<&str> {
        self.owner_statement.as_deref()
    }
    /// His own signed-in session wrote it.
    pub fn signed(&self) -> bool {
        self.signed
    }
    /// The evidence record and the exact slice of its text (never the writer's copy).
    pub fn evidence(&self) -> Option<&QuoteOf> {
        self.evidence.as_ref()
    }
}

/// The longest lesson text, in characters (upstream keeps 1500 of an answer).
pub const TEXT_MAX: usize = 1500;

/// Upstream's `needs_evidence`: CONFIRMED needs a quote or an owner statement.
pub fn needs_evidence(fact: &FactIn) -> EngineResult<()> {
    if fact.confidence == Grade::Confirmed && fact.owner_statement.is_none() && fact.quote.is_none() {
        return Err(EngineError::Refused(
            "a confirmed lesson needs evidence: words found in his material, or his own statement. Record it as \
             inferred otherwise."
                .to_owned(),
        ));
    }
    Ok(())
}

/// The evidence rule over one lesson. `record_text` is what the quoted record says now (`None` when it is gone);
/// `signed` is whether his own signed-in session writes the row. A quote that is not in the record word for word is
/// refused outright (a lesson never rests on words he did not say); a CONFIRMED lesson whose only evidence cannot
/// confirm is written INFERRED with the reason.
pub fn check_fact(fact: &FactIn, record_text: Option<&str>, signed: bool) -> EngineResult<Checked> {
    let text = fact.text.trim();
    if text.is_empty() {
        return Err(EngineError::Refused("a lesson needs text".to_owned()));
    }
    needs_evidence(fact)?;
    let evidence = match &fact.quote {
        Some(q) => {
            let source = record_text.ok_or_else(|| {
                EngineError::Refused(format!(
                    "{} is not in his material any more: nothing was written",
                    q.record
                ))
            })?;
            let found = whole_words(&q.text, source).ok_or_else(|| {
                EngineError::Refused(format!(
                    "the quote is not in {} word for word, as at least {QUOTE_MIN_WORDS} whole words (a paraphrase or \
                     a fragment is refused): nothing was written",
                    q.record
                ))
            })?;
            Some(QuoteOf {
                record: q.record.clone(),
                text: found.to_owned(),
            })
        }
        None => None,
    };
    let capped = if fact.confidence == Grade::Confirmed && evidence.is_none() && !signed {
        Some("its only evidence is a statement that his own session did not sign".to_owned())
    } else {
        None
    };
    let grade = if capped.is_some() {
        Grade::Inferred
    } else {
        fact.confidence
    };
    Ok(Checked {
        kind: fact.kind,
        text: text.chars().take(TEXT_MAX).collect(),
        grade,
        capped,
        source: fact.source.trim().to_owned(),
        by: fact.by.trim().to_owned(),
        owner_statement: fact.owner_statement.clone(),
        signed,
        evidence,
    })
}

/// The evidence slice for `quote` in `source`: the verbatim gate's slice, at least [`QUOTE_MIN_WORDS`] words, and
/// not cut out of a word ("he" inside "the" is not a quote).
fn whole_words<'a>(quote: &str, source: &'a str) -> Option<&'a str> {
    let found = verbatim(quote, source)?;
    if found.split_whitespace().count() < QUOTE_MIN_WORDS {
        return None;
    }
    // `found` is a slice of `source` (the gate's contract), so its offset is the pointer difference.
    let start = (found.as_ptr() as usize).checked_sub(source.as_ptr() as usize)?;
    let end = start + found.len();
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let cut_in = word(source.get(..start)?.chars().next_back()) && word(found.chars().next());
    let cut_out = word(source.get(end..)?.chars().next()) && word(found.chars().next_back());
    (!cut_in && !cut_out).then_some(found)
}

/// One line of the voice profile's learned lists (`profile:voice.learned`, `creative/learned.py`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedLine {
    /// The line.
    pub text: String,
    /// The bit it was written for, when known.
    #[serde(default)]
    pub bit: Option<String>,
    /// When.
    #[serde(default)]
    pub date: Option<String>,
    /// He edited it before keeping it.
    #[serde(default)]
    pub edited: Option<bool>,
}

/// A learned line as a lesson with its source: he kept it, or he passed it over. His action in his own session is
/// the evidence (an owner statement), so the lesson is CONFIRMED when `signed`; the line itself is the lesson's
/// words. Kept lines that became versions of a bit are quotable from that bit's record later; this lesson does not
/// depend on it.
pub fn from_learned(line: &LearnedLine, kept: bool) -> FactIn {
    let (what, statement) = if kept {
        ("He kept this line", "kept it in the app")
    } else {
        ("He passed over this line", "passed it over in the app")
    };
    FactIn {
        kind: if kept { Kind::Fact } else { Kind::Eliminated },
        text: format!("{what}: {}", line.text.trim()),
        confidence: Grade::Confirmed,
        source: line
            .bit
            .as_deref()
            .map_or_else(|| "a draft".to_owned(), |b| format!("a draft for {b}")),
        by: "app".to_owned(),
        owner_statement: Some(statement.to_owned()),
        quote: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(confidence: Grade, statement: Option<&str>, quote: Option<(&str, &str)>) -> FactIn {
        FactIn {
            kind: Kind::Fact,
            text: " He opens on the dog. ".into(),
            confidence,
            source: "packet:1".into(),
            by: "worker".into(),
            owner_statement: statement.map(str::to_owned),
            quote: quote.map(|(r, t)| QuoteOf {
                record: r.into(),
                text: t.into(),
            }),
        }
    }

    #[test]
    fn confirmed_needs_evidence_and_the_evidence_is_checked() {
        let e = check_fact(&fact(Grade::Confirmed, None, None), None, true).expect_err("refused");
        assert!(e.to_string().contains("needs evidence"), "{e}");
        // A quote kept as the record's exact slice, typography and all.
        let src = "My landlord\u{2019}s dog owns me.";
        let c = check_fact(
            &fact(Grade::Confirmed, None, Some(("unit:a", "landlord's dog owns me"))),
            Some(src),
            false,
        )
        .expect("kept");
        assert_eq!(c.grade(), Grade::Confirmed);
        assert_eq!(c.evidence().expect("evidence").text, "landlord\u{2019}s dog owns me");
        assert_eq!(c.text(), "He opens on the dog.");
        // A paraphrase, or a record that is gone: nothing is written.
        let e = check_fact(
            &fact(Grade::Inferred, None, Some(("unit:a", "the landlord's cat"))),
            Some(src),
            true,
        )
        .expect_err("refused");
        assert!(e.to_string().contains("word for word"), "{e}");
        assert!(
            check_fact(
                &fact(Grade::Inferred, None, Some(("unit:a", "dog owns me"))),
                None,
                true
            )
            .is_err()
        );
    }

    #[test]
    fn a_statement_confirms_only_when_his_session_signs_it() {
        let f = fact(Grade::Confirmed, Some("he said so"), None);
        assert_eq!(check_fact(&f, None, true).expect("kept").grade(), Grade::Confirmed);
        let c = check_fact(&f, None, false).expect("kept");
        assert_eq!(c.grade(), Grade::Inferred);
        assert!(c.capped().expect("a reason").contains("did not sign"));
        assert_eq!(
            check_fact(&fact(Grade::Inferred, None, None), None, false)
                .expect("kept")
                .grade(),
            Grade::Inferred
        );
    }

    #[test]
    fn evidence_is_at_least_three_whole_words() {
        let src = "the other day the dog bit the mailman";
        let q = |t: &str| fact(Grade::Confirmed, None, Some(("unit:a", t)));
        assert!(
            check_fact(&q("he"), Some(src), true).is_err(),
            "a fragment inside a word"
        );
        assert!(check_fact(&q("dog bit"), Some(src), true).is_err(), "two words");
        assert!(
            check_fact(&q("her day the"), Some(src), true).is_err(),
            "starts inside \"other\""
        );
        assert!(
            check_fact(&q("the dog bi"), Some(src), true).is_err(),
            "ends inside \"bit\""
        );
        let c = check_fact(&q("the dog bit"), Some(src), true).expect("kept");
        assert_eq!(c.evidence().expect("evidence").text, "the dog bit");
        assert!(c.signed());
    }

    #[test]
    fn learned_lines_become_lessons_with_their_source() {
        let line = LearnedLine {
            text: "  the dog has a lawyer ".into(),
            bit: Some("joke:7".into()),
            date: None,
            edited: Some(true),
        };
        let kept = from_learned(&line, true);
        assert_eq!(kept.text, "He kept this line: the dog has a lawyer");
        assert_eq!(kept.source, "a draft for joke:7");
        assert_eq!(check_fact(&kept, None, true).expect("kept").grade(), Grade::Confirmed);
        let dropped = from_learned(&line, false);
        assert_eq!(dropped.kind, Kind::Eliminated);
        assert!(
            check_fact(&dropped, None, false).expect("kept").capped().is_some(),
            "unsigned: inferred"
        );
    }
}
