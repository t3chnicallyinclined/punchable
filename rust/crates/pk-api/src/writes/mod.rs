//! The write routes (lane T10), group by group: the primitives every write shares, written out once from
//! `api-dev/server.py` and `api-dev/creative_api.py`.
//!
//! A knowledge-base write ([`Kb`]) validates the request against the view this request would read (`kb.view()`:
//! the visible one unless `?include_hidden=1`, proven current by a second `meta:state` read:
//! `AppState::data_for_write`), stamps the line (`ts`, `reason`, `by`) and appends it with
//! `pk_store::Store::append_decision`: the row and its effect commit in one transaction and the database's own
//! `fn::apply_decision` bumps `meta:state.gen`, which is the same generation every reader here and on the Python
//! waits for (`AppState::data`) and the `/api/live` hub's `change` event. A creative write ([`Creative`]) reads the
//! store's `creative_event` rows, replays them, runs the route's own check over the items, validates the event on a
//! copy (`LineStore.append`) and creates the next row; the Python finds the row on its next request
//! (`Creative.reconcile`: the database is ahead, its file is extended) and no file is written here.
//!
//! Two writers, one arbiter: this process serialises its own writes like the Python's `kb.lock` / `cr.lock`, and
//! across processes the unique indexes (`decision.seq`, `creative_event (store, seq)`) refuse the second of two
//! appends that raced for the same place; the loser reads again and retries ([`RETRIES`]), and every retry starts
//! over from a fresh view or fresh items (a decision is built again, an undo's inverse is built again, a route's
//! check runs again), so a line that no longer holds is refused the way a first attempt would be. Nothing is
//! merged silently and no append can land twice.
//!
//! A write runs to its end once started ([`detached`]): a client that hangs up mid-way (the phone losing signal)
//! cannot leave a unit without its edge or a merge half written, as the Python's request thread cannot.

pub mod bits;
pub mod ideas;
pub mod jobs;
pub mod learned;
pub mod merge;
pub mod transcript;

use std::sync::{Arc, LazyLock};

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode, header};
use pk_creative::{Event, Items};
use pk_kb::{Decision, KbError, View};
use pk_store::{CreativeEvent, DecisionRow, StoreError};
use regex::Regex;
use serde_json::{Map, Value};

use crate::auth::Caller;
use crate::error::{ApiError, ApiResult};
use crate::snapshot::Data;
use crate::state::AppState;

/// `MAX_BODY`: the largest JSON body an app route takes.
pub const MAX_BODY: usize = 256 * 1024;

/// `LEGAL_MSG`: the 422 every stored text can be refused with.
pub const LEGAL_MSG: &str = "That reads like court material; Punchable won't send it to the AI.";

/// How often an append is retried after another writer took its place (two processes write today; the scratch test
/// races eight tasks through the primitive without the process lock). Each retry waits a little longer than the
/// last, so two racers do not collide again in the same step.
pub const RETRIES: usize = 12;

/// The wait before retry `attempt` (0 for the first).
async fn back_off(attempt: usize) {
    if attempt > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(2 * attempt as u64)).await;
    }
}

/// `server.py` `ID_RE` under `re.match` (whose `$` also matches before one final newline).
static ID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A[A-Za-z0-9:_@.+\-]{1,300}\n?\z").unwrap_or_else(|e| unreachable!("constant regex: {e}"))
});

/// Whether `s` is an id the write routes accept (`ID_RE.match(s)`).
pub fn is_id(s: &str) -> bool {
    ID_RE.is_match(s)
}

/// `server.py` `body_json`: `Content-Length` decides what is read (nothing = `{}`; over [`MAX_BODY`] = refused
/// before the body is read, as the Python refuses it), then the three refusals in its order.
pub async fn body_json(headers: &HeaderMap, body: Body) -> ApiResult<Map<String, Value>> {
    let n: i64 = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    if n <= 0 {
        return Ok(Map::new());
    }
    let n = usize::try_from(n).unwrap_or(usize::MAX);
    if n > MAX_BODY {
        return Err(ApiError::bad_request("body too large"));
    }
    let bytes = axum::body::to_bytes(body, MAX_BODY)
        .await
        .map_err(|e| ApiError::bad_request(format!("body could not be read: {e}")))?;
    let raw = &bytes[..n.min(bytes.len())];
    match serde_json::from_slice::<Value>(raw) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(ApiError::bad_request("body must be an object")),
        Err(_) => Err(ApiError::bad_request("body is not JSON")),
    }
}

