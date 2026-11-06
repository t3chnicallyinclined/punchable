//! The merge writes (`api-dev/merge_api.py`): `POST /api/merge` (the comic taps Merge: every unit and every matched
//! night of the other bits re-pointed to the main, the others hidden, one `merge` event with the decision indexes and
//! a verbatim snapshot of every original's words), `POST /api/merge/attach` (his call that a note belongs under a
//! bit: an `attach` event by him, no judge), `POST /api/merge/undo` (a merge undone line by line, newest first, or a
//! note detached) and `POST /api/merge/dismiss` ("not the same": never suggested together again). Every decision a
//! merge writes goes through the normal builder, so it is his action and can be undone; nothing is deleted.

use std::collections::BTreeSet;

use axum::Extension;
use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_core::text::head;
use pk_creative::merge::{MergeState, sha8_value};
use pk_kb::{Bucket, View, bucket};
use serde_json::{Map, Value, json};

use super::{Creative, Kb, Pre, Written, body_json, detached, is_id};
use crate::auth::Caller;
use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::state::AppState;

/// `MAX_OTHERS`.
pub const MAX_OTHERS: usize = 20;
/// `TAG`: the reason prefix on every decision line a merge writes.
pub const TAG: &str = "[merge]";

/// `brief(m)`: a merge record without its snapshot.
pub fn brief(m: &Value) -> Value {
    match m.as_object() {
        Some(o) => Value::Object(
            o.iter()
                .filter(|(k, _)| *k != "snapshot")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
        None => Value::Null,
    }
}

fn http(status: u16, msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST), msg)
}

fn title_of(v: &View, jid: &str) -> String {
    // `v.nodes[o].get('title')` in an f-string: `None` prints as such
    v.nodes
        .get(jid)
        .and_then(|n| n.get("title"))
        .map_or_else(|| "None".to_string(), py::str)
}

fn rejected(v: &View, eid: &str) -> bool {
    v.edges
        .get(eid)
        .is_some_and(|e| bucket(e.get("status")) == Bucket::Rejected)
}

/// `_snapshot(v, jid)`: everything of one joke a merge must keep, verbatim: its units (text, date, doc) and, when the
/// text is his own, its summary. Catalog summaries are AI-written and never copied as his words.
pub fn snapshot(v: &View, jid: &str) -> Value {
    let empty = Map::new();
    let n = v.nodes.get(jid).unwrap_or(&empty);
    let mut units: Vec<Value> = v
        .joke_units
        .get(jid)
        .into_iter()
        .flatten()
        .filter(|(_, eid)| !rejected(v, eid))
        .map(|(u, _)| {
            let un = v.nodes.get(u).unwrap_or(&empty);
            let g = |k: &str| un.get(k).cloned().unwrap_or(Value::Null);
            json!({"id": u, "date": g("date"), "doc_id": g("doc_id"), "doc_title": g("doc_title"), "kind": g("kind"),
                   "text": un.get("text").filter(|t| py::truthy(Some(t))).cloned().unwrap_or_else(|| json!(""))})
        })
        .collect();
    units.sort_by(|a, b| {
        let key = |x: &Value| {
            (
                py::str(x.get("date").filter(|d| !d.is_null()).unwrap_or(&json!(""))),
                py::str(&x["id"]),
            )
        };
        key(a).cmp(&key(b))
    });
    let his_summary = py::truthy(n.get("_by_tristan"))
        || v.decisions.iter().any(|d| {
            d.target() == Some("node")
                && d.id() == Some(jid)
                && d.0
                    .get("after")
                    .and_then(Value::as_object)
                    .is_some_and(|a| a.contains_key("summary"))
        });
    let mut perfs: Vec<Value> = v
        .adj
        .get(jid)
        .into_iter()
        .flatten()
        .filter_map(|eid| v.edges.get(eid).map(|e| (eid, e)))
        .filter(|(_, e)| {
            e.get("kind").and_then(Value::as_str) == Some("performed_as") && bucket(e.get("status")) != Bucket::Rejected
        })
        .map(|(eid, e)| {
            let p = if e.get("dst").and_then(Value::as_str) == Some(jid) {
                e.get("src")
            } else {
                e.get("dst")
            };
            let p = p.cloned().unwrap_or(Value::Null);
            let pn = p.as_str().and_then(|p| v.nodes.get(p)).unwrap_or(&empty);
            json!({"id": p, "set": pn.get("set").cloned().unwrap_or(Value::Null), "edge": eid})
        })
        .collect();
    perfs.sort_by_key(|x| py::str(&x["id"]));
    let g = |k: &str| n.get(k).cloned().unwrap_or(Value::Null);
    json!({"title": g("title"), "stage": g("stage"), "first_seen": g("first_seen"), "last_seen": g("last_seen"),
           "summary": if his_summary { g("summary") } else { Value::Null }, "summary_is_his": his_summary,
           "added_at": if py::truthy(n.get("_by_tristan")) { g("first_seen") } else { Value::Null },
           "units": units, "performances": perfs, "from": g("from")})
}

