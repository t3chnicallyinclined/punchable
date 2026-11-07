//! Ask conversations (T10 round 5, group A; `api-dev/ask_api.py`): `GET /api/ask/chats` (his past conversations,
//! newest first), `POST /api/ask/chats {question, joke?}` (a new one with its first question), `GET`, `POST` and
//! `PATCH /api/ask/chats/<id>` (one conversation; a follow-up; a rename or a hide).
//!
//! One turn: this service retrieves his sources from the notebook of the generation
//! ([`pk_creative::ask::Notebook::from_corpus`], kept on the corpus) and builds the prompt; the model call is an
//! `ask_chat` job for the hosted worker; when the job is done a GET runs the gate over this service's own text and
//! writes the answer (`settle`), or the failure in plain words. Nothing retrieved is answered at once, no job. The
//! events are the `chats` store's, written as `ai:ask` for the answers and by the owner for his questions, so the
//! Python reads what this writes and the other way round.
//!
//! A turn that was settled by the other server between our read and our append is left alone (the append's check
//! runs over the rows as they are then); the Python would answer that race with a refused event.

use std::sync::{Arc, LazyLock};

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_creative::Items;
use pk_creative::ask::{MAX_SOURCES, Turn, material_owner, refusal};
use pk_creative::merge::sha8_value;
use regex::Regex;
use serde_json::{Map, Value, json};

use super::jobs::enqueue;
use super::{Creative, Pre, body_json, detached, http, refuse_legal};
use crate::auth::Caller;
use crate::creative::GenCorpus;
use crate::error::{ApiError, ApiResult, json_response, ok};
use crate::query::Query;
use crate::state::AppState;

type Obj = Map<String, Value>;

/// The store the conversations live in.
const STORE: &str = "chats";
/// `MAX_Q`.
const MAX_Q: usize = 1000;
/// `MAX_TURNS`.
const MAX_TURNS: usize = 60;
/// The answers are `by: ai:ask`.
const AI: &str = "ai:ask";

/// `ID_RE` under `re.match` (`$` also matches before one final newline).
static ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\Achat:[a-f0-9]{8,16}\n?\z").unwrap_or_else(|e| unreachable!("constant regex: {e}")));
/// `JOKE_RE`.
static JOKE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\Ajoke:[A-Za-z0-9._:-]{1,120}\n?\z").unwrap_or_else(|e| unreachable!("constant regex: {e}"))
});

fn field<'a>(m: &'a Obj, k: &str) -> &'a str {
    m.get(k).and_then(Value::as_str).unwrap_or("")
}

