//! The Rant tab (`docs/design/SPEC-2026-11-06-rant-tab.md` §2, §3, §7; slice 1). Rust only (R7: new server work is
//! written once, here; the Python never answers these paths and has no twin of them):
//!
//! - `GET /api/topics`: his topics, newest first (the `topics` creative store, `pk_creative::reducers::topics`).
//! - `POST /api/topics {text}`: a typed topic; the same words twice (case and spacing aside) answer the one he has.
//! - `PATCH /api/topics/<id>`: `{ranted: <joke id>}` the rant it became; `{delete: true}`, `{undelete: true}` (a
//!   tombstone: Undo brings its material back); `{rename: text}`; `{keep | drop | unmark: ref}` a bit or a moment on this
//!   topic, never a change to it; `{jot: ref}` a typed idea he wrote here; `{script: {unit, lines}}` where each line of
//!   that version came from (§12.7, the `script` store).
//! - `POST /api/rant/line {joke?, lines, at, ask?, say?, fresh?}`: Claude on one line of the script (a `rant_line` job);
//!   nothing changes until he picks one.
//! - `POST /api/rant/draft {topic, topic_id?, sources?: [id], keep?: [line], fresh?}`: queue a `rant_draft` job
//!   (pk-worker) over his words for the sources, read here from the KB (a bit's newest version, else its idea; a taped
//!   moment's text), never text the phone sends; `sources` left out on a topic = its Kept, else its Found (§12.3);
//!   `keep` = the script's lines so far, fixed: the worker refuses a result that does not bring each one back word for
//!   word, in order. The legal rules run on all of it first. 202 with the job, or 200 with a cached result
//!   (`jobs::enqueue`).
//! - `GET /api/rants`: the bits that have a rant cut (a version whose source is `rant` or `rant_*`, `film::cut_of`),
//!   newest rant first, for the tab's Drafts (the phone splits Finished off with its own filmed takes).

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_creative::reducers::script::{MAX_SCRIPT_LINES, clean_mark};
use pk_creative::reducers::topics::{MAX_TOPIC_CHARS, clean_topic, is_deleted, is_material_ref, is_topic_id};
use pk_kb::View;
use serde_json::{Map, Value, json};
use sha1::{Digest as _, Sha1};

use super::{Creative, Kb, Pre, Req, Written, actor, detached, http, is_id, refuse_legal, request};
use crate::auth::Caller;
use crate::display::short_name_of;
use crate::error::{ApiResult, json_response, ok};
use crate::query::Query;
use crate::routes::creative::items_of;
use crate::state::AppState;

/// The store.
pub const STORE: &str = "topics";
/// The store of where each line came from.
pub const SCRIPT_STORE: &str = "script";
/// The job kind.
pub const KIND: &str = "rant_draft";
/// The job kind of Claude on one line.
pub const LINE_KIND: &str = "rant_line";
/// What he can ask of a line (§12.5); `say` is his own words on top or instead.
pub const LINE_ASKS: [&str; 5] = ["sharper", "shorter", "angrier", "bigger", "tag"];
/// Lines either side of the one he asked about that go along as context.
const LINE_NEIGHBOURS: usize = 3;
/// What he types for Claude, at most (code points).
const MAX_SAY_CHARS: usize = 500;
/// One line of a script sent here, at most (code points).
const MAX_SCRIPT_LINE_CHARS: usize = 2000;
/// Lines a script sent here may have, at most.
const MAX_KEEP_LINES: usize = MAX_SCRIPT_LINES;
/// Material on one topic, at most (§12.7; the route's cap, never the reducer's).
pub const MAX_MATERIAL: usize = 500;
/// Sources one draft may draw on, at most (`pk_worker::kinds::rant_draft::MAX_SOURCES`).
pub const MAX_SOURCES: usize = 12;
/// Topics kept, at most: a list he reads on one screen, not an archive. The cap is the route's, never the reducer's
/// (a lower cap there later would refuse the history already written).
pub const MAX_TOPICS: usize = 200;

fn hex_of(seed: &str, n: usize) -> String {
    hex::encode(Sha1::digest(seed.as_bytes()))[..n].to_string()
}

