//! `GET /api/today` (`server.py` `do_GET`): the older feed (the latest set's card and how long its review takes,
//! the latest analysis, the glance, tonight's list, the pipeline counts) with `today.build` merged in
//! (`creative_api.merged_today`). `inbox` is how many uploads wait (`len(inbox_list(kb))`); they are rows in the
//! database since T10 group 6a, so the route sets it after `old_today` (which has no database and says 0).

use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::display::at_label;
use crate::index::STAGES;
use crate::pages::Ctx;

/// `TONIGHT_ID`.
pub const TONIGHT_ID: &str = "list:tonight";

/// `RANK`.
fn rank(r: Option<&str>) -> i64 {
    match r {
        Some("killed") => 3,
        Some("worked") => 2,
        Some("meh") => 1,
        Some("died") => 0,
        _ => -1,
    }
}

fn get(m: &Map<String, Value>, k: &str) -> Value {
    m.get(k).cloned().unwrap_or(Value::Null)
}

fn truthy(v: Option<&Value>) -> bool {
    pk_core::py::truthy(v)
}

/// The stage counts over the view's jokes: every stage at 0 first, then each joke's (`None` is `null`).
fn stage_counts(c: &Ctx<'_>) -> IndexMap<String, u64> {
    let mut counts: IndexMap<String, u64> = STAGES.iter().map(|s| ((*s).to_string(), 0)).collect();
    for j in &c.v.jokes {
        let st = c.v.nodes.get(j).and_then(|n| n.get("stage"));
        let k = match st {
            None | Some(Value::Null) => "null".to_string(),
            Some(v) => pk_core::py::str(v),
        };
        *counts.entry(k).or_default() += 1;
    }
    counts
}

/// `latest_analyzed(a)`: the newest set with analysis rows, else the newest set.
pub fn latest_analyzed<'a>(c: &Ctx<'a>) -> Option<&'a str> {
    let sets = &c.idx.sets;
    sets.iter()
        .rev()
        .find(|s| c.d.ai.rows.contains_key(s.as_str()))
        .or_else(|| sets.last())
        .map(String::as_str)
}

fn set_title(c: &Ctx<'_>, sid: &str) -> Value {
    c.v.nodes.get(sid).map_or(Value::Null, |n| get(n, "title"))
}

