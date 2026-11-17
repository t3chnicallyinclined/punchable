//! A set list's bullet notes and seams (`docs/design/SPEC-2026-11-16-bits-sets-tapes.md` §3.3; Rust only, no Python
//! twin, so none of this is in `setlist::build`, which stays the port of `notebook.build`): what `GET
//! /api/boards/<id>/build` adds to each step once the build is made.
//!
//! - `notes` and `notes_from`: his own bullet notes for the bit in this set (the `setnotes` store), or, while he has
//!   written none, the words the stage notes sheet showed before ([`derived`]: the written version's setup and reveal,
//!   else his typed idea, else his taped line), marked `derived`. His first note replaces them.
//! - `transition_in.same_shape`: the "same shape twice" seam ([`same_shape_twice`]): two neighbours with the same
//!   turn, by a stored `same_shape` edge or by the moves they share (the rule "Around this bit" lists Same shape by).
//! - `transition_in.source`: whose tie the seam's words name ([`tie_source`]: `his`, `ai` or none), so the app can
//!   put the AI badge on a callback or a premise he has not confirmed.

use std::collections::HashSet;
use std::hash::BuildHasher;

use pk_core::py;
use serde_json::{Map, Value, json};

use crate::connections::{idf_overlap, names};
use crate::corpus::{Corpus, s};
use crate::reducers::links::sorted_pair;
use crate::reducers::setnotes::notes_of;
use crate::store::Items;

/// The share of their device and structure labels two bits must hold in common (`connections::groups`, same shape).
const SHAPE_MIN_OVERLAP: f64 = 0.3;
/// How many such labels they must share.
const SHAPE_MIN_SHARED: usize = 2;
/// A label counts for a bit from this weight.
const SHAPE_MIN_WEIGHT: f64 = 0.55;

/// A derived note, at most (code points). A version can be a document of 40,000 characters and its setup most of
/// it; the set page shows two lines of a derived note, and the whole text is on the step already (`text`).
pub const MAX_DERIVED_CHARS: usize = 300;

/// A derived note cut to [`MAX_DERIVED_CHARS`], with an ellipsis where it was cut.
fn clip(t: &str) -> String {
    if py::len(t) <= MAX_DERIVED_CHARS {
        t.to_string()
    } else {
        let head: String = t.chars().take(MAX_DERIVED_CHARS - 1).collect();
        format!("{}…", py::strip(&head))
    }
}

fn text_of<'a>(v: Option<&'a Value>, k: &str) -> Option<&'a str> {
    v.and_then(|x| x.get(k))
        .and_then(Value::as_str)
        .map(py::strip)
        .filter(|t| !t.is_empty())
}

/// What a step shows while he has written no note for it, in the stage notes sheet's own order
/// (`app/src/rules/setListRules.ts` `notesFor`): the written version's setup and reveal; else his typed idea whole;
/// else his taped line (with the ellipses of a cut); else the guessed split of whatever text there is; else nothing.
pub fn derived(step: &Map<String, Value>) -> Vec<String> {
    let text = step.get("text");
    let split: Vec<String> = ["setup", "reveal"]
        .iter()
        .filter_map(|k| text_of(text, k).map(clip))
        .collect();
    let written = text.and_then(|t| t.get("from")).is_some_and(|f| !f.is_null());
    if written && !split.is_empty() {
        return split;
    }
    let own = |k: &str| {
        step.get(k)
            .and_then(Value::as_str)
            .map(py::strip)
            .filter(|t| !t.is_empty())
    };
    if let Some(idea) = own("idea") {
        return vec![clip(idea)];
    }
    if let Some(q) = own("quote") {
        let cut = |k: &str| if py::truthy(step.get(k)) { "…" } else { "" };
        return vec![clip(&format!("{}{q}{}", cut("cut_start"), cut("cut_end")))];
    }
    split
}

/// The "same shape twice" seam between two neighbours, or none: a stored `same_shape` edge between them (his, or the
/// AI's), else the moves they share when those clear the bar Around this bit uses, unless he said the two are not
/// connected (`gone`: the `links` store's dismissed pairs). `{source: "his" | "ai", shared: [names], edge}`.
pub fn same_shape_twice<S: BuildHasher>(
    c: &Corpus,
    one: &str,
    next: &str,
    gone: &HashSet<(String, String), S>,
) -> Option<Value> {
    if one == next || !c.is_joke(one) || !c.is_joke(next) {
        return None;
    }
    let stored = c
        .edges_by_node
        .get(one)
        .into_iter()
        .flatten()
        .map(|i| &c.edges[*i])
        .find(|e| {
            let ends = (s(e, "src").unwrap_or(""), s(e, "dst").unwrap_or(""));
            s(e, "kind") == Some("same_shape")
                && s(e, "status") != Some("rejected")
                && (ends == (one, next) || ends == (next, one))
        });
    if let Some(e) = stored {
        let source = if c.source_of(e) == "his" { "his" } else { "ai" };
        return Some(json!({"source": source, "shared": [], "edge": e.get("id")}));
    }
    if gone.contains(&sorted_pair(one, next)) {
        return None;
    }
    let (overlap, shared) = idf_overlap(c, one, next, &["device", "structure"], SHAPE_MIN_WEIGHT);
    if overlap < SHAPE_MIN_OVERLAP || shared.len() < SHAPE_MIN_SHARED {
        return None;
    }
    let top: Vec<&String> = shared.iter().take(3).collect();
    Some(json!({"source": "ai", "shared": names(c, &top), "edge": null}))
}

