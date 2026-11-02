//! Links between bits: a hypothesis that what one bit gets working (a capability card) answers what another bit is
//! stuck on (a problem card). Ranked with no model: BM25 of one card's facets against another's, the blocker of the
//! target against other problems' blockers carried onto the capability that solved them (Swanson's ABC chain), the
//! two fused by reciprocal rank fusion; then screened: a candidate that repeats a move the target bit already makes
//! is vetoed (the "same move is not another angle" rule of Punchable's own `creative/connections.py`), and one that
//! does not stand out against its own scores for every other problem is dropped (each row's own "usually this close"
//! level, `creative/merge.py`, never one global cutoff).
//!
//! From upstream `upstream-ops/src/bridge.rs` (`rank`, `screen`, `below`, `why`; Rust branch 9dfedad), which took
//! those two rules from Punchable. Changes (FORK.md): bits instead of projects; the reach rules (walls, levels,
//! isolation between projects), the store reads and writes, and the candidate -> testable -> supported states are
//! not forked (one comic's notebook has no walls; a link he keeps is a `custom` edge through the decision log). The
//! app sees a link as [`crate::edge::Link`], never a status.

use std::collections::{BTreeSet, HashMap};

use sha2::{Digest, Sha256};

use crate::card::{Card, CardKind, content_words};
use crate::edge::Link;
use crate::error::{EngineError, EngineResult};
use crate::rerank::{RRF_K, bm25, order_desc, rrf_positive};

/// The arms a link is ranked with: `random` (the floor), `flat` (whole card against whole card), `facet`.
pub const ARMS: [&str; 3] = ["random", "flat", "facet"];

/// A candidate whose mechanism shares at least this share of meaning words with one the target bit already has is
/// vetoed.
pub const VETO_JACCARD: f64 = 0.5;
/// A candidate stands out when its z over its own other scores is at least this, over at least [`Z_MIN_N`].
pub const Z_MIN: f64 = 2.5;
/// Below this many other scores, the candidate's own p99 is the bar instead.
pub const Z_MIN_N: usize = 20;

/// One proposal: the capability's index in the pool, its score, and the problem the chain went through.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    /// Index into the pool.
    pub index: usize,
    /// Its score.
    pub score: f64,
    /// The pool index of the problem card the chain went through.
    pub via: Option<usize>,
}

fn bag(text: &str) -> Vec<String> {
    content_words(text)
}

/// The capabilities of `pool` best first for `target` under `arm`, each with a positive score (every one, in a
/// seeded order, for `random`). Pure.
pub fn rank(arm: &str, target: &Card, pool: &[Card]) -> EngineResult<Vec<Ranked>> {
    let caps: Vec<usize> = (0..pool.len())
        .filter(|&i| pool[i].kind == CardKind::Capability)
        .collect();
    let ranked = match arm {
        "random" => {
            let mut order = caps.clone();
            let h = Sha256::digest(target.key.as_bytes());
            let mut seed = u64::from_be_bytes([h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]]) | 1;
            for i in (1..order.len()).rev() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                order.swap(i, (seed % (i as u64 + 1)) as usize);
            }
            order
                .into_iter()
                .map(|index| Ranked {
                    index,
                    score: 0.0,
                    via: None,
                })
                .collect()
        }
        "flat" => {
            let docs: Vec<Vec<String>> = caps.iter().map(|&i| bag(&pool[i].whole_text())).collect();
            let scores = bm25(&bag(&target.whole_text()), &docs);
            order_desc(&scores)
                .into_iter()
                .filter(|&j| scores[j] > 0.0)
                .map(|j| Ranked {
                    index: caps[j],
                    score: scores[j],
                    via: None,
                })
                .collect()
        }
        "facet" => {
            let x_docs: Vec<Vec<String>> = caps.iter().map(|&i| bag(&pool[i].x)).collect();
            let by_x = bm25(&bag(&target.x), &x_docs);
            let problems: Vec<usize> = (0..pool.len())
                .filter(|&i| pool[i].kind == CardKind::Problem && pool[i].y.is_some())
                .collect();
            let y_docs: Vec<Vec<String>> = problems
                .iter()
                .map(|&i| bag(pool[i].y.as_deref().unwrap_or_default()))
                .collect();
            let y_scores = bm25(&bag(target.y.as_deref().unwrap_or_default()), &y_docs);
            let mut by_y = vec![0.0f64; caps.len()];
            let mut via = vec![None; caps.len()];
            for (j, &c) in caps.iter().enumerate() {
                let Some(solves) = pool[c].solves.as_deref() else {
                    continue;
                };
                for (k, &p) in problems.iter().enumerate() {
                    if pool[p].key == solves && pool[p].bit == pool[c].bit && y_scores[k] > by_y[j] {
                        by_y[j] = y_scores[k];
                        via[j] = Some(p);
                    }
                }
            }
            rrf_positive(&[by_x.clone(), by_y.clone()], RRF_K)
                .into_iter()
                .map(|j| Ranked {
                    index: caps[j],
                    score: by_x[j] + by_y[j],
                    via: via[j].filter(|_| by_y[j] > 0.0),
                })
                .collect()
        }
        other => {
            return Err(EngineError::Refused(format!(
                "unknown arm {other:?}; the arms are {}",
                ARMS.join(", ")
            )));
        }
    };
    Ok(ranked)
}

