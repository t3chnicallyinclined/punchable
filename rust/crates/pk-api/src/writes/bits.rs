//! One bit's writes: `POST /api/jokes/<id>/versions` (`server.py` `add_version`: his new version of a bit, a unit
//! and its `part_of` edge, two lines undone together), `POST /api/decision` (any one decision: a title, a stage, a
//! retire, a confirm; Brainstorm's "Use as mine" teaches the voice profile) and `POST /api/undo`.

use std::sync::LazyLock;

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::HeaderMap;
use axum::response::Response;
use pk_core::py;
use pk_core::text::head;
use regex::Regex;
use serde_json::{Value, json};

use super::{Creative, Kb, actor, body_json, detached, is_id, refuse_legal};
use crate::auth::Caller;
use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::state::AppState;

/// `re.match(r"^[a-z_]{1,24}$", source)`.
static SOURCE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A[a-z_]{1,24}\n?\z").unwrap_or_else(|e| unreachable!("constant regex: {e}")));

/// `POST /api/jokes/<id>/versions {text, reason?, source?, draft?}`.
pub async fn versions(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(jid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let b = body_json(&headers, body).await?;
    let by = actor(&st, &caller, &b)?;
    let text = match b.get("text") {
        Some(Value::String(s)) => py::strip(s).to_string(),
        _ => String::new(),
    };
    if text.is_empty() || py::len(&text) > 4000 {
        return Err(ApiError::bad_request("text is required (up to 4000 characters)"));
    }
    refuse_legal(&st, [text.as_str()])?;
    let reason = match b.get("reason") {
        Some(Value::String(s)) => head(py::strip(s), 300),
        _ => String::new(),
    };
    let reason = if reason.is_empty() {
        "a new version written in the app".to_string()
    } else {
        reason
    };
    let source = match b.get("source") {
        Some(Value::String(s)) if SOURCE_RE.is_match(s) => s.clone(),
        _ => "app".to_string(),
    };
    let draft = py::truthy(b.get("draft"));
    let hidden = q.include_hidden();
    let out = detached(async move {
        let (i1, i2, uid) = {
            let _kb = st.kb_lock().lock().await;
            let kb = Kb::new(st.clone(), hidden);
            let (_, v) = kb.view().await?;
            if v.ntype(Some(&jid)) != Some("joke") {
                return Err(ApiError::not_found("no such joke"));
            }
            let date = head(&pk_core::time::now_iso(), 10);
            let (i1, d1) = kb
                .decide(
                    &json!({"target": "node", "action": "add", "reason": reason,
                            "after": {"type": "unit", "text": text, "kind": "tag", "source": source, "date": date}}),
                    &by,
                )
                .await?;
            let uid = d1.id().unwrap_or("").to_string();
            let edge = json!({"target": "edge", "action": "add", "reason": reason,
                              "after": {"src": uid, "dst": jid, "kind": "part_of"}});
            let (i2, _) = match kb.decide(&edge, &by).await {
                Ok(x) => x,
                Err(e) => {
                    // never a line that belongs to nothing
                    if let Err(u) = kb.undo(Some(i64::try_from(i1).unwrap_or(i64::MAX)), &by).await {
                        tracing::warn!("the unit's line could not be undone after its edge failed: {u}");
                    }
                    return Err(e);
                }
            };
            (i1, i2, uid)
        };
        let mut out = json!({"ok": true, "unit": uid, "joke": jid, "indexes": [i1, i2]});
        if by == st.config().owner_id && draft {
            let _cr = st.creative_lock().lock().await;
            let cr = Creative::new(st.clone());
            out["learned"] = super::learned::learn_kept(&cr, &text, Some(&jid), &by, true)
                .await
                .unwrap_or(Value::Null);
        }
        Ok(out)
    })
    .await?;
    Ok(ok(&out))
}

/// `POST /api/decision`: one decision as the app posts it (`{target, action, id?, after?, reason?}`).
pub async fn decision(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let b = body_json(&headers, body).await?;
    if let Some(id) = b.get("id").filter(|v| !v.is_null())
        && !id.as_str().is_some_and(is_id)
    {
        return Err(ApiError::bad_request("bad id"));
    }
    let by = actor(&st, &caller, &b)?;
    let hidden = q.include_hidden();
    let out = detached(async move {
        let (idx, line) = {
            let _kb = st.kb_lock().lock().await;
            Kb::new(st.clone(), hidden)
                .decide(&Value::Object(b.clone()), &by)
                .await?
        };
        let mut out = json!({"ok": true, "indexes": [idx], "decisions": [Value::Object(line.0.clone())]});
        let after = b.get("after").and_then(Value::as_object);
        let s = |k: &str| after.and_then(|a| a.get(k)).and_then(Value::as_str);
        if by == st.config().owner_id
            && super::str_of(&b, "target") == Some("node")
            && super::str_of(&b, "action") == Some("add")
            && s("type") == Some("unit")
            && s("source") == Some("draft")
            && let Some(text) = s("text")
        {
            // Brainstorm's "Use as mine": a punch-up draft kept as his version of the bit (the variant_of edge
            // follows in the app's next call; the unit's own id is the bit reference until then)
            let bit = s("id")
                .filter(|i| !i.is_empty())
                .map(str::to_string)
                .or_else(|| line.id().map(str::to_string));
            let _cr = st.creative_lock().lock().await;
            let cr = Creative::new(st.clone());
            out["learned"] = super::learned::learn_kept(&cr, text, bit.as_deref(), &by, false)
                .await
                .unwrap_or(Value::Null);
        }
        Ok(out)
    })
    .await?;
    Ok(ok(&out))
}

/// `POST /api/undo {indexes: [...]}` (or `{index}`): each undone newest first, `None` last.
pub async fn undo(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let b = body_json(&headers, body).await?;
    let idxs: Vec<Value> = match b.get("indexes") {
        None | Some(Value::Null) => vec![b.get("index").cloned().unwrap_or(Value::Null)],
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err(ApiError::bad_request("indexes must be a list of integers")),
    };
    let mut want: Vec<Option<i64>> = Vec::with_capacity(idxs.len());
    for i in &idxs {
        want.push(match i {
            Value::Null => None,
            // `isinstance(True, int)` holds in Python: a bool is its integer
            Value::Bool(x) => Some(i64::from(*x)),
            Value::Number(n) => Some(
                n.as_i64()
                    .ok_or_else(|| ApiError::bad_request("indexes must be a list of integers"))?,
            ),
            _ => return Err(ApiError::bad_request("indexes must be a list of integers")),
        });
    }
    let by = actor(&st, &caller, &b)?;
    // `sorted(idxs, key=lambda x: -1 if x is None else x, reverse=True)`: a stable sort, newest first, `None` last
    want.sort_by_key(|i| std::cmp::Reverse(i.unwrap_or(-1)));
    let hidden = q.include_hidden();
    let out = detached(async move {
        let mut indexes = Vec::new();
        let mut decisions = Vec::new();
        let _kb = st.kb_lock().lock().await;
        let kb = Kb::new(st.clone(), hidden);
        for i in want {
            let (idx, line) = kb.undo(i, &by).await?;
            indexes.push(idx);
            decisions.push(Value::Object(line.0));
        }
        Ok(json!({"ok": true, "indexes": indexes, "decisions": decisions}))
    })
    .await?;
    Ok(ok(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_is_a_short_lowercase_word() {
        assert!(SOURCE_RE.is_match("draft"));
        assert!(SOURCE_RE.is_match("app\n"));
        assert!(!SOURCE_RE.is_match("Draft"));
        assert!(!SOURCE_RE.is_match(""));
        assert!(!SOURCE_RE.is_match(&"a".repeat(25)));
    }

    #[test]
    fn undo_order_is_newest_first_with_none_last_and_stable() {
        let mut want = vec![Some(2), None, Some(5), Some(2), None];
        want.sort_by_key(|i| std::cmp::Reverse(i.unwrap_or(-1)));
        assert_eq!(want, vec![Some(5), Some(2), Some(2), None, None]);
    }
}
