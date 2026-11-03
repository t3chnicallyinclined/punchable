//! The Set page's and the Tonight list's writes (`server.py` `rate`, `set_row`, the reject-match route and
//! `save_tonight`): the lines the app writes as they are (`AppKB.append_lines`: the route's own `before` and
//! `after`, no builder), each stamped and appended through [`Kb::append_lines`], then the page's answer built from
//! the index after the write, as the Python builds it from `kb.app()` again.

use std::sync::Arc;

use axum::Extension;
use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::HeaderMap;
use axum::response::Response;
use pk_core::py;
use pk_kb::{Bucket, Rec, View, bucket, public};
use serde_json::{Map, Value, json};

use super::{Kb, Raw, body_json, detached, kb_err};
use crate::auth::Caller;
use crate::error::{ApiError, ApiResult, ok};
use crate::index::{NOT_COUNTED, RATINGS};
use crate::pages::Ctx;
use crate::query::Query;
use crate::snapshot::Data;
use crate::state::AppState;
use crate::today::TONIGHT_ID;

/// `suggest_rating(max_bars)`.
pub fn suggest_rating(max_bars: i64) -> &'static str {
    match max_bars {
        n if n >= 3 => "killed",
        2 => "worked",
        1 => "meh",
        _ => "died",
    }
}

/// `AppIndex.perf_edge[pid]`: the performance's `performed_as` edge, a live one before a rejected one, in edge
/// order (the index's rule, over the view this write validates against).
pub fn perf_edge<'a>(v: &'a View, pid: &str) -> Option<(&'a str, &'a Rec)> {
    let mut best: Option<(&str, &Rec)> = None;
    for (eid, e) in &v.edges {
        if e.get("kind").and_then(Value::as_str) != Some("performed_as") {
            continue;
        }
        let (mut p, mut j) = (sget(e, "src"), sget(e, "dst"));
        if v.ntype(p) != Some("performance") {
            std::mem::swap(&mut p, &mut j);
        }
        if p != Some(pid) || v.ntype(p) != Some("performance") || v.ntype(j) != Some("joke") {
            continue;
        }
        let replace = best.is_none_or(|(_, cur)| {
            bucket(cur.get("status")) == Bucket::Rejected && bucket(e.get("status")) != Bucket::Rejected
        });
        if replace {
            best = Some((eid.as_str(), e));
        }
    }
    best
}

fn sget<'a>(m: &'a Rec, k: &str) -> Option<&'a str> {
    m.get(k).and_then(Value::as_str)
}

/// `(b.get("reason") or "")`: a string, else nothing (a truthy non-string is the Python's 500 on `.strip()`).
fn reason_of(b: &Map<String, Value>) -> String {
    b.get("reason").and_then(Value::as_str).unwrap_or("").to_string()
}

fn performance<'a>(v: &'a View, pid: Option<&str>) -> ApiResult<&'a Rec> {
    pid.and_then(|p| v.nodes.get(p))
        .filter(|p| sget(p, "type") == Some("performance"))
        .ok_or_else(|| ApiError::new(axum::http::StatusCode::NOT_FOUND, "unknown performance"))
}

fn conflict(msg: &str) -> ApiError {
    ApiError::new(axum::http::StatusCode::CONFLICT, msg)
}

/// `{"ok": true, "indexes": [...], "decisions": [...]}`.
fn written(out: &[(usize, pk_kb::Decision)]) -> Value {
    json!({"ok": true, "indexes": out.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
           "decisions": out.iter().map(|(_, d)| Value::Object(d.0.clone())).collect::<Vec<_>>()})
}

/// The index after a write (`a2 = kb.app()`): over data proven to hold the lines just written (`last` is the
/// newest index), never the last data served while the database is unreachable.
async fn index_after(st: &AppState, hidden: bool, last: usize) -> ApiResult<(Arc<Data>, Arc<crate::index::AppIndex>)> {
    let d = st.data_for_write().await?;
    if d.decisions().len() <= last {
        return Err(ApiError::unavailable(
            "the data could not be refreshed after this write; try again in a moment",
        ));
    }
    let idx = d.index(hidden).await?;
    Ok((d, idx))
}

fn newest(lines: &[(usize, pk_kb::Decision)]) -> usize {
    lines.iter().map(|(i, _)| *i).max().unwrap_or(0)
}