/// Run a write to its end in its own task, whatever the client does meanwhile: the answer is awaited here and, when
/// the request future is dropped first, the write still finishes (the Python's request thread always runs to the
/// end). A panic inside is the 500 a fault is.
pub async fn detached<T: Send + 'static>(f: impl Future<Output = ApiResult<T>> + Send + 'static) -> ApiResult<T> {
    tokio::spawn(f).await.map_err(|e| ApiError::internal("JoinError", e))?
}

/// `b.get(k)` as a string.
pub fn str_of<'a>(b: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    b.get(k).and_then(Value::as_str)
}

/// `server.py` `actor(b)`: who this write is by.
pub fn actor(st: &AppState, caller: &Caller, body: &Map<String, Value>) -> ApiResult<String> {
    crate::auth::actor::for_write(
        st.verifier().is_some(),
        caller.0.as_ref(),
        Some(body),
        &st.config().owner_id,
    )
}

/// `refuse_legal(*texts)`: a text that reads like court material is a 422 and nothing is written.
pub fn refuse_legal<'a>(st: &AppState, texts: impl IntoIterator<Item = &'a str>) -> ApiResult<()> {
    for t in texts {
        if !t.is_empty() && st.legal().is_legal(t) {
            return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, LEGAL_MSG));
        }
    }
    Ok(())
}

/// `kb.DecisionError(msg, status)` as the route answers it.
pub fn kb_err(e: KbError) -> ApiError {
    ApiError::new(StatusCode::from_u16(e.status).unwrap_or(StatusCode::BAD_REQUEST), e.msg)
}

/// A store error as the Python answers it: a refusal with its status, a gap in a store's rows as the `HttpError`
/// `Mirror.lines` raises, anything else the 500 `do_POST` answers with.
pub fn store_err(e: StoreError) -> ApiError {
    match e {
        StoreError::Refused { status, msg } => ApiError::new(
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            msg,
        ),
        StoreError::Gap { store } => ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("creative_event {store}: sequence has gaps"),
        ),
        other => ApiError::internal("StoreError", other),
    }
}

/// Whether a write failed because another writer took its place: the unique index on `decision.seq`
/// (`decision_seq`) or on `creative_event (store, seq)` (`creative_event_store_seq`), the only errors an append is
/// retried after.
pub fn is_place_taken(e: &StoreError) -> bool {
    let text = e.to_string();
    text.contains("already contains") && (text.contains("decision_seq") || text.contains("creative_event_store_seq"))
}

fn fault(kind: &str, e: impl std::fmt::Display) -> ApiError {
    ApiError::internal(kind, e)
}

fn not_connected() -> ApiError {
    ApiError::internal("ConnectionError", "the database is not connected")
}

/// The knowledge-base write primitives of one request (`kb.decide`, `kb.append_lines`, `kb.undo`), over the view
/// this request reads. The caller holds [`AppState::kb_lock`] for as long as the Python holds `kb.lock`.
pub struct Kb {
    st: AppState,
    hidden: bool,
}

impl Kb {
    /// Over the view with or without hidden material (`?include_hidden=`).
    pub fn new(st: AppState, hidden: bool) -> Self {
        Self { st, hidden }
    }

    /// `kb.view()`: the current data, proven current ([`AppState::data_for_write`]), and its view.
    pub async fn view(&self) -> ApiResult<(Arc<Data>, Arc<View>)> {
        let d = self.st.data_for_write().await?;
        let v = d.view(self.hidden).await?;
        Ok((d, v))
    }

    fn owner(&self) -> &str {
        &self.st.config().owner_id
    }

