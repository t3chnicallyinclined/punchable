//! The iPhone voice memos (T10 group 6a; `api-dev/memos_api.py`): `GET /api/memos` (the list with the progress line),
//! `GET /api/memos/<id>` (one memo with its segments) and `GET /api/memos/<id>/audio`. gsync's state is the
//! `kb/gsync/memos.json` row of `srcfile` and each transcript a `corpus/memos/<dir>/transcript.json` row, which the
//! Python pod writes to its disk and reads back; here the rows are read by key (kept per path while their hash stays
//! the same, `SrcTexts`). The audio is never in the database (`db/push_remote.py`: "never their audio"), so a memo's
//! `audio` is always `null` and its audio route always the 404 the Python answers when the file is not on the pod.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use axum::extract::{Path, RawQuery, State};
use axum::response::Response;
use pk_core::py;
use pk_store::{from_db, vars};
use serde_json::{Map, Value, json};

use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::state::AppState;
use crate::tape::srcfile_key;
use crate::writes::http;

/// `LIST_TEXT`: a list card's text is cut to this many characters.
const LIST_TEXT: usize = 600;
/// `HIDDEN`: memos that are only counted.
const HIDDEN: [&str; 2] = ["skipped_legal", "duplicate"];
/// `PROC_ALIVE_S`: the processor's last sign of life counts this long.
const PROC_ALIVE_S: f64 = 20.0 * 60.0;
/// gsync's memo state.
const MEMOS_JSON: &str = "kb/gsync/memos.json";

/// A row's `path` joined under the comedy folder the way `(comedy / rel).resolve()` and `relative_to` read it: the
/// path inside, or none when it leaves the folder.
fn inside(rel: Option<&Value>) -> Option<String> {
    let rel = rel.and_then(Value::as_str).filter(|r| !r.is_empty())?;
    if rel.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for p in rel.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// The texts of `paths` (`srcfile` rows), each from the cache when its hash is the row's.
pub(crate) async fn texts(st: &AppState, paths: &[String]) -> ApiResult<BTreeMap<String, Arc<str>>> {
    if paths.is_empty() {
        return Ok(BTreeMap::new());
    }
    let store = st.store_handle().ok_or_else(crate::writes::not_connected)?;
    let keys: Vec<String> = paths.iter().map(|p| srcfile_key(p)).collect();
    let rows = store
        .last(
            "LET $ids = array::map($ks, |$k| type::record('srcfile', $k));\
             SELECT path, sha256 FROM $ids;",
            vars([("ks", pk_store::to_db(&json!(keys)))]),
        )
        .await
        .map_err(crate::writes::store_err)?;
    let want: Vec<(String, String)> = pk_store::store::rows_of(rows)
        .into_iter()
        .map(from_db)
        .filter_map(|r| {
            Some((
                r.get("path")?.as_str()?.to_string(),
                r.get("sha256")?.as_str()?.to_string(),
            ))
        })
        .collect();
    st.src_texts()
        .get(Some(&store), &want)
        .await
        .map_err(crate::writes::store_err)
}

/// `load(comedy)`: gsync's memo state, or none when the row is not there or is not JSON.
async fn load(st: &AppState) -> ApiResult<Option<Map<String, Value>>> {
    let mut t = texts(st, &[MEMOS_JSON.to_string()]).await?;
    let Some(text) = t.remove(MEMOS_JSON) else {
        return Ok(None);
    };
    // gsync's memo state is megabytes at production scale: parsed off the async threads
    tokio::task::spawn_blocking(move || {
        serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| if let Value::Object(m) = v { Some(m) } else { None })
    })
    .await
    .map_err(|e| ApiError::internal("JoinError", e))
}

fn items_of(m: &Map<String, Value>) -> Vec<&Map<String, Value>> {
    m.get("items")
        .and_then(Value::as_object)
        .map(|o| o.values().filter_map(Value::as_object).collect())
        .unwrap_or_default()
}

fn status_of(it: &Map<String, Value>) -> Option<&str> {
    it.get("status").and_then(Value::as_str)
}

fn hidden(it: &Map<String, Value>) -> bool {
    status_of(it).is_some_and(|s| HIDDEN.contains(&s))
}