/// `Handler.rate(b)`: the lines of one rating (the AI's match confirmed first unless `confirm_match` is false).
fn rate_lines(d: &Data, v: &View, b: &Map<String, Value>, rating: &str, confirm_match: bool) -> ApiResult<Vec<Raw>> {
    let pid = b.get("perf").and_then(Value::as_str);
    let p = performance(v, pid)?;
    let pid = pid.unwrap_or("");
    let mut lines = Vec::new();
    if let Some((eid, e)) = perf_edge(v, pid)
        && bucket(e.get("status")) == Bucket::Proposed
        && confirm_match
    {
        lines.push(Raw {
            target: "edge",
            id: eid.to_string(),
            action: "confirm",
            before: Value::Object(public(e)),
            after: json!({"status": "confirmed"}),
            reason: "review: this is the joke".into(),
        });
    }
    let laughs = crate::tape::laughs_for(&d.tape.laughs, sget(p, "set"), p.get("start_ms"), p.get("end_ms"));
    let max_bars = laughs.get("max_bars").and_then(Value::as_i64).unwrap_or(0);
    let after = json!({"rating": rating, "rating_auto": suggest_rating(max_bars), "max_bars": max_bars});
    let before = public(p);
    let same = after
        .as_object()
        .is_some_and(|a| a.iter().all(|(k, x)| before.get(k).unwrap_or(&Value::Null) == x));
    if same {
        return Err(conflict("already rated that way"));
    }
    lines.push(Raw {
        target: "node",
        id: pid.to_string(),
        action: "edit",
        before: Value::Object(before),
        after,
        reason: reason_of(b),
    });
    Ok(lines)
}

/// The rating written and the page's answer (`rate` after `append_lines`).
async fn rate_result(st: &AppState, hidden: bool, b: &Map<String, Value>, confirm_match: bool) -> ApiResult<Value> {
    let rating = b.get("rating").and_then(Value::as_str);
    let Some(rating) = rating.filter(|r| RATINGS.contains(r)) else {
        return Err(ApiError::bad_request(format!(
            "rating must be one of {}",
            py::repr(&json!(RATINGS))
        )));
    };
    let pid = b.get("perf").and_then(Value::as_str).unwrap_or("").to_string();
    let owner = st.config().owner_id.clone();
    let lines = {
        let _kb = st.kb_lock().lock().await;
        Kb::new(st.clone(), hidden)
            .append_lines(&owner, |d, v| rate_lines(d, v, b, rating, confirm_match))
            .await?
    };
    // the page's bit after the write (`a2 = kb.app()`), with the process's other writes free to go on
    let mut out = written(&lines);
    let (d2, idx) = index_after(st, hidden, newest(&lines)).await?;
    let tx = crate::tape::Transcripts::new();
    let c = Ctx::new(&idx, &d2, &tx);
    if let Some(jid) = c.joke_of(&pid) {
        let mut j = c.brief_joke(jid);
        if let (Value::Object(j), Value::Object(rooms)) = (&mut j, c.joke_rooms(jid)) {
            j.extend(rooms);
        }
        out["joke"] = j;
    }
    Ok(out)
}

/// `POST /api/review/rate {perf, rating, confirm_match?, reason?}`.
pub async fn rate(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let b = body_json(&headers, body).await?;
    let hidden = q.include_hidden();
    let confirm = b.get("confirm_match") != Some(&Value::Bool(false));
    let out = detached(async move { rate_result(&st, hidden, &b, confirm).await }).await?;
    Ok(ok(&out))
}

/// `POST /api/set-row {perf, op, joke?, rating?, reason?}`: in-place edits on the Set page.
pub async fn set_row(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let b = body_json(&headers, body).await?;
    let hidden = q.include_hidden();
    let out = detached(async move { set_row_result(&st, hidden, &b).await }).await?;
    Ok(ok(&out))
}