/// Whose tie a seam's words name, where that can be told: `his` (he did the two back to back, he wrote the segue,
/// he made or confirmed the edge) or `ai` (the edge is the app's reading and he has not confirmed it); none for a
/// seam that names no tie of its own (a shared world or subject, no link). `words` is `setlist::transition_words`'s.
/// A callback or a shared premise is read off the edges between the two: his when any such edge is his.
pub fn tie_source(c: &Corpus, one: &str, next: &str, words: &str) -> Option<&'static str> {
    let kind = match words {
        "Done back to back on stage" | "Your segue" => return Some("his"),
        "Could lead in" => return Some("ai"),
        "Callback" => "callback_to",
        "Same premise" => "same_premise",
        _ => return None,
    };
    let mut found = None;
    for e in c.edges_by_node.get(one).into_iter().flatten().map(|i| &c.edges[*i]) {
        let ends = (s(e, "src").unwrap_or(""), s(e, "dst").unwrap_or(""));
        if s(e, "kind") != Some(kind) || !(ends == (one, next) || ends == (next, one)) {
            continue;
        }
        if c.source_of(e) == "his" {
            return Some("his");
        }
        found = Some("ai");
    }
    found
}

/// Add the notes, the same-shape seam and whose tie the seam names to the steps of a build (`setlist::build`'s value,
/// after the route has put each step's `text` and `idea` in). `notes` is the `setnotes` store; `gone` the pairs he
/// said are not connected.
pub fn decorate<S: BuildHasher>(
    build: &mut Value,
    c: &Corpus,
    board: &str,
    notes: &Items,
    gone: &HashSet<(String, String), S>,
) {
    let Some(steps) = build.get_mut("steps").and_then(Value::as_array_mut) else {
        return;
    };
    let mut before: Option<String> = None;
    for step in steps.iter_mut().filter_map(Value::as_object_mut) {
        let jid = step.get("joke_id").and_then(Value::as_str).unwrap_or("").to_string();
        let his = notes_of(notes, board, &jid);
        let (list, from) = if his.is_empty() {
            (derived(step), "derived")
        } else {
            (his, "his")
        };
        step.insert("notes".into(), json!(list));
        step.insert("notes_from".into(), json!(from));
        if let (Some(prev), Some(tin)) = (
            before.as_deref(),
            step.get_mut("transition_in").and_then(Value::as_object_mut),
        ) {
            tin.insert(
                "same_shape".into(),
                same_shape_twice(c, prev, &jid, gone).unwrap_or(Value::Null),
            );
            let words = tin.get("words").and_then(Value::as_str).unwrap_or("").to_string();
            tin.insert("source".into(), json!(tie_source(c, prev, &jid, &words)));
        }
        before = Some(jid);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;
    use crate::corpus::{Parts, Vectors};
    use crate::event::Event;
    use crate::reducers::setnotes::{Setnotes, note_key};
    use crate::store::Reducer;

    fn step(v: &Value) -> Map<String, Value> {
        v.as_object().cloned().expect("an object")
    }

    #[test]
    fn derived_notes_follow_the_stage_sheet() {
        // a written version: its setup and reveal, the idea and the taped line ignored
        assert_eq!(
            derived(&step(
                &json!({"text": {"setup": " My fridge has wifi. ", "reveal": "It judges me.", "from": "unit:a"},
                                 "idea": "fridge", "quote": "taped"})
            )),
            ["My fridge has wifi.", "It judges me."]
        );
        // a typed idea: whole, not the server's guessed split of it
        assert_eq!(
            derived(&step(
                &json!({"text": {"setup": "The fridge", "reveal": "judges me", "from": null},
                                 "idea": "The fridge judges me"})
            )),
            ["The fridge judges me"]
        );
        // only a taped line, cut at both ends
        assert_eq!(
            derived(&step(
                &json!({"text": {"setup": null, "reveal": null, "from": null}, "idea": null,
                                 "quote": "and the fridge said no", "cut_start": true, "cut_end": true})
            )),
            ["…and the fridge said no…"]
        );
        // an unattributed split, then nothing
        assert_eq!(
            derived(&step(&json!({"text": {"setup": "a", "reveal": "", "from": null}}))),
            ["a"]
        );
        assert_eq!(
            derived(&step(
                &json!({"text": {"setup": " ", "reveal": null, "from": "unit:a"}})
            )),
            Vec::<String>::new()
        );
        assert_eq!(derived(&step(&json!({}))), Vec::<String>::new());
        // a version that is a whole document: the note is its first 300 characters, cut with an ellipsis
        let page = "meter ".repeat(2000);
        let long = derived(&step(
            &json!({"text": {"setup": page, "reveal": "short", "from": "unit:a"}}),
        ));
        assert_eq!(long.len(), 2);
        assert_eq!(py::len(&long[0]), MAX_DERIVED_CHARS);
        assert!(
            long[0].starts_with("meter meter") && long[0].ends_with('…'),
            "{}",
            long[0]
        );
        assert_eq!(long[1], "short");
    }

    /// Four bits: the fridge and the GPS share two moves, the doorbell shares one with them, the closer shares none;
    /// the doorbell and the closer are tied by the AI's `same_shape` edge.
    fn corpus() -> Corpus {
        let label = |id: &str, kind: &str, name: &str| json!({"id": id, "type": "label", "kind": kind, "name": name});
        let joke = |id: &str, title: &str| json!({"id": id, "type": "joke", "title": title, "stage": "idea"});
        let has = |n: usize, j: &str, l: &str| {
            json!({"id": format!("e:{n:012}"), "kind": "has_label", "src": j, "dst": l, "status": "confirmed",
                   "confidence": 0.9})
        };
        let g = json!({"version": 1, "nodes": [
            joke("joke:fridge", "Smart fridge"), joke("joke:gps", "GPS in the driveway"),
            joke("joke:bell", "Smart doorbell"), joke("joke:closer", "Closer"),
            label("label:device:misdirect", "device", "Misdirect"),
            label("label:structure:list", "structure", "List of three"),
            label("label:device:actout", "device", "Act out"),
            label("label:topic:home", "topic", "Home"),
        ], "edges": [
            has(1, "joke:fridge", "label:device:misdirect"), has(2, "joke:fridge", "label:structure:list"),
            has(3, "joke:gps", "label:device:misdirect"), has(4, "joke:gps", "label:structure:list"),
            has(5, "joke:bell", "label:device:misdirect"), has(6, "joke:bell", "label:device:actout"),
            has(7, "joke:closer", "label:topic:home"), has(8, "joke:fridge", "label:topic:home"),
            {"id": "e:000000000009", "kind": "same_shape", "src": "joke:closer", "dst": "joke:bell",
             "status": "proposed", "confidence": 0.6},
            // a callback the AI proposed, and a shared premise he made himself
            {"id": "e:000000000010", "kind": "callback_to", "src": "joke:bell", "dst": "joke:fridge",
             "status": "proposed", "confidence": 0.6},
            {"id": "e:000000000011", "kind": "same_premise", "src": "joke:gps", "dst": "joke:closer",
             "status": "confirmed", "by": "owner"},
        ]});
        let files = BTreeMap::from([("kb/v1/graph.json".to_string(), g.to_string())]);
        Corpus::new(Parts::from_files(&files, Vec::new(), Arc::new(Vectors::default()), "owner").expect("a corpus"))
    }

    #[test]
    fn same_shape_twice_is_a_stored_edge_or_two_shared_moves() {
        let c = corpus();
        let none = HashSet::new();
        // two shared moves, either order
        let seam = same_shape_twice(&c, "joke:fridge", "joke:gps", &none).expect("the same turn twice");
        assert_eq!(seam["source"], "ai");
        assert_eq!(seam["edge"], Value::Null);
        let mut shared: Vec<&str> = seam["shared"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        shared.sort_unstable();
        assert_eq!(shared, ["List of three", "Misdirect"]);
        assert!(same_shape_twice(&c, "joke:gps", "joke:fridge", &none).is_some());
        // one shared move is not a shape; a shared topic is not a move
        assert_eq!(same_shape_twice(&c, "joke:fridge", "joke:bell", &none), None);
        assert_eq!(same_shape_twice(&c, "joke:fridge", "joke:closer", &none), None);
        // the stored edge, whichever end comes first
        for (a, b) in [("joke:bell", "joke:closer"), ("joke:closer", "joke:bell")] {
            assert_eq!(
                same_shape_twice(&c, a, b, &none),
                Some(json!({"source": "ai", "shared": [], "edge": "e:000000000009"}))
            );
        }
        // "not connected" silences the computed seam, never a stored edge (that one is rejected instead)
        let gone = HashSet::from([
            sorted_pair("joke:gps", "joke:fridge"),
            sorted_pair("joke:bell", "joke:closer"),
        ]);
        assert_eq!(same_shape_twice(&c, "joke:fridge", "joke:gps", &gone), None);
        assert!(same_shape_twice(&c, "joke:bell", "joke:closer", &gone).is_some());
        // not a bit, or the same bit
        assert_eq!(same_shape_twice(&c, "joke:fridge", "joke:nope", &none), None);
        assert_eq!(same_shape_twice(&c, "joke:fridge", "joke:fridge", &none), None);
    }

    #[test]
    fn a_tie_is_his_or_the_apps_reading() {
        let c = corpus();
        // what he did or wrote himself; what the words already call a suggestion
        assert_eq!(
            tie_source(&c, "joke:a", "joke:b", "Done back to back on stage"),
            Some("his")
        );
        assert_eq!(tie_source(&c, "joke:a", "joke:b", "Your segue"), Some("his"));
        assert_eq!(tie_source(&c, "joke:a", "joke:b", "Could lead in"), Some("ai"));
        // an edge the AI proposed, whichever of the two comes first on the list; an edge he made
        assert_eq!(tie_source(&c, "joke:fridge", "joke:bell", "Callback"), Some("ai"));
        assert_eq!(tie_source(&c, "joke:bell", "joke:fridge", "Callback"), Some("ai"));
        assert_eq!(tie_source(&c, "joke:gps", "joke:closer", "Same premise"), Some("his"));
        // the words name a tie the two do not have, or no tie at all
        assert_eq!(tie_source(&c, "joke:fridge", "joke:gps", "Callback"), None);
        assert_eq!(tie_source(&c, "joke:fridge", "joke:bell", "Same premise"), None);
        assert_eq!(tie_source(&c, "joke:fridge", "joke:bell", "No link yet"), None);
        assert_eq!(
            tie_source(&c, "joke:fridge", "joke:bell", "Same world: Home, needs a line between"),
            None
        );
    }

    #[test]
    fn decorate_puts_his_notes_first_and_marks_the_seam() {
        let c = corpus();
        let mut notes = Items::new();
        let data = json!({"board": "sb:fri", "joke": "joke:gps", "notes": ["slow down", "look left"]});
        Setnotes
            .apply(
                &mut notes,
                &Event::new(
                    "set",
                    &note_key("sb:fri", "joke:gps"),
                    data,
                    "owner",
                    "2026-11-16T12:00:00+00:00",
                ),
            )
            .unwrap();
        let tin = |from: &str, to: &str| json!({"words": "No link yet", "weak": true, "kind": "jump", "from": from, "to": to});
        let mut build = json!({"steps": [
            {"joke_id": "joke:fridge", "transition_in": null, "idea": "The fridge judges me",
             "text": {"setup": null, "reveal": null, "from": null}},
            {"joke_id": "joke:gps", "transition_in": tin("joke:fridge", "joke:gps"), "idea": "ignored",
             "text": {"setup": null, "reveal": null, "from": null}},
            {"joke_id": "joke:closer", "text": {},
             "transition_in": {"words": "Same premise", "weak": false, "kind": "link", "from": "joke:gps",
                               "to": "joke:closer"}},
        ]});
        decorate(&mut build, &c, "sb:fri", &notes, &HashSet::new());
        let steps = build["steps"].as_array().unwrap();
        assert_eq!(
            (&steps[0]["notes"], &steps[0]["notes_from"]),
            (&json!(["The fridge judges me"]), &json!("derived"))
        );
        assert_eq!(steps[0]["transition_in"], Value::Null, "the first bit has no seam");
        assert_eq!(
            (&steps[1]["notes"], &steps[1]["notes_from"]),
            (&json!(["slow down", "look left"]), &json!("his")),
            "his notes replace the derived ones"
        );
        assert_eq!(steps[1]["transition_in"]["same_shape"]["source"], "ai");
        assert_eq!(
            steps[1]["transition_in"]["words"], "No link yet",
            "the rest of the seam is untouched"
        );
        assert_eq!(
            (&steps[2]["notes"], &steps[2]["notes_from"]),
            (&json!([]), &json!("derived"))
        );
        assert_eq!(steps[2]["transition_in"]["same_shape"], Value::Null);
        assert_eq!(
            (
                &steps[1]["transition_in"]["source"],
                &steps[2]["transition_in"]["source"]
            ),
            (&Value::Null, &json!("his")),
            "no tie to name between the first two; the premise he made himself between the last two"
        );
        // another set's notes are not this set's
        let mut other = json!({"steps": [{"joke_id": "joke:gps", "transition_in": null}]});
        decorate(&mut other, &c, "sb:sat", &notes, &HashSet::new());
        assert_eq!(other["steps"][0]["notes_from"], "derived");
    }
}
