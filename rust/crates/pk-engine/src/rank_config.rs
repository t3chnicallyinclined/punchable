//! The ranking path's knobs as one typed, versioned value: how deep each leg of the store is read, how the read is
//! ordered (one signal, or reciprocal rank fusion of several), how the packet is cut, and the ladder's constants.
//!
//! A config is parsed from JSON and validated before use (unknown fields and out-of-range values are refused, every
//! problem at once), and it names itself: [`RankConfig::sha256`] hashes its canonical JSON (keys sorted at every
//! level, no whitespace), so a packet, a replay and a rollback can say which config they used.
//!
//! From upstream `gw-ops/src/rank_config.rs` (rust-successor 9dfedad). Changes (FORK.md): the `own-first` signal
//! is gone (one notebook, no projects); `text` and `vector` signals order the pool by the store's two legs; a
//! `per_bit` cap keeps one bit from filling the packet (Ask's `PER_JOKE`); the default is Punchable's hybrid order
//! and [`RankConfig::upstream`] keeps the shipped upstream default for its tests; loading from a file is the
//! caller's (this crate does no IO).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{EngineError, EngineResult};
use crate::rerank::{self, Pool};

/// The most candidates a packet may hold.
pub const MAX_K: usize = 200;

/// BM25 over the read itself, with one vocabulary for the whole pool.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bm25Spec {
    /// Snowball (English) stemming of the question and the texts, as the store's analyzer does.
    pub stem: bool,
    /// BM25's term saturation.
    pub k1: f64,
    /// Length normalisation (0: none, 1: full).
    pub b: f64,
    /// How many times the locator's words are added to the text (0: text only).
    pub path_weight: usize,
    /// Score each text by its best window of this many lines (0: the whole text).
    pub window_lines: usize,
}

/// One order of a pool.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum Signal {
    /// The order given (the store's merged read).
    Store,
    /// The store's full-text (BM25) leg: what it returned, by rank, then the rest.
    Text,
    /// The store's vector (bge) leg: what it returned, by distance, then the rest.
    Vector,
    /// BM25 over the read itself.
    Bm25(Bm25Spec),
}

/// The packet's order of a read: one signal alone, or reciprocal rank fusion of several.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Order {
    /// The signals, fused when more than one.
    pub signals: Vec<Signal>,
    /// RRF's constant (used only with two or more signals).
    pub rrf_k: f64,
}

/// The constants the retrieval ladder's fixed arms ([`rerank::ARMS`]) are built from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LadderKnobs {
    /// BM25's k1.
    pub k1: f64,
    /// BM25's b.
    pub b: f64,
    /// The `bm25-b1` arm's b.
    pub b_alt: f64,
    /// RRF's constant.
    pub rrf_k: f64,
    /// The `bm25-fields` arm's locator weight.
    pub path_weight: usize,
    /// The `bm25-window` arm's window.
    pub window_lines: usize,
}

/// The whole config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankConfig {
    /// How deep each leg of the store is read before the packet's order is computed over the whole read.
    pub read_depth: usize,
    /// The packet's text budget: hits are taken in order while their text fits (at least one).
    pub packet_cap_bytes: usize,
    /// Most candidates in a packet.
    pub max_hits: usize,
    /// Recorded lessons take the packet's first places (true), or are ordered with the hits (false).
    pub facts_first: bool,
    /// At most this many recorded lessons in a packet.
    pub facts_k: usize,
    /// At most this many candidates of one bit in a packet (0: no cap).
    #[serde(default)]
    pub per_bit: usize,
    /// The order.
    pub order: Order,
    /// The ladder's constants.
    pub ladder: LadderKnobs,
}

impl Bm25Spec {
    /// Plain words, the given stemming, the ladder's k1 and b, no locator field, no window.
    pub fn plain(stem: bool, k: &LadderKnobs) -> Self {
        Self {
            stem,
            k1: k.k1,
            b: k.b,
            path_weight: 0,
            window_lines: 0,
        }
    }
}

impl Default for LadderKnobs {
    fn default() -> Self {
        Self {
            k1: rerank::K1,
            b: rerank::B,
            b_alt: 1.0,
            rrf_k: rerank::RRF_K,
            path_weight: rerank::PATH_WEIGHT,
            window_lines: rerank::WINDOW_LINES,
        }
    }
}