/// `plan(v, main, others, mid)`: the decision requests a merge writes, validated against the view by the builder.
pub fn plan(v: &View, main: &str, others: &[String], mid: &str) -> Vec<Value> {
    let main_units: BTreeSet<&str> = v
        .joke_units
        .get(main)
        .into_iter()
        .flatten()
        .filter(|(_, eid)| !rejected(v, eid))
        .map(|(u, _)| u.as_str())
        .collect();
    let end_of = |e: &Map<String, Value>, o: &str| {
        if e.get("dst").and_then(Value::as_str) == Some(o) {
            "dst"
        } else {
            "src"
        }
    };
    let mut reqs = Vec::new();
    for o in others {
        let why = format!("{TAG} {mid}: '{}' merged into '{}'", title_of(v, o), title_of(v, main));
        for (u, eid) in v.joke_units.get(o).into_iter().flatten() {
            let Some(e) = v.edges.get(eid) else { continue };
            if bucket(e.get("status")) == Bucket::Rejected || main_units.contains(u.as_str()) {
                continue;
            }
            reqs.push(
                json!({"target": "edge", "id": eid, "action": "edit", "after": {end_of(e, o): main}, "reason": why}),
            );
        }
        for eid in v.adj.get(o).into_iter().flatten() {
            let Some(e) = v.edges.get(eid) else { continue };
            if e.get("kind").and_then(Value::as_str) != Some("performed_as")
                || bucket(e.get("status")) == Bucket::Rejected
            {
                continue;
            }
            let end = end_of(e, o);
            let other_end = if end == "dst" { e.get("src") } else { e.get("dst") }
                .cloned()
                .unwrap_or(Value::Null);
            let pair = |a: &Value, b: &Value| BTreeSet::from([py::str(a), py::str(b)]);
            let want = pair(&other_end, &json!(main));
            let already = v.adj.get(main).into_iter().flatten().any(|x| {
                v.edges.get(x).is_some_and(|ex| {
                    ex.get("kind").and_then(Value::as_str) == Some("performed_as")
                        && pair(
                            ex.get("src").unwrap_or(&Value::Null),
                            ex.get("dst").unwrap_or(&Value::Null),
                        ) == want
                })
            });
            if already {
                continue; // already matched to the main: leave it, the hidden joke's copy drops out with it
            }
            reqs.push(json!({"target": "edge", "id": eid, "action": "edit", "after": {end: main}, "reason": why}));
        }
        reqs.push(json!({"target": "node", "id": o, "action": "edit", "after": {"hidden": true}, "reason": why}));
    }
    reqs
}

/// `typed_notes(v, others)`: `(joke id, text, date)` for each `other` that is a typed idea with no written version:
/// its words live only in `summary`, which a merge would otherwise hide with the joke. Only his own words.
pub fn typed_notes(v: &View, others: &[String]) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    for o in others {
        let snap = snapshot(v, o);
        if snap["units"].as_array().is_some_and(|u| !u.is_empty()) || snap["summary_is_his"] != true {
            continue;
        }
        let text = py::strip(snap["summary"].as_str().unwrap_or("")).to_string();
        if text.is_empty() {
            continue;
        }
        let date = head(snap["first_seen"].as_str().unwrap_or(""), 10);
        out.push((o.clone(), text, if date.is_empty() { Value::Null } else { json!(date) }));
    }
    out
}

fn reason_of(b: &Map<String, Value>, cap: usize) -> String {
    match b.get("reason") {
        Some(Value::String(s)) => head(py::strip(s), cap),
        _ => String::new(),
    }
}

