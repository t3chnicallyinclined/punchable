//! His sets' writes (`creative_api.Routes.ws_create`, `ws_version`, `ws_lock`, `try_add`, `try_patch`, `punchup`,
//! `joke_punchup`): `POST /api/workingsets` (a set list promoted to the set he performs: `workingset.promote`),
//! `POST /api/workingsets/<id>/versions` (a new version with a note), `POST /api/workingsets/<id>/lock` (a bit or
//! one verbatim line locked or unlocked), `POST /api/workingsets/<id>/try` (his own try-tonight note), `PATCH
//! /api/workingsets/<id>/try/<note>` (a note ticked), and the punch-up drafts (`POST
//! /api/workingsets/<id>/bits/<joke>/punchup`, `POST /api/jokes/<id>/punchup`): a locked bit or one with no tape is
//! answered here with no model; otherwise a `punchup` job is queued for the hosted worker, keyed by the locks.

use std::sync::{Arc, LazyLock};

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_creative::Items;
use pk_creative::merge::sha8_value;
use pk_creative::util::slug;
use regex::Regex;
use serde_json::{Map, Value, json};
use sha1::{Digest as _, Sha1};

use super::boards::{board_item, joke_of, jokes_list, start};
use super::{Creative, Pre, Req, Written, detached, http, int_arg, refuse_legal, request};
use crate::auth::Caller;
use crate::creative::GenCorpus;
use crate::error::{ApiError, ApiResult, json_response};
use crate::state::AppState;

type Obj = Map<String, Value>;

/// `AI_NOTE_RE`: the id of one of the AI's try-tonight notes (`<joke id>|<kind>`).
static AI_NOTE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\Ajoke:[^|]{1,280}\|(put_back|try_without|dropped|over_time|slow_start)\n?\z")
        .unwrap_or_else(|e| unreachable!("constant regex: {e}"))
});

/// `_ws(st, wid)`: a working set by id, not deleted.
pub fn ws_item<'a>(items: &'a Items, wid: &str) -> ApiResult<&'a Obj> {
    items
        .get(wid)
        .and_then(Value::as_object)
        .filter(|w| !py::truthy(w.get("deleted")))
        .ok_or_else(|| http(404, "no such working set"))
}

/// `creative_api.ws_detail(c, ws)` off the async threads.
async fn detail_of(c: Arc<GenCorpus>, ws: Value) -> ApiResult<Value> {
    let Some(ws) = ws.as_object().cloned() else {
        return Err(ApiError::internal("TypeError", "the working set is not an object"));
    };
    tokio::task::spawn_blocking(move || pk_creative::today::ws_detail(&c, &ws))
        .await
        .map_err(|e| ApiError::internal("JoinError", e))
}

/// `int(round(d / 5.0) * 5)`: Python rounds half to even.
fn to_five(d: f64) -> i64 {
    ((d / 5.0).round_ties_even() * 5.0) as i64
}

/// One bit of a version as `promote` and `ws_version` build it.
fn bit_of(c: &GenCorpus, j: &str) -> Value {
    let (d, basis) = c.duration(j);
    json!({"joke_id": j, "title": c.title(j), "target_s": to_five(d), "target_basis": basis})
}

