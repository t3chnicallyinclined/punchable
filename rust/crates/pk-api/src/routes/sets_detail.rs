//! One set list, one of his sets, one bit's connections (T9 round 8): `GET /api/boards/<id>` (`story.board_view`),
//! `/api/boards/<id>/build` (`notebook.build` + the working set made from it + each step's text), `/api/mysets/<id>`
//! (`mysets.view`), `/api/mysets/<id>/trim` (`mysets.trim`), `/api/jokes/<id>/connections`
//! (`connections.joke_connections`) and `/api/jokes/<id>/map` (`notebook.joke_map`). The rules are
//! `pk_creative::{setlist, mysets, connections}`; this module reads the stores and the notebook the way the Python
//! routes do, in the Python's order of refusals, and runs the work off the async threads.

use std::sync::{Arc, LazyLock};

use axum::extract::{Path, RawQuery, State};
use axum::response::Response;
use pk_creative::Items;
use regex::Regex;
use serde_json::{Map, Value};

use super::creative::{corpus_and_items, items_of};
use crate::error::{ApiError, ApiResult, ok};
use crate::query::Query;
use crate::state::AppState;

type Obj = Map<String, Value>;

/// `ID_RE` (`creative_api.py`): what a joke id may look like.
static ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A[A-Za-z0-9:_@.+\-]{1,300}\z").expect("a valid regex"));

/// Run `f` on the blocking pool (the notebook's rules are CPU work).
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::internal("JoinError", e))
}

/// An item of a store that is there and not deleted, or the 404 the Python says.
fn live<'a>(items: &'a Items, id: &str, missing: &str) -> Result<&'a Obj, ApiError> {
    items
        .get(id)
        .and_then(Value::as_object)
        .filter(|x| !pk_core::py::truthy(x.get("deleted")))
        .ok_or_else(|| ApiError::not_found(missing))
}

/// `str(v).isdigit() and lo <= int(v) <= hi`, else the 400 with `msg`.
fn digits_in(v: &str, lo: u64, hi: u64, msg: &str) -> Result<u64, ApiError> {
    if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ApiError::bad_request(msg));
    }
    // a run of digits past u64 is past every bound here
    let n = v.parse::<u64>().unwrap_or(u64::MAX);
    if (lo..=hi).contains(&n) {
        Ok(n)
    } else {
        Err(ApiError::bad_request(msg))
    }
}

/// `GET /api/boards/<id>`: everything the Storyboard screen needs (health with its checks, the best next bits, the
/// paper set list). An archived board is still found.
pub async fn board(
    State(st): State<AppState>,
    Path(bid): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_, c, items) = corpus_and_items(&st, &q, |s| &s.storyboards).await?;
    let b = live(&items, &bid, "no such storyboard")?.clone();
    let body = blocking(move || pk_creative::setlist::board_view(&c.story(), &c, &b)).await?;
    Ok(ok(&body))
}

/// `GET /api/boards/<id>/build`: the board as a set (`notebook.build`), the working set made from it last, and each
/// step's text (`server.joke_text`, the joke page's) or his typed idea when there is no written version.
pub async fn board_build(
    State(st): State<AppState>,
    Path(bid): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (d, nb, boards) = corpus_and_items(&st, &q, |s| &s.storyboards).await?;
    let wss = items_of(&st, |s| &s.workingsets).await?;
    let board = live(&boards, &bid, "no such storyboard")?.clone();
    let idx = d.index(q.include_hidden()).await?;
    let body = blocking(move || {
        let mut out = pk_creative::setlist::build(&nb.story(), &nb, &board, 5);
        // `max(ws, key=updated_at)`: the first of the latest
        let made = wss
            .values()
            .filter_map(Value::as_object)
            .filter(|w| {
                !pk_core::py::truthy(w.get("deleted"))
                    && w.get("storyboard_id").and_then(Value::as_str) == Some(bid.as_str())
            })
            .fold(None::<&Obj>, |best, w| {
                let at = |x: &Obj| x.get("updated_at").and_then(Value::as_str).unwrap_or("").to_string();
                match best {
                    Some(x) if at(w) <= at(x) => Some(x),
                    _ => Some(w),
                }
            });
        out["working_set"] = made.and_then(|w| w.get("id")).cloned().unwrap_or(Value::Null);
        let tx = crate::tape::Transcripts::new();
        let ctx = crate::pages::Ctx::new(&idx, &d, &tx);
        if let Some(steps) = out.get_mut("steps").and_then(Value::as_array_mut) {
            for s in steps {
                let j = s.get("joke_id").and_then(Value::as_str).unwrap_or("").to_string();
                let text = ctx.joke_text(&j);
                let idea = if text.get("from").is_none_or(Value::is_null) {
                    ctx.v
                        .nodes
                        .get(&j)
                        .and_then(|n| n.get("summary"))
                        .filter(|x| pk_core::py::truthy(Some(x)))
                        .cloned()
                        .unwrap_or(Value::Null)
                } else {
                    Value::Null
                };
                s["text"] = text;
                s["idea"] = idea;
            }
        }
        out
    })
    .await?;
    Ok(ok(&body))
}

