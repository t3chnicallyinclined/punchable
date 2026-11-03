//! The set lists and his sets as their list screens show them: `creative_api.board_summary` (`GET /api/boards`),
//! `mysets.summary` (`GET /api/mysets`) and `creative_api.tonight_board` (the Tonight screen's list), with the parts of
//! `creative/notebook.py` (`target_band`, `_transition_words`, `build`), `creative/story.py` (`health`) and
//! `creative/mysets.py` (`bit_label`, `tough_spots`, `health`, `runtime`, `length_status`) they read. Pure functions
//! over one generation's notebook and a replayed store item.
//!
//! Only what these payloads carry is computed: a transition's weakness (not its words), a bit's label (not its
//! words), how many tough spots a set has (not their texts or actions). Each is the Python's rule for that value
//! exactly; the texts follow when a route that shows them moves here.

use pk_core::py;
use pk_creative::ask::mmss;
use pk_creative::corpus::{Corpus, Perf, median, num, round_to};
use serde_json::{Map, Value, json};

use crate::creative::GenCorpus;
use crate::display::short_name_of;

type Obj = Map<String, Value>;

/// `notebook.BAND`: a 12:00 target means "10-12 min" (one minute under 8 min).
const BAND: i64 = 120;
/// `mysets.DEAD_ROOM`: a set whose whole room laughed under this (laugh-s/min) barely laughed.
const DEAD_ROOM: f64 = 2.0;
/// `mysets.KILL_VR`, `GOOD_VR`, `SOME_VR`: vs the room's own laugh rate that night.
const KILL_VR: f64 = 1.2;
const GOOD_VR: f64 = 0.8;
const SOME_VR: f64 = 0.5;
/// `mysets.LONG_QUIET_S`, `LONG_QUIET_SHARE`: a stretch with no laugh this long and this share of the bit.
const LONG_QUIET_S: f64 = 40.0;
const LONG_QUIET_SHARE: f64 = 0.4;
/// `mysets.RAN_LONG`: last night ran this many times its usual length (and 30 s over).
const RAN_LONG: f64 = 1.5;
/// `mysets.LENGTHS`: the set lengths the app offers.
pub const LENGTHS: [i64; 4] = [300, 420, 600, 900];

fn get(m: &Obj, k: &str) -> Value {
    m.get(k).cloned().unwrap_or(Value::Null)
}

fn sget<'a>(m: &'a Obj, k: &str) -> Option<&'a str> {
    m.get(k).and_then(Value::as_str)
}

fn truthy(v: Option<&Value>) -> bool {
    py::truthy(v)
}

/// `int(x or 0)` for a stored number (`bool` is 0/1, a float is cut toward zero).
fn int_or_zero(v: Option<&Value>) -> i64 {
    num(v.filter(|x| truthy(Some(x)))).map_or(0, |x| x.trunc() as i64)
}

/// `notebook.target_band(target_s)`: `(lo, hi)` seconds.
pub fn target_band(t: i64) -> (i64, i64) {
    ((t - if t >= 480 { BAND } else { 60 }).max(60), t)
}

/// `_transition_words(t)[1]`: whether the way into a bit is weak (nothing links the two, or only a shared world with no
/// line between them yet).
pub fn transition_weak(t: &Value) -> bool {
    match t.get("label").and_then(Value::as_str) {
        Some("proven") => false,
        Some("link") => {
            let kinds: Vec<&str> = t
                .get("via")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|v| v.get("kind").and_then(Value::as_str))
                .collect();
            let has = |k: &str| kinds.contains(&k);
            if has("segue") || has("callback_to") || has("same_premise") {
                false
            } else if has("persona") || has("topic") || has("subject") {
                true
            } else {
                truthy(t.get("needs_bridge"))
            }
        }
        _ => true,
    }
}

/// The joke ids of a list (`board["joke_ids"]`, a version's `bits`), as text (a non-text id is no joke here, as a
/// dict lookup with it finds nothing in the Python).
fn ids_of(v: Option<&Value>) -> Vec<&str> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|x| x.as_str().unwrap_or(""))
        .collect()
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
        .map(|i| i > 0 && transition_weak(&story.transition(ids[i - 1], ids[i])))
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