/// The live topics, last touched first (a tie: the newer one first).
fn topics_touched_first(items: &pk_creative::Items) -> Vec<Value> {
    let mut v: Vec<Value> = items.values().filter(|t| !is_deleted(t)).cloned().collect();
    v.reverse();
    let touched = |t: &Value| -> String {
        t.get("touched")
            .or_else(|| t.get("created"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    v.sort_by_cached_key(|t| std::cmp::Reverse(touched(t)));
    v
}

/// `GET /api/topics`.
pub async fn topics(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_d, view) = crate::routes::prepare_view(&st, &q).await?;
    let items = items_of(&st, |s| &s.topics).await?;
    let script = items_of(&st, |s| &s.script).await?;
    let rows: Vec<Value> = topics_touched_first(&items)
        .iter()
        .map(|t| super::topic::topic_row(&st, &view, t, &script))
        .collect();
    Ok(ok(&json!({"topics": rows})))
}

/// The same topic, for "the one he has": case and spacing aside.
fn same_topic(a: &str, b: &str) -> bool {
    clean_topic(a).to_lowercase() == clean_topic(b).to_lowercase()
}

/// `POST /api/topics {text, from_trend?}`.
pub async fn add_topic(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let text = clean_topic(b.get("text").and_then(Value::as_str).unwrap_or(""));
    if text.is_empty() {
        return Err(http(400, "text is required (the topic)"));
    }
    if py::len(&text) > MAX_TOPIC_CHARS {
        return Err(http(400, format!("a topic is at most {MAX_TOPIC_CHARS} characters")));
    }
    refuse_legal(&st, [text.as_str()])?;
    let by = actor(&st, &caller, &Map::new())?;
    let out = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let lines = cr.lines(STORE).await?;
        let id = format!(
            "topic:{}",
            hex_of(&format!("{text}|{}|{}", lines.len(), pk_core::time::now_iso()), 12)
        );
        let t = text.clone();
        let have = move |items: &pk_creative::Items| -> ApiResult<Pre> {
            Ok(
                match items
                    .values()
                    .filter(|x| !is_deleted(x))
                    .find(|x| x.get("text").and_then(Value::as_str).is_some_and(|s| same_topic(s, &t)))
                {
                    Some(x) => Pre::Done(StatusCode::OK, x.clone()),
                    None if items.values().filter(|x| !is_deleted(x)).count() >= MAX_TOPICS => {
                        return Err(http(400, format!("at most {MAX_TOPICS} topics: delete one first")));
                    }
                    None => Pre::Go,
                },
            )
        };
        let topic = match cr
            .append_if(STORE, "add", &id, json!({"text": text, "from_trend": null}), &by, have)
            .await?
        {
            Written::Landed(v) | Written::Skipped(_, v) => v,
        };
        Ok(json!({"ok": true, "topic": topic}))
    })
    .await?;
    Ok(ok(&out))
}

/// What a PATCH body asks: exactly one field.
fn patch_op(b: &Map<String, Value>) -> ApiResult<(&'static str, Value)> {
    const BAD: &str =
        "send one of {ranted}, {delete: true}, {undelete: true}, {rename}, {keep}, {drop}, {unmark}, {jot}, {script}";
    if b.len() != 1 {
        return Err(http(400, BAD));
    }
    let bad_ref = || http(400, "material is a bit or a taped moment id");
    Ok(match b.iter().next() {
        Some((k, Value::String(j))) if k == "ranted" && j.starts_with("joke:") && is_id(j) => {
            ("ranted", json!({"joke": j}))
        }
        Some((k, Value::Bool(true))) if k == "delete" => ("delete", json!({})),
        Some((k, Value::Bool(true))) if k == "undelete" => ("undelete", json!({})),
        Some((k, Value::String(t))) if k == "rename" => {
            let text = clean_topic(t);
            if text.is_empty() {
                return Err(http(400, "text is required (the topic)"));
            }
            if py::len(&text) > MAX_TOPIC_CHARS {
                return Err(http(400, format!("a topic is at most {MAX_TOPIC_CHARS} characters")));
            }
            ("rename", json!({"text": text}))
        }
        Some((k, v)) if matches!(k.as_str(), "keep" | "drop" | "unmark" | "jot") => {
            let r = v
                .as_str()
                .filter(|r| is_id(r) && is_material_ref(r))
                .ok_or_else(bad_ref)?;
            if k == "jot" && !r.starts_with("joke:") {
                return Err(bad_ref());
            }
            let op = match k.as_str() {
                "keep" => "keep",
                "drop" => "drop",
                "unmark" => "unmark",
                _ => "jot",
            };
            (op, json!({"ref": r}))
        }
        Some((k, v)) if k == "script" => {
            let unit = v
                .get("unit")
                .and_then(Value::as_str)
                .filter(|u| u.starts_with("unit:") && is_id(u));
            let lines = v
                .get("lines")
                .and_then(Value::as_array)
                .filter(|l| l.len() <= MAX_SCRIPT_LINES);
            let (Some(unit), Some(lines)) = (unit, lines) else {
                return Err(http(
                    400,
                    format!("script needs its unit and at most {MAX_SCRIPT_LINES} lines"),
                ));
            };
            let marks = lines
                .iter()
                .map(clean_mark)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| super::creative_err(&e))?;
            ("script", json!({"unit": unit, "lines": marks}))
        }
        _ => return Err(http(400, BAD)),
    })
}