/// The live working sets (not deleted), in store order (`_live(st)`).
fn live_sets(items: &Items) -> Vec<&Obj> {
    items
        .values()
        .filter_map(Value::as_object)
        .filter(|w| !pk_core::py::truthy(w.get("deleted")))
        .collect()
}

/// `GET /api/mysets/<id>`: the set screen (`mysets.view`).
pub async fn myset(
    State(st): State<AppState>,
    Path(sid): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_, c, items) = corpus_and_items(&st, &q, |s| &s.workingsets).await?;
    live(&items, &sid, "no such working set")?;
    let owner = st.config().owner_id.clone();
    let body = blocking(move || {
        let ws = live(&items, &sid, "no such working set").ok()?;
        Some(pk_creative::mysets::view(
            &c.story(),
            &c,
            ws,
            &live_sets(&items),
            &owner,
        ))
    })
    .await?
    .ok_or_else(|| ApiError::not_found("no such working set"))?;
    Ok(ok(&body))
}

/// `GET /api/mysets/<id>/trim?target_s=300`: "make a 5 from my 10", a suggestion; nothing is written.
pub async fn trim(State(st): State<AppState>, Path(sid): Path<String>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_, c, items) = corpus_and_items(&st, &q, |s| &s.workingsets).await?;
    let t = q.get("target_s").unwrap_or("");
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ApiError::bad_request("target_s is required (seconds)"));
    }
    // the set is looked up before the length is checked (`M.trim(c, CR._ws(st, sid), self._target(int(t)))`)
    let ws = live(&items, &sid, "no such working set")?.clone();
    let target = digits_in(t, 60, 4 * 3600, "target_s must be a number between 60 and 14400")?;
    let target = i64::try_from(target).unwrap_or(i64::MAX);
    let body = blocking(move || pk_creative::mysets::trim(&c, &ws, target)).await?;
    Ok(ok(&body))
}

/// `_joke(c, jid)`: a joke id the notebook has (400 for a malformed id, 404 for an unknown one); a hidden joke
/// passes here and fails in the rules, as the Python's `KeyError` does (a 500).
fn joke_ok(c: &pk_creative::corpus::Corpus, jid: &str) -> Result<(), ApiError> {
    if !ID_RE.is_match(jid) {
        return Err(ApiError::bad_request("bad joke id"));
    }
    match c.nodes.get(jid) {
        Some(n) if n.get("type").and_then(Value::as_str) == Some("joke") => Ok(()),
        _ => Err(ApiError::not_found(format!("no such joke {jid}"))),
    }
}

fn unknown(jid: &str) -> ApiError {
    ApiError::internal("KeyError", pk_core::py::quote(&format!("unknown joke {jid}")))
}

/// `GET /api/jokes/<id>/connections?cap=5`: one bit's connections, ranked and capped per group.
pub async fn connections(
    State(st): State<AppState>,
    Path(jid): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (d, c) = notebook(&st, &q).await?;
    drop(d);
    let cap = digits_in(q.get("cap").unwrap_or("5"), 1, 20, "cap must be 1-20")?;
    joke_ok(&c, &jid)?;
    let cap = usize::try_from(cap).unwrap_or(20);
    let body = blocking(move || {
        pk_creative::connections::joke_connections(&c.story(), &c, &jid, cap).ok_or_else(|| unknown(&jid))
    })
    .await??;
    Ok(ok(&body))
}

/// `GET /api/jokes/<id>/map?cap=8`: the joke page as a map.
pub async fn joke_map(
    State(st): State<AppState>,
    Path(jid): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (d, c) = notebook(&st, &q).await?;
    drop(d);
    let cap = digits_in(q.get("cap").unwrap_or("8"), 1, 20, "cap must be 1-20")?;
    joke_ok(&c, &jid)?;
    let cap = usize::try_from(cap).unwrap_or(20);
    let body =
        blocking(move || pk_creative::connections::joke_map(&c.story(), &c, &jid, cap).ok_or_else(|| unknown(&jid)))
            .await??;
    Ok(ok(&body))
}

