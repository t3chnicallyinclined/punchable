//! His rant topics (the Rant tab's "My topics", `docs/design/SPEC-2026-11-06-rant-tab.md` §2, §7): `add` a typed
//! topic, mark it `ranted` with the rant bit it became, `delete` it. Rust only (R7): there is no Python reducer for
//! this store, so no golden; `tests` below are the proof, each refusal with its words.

use serde_json::{Value, json};

use super::unknown_op;
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::store::{Items, Reducer};
use pk_core::py;

/// A topic, at most (code points).
pub const MAX_TOPIC_CHARS: usize = 200;
/// Topics kept, at most: a list he reads on one screen, not an archive.
pub const MAX_TOPICS: usize = 200;

/// Whether `id` is a topic id this store mints (`topic:` + 1-40 of `[a-z0-9]`).
pub fn is_topic_id(id: &str) -> bool {
    id.strip_prefix("topic:")
        .is_some_and(|h| (1..=40).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

/// Whether `id` looks like a joke id (`joke:` and something).
fn is_joke_id(id: &str) -> bool {
    id.strip_prefix("joke:")
        .is_some_and(|r| !r.is_empty() && r.len() <= 290)
}

/// A topic's text as stored: stripped, one line, runs of whitespace one space.
pub fn clean_topic(text: &str) -> String {
    py::split(text).collect::<Vec<_>>().join(" ")
}

/// The topics reducer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Topics;

impl Reducer for Topics {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, id) = (ev.op.as_str(), ev.id.as_str());
        if !is_topic_id(id) {
            return Err(CreativeError::refused("id must be a topic id"));
        }
        match op {
            "add" => {
                if items.contains_key(id) {
                    return Err(CreativeError::refused(format!("topic {id} exists")));
                }
                if items.len() >= MAX_TOPICS {
                    return Err(CreativeError::refused(format!("at most {MAX_TOPICS} topics")));
                }
                let d = ev.data()?;
                let text = clean_topic(d.get("text").and_then(Value::as_str).unwrap_or(""));
                if text.is_empty() {
                    return Err(CreativeError::refused("a topic needs words"));
                }
                if py::len(&text) > MAX_TOPIC_CHARS {
                    return Err(CreativeError::refused(format!(
                        "a topic is at most {MAX_TOPIC_CHARS} characters"
                    )));
                }
                let from_trend = d.get("from_trend").cloned().unwrap_or(Value::Null);
                items.insert(
                    id.to_string(),
                    json!({"id": id, "text": text, "created": ev.ts, "ranted_at": null, "rant": null,
                           "from_trend": from_trend, "by": ev.by}),
                );
            }
            "ranted" => {
                let d = ev.data()?;
                let joke = d.get("joke").and_then(Value::as_str).unwrap_or("");
                if !is_joke_id(joke) {
                    return Err(CreativeError::refused("ranted needs the rant's joke id"));
                }
                let Some(t) = items.get_mut(id).and_then(Value::as_object_mut) else {
                    return Err(CreativeError::refused(format!("no topic {id}")));
                };
                t.insert("ranted_at".into(), json!(ev.ts));
                t.insert("rant".into(), json!(joke));
            }
            "delete" => {
                if items.shift_remove(id).is_none() {
                    return Err(CreativeError::refused(format!("no topic {id}")));
                }
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
        Event::new(op, id, data, "owner", "2026-11-07T12:00:00+00:00")
    }

    fn refusal(r: CreativeResult<()>) -> String {
        match r {
            Err(CreativeError::Refused(m)) => m,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_topic_is_added_ranted_and_deleted() {
        let mut items = Items::new();
        Topics
            .apply(
                &mut items,
                &ev("add", "topic:a1", json!({"text": "  self-checkout \n machines "})),
            )
            .unwrap();
        assert_eq!(items["topic:a1"]["text"], "self-checkout machines");
        assert_eq!(items["topic:a1"]["ranted_at"], Value::Null);
        Topics
            .apply(&mut items, &ev("ranted", "topic:a1", json!({"joke": "joke:rant-1"})))
            .unwrap();
        assert_eq!(items["topic:a1"]["rant"], "joke:rant-1");
        assert_eq!(items["topic:a1"]["ranted_at"], "2026-11-07T12:00:00+00:00");
        Topics.apply(&mut items, &ev("delete", "topic:a1", json!({}))).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn every_refusal_says_why() {
        let mut items = Items::new();
        Topics
            .apply(&mut items, &ev("add", "topic:a1", json!({"text": "x"})))
            .unwrap();
        let cases = [
            (ev("add", "joke:a1", json!({"text": "x"})), "id must be a topic id"),
            (ev("add", "topic:A1", json!({"text": "x"})), "id must be a topic id"),
            (ev("add", "topic:a1", json!({"text": "x"})), "topic topic:a1 exists"),
            (ev("add", "topic:b1", json!({"text": " \n "})), "a topic needs words"),
            (
                ev("add", "topic:b1", json!({"text": "x".repeat(201)})),
                "a topic is at most 200 characters",
            ),
            (
                ev("ranted", "topic:a1", json!({"joke": "unit:x"})),
                "ranted needs the rant's joke id",
            ),
            (ev("ranted", "topic:zz", json!({"joke": "joke:x"})), "no topic topic:zz"),
            (ev("delete", "topic:zz", json!({})), "no topic topic:zz"),
            (ev("rename", "topic:a1", json!({})), "unknown op rename"),
        ];
        for (e, want) in cases {
            let before = items.clone();
            assert_eq!(refusal(Topics.apply(&mut items, &e)), want, "{} {}", e.op, e.id);
            assert_eq!(items, before);
        }
    }

    #[test]
    fn ids_are_minted_ids_only() {
        assert!(is_topic_id("topic:0af9"));
        for bad in [
            "topic:",
            "topic:a-b",
            "topic:a/b",
            "topic:Ab",
            &format!("topic:{}", "a".repeat(41)),
        ] {
            assert!(!is_topic_id(bad), "{bad}");
        }
    }
}
