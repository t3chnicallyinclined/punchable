//! `POST /api/sets/transcript` (`creative_api.Routes.set_transcript`): a set transcribed on the phone. The words are
//! checked and queued as one `set_text` job for the hosted worker, keyed by a digest of the segments so the same
//! transcript posted twice is one job (the app's retry lands on the job already queued, or the finished one): 202
//! `{status: "queued", job, poll}`, or 200 with the finished job's result. No audio is sent or kept.

use std::sync::LazyLock;

use axum::Extension;
use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_core::text::head;
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use super::{Creative, body_json, detached};
use crate::auth::Caller;
use crate::error::{ApiError, ApiResult, json_response};
use crate::query::Query;
use crate::state::AppState;

/// `re.match(r"^\d{4}-\d{2}-\d{2}", rec)` (Python's `\d` is any Unicode decimal digit, as the regex crate's).
static DATE_HEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A\d{4}-\d{2}-\d{2}").unwrap_or_else(|e| unreachable!("constant regex: {e}")));

/// `isinstance(x, (int, float)) and not isinstance(x, bool)`.
fn is_number(v: &Value) -> bool {
    v.is_number()
}

/// The validated request: the segments to send and the fields beside them (the same refusals, in the same order).
fn check(b: &Map<String, Value>) -> ApiResult<(Vec<Value>, Map<String, Value>)> {
    let bad = |m: &str| ApiError::bad_request(m);
    let segs = b
        .get("segments")
        .and_then(Value::as_array)
        .filter(|s| !s.is_empty() && s.len() <= 5000)
        .ok_or_else(|| bad("segments must be a list of 1-5000 {text, start?, end?}"))?;
    let mut chars = 0usize;
    for s in segs {
        let Some(text) = s.as_object().and_then(|o| o.get("text")).and_then(Value::as_str) else {
            return Err(bad("each segment needs text"));
        };
        chars += py::len(text);
        for k in ["start", "end"] {
            if let Some(v) = s.get(k).filter(|v| !v.is_null())
                && !is_number(v)
            {
                return Err(bad(&format!("segment {k} must be seconds (a number)")));
            }
        }
    }
    if chars > 400_000 {
        return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "transcript too long"));
    }
    if !segs
        .iter()
        .any(|s| !py::strip(s["text"].as_str().unwrap_or("")).is_empty())
    {
        return Err(bad("the transcript is empty"));
    }
    let title = b.get("title").filter(|t| !t.is_null());
    if let Some(t) = title
        && t.as_str().is_none_or(|s| py::len(s) > 200)
    {
        return Err(bad("title must be text (at most 200 characters)"));
    }
    let rec = b.get("recorded_at").filter(|t| !t.is_null());
    if let Some(r) = rec
        && !r.as_str().is_some_and(|s| py::len(s) <= 40 && DATE_HEAD.is_match(s))
    {
        return Err(bad("recorded_at must be an ISO date-time"));
    }
    let venue = b.get("venue").filter(|t| !t.is_null());
    if let Some(v) = venue
        && v.as_str().is_none_or(|s| py::len(s) > 120)
    {
        return Err(bad("venue must be text (at most 120 characters)"));
    }
    let dur = b.get("duration_s").filter(|t| !t.is_null());
    if let Some(d) = dur
        && !(is_number(d) && d.as_f64().is_some_and(|x| x >= 0.0))
    {
        return Err(bad("duration_s must be seconds"));
    }
    let clean: Vec<Value> = segs
        .iter()
        .map(|s| {
            let o = s.as_object().expect("checked above");
            let keep: Map<String, Value> = ["text", "start", "end"]
                .iter()
                .filter_map(|k| o.get(*k).map(|v| ((*k).to_string(), v.clone())))
                .collect();
            Value::Object(keep)
        })
        .collect();
    let mut fields = Map::new();
    fields.insert("title".into(), title.cloned().unwrap_or(Value::Null));
    fields.insert("recorded_at".into(), rec.cloned().unwrap_or(Value::Null));
    fields.insert("venue".into(), venue.cloned().unwrap_or(Value::Null));
    fields.insert("duration_s".into(), dur.cloned().unwrap_or(Value::Null));
    Ok((clean, fields))
}

