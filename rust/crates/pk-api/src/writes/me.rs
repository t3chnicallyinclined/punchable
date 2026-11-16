//! `DELETE /api/me {"confirm": "delete my material"}` (Settings: "delete my material"): ported from `api-dev/server.py`
//! `do_DELETE`, `delete_me` and `inventory`. It never deletes anything. Without sign-in (a loopback dev server) it
//! answers what a deletion WOULD cover (a dry run); with sign-in it records a deletion request in his `account` log
//! (item `deletion`) and answers the same list with 202. The owner deletes by hand and tells him when it is done.
//!
//! The order of the Python's refusals is kept: the 503 while booting, `?backend=`, the body (`body_json`), the confirm
//! phrase, then the inventory. What changes is only WHERE the inventory looks: the Python listed the folders of its
//! own pod; here every line names a database table (or the recordings bucket) and counts its rows. The `what` text of
//! each line, the order, the status, the `deleted` flag and the `note` are the Python's. The lines that had no
//! database home are kept: tapes (the server never holds audio: always 0) and the sets.

use axum::Extension;
use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_store::{from_db, to_db, vars};
use serde_json::{Value, json};

use super::{Creative, Req, detached, request, store_err};
use crate::auth::Caller;
use crate::error::{ApiError, ApiResult, ok};
use crate::state::AppState;

/// `DELETE_CONFIRM`.
pub const DELETE_CONFIRM: &str = "delete my material";

/// The creative stores the inventory lists, in the Python's order.
const STORES: [&str; 5] = ["storyboards", "workingsets", "venues", "account", "jobs"];

/// The dry run's note (`delete_me`, no sign-in).
const DRY_RUN_NOTE: &str = "Local dev: nothing is ever deleted by the API. This is what a deletion would cover.";
/// The recorded request's note.
const REQUESTED_NOTE: &str = "Your request is recorded. The owner deletes it by hand and tells you when it is done.";

fn line(what: impl Into<String>, place: impl Into<String>, count: impl Into<Value>) -> Value {
    json!({"what": what.into(), "where": place.into(), "count": count.into()})
}

/// `inventory(kb)`: what "delete all my data" covers, with sizes. Read-only.
async fn inventory(st: &AppState) -> ApiResult<Vec<Value>> {
    let data = st.data().await?;
    let view = data.view(true).await?;
    let store = st.store_handle().ok_or_else(super::not_connected)?;
    let cfg = &st.config().store;
    let mut out = vec![
        line(
            "your decisions (every confirm, edit, rating, idea)",
            "database table decision",
            json!(view.decisions.len()),
        ),
        line(
            "your jokes, versions and labels (the knowledge graph)",
            "database tables joke, unit, set, performance, label, list and their edges",
            json!(view.jokes.len()),
        ),
        line(
            "the database copy of all of it",
            format!("surrealdb {}/{}", cfg.ns, cfg.db),
            Value::Null,
        ),
    ];
    let cr = Creative::new(st.clone());
    for n in STORES {
        let lines = cr.lines(n).await?;
        let count = lines.iter().filter(|l| !py::strip(l).is_empty()).count();
        out.push(line(
            format!("{n} (append-only log + snapshot)"),
            format!("database table creative_event, store {n}"),
            json!(count),
        ));
    }
    // a set is a folder of files: one per distinct folder under corpus/sets/ in the stored files
    let rows = store
        .last(
            "SELECT path FROM srcfile WHERE string::starts_with(path, $p);",
            vars([("p", to_db(&json!("corpus/sets/")))]),
        )
        .await
        .map_err(store_err)?;
    let mut dirs = std::collections::BTreeSet::new();
    for r in pk_store::store::rows_of(rows).into_iter().map(from_db) {
        if let Some(rest) = r
            .get("path")
            .and_then(Value::as_str)
            .and_then(|p| p.strip_prefix("corpus/sets/"))
            && let Some((dir, _)) = rest.split_once('/')
        {
            dirs.insert(dir.to_string());
        }
    }
    out.push(line(
        "your sets: transcripts, laughs, analyses, clips",
        "database table srcfile, corpus/sets",
        json!(dirs.len()),
    ));
    // the server never holds audio: it is on his phone (GET /api/audio answers "no audio for that set")
    out.push(line(
        "your tapes (audio)",
        "not kept on the server (the recordings stay on your phone)",
        json!(0),
    ));
    let (place, waiting) = if st.config().uploads.is_some() {
        let n = store.upload_rows().await.map_err(store_err)?.len();
        ("the recordings bucket (one pointer row each)", n)
    } else {
        ("not set up on this server", 0)
    };
    out.push(line("uploads waiting in the inbox", place, json!(waiting)));
    Ok(out)
}

