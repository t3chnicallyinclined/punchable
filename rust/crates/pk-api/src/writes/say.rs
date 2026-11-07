//! `POST /api/say` (`api-dev/say_api.py` `say`, `run_command`): one input on every screen; the app does not ask him
//! which it is. The classifier and the command parser are `pk_creative::say`; this is the flow around them.
//!
//! Provenance (the review rule): the text is his, the classification is the app's. The guess is an `intents` store
//! event by `ai:intent`; when the app sends `kind` (his "Not a question? Save as a bit" tap) that is an `override`
//! event by him on top of the guess. The question, the idea, the set list are written by him, as before. A
//! question opens an Ask conversation (a job for the hosted worker), a bit is a jot (`POST /api/ideas`'s flow), a
//! command makes a set list or puts a bit on tonight's.
//!
//! The Python holds one lock for the whole request; here the creative lock is taken around each write of this
//! route only, because the idea flow takes it itself.

use axum::Extension;
use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_creative::merge::sha8_value;
use pk_creative::say::{classify, match_bits, parse_command};
use pk_creative::util::slug;
use serde_json::{Map, Value, json};

use super::{Creative, Req, actor, detached, http, request};
use crate::auth::Caller;
use crate::error::{ApiResult, json_response};
use crate::state::AppState;

type Obj = Map<String, Value>;

const STORE: &str = "intents";
const AI_ACTOR: &str = "ai:intent";
const KINDS: [&str; 3] = ["question", "bit", "command"];
const MAX_TEXT: usize = 4000;

/// `_text(b)`: the text, its whitespace folded.
fn text_of(b: &Obj) -> ApiResult<String> {
    b.get("text")
        .and_then(Value::as_str)
        .filter(|t| !py::strip(t).is_empty() && py::len(t) <= MAX_TEXT)
        .map(|t| py::split(t).collect::<Vec<_>>().join(" "))
        .ok_or_else(|| http(400, format!("text is required (at most {MAX_TEXT} characters)")))
}

/// `_context(b)`: where he said it, the four keys that are short texts.
fn context_of(b: &Obj) -> ApiResult<Option<Obj>> {
    let Some(c) = b.get("context").filter(|c| !c.is_null()) else {
        return Ok(None);
    };
    let Some(c) = c.as_object() else {
        return Err(http(400, "context must be an object"));
    };
    let out: Obj = ["set_id", "joke_id", "board_id", "screen"]
        .iter()
        .filter_map(|k| {
            let v = c.get(*k)?;
            (v.as_str().is_some_and(|s| py::len(s) <= 300)).then(|| ((*k).to_string(), v.clone()))
        })
        .collect();
    Ok((!out.is_empty()).then_some(out))
}

fn board_brief(b: &Value) -> Value {
    json!({"id": b["id"], "title": b["title"], "target_s": b.get("target_s").cloned().unwrap_or(Value::Null),
           "n": b.get("joke_ids").and_then(Value::as_array).map_or(0, Vec::len)})
}

