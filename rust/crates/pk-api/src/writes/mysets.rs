//! His sets, the writes (`api-dev/mysets_api.py`): `POST /api/mysets` (a new set), `PATCH /api/mysets/<id>` (a new
//! version, a rename, a delete), `POST /api/mysets/<id>/lock` and `/bridge`, and `GET /api/mysets/match` (the set a
//! recording lines up with). A set is a working set: the same `workingsets` store, so every write is one of his actions
//! in that log, and every answer is the set screen (`mysets.view`) as the Python builds it after the write.

use std::sync::Arc;

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_creative::Items;
use pk_creative::mysets as M;
use serde_json::{Map, Value, json};

use super::boards::{board_item, joke_of, jokes_list, start};
use super::workingsets::ws_item;
use super::{Creative, Req, detached, http, int_arg, request};
use crate::auth::Caller;
use crate::creative::GenCorpus;
use crate::error::{ApiError, ApiResult, json_response, ok};
use crate::state::AppState;

type Obj = Map<String, Value>;

const STORE: &str = "workingsets";

/// `_title(v, required)`: one to 200 characters, stripped; `None` when it is left out and not required.
fn title_of(v: Option<&Value>, required: bool) -> ApiResult<Option<String>> {
    if !required && v.is_none_or(Value::is_null) {
        return Ok(None);
    }
    v.and_then(Value::as_str)
        .filter(|t| !py::strip(t).is_empty() && py::len(t) <= 200)
        .map(|t| Some(py::strip(t).to_string()))
        .ok_or_else(|| http(400, "title is required (1-200 characters)"))
}

/// `_target(v, required)`: seconds, 60 to 14400.
fn target_of(v: Option<&Value>, required: bool) -> ApiResult<Option<i64>> {
    let t = int_arg(v, "target_s", 60, 4 * 3600)?;
    if t.is_none() && required {
        return Err(http(400, "target_s is required (seconds, 60 to 14400)"));
    }
    Ok(t)
}

/// The set screen of `wid` as the rows are now: `mysets.view(c, ws, live)`, off the async threads.
async fn view_of(st: &AppState, c: Arc<GenCorpus>, cr: &Creative, wid: &str) -> ApiResult<Value> {
    let items: Arc<Items> = cr.items(STORE).await?;
    let (wid, owner) = (wid.to_string(), st.config().owner_id.clone());
    tokio::task::spawn_blocking(move || {
        let ws = items.get(&wid).and_then(Value::as_object)?;
        let live: Vec<&Obj> = items
            .values()
            .filter_map(Value::as_object)
            .filter(|w| !py::truthy(w.get("deleted")))
            .collect();
        Some(M::view(&c.story(), &c, ws, &live, &owner))
    })
    .await
    .map_err(|e| ApiError::internal("JoinError", e))?
    .ok_or_else(|| ApiError::internal("KeyError", "the set vanished"))
}

fn bit_ids(ws: &Obj) -> Vec<String> {
    M::current(ws)
        .and_then(|v| v.get("bits"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|b| b.get("joke_id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn text(ws: &Obj, k: &str) -> String {
    ws.get(k).map(py::str).unwrap_or_default()
}

/// `POST /api/mysets`: `{title, target_s, joke_ids?}`, `{board_id, title?, target_s?}` or
/// `{from_set, target_s, title?, joke_ids?}`; 201 with the set screen.
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
        let (c, cr, items) = start(&st, STORE).await?;
        let (title, target, ids, note, sb): (String, i64, Vec<String>, String, Option<String>) =
            if let Some(bid) = b.get("board_id").filter(|v| !v.is_null()) {
                let boards = cr.items("storyboards").await?;
                let board = board_item(&boards, bid.as_str().unwrap_or(""))?;
                let ids = board
                    .get("joke_ids")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|j| c.is_joke(j))
                    .map(str::to_string)
                    .collect();
                let btitle = text(board, "title");
                let title = title_of(b.get("title"), false)?.unwrap_or_else(|| btitle.clone());
                let target = target_of(b.get("target_s"), false)?
                    .or_else(|| {
                        board
                            .get("target_s")
                            .filter(|t| py::truthy(Some(t)))
                            .and_then(Value::as_i64)
                    })
                    .unwrap_or(600);
                (
                    title,
                    target,
                    ids,
                    format!("v1: made from the board '{btitle}'"),
                    Some(text(board, "id")),
                )
            } else if let Some(src) = b.get("from_set").filter(|v| !v.is_null()) {
                let src = ws_item(&items, src.as_str().unwrap_or(""))?;
                let target = target_of(b.get("target_s"), true)?.unwrap_or(0);
                let sug = M::trim(&c, src, target);
                let ids = match b.get("joke_ids") {
                    Some(j) => jokes_list(&c, Some(j))?,
                    None => sug["keep"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|x| x["joke_id"].as_str().map(str::to_string))
                        .collect(),
                };
                let title = title_of(b.get("title"), false)?.unwrap_or_else(|| py::str(&sug["title"]));
                #[expect(clippy::cast_precision_loss, reason = "seconds of a set")]
                let words = M::length_words(target as f64);
                (
                    title,
                    target,
                    ids,
                    format!("v1: a {words} from '{}'", text(src, "title")),
                    None,
                )
            } else {
                let target = target_of(b.get("target_s"), true)?.unwrap_or(0);
                let ids = jokes_list(
                    &c,
                    Some(b.get("joke_ids").filter(|j| py::truthy(Some(j))).unwrap_or(&json!([]))),
                )?;
                let title = title_of(b.get("title"), true)?.unwrap_or_default();
                (title, target, ids, "v1: a new set".to_string(), None)
            };
        let (id, data) = M::create_event(&c, items.len(), &title, target, &ids, Some(&note), sb.as_deref());
        let rec = cr.append(STORE, "promote", &id, data, &owner).await?;
        view_of(&st, c, &cr, &text(rec.as_object().unwrap_or(&Obj::new()), "id")).await
    })
    .await?;
    Ok(json_response(StatusCode::CREATED, &out))
}