/// `PATCH /api/topics/<id>`: see the module's list.
pub async fn patch_topic(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(tid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    if !is_topic_id(&tid) {
        return Err(http(400, "bad topic id"));
    }
    let (op, data) = patch_op(&b)?;
    if op == "rename" {
        refuse_legal(&st, [data["text"].as_str().unwrap_or("")])?;
    }
    // what a mark or a script names must be there now; an unmark must not, so a bit since merged away can be put back
    let mut data = data;
    if matches!(op, "keep" | "drop" | "jot" | "script") {
        let (_, view) = {
            let _kb = st.kb_lock().lock().await;
            Kb::new(st.clone(), q_hidden(raw.as_deref())).view().await?
        };
        if op == "script" {
            let unit = data["unit"].as_str().unwrap_or("").to_string();
            if !view.unit_joke.contains_key(&unit) {
                return Err(http(404, format!("no version {unit}")));
            }
            let text = view
                .node(&unit)
                .and_then(|n| n.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let lines = super::film::cut_lines(text);
            if let Some(marks) = data["lines"].as_array_mut() {
                verify_claims(&view, &lines, marks);
            }
        } else {
            let r = data["ref"].as_str().unwrap_or("");
            if super::topic::mat_of(&view, r, None).is_none() {
                return Err(http(404, format!("{r} is not a bit or a taped moment with words")));
            }
        }
    }
    let by = actor(&st, &caller, &Map::new())?;
    let out = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let id = tid.clone();
        let adds = matches!(op, "keep" | "drop" | "jot").then(|| data["ref"].as_str().unwrap_or("").to_string());
        let exists = move |items: &pk_creative::Items| -> ApiResult<Pre> {
            let t = items.get(&id).filter(|t| is_deleted(t) == (op == "undelete"));
            let Some(t) = t else {
                return Err(http(404, format!("no topic {id}")));
            };
            if op == "undelete" && items.values().filter(|x| !is_deleted(x)).count() >= MAX_TOPICS {
                return Err(http(400, format!("at most {MAX_TOPICS} topics: delete one first")));
            }
            if let Some(r) = &adds {
                let m = t.get("material").and_then(Value::as_object);
                if m.is_some_and(|m| m.len() >= MAX_MATERIAL && !m.contains_key(r)) {
                    return Err(http(
                        400,
                        format!("at most {MAX_MATERIAL} on one topic: drop one first"),
                    ));
                }
            }
            Ok(Pre::Go)
        };
        // a script's marks: the topic is touched first (its check refuses a missing or deleted topic before anything
        // is written), then the marks land on their own store
        let script = (op == "script").then(|| (data["unit"].as_str().unwrap_or("").to_string(), data["lines"].clone()));
        let (op2, data2) = if script.is_some() {
            ("touch", json!({}))
        } else {
            (op, data)
        };
        let topic = match cr.append_if(STORE, op2, &tid, data2, &by, exists).await? {
            Written::Landed(v) | Written::Skipped(_, v) => v,
        };
        if let Some((unit, lines)) = script {
            cr.append(SCRIPT_STORE, "set", &unit, json!({"lines": lines}), &by)
                .await?;
        }
        let mut topic = topic;
        if let Some(t) = topic.as_object_mut() {
            t.remove("deleted_at");
        }
        Ok(json!({"ok": true, "id": tid, "topic": if op == "delete" { Value::Null } else { topic }}))
    })
    .await?;
    Ok(ok(&out))
}