/// `POST /api/say {text, kind?, context?, choice?}`.
pub async fn say(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q, b } = request(st, raw.as_deref(), &headers, body).await?;
    let text = text_of(&b)?;
    let ctx = context_of(&b)?;
    let want = match b.get("kind").filter(|k| !k.is_null()) {
        None => None,
        Some(k) => Some(
            k.as_str()
                .filter(|k| KINDS.contains(k))
                .ok_or_else(|| http(400, "kind must be question, bit or command"))?
                .to_string(),
        ),
    };
    let weekday = chrono::Local::now().format("%A").to_string();
    let (guess, rule) = classify(&text);
    let cmd = if guess == "command" || want.as_deref() == Some("command") {
        parse_command(&text, &weekday)
    } else {
        None
    };
    if want.as_deref() == Some("command") && cmd.is_none() {
        return Err(http(
            400,
            "not a command I know (try \"new 7 for friday\" or \"add <bit> to tonight\")",
        ));
    }
    let by = actor(&st, &caller, &b)?;
    if by != st.config().owner_id {
        return Err(http(403, "say is the comic's own input"));
    }
    let hidden = q.include_hidden();
    let (out, status) = detached(async move {
        let cr = Creative::new(st.clone());
        let ai = |op: &str, id: &str, data: Value, who: String| {
            let (st, op, id) = (st.clone(), op.to_string(), id.to_string());
            async move {
                let _cr = st.creative_lock().lock().await;
                Creative::new(st.clone()).append(STORE, &op, &id, data, &who).await
            }
        };
        let n_events = {
            let _cr = st.creative_lock().lock().await;
            cr.lines_and_items(STORE).await?.0.len()
        };
        let iid = format!(
            "say:{}",
            sha8_value(&json!({"t": text, "at": pk_core::time::now_iso(), "n": n_events}))
        );
        ai(
            "classify",
            &iid,
            json!({"text": text, "kind": guess, "rule": rule, "context": ctx, "command": cmd}),
            AI_ACTOR.to_string(),
        )
        .await?;
        let mut kind = guess.to_string();
        if let Some(w) = want.as_deref().filter(|w| *w != guess) {
            ai("override", &iid, json!({"kind": w}), by.clone()).await?;
            kind = w.to_string();
        }
        let mut out = json!({"kind": kind, "intent": iid, "guess": guess, "rule": rule, "overridden": kind != guess,
                             "text": text, "worker": crate::routes::creative::worker_status(&st).await?});
        if kind == "command" {
            let cmd = cmd.clone().unwrap_or(Value::Null);
            out["command"] = run_command(&st, &iid, &cmd, &b, &by, &weekday).await?;
            return Ok((out, StatusCode::OK));
        }
        if kind == "question" {
            let chat = super::chats::say_question(&st, &text, ctx.as_ref(), &by).await?;
            ai("acted", &iid, json!({"ref": chat["id"]}), by.clone()).await?;
            out["chat"] = chat;
            return Ok((out, StatusCode::CREATED));
        }
        let screen = ctx.as_ref().map(|c| {
            c.get("screen")
                .filter(|s| py::truthy(Some(s)))
                .and_then(Value::as_str)
                .unwrap_or("the input bar")
        });
        let reason = screen.map_or_else(|| "said on the input bar".to_string(), |s| format!("said on {s}"));
        let mut ib = Obj::new();
        ib.insert("text".into(), json!(text));
        ib.insert("reason".into(), json!(reason));
        let idea = super::ideas::idea_result(&st, hidden, &ib, &by).await?;
        ai("acted", &iid, json!({"ref": idea["id"]}), by.clone()).await?;
        let att = idea.get("attach").cloned().unwrap_or(Value::Null);
        let attached = att.get("decision").and_then(Value::as_str) == Some("attach");
        out["saved_to"] = if attached {
            json!({"joke_id": att["into"], "title": att.get("title").cloned().unwrap_or(Value::Null)})
        } else {
            Value::Null
        };
        out["undo"] = json!({"POST": "/api/undo", "indexes": idea["indexes"]});
        if attached && py::truthy(att.get("id")) {
            out["undo"]["detach"] = json!({"POST": "/api/merge/undo", "merge_id": att["id"]});
        }
        out["idea"] = idea;
        Ok((out, StatusCode::OK))
    })
    .await?;
    Ok(json_response(status, &out))
}

