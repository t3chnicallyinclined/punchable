//! The set lists and his sets as their list screens show them: `creative_api.board_summary` (`GET /api/boards`),
//! `mysets.summary` (`GET /api/mysets`) and `creative_api.tonight_board` (the Tonight screen's list). The rules they
//! read (`target_band`, `_transition_words`, `story.health`, `mysets.tough_spots`, ...) live in one home,
//! `pk_creative::setlist` and `pk_creative::mysets`; this module picks the items and shapes the rows.

use pk_core::py;
use pk_creative::ask::mmss;
use pk_creative::corpus::{Corpus, round_to};
use pk_creative::setlist::{ids_of, int_or_zero, target_band, transition_words};
use serde_json::{Map, Value, json};

use crate::creative::GenCorpus;
use crate::display::short_name_of;

type Obj = Map<String, Value>;

/// `mysets.LENGTHS`: the set lengths the app offers.
pub use pk_creative::mysets::LENGTHS;

fn get(m: &Obj, k: &str) -> Value {
    m.get(k).cloned().unwrap_or(Value::Null)
}

fn sget<'a>(m: &'a Obj, k: &str) -> Option<&'a str> {
    m.get(k).and_then(Value::as_str)
}

fn truthy(v: Option<&Value>) -> bool {
    py::truthy(v)
}

/// The paper-list name of a bit (`short_name_of(node.title, node.short_name)`).
fn short_name(c: &Corpus, jid: &str) -> String {
    let n = c.joke(jid);
    short_name_of(
        n.and_then(|n| sget(n, "title")).unwrap_or(""),
        n.and_then(|n| sget(n, "short_name")),
    )
}

/// The `(updated_at, id)` key the lists sort and pick by, borrowed from the item.
fn recency(x: &Obj) -> (&str, &str) {
    (sget(x, "updated_at").unwrap_or(""), sget(x, "id").unwrap_or(""))
}

/// Python's `max(xs, key=lambda x: (x["updated_at"], x["id"]))`: the first of the greatest.
fn newest<'a>(xs: impl IntoIterator<Item = &'a Obj>) -> Option<&'a Obj> {
    xs.into_iter().fold(None, |best, x| match best {
        Some(b) if recency(x) <= recency(b) => Some(b),
        _ => Some(x),
    })
}

/// Sort newest first by `(updated_at, id)` (Python's `sort(key=..., reverse=True)`, which keeps equal keys in order;
/// a stable sort on the reversed order does the same).
fn sort_newest(xs: &mut [&Obj]) {
    xs.sort_by(|a, b| recency(a).cmp(&recency(b)).reverse());
}

// ------------------------------------------------------------------------------------------------ the set lists

/// `creative_api.board_summary(c, b)`: one row of `GET /api/boards`.
pub fn board_summary(c: &Corpus, b: &Obj) -> Value {
    let ids = ids_of(b.get("joke_ids"));
    // `sum(...)`: the builtin, compensated since 3.12
    let run = py::sum(ids.iter().map(|j| c.duration(j).0));
    let names: Vec<String> = ids.iter().filter(|j| c.is_joke(j)).map(|j| short_name(c, j)).collect();
    json!({"id": get(b, "id"), "title": get(b, "title"), "n": ids.len(), "runtime_s": round_to(run, 1),
           "runtime": mmss(run), "target_s": get(b, "target_s"), "updated": get(b, "updated_at"),
           "source": get(b, "source"), "names": names, "archived": truthy(b.get("archived")), "seed": get(b, "seed"),
           "by": get(b, "by")})
}

/// `GET /api/boards` (`Routes.boards`): the live set lists newest first, or the archived ones.
pub fn boards(c: &Corpus, items: &Obj, archived: bool) -> Value {
    let mut bs: Vec<&Obj> = items
        .values()
        .filter_map(Value::as_object)
        .filter(|b| !truthy(b.get("deleted")) && truthy(b.get("archived")) == archived)
        .collect();
    sort_newest(&mut bs);
    let rows: Vec<Value> = bs.into_iter().map(|b| board_summary(c, b)).collect();
    json!({"boards": rows, "archived": archived})
}