/// `mysets.night_outcome(p)`: one night of one bit, or `None` when the whole room barely laughed (not counted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Killed,
    Good,
    Some,
    Quiet,
}

fn night_outcome(p: &Perf) -> Option<Outcome> {
    outcome_of(p.set_laugh_s_per_min, p.vs_room, &p.ai_rating)
}

/// [`night_outcome`] over the three fields it reads: the room's laugh rate, `vs_room`, the AI's rating.
fn outcome_of(rate: Option<f64>, vs_room: Option<f64>, r: &str) -> Option<Outcome> {
    if rate.is_some_and(|r| r < DEAD_ROOM) {
        return None;
    }
    let Some(vr) = vs_room else {
        return Some(match r {
            "killed" => Outcome::Killed,
            "worked" => Outcome::Good,
            "meh" => Outcome::Some,
            _ => Outcome::Quiet,
        });
    };
    Some(if r == "died" || vr <= 0.0 {
        Outcome::Quiet
    } else if vr >= KILL_VR || (r == "killed" && vr >= 1.0) {
        Outcome::Killed
    } else if vr >= GOOD_VR {
        Outcome::Good
    } else if vr >= SOME_VR {
        Outcome::Some
    } else {
        Outcome::Quiet
    })
}

/// `mysets.bit_label(c, j)["label"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Label {
    Killed,
    Solid,
    Mixed,
    Untested,
    Quiet,
}

fn perfs<'c>(c: &'c Corpus, jid: &str) -> &'c [Perf] {
    c.perfs.get(jid).map_or(&[][..], Vec::as_slice)
}

fn bit_label(c: &Corpus, jid: &str) -> Label {
    let outs: Vec<Outcome> = perfs(c, jid).iter().filter_map(night_outcome).collect();
    let n = outs.len();
    if n == 0 {
        return Label::Untested;
    }
    let count = |o: Outcome| outs.iter().filter(|x| **x == o).count();
    let k = count(Outcome::Killed);
    let hits = k + count(Outcome::Good);
    let q = count(Outcome::Quiet);
    if q == 0 && k > 0 && 2 * k >= n && 3 * hits >= 2 * n {
        Label::Killed
    } else if q == 0 && 3 * hits >= 2 * n {
        Label::Solid
    } else if hits == 0 || 3 * q >= 2 * n {
        Label::Quiet
    } else {
        Label::Mixed
    }
}

/// `workingset.current(ws)`: the newest version.
fn current(ws: &Obj) -> Option<&Obj> {
    ws.get("versions")?.as_array()?.last()?.as_object()
}

/// `workingset.bit_locked(ws, j)`.
fn bit_locked(ws: &Obj, j: &str) -> bool {
    ws.get("locks")
        .and_then(|l| l.get(j))
        .is_some_and(|l| truthy(l.get("bit")))
}

/// `mysets.runtime(c, ids)`: the run time (a plain `+=`, rounded to 0.1 s) and how many bits are estimates.
fn runtime(c: &Corpus, ids: &[&str]) -> (f64, usize) {
    let (mut total, mut est) = (0.0, 0);
    for j in ids {
        total += c.duration(j).0;
        est += usize::from(perfs(c, j).is_empty());
    }
    (round_to(total, 1), est)
}

/// `mysets.length_status(total, target_s)`: `(status, words)`.
fn length_status(total: f64, target_s: i64) -> (&'static str, String) {
    let (lo, hi) = target_band(target_s);
    if target_s == 0 {
        ("none", mmss(total))
    } else if total < lo as f64 {
        ("short", format!("{} short", mmss(lo as f64 - total)))
    } else if total > hi as f64 {
        ("over", format!("{} over", mmss(total - hi as f64)))
    } else {
        ("on", "on target".to_string())
    }
}