/// The verbatim gate on a script's marks: a line marked as his own words from a bit (`by: him`, a `from`, not
/// `edited`) keeps that claim only when the gate finds the line in the source's words now; otherwise the claim goes
/// (the line then shows nothing, never a source it is not in). Marks that do not line up with the version's lines all
/// lose their claims for the same reason.
fn verify_claims(view: &View, lines: &[String], marks: &mut [Value]) {
    let aligned = marks.len() == lines.len();
    for (i, m) in marks.iter_mut().enumerate() {
        let Some(o) = m.as_object_mut() else { continue };
        let Some(from) = o.get("from").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if o.get("edited") == Some(&Value::Bool(true)) {
            continue;
        }
        let words = super::topic::mat_of(view, &from, None)
            .and_then(|m| m.get("words").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        let ok = aligned && !words.is_empty() && pk_creative::verbatim::verbatim(&lines[i], &words).is_some();
        if !ok {
            o.remove("from");
            o.remove("label");
        }
    }
}

/// `?include_hidden` of a raw query string.
fn q_hidden(raw: Option<&str>) -> bool {
    Query::parse(raw).include_hidden()
}

/// A bit's versions now (a `part_of` edge that is not rejected, the unit not hidden), oldest first by date, a day's
/// versions in the order they were added (the app's stepper: rules/bitLines).
pub(super) fn versions_of<'a>(view: &'a View, jid: &str) -> Vec<(&'a str, &'a pk_kb::Rec)> {
    let mut vs: Vec<(&str, &pk_kb::Rec)> = view
        .joke_units
        .get(jid)
        .into_iter()
        .flatten()
        .filter(|(u, _)| view.unit_joke.get(u).is_some_and(|j| j == jid))
        .filter_map(|(u, _)| view.node(u).map(|n| (u.as_str(), n)))
        .filter(|(_, n)| !py::truthy(n.get("hidden")))
        .collect();
    vs.sort_by(|a, b| {
        str_field(a.1, "date")
            .unwrap_or("")
            .cmp(str_field(b.1, "date").unwrap_or(""))
    });
    vs
}

pub(super) fn str_field<'a>(n: &'a pk_kb::Rec, k: &str) -> Option<&'a str> {
    n.get(k).and_then(Value::as_str)
}

/// One source's words and its label, from the KB: a bit's newest version (else its typed idea), a taped moment's
/// text. `None` for an id that is neither, or one with no words.
fn source_of(view: &View, id: &str) -> Option<Value> {
    match view.ntype(Some(id))? {
        "joke" => {
            let j = view.node(id)?;
            let title = str_field(j, "title").unwrap_or("");
            let short = short_name_of(title, str_field(j, "short_name"));
            // what the bit says now: its newest version (the stepper's order: by date, a day's in the order they were
            // added; `search::newest_text` keeps the FIRST of a day's, so a version saved this morning would hide one
            // saved this afternoon), else the typed idea
            let vs = versions_of(view, id);
            let newest = vs
                .iter()
                .rev()
                .filter_map(|(_, n)| str_field(n, "text"))
                .find(|t| !py::strip(t).is_empty());
            if let Some(text) = newest {
                return Some(json!({"id": id, "kind": "bit", "label": format!("{short} v{}", vs.len()), "text": text}));
            }
            let idea = crate::search::newest_text(view, id);
            (!py::strip(&idea).is_empty()).then(|| json!({"id": id, "kind": "idea", "label": short, "text": idea}))
        }
        "performance" => {
            let p = view.node(id)?;
            str_field(p, "text")
                .filter(|t| !py::strip(t).is_empty())
                .map(|text| json!({"id": id, "kind": "moment", "label": "On tape", "text": text}))
        }
        _ => None,
    }
}