/// `progress(m)`: the counts and the line the app shows.
fn progress(m: &Map<String, Value>) -> Value {
    let items = items_of(m);
    let count = |s: &str| items.iter().filter(|it| status_of(it) == Some(s)).count();
    let total = items.iter().filter(|it| !hidden(it)).count();
    let done = count("done");
    let waiting = count("listed") + count("downloaded") + count("transcribing");
    let ts = m
        .get("processor")
        .filter(|p| py::truthy(Some(p)))
        .and_then(|p| p.get("ts"))
        .filter(|t| py::truthy(Some(t)))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let working = waiting > 0 && crate::worker::unix_now() - ts < PROC_ALIVE_S;
    let paused = m.get("paused").cloned().unwrap_or(Value::Null);
    let mut line = if total > 0 {
        format!(
            "{done} of {total} voice memo{} transcribed",
            if total == 1 { "" } else { "s" }
        )
    } else {
        "No voice memos yet".to_string()
    };
    if py::truthy(Some(&paused)) && paused.get("reason").and_then(Value::as_str) == Some("low_disk") {
        let free = paused.get("free_mb").map_or_else(|| "None".to_string(), py::str);
        let _ = write!(line, " \u{b7} paused: the PC is low on disk space ({free} MB free)");
    }
    json!({"line": line, "total": total, "transcribed": done, "waiting": waiting,
           "failed": count("failed"), "duplicates": count("duplicate"), "skipped_legal": count("skipped_legal"),
           "working": working, "paused": paused})
}

/// `_ts(s)`: `datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()`, 0 when it is not one.
fn ts_of(s: Option<&Value>) -> f64 {
    let Some(s) = s.and_then(Value::as_str) else {
        return 0.0;
    };
    let s = s.replace('Z', "+00:00");
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(&s.replacen(' ', "T", 1)) {
        return t.timestamp() as f64 + f64::from(t.timestamp_subsec_micros()) / 1e6;
    }
    for f in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(t) = chrono::NaiveDateTime::parse_from_str(&s.replacen(' ', "T", 1), f) {
            return t.and_utc().timestamp() as f64;
        }
    }
    chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map_or(0.0, |t| t.and_utc().timestamp() as f64)
}

/// `reference(it)`: where the recording lives (his Drive, or his phone).
fn reference(it: &Map<String, Value>) -> Value {
    let get = |k: &str| it.get(k).cloned().unwrap_or(Value::Null);
    if let Some(id) = it.get("drive_id").filter(|d| py::truthy(Some(d))) {
        return json!({"kind": "drive", "url": format!("https://drive.google.com/file/d/{}/view", py::str(id)),
                      "name": get("name")});
    }
    if py::truthy(it.get("name")) || py::truthy(it.get("recorded_at")) {
        return json!({"kind": "file", "name": get("name"), "recorded_at": get("recorded_at")});
    }
    Value::Null
}

/// `text_of(comedy, it)`: the memo's words joined, and its segments.
fn text_of(it: &Map<String, Value>, tx: &BTreeMap<String, Arc<str>>) -> (String, Vec<Value>) {
    let Some(raw) = inside(it.get("transcript")).and_then(|p| tx.get(&p)) else {
        return (String::new(), Vec::new());
    };
    let Ok(doc) = serde_json::from_str::<Value>(raw) else {
        return (String::new(), Vec::new());
    };
    let segs: Vec<&Map<String, Value>> = doc
        .get("segments")
        .filter(|s| py::truthy(Some(s)))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_object).collect())
        .unwrap_or_default();
    let words = |s: &Map<String, Value>| {
        s.get("text")
            .and_then(Value::as_str)
            .map(|t| py::strip(t).to_string())
            .unwrap_or_default()
    };
    let joined = segs.iter().map(|s| words(s)).collect::<Vec<_>>().join(" ");
    let text = py::split(&joined).collect::<Vec<_>>().join(" ");
    let segments = segs
        .iter()
        .map(|s| {
            json!({"start": s.get("start").cloned().unwrap_or(Value::Null),
                   "end": s.get("end").cloned().unwrap_or(Value::Null), "text": words(s)})
        })
        .collect();
    (text, segments)
}