/// `mysets.length_words(target_s)`: "5 min", "7.5 min".
fn length_words(target_s: f64) -> String {
    let m = target_s / 60.0;
    #[expect(clippy::float_cmp, reason = "the Python's `m == int(m)`, exactly")]
    let whole_minutes = m == m.trunc();
    if whole_minutes {
        let whole = m as i64;
        format!("{whole} min")
    } else {
        format!("{m:.1} min")
    }
}

/// How many tough spots `mysets.tough_spots(c, ws, with_actions=False)` finds (per bit and for the set), which is all
/// `health` reads of them.
fn tough_n(nb: &GenCorpus, ws: &Obj, ids: &[&str], target_s: i64) -> usize {
    let c: &Corpus = nb;
    let story = nb.story();
    let mut n = 0;
    for (i, j) in ids.iter().enumerate() {
        if i > 0 && transition_weak(&story.transition(ids[i - 1], j)) {
            n += 1;
        }
        if bit_locked(ws, j) {
            continue;
        }
        let lab = bit_label(c, j);
        let ps: Vec<(&Perf, Outcome)> = perfs(c, j)
            .iter()
            .filter_map(|p| night_outcome(p).map(|o| (p, o)))
            .collect();
        if lab == Label::Quiet {
            n += 1;
        } else if let [before @ .., (_, last)] = ps.as_slice()
            && !before.is_empty()
            && matches!(last, Outcome::Quiet | Outcome::Some)
            && before.iter().any(|(_, o)| matches!(o, Outcome::Killed | Outcome::Good))
        {
            n += 1;
        }
        if let Some((last, _)) = ps.last() {
            let da = last.dead_air_max_s;
            let dur = if last.duration_s == 0.0 { 1.0 } else { last.duration_s };
            if da >= LONG_QUIET_S && da >= LONG_QUIET_SHARE * dur && lab != Label::Quiet {
                n += 1;
            }
            let usual = median(ps.iter().map(|(p, _)| p.duration_s).collect());
            if ps.len() >= 2 && last.duration_s > RAN_LONG * usual && last.duration_s - usual > 30.0 {
                n += 1;
            }
        }
    }
    let (total, _) = runtime(c, ids);
    match length_status(total, target_s).0 {
        "over" if ids.len() > 2 => n += 1,
        "short" if !ids.is_empty() => n += 1,
        _ => {}
    }
    if let [.., last] = ids
        && ids.len() >= 3
        && !bit_locked(ws, last)
        && matches!(bit_label(c, last), Label::Quiet | Label::Mixed)
    {
        n += 1;
    }
    n
}

