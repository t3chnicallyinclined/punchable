//! The proposer's pure core: a ranking config scored over cached pools (deterministic, no tokens), the search space a
//! proposed config must stay in, the typed diff a model may return (unknown fields refused, so a reply cannot name a
//! file, a set or anything else), and the Pareto front and reflective batch a search draws from.
//!
//! From upstream `gw-ops/src/proposer.rs` (rust-successor 9dfedad). Not forked (FORK.md): the model call
//! (`ClaudeCli` spawns the `claude` command line: no process is ever spawned from Punchable's engine; a later lane
//! makes the one call through `pk-llm` in the worker), the search loop that drives it, and the store sink. The pool
//! entry carries the store's two leg ranks instead of a project and its rank there.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{EngineError, EngineResult};
use crate::rank_config::{Order, RankConfig, Signal, order_pool};
use crate::rerank::Pool;

/// Which half of the train set a question is in: A is shown to the proposer, B is scored and never shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fold {
    /// Shown.
    A,
    /// Held back.
    B,
}

/// The fold of a question id, by sha256 (about two thirds A).
pub fn fold(id: &str) -> Fold {
    let h = Sha256::digest(id.as_bytes());
    if h[0] % 3 < 2 { Fold::A } else { Fold::B }
}

/// One candidate of a cached pool, in the store's given order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cand {
    /// The record.
    pub key: String,
    /// What it says (title over text).
    pub text: String,
    /// Its rank in the full-text leg.
    pub text_rank: Option<usize>,
    /// Its rank in the vector leg.
    pub vector_rank: Option<usize>,
    /// The bit it belongs to, for the `per_bit` cap.
    #[serde(default)]
    pub bit: Option<String>,
    /// It answers the question.
    pub gold: bool,
}

/// One train question with its pool, read once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// The question's id.
    pub id: String,
    /// The question.
    pub question: String,
    /// Its fold.
    pub fold: Fold,
    /// The read, at the deepest depth the search space allows.
    pub pool: Vec<Cand>,
}

/// A config's result on one question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    /// A gold candidate made the packet.
    pub gold: bool,
    /// The packet's bytes.
    pub bytes: usize,
}

/// Gold in the packet the config would build from the cached pool: the read cut to `read_depth` per leg, the
/// config's order, at most `per_bit` of one bit, taken while the text fits in `packet_cap_bytes` (at least one) and
/// at most `max_hits` (the same cut [`crate::pack::pack`] makes, so a score is what a live packet would hold).
pub fn score_item(cfg: &RankConfig, item: &Item) -> Outcome {
    let within = |r: Option<usize>| r.is_some_and(|r| r <= cfg.read_depth);
    let kept: Vec<&Cand> = item
        .pool
        .iter()
        .filter(|c| within(c.text_rank) || within(c.vector_rank))
        .collect();
    let texts: Vec<&str> = kept.iter().map(|c| c.text.as_str()).collect();
    let locators: Vec<&str> = kept.iter().map(|c| c.key.as_str()).collect();
    let text_rank: Vec<Option<usize>> = kept
        .iter()
        .map(|c| c.text_rank.filter(|&r| r <= cfg.read_depth))
        .collect();
    let vector_rank: Vec<Option<usize>> = kept
        .iter()
        .map(|c| c.vector_rank.filter(|&r| r <= cfg.read_depth))
        .collect();
    let pool = Pool {
        texts: &texts,
        locators: &locators,
        text_rank: &text_rank,
        vector_rank: &vector_rank,
    };
    let order = order_pool(&item.question, &pool, &cfg.order);
    let (mut used, mut count, mut gold) = (0usize, 0usize, false);
    let mut per: BTreeMap<&str, usize> = BTreeMap::new();
    for i in order {
        if count == cfg.max_hits || (count > 0 && used + texts[i].len() > cfg.packet_cap_bytes) {
            break;
        }
        if cfg.per_bit > 0
            && let Some(bit) = kept[i].bit.as_deref()
        {
            let n = per.entry(bit).or_default();
            if *n >= cfg.per_bit {
                continue;
            }
            *n += 1;
        }
        used += texts[i].len();
        count += 1;
        gold |= kept[i].gold;
    }
    Outcome { gold, bytes: used }
}

/// A config scored on every train question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scored {
    /// The config's id.
    pub sha256: String,
    /// The config.
    pub cfg: RankConfig,
    /// Per question id.
    pub outcomes: BTreeMap<String, Outcome>,
}

