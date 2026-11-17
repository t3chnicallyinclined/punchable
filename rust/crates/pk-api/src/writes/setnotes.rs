//! His bullet notes under a bit of a set list (`docs/design/SPEC-2026-11-16-bits-sets-tapes.md` §3.3; Rust only, no
//! Python twin):
//!
//! - `POST /api/boards/<id>/notes {joke_id, notes}`: the whole list for that bit in that set, newest wins.
//!   `{ok, board, joke_id, notes, was}`; `was` is the list it replaced, so Undo is the same call with `was`. The
//!   same list again answers `already: true` and writes nothing. An archived list refuses, as every other change to
//!   it does; a bit that is not on the list refuses.
//! - `POST /api/boards/<id>/items` takes an optional `note` (`boards::item`): [`note_arg`] reads it, [`add_note`] puts
//!   it under the bit once the bit is on the list.
//!
//! The store is `setnotes` (`pk_creative::reducers::setnotes`): a new store, so no other process replays it (spec
//! B5). The notes are his words as he typed them: stored whole, checked against the court-material rules like every
//! stored text, never sent to a model by this route.

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_creative::reducers::setnotes::{MAX_NOTE_CHARS, MAX_NOTES, clean_note, note_key, notes_of, refusal};
use serde_json::{Map, Value, json};

use super::boards::{board_item, joke_of, start};
use super::{Creative, Plan, Pre, Req, Written, actor, detached, http, refuse_legal, request};
use crate::auth::Caller;
use crate::error::{ApiResult, ok};
use crate::state::AppState;

/// The store.
pub const STORE: &str = "setnotes";

/// `notes` of a body as it is stored: a list of at most [`MAX_NOTES`] texts, each cleaned to one line
/// (`clean_note`), the empty ones dropped (an empty bullet is no note), each at most [`MAX_NOTE_CHARS`] characters.
/// The count is checked before anything is cleaned and again after, so a body of thousands of blanks is refused as
/// it is.
pub fn notes_arg(v: Option<&Value>) -> ApiResult<Vec<String>> {
    let Some(list) = v.and_then(Value::as_array) else {
        return Err(http(400, "notes must be a list of texts"));
    };
    if list.len() > 4 * MAX_NOTES {
        return Err(http(400, format!("at most {MAX_NOTES} notes under a bit")));
    }
    let mut out = Vec::with_capacity(list.len());
    for x in list {
        let Some(text) = x.as_str() else {
            return Err(http(400, "notes must be a list of texts"));
        };
        let note = clean_note(text);
        if !note.is_empty() {
            out.push(note);
        }
    }
    match refusal(&out) {
        Some(why) => Err(http(400, why)),
        None => Ok(out),
    }
}

/// The optional `note` of an add (`POST /api/boards/<id>/items`): one text, cleaned; absent, `null` or blank is no
/// note.
pub fn note_arg(v: Option<&Value>) -> ApiResult<Option<String>> {
    let text = match v {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) => s,
        Some(_) => return Err(http(400, "note must be text")),
    };
    let note = clean_note(text);
    if py::len(&note) > MAX_NOTE_CHARS {
        return Err(http(400, format!("a note is at most {MAX_NOTE_CHARS} characters")));
    }
    Ok(Some(note).filter(|n| !n.is_empty()))
}

/// Put `note` under `joke` in `board`, after the notes already there (the ones a bit keeps when it is taken off a
/// list and put back): `(the notes now, whether it was added)`. Not added when it is already one of them, or when the
/// bit has [`MAX_NOTES`]: the bit was added all the same, and the answer says which.
///
/// The new list is built over the rows as they are when the row is written (`append_planned`), so a writer that took
/// the place meanwhile is read again, never overwritten with a list made before it.
pub async fn add_note(cr: &Creative, board: &str, joke: &str, note: &str, by: &str) -> ApiResult<(Vec<String>, bool)> {
    let key = note_key(board, joke);
    let plan = |items: &pk_creative::Items| -> ApiResult<Plan> {
        let mut notes = notes_of(items, board, joke);
        if notes.iter().any(|x| x == note) || notes.len() >= MAX_NOTES {
            return Ok(Plan::Done(StatusCode::OK, json!(notes)));
        }
        notes.push(note.to_string());
        Ok(Plan::Event {
            op: "set".to_string(),
            id: key.clone(),
            data: json!({"board": board, "joke": joke, "notes": notes}),
        })
    };
    let strings = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect()
    };
    Ok(match cr.append_planned(STORE, by, None, plan).await? {
        Written::Landed(item) => (strings(item.get("notes")), true),
        Written::Skipped(_, kept) => (strings(Some(&kept)), false),
    })
}