/// The bits of the current version (`W.current(ws)["bits"]`).
fn current_bits(ws: &Obj) -> Vec<&Value> {
    pk_creative::mysets::current(ws)
        .and_then(|v| v.get("bits"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect()
}

fn bit_ids(ws: &Obj) -> Vec<&str> {
    current_bits(ws)
        .into_iter()
        .filter_map(|b| b.get("joke_id").and_then(Value::as_str))
        .collect()
}

/// `POST /api/workingsets {board_id, target_s?}`: 201 with the set made from the list.
pub async fn create(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _cr = st.creative_lock().lock().await;
        let (c, cr, boards) = start(&st, "storyboards").await?;
        let bid = b.get("board_id").and_then(Value::as_str).unwrap_or("");
        let board = board_item(&boards, bid)?;
        let ids: Vec<&str> = board
            .get("joke_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if ids.is_empty() {
            return Err(http(400, "the storyboard has no bits"));
        }
        let target_s = int_arg(b.get("target_s"), "target_s", 30, 6 * 3600)?;
        // `workingset.promote`
        let bits: Vec<Value> = ids.iter().map(|j| bit_of(&c, j)).collect();
        let title = board.get("title").map(py::str).unwrap_or_default();
        let board_id = board.get("id").map(py::str).unwrap_or_default();
        let wid = format!("ws:{}-{}", slug(&title, 28), sha8_value(&json!([board_id, ""])));
        let sum: i64 = bits
            .iter()
            .filter_map(|b| b.get("target_s"))
            .filter_map(Value::as_i64)
            .sum();
        let target = target_s
            .filter(|t| *t != 0)
            .or_else(|| {
                board
                    .get("target_s")
                    .filter(|t| py::truthy(Some(t)))
                    .and_then(Value::as_i64)
            })
            .unwrap_or(sum);
        let rec = cr
            .append(
                "workingsets",
                "promote",
                &wid,
                json!({"title": title, "storyboard_id": board_id, "bits": bits, "target_s": target,
                       "note": format!("v1: promoted from storyboard '{title}'")}),
                &owner,
            )
            .await?;
        detail_of(c, rec).await
    })
    .await?;
    Ok(json_response(StatusCode::CREATED, &out))
}

/// `POST /api/workingsets/<id>/versions {note, joke_ids?, target_s?}`: 201 with the set at its new version.
pub async fn version(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path(wid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _cr = st.creative_lock().lock().await;
        let (c, cr, items) = start(&st, "workingsets").await?;
        let ws = ws_item(&items, &wid)?;
        let note = b
            .get("note")
            .and_then(Value::as_str)
            .filter(|n| !py::strip(n).is_empty() && py::len(n) <= 2000)
            .ok_or_else(|| http(400, "a new version needs a note saying what changed"))?;
        let bits: Value = match b.get("joke_ids") {
            None => Value::Null,
            Some(j) => {
                let ids = jokes_list(&c, Some(j))?;
                if ids.is_empty() {
                    return Err(http(400, "joke_ids is empty"));
                }
                let old: Vec<&Value> = current_bits(ws);
                Value::Array(
                    ids.iter()
                        .map(|j| {
                            old.iter()
                                .find(|x| x.get("joke_id").and_then(Value::as_str) == Some(j))
                                .map_or_else(|| bit_of(&c, j), |x| (*x).clone())
                        })
                        .collect(),
                )
            }
        };
        let target_s = int_arg(b.get("target_s"), "target_s", 30, 6 * 3600)?;
        let id = ws.get("id").map(py::str).unwrap_or_default();
        let rec = cr
            .append(
                "workingsets",
                "new_version",
                &id,
                json!({"note": py::strip(note), "bits": bits, "target_s": target_s}),
                &owner,
            )
            .await?;
        detail_of(c, rec).await
    })
    .await?;
    Ok(json_response(StatusCode::CREATED, &out))
}

/// `find_line_node(c, jid, line)`: the set segment (newest night first) or the unit where `line` is verbatim.
fn find_line_node(c: &GenCorpus, jid: &str, line: &str) -> Option<String> {
    for p in c.perfs.get(jid).into_iter().flatten().rev() {
        for sid in &p.seg_ids {
            if c.text_of(sid, None).is_some_and(|t| t.contains(line)) {
                return Some(sid.clone());
            }
        }
    }
    let u = c.lead_unit(jid)?;
    c.nodes
        .get(&u)
        .and_then(|n| n.get("text"))
        .and_then(Value::as_str)
        .is_some_and(|t| t.contains(line))
        .then_some(u)
}