impl Scored {
    /// Score `cfg` on `items`.
    pub fn of(cfg: RankConfig, items: &[Item]) -> Self {
        let outcomes = items.iter().map(|i| (i.id.clone(), score_item(&cfg, i))).collect();
        Self {
            sha256: cfg.sha256(),
            cfg,
            outcomes,
        }
    }

    /// Questions with gold in the packet, in one fold or all.
    pub fn gold(&self, items: &[Item], fold: Option<Fold>) -> usize {
        items
            .iter()
            .filter(|i| fold.is_none_or(|f| i.fold == f))
            .filter(|i| self.outcomes.get(&i.id).is_some_and(|o| o.gold))
            .count()
    }

    /// The median packet bytes.
    pub fn bytes_median(&self) -> usize {
        let mut b: Vec<usize> = self.outcomes.values().map(|o| o.bytes).collect();
        b.sort_unstable();
        b.get(b.len() / 2).copied().unwrap_or(0)
    }
}

/// The search space's bounds: [`in_search_space`] checks them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// Per leg.
    pub read_depth: (usize, usize),
    /// The packet's text budget.
    pub packet_cap_bytes: (usize, usize),
    /// Candidates in a packet.
    pub max_hits: (usize, usize),
    /// Lessons in a packet.
    pub facts_k: (usize, usize),
    /// RRF's constant.
    pub rrf_k: (f64, f64),
    /// How many signals.
    pub signals: (usize, usize),
    /// BM25's k1.
    pub k1: (f64, f64),
    /// BM25's b.
    pub b: (f64, f64),
    /// Locator weight.
    pub path_weight_max: usize,
    /// Window lines (or 0).
    pub window_lines: (usize, usize),
    /// Candidates of one bit (0: no cap).
    pub per_bit: (usize, usize),
}

/// The bounds (upstream's prereg values, with Punchable's `per_bit`).
pub const BOUNDS: Bounds = Bounds {
    read_depth: (1, 60),
    packet_cap_bytes: (1_000, 40_000),
    max_hits: (1, 40),
    facts_k: (1, 10),
    rrf_k: (1.0, 200.0),
    signals: (1, 4),
    k1: (0.3, 3.0),
    b: (0.0, 1.0),
    path_weight_max: 4,
    window_lines: (6, 40),
    per_bit: (0, 10),
};

/// Whether a proposed config is inside the search space (and valid); every problem at once.
pub fn in_search_space(c: &RankConfig, seed: &RankConfig) -> EngineResult<()> {
    let b = &BOUNDS;
    let mut p = Vec::new();
    let within = |v: usize, (lo, hi): (usize, usize)| (lo..=hi).contains(&v);
    let within_f = |v: f64, (lo, hi): (f64, f64)| (lo..=hi).contains(&v);
    if !within(c.read_depth, b.read_depth) {
        p.push(format!(
            "read_depth {} outside {}..={}",
            c.read_depth, b.read_depth.0, b.read_depth.1
        ));
    }
    if !within(c.packet_cap_bytes, b.packet_cap_bytes) {
        p.push(format!(
            "packet_cap_bytes {} outside {}..={}",
            c.packet_cap_bytes, b.packet_cap_bytes.0, b.packet_cap_bytes.1
        ));
    }
    if !within(c.max_hits, b.max_hits) {
        p.push(format!(
            "max_hits {} outside {}..={}",
            c.max_hits, b.max_hits.0, b.max_hits.1
        ));
    }
    if !within(c.facts_k, b.facts_k) || c.facts_k > c.max_hits {
        p.push(format!(
            "facts_k {} outside {}..={} or above max_hits",
            c.facts_k, b.facts_k.0, b.facts_k.1
        ));
    }
    if !within(c.per_bit, b.per_bit) {
        p.push(format!(
            "per_bit {} outside {}..={}",
            c.per_bit, b.per_bit.0, b.per_bit.1
        ));
    }
    if !within_f(c.order.rrf_k, b.rrf_k) {
        p.push(format!("rrf_k {} outside {}..={}", c.order.rrf_k, b.rrf_k.0, b.rrf_k.1));
    }
    if !within(c.order.signals.len(), b.signals) {
        p.push(format!("{} to {} signals", b.signals.0, b.signals.1));
    }
    for s in &c.order.signals {
        if let Signal::Bm25(m) = s {
            if !within_f(m.k1, b.k1) || !within_f(m.b, b.b) || m.path_weight > b.path_weight_max {
                p.push(format!(
                    "bm25 k1 {}..={}, b {}..={}, path_weight <= {}",
                    b.k1.0, b.k1.1, b.b.0, b.b.1, b.path_weight_max
                ));
            }
            if m.window_lines != 0 && !within(m.window_lines, b.window_lines) {
                p.push(format!("window_lines 0 or {}..={}", b.window_lines.0, b.window_lines.1));
            }
        }
    }
    if !c.facts_first || c.ladder != seed.ladder {
        p.push("facts_first and the ladder knobs are fixed".to_owned());
    }
    if p.is_empty() {
        c.validate()
    } else {
        Err(EngineError::Config(format!(
            "outside the search space: {}",
            p.join("; ")
        )))
    }
}