/// `run_command(cr, kb, st, iid, cmd, b, by)`: execute (or ask about) a parsed command. Every write here is his: after
/// his tap (`choice`, the bit he picked when the app asked), or at once when the command is unambiguous and the
/// screen shows Undo.
async fn run_command(st: &AppState, iid: &str, cmd: &Value, b: &Obj, by: &str, weekday: &str) -> ApiResult<Value> {
    let (c, cr, items) = super::boards::start(st, "storyboards").await?;
    let mut out = cmd.as_object().cloned().unwrap_or_default();
    let _lock = st.creative_lock().lock().await;
    let acted = |data: Value| {
        let (st, iid, by) = (st.clone(), iid.to_string(), by.to_string());
        async move { Creative::new(st).append(STORE, "acted", &iid, data, &by).await }
    };
    if cmd["op"] == "new_set" {
        let title = cmd["title"].as_str().unwrap_or("");
        let secs = cmd["minutes"].as_i64().unwrap_or(0) * 60;
        let bid = format!(
            "sb:{}-{}",
            slug(title, 32),
            sha8_value(&json!([title, "", items.len()]))
        );
        let rec = cr
            .append(
                "storyboards",
                "create",
                &bid,
                json!({"title": title, "joke_ids": [], "target_s": secs, "notes": "", "seed": null}),
                by,
            )
            .await?;
        acted(json!({"ref": rec["id"]})).await?;
        out.insert("done".into(), json!(true));
        out.insert("board".into(), board_brief(&rec));
        out.insert(
            "undo".into(),
            json!({"PATCH": format!("/api/boards/{}", rec["id"].as_str().unwrap_or("")), "body": {"delete": true}}),
        );
        return Ok(Value::Object(out));
    }
    let board = crate::setlists::tonight_board(&c, &items);
    let mut bid = board.get("id").and_then(Value::as_str).map(str::to_string);
    let choice = b.get("choice").filter(|c| !c.is_null());
    if let Some(ch) = choice
        && !ch.as_str().is_some_and(|j| c.is_joke(j))
    {
        return Err(http(400, "choice must be one of your bits"));
    }
    let words: Vec<String> = cmd["words"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| w.as_str().map(str::to_string))
        .collect();
    let cands = match_bits(
        &c,
        &words,
        cmd["query"].as_str().unwrap_or(""),
        pk_creative::say::MAX_CANDIDATES,
    );
    out.insert("candidates".into(), json!(cands));
    out.insert(
        "board".into(),
        if board.is_null() {
            Value::Null
        } else {
            json!({"id": board["id"], "title": board["title"], "target_s": board["target_s"],
                   "n": board["segments"].as_array().map_or(0, Vec::len)})
        },
    );
    let jid = choice.and_then(Value::as_str).map(str::to_string).or_else(|| {
        cands
            .first()
            .filter(|f| f.get("sure") == Some(&json!(true)))
            .and_then(|f| f["joke_id"].as_str().map(str::to_string))
    });
    let Some(jid) = jid else {
        out.insert("done".into(), json!(false));
        out.insert("ask".into(), json!(if cands.is_empty() { "none" } else { "which" }));
        return Ok(Value::Object(out));
    };
    let title = c.title(&jid);
    let pick = cands
        .iter()
        .find(|x| x["joke_id"] == jid.as_str())
        .cloned()
        .unwrap_or_else(|| {
            json!({"joke_id": jid, "title": pk_creative::ask::words::short_title(Some(&title)),
               "short_name": pk_creative::display::short_name_of(
                   &title, c.joke(&jid).and_then(|n| n.get("short_name")).and_then(Value::as_str)),
               "score": null})
        });
    let mut created = false;
    let board_now: Value;
    if let Some(id) = bid.clone() {
        let bb = items.get(&id).cloned().unwrap_or(Value::Null);
        let have: Vec<&str> = bb["joke_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if let Some(pos) = have.iter().position(|j| *j == jid) {
            acted(json!({"ref": id, "joke": jid, "already": true})).await?;
            out.insert("done".into(), json!(true));
            out.insert("already".into(), json!(true));
            out.insert("joke".into(), pick);
            out.insert("board".into(), board_brief(&bb));
            out.insert("position".into(), json!(pos));
            return Ok(Value::Object(out));
        }
        board_now = cr
            .append("storyboards", "add", &id, json!({"joke_id": jid, "at": null}), by)
            .await?;
    } else {
        let t = format!("{weekday} \u{b7} tonight");
        let id = format!("sb:{}-{}", slug(&t, 32), sha8_value(&json!([t, "", items.len()])));
        board_now = cr
            .append(
                "storyboards",
                "create",
                &id,
                json!({"title": t, "joke_ids": [jid], "target_s": 7 * 60, "notes": "", "seed": null}),
                by,
            )
            .await?;
        bid = Some(id);
        created = true;
    }
    let id = bid.unwrap_or_default();
    acted(json!({"ref": id, "joke": jid})).await?;
    let pos = board_now["joke_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .position(|j| j == jid.as_str())
        .unwrap_or(0);
    out.insert("done".into(), json!(true));
    out.insert("created".into(), json!(created));
    out.insert("joke".into(), pick);
    out.insert("board".into(), board_brief(&board_now));
    out.insert("position".into(), json!(pos));
    out.insert(
        "undo".into(),
        json!({"PATCH": format!("/api/boards/{id}"),
               "body": if created { json!({"delete": true}) } else { json!({"remove": {"joke_id": jid}}) }}),
    );
    Ok(Value::Object(out))
}
