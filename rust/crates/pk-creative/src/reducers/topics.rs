//! His rant topics (the Rant tab, `docs/design/SPEC-2026-11-06-rant-tab.md` §2, §7, §12.7): `add` a typed topic, mark
//! it `ranted` with the rant bit it became, `rename` it, `delete` it and `undelete` it (a tombstone, so Undo brings
//! its material back), and work its material: `keep`, `drop`, `unmark` a bit or a taped moment, `jot` a typed idea.
//! Material is references on the topic, never a change to the bit (§12.1). Every op stamps `touched`, the tab's order.
//! Rust only (R7): there is no Python reducer for this store, so no golden; `tests` below are the proof, each refusal
//! with its words.

use serde_json::{Map, Value, json};

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

/// Whether `r` can be material: a bit or a taped moment (`joke:` or `perf:` and something).
pub fn is_material_ref(r: &str) -> bool {
    r.strip_prefix("joke:")
        .or_else(|| r.strip_prefix("perf:"))
        .is_some_and(|x| !x.is_empty() && x.len() <= 290)
}

/// Whether a stored topic is a tombstone (deleted, kept so Undo can bring it back).
pub fn is_deleted(t: &Value) -> bool {
    t.get("deleted_at").is_some_and(|d| !d.is_null())
}

/// The rants a topic has made, oldest first; an item older than `rants` reads as `[rant]`.
pub fn rants_of(t: &Value) -> Vec<&str> {
    match t.get("rants").and_then(Value::as_array) {
        Some(a) if !a.is_empty() => a.iter().filter_map(Value::as_str).collect(),
        _ => t.get("rant").and_then(Value::as_str).into_iter().collect(),
    }
}

/// The ideas a live topic holds as jots: they are not also "to sort" (§12.7).
pub fn held_jots(items: &Items) -> std::collections::HashSet<&str> {
    items
        .values()
        .filter(|t| !is_deleted(t))
        .filter_map(|t| t.get("material").and_then(Value::as_object))
        .flat_map(|m| {
            m.iter()
                .filter(|(_, x)| x.get("jot") == Some(&Value::Bool(true)))
                .map(|(r, _)| r.as_str())
        })
        .collect()
}

/// The live topic `id`, or the refusal.
fn live<'a>(items: &'a mut Items, id: &str) -> CreativeResult<&'a mut Map<String, Value>> {
    items
        .get_mut(id)
        .filter(|t| !is_deleted(t))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| CreativeError::refused(format!("no topic {id}")))
}

/// A topic's cleaned words, or the refusal `add` and `rename` share.
fn topic_text(d: &Map<String, Value>) -> CreativeResult<String> {
    let text = clean_topic(d.get("text").and_then(Value::as_str).unwrap_or(""));
    if text.is_empty() {
        return Err(CreativeError::refused("a topic needs words"));
    }
    if py::len(&text) > MAX_TOPIC_CHARS {
        return Err(CreativeError::refused(format!(
            "a topic is at most {MAX_TOPIC_CHARS} characters"
        )));
    }
    Ok(text)
}

/// The `ref` of a material op, or the refusal.
fn material_ref(d: &Map<String, Value>) -> CreativeResult<String> {
    let r = d.get("ref").and_then(Value::as_str).unwrap_or("");
    if is_material_ref(r) {
        Ok(r.to_string())
    } else {
        Err(CreativeError::refused("material is a bit or a taped moment"))
    }
}

