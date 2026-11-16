//! "Not connected", remembered (`docs/design/SPEC-2026-11-16-bits-sets-tapes.md` §3.1): the pairs he said do not belong
//! together, so a suggestion that is computed (no stored edge to reject) stays gone. One item per pair, keyed by the
//! sorted pair ([`pair_id`]); `dismiss {a, b, group, reason?}` and `undismiss` (his Undo). Rust only (B5: a new store,
//! never a new op on a store another process replays): no Python twin, no golden; `tests` below are the proof.

use std::collections::HashSet;

use serde_json::{Value, json};
use sha1::{Digest as _, Sha1};

use super::{item_mut, unknown_op};
use crate::error::{CreativeError, CreativeResult};
use crate::event::Event;
use crate::store::{Items, Reducer};
use pk_core::py;

/// The groups of "Around this bit" a pair can be dismissed from (the group he was looking at; a record, not a key).
pub const LINK_GROUPS: [&str; 5] = ["same_idea", "callbacks", "on_stage", "loose", "same_shape"];
/// His reason, at most (code points).
pub const MAX_REASON_CHARS: usize = 300;

/// Whether `r` names something a bit can be tied to: a bit, a taped moment, a line (a version unit) or a taped night.
pub fn is_link_ref(r: &str) -> bool {
    ["joke:", "perf:", "unit:", "set:"]
        .iter()
        .any(|p| r.strip_prefix(p).is_some_and(|rest| !rest.is_empty()))
}

/// The pair in its one order.
pub fn sorted_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// The item id of a pair: the same whichever end is named first.
pub fn pair_id(a: &str, b: &str) -> String {
    let (x, y) = sorted_pair(a, b);
    format!("lk:{}", &hex::encode(Sha1::digest(format!("{x}|{y}").as_bytes()))[..16])
}

/// Whether an item of the store is a pair dismissed now (not undone).
pub fn is_dismissed(item: &Value) -> bool {
    item.get("status").and_then(Value::as_str) == Some("dismissed")
}

/// The pairs dismissed now, each in its one order.
pub fn dismissed_pairs(items: &Items) -> HashSet<(String, String)> {
    items
        .values()
        .filter(|x| is_dismissed(x))
        .filter_map(|x| Some(sorted_pair(x.get("a")?.as_str()?, x.get("b")?.as_str()?)))
        .collect()
}

/// The links reducer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Links;