/// The typed diff a model returns: only these fields, unknown ones refused.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diff {
    /// New read depth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_depth: Option<usize>,
    /// New byte cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packet_cap_bytes: Option<usize>,
    /// New place limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hits: Option<usize>,
    /// New lesson limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facts_k: Option<usize>,
    /// New per-bit cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_bit: Option<usize>,
    /// New order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Order>,
}

impl Diff {
    /// The parent with the diff applied (not yet validated: [`in_search_space`] does that).
    pub fn apply(&self, parent: &RankConfig) -> RankConfig {
        let mut c = parent.clone();
        if let Some(v) = self.read_depth {
            c.read_depth = v;
        }
        if let Some(v) = self.packet_cap_bytes {
            c.packet_cap_bytes = v;
        }
        if let Some(v) = self.max_hits {
            c.max_hits = v;
        }
        if let Some(v) = self.facts_k {
            c.facts_k = v;
        }
        if let Some(v) = self.per_bit {
            c.per_bit = v;
        }
        if let Some(o) = &self.order {
            c.order = o.clone();
        }
        c
    }
}

/// The JSON object in a model reply: `{"diff": {...}, "rationale": "..."}`. The reply is untrusted text: only the
/// typed diff survives, and a field the search space does not have is an error, never ignored.
pub fn parse_reply(text: &str) -> EngineResult<(Diff, String)> {
    let refused = |why: &str| EngineError::Refused(format!("the proposer's reply {why}"));
    let start = text.find('{').ok_or_else(|| refused("holds no JSON object"))?;
    let end = text.rfind('}').ok_or_else(|| refused("holds no JSON object"))?;
    if end < start {
        return Err(refused("holds no JSON object"));
    }
    let v: Value = serde_json::from_str(&text[start..=end]).map_err(EngineError::json("the proposer's reply"))?;
    let diff: Diff = serde_json::from_value(v.get("diff").cloned().unwrap_or(Value::Null)).map_err(
        EngineError::json("the diff names a field the search space does not have, or a wrong type"),
    )?;
    Ok((diff, v["rationale"].as_str().unwrap_or_default().to_owned()))
}

/// Per fold-A question, the archive configs that do best (gold, then fewest bytes); a config's weight is how many
/// questions it leads. Returns (index into archive, weight) for every config that leads one.
pub fn front(archive: &[Scored], items: &[Item]) -> Vec<(usize, usize)> {
    let mut leads = vec![0usize; archive.len()];
    for it in items.iter().filter(|i| i.fold == Fold::A) {
        let best = archive
            .iter()
            .filter_map(|s| s.outcomes.get(&it.id))
            .map(|o| (o.gold, std::cmp::Reverse(o.bytes)))
            .max();
        if let Some(best) = best {
            for (k, s) in archive.iter().enumerate() {
                if s.outcomes
                    .get(&it.id)
                    .is_some_and(|o| (o.gold, std::cmp::Reverse(o.bytes)) == best)
                {
                    leads[k] += 1;
                }
            }
        }
    }
    leads.into_iter().enumerate().filter(|(_, w)| *w > 0).collect()
}