/// `GET /api/workingsets`: every working set (not deleted), newest first, under both spellings.
pub async fn workingsets(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    super::pick(&q)?;
    let items = items_of(&st, |s| &s.workingsets).await?;
    let mut ws = live_sets(&items);
    let key = |w: &Obj| {
        (
            w.get("updated_at").and_then(Value::as_str).unwrap_or("").to_string(),
            w.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
        )
    };
    ws.sort_by_key(|w| std::cmp::Reverse(key(w)));
    let rows: Vec<Value> = ws
        .into_iter()
        .map(|w| {
            let ver = pk_creative::mysets::current(w);
            let vget = |k: &str| ver.and_then(|v| v.get(k)).cloned().unwrap_or(Value::Null);
            let n = ver
                .and_then(|v| v.get("bits"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let get = |k: &str| w.get(k).cloned().unwrap_or(Value::Null);
            serde_json::json!({"id": get("id"), "title": get("title"), "version": get("current"),
                               "target_s": vget("target_s"), "n": n, "storyboard_id": get("storyboard_id"),
                               "updated": get("updated_at"), "source": get("source")})
        })
        .collect();
    Ok(ok(&serde_json::json!({"workingsets": rows, "working_sets": rows})))
}

/// `GET /api/workingsets/<id>`: the working-set page (`creative_api.ws_detail`).
pub async fn workingset(
    State(st): State<AppState>,
    Path(wid): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_, c, items) = corpus_and_items(&st, &q, |s| &s.workingsets).await?;
    let ws = live(&items, &wid, "no such working set")?.clone();
    let body = blocking(move || pk_creative::today::ws_detail(&c, &ws)).await?;
    Ok(ok(&body))
}

/// `GET /api/workingsets/<id>/nights`: every recorded night aligned against the set.
pub async fn ws_nights(
    State(st): State<AppState>,
    Path(wid): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_, c, items) = corpus_and_items(&st, &q, |s| &s.workingsets).await?;
    let ws = live(&items, &wid, "no such working set")?.clone();
    let body =
        blocking(move || serde_json::json!({"working_set": wid, "nights": pk_creative::today::align_all(&c, &ws)}))
            .await?;
    Ok(ok(&body))
}

/// `GET /api/workingsets/<id>/bits/<joke>`: one bit's history in the set, with its locks.
pub async fn ws_bit(
    State(st): State<AppState>,
    Path((wid, jid)): Path<(String, String)>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (_, c, items) = corpus_and_items(&st, &q, |s| &s.workingsets).await?;
    let ws = live(&items, &wid, "no such working set")?.clone();
    joke_ok(&c, &jid)?;
    let body = blocking(move || {
        let mut out = pk_creative::today::bit_history_value(&c, &jid);
        let lk = ws.get("locks").and_then(|l| l.get(&jid)).and_then(Value::as_object);
        out["working_set"] = Value::String(wid);
        out["locked"] = Value::Bool(lk.is_some_and(|l| pk_core::py::truthy(l.get("bit"))));
        out["locked_lines"] = lk
            .and_then(|l| l.get("lines"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!([]));
        out
    })
    .await?;
    Ok(ok(&body))
}

/// `GET /api/notebook/related?q=<idea>&cap=10[&exclude=<joke>...]`: his own bits about an idea (`notebook.related`),
/// with the database's fused text + vector search added to his-words search (a failed search is left out: it is a
/// bonus).
pub async fn related(State(st): State<AppState>, RawQuery(raw): RawQuery) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let text = pk_core::py::strip(q.get("q").unwrap_or("")).to_string();
    let chars = text.chars().count();
    // the refusals come before the data is read (the Python checks the query first), the 503 while booting first
    let checked = if chars == 0 || chars > 4000 {
        Err(ApiError::bad_request("q is required (at most 4000 characters)"))
    } else {
        digits_in(q.get("cap").unwrap_or("10"), 1, 30, "cap must be 1-30")
    };
    let cap = match checked {
        Ok(c) => usize::try_from(c).unwrap_or(30),
        Err(e) => {
            super::prepare_rows(&st, &q).await?;
            return Err(e);
        }
    };
    let head: String = text.chars().take(500).collect();
    let st2 = st.clone();
    let work = async move { super::search::fused(&st2, &head, 20, false).await };
    let (d, _idx, found) = super::prepare_and(&st, &q, work).await?;
    let nb = d.corpus(&st).await?;
    let hybrid = match super::finish(found).await? {
        Ok(raw) => {
            let view = d.view(false).await?;
            Some(crate::search::fold_jokes(&raw.hits, &view))
        }
        Err(e) => {
            tracing::warn!("related: the fused search failed: {}; his words only", e.msg);
            None
        }
    };
    let exclude: Vec<String> = q.all("exclude").into_iter().map(str::to_string).collect();
    let body = blocking(move || {
        let ex: Vec<&str> = exclude.iter().map(String::as_str).collect();
        pk_creative::setlist::related(&nb, nb.notebook_index(), &text, cap, hybrid.as_deref(), &ex)
    })
    .await?;
    Ok(ok(&body))
}