    /// `KB._build(v, req)` alone: the validated decision, not yet stamped or appended (a plan is validated whole
    /// before its first line is written).
    pub fn build(&self, v: &View, req: &Value, now: &str) -> ApiResult<pk_kb::Built> {
        pk_kb::build(v, req, now, self.owner()).map_err(kb_err)
    }

    /// `KB.decide(req, by)`: validate against the current view, stamp, append. The reason is the request's.
    pub async fn decide(&self, req: &Value, by: &str) -> ApiResult<(usize, Decision)> {
        let reason = req.get("reason").and_then(Value::as_str).unwrap_or("").to_string();
        self.decide_at(req, &pk_core::time::now_iso(), &reason, by).await
    }

    /// [`Kb::decide`] with the stamp given (`kb.append_lines([{**built, "reason": ...}], by)`: one line of a
    /// sequence stamped like the others). Every attempt, the first included, builds the line from a fresh view, so a
    /// retry after another writer took the place is validated against what that writer did.
    pub async fn decide_at(&self, req: &Value, now: &str, reason: &str, by: &str) -> ApiResult<(usize, Decision)> {
        self.decide_patched(req, now, reason, by, |_| {}).await
    }

    /// [`Kb::decide_at`] with `patch` applied to the validated line before it is stamped (the idea's `from`, which
    /// the Python adds after `_build` so the builder never sees it); applied again on every rebuild.
    pub async fn decide_patched(
        &self,
        req: &Value,
        now: &str,
        reason: &str,
        by: &str,
        patch: impl Fn(&mut pk_kb::Built),
    ) -> ApiResult<(usize, Decision)> {
        for attempt in 0..RETRIES {
            back_off(attempt).await;
            let (_, v) = self.view().await?;
            let mut built = self.build(&v, req, now)?;
            patch(&mut built);
            let line = Decision(pk_kb::build::stamp(&built, now, reason, by));
            match self.append_once(&line).await {
                Ok(out) => return Ok(out),
                Err(Retry(e)) if attempt + 1 < RETRIES => {
                    tracing::info!(attempt, "decision: another writer took the place; building again: {e}");
                }
                Err(Retry(e)) => return Err(store_err(e)),
                Err(Final(e)) => return Err(e),
            }
        }
        unreachable!("the loop returns on its last attempt")
    }

    async fn append_once(&self, line: &Decision) -> Result<(usize, Decision), Appended> {
        let store = self.st.store_handle().ok_or_else(|| Final(not_connected()))?;
        match store.append_decision(line, None).await {
            Ok(a) => {
                let seq = usize::try_from(a.seq).map_err(|e| Final(fault("ValueError", e)))?;
                // the line as the Python hands it back: through the row and out again (`row_to_decision`)
                Ok((seq, DecisionRow::from_decision(line, a.seq).into_decision()))
            }
            Err(e) if is_place_taken(&e) => Err(Retry(e)),
            Err(e) => Err(Final(store_err(e))),
        }
    }

    /// `KB.undo(index, by)`: the inverse line of decision `index` (the newest undoable one when `None`), appended.
    /// Built again from a fresh view on every attempt: a line another writer undid meanwhile is the Python's 409.
    pub async fn undo(&self, index: Option<i64>, by: &str) -> ApiResult<(usize, Decision)> {
        for attempt in 0..RETRIES {
            back_off(attempt).await;
            let (_, v) = self.view().await?;
            let n = v.decisions.len();
            let index = match index {
                None => (0..n)
                    .rev()
                    .find(|i| !v.is_undone(*i) && !v.decisions[*i].has_undo_of())
                    .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "nothing to undo"))?,
                Some(i) => usize::try_from(i)
                    .ok()
                    .filter(|i| *i < n)
                    .ok_or_else(|| ApiError::not_found("no such decision"))?,
            };
            let now = pk_core::time::now_iso();
            let line = Decision(pk_kb::undo_line(&v, index, &now, by).map_err(kb_err)?);
            match self.append_once(&line).await {
                Ok(out) => return Ok(out),
                Err(Retry(e)) if attempt + 1 < RETRIES => {
                    tracing::info!(attempt, "undo: another writer took the place; building again: {e}");
                }
                Err(Retry(e)) => return Err(store_err(e)),
                Err(Final(e)) => return Err(e),
            }
        }
        unreachable!("the loop returns on its last attempt")
    }
}