/// `POST /api/workingsets/<id>/lock {joke_id, line?, node?, unlock?}`: a bit, or one verbatim line of it, locked or
/// unlocked.
pub async fn lock(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path(wid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _cr = st.creative_lock().lock().await;
        let (c, cr, items) = start(&st, "workingsets").await?;
        let ws = ws_item(&items, &wid)?;
        let jid = joke_of(&c, b.get("joke_id"))?;
        let in_locks = ws
            .get("locks")
            .and_then(Value::as_object)
            .is_some_and(|l| l.contains_key(&jid));
        if !bit_ids(ws).contains(&jid.as_str()) && !in_locks {
            return Err(http(400, format!("{jid} is not in this working set")));
        }
        let unlock = py::truthy(b.get("unlock"));
        let id = ws.get("id").map(py::str).unwrap_or_default();
        let line = b.get("line");
        if line.is_some_and(|l| !l.is_string()) {
            return Err(http(400, "line must be text (leave it out to lock the whole bit)"));
        }
        let (op, data) = match line.and_then(Value::as_str) {
            None => (if unlock { "unlock_bit" } else { "lock_bit" }, json!({"joke_id": jid})),
            Some(line) => {
                if py::strip(line).is_empty() || py::len(line) > 2000 {
                    return Err(http(400, "line must be text"));
                }
                if unlock {
                    ("unlock_line", json!({"joke_id": jid, "text": line}))
                } else {
                    let node = b
                        .get("node")
                        .filter(|n| py::truthy(Some(n)))
                        .map(py::str)
                        .or_else(|| find_line_node(&c, &jid, line))
                        .ok_or_else(|| {
                            http(
                                400,
                                "that line is not verbatim in any taped night or written version of the bit",
                            )
                        })?;
                    // `workingset.lock_line`: `c.verify({"node": node, "quote": text})`
                    if !c.text_of(&node, None).is_some_and(|t| t.contains(line)) {
                        return Err(http(400, format!("line is not verbatim in {node}")));
                    }
                    ("lock_line", json!({"joke_id": jid, "text": line, "node": node}))
                }
            }
        };
        let rec = cr.append("workingsets", op, &id, data, &owner).await?;
        detail_of(c, rec).await
    })
    .await?;
    Ok(json_response(StatusCode::OK, &out))
}

/// `POST /api/workingsets/<id>/try {joke_id?, text}`: 201 with his try-tonight note.
pub async fn try_add(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path(wid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _cr = st.creative_lock().lock().await;
        let (c, cr, items) = start(&st, "workingsets").await?;
        let ws = ws_item(&items, &wid)?;
        let jid = match b.get("joke_id").filter(|j| !j.is_null()) {
            None => None,
            Some(j) => {
                let jid = joke_of(&c, Some(j))?;
                if !bit_ids(ws).contains(&jid.as_str()) {
                    return Err(http(400, format!("{jid} is not in this working set")));
                }
                Some(jid)
            }
        };
        let text = b
            .get("text")
            .and_then(Value::as_str)
            .filter(|t| !py::strip(t).is_empty() && py::len(t) <= 500)
            .map(py::strip)
            .ok_or_else(|| http(400, "text is required (up to 500 characters)"))?;
        let id = ws.get("id").map(py::str).unwrap_or_default();
        // `workingset.add_try`: the note's id hashes the stamp the event carries and how many notes there are
        let ts = pk_core::time::now_iso();
        let n = ws.get("try_notes").and_then(Value::as_array).map_or(0, Vec::len);
        let nid = format!("try:{}", sha8_value(&json!([id, jid, text, ts, n])));
        let Written::Landed(rec) = cr
            .append_if_at(
                "workingsets",
                "try_add",
                &id,
                json!({"note_id": nid, "joke_id": jid, "text": text}),
                &owner,
                Some(&ts),
                |_| Ok(Pre::Go),
            )
            .await?
        else {
            unreachable!("a check that always says go")
        };
        let note = rec
            .get("try_notes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|n| n.get("id").and_then(Value::as_str) == Some(&nid))
            .and_then(Value::as_object)
            .cloned()
            .ok_or_else(|| ApiError::internal("StopIteration", ""))?;
        let mut note = note;
        note.insert(
            "title".into(),
            jid.as_deref().map_or(Value::Null, |j| json!(c.title(j))),
        );
        note.insert("source".into(), json!("his"));
        Ok(json!({"working_set": rec.get("id"), "note": note}))
    })
    .await?;
    Ok(json_response(StatusCode::CREATED, &out))
}