/// The script's lines a request sends (`keep`, `lines`): one line each, not empty, within the limits.
fn script_lines(v: Option<&Value>, what: &str) -> ApiResult<Vec<String>> {
    let bad = || {
        http(
            400,
            format!(
                "{what} must be a list of at most {MAX_KEEP_LINES} lines of at most {MAX_SCRIPT_LINE_CHARS} characters"
            ),
        )
    };
    let xs = match v {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(xs)) if xs.len() <= MAX_KEEP_LINES => xs,
        Some(_) => return Err(bad()),
    };
    xs.iter()
        .map(|x| {
            let t = py::strip(x.as_str().ok_or_else(bad)?);
            if t.is_empty() || t.contains('\n') || py::len(t) > MAX_SCRIPT_LINE_CHARS {
                return Err(bad());
            }
            Ok(t.to_string())
        })
        .collect()
}

/// `POST /api/rant/line {joke?, lines, at, ask?, say?, fresh?}`.
pub async fn line(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q: _, b } = request(st, raw.as_deref(), &headers, body).await?;
    let lines = script_lines(b.get("lines"), "lines")?;
    let at = super::int_arg(b.get("at"), "at", 0, i64::try_from(MAX_KEEP_LINES).unwrap_or(i64::MAX))?;
    let Some(at) = at.and_then(|a| usize::try_from(a).ok()).filter(|a| *a < lines.len()) else {
        return Err(http(400, "at must be the place of a line in lines"));
    };
    let ask = match b.get("ask") {
        None | Some(Value::Null) => None,
        Some(Value::String(a)) if LINE_ASKS.contains(&a.as_str()) => Some(a.clone()),
        Some(_) => return Err(http(400, format!("ask must be one of {}", LINE_ASKS.join(", ")))),
    };
    let say = py::strip(b.get("say").and_then(Value::as_str).unwrap_or("")).to_string();
    if py::len(&say) > MAX_SAY_CHARS {
        return Err(http(400, format!("say at most {MAX_SAY_CHARS} characters")));
    }
    if ask.is_none() && say.is_empty() {
        return Err(http(400, "ask or say is required (what he wants from the line)"));
    }
    let joke = match b.get("joke") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(j)) if j.starts_with("joke:") && is_id(j) => json!(j),
        Some(_) => return Err(http(400, "bad joke id")),
    };
    refuse_legal(
        &st,
        lines.iter().map(String::as_str).chain(std::iter::once(say.as_str())),
    )?;
    let before = lines[at.saturating_sub(LINE_NEIGHBOURS)..at].to_vec();
    let after: Vec<String> = lines[at + 1..].iter().take(LINE_NEIGHBOURS).cloned().collect();
    let key = format!(
        "{LINE_KIND}|{}",
        hex_of(
            &format!(
                "{}|{}|{}|{}|{}",
                lines[at],
                ask.as_deref().unwrap_or(""),
                say,
                before.join("\n"),
                after.join("\n")
            ),
            16
        )
    );
    let input = json!({"joke": joke, "line": lines[at], "before": before, "after": after, "ask": ask, "say": say});
    let fresh = py::truthy(b.get("fresh"));
    let owner = st.config().owner_id.clone();
    let (s, out) = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        super::jobs::enqueue(&cr, LINE_KIND, &key, input, fresh, &owner).await
    })
    .await?;
    Ok(json_response(s, &out))
}