/// `GET /api/sets/<id>/clips`: the set's `clips.json` as it is, or an empty list when it has none.
pub async fn clips(
    State(st): State<AppState>,
    Path(sid): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    let (d, v) = super::prepare_view(&st, &q).await?;
    if v.nodes.get(&sid).and_then(|s| s.get("type")).and_then(Value::as_str) != Some("set") {
        return Err(ApiError::not_found("no such set"));
    }
    let none = || serde_json::json!({"set": sid, "clips": [], "source": null});
    // `kb.set_dir(sid)`: `set:2024-01-05_x` -> `20240105_x`, none for any other shape
    let Some(dir) = set_dir_of(&sid) else {
        return Ok(ok(&none()));
    };
    let path = format!("corpus/sets/{dir}/clips.json");
    let Some(sha) = d.files.get(&path) else {
        return Ok(ok(&none()));
    };
    let texts = st
        .src_texts()
        .get(st.store_handle().as_deref(), &[(path.clone(), sha.clone())])
        .await
        .map_err(|e| ApiError::internal("StoreError", e))?;
    let Some(text) = texts.get(&path) else {
        return Ok(ok(&none()));
    };
    let body: Value = serde_json::from_str(text).map_err(|e| ApiError::internal("JSONDecodeError", e))?;
    Ok(ok(&body))
}

/// `server.set_dir(set_id)`: `^(\d{4})-?(\d{2})-?(\d{2})_([A-Za-z0-9_-]+)$` on the id without `set:`.
fn set_dir_of(sid: &str) -> Option<String> {
    static SET_DIR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\A(\d{4})-?(\d{2})-?(\d{2})_([A-Za-z0-9_-]+)\z").expect("a valid regex"));
    let s = sid.strip_prefix("set:").unwrap_or(sid);
    let m = SET_DIR.captures(s)?;
    Some(format!("{}{}{}_{}", &m[1], &m[2], &m[3], &m[4]))
}

/// The data and its notebook: the 503 while booting, then `?backend=`.
async fn notebook(
    st: &AppState,
    q: &Query,
) -> Result<(Arc<crate::snapshot::Data>, Arc<crate::creative::GenCorpus>), ApiError> {
    let d = super::prepare_rows(st, q).await?;
    let c = d.corpus(st).await?;
    Ok((d, c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_are_digits_in_range() {
        assert_eq!(digits_in("5", 1, 20, "m").unwrap(), 5);
        assert_eq!(digits_in("020", 1, 20, "m").unwrap(), 20);
        for bad in ["", "0", "21", "-1", "+5", "1.0", " 5", "99999999999999999999999"] {
            assert_eq!(
                digits_in(bad, 1, 20, "cap must be 1-20").unwrap_err().msg,
                "cap must be 1-20"
            );
        }
    }

    #[test]
    fn set_dirs_follow_the_server() {
        assert_eq!(set_dir_of("set:2024-01-05_x-y"), Some("20240105_x-y".into()));
        assert_eq!(set_dir_of("20240105_x"), Some("20240105_x".into()));
        assert_eq!(set_dir_of("set:2024-01-05_x/y"), None);
        assert_eq!(set_dir_of("set:nope"), None);
    }

    #[test]
    fn joke_ids_follow_the_python_pattern() {
        assert!(ID_RE.is_match("joke:a-b_c.d@e+f"));
        assert!(!ID_RE.is_match("joke a"));
        assert!(!ID_RE.is_match(""));
        assert!(!ID_RE.is_match(&"x".repeat(301)));
    }
}