struct Req {
    st: AppState,
    hidden: bool,
    b: Map<String, Value>,
}

async fn request(st: AppState, raw: Option<&str>, headers: &HeaderMap, body: Body) -> ApiResult<Req> {
    let q = Query::parse(raw);
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let b = body_json(headers, body).await?;
    Ok(Req {
        st,
        hidden: q.include_hidden(),
        b,
    })
}

/// `POST /api/merge/attach {note, into, reason?}`.
pub async fn attach(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, hidden, b } = request(st, raw.as_deref(), &headers, body).await?;
    let (note, into) = (super::str_of(&b, "note"), super::str_of(&b, "into"));
    let (Some(note), Some(into)) = (note.filter(|s| is_id(s)), into.filter(|s| is_id(s))) else {
        return Err(http(400, "note and into must be joke ids"));
    };
    if note == into {
        return Err(http(400, "a note can't be attached to itself"));
    }
    let (note, into) = (note.to_string(), into.to_string());
    let reason = reason_of(&b, 300);
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _kb = st.kb_lock().lock().await;
        let kb = Kb::new(st.clone(), hidden);
        let (_, v) = kb.view().await?;
        for j in [note.as_str(), into.as_str()] {
            if v.ntype(Some(j)) != Some("joke") {
                return Err(http(404, format!("unknown joke {j}")));
            }
        }
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let (lines, _) = cr.lines_and_items("merges").await?;
        let empty = Map::new();
        let n = v.nodes.get(&note).unwrap_or(&empty);
        let mut text = py::strip(n.get("summary").and_then(Value::as_str).unwrap_or("")).to_string();
        if text.is_empty() {
            let mut units: Vec<&(String, String)> = v.joke_units.get(&note).into_iter().flatten().collect();
            let date = |u: &str| {
                v.nodes
                    .get(u)
                    .and_then(|n| n.get("date"))
                    .filter(|d| py::truthy(Some(d)))
                    .map(py::str)
                    .unwrap_or_default()
            };
            units.sort_by_key(|a| std::cmp::Reverse(date(&a.0)));
            for (u, eid) in units {
                if !rejected(&v, eid)
                    && let Some(t) = v
                        .nodes
                        .get(u)
                        .and_then(|n| n.get("text"))
                        .and_then(Value::as_str)
                        .filter(|t| !t.is_empty())
                {
                    text = py::strip(t).to_string();
                    break;
                }
            }
        }
        let aid = format!("at:{}", sha8_value(&json!([note, into, pk_core::time::now_iso(), lines.len()])));
        let reasons = if reason.is_empty() { json!(["his call"]) } else { json!([reason]) };
        let already = |items: &pk_creative::Items| -> ApiResult<Pre> {
            if MergeState::from_items(items).attached.contains_key(&note) {
                return Err(http(409, "that note is already attached to a bit"));
            }
            Ok(Pre::Go)
        };
        let Written::Landed(rec) = cr
            .append_if(
                "merges",
                "attach",
                &aid,
                json!({"joke": note, "into": into, "text": head(&text, 2000), "date": head(&pk_core::time::now_iso(), 10),
                       "score": null, "reasons": reasons, "source": "owner"}),
                &owner,
                already,
            )
            .await?
        else {
            unreachable!("the check refuses or says go")
        };
        Ok(json!({"ok": true, "merge_id": aid, "merge": brief(&rec),
                  "undo": {"POST": "/api/merge/undo", "merge_id": aid}}))
    })
    .await?;
    Ok(ok(&out))
}

