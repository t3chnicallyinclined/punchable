//! The redline round on a bit (`docs/design/SPEC-2026-11-16-bits-sets-tapes.md` §3.2 and Slice 3 as built; B12, the
//! owner's Redline loop; Rust only, R7: the Python never answers these paths and has no twin of them):
//!
//! - `POST /api/jokes/<id>/redline {unit, marks, on?}`: his marks, sent once. `marks` = 1 to
//!   [`pk_creative::redline::MAX_MARKS`] of `{type: change|ask|keep, text, at?, note?}`: `text` is his words exactly as
//!   they stand, `at` where the phone saw them (a UTF-16 index, a hint when the words occur twice), `note` his note on
//!   a Change or his question on an Ask (required there). At least one Change or Ask. Round one marks his bit: `unit` is
//!   the version he marked, and it must be the newest of its cut (409 otherwise: the bit changed since he marked it; the
//!   marks stay on the phone). Round two marks the draft he is reading: `on` is that round's job, and the version the
//!   draft was made from must still be the newest (409 otherwise). The text marked is read here, never sent by the
//!   phone: the version from the KB, the draft from the job's result. Every quote must be in it (409: "the words you
//!   marked are not in this version"); a Keep and a Change on the same words are a 400. The Keeps of the round he marks
//!   in round two are carried into it. Queues one `redline` job (pk-worker) and records the round in the `redline`
//!   store; 202 `{status: queued, job, poll, round}`. The legal rules run on the text and his notes first.
//! - `GET /api/jokes/<id>/redline`: the round open on the bit: `{joke, newest, round: {job, status, unit, on, n,
//!   sent_at, marks, stale, result} | null, last}`. `status` is the job's (`queued`, `claimed`, `done`, `failed`);
//!   `result` the job's result once done (the whole new draft, the answers, the notes, what the checks dropped);
//!   `stale` = he saved a newer version of that cut since he sent it (the draft is offered against the new text,
//!   never applied by itself).
//! - `POST /api/jokes/<id>/redline/resolve {job, verdict: used|discarded|open, unit?}`: he closed the round: `used`
//!   (the app saved the draft as version `unit` first: `POST /api/jokes/<id>/versions`), `discarded`, or `open` (Undo
//!   of a Use: the round he last closed is open again). A job that is not the open round is a 409.
//! - `GET /api/redlines`: every bit with a round open, newest first: `{open: [{joke, title, short_name, job, status,
//!   state: waiting|ready|refused|failed, stale, sent_at}]}`; Bits shows the ready ones as "Waiting for you".
//!
//! The store is `redline` (`pk_creative::reducers::redline`): a new store, so no other process replays it (B5). The
//! job kind is new in the shared `jobs` store, so the Python image must know its name before this one rolls out
//! (`api-dev/creative_api.py` `JOB_KINDS`; the deploy order in the slice's Built note).

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use pk_core::py;
use pk_creative::Items;
use pk_creative::redline::{self as rl, Kind, MAX_MARKS, MAX_NOTE_CHARS, MAX_QUOTE_CHARS, Mark, WHOLE_CAP};
use pk_creative::reducers::redline::{last_round, open_round};
use pk_kb::View;
use serde_json::{Map, Value, json};
use sha1::{Digest as _, Sha1};

use super::film::cut_of;
use super::rant::versions_of;
use super::{Creative, Kb, Pre, Req, Written, actor, detached, http, is_id, refuse_legal, request};
use crate::auth::Caller;
use crate::display::short_name_of;
use crate::error::{ApiResult, json_response, ok};
use crate::query::Query;
use crate::routes::creative::items_of;
use crate::state::AppState;

/// The store.
pub const STORE: &str = "redline";
/// The job kind.
pub const KIND: &str = "redline";
/// The verdicts of a resolve.
pub const VERDICTS: [&str; 3] = ["used", "discarded", "open"];

fn hex_of(seed: &str, n: usize) -> String {
    hex::encode(Sha1::digest(seed.as_bytes()))[..n].to_string()
}

/// A mark as the body sends it, checked for shape (not yet located).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    /// Change, Ask or Keep.
    pub kind: Kind,
    /// His words, exactly.
    pub text: String,
    /// Where the phone saw them (UTF-16).
    pub at: Option<usize>,
    /// His note or question (empty on a Keep).
    pub note: String,
}

