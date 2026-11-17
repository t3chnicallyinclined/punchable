//! `POST /api/sets/transcript` (`creative_api.Routes.set_transcript`): a set transcribed on the phone. The words are
//! checked and queued as one `set_text` job for the hosted worker, keyed by a digest of the segments so the same
//! transcript posted twice is one job (the app's retry lands on the job already queued, or the finished one): 202
//! `{status: "queued", job, poll}`, or 200 with the finished job's result. No audio is sent or kept.
//!
//! Rust only (redesign slice 5, the walk-through): an optional `board` (the set list it followed, one he has),
//! `splits` (where he left each bit of it, from his own taps: `[{joke_id, start_ms, end_ms}]`, at most 500, in order,
//! never overlapping, inside the recording, each bit on that list) and `mode` (`sound` | `video`). They ride in the
//! job's input only when sent, so a phone without them sends exactly what it did; the Python twin drops them.

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

/// At most this many splits on one recording (a long set tapped bit by bit, Back and forward included).
pub const MAX_SPLITS: usize = 500;

/// The walk-through's fields as sent, checked for shape (the set list itself is checked under the lock).
#[derive(Debug, Default, PartialEq)]
pub struct Walk {
    /// The set list's id.
    pub board: Option<String>,
    /// `[{joke_id, start_ms, end_ms}]`, as sent once checked.
    pub splits: Option<Vec<Value>>,
    /// `sound` or `video`.
    pub mode: Option<String>,
}

/// One optional text field that must pass `ok`, else the refusal `msg`.
fn opt_text(b: &Map<String, Value>, k: &str, ok: impl Fn(&str) -> bool, msg: &str) -> ApiResult<Option<String>> {
    match b.get(k).filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .filter(|s| ok(s))
            .map(|s| Some(s.to_string()))
            .ok_or_else(|| ApiError::bad_request(msg)),
    }
}

/// `board`, `splits` and `mode`: their shapes, the order and overlap of the splits, and that they sit inside the
/// recording (`duration_s` plus a second of slack: the phone rounds it). Membership of the list is checked later.
pub fn check_walk(b: &Map<String, Value>, duration_s: Option<f64>) -> ApiResult<Walk> {
    let bad = |m: &str| ApiError::bad_request(m);
    let board = opt_text(
        b,
        "board",
        |s| super::is_id(s) && s.starts_with("sb:"),
        "board must be a set list's id",
    )?;
    let mode = opt_text(
        b,
        "mode",
        |s| matches!(s, "sound" | "video"),
        "mode must be sound or video",
    )?;
    let Some(sent) = b.get("splits").filter(|v| !v.is_null()) else {
        return Ok(Walk {
            board,
            splits: None,
            mode,
        });
    };
    if board.is_none() {
        return Err(bad("splits need the board they follow"));
    }
    let list = sent
        .as_array()
        .filter(|a| a.len() <= MAX_SPLITS)
        .ok_or_else(|| bad(&format!("splits must be a list of at most {MAX_SPLITS}")))?;
    let cap = duration_s.map(|d| (d * 1000.0).max(0.0) + 1000.0);
    let mut out = Vec::with_capacity(list.len());
    let mut last_end = 0u64;
    for (k, sp) in list.iter().enumerate() {
        let o = sp
            .as_object()
            .ok_or_else(|| bad("each split is {joke_id, start_ms, end_ms}"))?;
        let joke = o
            .get("joke_id")
            .and_then(Value::as_str)
            .filter(|j| super::is_id(j) && j.starts_with("joke:"))
            .ok_or_else(|| bad("each split needs a joke_id"))?;
        let (Some(from), Some(to)) = (
            o.get("start_ms").and_then(Value::as_u64),
            o.get("end_ms").and_then(Value::as_u64),
        ) else {
            return Err(bad("split start_ms and end_ms must be whole milliseconds"));
        };
        if from >= to {
            return Err(bad("a split must end after it starts"));
        }
        if k > 0 && from < last_end {
            return Err(bad("splits must be in order and never overlap"));
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "milliseconds of one recording, far below 2^52"
        )]
        let past = cap.is_some_and(|c| to as f64 > c);
        if past {
            return Err(bad("a split runs past the end of the recording"));
        }
        last_end = to;
        out.push(json!({"joke_id": joke, "start_ms": from, "end_ms": to}));
    }
    Ok(Walk {
        board,
        splits: Some(out),
        mode,
    })
}

