//! "Not connected", remembered (`docs/design/SPEC-2026-11-16-bits-sets-tapes.md` §3.1; Rust only, no Python twin):
//!
//! - `POST /api/links/dismiss {a, b, group, reason?}`: he says two things do not belong together (a bit and another
//!   bit, a taped moment, a line or a night's wording). `{ok, id}`; the same pair again answers `already: true` and
//!   writes nothing. `GET /api/jokes/<id>/around` then leaves the pair out of every computed suggestion.
//! - `POST /api/links/undismiss {id}`: his Undo. `{ok, id}`.
//!
//! A tie that is a stored edge is not dismissed here: it is rejected with `POST /api/decision`, as before. The store
//! is `links` (`pk_creative::reducers::links`): a new store, so no other process replays it (spec B5).

use axum::Extension;
use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_creative::reducers::links::{LINK_GROUPS, MAX_REASON_CHARS, is_dismissed, is_link_ref, pair_id};
use pk_kb::View;
use serde_json::{Map, Value, json};

use super::{Creative, Kb, Pre, Req, Written, actor, detached, http, is_id, refuse_legal, request};
use crate::auth::Caller;
use crate::error::{ApiResult, ok};
use crate::state::AppState;

/// The store.
pub const STORE: &str = "links";
/// Pairs dismissed at once, at most: a guard against a runaway client, far above what one comic says no to. The cap is
/// the route's, never the reducer's (a lower cap there later would refuse the history already written).
pub const MAX_LINKS: usize = 20_000;

/// Whether one end of a pair is there: a bit, a taped moment or a line this reader's view holds; a night (`set:`) is
/// named by its key and not looked up (a phone-taped night may not be a node yet).
fn known(v: &View, r: &str) -> bool {
    match r.split_once(':').map(|(kind, _)| kind) {
        Some("joke") => v.ntype(Some(r)) == Some("joke"),
        Some("perf") => v.ntype(Some(r)) == Some("performance"),
        Some("unit") => v.ntype(Some(r)) == Some("unit"),
        Some("set") => true,
        _ => false,
    }
}

/// `POST /api/links/dismiss {a, b, group, reason?}`.
pub async fn dismiss(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q, b } = request(st, raw.as_deref(), &headers, body).await?;
    let hidden = q.include_hidden();
    let end = |k: &str| super::str_of(&b, k).filter(|r| is_id(r) && is_link_ref(r));
    let (Some(a), Some(z)) = (end("a"), end("b")) else {
        return Err(http(
            400,
            "a and b must each be a bit, a taped moment, a line or a taped night",
        ));
    };
    if a == z {
        return Err(http(400, "a pair needs two different ends"));
    }
    if !a.starts_with("joke:") && !z.starts_with("joke:") {
        return Err(http(400, "one end of the pair must be a bit"));
    }
    let group = super::str_of(&b, "group").unwrap_or("");
    if !LINK_GROUPS.contains(&group) {
        return Err(http(400, format!("group must be one of {}", LINK_GROUPS.join(", "))));
    }
    let reason = py::strip(super::str_of(&b, "reason").unwrap_or("")).to_string();
    if py::len(&reason) > MAX_REASON_CHARS {
        return Err(http(400, format!("a reason is at most {MAX_REASON_CHARS} characters")));
    }
    refuse_legal(&st, [reason.as_str()])?;
    let by = actor(&st, &caller, &Map::new())?;
    let (a, z, group) = (a.to_string(), z.to_string(), group.to_string());
    let out = detached(async move {
        // what a pair names must be there now (the `Kb` contract: the view is read under the lock)
        let (_, view) = {
            let _kb = st.kb_lock().lock().await;
            Kb::new(st.clone(), hidden).view().await?
        };
        for r in [&a, &z] {
            if !known(&view, r) {
                return Err(http(404, format!("unknown {r}")));
            }
        }
        let id = pair_id(&a, &z);
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let pid = id.clone();
        let have = move |items: &pk_creative::Items| -> ApiResult<Pre> {
            if items.get(&pid).is_some_and(is_dismissed) {
                return Ok(Pre::Done(
                    StatusCode::OK,
                    json!({"ok": true, "id": pid, "already": true}),
                ));
            }
            if items.values().filter(|x| is_dismissed(x)).count() >= MAX_LINKS {
                return Err(http(400, format!("at most {MAX_LINKS} dismissed pairs")));
            }
            Ok(Pre::Go)
        };
        let data = json!({"a": a, "b": z, "group": group, "reason": reason});
        Ok(match cr.append_if(STORE, "dismiss", &id, data, &by, have).await? {
            Written::Landed(_) => json!({"ok": true, "id": id}),
            Written::Skipped(_, v) => v,
        })
    })
    .await?;
    Ok(ok(&out))
}

/// `POST /api/links/undismiss {id}`.
pub async fn undismiss(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let Some(id) = super::str_of(&b, "id").filter(|i| i.starts_with("lk:") && is_id(i)) else {
        return Err(http(400, "id is required (a dismissed pair)"));
    };
    let id = id.to_string();
    let by = actor(&st, &caller, &Map::new())?;
    let out = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let pid = id.clone();
        let have = move |items: &pk_creative::Items| -> ApiResult<Pre> {
            match items.get(&pid) {
                None => Err(http(404, "no such pair")),
                Some(x) if is_dismissed(x) => Ok(Pre::Go),
                Some(_) => Ok(Pre::Done(
                    StatusCode::OK,
                    json!({"ok": true, "id": pid, "already": true}),
                )),
            }
        };
        Ok(
            match cr
                .append_if(STORE, "undismiss", &id, Value::Object(Map::new()), &by, have)
                .await?
            {
                Written::Landed(_) => json!({"ok": true, "id": id}),
                Written::Skipped(_, v) => v,
            },
        )
    })
    .await?;
    Ok(ok(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_end_is_known_by_its_kind() {
        let g = json!({"version": 1, "nodes": [
            {"id": "joke:a", "type": "joke", "title": "A"},
            {"id": "unit:u", "type": "unit", "text": "a line"},
            {"id": "perf:p", "type": "performance", "set": "set:s"},
        ], "edges": []});
        let v = View::new(&g, Vec::new(), Vec::new(), "owner");
        for r in ["joke:a", "unit:u", "perf:p", "set:20260313_late"] {
            assert!(known(&v, r), "{r}");
        }
        // an id of the right shape that is another kind of thing, or nothing
        for r in ["joke:u", "perf:a", "unit:nope", "joke:nope", "label:x"] {
            assert!(!known(&v, r), "{r}");
        }
    }
}
