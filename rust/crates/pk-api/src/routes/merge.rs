//! `GET /api/merge/history?joke=<id>` and `GET /api/merge/<mg|at|dm:id>` (T10 group 6b; `merge_api.handle`): what was
//! folded into a bit, and one merge record, each with gsync's record of its Google Docs side (`drive`, from the
//! `srcfile` row `kb/gsync/map.json`, `merge_api.drive_status`).
//!
//! `GET /api/merge/candidates` (`merge_api.handle`): every same-idea group with the counts; `?joke=<id>` one joke's
//! group, its closest other jokes, what it is attached to and its history; `?jokes=a,b,...` the inbox's one call
//! for several cards. The judge's answers are the `srcfile` row `kb/creative/merge_judge.json`, read by key like
//! every file; the groups are memoised on the generation ([`crate::snapshot::Data::merge_groups`]).

use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use axum::http::Uri;
use axum::response::Response;
use pk_creative::merge::{MergeState, candidates_batch, candidates_for, history};
use serde_json::{Map, Value, json};

use crate::error::{ApiError, ApiResult, json_bytes, ok};
use crate::query::Query;
use crate::state::AppState;
use crate::writes::{Creative, http, is_id};

/// `MAX_BATCH`: how many cards the inbox may ask about at once.
pub const MAX_BATCH: usize = 50;

/// The whole-groups answer: the memoised groups and the stamp the Python adds (`generated_at`).
#[derive(serde::Serialize)]
struct Stamped<'a> {
    #[serde(flatten)]
    groups: &'a Value,
    generated_at: String,
}

/// `GET /api/merge/candidates[?joke=<id>|?jokes=a,b]`.
pub async fn candidates(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    // the Python: `cr.corpus(kb)` (the 503 while booting, then `?backend=`) before the arguments; here the
    // arguments are judged before the groups are built, which costs nothing when they are good and spares a build
    // when they are not
    let d = super::prepare_rows(&st, &q).await?;
    let many: Vec<String> = q
        .all("jokes")
        .iter()
        .flat_map(|v| v.split(','))
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .collect();
    if !many.is_empty() && (many.len() > MAX_BATCH || !many.iter().all(|x| is_id(x))) {
        return Err(http(400, format!("jokes must be 1-{MAX_BATCH} joke ids")));
    }
    let joke = q.get("joke").filter(|j| !j.is_empty()).map(str::to_string);
    if many.is_empty()
        && let Some(j) = &joke
        && !is_id(j)
    {
        return Err(http(400, "bad joke id"));
    }
    let ix = d.merge_index(&st).await?;
    let cr = Creative::new(st.clone());
    let (lines, items) = cr.lines_and_items("merges").await?;
    let groups = d.merge_groups(&st, lines.len(), &items).await?;
    if !many.is_empty() {
        let state = MergeState::from_items(&items);
        let judge = Arc::clone(&groups.judge);
        let g = Arc::clone(&groups);
        let body = blocking(
            &st,
            move || json!({"items": candidates_batch(&ix, &many, &judge, &state, g.list())}),
        )
        .await?;
        return Ok(ok(&body));
    }
    if let Some(jid) = joke {
        let state = MergeState::from_items(&items);
        let judge = Arc::clone(&groups.judge);
        let g = Arc::clone(&groups);
        let items = Arc::clone(&items);
        let body = blocking(&st, move || {
            let mut out = candidates_for(&ix, &jid, &judge, &state, 5, g.list());
            out["attached_to"] = state.attached.get(&jid).cloned().unwrap_or(Value::Null);
            out["history"] = history(&items, &jid);
            out
        })
        .await?;
        return Ok(ok(&body));
    }
    // the whole answer, stamped: serialised from the memoised groups without a copy of them
    let bytes = serde_json::to_vec(&Stamped {
        groups: &groups.groups,
        generated_at: pk_core::time::now_iso(),
    })
    .map_err(|e| ApiError::internal("TypeError", e))?;
    Ok(json_bytes(StatusCode::OK, bytes.into()))
}

/// Pair scoring is CPU work: off the async threads, one build at a time across the service (the CPU permit).
async fn blocking<T: Send + 'static>(st: &AppState, f: impl FnOnce() -> T + Send + 'static) -> Result<T, ApiError> {
    let permit = st
        .cpu()
        .acquire_owned()
        .await
        .map_err(|e| ApiError::internal("AcquireError", e))?;
    tokio::task::spawn_blocking(move || {
        let out = f();
        drop(permit);
        out
    })
    .await
    .map_err(|e| ApiError::internal("JoinError", e))
}