/// `marks` of a body: 1 to [`MAX_MARKS`], each a known type, its words within [`MAX_QUOTE_CHARS`], a note within
/// [`MAX_NOTE_CHARS`] (a question required on an Ask); at least one Change or Ask.
pub fn marks_arg(v: Option<&Value>) -> ApiResult<Vec<Sent>> {
    let bad = || http(400, format!("marks must be a list of 1 to {MAX_MARKS} marks"));
    let list = v
        .and_then(Value::as_array)
        .filter(|l| (1..=MAX_MARKS).contains(&l.len()))
        .ok_or_else(bad)?;
    let mut out = Vec::with_capacity(list.len());
    for m in list {
        let o = m.as_object().ok_or_else(bad)?;
        let kind = o
            .get("type")
            .and_then(Value::as_str)
            .and_then(Kind::parse)
            .ok_or_else(|| http(400, "a mark's type is change, ask or keep"))?;
        let text = o.get("text").and_then(Value::as_str).unwrap_or("");
        if py::strip(text).is_empty() || py::len(text) > MAX_QUOTE_CHARS {
            return Err(http(
                400,
                format!("a mark quotes 1 to {MAX_QUOTE_CHARS} characters of the bit"),
            ));
        }
        let at = match o.get("at") {
            None | Some(Value::Null) => None,
            Some(x) => Some(
                x.as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| http(400, "at is a place in the text"))?,
            ),
        };
        let note = match o.get("note") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => py::strip(s).to_string(),
            Some(_) => return Err(http(400, "a note is text")),
        };
        if py::len(&note) > MAX_NOTE_CHARS {
            return Err(http(400, format!("a note is at most {MAX_NOTE_CHARS} characters")));
        }
        if kind == Kind::Ask && note.is_empty() {
            return Err(http(400, "an Ask needs its question"));
        }
        let note = if kind == Kind::Keep { String::new() } else { note };
        out.push(Sent {
            kind,
            text: text.to_string(),
            at,
            note,
        });
    }
    if !out.iter().any(|m| m.kind != Kind::Keep) {
        return Err(http(400, "mark something to change or ask about"));
    }
    Ok(out)
}

/// The marks located in `base`, in document order; the first one that is not there is the 409.
pub fn locate_all(base: &str, sent: &[Sent]) -> ApiResult<Vec<Mark>> {
    let mut out = Vec::with_capacity(sent.len());
    for s in sent {
        let Some(start) = rl::locate(base, &s.text, s.at) else {
            return Err(http(
                409,
                "the words you marked are not in this version: it changed since you marked it",
            ));
        };
        out.push(Mark {
            kind: s.kind,
            text: s.text.clone(),
            start,
            note: s.note.clone(),
            carried: false,
        });
    }
    out.sort_by_key(|m| (m.start, m.text.len()));
    if rl::conflict(&out).is_some() {
        return Err(http(400, "a Keep and a Change are on the same words"));
    }
    Ok(out)
}

/// Marks in one round with the Keeps carried from the round before, at most (security review: a chain of rounds
/// must not grow the prompt and the job's row without end).
pub const MAX_WITH_CARRIED: usize = 2 * MAX_MARKS;

/// The Keeps of the round before that are in the new text and not marked again: carried into this round, up to
/// [`MAX_WITH_CARRIED`] marks in all.
pub fn carried_keeps(prev: Option<&Value>, base: &str, marks: &[Mark]) -> Vec<Mark> {
    let rows = prev.and_then(|r| r.get("marks")).and_then(Value::as_array);
    let mut out: Vec<Mark> = Vec::new();
    for m in rows.into_iter().flatten() {
        if marks.len() + out.len() >= MAX_WITH_CARRIED {
            break;
        }
        if m.get("type").and_then(Value::as_str) != Some("keep") {
            continue;
        }
        let Some(text) = m.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()) else {
            continue;
        };
        let again = marks.iter().chain(&out).any(|x| x.kind == Kind::Keep && x.text == text);
        if again {
            continue;
        }
        let hint = m
            .get("at")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok());
        if let Some(start) = rl::locate(base, text, hint) {
            // a carried Keep that a Change now sits on is left behind: the newer mark wins
            let k = Mark {
                kind: Kind::Keep,
                text: text.to_string(),
                start,
                note: String::new(),
                carried: true,
            };
            let mut with = marks.to_vec();
            with.push(k.clone());
            if rl::conflict(&with).is_none() {
                out.push(k);
            }
        }
    }
    out
}

