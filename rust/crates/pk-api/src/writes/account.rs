//! Settings, the voice profile and the "not a joke?" answers (T10 group 4): `PATCH /api/settings`
//! (`creative_api.settings_patch`: his four settings, one `settings` event when anything changed), `PUT
//! /api/profile/voice` (`profile_api.put`: the whole text, legal-checked before anything is written), `POST
//! /api/profile/voice/learn` (`learn_dropped`: the drafts he dismissed, one `learn` event), `POST
//! /api/suggestions/not-a-joke/keep` ("it is a joke": a `keep` event, never suggested again) and
//! `/api/suggestions/not-a-joke/hide` (his one tap: the idea hidden by a decision line, undoable).

use std::sync::LazyLock;

use axum::Extension;
use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_creative::reducers::learned::counts;
use regex::Regex;
use serde_json::{Map, Value, json};

use super::learned::VOICE_ID;
use super::{Creative, Kb, Pre, Req, Written, actor, detached, http, refuse_legal, request};
use crate::auth::Caller;
use crate::error::{ApiResult, ok};
use crate::state::AppState;

/// `profile_api.MAX_CHARS`.
pub const MAX_CHARS: usize = 60_000;
/// `profile_api.MAX_DISMISS`.
pub const MAX_DISMISS: usize = 10;
/// `profile_api.LINE_MAX`.
pub const LINE_MAX: usize = 2000;
/// `profile_api.LEGAL_MSG`: the 422 of a profile that reads like court material.
pub const PROFILE_LEGAL_MSG: &str =
    "That reads like court material; Punchable won't keep it as your voice profile. Nothing was saved.";

/// `suggest_api.ID_RE` (`re.match`: `$` matches before one final newline too).
static SUGGEST_ID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\Ajoke:[A-Za-z0-9:_@.+\-]{1,290}\n?\z").unwrap_or_else(|e| unreachable!("constant regex: {e}"))
});

/// `PATCH /api/settings {provider?, training?, upload_video?, cellular?}`: the settings as `GET` answers them.
pub async fn settings_patch(
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
        let cr = Creative::new(st.clone());
        let items = cr.items("account").await?;
        pk_creative::reducers::account::check_settings(&b).map_err(|e| http(400, e.msg()))?;
        let defaults = crate::setlists::settings(&Map::new());
        let cur: Value = items
            .get("settings")
            .filter(|s| py::truthy(Some(s)))
            .cloned()
            .unwrap_or(defaults);
        let changed = b.iter().any(|(k, v)| cur.get(k).unwrap_or(&Value::Null) != v);
        let items = if changed {
            cr.append("account", "settings", "settings", Value::Object(b), &owner)
                .await?;
            cr.items("account").await?
        } else {
            items
        };
        Ok(crate::setlists::settings(&items))
    })
    .await?;
    Ok(ok(&out))
}

/// `profile_api.clean_text(body)`: the text as stored (`\r\n` to `\n`, outer newlines off, one at the end).
fn clean_text(b: &Map<String, Value>) -> ApiResult<String> {
    let t = b
        .get("text")
        .and_then(Value::as_str)
        .filter(|t| !py::strip(t).is_empty())
        .ok_or_else(|| http(400, "text is required"))?;
    let t = format!("{}\n", t.replace("\r\n", "\n").trim_matches('\n'));
    if py::len(&t) > MAX_CHARS {
        return Err(http(413, "the profile is at most 60,000 characters"));
    }
    Ok(t)
}

/// `profile_api.put(cr, kb, body, by)`: validate (shape, size, legal), then one `put` event; the profile as the
/// app's GET answers it. Shared by his PUT and the loader's worker PUT.
pub(crate) async fn put_voice(st: &AppState, b: &Map<String, Value>, by: &str) -> ApiResult<Value> {
    let _cr = st.creative_lock().lock().await;
    let text = clean_text(b)?;
    if st.legal().is_legal(&text) {
        return Err(http(422, PROFILE_LEGAL_MSG));
    }
    let cr = Creative::new(st.clone());
    let doc = cr
        .append("profile", "put", VOICE_ID, json!({"kind": "voice", "text": text}), by)
        .await?;
    Ok(crate::routes::stores::voice_view(
        doc.as_object(),
        &st.config().owner_id,
    ))
}

/// `PUT /api/profile/voice {text}`: his voice profile replaced (the previous one kept for Undo).
pub async fn profile_put(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let by = actor(&st, &caller, &b)?;
    let out = detached(async move { put_voice(&st, &b, &by).await }).await?;
    Ok(ok(&out))
}

/// `profile_api.clean_dropped(body)`: 1-10 lines, each `{text, bit}`.
fn clean_dropped(b: &Map<String, Value>) -> ApiResult<Vec<Value>> {
    let xs = b
        .get("dropped")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.len() <= MAX_DISMISS)
        .ok_or_else(|| http(400, format!("dropped must be a list of 1-{MAX_DISMISS} lines")))?;
    let joke_id = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .filter(|s| super::is_id(s))
            .map(str::to_string)
    };
    let bit0 = joke_id(b.get("bit"));
    let mut out = Vec::with_capacity(xs.len());
    for x in xs {
        let (text, bit) = match x {
            Value::String(s) => (Some(s.as_str()), None),
            Value::Object(o) => (o.get("text").and_then(Value::as_str), joke_id(o.get("bit"))),
            _ => (None, None),
        };
        let Some(text) = text.filter(|t| !py::strip(t).is_empty()) else {
            return Err(http(400, "each dropped line needs its text"));
        };
        if py::len(text) > LINE_MAX {
            return Err(http(413, "a line is at most 2,000 characters"));
        }
        out.push(json!({"text": py::strip(text), "bit": bit.or_else(|| bit0.clone())}));
    }
    Ok(out)
}

