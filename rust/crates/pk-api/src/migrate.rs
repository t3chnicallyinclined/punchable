//! Rants and topics become sets (redesign slice 6, `docs/design/SPEC-2026-11-16-bits-sets-tapes.md` §3.6): one set
//! list per live topic (its rants first, oldest first, then the bits he kept for it in the order he kept them) and a
//! one-bit set list for each rant no topic holds. It only ADDS `create` events to the `storyboards` store, all in one
//! transaction: the `topics` and `script` stores, every bit and every version are untouched, and a topic's taped
//! moments stay on the topic (a moment is not a bit; the set page reads them back through `seed.topic`). Safe to run
//! twice: a topic (or a rant) that already has a list seeded from it, live, archived or deleted, is skipped.
//!
//! [`plan`] decides everything and is pure; [`rants_to_sets`] reads the stores, plans, and writes only when asked.
//! The binary `pk-rants-to-sets` (and `cargo xtask rants-to-sets`) runs it: a dry run by default.

use std::collections::HashSet;

use pk_core::py;
use pk_creative::Items;
use pk_creative::merge::sha8_value;
use pk_creative::util::slug;
use serde::Serialize;
use serde_json::{Value, json};

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::writes::Creative;

/// A board's title is at most this long (`POST /api/boards`).
pub const MAX_TITLE: usize = 200;

/// A set list holds at most this many bits (`POST /api/boards`'s `jokes_list`): a list the route could not make is
/// never made here either (security review).
pub const MAX_ITEMS: usize = 200;

/// A rant as `GET /api/rants` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RantRef {
    /// `joke:…`.
    pub id: String,
    /// The bit's title.
    pub title: String,
    /// Its short name (what a one-bit set is called).
    pub short_name: String,
}

/// One set list to make.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NewSet {
    /// The title (the topic's words, or the rant's short name).
    pub title: String,
    /// `{topic: id}` or `{rant: joke id}`.
    pub seed: Value,
    /// The bits, in order.
    pub joke_ids: Vec<String>,
}

/// What a run found and would make (counts only: no words of his).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    /// Live topics.
    pub topics: usize,
    /// Topics that get a set list.
    pub topics_made: usize,
    /// Topics that already have one (an earlier run).
    pub topics_skipped_already: usize,
    /// Deleted topics (no set list; the tombstone stays so its Undo works).
    pub tombstones: usize,
    /// Rants no topic holds.
    pub rants_without_topic: usize,
    /// Of those, the ones that get a one-bit set list.
    pub rant_boards_made: usize,
    /// Of those, the ones that already have one.
    pub rant_boards_skipped: usize,
    /// Bits across every set list to make.
    pub items_total: usize,
    /// Kept taped moments: not items, read back on the set page through the topic.
    pub also_kept_moments: usize,
    /// Rants and kept bits that are no longer bits (merged away, deleted): left out.
    pub missing_jokes_dropped: usize,
    /// Bits past a list's 200 (left out; they stay on the topic).
    pub over_cap_dropped: usize,
    /// Titles the court-material rules stopped: the list is made as "Untitled topic" / "Untitled rant".
    pub titles_withheld: usize,
}

/// The plan: the set lists to make, in order, and the counts.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Plan {
    /// The set lists to make.
    pub sets: Vec<NewSet>,
    /// What was found.
    pub counts: Counts,
}

/// The `seed.<key>` of every board, whatever its state (a list he deleted is not made again).
fn seeded(boards: &Items, key: &str) -> HashSet<String> {
    boards
        .values()
        .filter_map(|b| b.get("seed")?.get(key)?.as_str().map(str::to_string))
        .collect()
}