async fn set_row_result(st: &AppState, hidden: bool, b: &Map<String, Value>) -> ApiResult<Value> {
    let op = b.get("op").and_then(Value::as_str).unwrap_or("");
    if op == "rate" {
        return rate_result(st, hidden, b, false).await;
    }
    let pid = b.get("perf").and_then(Value::as_str);
    let owner = st.config().owner_id.clone();
    let _kb = st.kb_lock().lock().await;
    let kb = Kb::new(st.clone(), hidden);
    let reason = py::strip(&reason_of(b)).to_string();
    match op {
        "confirm" | "match" => {
            // one decision through the builder, from the view as it is (`kb.decide`)
            let (_, v) = kb.view().await?;
            performance(&v, pid)?;
            let pid = pid.unwrap_or("");
            let e = perf_edge(&v, pid);
            let live = e.is_some_and(|(_, e)| bucket(e.get("status")) != Bucket::Rejected);
            let req = if op == "confirm" {
                let Some((eid, _)) = e.filter(|_| live) else {
                    return Err(conflict("nothing matched to confirm"));
                };
                json!({"target": "edge", "id": eid, "action": "confirm", "reason": reason})
            } else {
                let jid = b.get("joke").and_then(Value::as_str);
                if v.ntype(jid) != Some("joke") {
                    return Err(ApiError::not_found("pick one of your jokes"));
                }
                let jid = jid.unwrap_or("");
                match e {
                    Some((eid, e)) => {
                        let end = if v.ntype(sget(e, "dst")) == Some("joke") {
                            "dst"
                        } else {
                            "src"
                        };
                        if sget(e, end) == Some(jid) {
                            if live && bucket(e.get("status")) == Bucket::Confirmed {
                                return Err(conflict("already matched to that joke"));
                            }
                            json!({"target": "edge", "id": eid, "action": "confirm", "reason": reason})
                        } else {
                            json!({"target": "edge", "id": eid, "action": "edit", "after": {end: jid}, "reason": reason})
                        }
                    }
                    None => json!({"target": "edge", "action": "add",
                                   "after": {"src": pid, "dst": jid, "kind": "performed_as"},
                                   "reason": if reason.is_empty() { "matched on the set page".to_string() } else { reason }}),
                }
            };
            let (idx, written_line) = kb.decide(&req, &owner).await?;
            Ok(written(&[(idx, written_line)]))
        }
        "crowd" | "bit" | "banter" => {
            let kind = match op {
                "crowd" => "crowd_work",
                "bit" => "bit",
                _ => "banter",
            };
            let lines = kb
                .append_lines(&owner, |_, v| {
                    let p = performance(v, pid)?;
                    if sget(p, "kind") == Some(kind) {
                        return Err(conflict("already marked that way"));
                    }
                    Ok(vec![Raw {
                        target: "node",
                        id: pid.unwrap_or("").to_string(),
                        action: "edit",
                        before: Value::Object(public(p)),
                        after: json!({"kind": kind}),
                        reason: reason.clone(),
                    }])
                })
                .await?;
            Ok(written(&lines))
        }
        "new" => {
            let lines = kb
                .append_lines(&owner, |_, v| {
                    let p = performance(v, pid)?;
                    let pid = pid.unwrap_or("");
                    let e = perf_edge(v, pid);
                    let live = e.is_some_and(|(_, e)| bucket(e.get("status")) != Bucket::Rejected);
                    let mut lines = Vec::new();
                    if sget(p, "kind").is_some_and(|k| NOT_COUNTED.contains(&k)) {
                        lines.push(Raw {
                            target: "node",
                            id: pid.to_string(),
                            action: "edit",
                            before: Value::Object(public(p)),
                            after: json!({"kind": "bit"}),
                            reason: reason.clone(),
                        });
                    }
                    if let Some((eid, e)) = e.filter(|_| live) {
                        lines.push(Raw {
                            target: "edge",
                            id: eid.to_string(),
                            action: "reject",
                            before: Value::Object(public(e)),
                            after: json!({"status": "rejected"}),
                            reason: if reason.is_empty() {
                                "new material, not this joke".to_string()
                            } else {
                                reason.clone()
                            },
                        });
                    }
                    if lines.is_empty() {
                        return Err(conflict("already new material"));
                    }
                    Ok(lines)
                })
                .await?;
            Ok(written(&lines))
        }
        _ => {
            // the performance is looked up before the op is judged
            let (_, v) = kb.view().await?;
            performance(&v, pid)?;
            Err(ApiError::bad_request(
                "op must be rate|match|new|crowd|bit|banter|confirm",
            ))
        }
    }
}

