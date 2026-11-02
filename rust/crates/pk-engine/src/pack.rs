//! A read made into a packet: recorded lessons first, then the read's hits in the config's order, at most
//! `per_bit` of one bit, cut by bytes and by places; with a trace of how (each signal's order, the cut, timings), so an
//! offline replay of the same read under the same config reproduces the packet exactly ([`crate::learning`]).
//!
//! From upstream `gw-ops/src/ask.rs` (`pack`, `order_traced`, `Trace`, rust-successor 9dfedad). Changes
//! (FORK.md): a [`Hit`] is a record of his material with its ranks in the store's two legs, not a file span of a
//! project; the hits keep the store's given order (upstream re-sorted by a score that spanned projects); the
//! `per_bit` cap is new; the trace drops the project index.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::rank_config::{Order, RankConfig, order_pool_traced};
use crate::rerank::{Pool, question_words};

/// Hash prefix kept in a trace: enough to tell that a record changed since the read.
pub const TRACE_HASH: usize = 12;
/// How many places of each signal's order a trace keeps (the packet order is kept in full).
pub const TRACE_SIGNAL_TOP: usize = 40;

/// One record of his material as a read returns it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    /// The record (`unit:…`, `joke:…`, `performance:…`).
    pub key: String,
    /// The table it came from.
    #[serde(default)]
    pub kind: Option<String>,
    /// sha256 of `text`.
    #[serde(default)]
    pub hash: String,
    /// The bit's title, when the record is a bit or belongs to one.
    #[serde(default)]
    pub title: Option<String>,
    /// What it says.
    pub text: String,
    /// The bit it belongs to (`joke:…`), when known.
    #[serde(default)]
    pub bit: Option<String>,
    /// `YYYY-MM-DD`, when known.
    #[serde(default)]
    pub date: Option<String>,
    /// Its full-text score in the store, when that leg returned it.
    #[serde(default)]
    pub score: Option<f64>,
    /// Its rank in the store's full-text leg (1 = best).
    #[serde(default)]
    pub text_rank: Option<usize>,
    /// Its rank in the store's vector leg (1 = nearest).
    #[serde(default)]
    pub vector_rank: Option<usize>,
    /// Its cosine distance to the question, when the vector leg returned it.
    #[serde(default)]
    pub distance: Option<f64>,
    /// When the hit is a recorded lesson's evidence: the lesson.
    #[serde(default)]
    pub fact: Option<String>,
    /// The lesson's grade (internal: read from the store, never written out, so a hit on any wire carries no grade;
    /// the app sees `crate::edge`).
    #[serde(default, skip_serializing)]
    pub fact_grade: Option<String>,
}

impl Hit {
    /// The text the local BM25 reads: the title (when there is one) over the text.
    pub fn scored_text(&self) -> String {
        match self.title.as_deref().filter(|t| !t.is_empty()) {
            Some(t) => format!("{t}\n{}", self.text),
            None => self.text.clone(),
        }
    }
}

/// How a packet was made, recorded with it; built from what the read already holds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    /// Format version of this record.
    pub v: u32,
    /// The packet's place limit.
    pub limit: usize,
    /// The whole read in read order: `[key, hash prefix, text rank, vector rank, 1 if a lesson's evidence]`.
    pub pool: Vec<serde_json::Value>,
    /// Each signal's top order over the hits (indices).
    pub signals: Vec<Vec<u32>>,
    /// The packet order's kept hits (indices).
    pub order: Vec<u32>,
    /// Hits kept.
    pub kept: usize,
    /// Lessons kept.
    pub facts: usize,
    /// Bytes of text in the packet.
    pub bytes: usize,
    /// Microseconds per step.
    pub us: BTreeMap<String, u64>,
    /// What the read could not do (a lessons read that failed), in plain words.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// What [`pack`] gives: the lessons that go first, the kept hits in packet order, and the trace.
#[derive(Debug, Clone, PartialEq)]
pub struct Packed {
    /// Lessons, best first.
    pub facts: Vec<Hit>,
    /// Hits, in packet order.
    pub hits: Vec<Hit>,
    /// How.
    pub trace: Trace,
}

