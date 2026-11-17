//! The redline rounds on his bits (`docs/design/SPEC-2026-11-16-bits-sets-tapes.md` §3.2 as built, Slice 3): one
//! item per bit, keyed by the joke id, holding the round he sent and is waiting on (or reading), and the last one he
//! closed. Ops:
//!
//! - `send {job, unit, on, n, marks}`: a round was sent (its `redline` job is in the `jobs` store). `unit` is the
//!   version he marked, `on` the job whose draft he marked instead (round two), `n` the round's number. It replaces the
//!   round before it: the next draft replaces the one he was reading.
//! - `resolve {job, verdict, unit?}`: he closed the open round: `used` (the draft is now version `unit`), `discarded`;
//!   or `open` on the round he last closed (Undo of a Use): that round is open again.
//!
//! The draft itself is the job's result, so a round that is waiting survives any restart: both are rows. Rust only
//! (B5: a new store, never a new op on a store another process replays): no Python twin, no golden; the tests below are
//! the proof. The reducer checks the SHAPE of an event only (gotcha 2026-11-17): what may be sent and closed is the
//! route's rule (`pk-api` `writes::redline`), and a resolve that names no round here is a no-op, never a refusal, so a
//! rule changed later can never make the store unreadable.

use serde_json::{Value, json};

use super::unknown_op;
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::store::{Items, Reducer};

/// The open round of a bit, when there is one.
pub fn open_round<'a>(items: &'a Items, joke: &str) -> Option<&'a Value> {
    items.get(joke).and_then(|x| x.get("round")).filter(|r| r.is_object())
}

/// The round he closed last, when there is one.
pub fn last_round<'a>(items: &'a Items, joke: &str) -> Option<&'a Value> {
    items.get(joke).and_then(|x| x.get("last")).filter(|r| r.is_object())
}

/// The redline reducer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Redline;

fn text_of<'a>(d: &'a serde_json::Map<String, Value>, k: &str) -> Option<&'a str> {
    d.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}

