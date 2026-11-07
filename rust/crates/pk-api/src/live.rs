//! The `/api/live` hub (`surreal_kb.LiveHub`): one `LIVE SELECT` subscription over the tables the app watches
//! (`pk_store::Store::subscribe`, on the shared connection: never a second session), the notifications coalesced
//! every 150 ms into one `{type: change, rev, count, changes, at}` event, and the event fanned out to every open
//! socket through a broadcast channel. The `rev` is the newest `meta:state.gen` the batch carried (`db:<gen>`): the
//! same generation `fn::apply_decision` bumped and every reader here and on the Python waits for, so a client that
//! refetches on the event is answered from that generation or a newer one, never an older.
//!
//! The subscription follows the connection: when the supervisor replaces the store (a failed `meta:state` read), the
//! loop notices within five seconds and subscribes again on the new one; a stream that ends or errors is retried
//! with a growing wait (1 s to 30 s), as the Python's hub reconnects.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::extract::ws::Utf8Bytes;
use futures::StreamExt as _;
use pk_store::Change;
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::state::AppState;

/// How often pending changes are flushed into one event.
pub const FLUSH_EVERY: Duration = Duration::from_millis(150);
/// How many changes one event lists (`changes[:50]`).
pub const MAX_CHANGES: usize = 50;
/// How many events a slow client may fall behind before it is told to refetch once (a lagged receiver).
const BACKLOG: usize = 64;

/// What a joke notification said: whether it is working, and how many sets it hit in.
type JokeSaid = (Option<bool>, Option<usize>);

struct Inner {
    tx: broadcast::Sender<Utf8Bytes>,
    pending: Mutex<Vec<Change>>,
    connected: AtomicBool,
    sent: AtomicU64,
    last_event_at: Mutex<Option<f64>>,
    /// What each joke's last notification said (`working`, how many sets it hit in): an update that says the same
    /// again moved nothing the app reads and is not reported (the sweeper's `UPDATE joke SET embedding`).
    jokes: Mutex<std::collections::HashMap<String, JokeSaid>>,
}

/// The hub: cheap to clone (one `Arc`).
#[derive(Clone)]
pub struct LiveHub(Arc<Inner>);

impl std::fmt::Debug for LiveHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveHub")
            .field("connected", &self.connected())
            .field("sent", &self.sent())
            .finish_non_exhaustive()
    }
}