/// `mysets.summary(c, ws)`: one row of `GET /api/mysets`.
pub fn set_summary(nb: &GenCorpus, ws: &Obj, owner: &str) -> Value {
    let c: &Corpus = nb;
    let empty = Obj::new();
    let ver = current(ws).unwrap_or(&empty);
    let ids: Vec<&str> = ver
        .get("bits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|b| b.get("joke_id").and_then(Value::as_str).unwrap_or(""))
        .collect();
    let target = ver.get("target_s");
    let target_i = int_or_zero(target);
    let (total, est) = runtime(c, &ids);
    let (status, words) = length_status(total, target_i);
    let tn = tough_n(nb, ws, &ids, target_i);
    let health = if ids.is_empty() {
        "empty".to_string()
    } else if tn > 0 {
        format!("{tn} tough spot{}", if tn == 1 { "" } else { "s" })
    } else if ids.iter().all(|j| bit_label(c, j) == Label::Untested) {
        "untested".to_string()
    } else {
        "solid".to_string()
    };
    let made_by = if sget(ws, "by") == Some(owner) {
        "you"
    } else {
        "Punchable"
    };
    json!({"id": get(ws, "id"), "title": get(ws, "title"), "target_s": target.cloned().unwrap_or(Value::Null),
           "length_words": length_words(num(target).unwrap_or(0.0)), "n": ids.len(), "runtime_s": total,
           "runtime": mmss(total), "estimated_n": est, "status": status, "status_words": words, "health": health,
           "tough_n": tn, "version": get(ver, "v"), "storyboard_id": get(ws, "storyboard_id"),
           "updated": get(ws, "updated_at"), "source": get(ws, "source"), "made_by": made_by})
}

/// `GET /api/mysets` (`mysets_api.Routes.list`): his sets (working sets not deleted), newest first.
pub fn mysets(nb: &GenCorpus, items: &Obj, owner: &str) -> Value {
    let mut ws: Vec<&Obj> = items
        .values()
        .filter_map(Value::as_object)
        .filter(|w| !truthy(w.get("deleted")))
        .collect();
    sort_newest(&mut ws);
    let sets: Vec<Value> = ws.into_iter().map(|w| set_summary(nb, w, owner)).collect();
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
    fn the_band_is_two_minutes_under_from_eight_minutes() {
        assert_eq!(target_band(0), (60, 0));
        assert_eq!(target_band(300), (240, 300));
        assert_eq!(target_band(479), (419, 479));
        assert_eq!(target_band(480), (360, 480));
        assert_eq!(target_band(100), (60, 100));
    }

    #[test]
    fn weak_transitions_follow_the_words() {
        assert!(!transition_weak(&json!({"label": "proven"})));
        assert!(transition_weak(&json!({"label": "jump", "needs_bridge": true})));
        assert!(!transition_weak(
            &json!({"label": "link", "via": [{"kind": "segue"}, {"kind": "persona"}]})
        ));
        assert!(!transition_weak(
            &json!({"label": "link", "via": [{"kind": "persona"}, {"kind": "callback_to"}]})
        ));
        assert!(transition_weak(
            &json!({"label": "link", "via": [{"kind": "tag_of"}, {"kind": "persona"}]})
        ));
        assert!(transition_weak(&json!({"label": "link", "via": [{"kind": "subject"}]})));
        assert!(!transition_weak(
            &json!({"label": "link", "via": [{"kind": "tag_of"}], "needs_bridge": false})
        ));
        assert!(transition_weak(
            &json!({"label": "link", "via": [], "needs_bridge": true})
        ));
    }

    #[test]
    fn length_words_are_whole_minutes_or_one_decimal() {
        assert_eq!(length_words(300.0), "5 min");
        assert_eq!(length_words(450.0), "7.5 min");
        assert_eq!(length_words(0.0), "0 min");
        assert_eq!(length_words(100.0), "1.7 min");
    }

    #[test]
    fn length_status_reads_the_band() {
        assert_eq!(length_status(100.0, 0), ("none", "1:40".into()));
        assert_eq!(length_status(200.0, 300), ("short", "0:40 short".into()));
        assert_eq!(length_status(250.0, 300), ("on", "on target".into()));
        assert_eq!(length_status(330.0, 300), ("over", "0:30 over".into()));
    }

    #[test]
    fn the_median_of_an_even_count_is_the_mean_of_the_middle_two() {
        assert!((median(vec![3.0, 1.0, 2.0]) - 2.0).abs() < f64::EPSILON);
        assert!((median(vec![4.0, 1.0, 2.0, 3.0]) - 2.5).abs() < f64::EPSILON);
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

    #[test]
    fn night_outcomes_follow_the_rules() {
        let p = |vr: Option<f64>, r: &str, rate: Option<f64>| outcome_of(rate, vr, r);
        assert_eq!(p(Some(2.0), "killed", Some(1.0)), None);
        assert_eq!(p(Some(2.0), "killed", Some(2.0)), Some(Outcome::Killed));
        assert_eq!(p(None, "worked", None), Some(Outcome::Good));
        assert_eq!(p(None, "meh", None), Some(Outcome::Some));
        assert_eq!(p(None, "", None), Some(Outcome::Quiet));
        assert_eq!(p(Some(1.0), "killed", None), Some(Outcome::Killed));
        assert_eq!(p(Some(1.0), "worked", None), Some(Outcome::Good));
        assert_eq!(p(Some(0.6), "worked", None), Some(Outcome::Some));
        assert_eq!(p(Some(0.4), "worked", None), Some(Outcome::Quiet));
        assert_eq!(p(Some(3.0), "died", None), Some(Outcome::Quiet));
        assert_eq!(p(Some(0.0), "killed", None), Some(Outcome::Quiet));
    }
}
