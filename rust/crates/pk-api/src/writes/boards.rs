//! The set lists' writes (`creative_api.Routes.board_create`, `board_patch`, `board_item`, `bridge`): `POST
//! /api/boards` (a new list: `story.create`), `PATCH /api/boards/<id>` (every change in one body, each op checked
//! on a copy before any is written: `story._apply` on a probe), `POST /api/boards/<id>/items` (one bit added after
//! another) and `POST /api/boards/<id>/bridge` (a transition drafted by the hosted worker: a `bridge` job queued, or
//! the finished one). Every answer is the board as the Storyboard screen reads it (`story.board_view`).

use std::sync::Arc;

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_creative::Items;
use pk_creative::merge::sha8_value;
use pk_creative::util::slug;
use serde_json::{Map, Value, json};

use super::{Creative, ProbeError, Req, detached, http, int_arg, is_id, request};
use crate::auth::Caller;
use crate::creative::GenCorpus;
use crate::error::{ApiError, ApiResult, json_response};
use crate::state::AppState;

type Obj = Map<String, Value>;

/// `_joke(c, jid)`: a joke id that is a joke of the notebook, else the Python's 400 / 404.
pub fn joke_of(c: &GenCorpus, jid: Option<&Value>) -> ApiResult<String> {
    let Some(jid) = jid.and_then(Value::as_str).filter(|j| is_id(j)) else {
        return Err(http(400, "bad joke id"));
    };
    match c.nodes.get(jid) {
        Some(n) if n.get("type").and_then(Value::as_str) == Some("joke") => Ok(jid.to_string()),
        _ => Err(http(404, format!("no such joke {jid}"))),
    }
}

/// `_jokes_list(c, xs)`: at most 200 joke ids, each a joke, none twice.
pub fn jokes_list(c: &GenCorpus, xs: Option<&Value>) -> ApiResult<Vec<String>> {
    let Some(xs) = xs.and_then(Value::as_array).filter(|a| a.len() <= 200) else {
        return Err(http(400, "joke_ids must be a list (at most 200)"));
    };
    let out: Vec<String> = xs.iter().map(|j| joke_of(c, Some(j))).collect::<ApiResult<_>>()?;
    let distinct: std::collections::HashSet<&String> = out.iter().collect();
    if distinct.len() != out.len() {
        return Err(http(400, "joke_ids has duplicates"));
    }
    Ok(out)
}

/// `_board(st, bid)`: a set list by id (an archived one is found; a deleted one is not).
pub fn board_item<'a>(items: &'a Items, bid: &str) -> ApiResult<&'a Obj> {
    items
        .get(bid)
        .and_then(Value::as_object)
        .filter(|b| !py::truthy(b.get("deleted")))
        .ok_or_else(|| http(404, "no such storyboard"))
}

/// What every board write starts from: this generation's notebook (over data proven current, as a write
/// validates: `AppState::data_for_write`) and the set lists as they are, under the creative lock (the Python's
/// `cr.lock` around a creative POST / PATCH).
pub(super) async fn start(st: &AppState, name: &str) -> ApiResult<(Arc<GenCorpus>, Creative, Arc<Items>)> {
    let d = st.data_for_write().await?;
    let c = d.corpus(st).await?;
    let cr = Creative::new(st.clone());
    let items = cr.items(name).await?;
    Ok((c, cr, items))
}

/// `story.board_view(c, b)` off the async threads.
async fn view_of(c: Arc<GenCorpus>, b: Value) -> ApiResult<Value> {
    let Some(b) = b.as_object().cloned() else {
        return Err(ApiError::internal("TypeError", "the board is not an object"));
    };
    tokio::task::spawn_blocking(move || pk_creative::setlist::board_view(&c.story(), &c, &b))
        .await
        .map_err(|e| ApiError::internal("JoinError", e))
}