/// `(b.get("confirm") or "").strip().lower()`: a falsy value is the empty phrase; a truthy one that is not text raises
/// the `AttributeError` the Python's `.strip()` raises, which the route answers as its 500.
fn confirm_phrase(b: &serde_json::Map<String, Value>) -> ApiResult<String> {
    match b.get("confirm") {
        Some(v) if py::truthy(Some(v)) => match v.as_str() {
            Some(s) => Ok(py::strip(s).to_lowercase()),
            None => Err(ApiError::internal(
                "AttributeError",
                format!("'{}' object has no attribute 'strip'", py::type_name(v)),
            )),
        },
        _ => Ok(String::new()),
    }
}

/// `DELETE /api/me`.
pub async fn delete_me(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    if confirm_phrase(&b)? != DELETE_CONFIRM {
        return Err(ApiError::bad_request(format!(
            "type \"{DELETE_CONFIRM}\" as {{\"confirm\": \"...\"}} to ask for deletion"
        )));
    }
    let would = inventory(&st).await?;
    if st.verifier().is_none() {
        return Ok(ok(&json!({
            "status": "dry_run", "deleted": false, "would_delete": would, "note": DRY_RUN_NOTE,
        })));
    }
    let by_sub = caller
        .0
        .as_ref()
        .and_then(|p| p.claims.get("sub"))
        .cloned()
        .unwrap_or(Value::Null);
    let owner = st.config().owner_id.clone();
    let would_logged = Value::Array(would.clone());
    detached({
        let st = st.clone();
        async move {
            let _cr = st.creative_lock().lock().await;
            Creative::new(st.clone())
                .append(
                    "account",
                    "request_deletion",
                    "deletion",
                    json!({"by_sub": by_sub, "would_delete": would_logged}),
                    &owner,
                )
                .await
                .map(|_| ())
        }
    })
    .await?;
    Ok(crate::error::json_response(
        StatusCode::ACCEPTED,
        &json!({"status": "requested", "deleted": false, "would_delete": would, "note": REQUESTED_NOTE}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    fn body(v: &Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn the_phrase_is_read_the_pythons_way() {
        assert_eq!(confirm_phrase(&body(&json!({}))).unwrap(), "");
        assert_eq!(confirm_phrase(&body(&json!({"confirm": null}))).unwrap(), "");
        assert_eq!(confirm_phrase(&body(&json!({"confirm": 0}))).unwrap(), "");
        assert_eq!(confirm_phrase(&body(&json!({"confirm": []}))).unwrap(), "");
        assert_eq!(
            confirm_phrase(&body(&json!({"confirm": "  Delete My MATERIAL\u{a0}\n"}))).unwrap(),
            DELETE_CONFIRM
        );
        for (v, kind) in [
            (json!(5), "int"),
            (json!(1.5), "float"),
            (json!(true), "bool"),
            (json!([1]), "list"),
        ] {
            let e = confirm_phrase(&body(&json!({ "confirm": v }))).unwrap_err();
            assert_eq!(e.status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(
                e.msg,
                format!("AttributeError: '{kind}' object has no attribute 'strip'")
            );
        }
        let e = confirm_phrase(&body(&json!({"confirm": {"a": 1}}))).unwrap_err();
        assert!(e.msg.contains("'dict'"));
    }
}