impl Default for LiveHub {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveHub {
    /// Not subscribed yet (`run` subscribes).
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BACKLOG);
        Self(Arc::new(Inner {
            tx,
            pending: Mutex::new(Vec::new()),
            connected: AtomicBool::new(false),
            sent: AtomicU64::new(0),
            last_event_at: Mutex::new(None),
            jokes: Mutex::new(std::collections::HashMap::new()),
        }))
    }

    /// A receiver of the events from now on (one per open socket).
    pub fn subscribe(&self) -> broadcast::Receiver<Utf8Bytes> {
        self.0.tx.subscribe()
    }

    /// Whether the `LIVE SELECT` subscription is up (`hub.connected`, what `hello` says).
    pub fn connected(&self) -> bool {
        self.0.connected.load(Ordering::SeqCst)
    }

    /// How many events were sent since start.
    pub fn sent(&self) -> u64 {
        self.0.sent.load(Ordering::SeqCst)
    }

    /// When the last event was sent (`time.time()`), if any.
    pub fn last_event_at(&self) -> Option<f64> {
        *self.0.last_event_at.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How many sockets are receiving now.
    pub fn receivers(&self) -> usize {
        self.0.tx.receiver_count()
    }

    fn set_connected(&self, up: bool) {
        self.0.connected.store(up, Ordering::SeqCst);
    }

    /// One notification, reduced (`LiveHub._on`): queued for the next flush.
    pub fn push(&self, c: Change) {
        if c.table.as_deref() == Some("joke")
            && let Some(id) = c.id.clone()
        {
            let mut seen = self.0.jokes.lock().unwrap_or_else(PoisonError::into_inner);
            if c.action == "delete" {
                seen.remove(&id);
            } else if seen.insert(id, (c.working, c.hit_sets)) == Some((c.working, c.hit_sets)) && c.action == "update"
            {
                // the same working flag and hit sets as the last time: not worth an event (a created or a first-seen
                // joke is always reported; so is any change)
                return;
            }
        }
        self.0.pending.lock().unwrap_or_else(PoisonError::into_inner).push(c);
    }

    /// `LiveHub._flush_loop`'s one pass: the pending changes become one event unless the newest is a running reload
    /// (`meta.loading`: one event when it ends). Returns the event sent, if any.
    pub fn flush_once(&self) -> Option<Utf8Bytes> {
        let batch = {
            let mut p = self.0.pending.lock().unwrap_or_else(PoisonError::into_inner);
            if p.is_empty() || p.last().is_some_and(reloading) {
                return None;
            }
            std::mem::take(&mut *p)
        };
        let at = crate::worker::unix_now();
        let text = Utf8Bytes::from(event(&batch, at).to_string());
        *self.0.last_event_at.lock().unwrap_or_else(PoisonError::into_inner) = Some(at);
        self.0.sent.fetch_add(1, Ordering::SeqCst);
        // no receiver = nobody listening: the event is simply dropped, as the Python's queue-less hub drops it
        let _ = self.0.tx.send(text.clone());
        Some(text)
    }

    /// `LiveHub._flush_loop`: every [`FLUSH_EVERY`], forever.
    pub async fn flush_loop(self) {
        loop {
            tokio::time::sleep(FLUSH_EVERY).await;
            self.flush_once();
        }
    }

    /// What a socket gets when the hub subscribed again (after a lost connection): a change with no `rev`, so it
    /// refetches once for whatever happened while nothing was watching. Nothing is sent when no socket is open.
    fn announce_gap(&self) {
        if self.0.tx.receiver_count() > 0 {
            let _ = self.0.tx.send(Utf8Bytes::from(catch_up(0, crate::worker::unix_now())));
        }
    }

    /// `LiveHub._run`: subscribe on the current connection and queue every notification; subscribe again when the
    /// connection is replaced or the stream ends. Runs until the process ends.
    pub async fn run(self, st: AppState) {
        let mut backoff = 1u64;
        let mut subscribed_before = false;
        loop {
            let Some(store) = st.store_handle() else {
                self.set_connected(false);
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            };
            match store.subscribe().await {
                Ok(mut changes) => {
                    self.set_connected(true);
                    backoff = 1;
                    tracing::info!("live: subscribed");
                    if subscribed_before {
                        self.announce_gap();
                    }
                    subscribed_before = true;
                    // the check that the supervisor has not replaced the connection runs every five seconds
                    // whatever the notification rate (an interval, not a sleep rebuilt per notification)
                    let mut tick = tokio::time::interval(Duration::from_secs(5));
                    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                    tick.tick().await;
                    loop {
                        tokio::select! {
                            next = changes.next() => match next {
                                Some(Ok(c)) => self.push(c),
                                Some(Err(e)) => {
                                    tracing::warn!(error = %e, "live: the subscription failed; subscribing again");
                                    break;
                                }
                                None => {
                                    tracing::warn!("live: the subscription ended; subscribing again");
                                    break;
                                }
                            },
                            _ = tick.tick() => {
                                // the supervisor replaced the connection: this stream is dead with the old one
                                if !st.store_handle().is_some_and(|s| Arc::ptr_eq(&s, &store)) {
                                    tracing::info!("live: the connection was replaced; subscribing again");
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, backoff, "live: could not subscribe; retrying");
                }
            }
            self.set_connected(false);
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(30);
        }
    }
}

/// `any(c.get("table") == "meta" and c.get("loading") for c in self.pending[-1:])`.
fn reloading(c: &Change) -> bool {
    c.table.as_deref() == Some("meta") && c.loading == Some(true)
}

/// The event one batch becomes (`LiveHub._flush_loop`): `rev` from the newest `meta` change that carried a
/// generation, `count` of the whole batch, the first 50 non-`meta` changes, and when.
pub fn event(batch: &[Change], at: f64) -> Value {
    let generation = batch.iter().rev().find_map(|c| c.generation);
    let changes: Vec<&Change> = batch
        .iter()
        .filter(|c| c.table.as_deref() != Some("meta"))
        .take(MAX_CHANGES)
        .collect();
    json!({
        "type": "change",
        "rev": generation.map(|g| format!("db:{g}")),
        "count": batch.len(),
        "changes": changes,
        "at": at,
    })
}

/// The event a client that fell behind gets instead of the ones it missed: a change with no `rev`, so it refetches
/// once (the app refetches on any `change`).
pub fn catch_up(missed: u64, at: f64) -> String {
    json!({"type": "change", "rev": Value::Null, "count": missed, "changes": [], "at": at}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(table: &str, action: &str, id: &str) -> Change {
        Change {
            table: Some(table.into()),
            action: action.into(),
            id: Some(id.into()),
            working: None,
            hit_sets: None,
            store: None,
            op: None,
            item: None,
            generation: None,
            loading: None,
        }
    }

    fn meta(generation: i64, loading: Option<bool>) -> Change {
        Change {
            generation: Some(generation),
            loading,
            ..change("meta", "update", "meta:state")
        }
    }

    fn joke(action: &str, id: &str, working: bool, hit: usize) -> Change {
        Change {
            working: Some(working),
            hit_sets: Some(hit),
            ..change("joke", action, id)
        }
    }

    fn reported(hub: &LiveHub) -> Vec<(String, String)> {
        let ev: Value = serde_json::from_str(hub.flush_once().expect("an event").as_str()).expect("json");
        ev["changes"]
            .as_array()
            .expect("changes")
            .iter()
            .map(|c| {
                (
                    c["action"].as_str().unwrap().to_string(),
                    c["id"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn a_joke_update_that_says_the_same_again_is_not_reported() {
        let hub = LiveHub::new();
        // created, then the sweeper's updates (nothing the app reads moved), then a real change, then a repeat of it
        for c in [
            joke("create", "joke:a", false, 0),
            joke("update", "joke:a", false, 0),
            joke("update", "joke:a", false, 0),
            joke("update", "joke:a", true, 1),
            joke("update", "joke:a", true, 1),
            joke("update", "joke:b", true, 2),
            change("decision", "create", "decision:x"),
        ] {
            hub.push(c);
        }
        assert_eq!(
            reported(&hub),
            [
                ("create".into(), "joke:a".into()),
                ("update".into(), "joke:a".into()),
                ("update".into(), "joke:b".into()),
                ("create".into(), "decision:x".into()),
            ],
            "the repeats of joke:a are gone, the first sight of joke:b and the decision stay"
        );
        // a delete is always reported, and the joke is a stranger again after it
        hub.push(joke("delete", "joke:a", false, 0));
        hub.push(joke("update", "joke:a", true, 1));
        assert_eq!(
            reported(&hub),
            [("delete".into(), "joke:a".into()), ("update".into(), "joke:a".into())]
        );
    }

    #[test]
    fn a_batch_becomes_the_pythons_event() {
        let mut d = change("decision", "create", "decision:x");
        d.working = None;
        let mut j = change("joke", "update", "joke:a");
        j.working = Some(true);
        j.hit_sets = Some(2);
        let batch = vec![d, meta(4, Some(false)), j, meta(5, Some(false))];
        let ev = event(&batch, 12.5);
        assert_eq!(ev["type"], "change");
        assert_eq!(ev["rev"], "db:5", "the newest generation in the batch");
        assert_eq!(ev["count"], 4);
        assert_eq!(ev["at"], 12.5);
        assert_eq!(
            ev["changes"],
            json!([{"table": "decision", "action": "create", "id": "decision:x"},
                   {"table": "joke", "action": "update", "id": "joke:a", "working": true, "hit_sets": 2}])
        );
        let ev = event(&[change("creative_event", "create", "creative_event:1")], 1.0);
        assert!(ev["rev"].is_null(), "no meta change: no rev");
        let many: Vec<Change> = (0..70)
            .map(|i| change("joke", "update", &format!("joke:{i}")))
            .collect();
        assert_eq!(event(&many, 1.0)["changes"].as_array().map(Vec::len), Some(50));
        assert_eq!(event(&many, 1.0)["count"], 70);
    }

    #[test]
    fn a_running_reload_holds_the_flush_until_it_ends() {
        let hub = LiveHub::new();
        assert!(hub.flush_once().is_none(), "nothing pending");
        hub.push(change("joke", "update", "joke:a"));
        hub.push(meta(3, Some(true)));
        assert!(hub.flush_once().is_none(), "the newest change is a reload in progress");
        hub.push(meta(4, Some(false)));
        let text = hub.flush_once().expect("one event when the reload ends");
        let ev: Value = serde_json::from_str(&text).unwrap();
        assert_eq!((ev["rev"].as_str(), ev["count"].as_u64()), (Some("db:4"), Some(3)));
        assert_eq!(hub.sent(), 1);
        assert!(hub.last_event_at().is_some());
        assert!(hub.flush_once().is_none(), "flushed: nothing left");
    }

    #[tokio::test]
    async fn receivers_get_each_event_once() {
        let hub = LiveHub::new();
        let mut a = hub.subscribe();
        let mut b = hub.subscribe();
        assert_eq!(hub.receivers(), 2);
        hub.push(change("decision", "create", "decision:1"));
        let text = hub.flush_once().unwrap();
        assert_eq!(a.recv().await.unwrap(), text);
        assert_eq!(b.recv().await.unwrap(), text);
        assert!(a.try_recv().is_err());
        let cu: Value = serde_json::from_str(&catch_up(3, 1.0)).unwrap();
        assert_eq!(
            (cu["type"].as_str(), cu["rev"].is_null(), cu["count"].as_u64()),
            (Some("change"), true, Some(3))
        );
    }
}