/// The material map of a live topic.
fn material(t: &mut Map<String, Value>) -> CreativeResult<&mut Map<String, Value>> {
    t.entry("material")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| CreativeError::refused("material must be an object"))
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
                if items.values().filter(|t| !is_deleted(t)).count() >= MAX_TOPICS {
                    return Err(CreativeError::refused(format!("at most {MAX_TOPICS} topics")));
                }
                let d = ev.data()?;
                let text = topic_text(d)?;
                let from_trend = d.get("from_trend").cloned().unwrap_or(Value::Null);
                items.insert(
                    id.to_string(),
                    json!({"id": id, "text": text, "created": ev.ts, "touched": ev.ts, "ranted_at": null, "rant": null,
                           "rants": [], "material": {}, "from_trend": from_trend, "by": ev.by}),
                );
            }
            "ranted" => {
                let d = ev.data()?;
                let joke = d.get("joke").and_then(Value::as_str).unwrap_or("");
                if !is_joke_id(joke) {
                    return Err(CreativeError::refused("ranted needs the rant's joke id"));
                }
                let t = live(items, id)?;
                t.insert("ranted_at".into(), json!(ev.ts));
                t.insert("rant".into(), json!(joke));
                // the newest last, each rant once (another version of the same rant is not another rant)
                let rants = t.entry("rants").or_insert_with(|| json!([]));
                if let Some(a) = rants.as_array_mut() {
                    a.retain(|x| x.as_str() != Some(joke));
                    a.push(json!(joke));
                }
                t.insert("touched".into(), json!(ev.ts));
            }
            "rename" => {
                let text = topic_text(ev.data()?)?;
                let t = live(items, id)?;
                t.insert("text".into(), json!(text));
                t.insert("touched".into(), json!(ev.ts));
            }
            "keep" | "drop" | "jot" => {
                let r = material_ref(ev.data()?)?;
                if op == "jot" && !r.starts_with("joke:") {
                    return Err(CreativeError::refused("a jot is a typed idea"));
                }
                let t = live(items, id)?;
                let m = material(t)?;
                // a jot he un-keeps and keeps again is still a jot; a jot dropped is no longer kept, but still his
                let was_jot = m.get(&r).and_then(|x| x.get("jot")) == Some(&Value::Bool(true));
                let state = if op == "drop" { "dropped" } else { "kept" };
                let mut row = json!({"state": state, "at": ev.ts});
                if op == "jot" || was_jot {
                    row["jot"] = json!(true);
                }
                m.insert(r, row);
                t.insert("touched".into(), json!(ev.ts));
            }
            "unmark" => {
                let r = material_ref(ev.data()?)?;
                let t = live(items, id)?;
                material(t)?.shift_remove(&r);
                t.insert("touched".into(), json!(ev.ts));
            }
            "touch" => {
                live(items, id)?.insert("touched".into(), json!(ev.ts));
            }
            "delete" => {
                live(items, id)?.insert("deleted_at".into(), json!(ev.ts));
            }
            "undelete" => {
                let Some(t) = items.get_mut(id).and_then(Value::as_object_mut) else {
                    return Err(CreativeError::refused(format!("no topic {id}")));
                };
                if t.remove("deleted_at").is_none() {
                    return Err(CreativeError::refused(format!("topic {id} is not deleted")));
                }
                t.insert("touched".into(), json!(ev.ts));
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
        assert_eq!(items["topic:a1"]["rants"], json!(["joke:rant-1"]));
        Topics.apply(&mut items, &ev("delete", "topic:a1", json!({}))).unwrap();
        assert!(
            is_deleted(&items["topic:a1"]),
            "a tombstone, so Undo keeps its material"
        );
        Topics
            .apply(&mut items, &ev("undelete", "topic:a1", json!({})))
            .unwrap();
        assert!(!is_deleted(&items["topic:a1"]));
        assert_eq!(items["topic:a1"]["rant"], "joke:rant-1");
    }

    fn at(ts: &str, op: &str, data: Value) -> Event {
        Event::new(op, "topic:a1", data, "owner", ts)
    }

    #[test]
    fn material_is_kept_dropped_unmarked_and_every_op_touches() {
        let mut items = Items::new();
        let mut go = |ts: &str, op: &str, data: Value| {
            Topics.apply(&mut items, &at(ts, op, data)).unwrap();
            items.clone()
        };
        go("2026-11-07T01:00:00+00:00", "add", json!({"text": "x"}));
        let i = go("2026-11-07T02:00:00+00:00", "keep", json!({"ref": "joke:a"}));
        assert_eq!(i["topic:a1"]["material"]["joke:a"]["state"], "kept");
        assert_eq!(i["topic:a1"]["touched"], "2026-11-07T02:00:00+00:00");
        let i = go("2026-11-07T03:00:00+00:00", "drop", json!({"ref": "joke:a"}));
        assert_eq!(i["topic:a1"]["material"]["joke:a"]["state"], "dropped");
        let i = go("2026-11-07T04:00:00+00:00", "unmark", json!({"ref": "joke:a"}));
        assert_eq!(i["topic:a1"]["material"], json!({}));
        let i = go("2026-11-07T05:00:00+00:00", "jot", json!({"ref": "joke:j"}));
        assert_eq!(i["topic:a1"]["material"]["joke:j"]["jot"], true);
        let i = go("2026-11-07T06:00:00+00:00", "keep", json!({"ref": "perf:m"}));
        assert_eq!(i["topic:a1"]["material"]["perf:m"]["state"], "kept");
        let i = go(
            "2026-11-07T07:00:00+00:00",
            "rename",
            json!({"text": "  airport \n bins "}),
        );
        assert_eq!(i["topic:a1"]["text"], "airport bins");
        let i = go("2026-11-07T08:00:00+00:00", "touch", json!({}));
        assert_eq!(i["topic:a1"]["touched"], "2026-11-07T08:00:00+00:00");
        // the same rant again moves to the end and is listed once; a second rant keeps the first
        go("2026-11-07T09:00:00+00:00", "ranted", json!({"joke": "joke:r1"}));
        go("2026-11-07T10:00:00+00:00", "ranted", json!({"joke": "joke:r2"}));
        let i = go("2026-11-07T11:00:00+00:00", "ranted", json!({"joke": "joke:r1"}));
        assert_eq!(i["topic:a1"]["rants"], json!(["joke:r2", "joke:r1"]));
        assert_eq!(i["topic:a1"]["rant"], "joke:r1");
    }

    #[test]
    fn a_jot_stays_a_jot_and_a_held_jot_is_listed_while_the_topic_lives() {
        let mut items = Items::new();
        for e in [
            ev("add", "topic:a1", json!({"text": "x"})),
            ev("jot", "topic:a1", json!({"ref": "joke:j"})),
            ev("keep", "topic:a1", json!({"ref": "joke:k"})),
            ev("keep", "topic:a1", json!({"ref": "joke:j"})),
        ] {
            Topics.apply(&mut items, &e).unwrap();
        }
        assert_eq!(
            items["topic:a1"]["material"]["joke:j"]["jot"], true,
            "keeping a jot leaves it a jot"
        );
        assert_eq!(held_jots(&items).into_iter().collect::<Vec<_>>(), ["joke:j"]);
        Topics.apply(&mut items, &ev("delete", "topic:a1", json!({}))).unwrap();
        assert_eq!(held_jots(&items).len(), 0, "a deleted topic holds nothing");
    }

    #[test]
    fn an_older_item_without_rants_reads_as_its_rant() {
        assert_eq!(rants_of(&json!({"rant": "joke:r"})), ["joke:r"]);
        assert_eq!(rants_of(&json!({"rant": "joke:r", "rants": []})), ["joke:r"]);
        assert_eq!(
            rants_of(&json!({"rant": "joke:b", "rants": ["joke:a", "joke:b"]})),
            ["joke:a", "joke:b"]
        );
        assert_eq!(rants_of(&json!({"rant": null})).len(), 0);
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
            (ev("rename", "topic:a1", json!({})), "a topic needs words"),
            (ev("rename", "topic:zz", json!({"text": "y"})), "no topic topic:zz"),
            (
                ev("keep", "topic:a1", json!({"ref": "unit:x"})),
                "material is a bit or a taped moment",
            ),
            (ev("drop", "topic:a1", json!({})), "material is a bit or a taped moment"),
            (ev("keep", "topic:zz", json!({"ref": "joke:x"})), "no topic topic:zz"),
            (ev("jot", "topic:a1", json!({"ref": "perf:x"})), "a jot is a typed idea"),
            (ev("undelete", "topic:a1", json!({})), "topic topic:a1 is not deleted"),
            (ev("undelete", "topic:zz", json!({})), "no topic topic:zz"),
            (ev("merge", "topic:a1", json!({})), "unknown op merge"),
        ];
        for (e, want) in cases {
            let before = items.clone();
            assert_eq!(refusal(Topics.apply(&mut items, &e)), want, "{} {}", e.op, e.id);
            assert_eq!(items, before);
        }
    }

    #[test]
    fn a_deleted_topic_refuses_everything_but_undelete() {
        let mut items = Items::new();
        for e in [
            ev("add", "topic:a1", json!({"text": "x"})),
            ev("delete", "topic:a1", json!({})),
        ] {
            Topics.apply(&mut items, &e).unwrap();
        }
        for (op, d) in [
            ("delete", json!({})),
            ("touch", json!({})),
            ("rename", json!({"text": "y"})),
            ("keep", json!({"ref": "joke:x"})),
            ("ranted", json!({"joke": "joke:x"})),
        ] {
            assert_eq!(
                refusal(Topics.apply(&mut items, &ev(op, "topic:a1", d))),
                "no topic topic:a1",
                "{op}"
            );
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