/// `card(comedy, it, full)`.
fn card(it: &Map<String, Value>, tx: &BTreeMap<String, Arc<str>>, full: bool) -> ApiResult<Value> {
    let id = it
        .get("id")
        .cloned()
        .ok_or_else(|| ApiError::internal("KeyError", "'id'"))?;
    let (text, segs) = if status_of(it) == Some("done") {
        text_of(it, tx)
    } else {
        (String::new(), Vec::new())
    };
    let get = |k: &str| it.get(k).cloned().unwrap_or(Value::Null);
    let att = it
        .get("attach")
        .filter(|a| py::truthy(Some(a)))
        .and_then(Value::as_object);
    let long = text.chars().count() > LIST_TEXT;
    let shown = if full || !long {
        text
    } else {
        let head: String = text.chars().take(LIST_TEXT).collect();
        let cut = head.rsplit_once(' ').map_or(head.as_str(), |(a, _)| a);
        format!("{cut}\u{2026}")
    };
    let title = it
        .get("title")
        .filter(|t| py::truthy(Some(t)))
        .cloned()
        .unwrap_or_else(|| {
            let name = it
                .get("name")
                .filter(|n| py::truthy(Some(n)))
                .map_or_else(|| "Voice memo".to_string(), py::str);
            json!(crate::writes::uploads::stem_of(&name))
        });
    let mut out = Map::new();
    out.insert("id".into(), id);
    out.insert("title".into(), title);
    out.insert("status".into(), get("status"));
    out.insert("date".into(), get("date"));
    out.insert("recorded_at".into(), get("recorded_at"));
    out.insert("date_source".into(), get("date_source"));
    out.insert("duration_s".into(), get("duration_s"));
    out.insert("text".into(), json!(shown));
    out.insert("text_cut".into(), json!(!full && long));
    out.insert("joke_id".into(), get("joke_id"));
    out.insert(
        "attached_to".into(),
        match att {
            Some(a) if a.get("decision").and_then(Value::as_str) == Some("attach") => {
                json!({"joke_id": a.get("into").cloned().unwrap_or(Value::Null),
                       "title": a.get("title").cloned().unwrap_or(Value::Null)})
            }
            _ => Value::Null,
        },
    );
    out.insert("long".into(), json!(py::truthy(it.get("long"))));
    out.insert("no_speech".into(), json!(py::truthy(it.get("no_speech"))));
    out.insert("audio".into(), Value::Null);
    out.insert("reference".into(), reference(it));
    out.insert(
        "error".into(),
        if status_of(it) == Some("failed") {
            get("error")
        } else {
            Value::Null
        },
    );
    if full {
        out.insert("segments".into(), Value::Array(segs));
    }
    Ok(Value::Object(out))
}

/// The booting 503, then `?backend=`, as every app read.
fn begin(st: &AppState, raw: Option<&str>) -> ApiResult<()> {
    st.require_loaded()?;
    super::pick(&Query::parse(raw))?;
    Ok(())
}

/// `GET /api/memos`.
pub async fn memos(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    begin(&st, raw.as_deref())?;
    let Some(m) = load(&st).await? else {
        return Ok(ok(
            &json!({"available": false, "progress": progress(&Map::new()), "items": []}),
        ));
    };
    let mut rows: Vec<&Map<String, Value>> = items_of(&m).into_iter().filter(|it| !hidden(it)).collect();
    let order = |it: &Map<String, Value>| match status_of(it) {
        Some("transcribing") => 0,
        Some("failed") => 1,
        Some("downloaded") => 2,
        Some("listed") => 3,
        Some("done") => 4,
        _ => 5,
    };
    // stable: `(order, -ts)`
    rows.sort_by(|a, b| {
        order(a)
            .cmp(&order(b))
            .then_with(|| ts_of(b.get("recorded_at")).total_cmp(&ts_of(a.get("recorded_at"))))
    });
    let paths: Vec<String> = rows
        .iter()
        .filter(|it| status_of(it) == Some("done"))
        .filter_map(|it| inside(it.get("transcript")))
        .collect();
    let tx = texts(&st, &paths).await?;
    let cards = rows
        .iter()
        .map(|it| card(it, &tx, false))
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(ok(
        &json!({"available": true, "progress": progress(&m), "items": cards}),
    ))
}