/// Every stored text is checked against the court-material rules (422, nothing written): each note, and the notes
/// read together, so material split across two bullets is still seen.
pub fn refuse_legal_notes(st: &AppState, notes: &[String]) -> ApiResult<()> {
    let joined = notes.join("\n");
    refuse_legal(st, notes.iter().map(String::as_str).chain([joined.as_str()]))
}

/// `POST /api/boards/<id>/notes {joke_id, notes}`.
pub async fn set(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(bid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let notes = notes_arg(b.get("notes"))?;
    refuse_legal_notes(&st, &notes)?;
    let by = actor(&st, &caller, &Map::new())?;
    let out = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let (c, cr, boards) = start(&st, "storyboards").await?;
        let board = board_item(&boards, &bid)?;
        let id = board.get("id").map(py::str).unwrap_or_default();
        if py::truthy(board.get("archived")) {
            // the words every other change to an archived list is refused with (`reducers::story`)
            let title = board.get("title").map(py::str).unwrap_or_default();
            return Err(http(400, format!("\"{title}\" is archived: unarchive it first")));
        }
        let jid = joke_of(&c, b.get("joke_id"))?;
        let on_it = board
            .get("joke_ids")
            .and_then(Value::as_array)
            .is_some_and(|ids| ids.iter().any(|x| x.as_str() == Some(jid.as_str())));
        if !on_it {
            return Err(http(400, format!("{jid} is not on the board")));
        }
        let key = note_key(&id, &jid);
        let (the_board, the_bit, want) = (id.clone(), jid.clone(), notes.clone());
        let same = move |items: &pk_creative::Items| -> ApiResult<Pre> {
            let have = notes_of(items, &the_board, &the_bit);
            // nothing stored and nothing sent is "already" too: clearing what was never written writes nothing
            if have == want {
                return Ok(Pre::Done(
                    StatusCode::OK,
                    json!({"ok": true, "board": the_board, "joke_id": the_bit, "notes": have, "was": have, "already": true}),
                ));
            }
            Ok(Pre::Go)
        };
        let data = json!({"board": id, "joke": jid, "notes": notes});
        Ok(match cr.append_if(STORE, "set", &key, data, &by, same).await? {
            Written::Landed(item) => json!({"ok": true, "board": id, "joke_id": jid,
                                            "notes": item.get("notes").cloned().unwrap_or_else(|| json!([])),
                                            "was": item.get("was").cloned().unwrap_or_else(|| json!([]))}),
            Written::Skipped(_, v) => v,
        })
    })
    .await?;
    Ok(ok(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_cleaned_and_capped() {
        assert_eq!(
            notes_arg(Some(&json!([" open on the beep ", "", "  ", "slow\tdown\n"]))).unwrap(),
            ["open on the beep", "slow down"],
            "blank bullets are no notes; a note is one line"
        );
        assert_eq!(notes_arg(Some(&json!([]))).unwrap(), Vec::<String>::new());
        let msg = |v: Value| notes_arg(Some(&v)).unwrap_err().msg;
        assert_eq!(notes_arg(None).unwrap_err().msg, "notes must be a list of texts");
        assert_eq!(msg(json!("one")), "notes must be a list of texts");
        assert_eq!(msg(json!(["one", 2])), "notes must be a list of texts");
        assert_eq!(msg(json!(["one", ["nested"]])), "notes must be a list of texts");
        let many = |n: usize| json!((0..n).map(|i| format!("note {i}")).collect::<Vec<_>>());
        assert_eq!(notes_arg(Some(&many(12))).unwrap().len(), 12);
        assert_eq!(msg(many(13)), "at most 12 notes under a bit");
        // blanks do not count, up to a point: a body of blanks is refused before it is cleaned
        let mut padded = vec![json!(""); 30];
        padded.push(json!("one"));
        assert_eq!(notes_arg(Some(&json!(padded))).unwrap(), ["one"]);
        assert_eq!(msg(json!(vec![""; 49])), "at most 12 notes under a bit");
        assert_eq!(
            notes_arg(Some(&json!(["é".repeat(200)]))).unwrap().len(),
            1,
            "code points, not bytes"
        );
        assert_eq!(msg(json!(["x".repeat(201)])), "a note is at most 200 characters");
    }

    #[test]
    fn the_note_of_an_add_is_optional() {
        assert_eq!(note_arg(None).unwrap(), None);
        assert_eq!(note_arg(Some(&Value::Null)).unwrap(), None);
        assert_eq!(note_arg(Some(&json!("  \n "))).unwrap(), None);
        assert_eq!(
            note_arg(Some(&json!(" the wifi line "))).unwrap().as_deref(),
            Some("the wifi line")
        );
        assert_eq!(note_arg(Some(&json!(7))).unwrap_err().msg, "note must be text");
        assert_eq!(
            note_arg(Some(&json!("x".repeat(201)))).unwrap_err().msg,
            "a note is at most 200 characters"
        );
    }
}
