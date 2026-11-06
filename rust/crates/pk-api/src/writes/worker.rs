//! The hosted worker's routes (T10 group 5; `creative_api.Routes.w_*`, `profile_api` worker half): `POST
//! /api/worker/claim` (the heartbeat and the next job: `check_claim`, the last claim kept in the database, the oldest
//! claimable job marked `claim`), `POST /api/worker/jobs/<id>/result` (`done` or `fail`), `GET
//! /api/worker/decisions` (every decision line with its hash, paged), `GET /api/worker/creative` (a store's lines,
//! with the replayed items on `?snapshot=`), `GET` and `PUT /api/worker/profile/voice` (the worker's copy of his
//! voice profile; the loader's write). `GET /api/worker/jobs/<id>/file` (the upload's bytes) is in `uploads`; a
//! `process_set` job that ends `ok` deletes its upload there too (`w_result`: no media library).
//!
//! The queue is the `jobs` store's rows on both servers, so a job this service hands out is the one the Python
//! would have handed out, and one marked done here is done there on its next read; two claims racing for the
//! same row are arbitrated by the unique index, the loser reading again (`Creative::append_planned`).

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use pk_core::py;
use pk_creative::Items;
use serde_json::{Map, Value, json};

use super::jobs::brief;
use super::{Creative, Plan, Req, Written, actor, detached, http, request_within, store_err};
use crate::auth::Caller;
use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::state::AppState;
use crate::worker::{self, can_drop, dumps_ascii_len};

/// The body limit on the worker's routes (`8 * 1024 * 1024 if path.startswith("/api/worker/")`).
pub const WORKER_BODY: usize = 8 * 1024 * 1024;
/// A result over this many characters, dumped, is refused (`len(json.dumps(res)) > 2_000_000`).
pub const RESULT_MAX: usize = 2_000_000;
/// How many decision lines one page carries.
pub const PAGE: usize = 2000;
/// The stores a worker may read (`w_creative`).
pub const WORKER_STORES: [&str; 5] = ["storyboards", "workingsets", "venues", "merges", "suggestions"];

fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

fn obj(v: &Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

/// `POST /api/worker/claim {kinds?, worker?, hosted?, model?, paused?}`: the heartbeat lands in the database for
/// both servers; a paused worker gets no job (204); else the oldest claimable job, marked claimed (200), or 204.
pub async fn claim(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request_within(st, raw.as_deref(), &headers, body, WORKER_BODY).await?;
    let (kinds, seen) = worker::check_claim(&b)?;
    let out = detached(async move {
        let store = st.store_handle().ok_or_else(super::not_connected)?;
        worker::save_seen(&store, &seen).await.map_err(store_err)?;
        if seen.paused.is_some() {
            // over its AI budget: the heartbeat lands, nothing is handed out, the jobs stay queued
            return Ok(None);
        }
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let by = format!("worker:{}", seen.name);
        let now = worker::unix_now();
        let plan = |items: &Items| -> ApiResult<Plan> {
            Ok(match worker::pick(items, kinds.as_deref(), now) {
                Some(id) => Plan::Event {
                    op: "claim".into(),
                    id,
                    data: json!({"worker": seen.name}),
                },
                None => Plan::Done(StatusCode::NO_CONTENT, Value::Null),
            })
        };
        // after twelve lost races the Python's `claim` gives up and the route says 204: nothing for this worker now
        let lost = Some((StatusCode::NO_CONTENT, Value::Null));
        match cr.append_planned_or("jobs", &by, None, plan, lost).await? {
            Written::Landed(job) => Ok(Some(job)),
            Written::Skipped(..) => Ok(None),
        }
    })
    .await?;
    Ok(match out {
        Some(job) => ok(&json!({"job": job})),
        None => no_content(),
    })
}

/// `POST /api/worker/jobs/<id>/result {ok, result?, error?, retry?}`: the job's end, `done` with its result or
/// `fail` with the error (back to the queue on `retry` while attempts remain).
pub async fn result(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    Path(jid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request_within(st, raw.as_deref(), &headers, body, WORKER_BODY).await?;
    let out = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let check = |items: &Items| -> ApiResult<()> {
            let j = items
                .get(&jid)
                .and_then(Value::as_object)
                .ok_or_else(|| http(404, "no such job"))?;
            let status = j.get("status").and_then(Value::as_str).unwrap_or("");
            if status != "claimed" && !can_drop(j, &b) {
                return Err(http(409, format!("job is {status}")));
            }
            Ok(())
        };
        let items = cr.items("jobs").await?;
        check(items.as_ref())?;
        let (op, data) = if py::truthy(b.get("ok")) {
            let res = b
                .get("result")
                .filter(|r| r.is_object())
                .ok_or_else(|| http(400, "result must be an object"))?;
            if dumps_ascii_len(res) > RESULT_MAX {
                return Err(http(413, "result too large"));
            }
            ("done", json!({"result": res}))
        } else {
            let error: String = b
                .get("error")
                .filter(|e| py::truthy(Some(e)))
                .map_or_else(|| "failed".to_string(), py::str)
                .chars()
                .take(2000)
                .collect();
            ("fail", json!({"error": error, "retry": py::truthy(b.get("retry"))}))
        };
        // the status checked again over the rows as they are at the append (the Python's check runs under its lock)
        let plan = |items: &Items| -> ApiResult<Plan> {
            check(items)?;
            Ok(Plan::Event {
                op: op.into(),
                id: jid.clone(),
                data: data.clone(),
            })
        };
        match cr.append_planned("jobs", "worker", None, plan).await? {
            Written::Landed(job) => {
                let job = obj(&job);
                // `w_result`: the transcriber is done with the recording, so it goes (no media library)
                if st.config().upload_db
                    && py::truthy(b.get("ok"))
                    && job.get("kind").and_then(Value::as_str) == Some("process_set")
                    && let Some(name) = super::uploads::upload_name(&job)
                {
                    let store = st.store_handle().ok_or_else(super::not_connected)?;
                    store.upload_delete(&name).await.map_err(store_err)?;
                }
                Ok(json!({"ok": true, "job": brief(&job)}))
            }
            Written::Skipped(..) => Err(ApiError::internal("StateError", "a result's plan never skips")),
        }
    })
    .await?;
    Ok(ok(&out))
}

