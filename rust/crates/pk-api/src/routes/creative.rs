//! The creative layer's list reads (`creative_api.py`, `mysets_api.py`, `say_api.py`): `GET /api/settings`,
//! `/api/boards`, `/api/mysets` and `/api/tonight/screen`. Each replays the creative stores it reads from their
//! `creative_event` lines, read on every request (creative writes do not move the generation; `Replayed` keeps the last
//! replay while the lines stay the same), over this generation's notebook where it needs one.

use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::response::Response;
use pk_creative::Items;
use serde_json::{Map, Value, json};

use crate::creative::{GenCorpus, Replayed};
use crate::display::{short_name_of, venue_label};
use crate::error::{ApiError, ApiResult, ok};
use crate::index::AppIndex;
use crate::pages::Ctx;
use crate::query::Query;
use crate::setlists;
use crate::snapshot::Data;
use crate::state::AppState;

/// A store's items for this request, or the 500 the Python's exception would be.
pub(crate) async fn items_of(
    st: &AppState,
    which: fn(&crate::creative::Stores) -> &Replayed,
) -> Result<Arc<Items>, ApiError> {
    let store = st.store_handle();
    which(st.stores())
        .items(store.as_deref(), &st.config().owner_id)
        .await
        .map_err(|e| ApiError::internal("CreativeError", e))
}

/// The data, its notebook and a store's items: the 503 while booting first, then `?backend=`; the store's lines are
/// read right after this request's `meta:state` read, while the data and the notebook are made ready.
pub(super) async fn corpus_and_items(
    st: &AppState,
    q: &Query,
    which: fn(&crate::creative::Stores) -> &Replayed,
) -> Result<(Arc<Data>, Arc<GenCorpus>, Arc<Items>), ApiError> {
    if let Err(e) = super::pick(q) {
        st.data().await?;
        return Err(e);
    }
    let me = st.clone();
    let (d, items) = st.data_and(async move { items_of(&me, which).await }).await?;
    let c = d.corpus(st).await?;
    let items = super::finish(items).await??;
    Ok((d, c, items))
}

/// `GET /api/settings`: his four settings over the defaults (the `account` store only: no generation, no notebook).
pub async fn settings(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    super::pick(&q)?;
    let items = items_of(&st, |s| &s.account).await?;
    Ok(ok(&setlists::settings(&items)))
}

/// `GET /api/boards`: the live set lists newest first; `?archived=1` (or `true`) lists the archived ones instead.
pub async fn boards(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let archived = matches!(q.get("archived"), Some("1" | "true"));
    let (_, c, items) = corpus_and_items(&st, &q, |s| &s.storyboards).await?;
    Ok(ok(&setlists::boards(&c, &items, archived)))
}

/// `GET /api/mysets`: his sets newest first, each with its length, run time and one-word health.
pub async fn mysets(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_, c, items) = corpus_and_items(&st, &q, |s| &s.workingsets).await?;
    let owner = st.config().owner_id.clone();
    // the transitions of every set: CPU work, off the async workers
    let body = tokio::task::spawn_blocking(move || setlists::mysets(&c, &items, &owner))
        .await
        .map_err(|e| ApiError::internal("JoinError", e))?;
    Ok(ok(&body))
}

/// `memolink.flagged(v)`: a triage verdict that makes a "not a joke?" suggestion.
pub(super) fn flagged(v: &Map<String, Value>) -> bool {
    const NOTJOKE_MIN: f64 = 0.75;
    const EMPTY_CHARS: f64 = 60.0;
    let conf = pk_creative::corpus::num(Some(v.get("confidence").unwrap_or(&json!(0)))).unwrap_or(0.0);
    if conf < NOTJOKE_MIN {
        return false;
    }
    let kind = v.get("kind").and_then(Value::as_str);
    matches!(kind, Some("conversation" | "note"))
        || (kind == Some("unclear")
            && pk_creative::corpus::num(v.get("chars").filter(|c| pk_core::py::truthy(Some(c)))).unwrap_or(0.0)
                <= EMPTY_CHARS)
}