/// `POST /api/boards {title, joke_ids?, target_s?, notes?}`: 201 with the new list.
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
        let (c, cr, items) = start(&st, "storyboards").await?;
        let title = b
            .get("title")
            .and_then(Value::as_str)
            .filter(|t| !py::strip(t).is_empty() && py::len(t) <= 200)
            .ok_or_else(|| http(400, "title is required (1-200 characters)"))?;
        let notes = match b.get("notes").filter(|n| py::truthy(Some(n))) {
            None => "",
            Some(Value::String(s)) if py::len(s) <= 20000 => s.as_str(),
            Some(_) => return Err(http(400, "notes must be text")),
        };
        let ids = jokes_list(
            &c,
            Some(b.get("joke_ids").filter(|j| py::truthy(Some(j))).unwrap_or(&json!([]))),
        )?;
        let target_s = int_arg(b.get("target_s"), "target_s", 30, 6 * 3600)?
            .filter(|t| *t != 0)
            .unwrap_or(900);
        let title = py::strip(title);
        // `story.create`: `sb:<slug>-<sha8([title, '', len(items)])>`
        let bid = format!(
            "sb:{}-{}",
            slug(title, 32),
            sha8_value(&json!([title, "", items.len()]))
        );
        let rec = cr
            .append(
                "storyboards",
                "create",
                &bid,
                json!({"title": title, "joke_ids": ids, "target_s": target_s, "notes": notes, "seed": null}),
                &owner,
            )
            .await?;
        view_of(c, rec).await
    })
    .await?;
    Ok(json_response(StatusCode::CREATED, &out))
}

/// `PATCH /api/boards/<id>`: `title`, `notes`, `target_s`, `joke_ids`, `add`, `remove`, `move`, `archive`,
/// `unarchive`, `delete`, all in one body; every op checked on a copy before any is written.
pub async fn patch(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path(bid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _cr = st.creative_lock().lock().await;
        let (c, cr, items) = start(&st, "storyboards").await?;
        let board = board_item(&items, &bid)?;
        let id = board.get("id").map(py::str).unwrap_or_default();
        // the delete answer names the title as it was when the request started (`b["title"]`)
        let title0 = board.get("title").cloned().unwrap_or(Value::Null);
        let mut ops: Vec<(String, String, Value)> = Vec::new();
        let mut upd = Map::new();
        if let Some(t) = b.get("title") {
            let t = t
                .as_str()
                .filter(|t| !py::strip(t).is_empty() && py::len(t) <= 200)
                .ok_or_else(|| http(400, "title must be 1-200 characters"))?;
            upd.insert("title".into(), json!(py::strip(t)));
        }
        if let Some(n) = b.get("notes") {
            if n.as_str().is_none_or(|s| py::len(s) > 20000) {
                return Err(http(400, "notes must be text"));
            }
            upd.insert("notes".into(), n.clone());
        }
        if let Some(t) = b.get("target_s") {
            upd.insert("target_s".into(), json!(int_arg(Some(t), "target_s", 30, 6 * 3600)?));
        }
        if !upd.is_empty() {
            ops.push(("update".into(), id.clone(), Value::Object(upd)));
        }
        if let Some(j) = b.get("joke_ids") {
            ops.push((
                "set_jokes".into(),
                id.clone(),
                json!({"joke_ids": jokes_list(&c, Some(j))?}),
            ));
        }
        if let Some(a) = b.get("add") {
            let (joke, at) = match a {
                Value::Object(o) => (o.get("joke_id"), o.get("at")),
                v => (Some(v), None),
            };
            ops.push((
                "add".into(),
                id.clone(),
                json!({"joke_id": joke_of(&c, joke)?, "at": int_arg(at, "at", 0, 200)?}),
            ));
        }
        if let Some(r) = b.get("remove") {
            let joke = match r {
                Value::Object(o) => o.get("joke_id"),
                v => Some(v),
            };
            ops.push(("remove".into(), id.clone(), json!({"joke_id": joke_of(&c, joke)?})));
        }
        if let Some(mv) = b.get("move") {
            let empty = Map::new();
            let mv = mv.as_object().unwrap_or(&empty);
            ops.push((
                "move".into(),
                id.clone(),
                json!({"from": int_arg(mv.get("from"), "move.from", 0, 200)?,
                       "to": int_arg(mv.get("to"), "move.to", 0, 200)?}),
            ));
        }
        if b.get("unarchive") == Some(&Value::Bool(true)) {
            // first, so a body that brings a list back and edits it works in one go
            ops.insert(0, ("unarchive".into(), id.clone(), json!({})));
        }
        if b.get("archive") == Some(&Value::Bool(true)) {
            ops.push(("archive".into(), id.clone(), json!({})));
        }
        let delete = b.get("delete") == Some(&Value::Bool(true));
        if delete {
            ops.push(("delete".into(), id.clone(), json!({})));
        }
        if ops.is_empty() {
            return Err(http(
                400,
                "nothing to change (title, notes, target_s, joke_ids, add, remove, move, archive, unarchive, delete)",
            ));
        }
        cr.probe("storyboards", &ops, &owner).await?;
        // one transaction for the whole sequence: a writer of the same list between two ops (the Python writes row by
        // row) can no longer leave half of this body written
        let last = cr.append_all("storyboards", &ops, &owner).await?;
        if delete {
            return Ok(json!({"ok": true, "board": {"id": id, "title": title0, "deleted": true}}));
        }
        view_of(c, last).await
    })
    .await?;
    Ok(json_response(StatusCode::OK, &out))
}

