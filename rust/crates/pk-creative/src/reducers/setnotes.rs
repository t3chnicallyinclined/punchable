//! His bullet notes under a bit of a set (`docs/design/SPEC-2026-11-16-bits-sets-tapes.md` §3.3): one item per bit
//! per set list, keyed `<board>|<joke>` ([`note_key`]); one op, `set {board, joke, notes}`, newest wins, and the item
//! keeps the list it replaced (`was`) so Undo is one more `set`. The notes belong to the pair, not to the board's
//! order: taking the bit off the list leaves them here, and putting it back brings them back. Rust only (B5: a new
//! store, never a new op on a store another process replays): no Python twin, no golden; `tests` below are the proof.
//!
//! The reducer checks the SHAPE of an event only (who the notes are for, a list of texts). How many notes, how long,
//! and what a clean note is ([`MAX_NOTES`], [`MAX_NOTE_CHARS`], [`clean_note`], [`refusal`]) are the route's rules
//! (`pk-api` `writes::setnotes`), never the reducer's: a rule tightened here later would refuse the history already
//! written, and the store could no longer be replayed.

use serde_json::{Value, json};

use super::unknown_op;
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::store::{Items, Reducer};
use pk_core::py;

/// Notes under one bit of one set, at most (the route's cap).
pub const MAX_NOTES: usize = 12;
/// One note, at most, in code points (the route's cap).
pub const MAX_NOTE_CHARS: usize = 200;

/// The item id of a bit's notes in a set list. Ids cannot hold a bar (the API's id pattern), so the two halves never
/// run together.
pub fn note_key(board: &str, joke: &str) -> String {
    format!("{board}|{joke}")
}

/// Whether a character may not stand in a note as it is: a control character (a pasted newline, a tab), a line or
/// paragraph separator, or a direction override or isolate (they reorder what is shown; a note is plain words).
fn breaks_a_line(ch: char) -> bool {
    ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// A note as the route stores it: every character that would break the line or reorder it becomes a space, and the
/// ends are trimmed. One note is one line.
pub fn clean_note(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|ch| if breaks_a_line(ch) { ' ' } else { ch })
        .collect();
    py::strip(&flat).to_string()
}

/// Why the route cannot store a list of notes, if it cannot: too many, one too long, one empty or not one clean line.
pub fn refusal(notes: &[String]) -> Option<String> {
    if notes.len() > MAX_NOTES {
        return Some(format!("at most {MAX_NOTES} notes under a bit"));
    }
    if notes.iter().any(|n| py::len(n) > MAX_NOTE_CHARS) {
        return Some(format!("a note is at most {MAX_NOTE_CHARS} characters"));
    }
    if notes.iter().any(|n| n.is_empty() || *n != clean_note(n)) {
        return Some("a note is one line of words".to_string());
    }
    None
}

/// The strings of a stored list (anything else in it is not a note).
fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect()
}

/// His notes under `joke` in `board`, in his order; none when he wrote none (or cleared them).
pub fn notes_of(items: &Items, board: &str, joke: &str) -> Vec<String> {
    strings(items.get(&note_key(board, joke)).and_then(|x| x.get("notes")))
}

/// The setnotes reducer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Setnotes;