/// `?after=`: `max(0, int(arg)) if arg.isdigit() else 0` (ASCII digits; a number past `usize` is the end).
fn after_of(q: &Query) -> usize {
    let arg = q.get("after").unwrap_or("0");
    if arg.is_empty() || !arg.chars().all(|c| c.is_ascii_digit()) {
        return 0;
    }
    arg.parse::<usize>().unwrap_or(usize::MAX)
}

/// `GET /api/worker/decisions?after=N`: the decision lines from `after`, 2000 at a time, each with the sha256 of its
/// compact sorted dump (`_h`), and the total.
pub async fn decisions(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let d = crate::routes::prepare_rows(&st, &q).await?;
    let after = after_of(&q);
    let all = d.decisions();
    let lines: Vec<Value> = all
        .iter()
        .enumerate()
        .skip(after)
        .take(PAGE)
        .map(|(i, dec)| {
            let v = Value::Object(dec.0.clone());
            let h = pk_store::line_hash(&py::dumps_compact_sorted(&v));
            json!({"index": i, "decision": v, "h": h})
        })
        .collect();
    Ok(ok(&json!({"total": all.len(), "after": after, "lines": lines})))
}

/// `GET /api/worker/creative?store=<name>[&snapshot=1]`: one creative store's lines as the database holds them (the
/// Python extends its own file to them first; this service keeps no file), and the replayed items on `?snapshot=`.
pub async fn creative(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let name = q
        .get("store")
        .filter(|n| WORKER_STORES.contains(n))
        .ok_or_else(|| {
            http(
                400,
                "store must be storyboards | workingsets | venues | merges | suggestions",
            )
        })?
        .to_string();
    let cr = Creative::new(st.clone());
    let mut out = if q.get("snapshot").is_some_and(|s| !s.is_empty()) {
        let (lines, items) = cr.lines_and_items(&name).await?;
        json!({"store": name, "total": lines.len(), "lines": lines, "items": *items})
    } else {
        let lines = cr.lines(&name).await?;
        json!({"store": name, "total": lines.len(), "lines": lines})
    };
    // the Python's key order: store, total, lines, then items
    if let Some(m) = out.as_object_mut()
        && let Some(items) = m.remove("items")
    {
        m.insert("items".into(), items);
    }
    Ok(ok(&out))
}

/// `GET /api/worker/profile/voice`: the worker's copy (`view(doc, worker=True, laughs=laugh_lines(cr, kb))`): no
/// `previous`, and his top punchlines by their laughs from this generation's notebook.
pub async fn profile_get(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let items = crate::routes::creative::items_of(&st, |s| &s.profile).await?;
    // `laugh_lines` never fails a GET: a notebook that cannot be built here gives none
    let laughs = match st.data().await {
        Ok(d) => match d.corpus(&st).await {
            Ok(c) => pk_creative::reducers::learned::laugh_lines(&c, pk_creative::reducers::learned::CAP_LAUGH),
            Err(e) => {
                tracing::warn!("laugh lines: the notebook could not be built: {e}; none");
                Vec::new()
            }
        },
        Err(e) => {
            tracing::warn!("laugh lines: no data: {e}; none");
            Vec::new()
        }
    };
    let doc = items.get(super::learned::VOICE_ID).and_then(Value::as_object);
    Ok(ok(&crate::routes::stores::voice_worker_view(
        doc,
        &st.config().owner_id,
        laughs,
    )))
}

/// `import_actor(h, body)`: a worker write names its actor (`import:<what>` or `ai:<what>`), never the comic's id.
fn import_actor(st: &AppState, caller: &Caller, b: &Map<String, Value>) -> ApiResult<String> {
    let a = actor(st, caller, b)?;
    match b.get("actor").and_then(Value::as_str) {
        Some(want) if want == a => Ok(a),
        _ => Err(ApiError::bad_request(
            "a worker write must name its actor: import:<source> or ai:<what>",
        )),
    }
}

/// `PUT /api/worker/profile/voice {text, actor}`: the one-off loader's write, with the worker's credentials.
pub async fn profile_put(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request_within(st, raw.as_deref(), &headers, body, WORKER_BODY).await?;
    let by = import_actor(&st, &caller, &b)?;
    let out = detached(async move { super::account::put_voice(&st, &b, &by).await }).await?;
    Ok(ok(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn after_is_a_non_negative_ascii_number_else_zero() {
        let q = |s: &str| Query::parse(Some(s));
        assert_eq!(after_of(&q("after=12")), 12);
        assert_eq!(after_of(&q("after=0")), 0);
        assert_eq!(after_of(&q("after=-3")), 0);
        assert_eq!(after_of(&q("after=x")), 0);
        assert_eq!(after_of(&q("after=")), 0);
        assert_eq!(after_of(&q("")), 0);
        assert_eq!(after_of(&q("after=99999999999999999999999")), usize::MAX);
    }
}