/// `POST /api/boards/<id>/items {joke_id, after?}`: one bit added (after another, at a place, or at the end).
pub async fn item(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path(bid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _cr = st.creative_lock().lock().await;
        let (c, cr, items) = start(&st, "storyboards").await?;
        let board = board_item(&items, &bid)?;
        let id = board.get("id").map(py::str).unwrap_or_default();
        let jid = joke_of(&c, b.get("joke_id"))?;
        let ids: Vec<&Value> = board
            .get("joke_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect();
        let at: Option<i64> = match b.get("after") {
            Some(Value::String(after)) => {
                let pos = ids
                    .iter()
                    .position(|x| x.as_str() == Some(after))
                    .ok_or_else(|| http(400, format!("{after} is not on the board")))?;
                Some(i64::try_from(pos).unwrap_or(i64::MAX) + 1)
            }
            Some(v) if !v.is_null() => int_arg(Some(v), "after", 0, 200)?.map(|n| n + 1),
            _ => None,
        };
        // `S.add`: the reducer's refusal is a 409 when the bit is already on the board, else a 400
        let op = [("add".to_string(), id.clone(), json!({"joke_id": jid, "at": at}))];
        match cr.probe("storyboards", &op, &owner).await {
            Ok(()) => {}
            Err(ProbeError::Refused(m)) => {
                return Err(http(if m.contains("already") { 409 } else { 400 }, m));
            }
            Err(ProbeError::Failed(e)) => return Err(e),
        }
        let rec = cr.append("storyboards", "add", &id, op[0].2.clone(), &owner).await?;
        view_of(c, rec).await
    })
    .await?;
    Ok(json_response(StatusCode::OK, &out))
}

/// `POST /api/boards/<id>/bridge {from, to, fresh?}`: a transition between two bits, drafted by the worker.
pub async fn bridge(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path(bid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let (status, out) = detached(async move {
        let owner = st.config().owner_id.clone();
        let _cr = st.creative_lock().lock().await;
        let (c, cr, items) = start(&st, "storyboards").await?;
        board_item(&items, &bid)?;
        let a = joke_of(&c, b.get("from"))?;
        let to = joke_of(&c, b.get("to"))?;
        super::jobs::enqueue(
            &cr,
            "bridge",
            &format!("bridge|{a}|{to}"),
            json!({"from": a, "to": to, "board": bid}),
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
    fn int_arg_is_the_pythons_int() {
        assert_eq!(int_arg(None, "x", 0, 10).unwrap(), None);
        assert_eq!(int_arg(Some(&Value::Null), "x", 0, 10).unwrap(), None);
        assert_eq!(int_arg(Some(&json!(3.9)), "x", 0, 10).unwrap(), Some(3));
        assert_eq!(int_arg(Some(&json!(10)), "x", 0, 10).unwrap(), Some(10));
        let e = int_arg(Some(&json!(true)), "at", 0, 200).unwrap_err();
        assert_eq!(e.msg, "at must be a number between 0 and 200");
        assert!(int_arg(Some(&json!("3")), "x", 0, 10).is_err());
        assert!(int_arg(Some(&json!(11)), "x", 0, 10).is_err());
    }
}