/// `POST /api/merge/dismiss {jokes[], reason?}`.
pub async fn dismiss(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, hidden: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let js: Option<Vec<&str>> = b.get("jokes").and_then(Value::as_array).and_then(|a| {
        (a.len() >= 2 && a.len() <= MAX_OTHERS + 1)
            .then(|| {
                a.iter()
                    .map(|x| x.as_str().filter(|s| is_id(s)))
                    .collect::<Option<Vec<_>>>()
            })
            .flatten()
    });
    let Some(js) = js else {
        return Err(http(400, "jokes must be a list of 2 or more joke ids"));
    };
    let js: Vec<String> = js
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(str::to_string)
        .collect();
    let did = format!("dm:{}", sha8_value(&json!(js)));
    let reason = reason_of(&b, 500);
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _kb = st.kb_lock().lock().await;
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let already = |items: &pk_creative::Items| -> ApiResult<Pre> {
            Ok(if items.contains_key(&did) {
                Pre::Done(StatusCode::OK, json!({"ok": true, "id": did, "already": true}))
            } else {
                Pre::Go
            })
        };
        match cr
            .append_if(
                "merges",
                "dismiss",
                &did,
                json!({"jokes": js, "reason": reason}),
                &owner,
                already,
            )
            .await?
        {
            Written::Landed(_) => Ok(json!({"ok": true, "id": did})),
            Written::Skipped(_, v) => Ok(v),
        }
    })
    .await?;
    Ok(ok(&out))
}

/// `POST /api/merge {main, others[], reason?, group?, reasons?}`.
pub async fn merge(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, hidden, b } = request(st, raw.as_deref(), &headers, body).await?;
    let Some(main) = super::str_of(&b, "main").filter(|s| is_id(s)) else {
        return Err(http(400, "main must be a joke id"));
    };
    let others: Option<Vec<&str>> = b.get("others").and_then(Value::as_array).and_then(|a| {
        (!a.is_empty() && a.len() <= MAX_OTHERS)
            .then(|| {
                a.iter()
                    .map(|x| x.as_str().filter(|s| is_id(s)))
                    .collect::<Option<Vec<_>>>()
            })
            .flatten()
    });
    let Some(others) = others else {
        return Err(http(400, format!("others must be a list of 1-{MAX_OTHERS} joke ids")));
    };
    // `list(dict.fromkeys(others))`: each once, first position kept
    let mut seen = BTreeSet::new();
    let others: Vec<String> = others
        .into_iter()
        .filter(|o| seen.insert(*o))
        .map(str::to_string)
        .collect();
    if others.iter().any(|o| o == main) {
        return Err(http(400, "the main joke can't also be one of the others"));
    }
    let main = main.to_string();
    let reason = reason_of(&b, 500);
    let group = b.get("group").cloned().unwrap_or(Value::Null);
    let reasons = b
        .get("reasons")
        .filter(|r| py::truthy(Some(r)))
        .cloned()
        .unwrap_or_else(|| json!([]));
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _kb = st.kb_lock().lock().await;
        let kb = Kb::new(st.clone(), hidden);
        let (_, mut v) = kb.view().await?;
        for j in std::iter::once(main.as_str()).chain(others.iter().map(String::as_str)) {
            if v.ntype(Some(j)) != Some("joke") {
                return Err(http(404, format!("unknown joke {j} (or it was already merged)")));
            }
        }
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let (lines, _) = cr.lines_and_items("merges").await?;
        let mut sorted_others = others.clone();
        sorted_others.sort();
        let mid = format!(
            "mg:{}",
            sha8_value(&json!([main, sorted_others, pk_core::time::now_iso(), lines.len()]))
        );
        let snap: Map<String, Value> = std::iter::once(main.as_str())
            .chain(others.iter().map(String::as_str))
            .map(|j| (j.to_string(), snapshot(&v, j)))
            .collect();
        let with_reason = |why: &str| if reason.is_empty() { why.to_string() } else { format!("{why} ({reason})") };
        let mut idx: Vec<usize> = Vec::new();
        let written: ApiResult<()> = async {
            // a typed idea with no version: its words become a version of the main FIRST (a unit + part_of), so the
            // merge keeps them where the bit page shows versions; then the plan hides the note like any other
            for (o, text, date) in typed_notes(&v, &others) {
                let why = format!(
                    "{TAG} {mid}: the idea typed as '{}' kept as a version of '{}'",
                    title_of(&v, &o),
                    title_of(&v, &main)
                );
                let full = with_reason(&why);
                let now = pk_core::time::now_iso();
                let unit = json!({"target": "node", "action": "add", "reason": full,
                                  "after": {"type": "unit", "text": text, "kind": "tag", "source": "idea", "date": date}});
                let (i, d) = kb.decide_at(&unit, &now, &full, &owner).await?;
                idx.push(i);
                let edge = json!({"target": "edge", "action": "add", "reason": full,
                                  "after": {"src": d.id(), "dst": main, "kind": "part_of"}});
                idx.push(kb.decide_at(&edge, &now, &full, &owner).await?.0);
                v = kb.view().await?.1;
            }
            // the plan is validated whole against one view before its first line is written (`kb._build` for
            // each request, then the appends); each append builds its line again from the view as it is then
            let now = pk_core::time::now_iso();
            let reqs = plan(&v, &main, &others, &mid);
            for req in &reqs {
                kb.build(&v, req, &now)?;
            }
            for req in reqs {
                let full = with_reason(req["reason"].as_str().unwrap_or(""));
                idx.push(kb.decide_at(&req, &now, &full, &owner).await?.0);
            }
            Ok(())
        }
        .await;
        if let Err(e) = written {
            // a half-written merge never stays half-written
            for i in idx.iter().rev() {
                if let Err(u) = kb.undo(Some(i64::try_from(*i).unwrap_or(i64::MAX)), &owner).await
                    && u.status.is_server_error()
                {
                    tracing::warn!("undoing a half-written merge: {u}");
                }
            }
            return Err(e);
        }
        let rec = cr
            .append(
                "merges",
                "merge",
                &mid,
                json!({"main": main, "others": others, "indexes": idx, "snapshot": snap, "group": group,
                       "reasons": reasons, "reason": reason}),
                &owner,
            )
            .await?;
        Ok(json!({"ok": true, "merge_id": mid, "merge": brief(&rec), "indexes": idx,
                  "drive": {"state": "queued",
                            "note": "gsync on your PC writes the merged Google Doc and moves the old docs into \
                                     'Merged (archive)' on its next pass"}}))
    })
    .await?;
    Ok(ok(&out))
}