/// gsync's keys of a merge's Google Docs record (`drive_status`).
const DRIVE_KEYS: [&str; 10] = [
    "state",
    "doc_id",
    "doc_url",
    "moved",
    "left",
    "skipped_legal",
    "archive_folder",
    "applied_at",
    "undone_at",
    "error",
];

/// The `merges` of gsync's `kb/gsync/map.json`, or none when the row is not there or is not JSON.
async fn drive_map(st: &AppState) -> ApiResult<Option<Map<String, Value>>> {
    const MAP: &str = "kb/gsync/map.json";
    let mut t = super::memos::texts(st, &[MAP.to_string()]).await?;
    Ok(t.remove(MAP)
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|m| m.get("merges").and_then(Value::as_object).cloned()))
}

/// `drive_status(cr, mid)`: the record's keys, or null when gsync has none for this merge.
fn drive_of(merges: Option<&Map<String, Value>>, mid: &str) -> Value {
    match merges.and_then(|m| m.get(mid)).filter(|d| pk_core::py::truthy(Some(d))) {
        Some(d) => Value::Object(
            DRIVE_KEYS
                .iter()
                .map(|k| ((*k).to_string(), d.get(*k).cloned().unwrap_or(Value::Null)))
                .collect(),
        ),
        None => Value::Null,
    }
}

/// `GET /api/merge/history?joke=<id>`.
pub async fn history_route(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    super::prepare_rows(&st, &q).await?;
    let jid = q
        .get("joke")
        .filter(|j| !j.is_empty() && is_id(j))
        .ok_or_else(|| http(400, "joke is required"))?
        .to_string();
    let (_, items) = Creative::new(st.clone()).lines_and_items("merges").await?;
    let mut out = history(&items, &jid);
    // gsync's row is read only for a bit that has merges (the Python reads its file always; nothing shows the difference)
    if let Some(merges) = out
        .get_mut("merges")
        .and_then(Value::as_array_mut)
        .filter(|m| !m.is_empty())
    {
        let drives = drive_map(&st).await?;
        for m in merges {
            let mid = m.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            m["drive"] = drive_of(drives.as_ref(), &mid);
        }
    }
    Ok(ok(&out))
}

/// `^/api/merge/((?:mg|at|dm):[A-Za-z0-9]+)$` over the path as sent (not decoded): the id, or none.
fn merge_id(path: &str) -> Option<&str> {
    let id = path.strip_prefix("/api/merge/")?;
    let (kind, rest) = id.split_once(':')?;
    (matches!(kind, "mg" | "at" | "dm") && !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then_some(id)
}

/// `GET /api/merge/<id>`: one merge record with its Google Docs side; anything else under `/api/merge/` is the
/// Python's "not found".
pub async fn one(State(st): State<AppState>, uri: Uri, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    super::prepare_rows(&st, &q).await?;
    let id = merge_id(uri.path()).ok_or_else(|| http(404, "not found"))?;
    let (_, items) = Creative::new(st.clone()).lines_and_items("merges").await?;
    let rec = items
        .get(id)
        .and_then(Value::as_object)
        .ok_or_else(|| http(404, "no such merge"))?;
    let drives = drive_map(&st).await?;
    let mut out = rec.clone();
    out.insert("drive".into(), drive_of(drives.as_ref(), id));
    Ok(ok(&Value::Object(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_merge_record_id_as_sent_is_one() {
        assert_eq!(merge_id("/api/merge/mg:ab12CD"), Some("mg:ab12CD"));
        assert_eq!(merge_id("/api/merge/at:x"), Some("at:x"));
        assert_eq!(merge_id("/api/merge/dm:0"), Some("dm:0"));
        for bad in [
            "/api/merge/mg:",
            "/api/merge/mg%3Aab",
            "/api/merge/xx:ab",
            "/api/merge/mg:a-b",
            "/api/merge/mg:ab/c",
            "/api/merge/",
            "/api/merge/attach",
        ] {
            assert_eq!(merge_id(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_drive_record_carries_the_ten_keys_or_null() {
        let m = json!({"mg:a": {"state": "applied", "doc_id": "d"}, "mg:b": {}})
            .as_object()
            .cloned();
        let d = drive_of(m.as_ref(), "mg:a");
        assert_eq!(d.as_object().map(Map::len), Some(10));
        assert_eq!(
            (d["state"].clone(), d["error"].clone()),
            (json!("applied"), Value::Null)
        );
        assert_eq!(drive_of(m.as_ref(), "mg:b"), Value::Null);
        assert_eq!(drive_of(m.as_ref(), "mg:none"), Value::Null);
        assert_eq!(drive_of(None, "mg:a"), Value::Null);
    }
}