/// What one append attempt came to.
enum Appended {
    /// Another writer took the place: read again and retry.
    Retry(StoreError),
    /// Answered as it is.
    Final(ApiError),
}
use Appended::{Final, Retry};

/// What a route's own check over the current items says before an append.
pub enum Pre {
    /// Append.
    Go,
    /// Nothing to append: answer this instead (a job already queued, a pair already dismissed).
    Done(StatusCode, Value),
}

/// What [`Creative::append_if`] came to.
pub enum Written {
    /// The row landed: the item the event named, as `Store.append` returns it.
    Landed(Value),
    /// The check said there was nothing to write.
    Skipped(StatusCode, Value),
}

/// The creative-store write primitives of one request (`Creative.stores(kb)[name].append`), each store read from
/// its rows as it is now. The caller holds [`AppState::creative_lock`] for as long as the Python holds `cr.lock`.
pub struct Creative {
    st: AppState,
}

impl Creative {
    /// Over this service's state.
    pub fn new(st: AppState) -> Self {
        Self { st }
    }

    fn replayed(&self, name: &str) -> ApiResult<&crate::creative::Replayed> {
        self.st
            .stores()
            .by_name(name)
            .ok_or_else(|| fault("KeyError", pk_core::py::quote(name)))
    }

    /// The store's items as its rows are now (`Store.items`).
    pub async fn items(&self, name: &str) -> ApiResult<Arc<Items>> {
        let store = self.st.store_handle();
        self.replayed(name)?
            .items(store.as_deref(), &self.st.config().owner_id)
            .await
            .map_err(|e| ApiError::internal("CreativeError", e))
    }

    /// The store's lines and items as they are now: what a route reads to mint an id (`st.n_events`).
    pub async fn lines_and_items(&self, name: &str) -> ApiResult<(Vec<String>, Arc<Items>)> {
        let store = self.st.store_handle().ok_or_else(not_connected)?;
        let lines = store.creative_lines(name).await.map_err(store_err)?;
        let items = self
            .replayed(name)?
            .replay(lines.clone(), &self.st.config().owner_id)
            .map_err(|e| ApiError::internal("CreativeError", e))?;
        Ok((lines, items))
    }

    /// [`Creative::append_if`] with no check of its own.
    pub async fn append(&self, name: &str, op: &str, id: &str, data: Value, by: &str) -> ApiResult<Value> {
        match self.append_if(name, op, id, data, by, |_| Ok(Pre::Go)).await? {
            Written::Landed(v) => Ok(v),
            Written::Skipped(..) => unreachable!("a check that always says go"),
        }
    }