pub(crate) fn micros(t: Instant) -> u64 {
    u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// The trace's record of one read entry.
pub fn pool_entry(h: &Hit, fact: bool) -> serde_json::Value {
    serde_json::json!([
        h.key,
        h.hash.get(..TRACE_HASH).unwrap_or(&h.hash),
        h.text_rank,
        h.vector_rank,
        u8::from(fact)
    ])
}

/// A read made into a packet under `cfg`: lessons first (deduplicated, best first, at most `facts_k`), then the
/// hits in the config's order, at most `per_bit` of one bit, cut by bytes and by places. The one function a live
/// read and the offline replay both run.
pub fn pack(question: &str, limit: usize, mut facts: Vec<Hit>, hits: Vec<Hit>, cfg: &RankConfig) -> Packed {
    // Lessons first: the recorded lesson is the checked answer (upstream measured that ranking them with the
    // hits dropped the ones that answer out of most packets, ask-v2 r8). A lesson's evidence is shown once.
    // The trace keeps the whole read as it came in, lessons marked, before anything is cut.
    let pool: Vec<serde_json::Value> = facts
        .iter()
        .map(|h| pool_entry(h, true))
        .chain(hits.iter().map(|h| pool_entry(h, false)))
        .collect();
    facts.sort_by(|a, b| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)));
    let mut seen = BTreeSet::new();
    facts.retain(|h| seen.insert(h.key.clone()));
    facts.truncate(cfg.facts_k);
    // Only the lessons kept hide their evidence from the hits (upstream hid every lesson's evidence before the
    // cut, so a config with a smaller `facts_k` lost those records altogether: FORK.md).
    let shown: BTreeSet<&str> = facts.iter().map(|h| h.key.as_str()).collect();
    let mut hits: Vec<Hit> = hits.into_iter().filter(|h| !shown.contains(h.key.as_str())).collect();
    if !cfg.facts_first {
        hits.append(&mut facts);
    }
    let places = limit.saturating_sub(facts.len()).max(1);
    let rank_start = Instant::now();
    let (ordered, signals, order) = order_traced(question, hits, &cfg.order);
    let rank_us = micros(rank_start);
    let cut_start = Instant::now();
    let mut used: usize = facts.iter().map(|h| h.text.len()).sum();
    let mut per: HashMap<String, usize> = HashMap::new();
    let mut kept_hits = Vec::new();
    let mut kept_order = Vec::new();
    for (h, i) in ordered.into_iter().zip(order) {
        if kept_hits.len() == places {
            break;
        }
        if !kept_hits.is_empty() && used + h.text.len() > cfg.packet_cap_bytes {
            break;
        }
        if cfg.per_bit > 0
            && let Some(bit) = &h.bit
        {
            let n = per.entry(bit.clone()).or_default();
            if *n >= cfg.per_bit {
                continue;
            }
            *n += 1;
        }
        used += h.text.len();
        kept_hits.push(h);
        kept_order.push(i);
    }
    let mut trace = Trace {
        v: 1,
        limit,
        pool,
        signals,
        kept: kept_hits.len(),
        order: kept_order,
        facts: facts.len(),
        bytes: used,
        ..Trace::default()
    };
    trace.us.insert("rank".to_owned(), rank_us);
    trace.us.insert("cut".to_owned(), micros(cut_start));
    Packed {
        facts,
        hits: kept_hits,
        trace,
    }
}