/// How many "not a joke?" suggestions are waiting (`len(suggest_api.items(cr, kb))`): the memos the triage judge
/// flagged (`kb/creative/suggest_judge.json`) whose idea is a joke in this request's view and not one he kept.
fn to_sort(judge: Option<&str>, kept: &Items, idx: &AppIndex) -> usize {
    let judged: Value = judge.and_then(|t| serde_json::from_str(t).ok()).unwrap_or(Value::Null);
    let Some(items) = judged.get("items").and_then(Value::as_object) else {
        return 0;
    };
    items
        .values()
        .filter_map(Value::as_object)
        .filter(|x| flagged(x))
        .filter_map(|x| x.get("joke_id").and_then(Value::as_str).filter(|j| !j.is_empty()))
        .filter(|j| {
            idx.view
                .nodes
                .get(*j)
                .is_some_and(|n| n.get("type").and_then(Value::as_str) == Some("joke"))
                && !kept.contains_key(*j)
        })
        .count()
}

/// `worker_status(cr)`: the worker's line from its last claim in the database, whichever server answered it; a
/// database that cannot be read is the 500 the Python's would be.
pub(super) async fn worker_status(st: &AppState) -> Result<Value, ApiError> {
    let store = st
        .store_handle()
        .ok_or_else(|| ApiError::internal("ConnectionError", "the database is not connected"))?;
    let seen = crate::worker::load_seen(&store)
        .await
        .map_err(|e| ApiError::internal("StoreError", e))?;
    Ok(crate::worker::status(
        seen.as_ref(),
        st.config().auth.is_some(),
        crate::worker::unix_now(),
    ))
}

/// The path of the judge's verdicts.
const SUGGEST_JUDGE: &str = "kb/creative/suggest_judge.json";

/// The judge's verdicts file of this generation (`suggest_api.judged`): `None` when there is none, or when it cannot be
/// read (the Python's `except (OSError, ValueError)` counts nothing).
pub(super) async fn judge_text(st: &AppState, d: &Data) -> Option<Arc<str>> {
    let sha = d.files.get(SUGGEST_JUDGE)?;
    match st
        .src_texts()
        .get(
            st.store_handle().as_deref(),
            &[(SUGGEST_JUDGE.to_string(), sha.clone())],
        )
        .await
    {
        Ok(mut t) => t.remove(SUGGEST_JUDGE),
        Err(e) => {
            tracing::warn!("reading {SUGGEST_JUDGE}: {e}; nothing flagged");
            None
        }
    }
}

/// `GET /api/tonight/screen` (`say_api.tonight_screen`): the whole Tonight tab in one call: tonight's list, the
/// newest set, the suggested questions, how many "not a joke?" suggestions wait, the worker and the counts. The
/// worker's status comes from its last claim in the database (`worker:seen`, written by whichever server answered
/// it), read beside the stores while the rest is built.
pub async fn tonight_screen(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    if let Err(e) = super::pick(&q) {
        st.data().await?;
        return Err(e);
    }
    let me = st.clone();
    let (d, stores) = st
        .data_and(async move {
            tokio::try_join!(
                items_of(&me, |s| &s.storyboards),
                items_of(&me, |s| &s.suggestions),
                worker_status(&me)
            )
        })
        .await?;
    let (idx, c) = tokio::try_join!(d.index(q.include_hidden()), d.corpus(&st))?;
    let (boards, kept, worker) = super::finish(stores).await??;
    let judge = judge_text(&st, &d).await;
    // `time.strftime` on the Python pod: its local day
    let now = chrono::Local::now();
    let (date, weekday) = (now.format("%Y-%m-%d").to_string(), now.format("%A").to_string());
    let body = tokio::task::spawn_blocking(move || {
        let board = setlists::tonight_board(&c, &boards);
        let tx = crate::tape::Transcripts::new();
        let ctx = Ctx::new(&idx, &d, &tx);
        let latest_set = idx.sets.last().map_or(Value::Null, |sid| latest_card(&ctx, sid));
        let mut qs = Vec::new();
        if let Some(g) = board.get("first_gap").filter(|g| !g.is_null()) {
            let name = |k: &str| g.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            qs.push(format!(
                "How do I get from {} to {}?",
                name("from_name"),
                name("to_name")
            ));
        }
        qs.push("What's my strongest closer right now?".to_string());
        qs.push(never_tried(&c));
        json!({"date": date, "weekday": weekday, "board": board, "to_sort": to_sort(judge.as_deref(), &kept, &idx),
               "latest_set": latest_set, "questions": qs, "worker": worker,
               "counts": {"bits": c.jokes.len(), "sets": c.sets.len()}})
    })
    .await
    .map_err(|e| ApiError::internal("JoinError", e))?;
    Ok(ok(&body))
}

