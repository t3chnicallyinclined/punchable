//! The one-shot questions and the imports that only queue a job (T10 group 6, the cheap half; `creative_api.Routes`
//! `ask`, `riff`, `facts`, `youtube`, `job_drop`): `POST /api/ask {question, fresh?}`, `POST /api/riff {idea,
//! lines?, fresh?}`, `POST /api/facts {text, fresh?}`, `POST /api/import/youtube {url}` and `POST /api/jobs/<id>/drop
//! {reason?}`. Each validates, refuses court material, and queues (or answers the finished job of the same key)
//! through the one job queue both servers share. `/api/ask/chats*` (the conversations) stays the Python's.

use std::sync::LazyLock;

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use pk_core::py;
use pk_creative::Items;
use regex::Regex;
use serde_json::{Map, Value, json};
use sha1::{Digest as _, Sha1};

use super::jobs::{brief, enqueue};
use super::{Creative, Plan, Req, Written, actor, detached, http, refuse_legal, request};
use crate::auth::Caller;
use crate::error::{ApiResult, json_response, ok};
use crate::state::AppState;

/// `YT_RE`: a video link, its id in the one group.
static YT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\Ahttps?://(?:www\.|m\.|music\.)?(?:youtube\.com/(?:watch\?(?:[^#]*&)?v=|shorts/|live/|embed/)|youtu\.be/)([A-Za-z0-9_-]{11})(?:[?&#/].*)?\n?\z",
    )
    .unwrap_or_else(|e| unreachable!("constant regex: {e}"))
});

/// `hashlib.sha1(x).hexdigest()[:16]`.
fn sha16(seed: &str) -> String {
    hex::encode(Sha1::digest(seed.as_bytes()))[..16].to_string()
}

/// `" ".join(text.lower().split())`.
fn folded(text: &str) -> String {
    py::split(&text.to_lowercase()).collect::<Vec<_>>().join(" ")
}

fn fresh(b: &Map<String, Value>) -> bool {
    py::truthy(b.get("fresh"))
}

/// A text field: a string with words, at most `max` characters.
fn text_field<'a>(b: &'a Map<String, Value>, k: &str, max: usize) -> Option<&'a str> {
    b.get(k)
        .and_then(Value::as_str)
        .filter(|t| !py::strip(t).is_empty() && py::len(t) <= max)
}

async fn queued(st: AppState, kind: &str, key: String, input: Value, is_fresh: bool) -> ApiResult<(StatusCode, Value)> {
    let owner = st.config().owner_id.clone();
    let _cr = st.creative_lock().lock().await;
    let cr = Creative::new(st.clone());
    enqueue(&cr, kind, &key, input, is_fresh, &owner).await
}

/// `POST /api/ask {question, fresh?}`: one question for the worker, keyed by its words.
pub async fn ask(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: axum::http::HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let qn = text_field(&b, "question", 1000)
        .ok_or_else(|| http(400, "question is required (at most 1000 characters)"))?
        .to_string();
    let key = format!("ask|{}", sha16(&folded(&qn)));
    let (s, v) =
        detached(async move { queued(st, "ask", key, json!({"question": py::strip(&qn)}), fresh(&b)).await }).await?;
    Ok(json_response(s, &v))
}

/// `POST /api/riff {idea, lines?, fresh?}`: lines on an idea, keyed by the idea and the lines.
pub async fn riff(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: axum::http::HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let idea = text_field(&b, "idea", 2000)
        .ok_or_else(|| http(400, "idea is required (at most 2000 characters)"))?
        .to_string();
    let lines: Vec<&str> = match b.get("lines").filter(|l| py::truthy(Some(l))) {
        None => Vec::new(),
        Some(Value::Array(xs)) if xs.len() <= 40 => {
            let mut out = Vec::with_capacity(xs.len());
            for x in xs {
                match x.as_str().filter(|s| py::len(s) <= 2000) {
                    Some(s) => out.push(s),
                    None => return Err(http(400, "lines must be a list of text (at most 40)")),
                }
            }
            out
        }
        Some(_) => return Err(http(400, "lines must be a list of text (at most 40)")),
    };
    refuse_legal(&st, std::iter::once(idea.as_str()).chain(lines.iter().copied()))?;
    let input = json!({"idea": py::strip(&idea),
                       "lines": lines.iter().map(|x| py::strip(x)).filter(|x| !x.is_empty()).collect::<Vec<_>>()});
    let key = format!("riff|{}", sha16(&py::dumps_sorted(&input).to_lowercase()));
    let (s, v) = detached(async move { queued(st, "riff", key, input, fresh(&b)).await }).await?;
    Ok(json_response(s, &v))
}