/// The marks as the store and the job hold them.
fn marks_json(base: &str, marks: &[Mark]) -> Vec<Value> {
    marks
        .iter()
        .map(|m| {
            json!({"type": m.kind.name(), "text": m.text, "start": m.start, "at": rl::utf16_of(base, m.start),
                   "note": m.note, "carried": m.carried})
        })
        .collect()
}

/// The newest version of a cut on the bit (the stepper's order), with its text.
fn newest_of_cut<'a>(view: &'a View, jid: &str, cut: &str) -> Option<(&'a str, &'a str)> {
    versions_of(view, jid)
        .into_iter()
        .rev()
        .find(|(_, n)| cut_of(n.get("source")) == cut)
        .map(|(u, n)| (u, n.get("text").and_then(Value::as_str).unwrap_or("")))
}

/// A version on the bit: its text and its cut.
fn version_on<'a>(view: &'a View, jid: &str, unit: &str) -> Option<(&'a str, &'static str)> {
    versions_of(view, jid)
        .into_iter()
        .find(|(u, _)| *u == unit)
        .map(|(_, n)| {
            (
                n.get("text").and_then(Value::as_str).unwrap_or(""),
                cut_of(n.get("source")),
            )
        })
}

/// Whether a round's version is no longer the newest of its cut (he saved again since he sent it).
fn is_stale(view: &View, jid: &str, round: &Value) -> (bool, Option<String>) {
    let unit = round.get("unit").and_then(Value::as_str).unwrap_or("");
    let cut = version_on(view, jid, unit).map_or("stage", |(_, c)| c);
    let newest = newest_of_cut(view, jid, cut).map(|(u, _)| u.to_string());
    (newest.as_deref() != Some(unit), newest)
}

/// The job of a round, from the `jobs` store.
fn job_of<'a>(jobs: &'a Items, round: &Value) -> Option<&'a Map<String, Value>> {
    round
        .get("job")
        .and_then(Value::as_str)
        .and_then(|j| jobs.get(j))
        .and_then(Value::as_object)
}

/// A done job's draft, when the round made one.
fn draft_of(job: &Map<String, Value>) -> Option<&str> {
    let r = job.get("result")?;
    (job.get("status").and_then(Value::as_str) == Some("done") && r.get("status").and_then(Value::as_str) == Some("ok"))
        .then(|| r.get("draft").and_then(Value::as_str))
        .flatten()
        .filter(|d| !d.is_empty())
}

/// The joke checked: a bit id that the KB holds.
fn joke_checked(view: &View, jid: &str) -> ApiResult<()> {
    if !is_id(jid) {
        return Err(http(400, "bad joke id"));
    }
    if view.ntype(Some(jid)) != Some("joke") {
        return Err(http(404, format!("no such joke {jid}")));
    }
    Ok(())
}