fn turns_of(ch: &Obj) -> &[Value] {
    ch.get("turns").and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// `_history(ch)`: the finished turns as the retrieval reads them.
fn history(ch: &Obj) -> Vec<Turn> {
    turns_of(ch)
        .iter()
        .filter_map(Value::as_object)
        .filter(|t| field(t, "status") == "done")
        .map(|t| {
            let r = t.get("result").and_then(Value::as_object);
            let cited: Vec<Value> = r
                .and_then(|r| r.get("chips"))
                .and_then(Value::as_array)
                .map(|cs| {
                    cs.iter()
                        .map(|c| c.get("source").cloned().unwrap_or(Value::Null))
                        .collect()
                })
                .unwrap_or_default();
            serde_json::from_value(json!({
                "q": t.get("q"), "answer": r.and_then(|r| r.get("answer")), "cited": cited,
            }))
            .unwrap_or_default()
        })
        .collect()
}

/// `turn_view(t)`.
fn turn_view(t: &Obj) -> Value {
    let status = field(t, "status");
    let mut out = Map::new();
    for k in ["id", "q", "status", "asked_at"] {
        out.insert(k.into(), t.get(k).cloned().unwrap_or(Value::Null));
    }
    out.insert("done_at".into(), t.get("done_at").cloned().unwrap_or(Value::Null));
    let empty = Obj::new();
    let r = t.get("result").and_then(Value::as_object).unwrap_or(&empty);
    if status == "done" {
        out.insert("answer".into(), r.get("answer").cloned().unwrap_or(Value::Null));
        out.insert("found".into(), json!(py::truthy(r.get("found"))));
        out.insert(
            "chips".into(),
            r.get("chips")
                .filter(|c| py::truthy(Some(c)))
                .cloned()
                .unwrap_or_else(|| json!([])),
        );
        let dropped_n = r
            .get("dropped")
            .and_then(Value::as_array)
            .map_or(0, |d| d.iter().filter(|x| py::truthy(x.get("quote"))).count());
        out.insert("dropped_n".into(), json!(dropped_n));
        out.insert("model".into(), r.get("model").cloned().unwrap_or(Value::Null));
        out.insert("provider".into(), r.get("provider").cloned().unwrap_or(Value::Null));
    } else if status == "failed" {
        out.insert("error".into(), t.get("error").cloned().unwrap_or(Value::Null));
    }
    Value::Object(out)
}

/// `chat_view(ch, cr)`.
fn chat_view(ch: &Obj, worker: Value) -> Value {
    let mut out = Map::new();
    for k in ["id", "title", "created_at", "updated_at"] {
        out.insert(k.into(), ch.get(k).cloned().unwrap_or(Value::Null));
    }
    out.insert("joke".into(), ch.get("joke").cloned().unwrap_or(Value::Null));
    out.insert(
        "turns".into(),
        Value::Array(
            turns_of(ch)
                .iter()
                .filter_map(Value::as_object)
                .map(turn_view)
                .collect(),
        ),
    );
    out.insert("worker".into(), worker);
    Value::Object(out)
}

/// `_plain_error(err)`: what the worker's failure says, in words for the comic.
fn plain_error(err: &str) -> String {
    let e = err.to_lowercase();
    if e.contains("claude declined this one") || e.contains("claude ran out of room") {
        // engine.providers.Refusal / CutShort: the worker posts "<Type>: <plain words>"; the words are the message
        return match err.split_once(": ") {
            Some((_, words)) if err.starts_with("Refusal: ") || err.starts_with("CutShort: ") => words.to_string(),
            _ => err.to_string(),
        };
    }
    if e.contains("rate limit") || e.contains("usage limit") || e.contains("ratelimited") {
        return "Claude is at its usage limit on your PC right now. Ask again in a bit.".to_string();
    }
    if e.contains("timeout") || e.contains("timed out") {
        return "Your PC took too long to answer. Ask again.".to_string();
    }
    "Your PC couldn't answer that one. Ask again.".to_string()
}

/// `_question(b)`: one to a thousand characters, its whitespace folded.
fn question_of(b: &Obj) -> ApiResult<String> {
    b.get("question")
        .and_then(Value::as_str)
        .filter(|q| !py::strip(q).is_empty() && py::len(q) <= MAX_Q)
        .map(|q| py::split(q).collect::<Vec<_>>().join(" "))
        .ok_or_else(|| http(400, format!("question is required (at most {MAX_Q} characters)")))
}

/// Whether the turn `turn` of chat `cid` is still waiting for its answer in these rows.
fn still_thinking(items: &Items, cid: &str, turn: &str) -> bool {
    items.get(cid).and_then(Value::as_object).is_some_and(|ch| {
        turns_of(ch)
            .iter()
            .any(|t| field_v(t, "id") == turn && field_v(t, "status") == "thinking")
    })
}

fn field_v<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// The generation's corpus (the notebook of the answers), loaded on first need.
async fn corpus(st: &AppState) -> ApiResult<Arc<GenCorpus>> {
    st.data().await?.corpus(st).await
}

/// `settle(cr, kb, st, ch)`: the thinking turns whose job has finished are gated and written now (a done job), or
/// recorded as failed (a failed or a lost one). Rows are read again by the caller.
async fn settle(st: &AppState, cr: &Creative, ch: &Obj, c: &mut Option<Arc<GenCorpus>>) -> ApiResult<()> {
    let cid = field(ch, "id").to_string();
    let pend: Vec<&Obj> = turns_of(ch)
        .iter()
        .filter_map(Value::as_object)
        .filter(|t| field(t, "status") == "thinking" && py::truthy(t.get("job")))
        .collect();
    if pend.is_empty() {
        return Ok(());
    }
    let jobs = cr.items("jobs").await?;
    for t in pend {
        let turn = field(t, "id").to_string();
        let job = t.get("job").map(py::str).unwrap_or_default();
        let (op, data) = match jobs.get(&job).and_then(Value::as_object) {
            None => (
                "fail",
                json!({"turn": turn, "error": "The question was lost. Ask again."}),
            ),
            Some(j) => match field(j, "status") {
                "done" => {
                    if c.is_none() {
                        *c = Some(corpus(st).await?);
                    }
                    let corp = Arc::clone(c.as_ref().unwrap_or_else(|| unreachable!("loaded above")));
                    let res = j
                        .get("result")
                        .filter(|r| py::truthy(Some(r)))
                        .cloned()
                        .unwrap_or(json!({}));
                    let ids: Vec<String> = t
                        .get("sources")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().map(py::str).collect())
                        .unwrap_or_default();
                    let out = tokio::task::spawn_blocking(move || {
                        corp.ask_notebook().finish(
                            &ids,
                            res.get("data").unwrap_or(&Value::Null),
                            res.get("provider").and_then(Value::as_str),
                            res.get("model").and_then(Value::as_str),
                        )
                    })
                    .await
                    .map_err(|e| ApiError::internal("JoinError", e))?
                    .map_err(|e| ApiError::internal("CreativeError", e.msg()))?;
                    (
                        "answer",
                        json!({"turn": turn, "result": serde_json::to_value(out).map_err(|e| ApiError::internal("TypeError", e))?}),
                    )
                }
                "failed" => ("fail", json!({"turn": turn, "error": plain_error(field(j, "error"))})),
                _ => continue,
            },
        };
        let check = |items: &Items| -> ApiResult<Pre> {
            Ok(if still_thinking(items, &cid, &turn) {
                Pre::Go
            } else {
                Pre::Done(StatusCode::OK, Value::Null)
            })
        };
        cr.append_if(STORE, op, &cid, data, AI, check).await?;
    }
    Ok(())
}

