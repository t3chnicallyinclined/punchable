//! The his-words search over the sources (`creative/askchat.py::_index`, `_bm25`): a term-frequency index with the
//! title's words counted twice, and the scoring the Python does, written out here because the proof is "the same
//! source ids in the same order" and only the identical formula (its constants, its document-frequency cap, its
//! summation order) gives that. Twenty lines of arithmetic, not a search engine: the hosted retrieval will move to
//! `SurrealDB`'s own BM25 when the API lane cuts over (docs/RUST.md "Crates we stand on").

use std::collections::{HashMap, HashSet};

use super::words::tokens;
use super::{QSTOP, Sources};

/// One source's term frequencies and length.
struct Doc {
    sid: String,
    tf: HashMap<String, u32>,
    /// `sum(tf.values()) or 1`.
    dl: u32,
}

/// The index over a notebook's sources, in source order.
pub(crate) struct Index {
    docs: Vec<Doc>,
    df: HashMap<String, usize>,
    /// Mean document length, `1.0` when the notebook is empty.
    avg: f64,
}

impl Index {
    /// `_index(c)`: term frequencies per source (title twice, then text), document frequencies, the mean length.
    pub(crate) fn build(sources: &Sources) -> Self {
        let mut docs = Vec::with_capacity(sources.len());
        let mut df: HashMap<String, usize> = HashMap::new();
        let mut total: u64 = 0;
        for (sid, s) in sources {
            let mut tf: HashMap<String, u32> = HashMap::new();
            let title = tokens(&s.title);
            for w in title.iter().chain(title.iter()).cloned().chain(tokens(&s.text)) {
                *tf.entry(w).or_insert(0) += 1;
            }
            for w in tf.keys() {
                *df.entry(w.clone()).or_insert(0) += 1;
            }
            let sum: u32 = tf.values().sum();
            total += u64::from(sum);
            docs.push(Doc {
                sid: sid.clone(),
                tf,
                dl: sum.max(1),
            });
        }
        let avg = total as f64 / docs.len().max(1) as f64;
        let avg = if avg == 0.0 { 1.0 } else { avg };
        Index { docs, df, avg }
    }

    /// `_bm25(c, text)`: every source with a positive score, in source order. The query is the text's distinct
    /// search words (first-seen order) that are not question words and that occur in at least one and at most
    /// `max(3, n // 4)` sources; the score sums `idf * f * 2.2 / (f + 1.2 * (0.25 + 0.75 * dl / avg))` over them in
    /// that order, with `idf = ln(1 + (n - df + 0.5) / (df + 0.5))`.
    pub(crate) fn score(&self, text: &str) -> Vec<(String, f64)> {
        let n = self.docs.len().max(1);
        let cap = (n / 4).max(3);
        let mut seen = HashSet::new();
        let q: Vec<String> = tokens(text)
            .into_iter()
            .filter(|w| seen.insert(w.clone()))
            .filter(|w| !QSTOP.contains(&w.as_str()))
            .filter(|w| self.df.get(w).is_some_and(|&d| d > 0 && d <= cap))
            .collect();
        let mut out = Vec::new();
        for doc in &self.docs {
            let dl = f64::from(doc.dl);
            let mut s = 0.0;
            for w in &q {
                let f = doc.tf.get(w).copied().unwrap_or(0);
                if f > 0 {
                    let df = self.df[w];
                    let idf = (1.0 + ((n - df) as f64 + 0.5) / (df as f64 + 0.5)).ln();
                    let f = f64::from(f);
                    s += idf * f * 2.2 / (f + 1.2 * (0.25 + 0.75 * dl / self.avg));
                }
            }
            if s > 0.0 {
                out.push((doc.sid.clone(), s));
            }
        }
        out
    }
}