impl Default for RankConfig {
    /// Punchable's order: reciprocal rank fusion of the store's full-text leg, its vector leg and stemmed BM25 over
    /// the read; 24 candidates (Ask's `MAX_SOURCES`), at most 3 of one bit (Ask's `PER_JOKE`).
    fn default() -> Self {
        let ladder = LadderKnobs::default();
        Self {
            read_depth: 60,
            packet_cap_bytes: 40_000,
            max_hits: 24,
            facts_first: true,
            facts_k: 5,
            per_bit: 3,
            order: Order {
                signals: vec![
                    Signal::Text,
                    Signal::Vector,
                    Signal::Bm25(Bm25Spec::plain(true, &ladder)),
                ],
                rrf_k: rerank::RRF_K,
            },
            ladder,
        }
    }
}

fn finite_in(name: &str, v: f64, lo: f64, hi: f64, problems: &mut Vec<String>) {
    if !(v.is_finite() && v >= lo && v <= hi) {
        problems.push(format!("{name} = {v} is outside {lo}..={hi}"));
    }
}

fn bm25_ok(name: &str, s: &Bm25Spec, problems: &mut Vec<String>) {
    finite_in(&format!("{name}.k1"), s.k1, 0.01, 10.0, problems);
    finite_in(&format!("{name}.b"), s.b, 0.0, 1.0, problems);
    if s.path_weight > 10 {
        problems.push(format!("{name}.path_weight = {} is above 10", s.path_weight));
    }
    if s.window_lines == 1 || s.window_lines > 500 {
        problems.push(format!("{name}.window_lines = {} must be 0 or 2..=500", s.window_lines));
    }
    if s.window_lines > 0 && s.path_weight > 0 {
        problems.push(format!("{name}: a window and a path field together are not defined"));
    }
}

impl RankConfig {
    /// Upstream's shipped default at 9dfedad (RRF of the store's order and stemmed BM25, 20 candidates, no bit
    /// cap): what its ported tests run against.
    pub fn upstream() -> Self {
        let ladder = LadderKnobs::default();
        Self {
            read_depth: 60,
            packet_cap_bytes: 40_000,
            max_hits: 20,
            facts_first: true,
            facts_k: 5,
            per_bit: 0,
            order: Order {
                signals: vec![Signal::Store, Signal::Bm25(Bm25Spec::plain(true, &ladder))],
                rrf_k: rerank::RRF_K,
            },
            ladder,
        }
    }

    /// Every rule a config must meet; all problems at once.
    pub fn validate(&self) -> EngineResult<()> {
        let mut p = Vec::new();
        if !(1..=1000).contains(&self.read_depth) {
            p.push(format!("read_depth = {} is outside 1..=1000", self.read_depth));
        }
        if !(1_000..=2_000_000).contains(&self.packet_cap_bytes) {
            p.push(format!(
                "packet_cap_bytes = {} is outside 1000..=2000000",
                self.packet_cap_bytes
            ));
        }
        if !(1..=MAX_K).contains(&self.max_hits) {
            p.push(format!("max_hits = {} is outside 1..={MAX_K}", self.max_hits));
        }
        if self.facts_k > self.max_hits {
            p.push(format!(
                "facts_k = {} is above max_hits = {}",
                self.facts_k, self.max_hits
            ));
        }
        if self.per_bit > self.max_hits {
            p.push(format!(
                "per_bit = {} is above max_hits = {}",
                self.per_bit, self.max_hits
            ));
        }
        if self.order.signals.is_empty() || self.order.signals.len() > 4 {
            p.push(format!("order.signals has {} entries; 1..=4", self.order.signals.len()));
        }
        finite_in("order.rrf_k", self.order.rrf_k, 0.0, 1000.0, &mut p);
        for (i, s) in self.order.signals.iter().enumerate() {
            if let Signal::Bm25(spec) = s {
                bm25_ok(&format!("order.signals[{i}]"), spec, &mut p);
            }
        }
        let l = &self.ladder;
        finite_in("ladder.k1", l.k1, 0.01, 10.0, &mut p);
        finite_in("ladder.b", l.b, 0.0, 1.0, &mut p);
        finite_in("ladder.b_alt", l.b_alt, 0.0, 1.0, &mut p);
        finite_in("ladder.rrf_k", l.rrf_k, 0.0, 1000.0, &mut p);
        if l.path_weight > 10 {
            p.push(format!("ladder.path_weight = {} is above 10", l.path_weight));
        }
        if !(2..=500).contains(&l.window_lines) {
            p.push(format!("ladder.window_lines = {} is outside 2..=500", l.window_lines));
        }
        if p.is_empty() {
            Ok(())
        } else {
            Err(EngineError::Config(p.join("; ")))
        }
    }