fn word_set(text: &str) -> BTreeSet<String> {
    content_words(text).into_iter().collect()
}

fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        a.intersection(b).count() as f64 / union as f64
    }
}

/// What the screen did to one ranking.
#[derive(Debug, Default, PartialEq)]
pub struct Screened {
    /// Kept, best first, each score lowered by its closeness to the target bit's own capabilities.
    pub kept: Vec<Ranked>,
    /// Each vetoed candidate and the own card whose mechanism it repeats.
    pub vetoed: Vec<(usize, String)>,
    /// Each candidate that did not stand out against its own distribution, and why.
    pub below: Vec<(usize, String)>,
}

/// Screen `ranked` (from [`rank`]) for `target`: the mechanism veto against `own` (the target bit's capability
/// cards), the relative threshold over each candidate's scores for every other problem it could answer, and the
/// whole-text penalty. The random arm is not screened (it is the floor). Pure.
pub fn screen(arm: &str, target: &Card, pool: &[Card], own: &[Card], ranked: Vec<Ranked>) -> EngineResult<Screened> {
    let mut out = Screened::default();
    if arm == "random" {
        out.kept = ranked;
        return Ok(out);
    }
    let own_caps: Vec<&Card> = own.iter().filter(|c| c.kind == CardKind::Capability).collect();
    let own_mech: Vec<(BTreeSet<String>, &Card)> = own_caps
        .iter()
        .filter_map(|c| c.mech.as_deref().map(|m| (word_set(m), *c)))
        .collect();
    let own_whole: Vec<BTreeSet<String>> = own_caps.iter().map(|c| word_set(&c.whole_text())).collect();
    let mut other_scores: Vec<(String, HashMap<usize, f64>)> = Vec::new();
    for q in pool
        .iter()
        .filter(|q| q.kind == CardKind::Problem && q.bit != target.bit)
    {
        let scores = rank(arm, q, pool)?.into_iter().map(|r| (r.index, r.score)).collect();
        other_scores.push((q.bit.clone(), scores));
    }
    for r in ranked {
        let c = &pool[r.index];
        if let Some(m) = c.mech.as_deref() {
            let mine = word_set(m);
            if let Some((_, o)) = own_mech.iter().find(|(w, _)| jaccard(&mine, w) >= VETO_JACCARD) {
                out.vetoed.push((r.index, o.key.clone()));
                continue;
            }
        }
        let dist: Vec<f64> = other_scores
            .iter()
            .filter(|(bit, _)| *bit != c.bit)
            .map(|(_, s)| s.get(&r.index).copied().unwrap_or(0.0))
            .collect();
        if let Some(why) = below(r.score, &dist) {
            out.below.push((r.index, why));
            continue;
        }
        let whole = word_set(&c.whole_text());
        let close = own_whole.iter().map(|o| jaccard(&whole, o)).fold(0.0f64, f64::max);
        out.kept.push(Ranked {
            score: r.score * (1.0 - close),
            ..r
        });
    }
    out.kept.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(out)
}

/// Why `score` does not stand out against `dist` (the candidate's scores for every other problem), if it does not.
pub fn below(score: f64, dist: &[f64]) -> Option<String> {
    let n = dist.len();
    if n == 0 {
        return None;
    }
    if n >= Z_MIN_N {
        let mean = dist.iter().sum::<f64>() / n as f64;
        let sd = (dist.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64).sqrt();
        let z = if sd > 0.0 {
            (score - mean) / sd
        } else if score > mean {
            f64::INFINITY
        } else {
            0.0
        };
        return (z < Z_MIN).then(|| format!("z {z:.2} below {Z_MIN} over its {n} other problems"));
    }
    let k = (n / 100).max(1);
    let mut sorted = dist.to_vec();
    sorted.sort_by(|a, b| b.total_cmp(a));
    let p99 = sorted[k - 1];
    (score <= p99).then(|| format!("score {score:.3} not above its own p99 {p99:.3} over {n} other problem(s)"))
}