impl Reducer for Setnotes {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, id) = (ev.op.as_str(), ev.id.as_str());
        if op != "set" {
            return Err(unknown_op(op));
        }
        let d = ev.data()?;
        let part = |k: &str| {
            d.get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && !s.contains('|'))
        };
        let (Some(board), Some(joke)) = (part("board"), part("joke")) else {
            return Err(CreativeError::refused("board and joke are required"));
        };
        if id != note_key(board, joke) {
            return Err(CreativeError::refused("id must be the board and the joke"));
        }
        let Some(list) = d.get("notes").and_then(Value::as_array) else {
            return Err(CreativeError::refused("notes must be a list"));
        };
        if list.iter().any(|x| !x.is_string()) {
            return Err(CreativeError::refused("a note is text"));
        }
        let notes = strings(d.get("notes"));
        let old = items.get(id);
        let was = strings(old.and_then(|x| x.get("notes")));
        let created = old
            .and_then(|x| x.get("created_at"))
            .cloned()
            .unwrap_or_else(|| json!(ev.ts));
        items.insert(
            id.to_string(),
            json!({"id": id, "board": board, "joke": joke, "notes": notes, "was": was, "by": ev.by,
                   "created_at": created, "updated_at": ev.ts}),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(op: &str, id: &str, data: Value) -> Event {
        Event::new(op, id, data, "owner", "2026-11-16T12:00:00+00:00")
    }

    fn set(items: &mut Items, board: &str, joke: &str, notes: Value) -> CreativeResult<()> {
        let mut data = json!({"board": board, "joke": joke});
        data["notes"] = notes;
        Setnotes.apply(items, &ev("set", &note_key(board, joke), data))
    }

    #[test]
    fn the_newest_list_wins_and_keeps_the_one_it_replaced() {
        let mut items = Items::new();
        assert_eq!(notes_of(&items, "sb:fri", "joke:fridge"), Vec::<String>::new());
        set(&mut items, "sb:fri", "joke:fridge", json!(["open on the beep"])).unwrap();
        set(
            &mut items,
            "sb:fri",
            "joke:fridge",
            json!(["open on the beep", "slow down"]),
        )
        .unwrap();
        assert_eq!(
            notes_of(&items, "sb:fri", "joke:fridge"),
            ["open on the beep", "slow down"]
        );
        let it = &items["sb:fri|joke:fridge"];
        assert_eq!(it["was"], json!(["open on the beep"]), "Undo is the list before");
        assert_eq!(
            (&it["board"], &it["joke"], &it["by"]),
            (&json!("sb:fri"), &json!("joke:fridge"), &json!("owner"))
        );
        // cleared: the item stays, with nothing in it
        set(&mut items, "sb:fri", "joke:fridge", json!([])).unwrap();
        assert_eq!(notes_of(&items, "sb:fri", "joke:fridge"), Vec::<String>::new());
        assert_eq!(
            items["sb:fri|joke:fridge"]["was"],
            json!(["open on the beep", "slow down"])
        );
    }

    #[test]
    fn the_notes_belong_to_one_bit_in_one_set() {
        let mut items = Items::new();
        set(&mut items, "sb:fri", "joke:fridge", json!(["late show version"])).unwrap();
        set(&mut items, "sb:sat", "joke:fridge", json!(["clean version"])).unwrap();
        set(&mut items, "sb:fri", "joke:gps", json!(["after the fridge"])).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(notes_of(&items, "sb:fri", "joke:fridge"), ["late show version"]);
        assert_eq!(notes_of(&items, "sb:sat", "joke:fridge"), ["clean version"]);
        assert_eq!(notes_of(&items, "sb:sat", "joke:gps"), Vec::<String>::new());
    }

    #[test]
    fn a_note_is_cleaned_to_one_line() {
        assert_eq!(clean_note("  slow\tdown\nhere \u{7}"), "slow down here");
        assert_eq!(clean_note(" \n "), "");
        assert_eq!(clean_note("caf\u{e9} \u{2014} beat"), "caf\u{e9} \u{2014} beat");
        // a line separator is a line break too; a direction override would reorder what he reads
        assert_eq!(clean_note("one\u{2028}two\u{2029}three"), "one two three");
        assert_eq!(clean_note("\u{202E}drawkcab\u{202C} \u{2066}x\u{2069}"), "drawkcab   x");
        // a joiner inside an emoji stays
        assert_eq!(
            clean_note("ok \u{1F468}\u{200D}\u{1F469}"),
            "ok \u{1F468}\u{200D}\u{1F469}"
        );
    }

    #[test]
    fn the_caps_of_the_route_say_why() {
        let list = |xs: &[&str]| xs.iter().map(|x| (*x).to_string()).collect::<Vec<_>>();
        let thirteen: Vec<String> = (0..13).map(|i| format!("note {i}")).collect();
        assert_eq!(refusal(&thirteen).as_deref(), Some("at most 12 notes under a bit"));
        assert_eq!(refusal(&thirteen[..12]), None);
        assert_eq!(
            refusal(&["x".repeat(201)]).as_deref(),
            Some("a note is at most 200 characters")
        );
        assert_eq!(refusal(&["\u{e9}".repeat(200)]), None, "code points, not bytes");
        for bad in [&["fine", ""][..], &["two\nlines"], &[" padded "], &["a\u{2028}b"]] {
            assert_eq!(
                refusal(&list(bad)).as_deref(),
                Some("a note is one line of words"),
                "{bad:?}"
            );
        }
        assert_eq!(refusal(&[]), None);
    }

    #[test]
    fn every_refusal_says_why_and_writes_nothing() {
        let key = note_key("sb:fri", "joke:fridge");
        let data = |notes: Value| json!({"board": "sb:fri", "joke": "joke:fridge", "notes": notes});
        let cases = [
            (
                ev("set", &key, json!({"joke": "joke:fridge", "notes": []})),
                "board and joke are required",
            ),
            (
                ev("set", &key, json!({"board": "sb:fri", "joke": "a|b", "notes": []})),
                "board and joke are required",
            ),
            (
                ev("set", "sb:fri|joke:gps", data(json!([]))),
                "id must be the board and the joke",
            ),
            (ev("set", &key, data(json!("one"))), "notes must be a list"),
            (ev("set", &key, data(json!(["one", 2]))), "a note is text"),
            (ev("clear", &key, data(json!([]))), "unknown op clear"),
        ];
        for (e, want) in cases {
            let mut items = Items::new();
            match Setnotes.apply(&mut items, &e) {
                Err(CreativeError::Refused(m)) => assert_eq!(m, want),
                other => panic!("expected a refusal, got {other:?}"),
            }
            assert_eq!(items.len(), 0);
        }
    }

    /// The caps and the cleaning are the route's: whatever list of texts the log holds is replayed as it is, so a
    /// rule changed later can never make the store unreadable.
    #[test]
    fn the_reducer_replays_whatever_texts_the_log_holds() {
        let mut items = Items::new();
        let thirteen: Vec<String> = (0..13).map(|i| format!("note {i}")).collect();
        set(&mut items, "sb:fri", "joke:fridge", json!(thirteen)).unwrap();
        assert_eq!(notes_of(&items, "sb:fri", "joke:fridge").len(), 13);
        set(
            &mut items,
            "sb:fri",
            "joke:fridge",
            json!(["x".repeat(500), " padded ", "two\nlines", ""]),
        )
        .unwrap();
        assert_eq!(notes_of(&items, "sb:fri", "joke:fridge").len(), 4);
    }
}