/// The hits in `order`, each signal's top [`TRACE_SIGNAL_TOP`] and the fused order (indices into the given hits).
pub fn order_traced(question: &str, hits: Vec<Hit>, order: &Order) -> (Vec<Hit>, Vec<Vec<u32>>, Vec<u32>) {
    let texts_owned: Vec<String> = hits.iter().map(Hit::scored_text).collect();
    let texts: Vec<&str> = texts_owned.iter().map(String::as_str).collect();
    let locators: Vec<&str> = hits.iter().map(|h| h.key.as_str()).collect();
    let text_rank: Vec<Option<usize>> = hits.iter().map(|h| h.text_rank).collect();
    let vector_rank: Vec<Option<usize>> = hits.iter().map(|h| h.vector_rank).collect();
    let pool = Pool {
        texts: &texts,
        locators: &locators,
        text_rank: &text_rank,
        vector_rank: &vector_rank,
    };
    // The local BM25 reads what the question is about, as the store's legs do (`rerank::question_words`).
    let about = question_words(question).join(" ");
    let (orders, fused) = order_pool_traced(&about, &pool, order);
    let idx = |v: &[usize], n: usize| -> Vec<u32> { v.iter().take(n).map(|&i| i as u32).collect() };
    let signals = orders.iter().map(|o| idx(o, TRACE_SIGNAL_TOP)).collect();
    let full = idx(&fused, fused.len());
    let mut taken: Vec<Option<Hit>> = hits.into_iter().map(Some).collect();
    (
        fused.into_iter().filter_map(|i| taken[i].take()).collect(),
        signals,
        full,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rank_config::{Bm25Spec, LadderKnobs, Signal};

    fn hit(key: &str, text: &str, bit: Option<&str>, text_rank: Option<usize>, vector_rank: Option<usize>) -> Hit {
        Hit {
            key: key.into(),
            text: text.into(),
            bit: bit.map(str::to_owned),
            text_rank,
            vector_rank,
            hash: format!("{key}-hash"),
            ..Hit::default()
        }
    }

    // Upstream's `the_packet_orders_the_read_by_its_own_words_not_the_stores_scores`, on its shipped order.
    #[test]
    fn the_packet_orders_the_read_by_its_own_words_not_only_the_stores_order() {
        let mut read: Vec<Hit> = (0..30)
            .map(|i| {
                hit(
                    &format!("unit:big{i}"),
                    "frames and rollback in general terms",
                    None,
                    None,
                    None,
                )
            })
            .collect();
        read.push(hit(
            "unit:note",
            "the save set is 277,288 bytes, every rollback restores it",
            None,
            None,
            None,
        ));
        read.push(hit("unit:other", "unrelated note", None, None, None));
        let gw = RankConfig::upstream();
        let (out, _, _) = order_traced("how many bytes is the save set a rollback restores", read, &gw.order);
        assert_eq!(out.len(), 32, "nothing dropped: the caller cuts");
        let note = out.iter().position(|h| h.key == "unit:note").expect("kept");
        assert!(note < 15, "the note with the words rises into the first half: {note}");
        assert_eq!(
            out[0].key, "unit:big0",
            "the store's best, which also matches, stays first"
        );
        assert_eq!(out[31].key, "unit:other", "the unrelated note stays last");
    }

    #[test]
    fn a_meaning_match_the_words_miss_reaches_the_packet_through_the_vector_leg() {
        // "landlord" is never said; only the vector leg finds the dog bit. Words alone leave it last, behind a
        // hit no signal likes; the hybrid order lifts it above that one.
        let read = vec![
            hit(
                "unit:a",
                "apartment hunting is a bit much",
                Some("joke:a"),
                Some(1),
                Some(2),
            ),
            hit("unit:c", "a bit about my car", Some("joke:c"), Some(2), Some(3)),
            hit("unit:d", "airports at night", Some("joke:d"), None, None),
            hit(
                "unit:b",
                "guy who owns my building has a dog that owns him",
                Some("joke:b"),
                None,
                Some(1),
            ),
        ];
        let q = "the landlord bit";
        let words_only = Order {
            signals: vec![Signal::Bm25(Bm25Spec::plain(true, &LadderKnobs::default()))],
            rrf_k: 60.0,
        };
        let (w, _, _) = order_traced(q, read.clone(), &words_only);
        assert_eq!(w[3].key, "unit:b", "words alone: last");
        let (h, _, _) = order_traced(q, read, &RankConfig::default().order);
        let at = |k: &str| h.iter().position(|x| x.key == k).expect("kept");
        assert!(
            at("unit:b") < at("unit:d"),
            "hybrid: the meaning match passes the stranger"
        );
    }

    #[test]
    fn lessons_go_first_once_and_the_cut_counts_bytes_places_and_bits() {
        let cfg = RankConfig {
            per_bit: 2,
            packet_cap_bytes: 1_000,
            ..RankConfig::default()
        };
        let mut lesson = hit(
            "unit:x",
            "he keeps lines that end on the dog",
            Some("joke:x"),
            Some(1),
            Some(1),
        );
        lesson.fact = Some("he keeps lines that end on the dog".into());
        let hits = vec![
            hit(
                "unit:x",
                "he keeps lines that end on the dog",
                Some("joke:x"),
                Some(1),
                Some(1),
            ),
            hit("unit:1", "dog one", Some("joke:d"), Some(2), Some(2)),
            hit("unit:2", "dog two", Some("joke:d"), Some(3), Some(3)),
            hit("unit:3", "dog three", Some("joke:d"), Some(4), Some(4)),
            hit("unit:4", "cat", Some("joke:c"), Some(5), Some(5)),
        ];
        let p = pack("dog", 24, vec![lesson], hits, &cfg);
        assert_eq!(p.facts.len(), 1);
        let keys: Vec<&str> = p.hits.iter().map(|h| h.key.as_str()).collect();
        assert!(
            !keys.contains(&"unit:x"),
            "the lesson's evidence is shown once, as the lesson: {keys:?}"
        );
        assert_eq!(
            keys.iter()
                .filter(|k| k.starts_with("unit:") && **k != "unit:4")
                .count(),
            2,
            "per_bit 2: {keys:?}"
        );
        assert!(keys.contains(&"unit:4"));
        assert_eq!(p.trace.kept, keys.len());
        assert_eq!(
            p.trace.pool.len(),
            6,
            "the trace keeps the whole read, the lesson included"
        );
        // Byte cap: the first hit always goes in; the next only while it fits.
        let tight = RankConfig {
            packet_cap_bytes: 1_000,
            per_bit: 0,
            ..RankConfig::default()
        };
        let big = vec![
            hit("unit:big", &"w ".repeat(600), None, Some(1), None),
            hit("unit:s", "small", None, Some(2), None),
        ];
        let p = pack("w", 24, vec![], big, &tight);
        assert_eq!(p.hits.len(), 1, "the second would cross the cap");
    }

    #[test]
    fn a_lesson_cut_by_facts_k_leaves_its_evidence_record_in_the_read() {
        let lesson = |key: &str, score: f64| Hit {
            fact: Some(format!("lesson on {key}")),
            score: Some(score),
            ..hit(key, "dog words", None, Some(1), None)
        };
        let cfg = RankConfig {
            facts_k: 1,
            per_bit: 0,
            ..RankConfig::default()
        };
        let hits = vec![
            hit("unit:1", "dog one", None, Some(1), None),
            hit("unit:2", "dog two", None, Some(2), None),
        ];
        let p = pack(
            "dog",
            24,
            vec![lesson("unit:1", 2.0), lesson("unit:2", 1.0)],
            hits,
            &cfg,
        );
        assert_eq!(p.facts.iter().map(|h| h.key.as_str()).collect::<Vec<_>>(), ["unit:1"]);
        assert!(
            p.hits.iter().any(|h| h.key == "unit:2"),
            "the cut lesson's record is still a hit: {:?}",
            p.hits
        );
        assert_eq!(p.trace.pool.len(), 4, "the trace keeps the read as it came in");
        assert_eq!(p.trace.pool[1][4], 1, "lessons are marked in the trace");
    }

    #[test]
    fn the_same_read_under_the_same_config_packs_the_same() {
        let read: Vec<Hit> = (0..12)
            .map(|i| {
                hit(
                    &format!("unit:{i}"),
                    &format!("word{} common", i % 4),
                    Some(&format!("joke:{}", i % 5)),
                    Some(i + 1),
                    Some(12 - i),
                )
            })
            .collect();
        let a = pack("word1 common", 8, vec![], read.clone(), &RankConfig::default());
        let b = pack("word1 common", 8, vec![], read, &RankConfig::default());
        assert_eq!(a.hits, b.hits);
        assert_eq!(a.trace.order, b.trace.order);
    }
}
