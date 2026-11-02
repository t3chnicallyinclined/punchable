//! The engine in the app's words. Inside the engine a lesson has a grade (CONFIRMED / INFERRED) and a link is a
//! hypothesis with a status; the app sees neither word. It sees what he learned and where it came from ("you kept
//! this line"), and bits that go together with the plain reason why. These are the only engine types meant for the
//! wire; nothing here carries a grade, by construction (no field for one), and a test pins the JSON keys.

use serde::Serialize;

use crate::fact::{Checked, Kind};

/// A lesson as the app shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Lesson {
    /// The lesson, in plain words.
    pub text: String,
    /// Where it came from, in his words ("You kept this line", "From your set at …").
    pub from: String,
    /// The words of his material it rests on, exactly as he wrote or said them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub his_words: Option<String>,
    /// The record those words are in, so a tap can open it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<String>,
    /// Ruled out: shown as something that does not work for him, never as a fact.
    pub ruled_out: bool,
}

impl Lesson {
    /// A checked lesson in the app's words; the grade stays behind. "You …" only for a statement his own signed-in
    /// session made: an unsigned one is never put in his mouth.
    pub fn of(c: &Checked) -> Self {
        let from = match (c.owner_statement().filter(|_| c.signed()), c.evidence()) {
            (Some(s), _) => format!("You {s}"),
            (None, Some(_)) => "From your own words".to_owned(),
            (None, None) => "Punchable noticed this".to_owned(),
        };
        Lesson {
            text: c.text().to_owned(),
            from,
            his_words: c.evidence().map(|q| q.text.clone()),
            record: c.evidence().map(|q| q.record.clone()),
            ruled_out: c.kind() == Kind::Eliminated,
        }
    }
}

/// Two bits that may go together, as the app shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    /// The bit he is on.
    pub from: String,
    /// The bit it may go with.
    pub to: String,
    /// Why, in plain words (the words they share).
    pub why: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fact::{Grade, LearnedLine, check_fact, from_learned};

    #[test]
    fn no_grade_word_reaches_the_app() {
        let line = LearnedLine {
            text: "the dog has a lawyer".into(),
            bit: None,
            date: None,
            edited: None,
        };
        let c = check_fact(&from_learned(&line, true), None, true).expect("kept");
        assert_eq!(c.grade(), Grade::Confirmed);
        let lesson = Lesson::of(&c);
        assert_eq!(lesson.from, "You kept it in the app");
        // The same statement from a session that is not his: never "You …".
        let unsigned = check_fact(&from_learned(&line, true), None, false).expect("kept");
        assert_eq!(Lesson::of(&unsigned).from, "Punchable noticed this");
        let wire = serde_json::to_string(&lesson).expect("json");
        for word in ["CONFIRMED", "INFERRED", "UNKNOWN", "grade", "confidence"] {
            assert!(!wire.contains(word), "{word} in {wire}");
        }
        let keys: Vec<String> = serde_json::to_value(&lesson)
            .expect("json")
            .as_object()
            .expect("object")
            .keys()
            .cloned()
            .collect();
        assert_eq!(keys, ["text", "from", "ruled_out"]);
    }
}