/// The set list the walk-through names, as he has it now (not deleted), and every split's bit on it.
fn check_board(items: &pk_creative::Items, w: &Walk) -> ApiResult<()> {
    let Some(bid) = w.board.as_deref() else {
        return Ok(());
    };
    let board = items
        .get(bid)
        .and_then(Value::as_object)
        .filter(|b| !py::truthy(b.get("deleted")))
        .ok_or_else(|| ApiError::bad_request("board must be a set list you have"))?;
    let on: Vec<&str> = board
        .get("joke_ids")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    for sp in w.splits.iter().flatten() {
        let j = sp["joke_id"].as_str().unwrap_or("");
        if !on.contains(&j) {
            return Err(ApiError::bad_request(format!(
                "a split names {j}, which is not on that set list"
            )));
        }
    }
    Ok(())
}

/// The job key's digest: sha256 of `json.dumps([stripped texts], ensure_ascii=False)`, first 16 hex.
fn digest(segs: &[Value]) -> String {
    let texts: Vec<Value> = segs
        .iter()
        .map(|s| json!(py::strip(s["text"].as_str().unwrap_or(""))))
        .collect();
    hex::encode(Sha256::digest(py::dumps(&Value::Array(texts)).as_bytes()))[..16].to_string()
}

/// The job key: the Python's (`set_text|<digest of the texts>`) when no walk-through field is sent, so a phone without
/// the walk-through and the reads differential see the same job as before; with them, a digest of the list, the splits
/// and the mode is added, so the same words posted again with another list or corrected taps are a new job instead of
/// the old job's answer with the new fields dropped (security review).
fn walk_key(base: &str, walk: &Walk) -> String {
    if walk.board.is_none() && walk.splits.is_none() && walk.mode.is_none() {
        return base.to_string();
    }
    let w = json!({"board": walk.board, "splits": walk.splits, "mode": walk.mode});
    format!(
        "{base}|{}",
        &hex::encode(Sha256::digest(py::dumps(&w).as_bytes()))[..12]
    )
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
    let walk = check_walk(&b, fields.get("duration_s").and_then(Value::as_f64))?;
    let key = walk_key(&format!("set_text|{}", digest(&segs)), &walk);
    let engine = head(
        &b.get("engine")
            .filter(|e| py::truthy(Some(e)))
            .map_or_else(|| "phone".to_string(), py::str),
        40,
    );
    // the usage meter: how long and how big, never the words (spawned after the answer is decided, off its path)
    let seconds = fields.get("duration_s").and_then(Value::as_f64).unwrap_or(0.0);
    let size = headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let mut input = Map::new();
    input.insert("segments".into(), Value::Array(segs));
    input.extend(fields);
    input.insert("received_at".into(), json!(pk_core::time::now_iso()));
    input.insert("engine".into(), json!(engine));
    if let Some(bd) = &walk.board {
        input.insert("board".into(), json!(bd));
    }
    if let Some(sp) = &walk.splits {
        input.insert("splits".into(), Value::Array(sp.clone()));
    }
    if let Some(m) = &walk.mode {
        input.insert("mode".into(), json!(m));
    }
    let meter = (st.clone(), key.clone());
    let (status, out) = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        if walk.board.is_some() {
            let boards = cr.items("storyboards").await?;
            check_board(&boards, &walk)?;
        }
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
    let (st, key) = meter;
    if status.is_success()
        && let Some(store) = st.store_handle()
    {
        tokio::spawn(async move {
            let account = st.config().owner_id.clone();
            st.usage()
                .media(&store, &account, &key, "transcript", seconds, size)
                .await;
        });
    }
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
    fn the_walk_through_fields_are_checked_and_kept_only_when_sent() {
        let w = |v: Value| check_walk(&body(v), Some(60.0));
        assert_eq!(w(json!({})).unwrap(), Walk::default(), "nothing sent, nothing kept");
        let ok = w(json!({"board": "sb:fri-1", "mode": "video", "splits": [
            {"joke_id": "joke:a", "start_ms": 0, "end_ms": 1000, "x": 1},
            {"joke_id": "joke:b", "start_ms": 1000, "end_ms": 61000}]}))
        .unwrap();
        assert_eq!(ok.board.as_deref(), Some("sb:fri-1"));
        assert_eq!(ok.mode.as_deref(), Some("video"));
        assert_eq!(
            ok.splits.unwrap()[0],
            json!({"joke_id": "joke:a", "start_ms": 0, "end_ms": 1000}),
            "unknown keys dropped"
        );
        let msg = |v: Value| w(v).unwrap_err().msg;
        assert_eq!(msg(json!({"board": 3})), "board must be a set list's id");
        assert_eq!(msg(json!({"board": "joke:a"})), "board must be a set list's id");
        assert_eq!(msg(json!({"mode": "film"})), "mode must be sound or video");
        assert_eq!(
            msg(json!({"splits": [{"joke_id": "joke:a", "start_ms": 0, "end_ms": 1}]})),
            "splits need the board they follow"
        );
        assert_eq!(
            msg(json!({"board": "sb:x", "splits": {}})),
            "splits must be a list of at most 500"
        );
        let many: Vec<Value> = (0..501u64)
            .map(|i| json!({"joke_id": "joke:a", "start_ms": i, "end_ms": i + 1}))
            .collect();
        assert_eq!(
            check_walk(&body(json!({"board": "sb:x", "splits": many})), None)
                .unwrap_err()
                .msg,
            "splits must be a list of at most 500"
        );
        assert_eq!(
            msg(json!({"board": "sb:x", "splits": [{"start_ms": 0, "end_ms": 1}]})),
            "each split needs a joke_id"
        );
        assert_eq!(
            msg(json!({"board": "sb:x", "splits": [{"joke_id": "joke:a", "start_ms": -1, "end_ms": 1}]})),
            "split start_ms and end_ms must be whole milliseconds"
        );
        assert_eq!(
            msg(json!({"board": "sb:x", "splits": [{"joke_id": "joke:a", "start_ms": 5, "end_ms": 5}]})),
            "a split must end after it starts"
        );
        assert_eq!(
            msg(json!({"board": "sb:x", "splits": [
                {"joke_id": "joke:a", "start_ms": 0, "end_ms": 10},
                {"joke_id": "joke:b", "start_ms": 9, "end_ms": 20}]})),
            "splits must be in order and never overlap"
        );
        assert_eq!(
            msg(json!({"board": "sb:x", "splits": [{"joke_id": "joke:a", "start_ms": 0, "end_ms": 61001}]})),
            "a split runs past the end of the recording"
        );
    }

    #[test]
    fn every_split_names_a_bit_on_a_list_he_has() {
        let mut items = pk_creative::Items::new();
        items.insert(
            "sb:x".into(),
            json!({"id": "sb:x", "joke_ids": ["joke:a"], "deleted": false}),
        );
        items.insert(
            "sb:gone".into(),
            json!({"id": "sb:gone", "joke_ids": ["joke:a"], "deleted": true}),
        );
        let walk = |b: &str, j: &str| Walk {
            board: Some(b.into()),
            splits: Some(vec![json!({"joke_id": j, "start_ms": 0, "end_ms": 1})]),
            mode: None,
        };
        assert!(check_board(&items, &walk("sb:x", "joke:a")).is_ok());
        assert_eq!(
            check_board(&items, &walk("sb:x", "joke:b")).unwrap_err().msg,
            "a split names joke:b, which is not on that set list"
        );
        assert_eq!(
            check_board(&items, &walk("sb:gone", "joke:a")).unwrap_err().msg,
            "board must be a set list you have"
        );
        assert_eq!(
            check_board(&items, &walk("sb:none", "joke:a")).unwrap_err().msg,
            "board must be a set list you have"
        );
    }

    #[test]
    fn the_walk_fields_change_the_key_and_no_fields_keep_the_pythons() {
        let none = Walk::default();
        assert_eq!(walk_key("set_text|abc", &none), "set_text|abc");
        let a = Walk {
            board: Some("sb:a".into()),
            ..Walk::default()
        };
        let b = Walk {
            board: Some("sb:b".into()),
            ..Walk::default()
        };
        let ka = walk_key("set_text|abc", &a);
        assert!(ka.starts_with("set_text|abc|") && ka != walk_key("set_text|abc", &b));
        assert_eq!(ka, walk_key("set_text|abc", &a), "the same fields are the same job");
    }

    #[test]
    fn the_key_digest_is_over_the_stripped_texts_as_python_dumps_them() {
        // sha256 of '["a", "b c"]' (json.dumps default separators), first 16 hex
        let segs = json!([{"text": " a "}, {"text": "b c"}]).as_array().cloned().unwrap();
        let want = hex::encode(Sha256::digest(br#"["a", "b c"]"#))[..16].to_string();
        assert_eq!(digest(&segs), want);
    }
}