/// The fold-A questions the parent misses, those the archive disagrees on most first (p(1-p)), at most `n`.
pub fn batch<'a>(parent: &Scored, archive: &[Scored], items: &'a [Item], n: usize) -> Vec<&'a Item> {
    let mut missed: Vec<(&Item, f64)> = items
        .iter()
        .filter(|i| i.fold == Fold::A && !parent.outcomes.get(&i.id).is_some_and(|o| o.gold))
        .map(|i| {
            let p = archive
                .iter()
                .filter(|s| s.outcomes.get(&i.id).is_some_and(|o| o.gold))
                .count() as f64
                / archive.len().max(1) as f64;
            (i, p * (1.0 - p))
        })
        .collect();
    missed.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
    missed.into_iter().take(n).map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, gold_at: usize) -> Item {
        Item {
            id: id.into(),
            question: "dog lawyer".into(),
            fold: Fold::A,
            pool: (1..=5)
                .map(|i| Cand {
                    key: format!("unit:{i}"),
                    text: if i == gold_at {
                        "the dog has a lawyer".into()
                    } else {
                        format!("filler {i}")
                    },
                    text_rank: Some(i),
                    vector_rank: None,
                    bit: Some(if i <= 2 {
                        "joke:same".into()
                    } else {
                        format!("joke:{i}")
                    }),
                    gold: i == gold_at,
                })
                .collect(),
        }
    }

    #[test]
    fn scoring_is_deterministic_and_counts_gold_in_the_packet() {
        let items = [item("q1", 5), item("q2", 1)];
        let tight = RankConfig {
            max_hits: 1,
            order: Order {
                signals: vec![Signal::Store],
                rrf_k: 60.0,
            },
            ..RankConfig::default()
        };
        let s = Scored::of(tight.clone(), &items);
        assert_eq!(s.gold(&items, None), 1, "only q2's gold is first in the store's order");
        let by_words = RankConfig {
            max_hits: 1,
            order: Order {
                signals: vec![Signal::Bm25(crate::rank_config::Bm25Spec::plain(
                    true,
                    &RankConfig::default().ladder,
                ))],
                rrf_k: 60.0,
            },
            ..RankConfig::default()
        };
        let words = Scored::of(by_words, &items);
        assert_eq!(
            words.gold(&items, None),
            2,
            "BM25 over the read finds q1's gold by its words"
        );
        assert_eq!(Scored::of(tight, &items), s, "the same config scores the same");
        let f = front(&[s.clone(), words.clone()], &items);
        assert_eq!(f, [(0, 1), (1, 2)]);
        assert_eq!(
            batch(&s, &[s.clone(), words], &items, 5)
                .iter()
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>(),
            ["q1"]
        );
    }

    #[test]
    fn scoring_applies_the_per_bit_cap_as_a_live_packet_does() {
        // unit:1 and unit:2 are one bit; gold is unit:2. The store's order, two places.
        let items = [item("q", 2)];
        let store_order = |per_bit| RankConfig {
            max_hits: 2,
            per_bit,
            order: Order {
                signals: vec![Signal::Store],
                rrf_k: 60.0,
            },
            ..RankConfig::default()
        };
        assert!(
            Scored::of(store_order(0), &items).outcomes["q"].gold,
            "no cap: unit:2 is second"
        );
        assert!(
            !Scored::of(store_order(1), &items).outcomes["q"].gold,
            "one per bit: unit:2 is skipped"
        );
    }

    #[test]
    fn a_reply_carries_only_the_typed_diff() {
        let (d, why) = parse_reply("Sure. {\"diff\": {\"max_hits\": 12, \"per_bit\": 2}, \"rationale\": \"fewer\"}")
            .expect("parsed");
        assert_eq!((d.max_hits, d.per_bit, why.as_str()), (Some(12), Some(2), "fewer"));
        let child = d.apply(&RankConfig::default());
        in_search_space(&child, &RankConfig::default()).expect("inside");
        assert!(
            parse_reply("{\"diff\": {\"path\": \"/etc/passwd\"}}").is_err(),
            "an unknown field is refused"
        );
        assert!(parse_reply("no json here").is_err());
        assert!(parse_reply("} backwards {").is_err());
        let wide = Diff {
            max_hits: Some(500),
            ..Diff::default()
        }
        .apply(&RankConfig::default());
        assert!(in_search_space(&wide, &RankConfig::default()).is_err());
    }

    #[test]
    fn folds_are_stable() {
        assert_eq!(fold("q1"), fold("q1"));
        let a = (0..300).filter(|i| fold(&format!("q{i}")) == Fold::A).count();
        assert!((150..250).contains(&a), "about two thirds A: {a}");
    }
}