/// `POST /api/jokes/<id>/redline {unit, marks, on?}`.
pub async fn send(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(jid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q, b } = request(st, raw.as_deref(), &headers, body).await?;
    let sent = marks_arg(b.get("marks"))?;
    let on = match b.get("on") {
        None | Some(Value::Null) => None,
        Some(Value::String(j)) if j.starts_with("job:") && is_id(j) => Some(j.clone()),
        Some(_) => return Err(http(400, "on is the job of the draft he marked")),
    };
    let unit_arg = match b.get("unit") {
        None | Some(Value::Null) => None,
        Some(Value::String(u)) if is_id(u) => Some(u.clone()),
        Some(_) => return Err(http(400, "unit is the version he marked")),
    };
    if on.is_none() && unit_arg.is_none() {
        return Err(http(400, "unit is required (the version he marked)"));
    }
    let (_, view) = {
        let _kb = st.kb_lock().lock().await;
        Kb::new(st.clone(), q.include_hidden()).view().await?
    };
    joke_checked(&view, &jid)?;
    let title = view
        .node(&jid)
        .and_then(|j| j.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let rounds = items_of(&st, |s| &s.redline).await?;
    let jobs = items_of(&st, |s| &s.jobs).await?;
    let open = open_round(&rounds, &jid).cloned();
    // the text he marked, read here: his version, or the draft of the round he is reading
    let (unit, base, bit, prev, n) = if let Some(on) = &on {
        // the open round is the one he marked, or (a double tap) the round already made from it
        let round = open
            .as_ref()
            .filter(|r| {
                r.get("job").and_then(Value::as_str) == Some(on.as_str())
                    || r.get("on").and_then(Value::as_str) == Some(on.as_str())
            })
            .ok_or_else(|| http(409, "that draft is not the one waiting on this bit"))?;
        // the round he marked, as its job holds it: its draft (the text marked) and its marks (the Keeps carried)
        let marked = jobs
            .get(on.as_str())
            .and_then(Value::as_object)
            .ok_or_else(|| http(409, "that round has no draft to mark"))?;
        let draft = draft_of(marked)
            .ok_or_else(|| http(409, "that round has no draft to mark"))?
            .to_string();
        if is_stale(&view, &jid, round).0 {
            return Err(http(409, "the bit changed since that draft was made"));
        }
        let unit = round.get("unit").and_then(Value::as_str).unwrap_or("").to_string();
        let mine = version_on(&view, &jid, &unit)
            .map(|(t, _)| t.to_string())
            .unwrap_or_default();
        // his bit goes along for context when it is not long (the draft carries the rest of it)
        let bit = (py::len(&mine) <= WHOLE_CAP).then_some(mine);
        let input = marked.get("input");
        let n = input.and_then(|i| i.get("n")).and_then(Value::as_i64).unwrap_or(1) + 1;
        (unit, draft, bit, input.cloned(), n)
    } else {
        let unit = unit_arg.unwrap_or_default();
        let (text, cut) = version_on(&view, &jid, &unit).ok_or_else(|| http(404, "that version is not on this bit"))?;
        if newest_of_cut(&view, &jid, cut).map(|(u, _)| u) != Some(unit.as_str()) {
            return Err(http(
                409,
                "the bit changed since you marked it: a newer version is saved",
            ));
        }
        (unit, text.to_string(), None, None, 1)
    };
    let mut marks = locate_all(&base, &sent)?;
    marks.extend(carried_keeps(prev.as_ref(), &base, &marks));
    marks.sort_by_key(|m| (m.start, m.text.len()));
    // every text that goes to the model, each alone and all of them read together (security review: a marker in the
    // bit and another in a note are court material together, as `setnotes` reads its notes)
    let together = std::iter::once(title.as_str())
        .chain(std::iter::once(base.as_str()))
        .chain(bit.as_deref())
        .chain(marks.iter().map(|m| m.note.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    refuse_legal(
        &st,
        std::iter::once(base.as_str())
            .chain(bit.as_deref())
            .chain(marks.iter().map(|m| m.note.as_str()))
            .chain([title.as_str(), together.as_str()]),
    )?;
    let stored = marks_json(&base, &marks);
    let key = format!(
        "{KIND}|{}",
        hex_of(
            &format!("{jid}|{unit}|{}|{base}|{}", on.as_deref().unwrap_or(""), json!(stored)),
            16
        )
    );
    let input = json!({"joke": jid, "title": title, "unit": unit, "on": on, "n": n, "base": base, "bit": bit,
                       "marks": stored});
    let owner = st.config().owner_id.clone();
    let by = actor(&st, &caller, &Map::new())?;
    let (s, out) = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        // round two: the round he marked must still be the open one now, under the lock (a Discard may have landed
        // since the check above); nothing is queued otherwise
        let rounds_now = cr.items(STORE).await?;
        let open_now = open_round(&rounds_now, &jid).cloned();
        let open_job = open_now.as_ref().and_then(|r| r.get("job")).and_then(Value::as_str);
        if let Some(o) = on.as_deref()
            && open_job != Some(o)
        {
            // a double tap (security review, second look): this very round, made from that draft with these marks
            // (the same job key), is already the open one: the answer is that round, as for a round-one repeat
            let jobs_now = cr.items("jobs").await?;
            let repeat = open_job
                .and_then(|j| jobs_now.get(j))
                .and_then(Value::as_object)
                .filter(|j| j.get("key").and_then(Value::as_str) == Some(key.as_str()));
            let Some(j) = repeat else {
                return Err(http(409, "that draft is not the one waiting on this bit"));
            };
            let mut out = super::jobs::queued(j);
            out["round"] = open_now.unwrap_or(Value::Null);
            return Ok((StatusCode::ACCEPTED, out));
        }
        let (s, mut out) = super::jobs::enqueue(&cr, KIND, &key, input, true, &owner).await?;
        let Some(job) = out
            .get("job")
            .and_then(|j| j.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return Ok((s, out));
        };
        let data = json!({"job": job, "unit": unit, "on": on, "n": n, "marks": stored});
        let (jk, same, was_on) = (jid.clone(), job.clone(), on.clone());
        // the same job sent twice (a double tap: the queue answered the job already waiting) is one round
        let pre = move |items: &Items| -> ApiResult<Pre> {
            let open = open_round(items, &jk)
                .and_then(|r| r.get("job"))
                .and_then(Value::as_str);
            if let Some(o) = was_on.as_deref()
                && open != Some(o)
                && open != Some(same.as_str())
            {
                return Err(http(409, "that draft is not the one waiting on this bit"));
            }
            Ok(match open_round(items, &jk) {
                Some(r) if r.get("job").and_then(Value::as_str) == Some(same.as_str()) => {
                    Pre::Done(StatusCode::OK, r.clone())
                }
                _ => Pre::Go,
            })
        };
        out["round"] = match cr.append_if(STORE, "send", &jid, data, &by, pre).await? {
            Written::Landed(item) => item.get("round").cloned().unwrap_or(Value::Null),
            Written::Skipped(_, r) => r,
        };
        Ok((s, out))
    })
    .await?;
    Ok(json_response(s, &out))
}

/// The marks of a stored round as the phone reads them (the byte offset is this service's own).
fn marks_for_phone(round: &Value) -> Value {
    let ms = round
        .get("marks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Value::Array(
        ms.into_iter()
            .map(|mut m| {
                if let Some(o) = m.as_object_mut() {
                    o.remove("start");
                }
                m
            })
            .collect(),
    )
}

/// The last closed round as the phone reads it.
fn last_view(l: &Value) -> Value {
    json!({"job": l.get("job"), "verdict": l.get("verdict"), "unit": l.get("unit"), "at": l.get("at")})
}

/// The round as the page reads it: the job's status and result, and whether he saved a newer version since.
fn round_view(view: &View, jid: &str, round: &Value, jobs: &Items) -> Value {
    let job = job_of(jobs, round);
    let status = job
        .and_then(|j| j.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("queued");
    let result = job
        .filter(|_| status == "done")
        .and_then(|j| j.get("result"))
        .cloned()
        .unwrap_or(Value::Null);
    let (stale, newest) = is_stale(view, jid, round);
    json!({
        "job": round.get("job"), "status": status, "unit": round.get("unit"), "on": round.get("on"),
        "n": round.get("n"), "sent_at": round.get("sent_at"), "marks": marks_for_phone(round),
        "stale": stale, "newest": newest, "result": result,
        "error": job.and_then(|j| j.get("error")).cloned().unwrap_or(Value::Null),
    })
}

/// `GET /api/jokes/<id>/redline`.
pub async fn one(State(st): State<AppState>, Path(jid): Path<String>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let view = st.data().await?.view(q.include_hidden()).await?;
    joke_checked(&view, &jid)?;
    let rounds = items_of(&st, |s| &s.redline).await?;
    let jobs = items_of(&st, |s| &s.jobs).await?;
    let newest = newest_of_cut(&view, &jid, "stage").map(|(u, _)| u.to_string());
    let round = open_round(&rounds, &jid).map(|r| round_view(&view, &jid, r, &jobs));
    let last = last_round(&rounds, &jid).map(last_view);
    Ok(ok(
        &json!({"joke": jid, "newest": newest, "round": round, "last": last}),
    ))
}

/// `POST /api/jokes/<id>/redline/resolve {job, verdict, unit?}`.
pub async fn resolve(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(jid): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Body,
) -> ApiResult<Response> {
    let Req { st, q, b } = request(st, raw.as_deref(), &headers, body).await?;
    let job = b
        .get("job")
        .and_then(Value::as_str)
        .filter(|j| j.starts_with("job:") && is_id(j))
        .ok_or_else(|| http(400, "job is required (the round's job)"))?
        .to_string();
    let verdict = b
        .get("verdict")
        .and_then(Value::as_str)
        .filter(|v| VERDICTS.contains(v))
        .ok_or_else(|| http(400, format!("verdict must be one of {}", VERDICTS.join(", "))))?
        .to_string();
    let unit = match b.get("unit") {
        None | Some(Value::Null) => None,
        Some(Value::String(u)) if is_id(u) => Some(u.clone()),
        Some(_) => return Err(http(400, "unit is the version the draft became")),
    };
    let (_, view) = {
        let _kb = st.kb_lock().lock().await;
        Kb::new(st.clone(), q.include_hidden()).view().await?
    };
    joke_checked(&view, &jid)?;
    if verdict == "used" {
        let u = unit
            .as_deref()
            .ok_or_else(|| http(400, "a draft he used names the version it became"))?;
        if version_on(&view, &jid, u).is_none() {
            return Err(http(404, "that version is not on this bit"));
        }
    }
    let by = actor(&st, &caller, &Map::new())?;
    let out = detached(async move {
        let _cr = st.creative_lock().lock().await;
        let cr = Creative::new(st.clone());
        let (jk, jb, vd) = (jid.clone(), job.clone(), verdict.clone());
        let pre = move |items: &Items| -> ApiResult<Pre> {
            let is = |r: Option<&Value>| r.and_then(|x| x.get("job")).and_then(Value::as_str) == Some(jb.as_str());
            if vd == "open" {
                if open_round(items, &jk).is_some() {
                    return Err(http(409, "another round is open on this bit"));
                }
                if !is(last_round(items, &jk)) {
                    return Err(http(409, "that round is not the last one closed"));
                }
            } else if !is(open_round(items, &jk)) {
                return Err(http(409, "that draft is not the one waiting on this bit"));
            }
            Ok(Pre::Go)
        };
        let data = json!({"job": job, "verdict": verdict, "unit": unit});
        let item = match cr.append_if(STORE, "resolve", &jid, data, &by, pre).await? {
            Written::Landed(v) | Written::Skipped(_, v) => v,
        };
        let last = item.get("last").filter(|l| l.is_object()).map(last_view);
        Ok(json!({"ok": true, "joke": jid, "round": item.get("round").cloned().unwrap_or(Value::Null), "last": last}))
    })
    .await?;
    Ok(ok(&out))
}

/// What a round comes to for the Bits list.
fn state_of(job: Option<&Map<String, Value>>) -> &'static str {
    match job.and_then(|j| j.get("status")).and_then(Value::as_str) {
        Some("done") if job.and_then(draft_of).is_some() => "ready",
        Some("done") => "refused",
        Some("failed") => "failed",
        _ => "waiting",
    }
}

/// `GET /api/redlines`: every bit with a round open, the newest sent first.
pub async fn list(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    crate::routes::pick(&q)?;
    let view = st.data().await?.view(q.include_hidden()).await?;
    let rounds = items_of(&st, |s| &s.redline).await?;
    let jobs = items_of(&st, |s| &s.jobs).await?;
    let mut rows: Vec<(String, Value)> = rounds
        .keys()
        .filter_map(|jid| {
            let r = open_round(&rounds, jid)?;
            let j = view.node(jid).filter(|_| view.ntype(Some(jid)) == Some("joke"))?;
            let title = j.get("title").and_then(Value::as_str).unwrap_or("");
            let at = r.get("sent_at").and_then(Value::as_str).unwrap_or("").to_string();
            let job = job_of(&jobs, r);
            let row = json!({
                "joke": jid, "title": title,
                "short_name": short_name_of(title, j.get("short_name").and_then(Value::as_str)),
                "job": r.get("job"),
                "status": job.and_then(|x| x.get("status")).cloned().unwrap_or_else(|| json!("queued")),
                "state": state_of(job), "stale": is_stale(&view, jid, r).0, "sent_at": at,
            });
            Some((at, row))
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(ok(&json!({"open": rows.into_iter().map(|r| r.1).collect::<Vec<_>>()})))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIT: &str = "My fridge has an app now.\nIt texts me when the milk is sad.\n\nThe doorbell judges my outfits.";

    fn sent(v: &Value) -> Vec<Sent> {
        marks_arg(Some(v)).unwrap()
    }

    #[test]
    fn marks_are_checked_for_shape() {
        let msg = |v: Value| marks_arg(Some(&v)).unwrap_err().msg;
        assert_eq!(msg(json!([])), "marks must be a list of 1 to 20 marks");
        let many: Vec<Value> = (0..21).map(|_| json!({"type": "change", "text": "x"})).collect();
        assert_eq!(msg(json!(many)), "marks must be a list of 1 to 20 marks");
        assert_eq!(
            msg(json!([{"type": "fork", "text": "x"}])),
            "a mark's type is change, ask or keep"
        );
        assert_eq!(
            msg(json!([{"type": "change", "text": "  "}])),
            "a mark quotes 1 to 4000 characters of the bit"
        );
        assert_eq!(msg(json!([{"type": "ask", "text": "x"}])), "an Ask needs its question");
        assert_eq!(
            msg(json!([{"type": "keep", "text": "x"}])),
            "mark something to change or ask about"
        );
        assert_eq!(
            msg(json!([{"type": "change", "text": "x", "note": "n".repeat(501)}])),
            "a note is at most 500 characters"
        );
        assert_eq!(
            msg(json!([{"type": "change", "text": "x", "at": -1}])),
            "at is a place in the text"
        );
        let ok =
            sent(&json!([{"type": "keep", "text": "a", "note": "ignored"}, {"type": "change", "text": "b", "at": 3}]));
        assert_eq!(ok[0].note, "", "a Keep carries no note");
        assert_eq!(ok[1].at, Some(3));
    }

    #[test]
    fn marks_are_found_by_their_words_and_a_stale_one_is_the_409() {
        let s = sent(&json!([
            {"type": "keep", "text": "The doorbell judges my outfits."},
            {"type": "change", "text": "the milk is sad", "note": "sharper"}]));
        let m = locate_all(BIT, &s).unwrap();
        assert_eq!(m[0].kind, Kind::Change, "document order");
        assert!(m.iter().all(|x| x.sits_in(BIT)));
        let gone = sent(&json!([{"type": "change", "text": "the cheese is sad"}]));
        assert_eq!(locate_all(BIT, &gone).unwrap_err().status, StatusCode::CONFLICT);
        let clash = sent(&json!([{"type": "keep", "text": "the milk is sad"}, {"type": "change", "text": "milk"}]));
        assert_eq!(locate_all(BIT, &clash).unwrap_err().status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn the_keeps_of_the_round_before_are_carried_when_they_are_still_there() {
        let draft = "My fridge texts me. The milk is sad.\n\nThe doorbell judges my outfits.";
        let prev = json!({"marks": [
            {"type": "keep", "text": "The doorbell judges my outfits.", "at": 52},
            {"type": "keep", "text": "a line the draft dropped"},
            {"type": "change", "text": "the milk is sad"}]});
        let now = locate_all(draft, &sent(&json!([{"type": "change", "text": "The milk is sad."}]))).unwrap();
        let c = carried_keeps(Some(&prev), draft, &now);
        assert_eq!(c.len(), 1);
        assert!(c[0].carried && c[0].sits_in(draft));
        // marked Keep again: not twice; a Change now on those words: the newer mark wins
        let again = locate_all(
            draft,
            &sent(&json!([{"type": "change", "text": "The milk is sad."},
                         {"type": "keep", "text": "The doorbell judges my outfits."}])),
        )
        .unwrap();
        assert_eq!(carried_keeps(Some(&prev), draft, &again), Vec::<Mark>::new());
        let changed = locate_all(draft, &sent(&json!([{"type": "change", "text": "judges my outfits"}]))).unwrap();
        assert_eq!(carried_keeps(Some(&prev), draft, &changed), Vec::<Mark>::new());
        assert_eq!(carried_keeps(None, draft, &now), Vec::<Mark>::new());
    }
}