/// `PATCH /api/mysets/<id>`: `{joke_ids?, target_s?, note?}` a new version (the note written for him when he leaves it
/// out); `{title}` a rename; `{delete: true}`.
pub async fn patch(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path(sid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _cr = st.creative_lock().lock().await;
        let (c, cr, items) = start(&st, STORE).await?;
        let ws = ws_item(&items, &sid)?;
        let wid = text(ws, "id");
        if b.get("delete") == Some(&json!(true)) {
            cr.append(STORE, "delete", &wid, json!({}), &owner).await?;
            return Ok(json!({"deleted": wid}));
        }
        let mut changed = false;
        if let Some(t) = b.get("title") {
            let t = title_of(Some(t), true)?.unwrap_or_default();
            if t != text(ws, "title") {
                cr.append(STORE, "update", &wid, json!({"title": t}), &owner).await?;
                changed = true;
            }
        }
        if b.contains_key("joke_ids") || b.contains_key("target_s") {
            let cur = M::current(ws).cloned().unwrap_or_default();
            let old_ids = bit_ids(ws);
            let ids = match b.get("joke_ids") {
                Some(j) => jokes_list(&c, Some(j))?,
                None => old_ids.clone(),
            };
            let cur_t = cur.get("target_s").and_then(Value::as_i64).unwrap_or(0);
            let target = match b.get("target_s") {
                Some(t) => target_of(Some(t), true)?.unwrap_or(0),
                None => cur_t,
            };
            if ids != old_ids || target != cur_t {
                let note = b.get("note").filter(|n| !n.is_null());
                if let Some(n) = note
                    && !n.as_str().is_some_and(|n| !py::strip(n).is_empty() && py::len(n) <= 2000)
                {
                    return Err(http(400, "note must be text"));
                }
                let given = note.and_then(Value::as_str).map(py::strip).unwrap_or("");
                let note = if given.is_empty() {
                    M::change_note(&c, &old_ids, &ids, cur_t, target)
                } else {
                    given.to_string()
                };
                let old: Vec<&Value> = cur.get("bits").and_then(Value::as_array).into_iter().flatten().collect();
                let bits: Vec<Value> = ids
                    .iter()
                    .map(|j| {
                        old.iter()
                            .find(|x| x.get("joke_id").and_then(Value::as_str) == Some(j))
                            .map_or_else(
                                || {
                                    let (d, basis) = c.duration(j);
                                    json!({"joke_id": j, "title": c.title(j),
                                           "target_s": ((d / 5.0).round_ties_even() * 5.0) as i64, "target_basis": basis})
                                },
                                |x| (*x).clone(),
                            )
                    })
                    .collect();
                cr.append(
                    STORE,
                    "new_version",
                    &wid,
                    json!({"note": note, "bits": bits, "target_s": target}),
                    &owner,
                )
                .await?;
                changed = true;
            }
        }
        if !changed && !["title", "joke_ids", "target_s"].iter().any(|k| b.contains_key(*k)) {
            return Err(http(400, "nothing to change (title, joke_ids, target_s, delete)"));
        }
        view_of(&st, c, &cr, &wid).await
    })
    .await?;
    Ok(json_response(StatusCode::OK, &out))
}

/// `POST /api/mysets/<id>/lock {joke_id, unlock?}`.
pub async fn lock(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path(sid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _cr = st.creative_lock().lock().await;
        let (c, cr, items) = start(&st, STORE).await?;
        let ws = ws_item(&items, &sid)?;
        let jid = joke_of(&c, b.get("joke_id"))?;
        if !bit_ids(ws).contains(&jid) {
            return Err(http(400, format!("{jid} is not in this set")));
        }
        let unlock = py::truthy(b.get("unlock"));
        let wid = text(ws, "id");
        if M::bit_locked(ws, &jid) != !unlock {
            let op = if unlock { "unlock_bit" } else { "lock_bit" };
            cr.append(STORE, op, &wid, json!({"joke_id": jid}), &owner).await?;
        }
        view_of(&st, c, &cr, &wid).await
    })
    .await?;
    Ok(ok(&out))
}

/// `POST /api/mysets/<id>/bridge {from, to, fresh?}`: the bridge draft (cached, or 202 and a job for the worker).
pub async fn bridge(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path(sid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let (status, out) = detached(async move {
        let owner = st.config().owner_id.clone();
        let _cr = st.creative_lock().lock().await;
        let (c, cr, items) = start(&st, STORE).await?;
        let ws = ws_item(&items, &sid)?;
        let a = joke_of(&c, b.get("from"))?;
        let to = joke_of(&c, b.get("to"))?;
        let ids = bit_ids(ws);
        if !ids.contains(&a) || !ids.contains(&to) {
            return Err(http(400, "both bits must be in this set"));
        }
        super::jobs::enqueue(
            &cr,
            "bridge",
            &format!("bridge|{a}|{to}"),
            json!({"from": a, "to": to, "set": text(ws, "id")}),
            py::truthy(b.get("fresh")),
            &owner,
        )
        .await
    })
    .await?;
    Ok(json_response(status, &out))
}