/// The job key's digest: sha256 of `json.dumps([stripped texts], ensure_ascii=False)`, first 16 hex.
fn digest(segs: &[Value]) -> String {
    let texts: Vec<Value> = segs
        .iter()
        .map(|s| json!(py::strip(s["text"].as_str().unwrap_or(""))))
        .collect();
    hex::encode(Sha256::digest(py::dumps(&Value::Array(texts)).as_bytes()))[..16].to_string()
}

/// `POST /api/sets/transcript`.
pub async fn transcript(
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
    let (segs, fields) = check(&b)?;
    let key = format!("set_text|{}", digest(&segs));
    let engine = head(
        &b.get("engine")
            .filter(|e| py::truthy(Some(e)))
            .map_or_else(|| "phone".to_string(), py::str),
        40,
    );
    let mut input = Map::new();
    input.insert("segments".into(), Value::Array(segs));
    input.extend(fields);
    input.insert("received_at".into(), json!(pk_core::time::now_iso()));
    input.insert("engine".into(), json!(engine));
    let (status, out) = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        super::jobs::enqueue(
            &cr,
            "set_text",
            &key,
            Value::Object(input),
            false,
            &st.config().owner_id,
        )
        .await
    })
    .await?;
    Ok(json_response(status, &out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => panic!("an object"),
        }
    }

    #[test]
    fn the_refusals_are_the_pythons_in_its_order() {
        let msg = |v: Value| check(&body(v)).unwrap_err();
        assert_eq!(
            msg(json!({})).msg,
            "segments must be a list of 1-5000 {text, start?, end?}"
        );
        assert_eq!(
            msg(json!({"segments": []})).msg,
            "segments must be a list of 1-5000 {text, start?, end?}"
        );
        assert_eq!(msg(json!({"segments": [{"start": 1}]})).msg, "each segment needs text");
        assert_eq!(msg(json!({"segments": ["x"]})).msg, "each segment needs text");
        assert_eq!(
            msg(json!({"segments": [{"text": "a", "start": true}]})).msg,
            "segment start must be seconds (a number)"
        );
        assert_eq!(
            msg(json!({"segments": [{"text": "a", "end": "3"}]})).msg,
            "segment end must be seconds (a number)"
        );
        let long = json!({"segments": [{"text": "x".repeat(400_001)}]});
        let e = msg(long);
        assert_eq!((e.status.as_u16(), e.msg.as_str()), (413, "transcript too long"));
        assert_eq!(
            msg(json!({"segments": [{"text": "  "}]})).msg,
            "the transcript is empty"
        );
        assert_eq!(
            msg(json!({"segments": [{"text": "a"}], "title": 3})).msg,
            "title must be text (at most 200 characters)"
        );
        assert_eq!(
            msg(json!({"segments": [{"text": "a"}], "recorded_at": "yesterday"})).msg,
            "recorded_at must be an ISO date-time"
        );
        assert_eq!(
            msg(json!({"segments": [{"text": "a"}], "venue": "v".repeat(121)})).msg,
            "venue must be text (at most 120 characters)"
        );
        assert_eq!(
            msg(json!({"segments": [{"text": "a"}], "duration_s": -1})).msg,
            "duration_s must be seconds"
        );
        assert_eq!(
            msg(json!({"segments": [{"text": "a"}], "duration_s": false})).msg,
            "duration_s must be seconds"
        );
        let (segs, fields) = check(&body(
            json!({"segments": [{"text": "a", "start": 0, "end": null, "x": 1}],
                                                 "recorded_at": "２０２６-１１-０３T10", "duration_s": 12.5}),
        ))
        .unwrap();
        assert_eq!(
            segs,
            json!([{"text": "a", "start": 0, "end": null}])
                .as_array()
                .cloned()
                .unwrap()
        );
        assert_eq!(fields["title"], Value::Null);
        assert_eq!(fields["duration_s"], 12.5);
        assert_eq!(
            fields["recorded_at"], "２０２６-１１-０３T10",
            "Python's \\d takes any Unicode digit"
        );
    }

    #[test]
    fn the_key_digest_is_over_the_stripped_texts_as_python_dumps_them() {
        // sha256 of '["a", "b c"]' (json.dumps default separators), first 16 hex
        let segs = json!([{"text": " a "}, {"text": "b c"}]).as_array().cloned().unwrap();
        let want = hex::encode(Sha256::digest(br#"["a", "b c"]"#))[..16].to_string();
        assert_eq!(digest(&segs), want);
    }
}