/// `creative_api.tonight_board(c, st)`: the one of his own boards he touched last (with bits, not archived, not
/// deleted) as tonight's list, built once (`notebook.build(c, b, cap=0)`), or `null` when he has none.
pub fn tonight_board(nb: &GenCorpus, items: &Obj) -> Value {
    let own = items.values().filter_map(Value::as_object).filter(|b| {
        !truthy(b.get("deleted"))
            && !truthy(b.get("archived"))
            && sget(b, "source") == Some("his")
            && truthy(b.get("joke_ids"))
    });
    let Some(b) = newest(own) else {
        return Value::Null;
    };
    let c: &Corpus = nb;
    let story = nb.story();
    // `story.health`: the bits on the board that are in the notebook, their run time (a plain `+=`), the transitions
    let ids: Vec<&str> = ids_of(b.get("joke_ids")).into_iter().filter(|j| c.is_joke(j)).collect();
    let mut total = 0.0;
    let durs: Vec<f64> = ids
        .iter()
        .map(|j| {
            let d = c.duration(j).0;
            total += d;
            d
        })
        .collect();
    let total = round_to(total, 1);
    let (lo, hi) = target_band(int_or_zero(b.get("target_s")));
    let weak: Vec<bool> = (0..ids.len())
        .map(|i| i > 0 && transition_words(&story.transition(ids[i - 1], ids[i])).1)
        .collect();
    let (status, words) = if hi == 0 {
        ("none", mmss(total))
    } else if total < lo as f64 {
        ("short", format!("{} to fill", mmss(lo as f64 - total)))
    } else if total > hi as f64 {
        ("over", format!("{} over", mmss(total - hi as f64)))
    } else {
        ("on", "on target".to_string())
    };
    let segments: Vec<Value> = ids
        .iter()
        .zip(&durs)
        .zip(&weak)
        .map(|((j, d), w)| json!({"joke_id": j, "short_name": short_name(c, j), "duration_s": d, "gap": w}))
        .collect();
    let first_gap = weak.iter().position(|w| *w).map_or(Value::Null, |i| {
        let (a, z) = (ids[i - 1], ids[i]);
        let name = |j: &str| short_name_of(&c.title(j), c.joke(j).and_then(|n| sget(n, "short_name")));
        json!({"from": a, "to": z, "from_name": name(a), "to_name": name(z)})
    });
    json!({"id": get(b, "id"), "title": get(b, "title"), "target_s": hi, "runtime_s": total, "runtime": mmss(total),
           "status": status, "status_words": words, "weak_n": weak.iter().filter(|w| **w).count(),
           "segments": segments, "source": get(b, "source"), "first_gap": first_gap})
}

// ------------------------------------------------------------------------------------------------ his sets

/// `GET /api/mysets` (`mysets_api.Routes.list`): his sets (working sets not deleted), newest first.
pub fn mysets(nb: &GenCorpus, items: &Obj, owner: &str) -> Value {
    let mut ws: Vec<&Obj> = items
        .values()
        .filter_map(Value::as_object)
        .filter(|w| !truthy(w.get("deleted")))
        .collect();
    sort_newest(&mut ws);
    let story = nb.story();
    let sets: Vec<Value> = ws
        .into_iter()
        .map(|w| pk_creative::mysets::summary(&story, nb, w, owner))
        .collect();
    json!({"sets": sets, "lengths": LENGTHS})
}

/// `GET /api/settings` (`Routes.settings`): the four settings over `SETTINGS_DEFAULTS`, when he last changed one,
/// and whether he has.
pub fn settings(items: &Obj) -> Value {
    let empty = Obj::new();
    let it = items.get("settings").and_then(Value::as_object).unwrap_or(&empty);
    let defaults = [
        ("provider", json!("hosted")),
        ("training", json!(false)),
        ("upload_video", json!(false)),
        ("cellular", json!(false)),
    ];
    let mut out: Obj = defaults
        .into_iter()
        .map(|(k, d)| (k.to_string(), it.get(k).cloned().unwrap_or(d)))
        .collect();
    out.insert("updated_at".into(), get(it, "updated_at"));
    out.insert("source".into(), json!(if it.is_empty() { "default" } else { "his" }));
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Obj {
        let Value::Object(m) = v else { panic!("an object") };
        m
    }

    #[test]
    fn newest_is_the_first_of_the_greatest_and_sorting_keeps_ties() {
        let a = obj(json!({"id": "a", "updated_at": "2024-01-02"}));
        let b = obj(json!({"id": "b", "updated_at": "2024-01-03"}));
        let c = obj(json!({"id": "c", "updated_at": "2024-01-03"}));
        assert_eq!(newest([&a, &b, &c]).and_then(|x| sget(x, "id")), Some("c"));
        let mut xs = vec![&a, &c, &b];
        sort_newest(&mut xs);
        let ids: Vec<&str> = xs.iter().filter_map(|x| sget(x, "id")).collect();
        assert_eq!(ids, ["c", "b", "a"]);
    }

    #[test]
    fn settings_fill_the_defaults_and_say_whose_they_are() {
        let none = settings(&Obj::new());
        assert_eq!(
            none,
            json!({"provider": "hosted", "training": false, "upload_video": false, "cellular": false,
                   "updated_at": null, "source": "default"})
        );
        let his = settings(&obj(
            json!({"settings": {"id": "settings", "provider": "key", "training": true,
                                                     "updated_at": "2024-01-01T00:00:00+00:00"}}),
        ));
        assert_eq!(his["provider"], "key");
        assert_eq!(his["training"], true);
        assert_eq!(his["cellular"], false);
        assert_eq!(his["source"], "his");
        assert!(his.get("id").is_none());
    }
}