/// `ask(cr, kb, st, chat_id, question)`: retrieve, queue the model's turn (or answer the refusal at once), and write
/// the question.
async fn ask(st: &AppState, cr: &Creative, cid: &str, question: &str) -> ApiResult<()> {
    let owner = st.config().owner_id.clone();
    let items = cr.items(STORE).await?;
    let ch = items
        .get(cid)
        .and_then(Value::as_object)
        .ok_or_else(|| ApiError::internal("KeyError", py::quote(cid)))?;
    if turns_of(ch).len() >= MAX_TURNS {
        return Err(http(
            400,
            format!("a conversation holds at most {MAX_TURNS} questions; start a new one"),
        ));
    }
    if turns_of(ch).iter().any(|t| field_v(t, "status") == "thinking") {
        return Err(http(409, "still thinking about the last question"));
    }
    refuse_legal(st, [question])?;
    let c = corpus(st).await?;
    let hist = history(ch);
    let turn = format!("t{}", turns_of(ch).len() + 1);
    let jid = ch.get("joke").and_then(Value::as_str).map(str::to_string);
    let comic = st.config().comic_name.clone();
    let (q2, jid2, c2) = (question.to_string(), jid.clone(), Arc::clone(&c));
    let (ids, msgs) = tokio::task::spawn_blocking(move || {
        let nb = c2.ask_notebook();
        let pin = nb.pinned(jid2.as_deref());
        let ids = nb.retrieve(&q2, &hist, MAX_SOURCES, &pin);
        if ids.is_empty() {
            return Ok((ids, None));
        }
        let scope = nb.scope_line(jid2.as_deref());
        nb.prompt(&q2, &hist, &ids, scope.as_deref(), &material_owner(comic.as_deref()))
            .map(|m| (ids, Some(m)))
    })
    .await
    .map_err(|e| ApiError::internal("JoinError", e))?
    .map_err(|e| ApiError::internal("CreativeError", e.msg()))?;
    let Some(msgs) = msgs else {
        cr.append(
            STORE,
            "ask",
            cid,
            json!({"turn": turn, "q": question, "job": null, "sources": []}),
            &owner,
        )
        .await?;
        let r = serde_json::to_value(refusal(&[])).map_err(|e| ApiError::internal("TypeError", e))?;
        cr.append(STORE, "answer", cid, json!({"turn": turn, "result": r}), AI)
            .await?;
        return Ok(());
    };
    let key = format!("ask_chat|{cid}|{turn}");
    let (_, queued) = enqueue(cr, "ask_chat", &key, json!({"messages": msgs}), false, &owner).await?;
    let job = queued
        .get("job")
        .and_then(|j| j.get("id"))
        .cloned()
        .ok_or_else(|| ApiError::internal("KeyError", "'job'"))?;
    cr.append(
        STORE,
        "ask",
        cid,
        json!({"turn": turn, "q": question, "job": job, "sources": ids}),
        &owner,
    )
    .await?;
    Ok(())
}