/// `POST /api/profile/voice/learn {dropped: [text | {text, bit?}], bit?}`: he dismissed a draft.
pub async fn profile_learn(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let by = actor(&st, &caller, &Map::new())?;
    let out = detached(async move {
        let lines = clean_dropped(&b)?;
        refuse_legal(&st, lines.iter().filter_map(|l| l.get("text").and_then(Value::as_str)))?;
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let doc = cr
            .append(
                "profile",
                "learn",
                VOICE_ID,
                json!({"dropped": lines, "from": "dismiss"}),
                &by,
            )
            .await?;
        Ok(json!({"ok": true, "learned": counts(doc.get("learned"))}))
    })
    .await?;
    Ok(ok(&out))
}

fn suggested_joke(b: &Map<String, Value>) -> ApiResult<String> {
    b.get("joke_id")
        .and_then(Value::as_str)
        .filter(|j| SUGGEST_ID_RE.is_match(j))
        .map(str::to_string)
        .ok_or_else(|| http(400, "joke_id is required"))
}

/// `POST /api/suggestions/not-a-joke/keep {joke_id}`: "it is a joke": never suggested again.
pub async fn keep(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let jid = suggested_joke(&b)?;
    let by = actor(&st, &caller, &Map::new())?;
    let out = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let already = |items: &pk_creative::Items| -> ApiResult<Pre> {
            Ok(if items.contains_key(&jid) {
                Pre::Done(StatusCode::OK, Value::Null)
            } else {
                Pre::Go
            })
        };
        match cr
            .append_if("suggestions", "keep", &jid, json!({"kind": "not_a_joke"}), &by, already)
            .await?
        {
            Written::Landed(_) | Written::Skipped(..) => Ok(json!({"ok": true, "joke_id": jid, "kept": true})),
        }
    })
    .await?;
    Ok(ok(&out))
}

/// `POST /api/suggestions/not-a-joke/hide {joke_id}`: the idea hidden, by him, undoable.
pub async fn hide(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q, b } = request(st, raw.as_deref(), &headers, body).await?;
    let jid = suggested_joke(&b)?;
    let by = actor(&st, &caller, &Map::new())?;
    let hidden = q.include_hidden();
    let out = detached(async move {
        let _kb = st.kb_lock().lock().await;
        let req = json!({"target": "node", "action": "edit", "id": jid, "after": {"hidden": true},
                         "reason": "not a joke (Punchable suggested it; you hid it)"});
        let (idx, line) = Kb::new(st.clone(), hidden).decide(&req, &by).await?;
        Ok(
            json!({"ok": true, "joke_id": jid, "indexes": [idx], "decisions": [Value::Object(line.0)],
                  "undo": {"POST": "/api/undo", "indexes": [idx]}}),
        )
    })
    .await?;
    Ok(ok(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(v: &Value) -> Map<String, Value> {
        v.as_object().cloned().expect("an object")
    }

    #[test]
    fn the_profile_text_is_cleaned_as_the_python_cleans_it() {
        assert_eq!(
            clean_text(&body(&json!({"text": "\n\r\nhi\r\nthere\n\n"}))).unwrap(),
            "hi\nthere\n"
        );
        assert_eq!(
            clean_text(&body(&json!({"text": "  "}))).unwrap_err().msg,
            "text is required"
        );
        assert_eq!(
            clean_text(&body(&json!({"text": 3}))).unwrap_err().msg,
            "text is required"
        );
        let e = clean_text(&body(&json!({"text": "x".repeat(60_001)}))).unwrap_err();
        assert_eq!(
            (e.status.as_u16(), e.msg.as_str()),
            (413, "the profile is at most 60,000 characters")
        );
        assert!(clean_text(&body(&json!({"text": "x".repeat(59_999)}))).is_ok());
    }

    #[test]
    fn dropped_lines_take_a_text_or_an_object_with_the_bit() {
        let got = clean_dropped(&body(
            &json!({"dropped": [" a ", {"text": "b", "bit": "joke:x"}, {"text": "c"}],
                                             "bit": "joke:y"}),
        ))
        .unwrap();
        assert_eq!(
            got,
            vec![
                json!({"text": "a", "bit": "joke:y"}),
                json!({"text": "b", "bit": "joke:x"}),
                json!({"text": "c", "bit": "joke:y"})
            ]
        );
        assert_eq!(
            clean_dropped(&body(&json!({"dropped": []}))).unwrap_err().msg,
            "dropped must be a list of 1-10 lines"
        );
        assert_eq!(
            clean_dropped(&body(&json!({"dropped": [{"bit": "joke:x"}]})))
                .unwrap_err()
                .msg,
            "each dropped line needs its text"
        );
        assert_eq!(
            clean_dropped(&body(&json!({"dropped": [" "]}))).unwrap_err().msg,
            "each dropped line needs its text"
        );
        let e = clean_dropped(&body(&json!({"dropped": ["x".repeat(2001)]}))).unwrap_err();
        assert_eq!(
            (e.status.as_u16(), e.msg.as_str()),
            (413, "a line is at most 2,000 characters")
        );
        assert_eq!(
            clean_dropped(&body(&json!({"dropped": ["a"], "bit": "not an id"}))).unwrap()[0]["bit"],
            Value::Null
        );
    }

    #[test]
    fn a_suggested_joke_is_a_joke_id() {
        assert_eq!(suggested_joke(&body(&json!({"joke_id": "joke:a"}))).unwrap(), "joke:a");
        assert_eq!(
            suggested_joke(&body(&json!({"joke_id": "unit:a"}))).unwrap_err().msg,
            "joke_id is required"
        );
        assert!(suggested_joke(&body(&json!({}))).is_err());
    }
}