    /// Parse and validate a config from JSON text.
    pub fn from_json(text: &str) -> EngineResult<Self> {
        let cfg: Self = serde_json::from_str(text).map_err(EngineError::json("parsing the rank config"))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// The canonical JSON: object keys sorted at every level, no whitespace.
    pub fn canonical_json(&self) -> String {
        // A RankConfig is plain numbers, bools and strings: serialising it cannot fail (a non-finite float becomes
        // null, which `validate` refuses on the way back in).
        let v = serde_json::to_value(self).unwrap_or(Value::Null);
        let mut out = String::new();
        canonical(&v, &mut out);
        out
    }

    /// sha256 of the canonical JSON, hex: the name a packet, a replay or a rollback gives its config.
    pub fn sha256(&self) -> String {
        hex::encode(Sha256::digest(self.canonical_json().as_bytes()))
    }
}

fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                canonical(&map[k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// One signal's order of the pool against the question, as indices into it.
pub fn signal_order(question: &str, pool: &Pool<'_>, signal: &Signal) -> Vec<usize> {
    match signal {
        Signal::Store => (0..pool.texts.len()).collect(),
        Signal::Text => rerank::by_leg(pool.text_rank),
        Signal::Vector => rerank::by_leg(pool.vector_rank),
        Signal::Bm25(s) => {
            let query = rerank::words(question, s.stem);
            if s.window_lines > 0 {
                return rerank::order_desc(&rerank::bm25_window_with(
                    &query,
                    pool.texts,
                    s.stem,
                    (s.k1, s.b),
                    s.window_lines,
                ));
            }
            let docs: Vec<Vec<String>> = pool
                .texts
                .iter()
                .zip(pool.locators)
                .map(|(t, l)| {
                    let mut doc = rerank::words(t, s.stem);
                    if s.path_weight > 0 {
                        let path = rerank::path_words(l, s.stem);
                        for _ in 0..s.path_weight {
                            doc.extend(path.iter().cloned());
                        }
                    }
                    doc
                })
                .collect();
            rerank::order_desc(&rerank::bm25_params(&query, &docs, s.k1, s.b))
        }
    }
}

/// The order's indices into the pool, best first.
pub fn order_pool(question: &str, pool: &Pool<'_>, order: &Order) -> Vec<usize> {
    order_pool_traced(question, pool, order).1
}

/// [`order_pool`] with each signal's own order beside the fused one (a packet's trace keeps them).
pub fn order_pool_traced(question: &str, pool: &Pool<'_>, order: &Order) -> (Vec<Vec<usize>>, Vec<usize>) {
    let orders: Vec<Vec<usize>> = order.signals.iter().map(|s| signal_order(question, pool, s)).collect();
    let fused = match orders.as_slice() {
        [] => (0..pool.texts.len()).collect(),
        [only] => only.clone(),
        _ => rerank::rrf(&orders, order.rrf_k),
    };
    (orders, fused)
}

/// The retrieval ladder's arms as orders, in [`rerank::ARMS`] order, built from the knobs.
pub fn ladder_arms(k: &LadderKnobs) -> Vec<(&'static str, Order)> {
    let one = |s: Signal| Order {
        signals: vec![s],
        rrf_k: k.rrf_k,
    };
    let fuse = |signals: Vec<Signal>| Order {
        signals,
        rrf_k: k.rrf_k,
    };
    let plain = Signal::Bm25(Bm25Spec::plain(false, k));
    let stem = Signal::Bm25(Bm25Spec::plain(true, k));
    let b1 = Signal::Bm25(Bm25Spec {
        b: k.b_alt,
        ..Bm25Spec::plain(false, k)
    });
    let window = Signal::Bm25(Bm25Spec {
        window_lines: k.window_lines,
        ..Bm25Spec::plain(false, k)
    });
    let fields = Signal::Bm25(Bm25Spec {
        path_weight: k.path_weight,
        ..Bm25Spec::plain(true, k)
    });
    vec![
        ("store", one(Signal::Store)),
        ("bm25", one(plain)),
        ("bm25-b1", one(b1)),
        ("bm25-window", one(window)),
        ("bm25-stem", one(stem)),
        ("bm25-fields", one(fields)),
        ("rrf", fuse(vec![Signal::Store, stem])),
        ("rrf-window", fuse(vec![plain, window])),
        ("text", one(Signal::Text)),
        ("vector", one(Signal::Vector)),
        ("hybrid", fuse(vec![Signal::Text, Signal::Vector, stem])),
    ]
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::many_single_char_names)]
mod tests {
    use super::*;

    #[test]
    fn the_upstream_default_is_its_shipped_constants_and_validates() {
        let c = RankConfig::upstream();
        c.validate().expect("valid");
        assert_eq!(
            (c.read_depth, c.packet_cap_bytes, c.max_hits, c.facts_k),
            (60, 40_000, 20, 5)
        );
        assert!(c.facts_first);
        assert_eq!(c.order.rrf_k, 60.0);
        assert_eq!(c.ladder.b_alt, 1.0);
        let arms = ladder_arms(&c.ladder);
        assert_eq!(arms.iter().map(|(a, _)| *a).collect::<Vec<_>>(), rerank::ARMS);
        assert_eq!(arms.iter().find(|(a, _)| *a == "rrf").expect("rrf arm").1, c.order);
    }

    #[test]
    fn punchables_default_is_the_hybrid_arm_with_asks_caps() {
        let c = RankConfig::default();
        c.validate().expect("valid");
        assert_eq!((c.max_hits, c.per_bit), (24, 3));
        let arms = ladder_arms(&c.ladder);
        assert_eq!(
            arms.iter().find(|(a, _)| *a == "hybrid").expect("hybrid arm").1,
            c.order
        );
    }

    #[test]
    fn the_hash_is_stable_names_the_config_and_survives_a_round_trip() {
        let c = RankConfig::default();
        let back = RankConfig::from_json(&serde_json::to_string_pretty(&c).expect("json")).expect("parse");
        assert_eq!(back, c);
        assert_eq!(back.sha256(), c.sha256());
        assert_eq!(c.sha256().len(), 64);
        assert!(
            c.canonical_json()
                .starts_with(r#"{"facts_first":true,"facts_k":5,"ladder":{"b":0.75,"#)
        );
        let mut other = c.clone();
        other.order.rrf_k += 1.0;
        assert_ne!(other.sha256(), c.sha256());
        assert_ne!(RankConfig::upstream().sha256(), c.sha256());
    }

    #[test]
    fn a_bad_config_is_refused_with_every_problem() {
        let mut c = RankConfig {
            read_depth: 0,
            facts_k: 50,
            per_bit: 99,
            ..Default::default()
        };
        c.order.signals.clear();
        c.ladder.b = 1.5;
        let e = c.validate().expect_err("refused").to_string();
        for want in ["read_depth", "facts_k", "per_bit", "order.signals", "ladder.b "] {
            assert!(e.contains(want), "{want} missing from: {e}");
        }
        let mut v = serde_json::to_value(RankConfig::default()).expect("json");
        v["surprise"] = Value::Bool(true);
        assert!(
            RankConfig::from_json(&v.to_string()).is_err(),
            "an unknown field is refused"
        );
        let mut w = serde_json::to_value(RankConfig::default()).expect("json");
        w["order"]["signals"][2]["bm25"]["b"] = serde_json::json!(f64::from(2));
        assert!(
            RankConfig::from_json(&w.to_string())
                .expect_err("refused")
                .to_string()
                .contains("b = 2")
        );
        // A upstream config without per_bit still reads (no cap).
        let mut g = serde_json::to_value(RankConfig::upstream()).expect("json");
        g.as_object_mut().expect("object").remove("per_bit");
        assert_eq!(RankConfig::from_json(&g.to_string()).expect("reads").per_bit, 0);
    }

    #[test]
    fn the_config_orders_equal_the_fixed_ladder_arms() {
        let owned = [
            "the parser rejects empty input with an error".to_owned(),
            "src/parse.rs reads the input\nand rejects nothing\n".repeat(8),
            "unrelated notes about the bench".to_owned(),
            "empty input: the parser returns early and rejects it".to_owned(),
        ];
        let texts: Vec<&str> = owned.iter().map(String::as_str).collect();
        let locators = ["a.md#L1-1", "src/parse.rs#L1-16", "b.md#L1-1", "c.md#L1-1"];
        let pool = Pool {
            texts: &texts,
            locators: &locators,
            text_rank: &[Some(2), None, Some(1), None],
            vector_rank: &[None, Some(1), Some(3), Some(2)],
        };
        let q = "why does the parser reject empty input";
        let fixed = rerank::ladder_orders(q, &pool);
        let configured: Vec<(&str, Vec<usize>)> = ladder_arms(&LadderKnobs::default())
            .iter()
            .map(|(a, o)| (*a, order_pool(q, &pool, o)))
            .collect();
        assert_eq!(configured, fixed);
    }
}