/// `glance(kb, a)`.
pub fn glance(c: &Ctx<'_>) -> Value {
    let counts = stage_counts(c);
    let ai_working =
        c.v.jokes
            .iter()
            .filter(|j| {
                c.v.nodes.get(*j).and_then(|n| n.get("stage")).and_then(Value::as_str) == Some("working")
                    && c.stage_ai(j)["stage_ai"] == json!(true)
            })
            .count();
    let s = latest_analyzed(c);
    let rows = s.and_then(|sid| c.analysis_rows(sid));
    let (mut new, mut hints) = (Vec::new(), Vec::new());
    if let (Some(sid), Some(rows)) = (s, rows.filter(|r| !r.is_empty())) {
        for r in rows
            .iter()
            .filter(|r| r.get("status").and_then(Value::as_str) == Some("new"))
        {
            let punch = match r.get("punchlines").and_then(Value::as_array).and_then(|p| p.last()) {
                Some(p) => p.get("text").cloned().unwrap_or(Value::Null),
                None => json!(
                    r.get("transcript")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .chars()
                        .take(160)
                        .collect::<String>()
                ),
            };
            new.push(
                json!({"set": sid, "idx": get(r, "idx"), "perf": get(r, "perf"), "start_ms": get(r, "start_ms"),
                            "end_ms": get(r, "end_ms"), "at_label": at_label(r.get("start_ms")),
                            "title": get(r, "suggested_title"), "punch": punch, "rating": get(r, "rating"),
                            "saved_as": get(r, "saved_as")}),
            );
        }
        let mut ranked: Vec<&Map<String, Value>> = rows.iter().filter(|r| truthy(r.get("joke"))).collect();
        let total = |r: &Map<String, Value>| {
            r.get("laughs")
                .and_then(|l| l.get("total_s"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        };
        ranked.sort_by(|a, b| {
            let ra = rank(a.get("rating").and_then(Value::as_str));
            let rb = rank(b.get("rating").and_then(Value::as_str));
            rb.cmp(&ra)
                .then_with(|| total(b).partial_cmp(&total(a)).unwrap_or(std::cmp::Ordering::Equal))
        });
        // one hint per joke, best jokes first, alternating "similar joke" and "label"
        let mut seen: Vec<Value> = Vec::new();
        for (k, r) in ranked.iter().enumerate() {
            if hints.len() >= 3 {
                break;
            }
            let order = if k % 2 == 0 {
                ["similar", "label"]
            } else {
                ["label", "similar"]
            };
            for kind in order {
                let xs = r
                    .get(if kind == "similar" { "similar" } else { "hints" })
                    .and_then(Value::as_array);
                let key = |x: &Value| {
                    x.get("joke_id")
                        .filter(|v| truthy(Some(v)))
                        .or_else(|| x.get("label_id"))
                        .cloned()
                        .unwrap_or(Value::Null)
                };
                let Some(x) = xs
                    .into_iter()
                    .flatten()
                    .find(|x| !seen.contains(&key(x)) && truthy(x.get("why")))
                else {
                    continue;
                };
                seen.push(key(x));
                let tgt = if kind == "similar" {
                    json!({"id": x["joke_id"], "title": x["title"]})
                } else {
                    json!({"id": x["label_id"], "title": x["name"]})
                };
                let joke = r.get("joke").cloned().unwrap_or(Value::Null);
                hints.push(json!({"kind": kind, "idx": get(r, "idx"),
                                  "joke": {"id": joke["id"], "title": joke["title"]}, "target": tgt, "why": x["why"]}));
                break;
            }
        }
    }
    hints.truncate(3);
    json!({"stages": counts, "ai_working": ai_working,
           "set": s.map_or(Value::Null, |sid| json!({"id": sid, "title": set_title(c, sid)})),
           "new_material": new, "hints": hints})
}

/// `tonight(a)`: his saved list for tonight, else a default from the bits he is testing and wants to try.
pub fn tonight(c: &Ctx<'_>) -> Value {
    let node = c.v.nodes.get(TONIGHT_ID);
    let saved = node.and_then(|n| n.get("items")).and_then(Value::as_array);
    let (items, source): (Vec<Value>, &str) = if let Some(items) = saved {
        (items.clone(), "saved")
    } else {
        fn stage(r: &crate::index::Row) -> &str {
            r.summary.stage.as_deref().unwrap_or("")
        }
        let seen = |r: &&crate::index::Row| r.summary.last_seen.clone().unwrap_or_default();
        let mut to_try: Vec<&crate::index::Row> = c.idx.summaries.iter().filter(|r| stage(r) == "to_try").collect();
        to_try.sort_by_key(seen);
        let mut testing: Vec<&crate::index::Row> = c.idx.summaries.iter().filter(|r| stage(r) == "testing").collect();
        testing.sort_by_key(seen);
        // `sorted(..., reverse = True)` over the ascending order: stable, equal dates keep their order
        testing.sort_by_key(|r| std::cmp::Reverse(seen(r)));
        let items = testing
            .iter()
            .take(4)
            .chain(to_try.iter().take(4))
            .map(|r| json!({"joke": r.summary.id, "done": false}))
            .collect();
        (items, "default")
    };
    let mut out = Vec::new();
    for it in &items {
        let Some(jid) = it.get("joke").and_then(Value::as_str) else {
            continue;
        };
        if c.v.ntype(Some(jid)) != Some("joke") {
            continue;
        }
        let mut b = c.brief_joke(jid);
        if let Value::Object(m) = &mut b {
            m.insert("done".into(), json!(truthy(it.get("done"))));
        }
        out.push(b);
    }
    let candidates: Vec<Value> = c
        .idx
        .summaries
        .iter()
        .filter(|r| matches!(r.summary.stage.as_deref(), Some("to_try" | "testing")))
        .map(|r| json!({"id": r.summary.id, "title": r.summary.title, "stage": r.summary.stage}))
        .collect();
    json!({"items": out, "source": source, "candidates": candidates})
}

/// The older feed (everything before `merged_today`).
pub fn old_today(c: &Ctx<'_>) -> Map<String, Value> {
    let latest = c.idx.sets.last().map(String::as_str);
    let card = latest.map(|sid| {
        let mut card = c.set_card(sid);
        let n = c.review_steps(sid).len();
        if let Value::Object(m) = &mut card {
            m.insert("moments".into(), json!(n));
            #[expect(clippy::cast_possible_truncation, reason = "minutes of review")]
            let mins = ((n as f64 * 25.0 / 60.0).round_ties_even() as i64).max(1);
            m.insert("review_minutes".into(), json!(mins));
        }
        card
    });
    let latest_an = latest_analyzed(c).and_then(|sid| {
        let sa = c.set_analysis(sid)?;
        let node = c.v.nodes.get(sid);
        let mut m = Map::new();
        m.insert("id".into(), json!(sid));
        m.insert("title".into(), node.map_or(Value::Null, |n| get(n, "title")));
        m.insert("date".into(), node.map_or(Value::Null, |n| get(n, "date")));
        m.insert("source".into(), sa.get("source").cloned().unwrap_or(Value::Null));
        for k in [
            "jokes_counted",
            "known_jokes",
            "new_material",
            "crowd_work",
            "laughs_per_min",
            "best",
            "weakest",
        ] {
            m.insert(
                k.into(),
                sa.pointer(&format!("/summary/{k}")).cloned().unwrap_or(Value::Null),
            );
        }
        Some(Value::Object(m))
    });
    let unreviewed = c
        .idx
        .sets
        .iter()
        .filter(|s| !truthy(c.set_card(s).get("reviewed")))
        .count();
    let mut out = Map::new();
    out.insert("latest_set".into(), card.unwrap_or(Value::Null));
    out.insert("unreviewed_sets".into(), json!(unreviewed));
    out.insert("latest_analysis".into(), latest_an.unwrap_or(Value::Null));
    out.insert("glance".into(), glance(c));
    out.insert("tonight".into(), tonight(c));
    out.insert("pipeline".into(), json!(stage_counts(c)));
    out.insert("next_show".into(), Value::Null);
    out.insert("inbox".into(), json!(0));
    out
}

/// `merged_today(old, c, ws)`: `today.build` over the older feed; `latest_set` is the older card with the build's
/// set fields under it (the card's own keys win).
pub fn merged_today(mut old: Map<String, Value>, t: Map<String, Value>) -> Map<String, Value> {
    for (k, v) in t {
        if k == "latest_set" {
            let card = old.get("latest_set").cloned().unwrap_or(Value::Null);
            let merged = if truthy(Some(&v)) || truthy(Some(&card)) {
                let mut m = v.as_object().cloned().unwrap_or_default();
                m.extend(card.as_object().cloned().unwrap_or_default());
                Value::Object(m)
            } else {
                Value::Null
            };
            old.insert(k, merged);
        } else {
            old.insert(k, v);
        }
    }
    old
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_wins_over_the_build_and_nothing_stays_nothing() {
        let old: Map<String, Value> = json!({"latest_set": {"title": "card", "id": "set:x"}, "inbox": 0})
            .as_object()
            .cloned()
            .unwrap();
        let t: Map<String, Value> = json!({"latest_set": {"title": "build", "material": "1:00"}, "date": "d"})
            .as_object()
            .cloned()
            .unwrap();
        let m = merged_today(old, t);
        assert_eq!(
            m["latest_set"],
            json!({"title": "card", "id": "set:x", "material": "1:00"})
        );
        assert_eq!(m["date"], "d");
        let m = merged_today(
            json!({"latest_set": null}).as_object().cloned().unwrap(),
            json!({"latest_set": null}).as_object().cloned().unwrap(),
        );
        assert_eq!(m["latest_set"], Value::Null);
        assert_eq!(rank(Some("killed")), 3);
        assert_eq!(rank(None), -1);
    }
}