/// `POST /api/review/reject-match {perf, reason?}`: the AI's match rejected.
pub async fn reject_match(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let b = body_json(&headers, body).await?;
    let hidden = q.include_hidden();
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _kb = st.kb_lock().lock().await;
        let kb = Kb::new(st.clone(), hidden);
        let (_, v) = kb.view().await?;
        let pid = b.get("perf").and_then(Value::as_str).unwrap_or("");
        let Some((eid, _)) = perf_edge(&v, pid) else {
            return Err(ApiError::not_found("no match to reject for that performance"));
        };
        let req = json!({"target": "edge", "id": eid, "action": "reject", "reason": reason_of(&b)});
        let (idx, line) = kb.decide(&req, &owner).await?;
        Ok(written(&[(idx, line)]))
    })
    .await?;
    Ok(ok(&out))
}

/// `POST /api/tonight {items: [{joke, done?}], reason?}`: his list for tonight, saved as one node.
pub async fn tonight(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let b = body_json(&headers, body).await?;
    let items = b
        .get("items")
        .and_then(Value::as_array)
        .filter(|a| a.len() <= 60)
        .cloned()
        .ok_or_else(|| ApiError::bad_request("items must be a list (up to 60)"))?;
    let hidden = q.include_hidden();
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let reason = reason_of(&b);
        let lines = {
            let _kb = st.kb_lock().lock().await;
            Kb::new(st.clone(), hidden)
                .append_lines(&owner, |_, v| {
                    let mut clean = Vec::with_capacity(items.len());
                    for it in &items {
                        let joke = it.as_object().and_then(|o| o.get("joke")).and_then(Value::as_str);
                        if v.ntype(joke) != Some("joke") {
                            let msg: String = format!("unknown joke in list: {}", py::repr(it))
                                .chars()
                                .take(200)
                                .collect();
                            return Err(kb_err(pk_kb::KbError::bad(msg)));
                        }
                        clean.push(json!({"joke": joke, "done": py::truthy(it.get("done"))}));
                    }
                    let clean = Value::Array(clean);
                    Ok(vec![match v.nodes.get(TONIGHT_ID) {
                        None => Raw {
                            target: "node",
                            id: TONIGHT_ID.into(),
                            action: "add",
                            before: Value::Null,
                            after: json!({"type": "list", "name": "Tonight", "items": clean}),
                            reason: reason.clone(),
                        },
                        Some(node) => {
                            if node.get("items") == Some(&clean) {
                                return Err(conflict("nothing changed"));
                            }
                            Raw {
                                target: "node",
                                id: TONIGHT_ID.into(),
                                action: "edit",
                                before: Value::Object(public(node)),
                                after: json!({"items": clean}),
                                reason: reason.clone(),
                            }
                        }
                    }])
                })
                .await?
        };
        let (d2, idx) = index_after(&st, hidden, newest(&lines)).await?;
        let tx = crate::tape::Transcripts::new();
        let c = Ctx::new(&idx, &d2, &tx);
        Ok(
            json!({"ok": true, "indexes": lines.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
                  "tonight": crate::today::tonight(&c)}),
        )
    })
    .await?;
    Ok(ok(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_suggested_rating_follows_the_bars() {
        assert_eq!(suggest_rating(5), "killed");
        assert_eq!(suggest_rating(3), "killed");
        assert_eq!(suggest_rating(2), "worked");
        assert_eq!(suggest_rating(1), "meh");
        assert_eq!(suggest_rating(0), "died");
        assert_eq!(suggest_rating(-1), "died");
    }

    #[test]
    fn the_ratings_refusal_prints_the_list_as_python_does() {
        assert_eq!(py::repr(&json!(RATINGS)), "['killed', 'worked', 'meh', 'died']");
    }

    #[test]
    fn a_live_edge_wins_over_a_rejected_one() {
        let g = json!({"version": 1, "nodes": [
            {"id": "perf:p", "type": "performance", "set": "set:s"},
            {"id": "joke:a", "type": "joke", "title": "A"}, {"id": "joke:b", "type": "joke", "title": "B"}],
            "edges": [
            {"id": "e:1", "src": "perf:p", "dst": "joke:a", "kind": "performed_as", "status": "rejected"},
            {"id": "e:2", "src": "joke:b", "dst": "perf:p", "kind": "performed_as", "status": "proposed"},
            {"id": "e:3", "src": "perf:p", "dst": "joke:a", "kind": "performed_as", "status": "confirmed"}]});
        let v = View::new(&g, Vec::new(), Vec::new(), "owner");
        assert_eq!(
            perf_edge(&v, "perf:p").map(|(e, _)| e),
            Some("e:2"),
            "the first live edge, either way round"
        );
        assert_eq!(perf_edge(&v, "perf:x"), None);
    }
}