/// `joke_scope(cr, kb, body)`: the bit a new conversation is about, when the body names one; it must exist.
async fn joke_scope(st: &AppState, b: &Obj) -> ApiResult<Option<String>> {
    let Some(j) = b.get("joke").filter(|j| !j.is_null()) else {
        return Ok(None);
    };
    let Some(j) = j.as_str().filter(|j| JOKE_RE.is_match(j)) else {
        return Err(http(400, "joke must be a joke id"));
    };
    if !corpus(st).await?.is_joke(j) {
        return Err(http(404, format!("no such joke {j}")));
    }
    Ok(Some(j.to_string()))
}

/// What every route here starts from: the 503 while booting, `?backend=`, and the id's shape (the 404 comes before
/// the body is read).
fn start(st: &AppState, raw: Option<&str>, cid: Option<&str>) -> ApiResult<()> {
    let q = Query::parse(raw);
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    if cid.is_some_and(|c| !ID_RE.is_match(c)) {
        return Err(http(404, "no such conversation"));
    }
    Ok(())
}

/// The conversation as the app reads it, from the rows now.
async fn view(st: &AppState, cr: &Creative, cid: &str) -> ApiResult<Value> {
    let items = cr.items(STORE).await?;
    let ch = items
        .get(cid)
        .and_then(Value::as_object)
        .ok_or_else(|| ApiError::internal("KeyError", py::quote(cid)))?;
    Ok(chat_view(ch, crate::routes::creative::worker_status(st).await?))
}

/// `GET /api/ask/chats`: his conversations, newest first, each settled first.
pub async fn list(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    start(&st, raw.as_deref(), None)?;
    let out = detached(async move {
        let _lock = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let mut c = None;
        for ch in cr.items(STORE).await?.values().filter_map(Value::as_object) {
            if !py::truthy(ch.get("deleted")) {
                settle(&st, &cr, ch, &mut c).await?;
            }
        }
        let mut rows: Vec<(String, String, Value)> = Vec::new();
        for ch in cr.items(STORE).await?.values().filter_map(Value::as_object) {
            if py::truthy(ch.get("deleted")) {
                continue;
            }
            let last = turns_of(ch).last().and_then(Value::as_object);
            let row = json!({
                "id": ch.get("id"), "title": ch.get("title"), "updated_at": ch.get("updated_at"),
                "questions": turns_of(ch).len(), "last_q": last.map_or(Value::Null, |t| t.get("q").cloned().unwrap_or(Value::Null)),
                "thinking": last.is_some_and(|t| field(t, "status") == "thinking"),
            });
            rows.push((field(ch, "updated_at").to_string(), field(ch, "id").to_string(), row));
        }
        // `sort(key=(updated_at, id), reverse=True)`
        rows.sort_by(|a, b| (&b.0, &b.1).cmp(&(&a.0, &a.1)));
        let worker = crate::routes::creative::worker_status(&st).await?;
        Ok(json!({"chats": rows.into_iter().map(|r| r.2).collect::<Vec<_>>(), "worker": worker}))
    })
    .await?;
    Ok(ok(&out))
}