impl Reducer for Links {
    fn apply(&self, items: &mut Items, ev: &Event) -> CreativeResult<()> {
        let (op, id) = (ev.op.as_str(), ev.id.as_str());
        match op {
            "dismiss" => {
                let d = ev.data()?;
                let end = |k: &str| d.get(k).and_then(Value::as_str).filter(|r| is_link_ref(r));
                let (Some(a), Some(b)) = (end("a"), end("b")) else {
                    return Err(CreativeError::refused(
                        "a and b must each be a bit, a taped moment, a line or a taped night",
                    ));
                };
                if a == b {
                    return Err(CreativeError::refused("a pair needs two different ends"));
                }
                if id != pair_id(a, b) {
                    return Err(CreativeError::refused("id must be the pair's id"));
                }
                let group = d.get("group").and_then(Value::as_str).unwrap_or("");
                if !LINK_GROUPS.contains(&group) {
                    return Err(CreativeError::refused(format!(
                        "group must be one of {}",
                        LINK_GROUPS.join(", ")
                    )));
                }
                let reason = d.get("reason").and_then(Value::as_str).unwrap_or("");
                if py::len(reason) > MAX_REASON_CHARS {
                    return Err(CreativeError::refused(format!(
                        "a reason is at most {MAX_REASON_CHARS} characters"
                    )));
                }
                if items.get(id).is_some_and(is_dismissed) {
                    return Err(CreativeError::refused("that pair is already dismissed"));
                }
                let (lo, hi) = sorted_pair(a, b);
                let created = items
                    .get(id)
                    .and_then(|i| i.get("created_at"))
                    .cloned()
                    .unwrap_or_else(|| json!(ev.ts));
                items.insert(
                    id.to_string(),
                    json!({"id": id, "a": lo, "b": hi, "group": group, "reason": reason, "status": "dismissed",
                           "by": ev.by, "created_at": created, "updated_at": ev.ts}),
                );
            }
            "undismiss" => {
                if !items.get(id).is_some_and(is_dismissed) {
                    return Err(CreativeError::refused(format!("no dismissed pair {id}")));
                }
                let rec = item_mut(items, id).expect("checked above");
                rec.insert("status".into(), json!("undone"));
                rec.insert("undone_at".into(), json!(ev.ts));
                rec.insert("updated_at".into(), json!(ev.ts));
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
        Event::new(op, id, data, "owner", "2026-11-16T12:00:00+00:00")
    }

    #[test]
    fn a_pair_has_one_id_whichever_end_comes_first() {
        assert_eq!(pair_id("joke:a", "perf:p"), pair_id("perf:p", "joke:a"));
        assert_ne!(pair_id("joke:a", "joke:b"), pair_id("joke:a", "joke:c"));
        assert!(pair_id("joke:a", "joke:b").starts_with("lk:"));
        assert_eq!(pair_id("joke:a", "joke:b").len(), 19);
        // the separator keeps `a|b` + `c` apart from `a` + `b|c`: ids cannot hold a bar (the API's id pattern)
        assert_eq!(sorted_pair("joke:b", "joke:a"), ("joke:a".into(), "joke:b".into()));
    }

    #[test]
    fn dismiss_then_undismiss_then_dismiss_again() {
        let mut items = Items::new();
        let id = pair_id("joke:fridge", "joke:doorbell");
        let data = json!({"a": "joke:fridge", "b": "joke:doorbell", "group": "same_idea"});
        Links.apply(&mut items, &ev("dismiss", &id, data.clone())).unwrap();
        assert_eq!(items[&id]["a"], "joke:doorbell", "stored in the pair's one order");
        assert_eq!(items[&id]["b"], "joke:fridge");
        assert_eq!(items[&id]["status"], "dismissed");
        assert_eq!(
            dismissed_pairs(&items),
            HashSet::from([("joke:doorbell".to_string(), "joke:fridge".to_string())])
        );
        Links.apply(&mut items, &ev("undismiss", &id, json!({}))).unwrap();
        assert_eq!(items[&id]["status"], "undone");
        assert!(dismissed_pairs(&items).is_empty(), "an undone pair is suggested again");
        // the other end first: the same item, dismissed again
        let swapped =
            json!({"a": "joke:doorbell", "b": "joke:fridge", "group": "same_shape", "reason": "different turn"});
        Links.apply(&mut items, &ev("dismiss", &id, swapped)).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[&id]["group"], "same_shape");
        assert_eq!(dismissed_pairs(&items).len(), 1);
    }

    #[test]
    fn every_refusal_says_why_and_writes_nothing() {
        let id = pair_id("joke:a", "joke:b");
        let ok = json!({"a": "joke:a", "b": "joke:b", "group": "loose"});
        let cases = [
            (
                ev("dismiss", &id, json!({"a": "joke:a", "group": "loose"})),
                "a and b must each be a bit, a taped moment, a line or a taped night",
            ),
            (
                ev("dismiss", &id, json!({"a": "joke:a", "b": "label:x", "group": "loose"})),
                "a and b must each be a bit, a taped moment, a line or a taped night",
            ),
            (
                ev("dismiss", &id, json!({"a": "joke:a", "b": "joke:a", "group": "loose"})),
                "a pair needs two different ends",
            ),
            (ev("dismiss", "lk:other", ok.clone()), "id must be the pair's id"),
            (
                ev("dismiss", &id, json!({"a": "joke:a", "b": "joke:b", "group": "next"})),
                "group must be one of same_idea, callbacks, on_stage, loose, same_shape",
            ),
            (
                ev(
                    "dismiss",
                    &id,
                    json!({"a": "joke:a", "b": "joke:b", "group": "loose", "reason": "x".repeat(301)}),
                ),
                "a reason is at most 300 characters",
            ),
            (ev("undismiss", &id, json!({})), "no dismissed pair lk:"),
            (ev("delete", &id, json!({})), "unknown op delete"),
        ];
        for (e, want) in cases {
            let mut items = Items::new();
            match Links.apply(&mut items, &e) {
                Err(CreativeError::Refused(m)) => assert!(m.starts_with(want), "{m} / {want}"),
                other => panic!("expected a refusal, got {other:?}"),
            }
            assert_eq!(items.len(), 0);
        }
        // twice is refused (the route answers "already" before it appends), and an undone pair cannot be undone again
        let mut items = Items::new();
        Links.apply(&mut items, &ev("dismiss", &id, ok.clone())).unwrap();
        assert!(matches!(
            Links.apply(&mut items, &ev("dismiss", &id, ok)),
            Err(CreativeError::Refused(m)) if m == "that pair is already dismissed"
        ));
        Links.apply(&mut items, &ev("undismiss", &id, json!({}))).unwrap();
        assert!(Links.apply(&mut items, &ev("undismiss", &id, json!({}))).is_err());
    }

    #[test]
    fn link_refs_are_bits_moments_lines_and_nights() {
        for r in ["joke:a", "perf:p1", "unit:u", "set:2025-03-04_x"] {
            assert!(is_link_ref(r), "{r}");
        }
        for r in ["", "joke:", "label:x", "sb:1", "jokea"] {
            assert!(!is_link_ref(r), "{r}");
        }
    }
}