/// `PATCH /api/workingsets/<id>/try/<note> {done}`: a try note ticked or unticked (his, or one of the AI's).
pub async fn try_patch(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path((wid, nid)): Path<(String, String)>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let items = cr.items("workingsets").await?;
        let ws = ws_item(&items, &wid)?;
        let Some(done) = b.get("done").and_then(Value::as_bool) else {
            return Err(http(400, "done must be true or false"));
        };
        let mine_of = |ws: &Obj| {
            ws.get("try_notes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|n| n.get("id").and_then(Value::as_str) == Some(&nid))
                .and_then(Value::as_object)
                .cloned()
        };
        let mine = mine_of(ws);
        if mine.is_none() && !AI_NOTE_RE.is_match(&nid) {
            return Err(http(404, "no such try note"));
        }
        let was = match &mine {
            Some(m) => py::truthy(m.get("done")),
            None => py::truthy(ws.get("try_done").and_then(|t| t.get(&nid))),
        };
        let id = ws.get("id").map(py::str).unwrap_or_default();
        let (ws_after, mine) = if was == done {
            (Value::Object(ws.clone()), mine)
        } else {
            let rec = cr
                .append(
                    "workingsets",
                    "try_done",
                    &id,
                    json!({"note_id": nid, "done": done}),
                    &owner,
                )
                .await?;
            let m = rec.as_object().and_then(mine_of);
            (rec, m)
        };
        let note = match mine {
            Some(mut m) => {
                m.insert("source".into(), json!("his"));
                Value::Object(m)
            }
            None => json!({"id": nid, "done": done, "source": "inferred"}),
        };
        Ok(json!({"working_set": ws_after.get("id"), "changed": was != done, "note": note}))
    })
    .await?;
    Ok(json_response(StatusCode::OK, &out))
}

/// `workingset.punch_up(c, ws, jid, complete=_no_model)` for the cases answered without a model: a locked bit, a
/// bit with no taped night. (A bit with lines to punch up is never answered here: it is queued.)
fn punch_up_no_model(c: &GenCorpus, ws: &Value, jid: &str) -> Value {
    let mut base = json!({"joke_id": jid, "title": c.title(jid), "draft": true,
                          "label": pk_creative::write::DRAFT_LABEL, "drafts": []});
    let (status, why) = if pk_creative::punch::bit_locked(ws, jid) {
        ("locked", "This bit is locked (perfected); no punch-ups.")
    } else {
        ("no_tape", "No taped night to work from.")
    };
    base["status"] = json!(status);
    base["why"] = json!(why);
    base
}