/// `POST /api/rant/draft {topic, topic_id?, sources?: [id], keep?: [line], fresh?}`.
pub async fn draft(
    State(st): State<AppState>,
    Extension(_caller): Extension<Caller>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q, b } = request(st, raw.as_deref(), &headers, body).await?;
    let topic = clean_topic(b.get("topic").and_then(Value::as_str).unwrap_or(""));
    if topic.is_empty() {
        return Err(http(400, "topic is required (what the rant is about)"));
    }
    if py::len(&topic) > MAX_TOPIC_CHARS {
        return Err(http(400, format!("a topic is at most {MAX_TOPIC_CHARS} characters")));
    }
    let topic_id = match b.get("topic_id") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(t)) if is_topic_id(t) => json!(t),
        Some(_) => return Err(http(400, "bad topic id")),
    };
    let bad = || {
        http(
            400,
            format!("sources must be a list of at most {MAX_SOURCES} bit or moment ids"),
        )
    };
    let keep = script_lines(b.get("keep"), "keep")?;
    let named = !matches!(b.get("sources"), None | Some(Value::Null));
    let ids: Vec<String> = match b.get("sources") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(xs)) if xs.len() <= MAX_SOURCES => {
            let mut out: Vec<String> = Vec::with_capacity(xs.len());
            for x in xs {
                match x.as_str().filter(|s| is_id(s)) {
                    Some(s) if !out.iter().any(|o| o == s) => out.push(s.to_string()),
                    Some(_) => {}
                    None => return Err(bad()),
                }
            }
            out
        }
        Some(_) => return Err(bad()),
    };
    let (_, view) = {
        let _kb = st.kb_lock().lock().await;
        Kb::new(st.clone(), q.include_hidden()).view().await?
    };
    // nothing named, on a topic: what is kept there, else what it found (§12.3); a source with no words now is left out
    let ids = if !named && topic_id.is_string() {
        let items = items_of(&st, |s| &s.topics).await?;
        let tid = topic_id.as_str().unwrap_or("");
        let t = items
            .get(tid)
            .filter(|t| !is_deleted(t))
            .ok_or_else(|| http(404, format!("no topic {tid}")))?;
        let mut d = super::topic::default_sources(&st, &view, t, tid, q.include_hidden()).await?;
        d.truncate(MAX_SOURCES);
        d
    } else {
        ids
    };
    let mut sources = Vec::with_capacity(ids.len());
    for id in &ids {
        match source_of(&view, id) {
            Some(s) => sources.push(s),
            None => return Err(http(404, format!("{id} is not a bit or a taped moment with words"))),
        }
    }
    refuse_legal(
        &st,
        std::iter::once(topic.as_str())
            .chain(sources.iter().filter_map(|s| s["text"].as_str()))
            .chain(keep.iter().map(String::as_str)),
    )?;
    // the same topic over the same words is the same draft, whatever order he ticked them in; a new version of a
    // ticked bit changes its text, so it is a fresh draft
    let mut keyed: Vec<String> = sources.iter().map(|s| format!("{} {}", s["id"], s["text"])).collect();
    keyed.sort_unstable();
    let key = format!(
        "{KIND}|{}",
        hex_of(
            &format!("{}|{}|{}", topic.to_lowercase(), keyed.join("|"), keep.join("\n")),
            16
        )
    );
    let input = json!({"topic": topic, "topic_id": topic_id, "sources": sources, "keep": keep});
    let fresh = py::truthy(b.get("fresh"));
    let owner = st.config().owner_id.clone();
    let (s, out) = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        super::jobs::enqueue(&cr, KIND, &key, input, fresh, &owner).await
    })
    .await?;
    Ok(json_response(s, &out))
}

/// One row of the tab's Drafts: a bit with a rant cut.
fn rant_row(view: &View, jid: &str) -> Option<(String, usize, Value)> {
    let vs = versions_of(view, jid);
    let rants: Vec<(usize, &pk_kb::Rec)> = vs
        .iter()
        .enumerate()
        .filter(|(_, (_, n))| super::film::cut_of(n.get("source")) == "rant")
        .map(|(i, (_, n))| (i, *n))
        .collect();
    let (i, newest) = *rants.last()?;
    let j = view.node(jid)?;
    let title = str_field(j, "title").unwrap_or("");
    let at = str_field(newest, "date").unwrap_or("").to_string();
    let hook = vs.iter().any(|(_, n)| str_field(n, "source") == Some("rant_hook"));
    let row = json!({
        "id": jid, "title": title, "short_name": short_name_of(title, str_field(j, "short_name")),
        "versions": vs.len(), "v": i + 1, "unit": vs[i].0, "at": if at.is_empty() { Value::Null } else { json!(at) },
        "hook": hook, "written": vs.iter().any(|(_, n)| str_field(n, "source") == Some("rant_draft")),
    });
    Some((at, i, row))
}

/// `GET /api/rants`: newest rant first (the newest rant version's day, then the joke's place, newest last).
pub async fn rants(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let view = st.data().await?.view(q.include_hidden()).await?;
    let mut rows: Vec<(String, usize, Value)> = view
        .jokes
        .iter()
        .enumerate()
        .filter_map(|(k, jid)| rant_row(&view, jid).map(|(at, _, row)| (at, k, row)))
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    Ok(ok(&json!({"rants": rows.into_iter().map(|r| r.2).collect::<Vec<_>>()})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_topic_is_case_and_spacing_aside() {
        assert!(same_topic("Self-checkout  machines", " self-checkout machines\n"));
        assert!(!same_topic("self-checkout", "self checkout"));
    }
}