/// `POST /api/merge/undo {merge_id}`: undo a merge (`mg:`) or detach an auto-attached note (`at:`).
pub async fn undo(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, hidden, b } = request(st, raw.as_deref(), &headers, body).await?;
    let Some(mid) = super::str_of(&b, "merge_id").filter(|s| is_id(s)) else {
        return Err(http(400, "merge_id is required"));
    };
    let mid = mid.to_string();
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _kb = st.kb_lock().lock().await;
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let (_, items) = cr.lines_and_items("merges").await?;
        let Some(m) = items.get(&mid).and_then(Value::as_object).filter(|m| !m.is_empty()) else {
            return Err(http(404, "no such merge"));
        };
        let status = m.get("status").map_or_else(|| "None".to_string(), py::str);
        if m.get("kind").and_then(Value::as_str) == Some("attach") {
            if status != "attached" {
                return Err(http(409, format!("that note is already {status}")));
            }
            let rec = cr.append("merges", "detach", &mid, json!({}), &owner).await?;
            return Ok(json!({"ok": true, "merge_id": mid, "merge": brief(&rec), "indexes": []}));
        }
        if m.get("kind").and_then(Value::as_str) != Some("merge") || status != "merged" {
            return Err(http(409, format!("that merge is {status}")));
        }
        let mut want: Vec<i64> = m
            .get("indexes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|i| {
                i.as_i64()
                    .ok_or_else(|| ApiError::internal("TypeError", "a merge index is not an integer"))
            })
            .collect::<ApiResult<_>>()?;
        want.sort_by_key(|i| std::cmp::Reverse(*i));
        let kb = Kb::new(st.clone(), hidden);
        let mut done = Vec::new();
        for i in want {
            match kb.undo(Some(i), &owner).await {
                Ok((idx, _)) => done.push(idx),
                // already undone by hand: fine, anything else is not
                Err(e) if e.status == StatusCode::CONFLICT => {}
                Err(e) => return Err(e),
            }
        }
        let rec = cr
            .append("merges", "undo", &mid, json!({"indexes": done}), &owner)
            .await?;
        Ok(
            json!({"ok": true, "merge_id": mid, "merge": brief(&rec), "indexes": done,
                  "drive": {"state": "undo_queued",
                            "note": "gsync moves the old docs back and releases the merged doc on its next pass"}}),
        )
    })
    .await?;
    Ok(ok(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View {
        let g = json!({"version": 1, "nodes": [
            {"id": "joke:a", "type": "joke", "title": "A", "stage": "working", "first_seen": "2024-01-01", "_by_tristan": true,
             "summary": "his own summary"},
            {"id": "joke:b", "type": "joke", "title": "B", "stage": "idea", "summary": "A typed idea with no version",
             "first_seen": "2024-02-02T10:00:00"},
            {"id": "joke:c", "type": "joke", "title": "C", "stage": "idea", "summary": "catalog words"},
            {"id": "unit:a1", "type": "unit", "text": "line one", "date": "2024-01-02"},
            {"id": "unit:c1", "type": "unit", "text": "c line", "date": "2024-03-03"},
            {"id": "unit:c2", "type": "unit", "text": "c rejected", "date": "2024-03-04"},
            {"id": "perf:p1", "type": "performance", "set": "set:s1"},
            {"id": "perf:p2", "type": "performance", "set": "set:s2"},
        ], "edges": [
            {"id": "e:pa1", "src": "unit:a1", "dst": "joke:a", "kind": "part_of", "status": "confirmed"},
            {"id": "e:pc1", "src": "unit:c1", "dst": "joke:c", "kind": "part_of", "status": "inferred"},
            {"id": "e:pc2", "src": "unit:c2", "dst": "joke:c", "kind": "part_of", "status": "rejected"},
            {"id": "e:perf1", "src": "perf:p1", "dst": "joke:c", "kind": "performed_as", "status": "proposed"},
            {"id": "e:perf2", "src": "perf:p2", "dst": "joke:c", "kind": "performed_as", "status": "proposed"},
            {"id": "e:perfa", "src": "perf:p2", "dst": "joke:a", "kind": "performed_as", "status": "confirmed"},
        ]});
        View::new(&g, Vec::new(), Vec::new(), "owner")
    }

    #[test]
    fn a_snapshot_keeps_his_words_and_the_live_matches() {
        let v = view();
        let s = snapshot(&v, "joke:c");
        assert_eq!(s["units"].as_array().unwrap().len(), 1, "the rejected unit is left out");
        assert_eq!(s["units"][0]["text"], "c line");
        assert_eq!(s["summary"], Value::Null, "a catalog summary is never copied");
        assert_eq!(s["summary_is_his"], false);
        assert_eq!(s["performances"].as_array().unwrap().len(), 2);
        assert_eq!(s["performances"][0]["id"], "perf:p1");
        let a = snapshot(&v, "joke:a");
        assert_eq!(a["summary"], "his own summary");
        assert_eq!(a["added_at"], "2024-01-01");
        let b = snapshot(&v, "joke:b");
        assert_eq!(b["summary_is_his"], false, "no `_by_tristan` and no summary decision");
    }

    #[test]
    fn the_plan_repoints_units_and_nights_then_hides_the_other() {
        let v = view();
        let reqs = plan(&v, "joke:a", &["joke:c".to_string()], "mg:x");
        let kinds: Vec<(String, String)> = reqs
            .iter()
            .map(|r| {
                (
                    r["target"].as_str().unwrap().to_string(),
                    r["id"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        // the live unit, the night not yet matched to the main (p2 is: left), then the hide
        assert_eq!(
            kinds,
            vec![
                ("edge".to_string(), "e:pc1".to_string()),
                ("edge".to_string(), "e:perf1".to_string()),
                ("node".to_string(), "joke:c".to_string())
            ]
        );
        assert_eq!(reqs[0]["after"], json!({"dst": "joke:a"}));
        assert_eq!(reqs[2]["after"], json!({"hidden": true}));
        assert_eq!(reqs[0]["reason"], "[merge] mg:x: 'C' merged into 'A'");
    }

    #[test]
    fn typed_notes_are_his_summaries_without_a_version() {
        let mut v = view();
        // make b's summary his: a decision that set it
        v.decisions.push(pk_kb::Decision(
            json!({"target": "node", "id": "joke:b", "action": "edit", "after": {"summary": "x"}})
                .as_object()
                .cloned()
                .unwrap(),
        ));
        let notes = typed_notes(&v, &["joke:b".to_string(), "joke:c".to_string(), "joke:a".to_string()]);
        assert_eq!(
            notes,
            vec![(
                "joke:b".to_string(),
                "A typed idea with no version".to_string(),
                json!("2024-02-02")
            )]
        );
        assert_eq!(
            brief(&json!({"id": "m", "snapshot": {"a": 1}, "status": "merged"})),
            json!({"id": "m", "status": "merged"})
        );
    }
}