/// Cut to at most `n` characters.
fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// The plan over the stores as they are: `topics` and `boards` are the replayed `topics` and `storyboards` stores,
/// `rants` the rants as `GET /api/rants` lists them, `joke_exists` whether an id is a bit of the notebook now.
pub fn plan(topics: &Items, boards: &Items, rants: &[RantRef], joke_exists: impl Fn(&str) -> bool) -> Plan {
    let mut c = Counts::default();
    let mut sets = Vec::new();
    let done_topics = seeded(boards, "topic");
    let done_rants = seeded(boards, "rant");
    // every bit a topic names, live or deleted: a rant of a deleted topic is not "without a topic"
    let mut held: HashSet<String> = HashSet::new();
    for (id, t) in topics {
        let deleted = t.get("deleted_at").is_some_and(|d| !d.is_null());
        let rants_of: Vec<String> = t
            .get("rants")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        // the kept material, in the order he kept it (`at`, then as stored)
        let mut kept: Vec<(String, String)> = t
            .get("material")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter(|(_, x)| x.get("state").and_then(Value::as_str) == Some("kept"))
                    .map(|(r, x)| {
                        let at = x.get("at").and_then(Value::as_str).unwrap_or("").to_string();
                        (at, r.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();
        kept.sort_by(|a, b| a.0.cmp(&b.0));
        held.extend(rants_of.iter().cloned());
        held.extend(
            kept.iter()
                .filter(|(_, r)| r.starts_with("joke:"))
                .map(|(_, r)| r.clone()),
        );
        if deleted {
            c.tombstones += 1;
            continue;
        }
        c.topics += 1;
        if done_topics.contains(id) {
            c.topics_skipped_already += 1;
            continue;
        }
        let mut ids: Vec<String> = Vec::new();
        for j in rants_of
            .iter()
            .chain(kept.iter().map(|(_, r)| r).filter(|r| r.starts_with("joke:")))
        {
            if ids.contains(j) {
                continue;
            }
            if joke_exists(j) {
                ids.push(j.clone());
            } else {
                c.missing_jokes_dropped += 1;
            }
        }
        if ids.len() > MAX_ITEMS {
            c.over_cap_dropped += ids.len() - MAX_ITEMS;
            ids.truncate(MAX_ITEMS);
        }
        c.also_kept_moments += kept.iter().filter(|(_, r)| r.starts_with("perf:")).count();
        let text = py::strip(t.get("text").and_then(Value::as_str).unwrap_or("")).to_string();
        let title = if text.is_empty() {
            "Untitled topic".to_string()
        } else {
            cut(&text, MAX_TITLE)
        };
        c.topics_made += 1;
        c.items_total += ids.len();
        sets.push(NewSet {
            title,
            seed: json!({"topic": id}),
            joke_ids: ids,
        });
    }
    for r in rants {
        if held.contains(&r.id) {
            continue;
        }
        c.rants_without_topic += 1;
        if done_rants.contains(&r.id) {
            c.rant_boards_skipped += 1;
            continue;
        }
        if !joke_exists(&r.id) {
            c.missing_jokes_dropped += 1;
            continue;
        }
        let name = [r.short_name.as_str(), r.title.as_str()]
            .into_iter()
            .map(py::strip)
            .find(|s| !s.is_empty())
            .unwrap_or("Untitled rant");
        c.rant_boards_made += 1;
        c.items_total += 1;
        sets.push(NewSet {
            title: cut(name, MAX_TITLE),
            seed: json!({"rant": r.id}),
            joke_ids: vec![r.id.clone()],
        });
    }
    Plan { sets, counts: c }
}

/// The board ids for the sets, as `POST /api/boards` mints them (`sb:<slug>-<sha8([title, '', n])>`), `n` counting on
/// from the store's size and skipping any id already taken.
pub fn mint_ids(boards: &Items, sets: &[NewSet]) -> Vec<String> {
    let mut taken: HashSet<String> = boards.keys().cloned().collect();
    let mut n = boards.len();
    sets.iter()
        .map(|s| {
            loop {
                let id = format!("sb:{}-{}", slug(&s.title, 32), sha8_value(&json!([s.title, "", n])));
                n += 1;
                if taken.insert(id.clone()) {
                    break id;
                }
            }
        })
        .collect()
}

/// The result of a run.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Whether anything was written.
    pub wrote: bool,
    /// The plan as it stood when it was written (or would have been).
    pub plan: Plan,
    /// The ids of the set lists made (empty on a dry run).
    pub made: Vec<String>,
}

/// Read the stores and the notebook, plan, and write the `create` events when `write` (one transaction, under the
/// creative lock, over the rows as they are then).
pub async fn rants_to_sets(st: &AppState, write: bool) -> ApiResult<Report> {
    let owner = st.config().owner_id.clone();
    let _cr = st.creative_lock().lock().await;
    let d = st.data_for_write().await?;
    let corpus = d.corpus(st).await?;
    let view = st.data().await?.view(false).await?;
    let rants: Vec<RantRef> = view
        .jokes
        .iter()
        .filter_map(|jid| crate::writes::rant::rant_row(&view, jid))
        .map(|(_, _, row)| RantRef {
            id: row["id"].as_str().unwrap_or("").to_string(),
            title: row["title"].as_str().unwrap_or("").to_string(),
            short_name: row["short_name"].as_str().unwrap_or("").to_string(),
        })
        .collect();
    let cr = Creative::new(st.clone());
    let topics = cr.items("topics").await?;
    let boards = cr.items("storyboards").await?;
    let exists = |j: &str| {
        corpus
            .nodes
            .get(j)
            .is_some_and(|n| n.get("type").and_then(Value::as_str) == Some("joke"))
    };
    let mut p = plan(&topics, &boards, &rants, exists);
    // the court-material rules on every title before it is written (a topic stored before a rule tightened, a rant's
    // name): a title they stop becomes a neutral one; the bits are still listed (security review)
    for set in &mut p.sets {
        if st.legal().is_legal(&set.title) {
            set.title = if set.seed.get("topic").is_some() {
                "Untitled topic"
            } else {
                "Untitled rant"
            }
            .to_string();
            p.counts.titles_withheld += 1;
        }
    }
    if !write || p.sets.is_empty() {
        return Ok(Report {
            wrote: false,
            plan: p,
            made: Vec::new(),
        });
    }
    let ids = mint_ids(&boards, &p.sets);
    let ops: Vec<(String, String, Value)> = p
        .sets
        .iter()
        .zip(&ids)
        .map(|(s, id)| {
            (
                "create".to_string(),
                id.clone(),
                json!({"title": s.title, "joke_ids": s.joke_ids, "target_s": 900, "notes": "", "seed": s.seed}),
            )
        })
        .collect();
    cr.append_all("storyboards", &ops, &owner)
        .await
        .map_err(|e| ApiError::internal("Migration", e.msg))?;
    Ok(Report {
        wrote: true,
        plan: p,
        made: ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(v: Value) -> Items {
        match v {
            Value::Object(m) => m,
            _ => panic!("an object"),
        }
    }

    fn rant(id: &str, short: &str) -> RantRef {
        RantRef {
            id: id.into(),
            title: format!("{short} title"),
            short_name: short.into(),
        }
    }

    fn topics() -> Items {
        items(json!({
            "topic:a": {"id": "topic:a", "text": "  Self-checkout machines ", "rants": ["joke:r1", "joke:r2"],
                "material": {
                    "joke:k2": {"state": "kept", "at": "2026-11-02T00:00:00Z"},
                    "joke:jot": {"state": "kept", "at": "2026-11-01T00:00:00Z", "jot": true},
                    "perf:s@1000": {"state": "kept", "at": "2026-11-03T00:00:00Z"},
                    "joke:drop": {"state": "dropped", "at": "2026-11-01T00:00:00Z"},
                    "joke:r1": {"state": "kept", "at": "2026-11-04T00:00:00Z"},
                    "joke:gone": {"state": "kept", "at": "2026-11-05T00:00:00Z"}}},
            "topic:dead": {"id": "topic:dead", "text": "old", "rants": ["joke:r3"], "material": {},
                "deleted_at": "2026-11-05T00:00:00Z"}
        }))
    }

    const LIVE: [&str; 6] = ["joke:r1", "joke:r2", "joke:r3", "joke:k2", "joke:jot", "joke:solo"];

    #[test]
    fn a_topic_becomes_its_rants_then_its_kept_bits_and_a_loose_rant_its_own_set() {
        let rants = [
            rant("joke:r1", "R1"),
            rant("joke:r3", "R3"),
            rant("joke:solo", "Solo"),
            rant("joke:nope", "N"),
        ];
        let p = plan(&topics(), &Items::new(), &rants, |j| LIVE.contains(&j));
        assert_eq!(p.sets.len(), 2);
        let s = &p.sets[0];
        assert_eq!(s.title, "Self-checkout machines");
        assert_eq!(s.seed, json!({"topic": "topic:a"}));
        assert_eq!(
            s.joke_ids,
            ["joke:r1", "joke:r2", "joke:jot", "joke:k2"],
            "rants first; kept in keep order; a kept rant once; dropped and moments are not items"
        );
        assert_eq!(p.sets[1].title, "Solo");
        assert_eq!(p.sets[1].seed, json!({"rant": "joke:solo"}));
        assert_eq!(
            p.counts,
            Counts {
                topics: 1,
                topics_made: 1,
                topics_skipped_already: 0,
                tombstones: 1,
                rants_without_topic: 2,
                rant_boards_made: 1,
                rant_boards_skipped: 0,
                items_total: 5,
                also_kept_moments: 1,
                missing_jokes_dropped: 2,
                over_cap_dropped: 0,
                titles_withheld: 0,
            },
            "the deleted topic's rant is held by it (no set of its own); a gone joke and a gone rant are dropped"
        );
    }

    #[test]
    fn a_second_run_makes_nothing() {
        let rants = [rant("joke:solo", "Solo")];
        let first = plan(&topics(), &Items::new(), &rants, |j| LIVE.contains(&j));
        let ids = mint_ids(&Items::new(), &first.sets);
        let mut boards = Items::new();
        for (s, id) in first.sets.iter().zip(&ids) {
            // archived and deleted lists count too: what he put away is not made again
            boards.insert(id.clone(), json!({"id": id, "seed": s.seed, "deleted": true}));
        }
        let again = plan(&topics(), &boards, &rants, |j| LIVE.contains(&j));
        assert_eq!(again.sets, Vec::new());
        assert_eq!(
            (again.counts.topics_skipped_already, again.counts.rant_boards_skipped),
            (1, 1)
        );
    }

    #[test]
    fn ids_are_minted_as_the_route_does_and_never_collide() {
        let s = |t: &str| NewSet {
            title: t.into(),
            seed: Value::Null,
            joke_ids: vec![],
        };
        let ids = mint_ids(&Items::new(), &[s("Same"), s("Same")]);
        assert_eq!(ids[0], format!("sb:same-{}", sha8_value(&json!(["Same", "", 0]))));
        assert_ne!(ids[0], ids[1]);
        let mut boards = Items::new();
        boards.insert(ids[0].clone(), json!({}));
        let next = mint_ids(&boards, &[s("Same")]);
        assert!(!boards.contains_key(&next[0]));
    }

    #[test]
    fn a_long_topic_is_cut_to_a_title() {
        let t = items(json!({"topic:l": {"id": "topic:l", "text": "x".repeat(300), "rants": []}}));
        let p = plan(&t, &Items::new(), &[], |_| true);
        assert_eq!(p.sets[0].title.chars().count(), MAX_TITLE);
    }

    #[test]
    fn a_list_never_holds_more_than_the_route_allows() {
        let many: Vec<String> = (0..250).map(|k| format!("joke:k{k}")).collect();
        let t = items(json!({"topic:big": {"id": "topic:big", "text": "big", "rants": many, "material": {}}}));
        let p = plan(&t, &Items::new(), &[], |_| true);
        assert_eq!(p.sets[0].joke_ids.len(), MAX_ITEMS);
        assert_eq!((p.counts.over_cap_dropped, p.counts.items_total), (50, MAX_ITEMS));
    }
}