/// `workingset.punch_text(c, jid)`: every word a punch-up would show the model (the legal check runs on it).
fn punch_text(c: &GenCorpus, jid: &str) -> String {
    let lines = pk_creative::punch::latest_lines(c, jid)
        .map(|(_, l)| l)
        .unwrap_or_default();
    let lead = c
        .lead_unit(jid)
        .and_then(|u| {
            c.nodes
                .get(&u)
                .and_then(|n| n.get("text"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    let mut out = lines.iter().map(|(ph, _)| ph.as_str()).collect::<Vec<_>>().join("\n");
    out.push('\n');
    out.push_str(&lead);
    out
}

/// `sha1(json.dumps(x, sort_keys=True))[:n]`.
fn sha_sorted(v: &Value, n: usize) -> String {
    hex::encode(Sha1::digest(py::dumps_sorted(v).as_bytes()))[..n].to_string()
}

/// `POST /api/workingsets/<id>/bits/<joke>/punchup {fresh?}`: drafts for one bit of the set, by the worker.
pub async fn punchup(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path((wid, jid)): Path<(String, String)>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let (status, out) = detached(async move {
        let owner = st.config().owner_id.clone();
        let _cr = st.creative_lock().lock().await;
        let (c, cr, items) = start(&st, "workingsets").await?;
        let ws = ws_item(&items, &wid)?.clone();
        joke_of(&c, Some(&json!(jid)))?;
        let wsv = Value::Object(ws.clone());
        if pk_creative::punch::bit_locked(&wsv, &jid) {
            return Ok((StatusCode::OK, punch_up_no_model(&c, &wsv, &jid)));
        }
        refuse_legal(&st, [punch_text(&c, &jid).as_str()])?;
        let locks = ws
            .get("locks")
            .and_then(|l| l.get(&jid))
            .filter(|l| py::truthy(Some(l)))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let key = format!(
            "punchup|{}|{jid}|v{}|{}",
            ws.get("id").map(py::str).unwrap_or_default(),
            ws.get("current").map(py::str).unwrap_or_default(),
            sha_sorted(&locks, 10)
        );
        super::jobs::enqueue(
            &cr,
            "punchup",
            &key,
            json!({"working_set": ws, "joke_id": jid}),
            py::truthy(b.get("fresh")),
            &owner,
        )
        .await
    })
    .await?;
    Ok(json_response(status, &out))
}

/// `POST /api/jokes/<id>/punchup {fresh?}`: Brainstorm drafts for any bit; his locks on it in ANY set hold.
pub async fn joke_punchup(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path(jid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let (status, out) = detached(async move {
        let owner = st.config().owner_id.clone();
        let _cr = st.creative_lock().lock().await;
        let (c, cr, items) = start(&st, "workingsets").await?;
        joke_of(&c, Some(&json!(jid)))?;
        let mut bit = false;
        let mut lines: Vec<Value> = Vec::new();
        let mut sets: Vec<&Obj> = items.values().filter_map(Value::as_object).collect();
        sets.sort_by_key(|w| w.get("id").map(py::str).unwrap_or_default());
        for w in sets {
            if py::truthy(w.get("deleted")) {
                continue;
            }
            let Some(x) = w.get("locks").and_then(|l| l.get(&jid)).filter(|x| py::truthy(Some(x))) else {
                continue;
            };
            bit = bit || py::truthy(x.get("bit"));
            for ln in x.get("lines").and_then(Value::as_array).into_iter().flatten() {
                let have = lines.iter().any(|y| y.get("text") == ln.get("text"));
                if !have {
                    lines.push(ln.clone());
                }
            }
        }
        let lk = json!({"bit": bit, "lines": lines});
        let ws = json!({"id": null, "title": null, "current": 0, "locks": {&jid: lk.clone()}});
        if bit || c.perfs.get(&jid).is_none_or(Vec::is_empty) {
            return Ok((StatusCode::OK, punch_up_no_model(&c, &ws, &jid)));
        }
        refuse_legal(&st, [punch_text(&c, &jid).as_str()])?;
        let key = format!("punchup|joke|{jid}|{}", sha_sorted(&lk, 10));
        super::jobs::enqueue(
            &cr,
            "punchup",
            &key,
            json!({"working_set": ws, "joke_id": jid}),
            py::truthy(b.get("fresh")),
            &owner,
        )
        .await
    })
    .await?;
    Ok(json_response(status, &out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_second_targets_round_half_to_even() {
        assert_eq!(to_five(12.5), 10, "2.5 rounds to 2");
        assert_eq!(to_five(17.5), 20, "3.5 rounds to 4");
        assert_eq!(to_five(13.0), 15);
        assert_eq!(to_five(0.0), 0);
    }

    #[test]
    fn an_ai_note_id_is_a_joke_and_a_kind() {
        assert!(AI_NOTE_RE.is_match("joke:a|put_back"));
        assert!(AI_NOTE_RE.is_match("joke:a b|slow_start"));
        assert!(!AI_NOTE_RE.is_match("joke:a|nope"));
        assert!(!AI_NOTE_RE.is_match("try:abcd1234"));
    }

    #[test]
    fn the_locks_key_hashes_the_sorted_dump() {
        // sha1 of '{"bit": false, "lines": []}'
        let want = hex::encode(Sha1::digest(br#"{"bit": false, "lines": []}"#))[..10].to_string();
        assert_eq!(sha_sorted(&json!({"lines": [], "bit": false}), 10), want);
    }
}