impl Reducer for Redline {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, id) = (ev.op.as_str(), ev.id.as_str());
        if id.is_empty() {
            return Err(CreativeError::refused("the joke is required"));
        }
        let d = ev.data()?;
        let old = items.get(id).cloned().unwrap_or_else(|| json!({}));
        let rounds = old.get("rounds").and_then(Value::as_i64).unwrap_or(0);
        match op {
            "send" => {
                let (Some(job), Some(unit)) = (text_of(d, "job"), text_of(d, "unit")) else {
                    return Err(CreativeError::refused("job and unit are required"));
                };
                let Some(marks) = d.get("marks").filter(|m| m.is_array()) else {
                    return Err(CreativeError::refused("marks must be a list"));
                };
                let round = json!({
                    "job": job, "unit": unit, "on": text_of(d, "on"), "n": d.get("n").and_then(Value::as_i64).unwrap_or(1),
                    "marks": marks, "sent_at": ev.ts, "by": ev.by,
                });
                items.insert(
                    id.to_string(),
                    json!({"id": id, "round": round, "last": old.get("last").cloned().unwrap_or(Value::Null),
                           "rounds": rounds + 1, "updated_at": ev.ts}),
                );
            }
            "resolve" => {
                let (Some(job), Some(verdict)) = (text_of(d, "job"), text_of(d, "verdict")) else {
                    return Err(CreativeError::refused("job and verdict are required"));
                };
                let round = old.get("round").filter(|r| r.is_object());
                let last = old.get("last").filter(|r| r.is_object());
                let is = |r: Option<&Value>| r.and_then(|x| x.get("job")).and_then(Value::as_str) == Some(job);
                let (round, last) = if verdict == "open" {
                    if !is(last) {
                        return Ok(()); // no such closed round: nothing to reopen
                    }
                    (
                        last.and_then(|l| l.get("round")).cloned().unwrap_or(Value::Null),
                        Value::Null,
                    )
                } else {
                    if !is(round) {
                        return Ok(()); // not the open round: nothing to close
                    }
                    let unit = d.get("unit").cloned().unwrap_or(Value::Null);
                    (
                        Value::Null,
                        json!({"job": job, "verdict": verdict, "unit": unit, "at": ev.ts, "round": round}),
                    )
                };
                items.insert(
                    id.to_string(),
                    json!({"id": id, "round": round, "last": last, "rounds": rounds, "updated_at": ev.ts}),
                );
            }
            _ => return Err(unknown_op(op)),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(op: &str, id: &str, data: Value) -> Event {
        Event::new(op, id, data, "owner", "2026-11-17T12:00:00+00:00")
    }

    fn send(items: &mut Items, job: &str, on: Option<&str>) {
        Redline
            .apply(
                items,
                &ev(
                    "send",
                    "joke:fridge",
                    json!({"job": job, "unit": "unit:v3", "on": on, "n": 1,
                                                  "marks": [{"type": "keep", "text": "the milk"}]}),
                ),
            )
            .unwrap();
    }

    #[test]
    fn a_round_waits_until_he_closes_it_and_the_next_replaces_it() {
        let mut items = Items::new();
        assert!(open_round(&items, "joke:fridge").is_none());
        send(&mut items, "job:redline-a", None);
        let r = open_round(&items, "joke:fridge").unwrap();
        assert_eq!(
            (&r["job"], &r["unit"], &r["on"]),
            (&json!("job:redline-a"), &json!("unit:v3"), &Value::Null)
        );
        // round two, on the draft: it replaces the first
        send(&mut items, "job:redline-b", Some("job:redline-a"));
        let r = open_round(&items, "joke:fridge").unwrap();
        assert_eq!(
            (&r["job"], &r["on"]),
            (&json!("job:redline-b"), &json!("job:redline-a"))
        );
        assert_eq!(items["joke:fridge"]["rounds"], 2);
        // a resolve of the replaced round is no-op
        let stale = ev(
            "resolve",
            "joke:fridge",
            json!({"job": "job:redline-a", "verdict": "used"}),
        );
        Redline.apply(&mut items, &stale).unwrap();
        assert!(open_round(&items, "joke:fridge").is_some());
        // used: closed, the version it became kept
        let used = ev(
            "resolve",
            "joke:fridge",
            json!({"job": "job:redline-b", "verdict": "used", "unit": "unit:v4"}),
        );
        Redline.apply(&mut items, &used).unwrap();
        assert!(open_round(&items, "joke:fridge").is_none());
        let l = last_round(&items, "joke:fridge").unwrap();
        assert_eq!((&l["verdict"], &l["unit"]), (&json!("used"), &json!("unit:v4")));
        // Undo of the Use: the same round is open again
        let reopen = ev(
            "resolve",
            "joke:fridge",
            json!({"job": "job:redline-b", "verdict": "open"}),
        );
        Redline.apply(&mut items, &reopen).unwrap();
        assert_eq!(open_round(&items, "joke:fridge").unwrap()["job"], "job:redline-b");
        assert!(last_round(&items, "joke:fridge").is_none());
    }

    #[test]
    fn every_refusal_says_why_and_writes_nothing() {
        let cases = [
            (
                ev("send", "joke:a", json!({"unit": "unit:v1", "marks": []})),
                "job and unit are required",
            ),
            (
                ev("send", "joke:a", json!({"job": "job:x", "unit": "", "marks": []})),
                "job and unit are required",
            ),
            (
                ev(
                    "send",
                    "joke:a",
                    json!({"job": "job:x", "unit": "unit:v1", "marks": {}}),
                ),
                "marks must be a list",
            ),
            (
                ev("resolve", "joke:a", json!({"job": "job:x"})),
                "job and verdict are required",
            ),
            (
                ev("send", "", json!({"job": "job:x", "unit": "unit:v1", "marks": []})),
                "the joke is required",
            ),
            (ev("forget", "joke:a", json!({})), "unknown op forget"),
        ];
        for (e, want) in cases {
            let mut items = Items::new();
            match Redline.apply(&mut items, &e) {
                Err(CreativeError::Refused(m)) => assert_eq!(m, want),
                other => panic!("expected a refusal, got {other:?}"),
            }
            assert_eq!(items.len(), 0);
        }
    }

    /// A close that names nothing open (a rule of the route changed later, a second phone) is replayed as nothing, never
    /// refused: the store stays readable whatever the log holds.
    #[test]
    fn a_resolve_that_names_no_round_is_replayed_as_nothing() {
        let mut items = Items::new();
        let e = ev("resolve", "joke:a", json!({"job": "job:none", "verdict": "discarded"}));
        Redline.apply(&mut items, &e).unwrap();
        let o = ev("resolve", "joke:a", json!({"job": "job:none", "verdict": "open"}));
        Redline.apply(&mut items, &o).unwrap();
        assert!(open_round(&items, "joke:a").is_none() && last_round(&items, "joke:a").is_none());
    }
}