/// The memo `id` names, unless it is hidden or not there.
async fn find(st: &AppState, id: &str) -> ApiResult<Map<String, Value>> {
    load(st)
        .await?
        .and_then(|m| {
            m.get("items")
                .and_then(Value::as_object)
                .and_then(|o| o.get(id))
                .and_then(Value::as_object)
                .filter(|it| !it.is_empty() && !hidden(it))
                .cloned()
        })
        .ok_or_else(|| http(404, "no such voice memo"))
}

/// `GET /api/memos/<id>`: one memo with its segments.
pub async fn memo(State(st): State<AppState>, Path(id): Path<String>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    begin(&st, raw.as_deref())?;
    let it = find(&st, &id).await?;
    let paths: Vec<String> = inside(it.get("transcript")).into_iter().collect();
    let tx = texts(&st, &paths).await?;
    Ok(ok(&json!({"memo": card(&it, &tx, true)?})))
}

/// `GET /api/memos/<id>/audio`: the recording is never in the database, so the memo that is there has no audio here.
pub async fn audio(State(st): State<AppState>, Path(id): Path<String>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    begin(&st, raw.as_deref())?;
    find(&st, &id).await?;
    Err(http(404, "no audio for that memo"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: &Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn a_path_stays_inside_the_folder() {
        assert_eq!(
            inside(Some(&json!("corpus/memos/a/transcript.json"))).as_deref(),
            Some("corpus/memos/a/transcript.json")
        );
        assert_eq!(
            inside(Some(&json!("corpus/./memos/../memos/a.json"))).as_deref(),
            Some("corpus/memos/a.json")
        );
        assert_eq!(inside(Some(&json!("../etc/passwd"))), None);
        assert_eq!(inside(Some(&json!("/etc/passwd"))), None);
        assert_eq!(inside(Some(&json!(""))), None);
        assert_eq!(inside(None), None);
    }

    #[test]
    fn the_progress_line_counts_what_is_not_hidden() {
        let m = obj(
            &json!({"items": {"a": {"status": "done"}, "b": {"status": "listed"}, "c": {"status": "duplicate"},
                                      "d": {"status": "failed"}}}),
        );
        let p = progress(&m);
        assert_eq!(p["line"], "1 of 3 voice memos transcribed");
        assert_eq!(
            (p["total"].clone(), p["waiting"].clone(), p["failed"].clone()),
            (json!(3), json!(1), json!(1))
        );
        assert_eq!(p["working"], false);
        assert_eq!(progress(&Map::new())["line"], "No voice memos yet");
        let one = obj(&json!({"items": {"a": {"status": "done"}}}));
        assert_eq!(progress(&one)["line"], "1 of 1 voice memo transcribed");
        let low = obj(&json!({"paused": {"reason": "low_disk", "free_mb": 12}}));
        assert_eq!(
            progress(&low)["line"],
            "No voice memos yet \u{b7} paused: the PC is low on disk space (12 MB free)"
        );
    }

    #[test]
    fn a_long_text_is_cut_at_a_word() {
        let words = "word ".repeat(200);
        let it = obj(&json!({"id": "m", "status": "done", "transcript": "t.json"}));
        let tx: BTreeMap<String, Arc<str>> = BTreeMap::from([(
            "t.json".to_string(),
            Arc::from(json!({"segments": [{"start": 0, "end": 1, "text": words}]}).to_string()),
        )]);
        let list = card(&it, &tx, false).unwrap_or_default();
        assert_eq!(list["text_cut"], true);
        let text = list["text"].as_str().unwrap_or_default();
        assert!(text.ends_with("word\u{2026}") && text.chars().count() <= LIST_TEXT + 1);
        let full = card(&it, &tx, true).unwrap_or_default();
        assert_eq!(full["text_cut"], false);
        assert_eq!(full["segments"].as_array().map(Vec::len), Some(1));
    }
}