/// `POST /api/facts {text, fresh?}`: a fact check of a line, keyed by its words.
pub async fn facts(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: axum::http::HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let text = text_field(&b, "text", 2000)
        .ok_or_else(|| http(400, "text is required (at most 2000 characters)"))?
        .to_string();
    refuse_legal(&st, [text.as_str()])?;
    let key = format!("facts|{}", sha16(&folded(&text)));
    let (s, v) =
        detached(async move { queued(st, "facts", key, json!({"text": py::strip(&text)}), fresh(&b)).await }).await?;
    Ok(json_response(s, &v))
}

/// `POST /api/import/youtube {url}`: a set from a video link, queued for a transcriber (none runs now: the job
/// waits, honestly); a set already imported from that video answers `exists`.
pub async fn youtube(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: axum::http::HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q, b } = request(st, raw.as_deref(), &headers, body).await?;
    let url = b
        .get("url")
        .and_then(Value::as_str)
        .filter(|u| py::len(u) <= 500)
        .ok_or_else(|| http(400, "url is required"))?;
    let vid = YT_RE
        .captures(py::strip(url))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| {
            http(
                400,
                "that is not a YouTube video link (youtube.com/watch?v=..., youtu.be/..., /shorts/...)",
            )
        })?;
    let hidden = q.include_hidden();
    let (s, v) = detached(async move {
        let d = st.data().await?;
        let v = d.view(hidden).await?;
        let suffix = format!("_{vid}");
        let have = v
            .nodes
            .values()
            .find(|n| {
                n.get("type").and_then(Value::as_str) == Some("set")
                    && n.get("id").map(py::str).is_some_and(|id| id.ends_with(&suffix))
            })
            .and_then(|n| n.get("id").cloned());
        if let Some(set) = have {
            return Ok((StatusCode::OK, json!({"status": "exists", "set": set, "video_id": vid})));
        }
        let input = json!({"url": format!("https://www.youtube.com/watch?v={vid}"), "video_id": vid});
        queued(st, "import_youtube", format!("youtube|{vid}"), input, false).await
    })
    .await?;
    Ok(json_response(s, &v))
}

/// `POST /api/jobs/<id>/drop {reason?}`: his call to stop waiting for a queued job; it ends `failed` with the
/// reason (nothing of his is deleted). 409 when it is running or over.
pub async fn drop(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(jid): Path<String>,
    headers: axum::http::HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let by = actor(&st, &caller, &b)?;
    let reason: String = b
        .get("reason")
        .filter(|r| py::truthy(Some(r)))
        .map_or_else(|| "dropped by the owner".to_string(), py::str)
        .chars()
        .take(300)
        .collect();
    let out = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let check = |items: &Items| -> ApiResult<()> {
            let j = items
                .get(&jid)
                .and_then(Value::as_object)
                .ok_or_else(|| http(404, "no such job"))?;
            match j.get("status").and_then(Value::as_str).unwrap_or("") {
                "queued" => Ok(()),
                other => Err(http(409, format!("job is {other}"))),
            }
        };
        let items = cr.items("jobs").await?;
        check(items.as_ref())?;
        let plan = |items: &Items| -> ApiResult<Plan> {
            check(items)?;
            Ok(Plan::Event {
                op: "fail".into(),
                id: jid.clone(),
                data: json!({"error": reason, "retry": false}),
            })
        };
        match cr.append_planned("jobs", &by, None, plan).await? {
            Written::Landed(job) => {
                let job = job.as_object().cloned().unwrap_or_default();
                super::uploads::release_for_job(&st, &job).await;
                Ok(json!({"ok": true, "job": brief(&job)}))
            }
            Written::Skipped(..) => Err(crate::error::ApiError::internal(
                "StateError",
                "a drop's plan never skips",
            )),
        }
    })
    .await?;
    Ok(ok(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_video_id_is_the_pythons_group() {
        let id = |u: &str| YT_RE.captures(u).map(|c| c[1].to_string());
        assert_eq!(
            id("https://www.youtube.com/watch?v=dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(id("https://youtu.be/dQw4w9WgXcQ?t=4").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(
            id("https://m.youtube.com/shorts/dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(
            id("https://www.youtube.com/watch?list=x&v=dQw4w9WgXcQ&t=1").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(id("https://vimeo.com/123"), None);
        assert_eq!(id("https://www.youtube.com/watch?v=short"), None);
    }

    #[test]
    fn keys_fold_case_and_spacing_like_the_python() {
        assert_eq!(folded("  Parking   METERS\nthat talk "), "parking meters that talk");
        assert_eq!(sha16("abc"), "a9993e364706816a");
        let input = json!({"idea": "x", "lines": ["A", "b"]});
        assert_eq!(
            py::dumps_sorted(&input).to_lowercase(),
            r#"{"idea": "x", "lines": ["a", "b"]}"#
        );
    }
}