/// `POST /api/ask/chats {question, joke?}`: 201 with the new conversation and its first turn.
pub async fn create(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    start(&st, raw.as_deref(), None)?;
    let b = body_json(&headers, body).await?;
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _lock = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let question = question_of(&b)?;
        let jscope = joke_scope(&st, &b).await?;
        let n = cr.items(STORE).await?.len();
        let cid = format!(
            "chat:{}",
            sha8_value(&json!({"q": question, "at": pk_core::time::now_iso(), "n": n}))
        );
        let title = if py::len(&question) <= 60 {
            question.clone()
        } else {
            format!(
                "{}\u{2026}",
                question
                    .chars()
                    .take(59)
                    .collect::<String>()
                    .trim_end_matches(py::isspace)
            )
        };
        let mut data = json!({"title": title});
        if let Some(j) = jscope {
            data["joke"] = json!(j);
        }
        cr.append(STORE, "new", &cid, data, &owner).await?;
        ask(&st, &cr, &cid, &question).await?;
        view(&st, &cr, &cid).await
    })
    .await?;
    Ok(json_response(StatusCode::CREATED, &out))
}

/// One conversation: `GET` (settled), `POST {question}` (a follow-up), `PATCH {title?, deleted?}`. Any other method
/// on the path is forwarded (the Python answers a `PUT` with its 405, `DELETE` with its 404).
pub async fn one(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    method: Method,
    Path(cid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    start(&st, raw.as_deref(), Some(&cid))?;
    let b = if method == Method::POST || method == Method::PATCH {
        body_json(&headers, body).await?
    } else {
        Obj::new()
    };
    let out = detached(async move {
        let owner = st.config().owner_id.clone();
        let _lock = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let items = cr.items(STORE).await?;
        let ch = items.get(&cid).and_then(Value::as_object);
        let Some(ch) = ch.filter(|ch| !py::truthy(ch.get("deleted")) || method == Method::PATCH) else {
            return Err(http(404, "no such conversation"));
        };
        if method == Method::GET {
            settle(&st, &cr, ch, &mut None).await?;
            return view(&st, &cr, &cid).await;
        }
        if method == Method::POST {
            let question = question_of(&b)?;
            ask(&st, &cr, &cid, &question).await?;
            return view(&st, &cr, &cid).await;
        }
        if method == Method::PATCH {
            let d: Obj = ["title", "deleted"]
                .iter()
                .filter_map(|k| b.get(*k).map(|v| ((*k).to_string(), v.clone())))
                .collect();
            if d.is_empty() {
                return Err(http(400, "nothing to change (title, deleted)"));
            }
            let ops = [("patch".to_string(), cid.clone(), Value::Object(d))];
            cr.probe(STORE, &ops, &owner).await?;
            let [(op, id, data)] = ops;
            cr.append(STORE, &op, &id, data, &owner).await?;
            return view(&st, &cr, &cid).await;
        }
        Err(http(405, "method not allowed"))
    })
    .await?;
    Ok(ok(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_follow_the_pythons_patterns() {
        assert!(ID_RE.is_match("chat:0123abcd"));
        assert!(ID_RE.is_match("chat:0123abcd\n"));
        assert!(!ID_RE.is_match("chat:0123abc"));
        assert!(!ID_RE.is_match("chat:0123ABCD"));
        assert!(JOKE_RE.is_match("joke:a.b_c:d-1"));
        assert!(!JOKE_RE.is_match("joke:"));
        assert!(!JOKE_RE.is_match("unit:a"));
    }

    #[test]
    fn a_failure_is_said_in_plain_words() {
        assert_eq!(
            plain_error("Refusal: Claude declined this one."),
            "Claude declined this one."
        );
        assert_eq!(
            plain_error("CutShort: Claude ran out of room on that one."),
            "Claude ran out of room on that one."
        );
        assert_eq!(plain_error("claude declined this one"), "claude declined this one");
        assert!(plain_error("429 rate limit hit").starts_with("Claude is at its usage limit"));
        assert_eq!(
            plain_error("read timed out"),
            "Your PC took too long to answer. Ask again."
        );
        assert_eq!(plain_error(""), "Your PC couldn't answer that one. Ask again.");
    }

    #[test]
    fn the_question_folds_its_spaces_and_is_bounded() {
        let b = |q: Value| json!({ "question": q }).as_object().cloned().unwrap();
        assert_eq!(
            question_of(&b(json!("  what   did\tI write\n"))).unwrap(),
            "what did I write"
        );
        assert!(question_of(&b(json!("   "))).is_err());
        assert!(question_of(&b(json!(5))).is_err());
        assert!(question_of(&b(json!("x".repeat(1001)))).is_err());
        assert!(question_of(&b(json!("x".repeat(1000)))).is_ok());
    }
}