/// The newest set as the Tonight row shows it (`latest_card`).
fn latest_card(c: &Ctx<'_>, sid: &str) -> Value {
    let empty = Map::new();
    let s = c.v.nodes.get(sid).map_or(&empty, |n| n);
    let sc = c.set_info(sid);
    let (name, guessed) = venue_label(s.get("venue").and_then(Value::as_str));
    let when = sc
        .get("show_date")
        .filter(|x| pk_core::py::truthy(Some(x)))
        .or_else(|| s.get("date"));
    let when_label = match when.and_then(Value::as_str) {
        Some(w) if !w.is_empty() => pk_creative::ask::month_year(Some(w)),
        _ => String::new(),
    };
    let field = |k: &str| s.get(k).cloned().unwrap_or(Value::Null);
    // `bool(sc.get("reviewed")) if sc else None`: `set_info` always has keys and never `reviewed`
    json!({"id": sid, "title": field("title"), "date": field("date"), "venue": field("venue"),
           "reviewed": sc.get("reviewed").is_some_and(|r| pk_core::py::truthy(Some(r))),
           "venue_short": name, "venue_guessed": guessed, "when_label": when_label})
}

/// The third suggested question: a bit he wrote and never tried on stage (the one seen last), else a quiet-room one.
fn never_tried(c: &GenCorpus) -> String {
    let key = |j: &str| {
        (
            c.joke(j)
                .and_then(|n| n.get("last_seen"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            j.to_string(),
        )
    };
    let pick = c
        .jokes
        .iter()
        .filter(|j| c.perfs.get(*j).is_none_or(Vec::is_empty))
        .fold(None::<(String, String)>, |best, j| {
            let k = key(j);
            match best {
                Some(b) if k <= b => Some(b),
                _ => Some(k),
            }
        });
    match pick {
        Some((_, j)) => {
            let custom = c.joke(&j).and_then(|n| n.get("short_name")).and_then(Value::as_str);
            format!(
                "Anything I wrote about {} I never tried?",
                short_name_of(&c.title(&j), custom).to_lowercase()
            )
        }
        None => "What do I have that works in a quiet room?".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        let Value::Object(m) = v else { panic!("an object") };
        m
    }

    #[test]
    fn flagged_is_a_confident_conversation_note_or_empty_unclear() {
        assert!(flagged(&obj(json!({"kind": "conversation", "confidence": 0.8}))));
        assert!(flagged(&obj(json!({"kind": "note", "confidence": 0.75}))));
        assert!(!flagged(&obj(json!({"kind": "note", "confidence": 0.74}))));
        assert!(!flagged(&obj(json!({"kind": "note"}))));
        assert!(flagged(&obj(
            json!({"kind": "unclear", "confidence": 0.9, "chars": 60})
        )));
        assert!(flagged(&obj(
            json!({"kind": "unclear", "confidence": 0.9, "chars": null})
        )));
        assert!(!flagged(&obj(
            json!({"kind": "unclear", "confidence": 0.9, "chars": 61})
        )));
        assert!(!flagged(&obj(json!({"kind": "joke", "confidence": 1.0}))));
        assert!(flagged(&obj(json!({"kind": "note", "confidence": true}))));
    }
}