    /// `LineStore.append(op, id, data, by)` behind the route's own check: the rows as they are now are replayed,
    /// `pre` runs over the items (it refuses, says there is nothing to do, or says go), the event is validated on a
    /// copy, and the next row is created (`Mirror.create`); when another writer took that row meanwhile, everything
    /// runs again from the rows as they are then.
    pub async fn append_if(
        &self,
        name: &str,
        op: &str,
        id: &str,
        data: Value,
        by: &str,
        pre: impl Fn(&Items) -> ApiResult<Pre>,
    ) -> ApiResult<Written> {
        let store = self.st.store_handle().ok_or_else(not_connected)?;
        let owner = self.st.config().owner_id.clone();
        for attempt in 0..RETRIES {
            back_off(attempt).await;
            let lines = store.creative_lines(name).await.map_err(store_err)?;
            let reducer =
                pk_creative::for_store(name, &owner).ok_or_else(|| fault("KeyError", pk_core::py::quote(name)))?;
            let mut s = pk_creative::Store::replay(reducer, &lines).map_err(|e| fault("ValueError", e.msg()))?;
            if let Pre::Done(status, v) = pre(s.items())? {
                return Ok(Written::Skipped(status, v));
            }
            let ev = Event::new(op, id, data.clone(), by, &pk_core::time::now_iso());
            // a bad op never reaches the log: the Python's `_apply(probe, ev)` raises a ValueError, a 500
            s.append_event(ev.clone()).map_err(|e| fault("ValueError", e.msg()))?;
            let line = pk_core::py::dumps(&serde_json::to_value(&ev).map_err(|e| fault("TypeError", e))?);
            let seq = i64::try_from(lines.len()).map_err(|e| fault("OverflowError", e))?;
            let row = CreativeEvent::new(name, seq, &line).map_err(store_err)?;
            match store.create_creative_event(&row).await {
                Ok(_) => {
                    // the next reader finds the replay warm, under exactly the lines just written
                    let item = s.get(id).cloned().unwrap_or(Value::Null);
                    let mut all = lines;
                    all.push(line);
                    self.replayed(name)?.store(all, Arc::new(s.items().clone()));
                    return Ok(Written::Landed(item));
                }
                Err(e) if is_place_taken(&e) && attempt + 1 < RETRIES => {
                    tracing::info!(store = name, attempt, "another writer took the row; reading again: {e}");
                }
                Err(e) => return Err(store_err(e)),
            }
        }
        unreachable!("the loop returns on its last attempt")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(n: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(n) = n {
            h.insert(header::CONTENT_LENGTH, HeaderValue::from_str(n).unwrap());
        }
        h
    }

    async fn read(n: Option<&str>, raw: &'static [u8]) -> Result<Map<String, Value>, String> {
        body_json(&headers(n), Body::from(raw)).await.map_err(|e| e.msg)
    }

    #[tokio::test]
    async fn the_body_is_read_the_way_the_python_reads_it() {
        assert_eq!(
            read(Some("8"), br#"{"a": 1}"#).await.unwrap(),
            serde_json::json!({"a": 1}).as_object().cloned().unwrap()
        );
        assert!(
            read(None, br#"{"a": 1}"#).await.unwrap().is_empty(),
            "no length: nothing is read"
        );
        assert!(read(Some("0"), br#"{"a": 1}"#).await.unwrap().is_empty());
        assert_eq!(read(Some("300000"), b"{").await.unwrap_err(), "body too large");
        assert_eq!(read(Some("3"), b"{x}").await.unwrap_err(), "body is not JSON");
        assert_eq!(read(Some("3"), b"[1]").await.unwrap_err(), "body must be an object");
        assert_eq!(
            read(Some("4"), b"\xff\xfe\xfd\xfc").await.unwrap_err(),
            "body is not JSON",
            "bytes that are not UTF-8 are not JSON either"
        );
    }

    #[test]
    fn ids_follow_id_re_under_re_match() {
        assert!(is_id("joke:a"));
        assert!(is_id("at:ab12CD34"));
        assert!(is_id("joke:a\n"));
        assert!(!is_id("joke a"));
        assert!(!is_id(""));
        assert!(!is_id(&"a".repeat(301)));
        assert!(is_id(&"a".repeat(300)));
    }

    #[test]
    fn a_taken_place_is_the_only_retry() {
        let taken = |msg: &str| StoreError::Statement {
            index: 1,
            msg: msg.into(),
            sql: String::new(),
        };
        assert!(is_place_taken(&taken(
            "Database index `decision_seq` already contains 5, with record `decision:x`"
        )));
        assert!(is_place_taken(&taken(
            "Database index `creative_event_store_seq` already contains ['jobs', 3], with record `creative_event:x`"
        )));
        assert!(!is_place_taken(&taken(
            "Database index `srcfile_path` already contains 'a'"
        )));
        assert!(!is_place_taken(&StoreError::mapping("a row that does not fit")));
        assert_eq!(
            store_err(StoreError::refused(503, "the database is reloading")).status,
            503
        );
        assert_eq!(
            store_err(StoreError::Gap { store: "jobs".into() }).msg,
            "creative_event jobs: sequence has gaps"
        );
    }
}