/// The meaning words `a` shares with `b`, in `a`'s order, once each.
fn shared(a: &str, b: &str) -> Vec<String> {
    let b = word_set(b);
    let mut seen = BTreeSet::new();
    content_words(a)
        .into_iter()
        .filter(|w| b.contains(w) && seen.insert(w.clone()))
        .collect()
}

/// The link as the app shows it: the two bits and why, in plain words (the words they share).
pub fn link(target: &Card, c: &Card, via: Option<&Card>) -> Link {
    let mut parts = Vec::new();
    let goal = shared(&target.x, &c.x);
    if !goal.is_empty() {
        parts.push(format!("both go for {}", goal.join(", ")));
    }
    if let Some(v) = via {
        let blocker = shared(
            target.y.as_deref().unwrap_or_default(),
            v.y.as_deref().unwrap_or_default(),
        );
        if !blocker.is_empty() {
            parts.push(format!("it got past the same snag ({})", blocker.join(", ")));
        }
    }
    if parts.is_empty() {
        parts.push(format!(
            "they share {}",
            shared(&target.whole_text(), &c.whole_text()).join(", ")
        ));
    }
    Link {
        from: target.bit.clone(),
        to: c.bit.clone(),
        why: parts.join("; "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::tests::card;

    fn cap(bit: &str, key: &str, x: &str, mech: Option<&str>, solves: Option<&str>) -> Card {
        Card {
            mech: mech.map(str::to_owned),
            solves: solves.map(str::to_owned),
            a: vec!["the room knows the setup".into()],
            ..card(bit, key, CardKind::Capability, x, None, x)
        }
    }

    #[test]
    fn the_facet_arm_follows_the_chain_and_the_flat_arm_the_words() {
        let target = card(
            "joke:1",
            "p1",
            CardKind::Problem,
            "landlord dog escalation",
            Some("the tag runs long"),
            "q",
        );
        let pool = vec![
            target.clone(),
            card(
                "joke:2",
                "p2",
                CardKind::Problem,
                "airport security",
                Some("the tag runs long"),
                "q",
            ),
            cap(
                "joke:2",
                "c2",
                "callback to the airport",
                Some("cut the tag to one beat"),
                Some("p2"),
            ),
            cap("joke:3", "c3", "dog escalation payoff", Some("act out the dog"), None),
            cap("joke:4", "c4", "nothing in common here", None, None),
        ];
        let facet = rank("facet", &target, &pool).expect("ranked");
        let keys: Vec<&str> = facet.iter().map(|r| pool[r.index].key.as_str()).collect();
        assert!(keys.contains(&"c2") && keys.contains(&"c3"), "{keys:?}");
        assert!(!keys.contains(&"c4"), "a card that matched nothing is not proposed");
        let c2 = facet.iter().find(|r| pool[r.index].key == "c2").expect("c2");
        assert_eq!(c2.via, Some(1), "through the problem it solved");
        let flat = rank("flat", &target, &pool).expect("ranked");
        assert_eq!(pool[flat[0].index].key, "c3");
        assert_eq!(rank("random", &target, &pool).expect("ranked").len(), 3);
        assert!(rank("guess", &target, &pool).is_err());
        let l = link(&target, &pool[2], Some(&pool[1]));
        assert_eq!((l.from.as_str(), l.to.as_str()), ("joke:1", "joke:2"));
        assert!(l.why.contains("same snag (tag, run, long)"), "{}", l.why);
    }

    #[test]
    fn the_screen_vetoes_a_repeat_move_and_keeps_one_that_stands_out() {
        let target = card(
            "joke:1",
            "p1",
            CardKind::Problem,
            "dog escalation",
            Some("flat ending"),
            "q",
        );
        let pool = vec![
            target.clone(),
            cap("joke:3", "c3", "dog escalation payoff", Some("act out the dog"), None),
            cap(
                "joke:5",
                "c5",
                "dog escalation again",
                Some("act out the dog barking"),
                None,
            ),
        ];
        let own = vec![cap("joke:1", "o1", "the dog bit", Some("act out the dog"), None)];
        let ranked = rank("flat", &target, &pool).expect("ranked");
        let s = screen("flat", &target, &pool, &own, ranked).expect("screened");
        assert!(s.vetoed.iter().any(|(i, o)| pool[*i].key == "c3" && o == "o1"), "{s:?}");
        assert!(s.kept.iter().all(|r| pool[r.index].key != "c3"));
        assert_eq!(below(1.0, &[]), None);
        assert!(below(1.0, &[2.0]).is_some(), "not above its own p99");
        assert!(below(10.0, &[1.0; 25]).is_none(), "far above every other score");
    }
}
