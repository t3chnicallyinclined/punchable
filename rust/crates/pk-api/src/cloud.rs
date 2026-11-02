//! The cloud map (`api-dev/cloud_api.py`): every bit placed by what it says. One vector per bit (the mean of its
//! versions' bge-small vectors, else its own), projected to three axes with PCA (each scaled to [-1, 1] at the 2nd /
//! 98th percentile), clustered with spherical k-means (k-means++ seeded exactly as numpy's `default_rng(7)`), the
//! clusters named by the words most over-represented in their titles.
//!
//! What cannot be bit-equal, and why: numpy runs the PCA as a float32 LAPACK SVD and the dot products through BLAS,
//! whose summation order is its own; here the PCA is a float64 eigen-decomposition of the covariance (`nalgebra`).
//! The coordinates agree to about 1e-6 before the 4-decimal rounding, so the T9 differential compares `x`, `y`, `z`
//! and the cluster centres within 1e-3 and everything else exactly (`rust/tools/diff_api_reads.py` `ALLOW`). The
//! random stream itself is numpy's, bit for bit (`NumpyRng`, tested against values numpy printed).

// the maths keeps the names of the numpy it mirrors (n, k, x, s, t)
#![allow(clippy::many_single_char_names)]

use std::collections::HashMap;
use std::sync::LazyLock;

use nalgebra::{DMatrix, SymmetricEigen};
use regex::Regex;
use serde_json::{Value, json};

/// `CAP_DEFAULT`, `CAP_MAX`.
pub const CAP_DEFAULT: i64 = 1600;
/// The most bits one cloud places.
pub const CAP_MAX: i64 = 2000;
/// How long one cloud is served from the cache (the sweeper may add vectors without a new generation).
pub const CACHE_S: u64 = 300;

// ------------------------------------------------------------------------------------------- numpy's default_rng

/// `numpy.random.default_rng(seed)`: `SeedSequence` + `PCG64` (XSL-RR 128/64), with the three draws k-means uses.
pub struct NumpyRng {
    state: u128,
    inc: u128,
    half: Option<u32>,
}

const PCG_MULT: u128 = 0x2360_ED05_1FC6_5DA4_4385_DF64_9FCC_F645;

fn hashmix(value: u32, hash_const: &mut u32) -> u32 {
    let mut v = value ^ *hash_const;
    *hash_const = hash_const.wrapping_mul(0x931e_8875);
    v = v.wrapping_mul(*hash_const);
    v ^ (v >> 16)
}

fn mix(x: u32, y: u32) -> u32 {
    let r = 0xca01_f9dd_u32
        .wrapping_mul(x)
        .wrapping_sub(0x4973_f715_u32.wrapping_mul(y));
    r ^ (r >> 16)
}

impl NumpyRng {
    /// `default_rng(seed)` for a seed below 2^32.
    pub fn new(seed: u32) -> Self {
        // SeedSequence(seed).generate_state(4, uint64)
        let entropy = [seed];
        let mut pool = [0u32; 4];
        let mut hc = 0x43b0_d7e5_u32;
        for (i, p) in pool.iter_mut().enumerate() {
            *p = hashmix(entropy.get(i).copied().unwrap_or(0), &mut hc);
        }
        for src in 0..4 {
            for dst in 0..4 {
                if src != dst {
                    pool[dst] = mix(pool[dst], hashmix(pool[src], &mut hc));
                }
            }
        }
        let mut hb = 0x8b51_f9dd_u32;
        let mut words = [0u32; 8];
        for (i, w) in words.iter_mut().enumerate() {
            let mut d = pool[i % 4] ^ hb;
            hb = hb.wrapping_mul(0x58f3_8ded);
            d = d.wrapping_mul(hb);
            *w = d ^ (d >> 16);
        }
        let u64_at = |i: usize| u64::from(words[2 * i]) | (u64::from(words[2 * i + 1]) << 32);
        let seed128 = (u128::from(u64_at(0)) << 64) | u128::from(u64_at(1));
        let inc128 = (u128::from(u64_at(2)) << 64) | u128::from(u64_at(3));
        let mut r = Self {
            state: 0,
            inc: (inc128 << 1) | 1,
            half: None,
        };
        r.step();
        r.state = r.state.wrapping_add(seed128);
        r.step();
        r
    }

    fn step(&mut self) {
        self.state = self.state.wrapping_mul(PCG_MULT).wrapping_add(self.inc);
    }

    fn next_u64(&mut self) -> u64 {
        self.step();
        let x = ((self.state >> 64) as u64) ^ (self.state as u64);
        x.rotate_right((self.state >> 122) as u32)
    }

    fn next_u32(&mut self) -> u32 {
        if let Some(h) = self.half.take() {
            return h;
        }
        let n = self.next_u64();
        self.half = Some((n >> 32) as u32);
        n as u32
    }

    /// `rng.random()`.
    pub fn random(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }

    /// `rng.integers(n)` for `0 < n <= 2^32` (Lemire's bounded draw on the 32-bit stream, as numpy does).
    pub fn integers(&mut self, n: u64) -> u64 {
        let rng = n - 1;
        if rng == 0 {
            return 0;
        }
        let excl = n;
        let mut m = u64::from(self.next_u32()) * excl;
        let mut left = m & 0xffff_ffff;
        if left < excl {
            let threshold = (u64::from(u32::MAX) - rng) % excl;
            while left < threshold {
                m = u64::from(self.next_u32()) * excl;
                left = m & 0xffff_ffff;
            }
        }
        m >> 32
    }

    /// `rng.choice(n, p=p)`: the cumulative sum in float64, one `random()`, `searchsorted(side="right")`.
    pub fn choice(&mut self, p: &[f64]) -> usize {
        let mut cdf = Vec::with_capacity(p.len());
        let mut acc = 0.0;
        for x in p {
            acc += x;
            cdf.push(acc);
        }
        let last = cdf.last().copied().unwrap_or(1.0);
        for c in &mut cdf {
            *c /= last;
        }
        let u = self.random();
        cdf.partition_point(|c| *c <= u).min(p.len().saturating_sub(1))
    }
}

// --------------------------------------------------------------------------------------------------- the maths

/// numpy's `percentile(col, q)` (linear interpolation, its two-sided lerp).
fn percentile(sorted: &[f64], q: f64) -> f64 {
    let n = sorted.len();
    let pos = q / 100.0 * (n as f64 - 1.0);
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(n - 1);
    let t = pos - lo as f64;
    let (a, b) = (sorted[lo], sorted[hi]);
    if t >= 0.5 {
        b - (b - a) * (1.0 - t)
    } else {
        a + (b - a) * t
    }
}

/// `project(M, 3)`: PCA to three axes, the sign of each set so its largest-loading dimension is positive, each axis
/// scaled to [-1, 1] at its 2nd / 98th percentile and clamped.
pub fn project(rows: &[Vec<f32>], dims: usize) -> Vec<[f64; 3]> {
    let n = rows.len();
    if n <= 1 {
        return vec![[0.0; 3]; n];
    }
    let d = rows[0].len();
    let mut mean = vec![0.0f64; d];
    for r in rows {
        for (m, x) in mean.iter_mut().zip(r) {
            *m += f64::from(*x);
        }
    }
    for m in &mut mean {
        *m /= n as f64;
    }
    let xc = DMatrix::<f64>::from_fn(n, d, |i, j| f64::from(rows[i][j]) - mean[j]);
    let cov = xc.transpose() * &xc;
    let eig = SymmetricEigen::new(cov);
    let mut order: Vec<usize> = (0..d).collect();
    order.sort_by(|a, b| eig.eigenvalues[*b].total_cmp(&eig.eigenvalues[*a]));
    // numpy's SVD has min(n, d) right vectors: an axis beyond them is zero
    let ncomp = dims.min(n).min(d);
    let mut out = vec![[0.0f64; 3]; n];
    for (axis, &ci) in order.iter().take(ncomp).enumerate() {
        let mut comp: Vec<f64> = eig.eigenvectors.column(ci).iter().copied().collect();
        let j = comp
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()).then(b.0.cmp(&a.0)))
            .map_or(0, |(j, _)| j);
        if comp[j] < 0.0 {
            for c in &mut comp {
                *c = -*c;
            }
        }
        let col: Vec<f64> = (0..n)
            .map(|i| xc.row(i).iter().zip(&comp).map(|(x, c)| x * c).sum())
            .collect();
        let mut sorted = col.clone();
        sorted.sort_by(f64::total_cmp);
        let (lo, hi) = (percentile(&sorted, 2.0), percentile(&sorted, 98.0));
        if hi - lo < 1e-9 {
            continue;
        }
        for (i, v) in col.iter().enumerate() {
            out[i][axis] = ((v - lo) / (hi - lo) * 2.0 - 1.0).clamp(-1.0, 1.0);
        }
    }
    out
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Spherical k-means, k-means++ seeding from `default_rng(7)`, at most 25 rounds. Returns the labels.
pub fn kmeans(x: &[Vec<f32>], k: usize) -> Vec<usize> {
    let n = x.len();
    let k = k.clamp(1, n.max(1));
    let mut rng = NumpyRng::new(7);
    let mut centres: Vec<Vec<f32>> = vec![x[rng.integers(n as u64) as usize].clone()];
    for _ in 1..k {
        let d2: Vec<f32> = x
            .iter()
            .map(|r| {
                let s = centres.iter().map(|c| dot(r, c)).fold(f32::NEG_INFINITY, f32::max);
                (1.0 - s).max(0.0).powi(2)
            })
            .collect();
        let s: f32 = d2.iter().sum();
        let idx = if s <= 0.0 {
            rng.integers(n as u64) as usize
        } else {
            let p: Vec<f64> = d2.iter().map(|v| f64::from(v / s)).collect();
            rng.choice(&p)
        };
        centres.push(x[idx].clone());
    }
    let mut labels = vec![0usize; n];
    for round in 0..25 {
        let new: Vec<usize> = x
            .iter()
            .map(|r| {
                let mut best = (0usize, f32::NEG_INFINITY);
                for (c, cv) in centres.iter().enumerate() {
                    let s = dot(r, cv);
                    if s > best.1 {
                        best = (c, s);
                    }
                }
                best.0
            })
            .collect();
        if new == labels && round > 0 {
            break;
        }
        labels = new;
        for (c, centre) in centres.iter_mut().enumerate() {
            let members: Vec<&Vec<f32>> = x
                .iter()
                .zip(&labels)
                .filter(|(_, l)| **l == c)
                .map(|(r, _)| r)
                .collect();
            if members.is_empty() {
                continue;
            }
            let mut v = vec![0.0f32; centre.len()];
            for m in &members {
                for (a, b) in v.iter_mut().zip(m.iter()) {
                    *a += b;
                }
            }
            for a in &mut v {
                *a /= members.len() as f32;
            }
            let norm = v.iter().map(|a| a * a).sum::<f32>().sqrt();
            let norm = if norm == 0.0 { 1.0 } else { norm };
            *centre = v.into_iter().map(|a| a / norm).collect();
        }
    }
    labels
}

/// `pick_k`.
pub fn pick_k(n: usize) -> usize {
    if n >= 12 {
        let r = ((n as f64) / 2.0).sqrt();
        // Python's round(): ties to even
        let k = r.round_ties_even() as usize;
        k.clamp(6, 14).max(1)
    } else {
        n.clamp(1, 3)
    }
}

static STOP_SET: LazyLock<std::collections::HashSet<&'static str>> =
    LazyLock::new(|| STOP.split_whitespace().collect());

static WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[a-z][a-z']+").unwrap_or_else(|e| unreachable!("constant regex: {e}")));

const STOP: &str = "a an the and or but so to of in on at for with is it's its i i'm im you your he she they we me my \
our us them his her their this that these those was were be been being are am do does did have has had not no yes \
if then than when where what who why how all any some one two about into over under out up down off from by as \
just like get got go going went come came say said says make made thing things really very also too again more \
most much many can could would should will shall may might must there here now never always every ever still \
even way yeah ok okay oh uh um dont don't cant can't didnt didn't wont won't thats that's youre you're ive i've \
bit joke jokes tag tags idea ideas story version part new old note notes set list roast";

fn words(title: &str) -> Vec<String> {
    let lower = title.to_lowercase();
    let mut out: Vec<String> = Vec::new();
    for m in WORD.find_iter(&lower) {
        let w = m.as_str();
        if w.chars().count() > 2 && !STOP_SET.contains(w) && !out.iter().any(|o| o == w) {
            out.push(w.to_string());
        }
    }
    out
}

/// `label_clusters`: the two or three words most over-represented in each cluster's titles.
pub fn label_clusters(titles: &[(usize, Vec<String>)]) -> HashMap<usize, String> {
    let mut total: HashMap<String, usize> = HashMap::new();
    let mut per: Vec<(usize, Vec<(String, usize)>)> = Vec::new();
    for (cid, ts) in titles {
        let mut c: Vec<(String, usize)> = Vec::new();
        for t in ts {
            for w in words(t) {
                match c.iter_mut().find(|(x, _)| *x == w) {
                    Some(e) => e.1 += 1,
                    None => c.push((w, 1)),
                }
            }
        }
        for (w, n) in &c {
            *total.entry(w.clone()).or_default() += n;
        }
        per.push((*cid, c));
    }
    let n_all = titles.iter().map(|(_, t)| t.len()).sum::<usize>().max(1) as f64;
    let mut out = HashMap::new();
    for (cid, c) in per {
        let n = titles
            .iter()
            .find(|(x, _)| *x == cid)
            .map_or(1, |(_, t)| t.len().max(1)) as f64;
        let mut scored: Vec<(f64, &str)> = c
            .iter()
            .filter(|(_, cnt)| !(*cnt < 2 && n > 3.0))
            .map(|(w, cnt)| {
                let cnt = *cnt as f64;
                let lift = (cnt / n) / (total[w] as f64 / n_all);
                (lift * cnt.ln_1p(), w.as_str())
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(a.1)));
        let mut picked: Vec<&str> = scored.iter().take(3).map(|(_, w)| *w).collect();
        if picked.is_empty()
            && let Some(top) = c.iter().max_by(|a, b| a.1.cmp(&b.1).then(std::cmp::Ordering::Greater))
        {
            picked.push(top.0.as_str());
        }
        out.insert(
            cid,
            if picked.is_empty() {
                "—".to_string()
            } else {
                picked.join(" · ")
            },
        );
    }
    out
}

/// Python's `round(x, 4)`.
fn r4(x: f64) -> f64 {
    format!("{x:.4}").parse().unwrap_or(x)
}

/// One bit on the map, as the index row gives it.
pub struct Bit<'a> {
    /// The joke id.
    pub id: &'a str,
    /// What he writes on a paper set list (else the short title, else the title).
    pub short_name: &'a str,
    /// The title.
    pub title: &'a str,
    /// The stage.
    pub stage: Option<&'a str>,
    /// Nights on tape.
    pub nights: u64,
    /// Mean `vs_room`.
    pub heat: Option<f64>,
    /// His last decision on it.
    pub touched: Option<&'a str>,
}

/// `build`: the placed bits (already capped and ordered) with their vectors.
pub fn build(bits: &[Bit<'_>], vecs: &[Vec<f32>]) -> (Vec<Value>, Vec<Value>) {
    let p = project(vecs, 3);
    let labels = kmeans(vecs, pick_k(bits.len()));
    let mut titles: Vec<(usize, Vec<String>)> = Vec::new();
    for (b, l) in bits.iter().zip(&labels) {
        match titles.iter_mut().find(|(c, _)| c == l) {
            Some(e) => e.1.push(b.title.to_string()),
            None => titles.push((*l, vec![b.title.to_string()])),
        }
    }
    let names = label_clusters(&titles);
    let nodes: Vec<Value> = bits
        .iter()
        .zip(&p)
        .zip(&labels)
        .map(|((b, xyz), l)| {
            json!({"id": b.id, "short_name": b.short_name, "title": b.title, "stage": b.stage, "nights": b.nights,
                   "heat": b.heat, "touched": b.touched, "x": r4(xyz[0]), "y": r4(xyz[1]), "z": r4(xyz[2]), "cluster": l})
        })
        .collect();
    let mut ids: Vec<usize> = titles.iter().map(|(c, _)| *c).collect();
    ids.sort_unstable();
    let mut clusters: Vec<Value> = ids
        .into_iter()
        .map(|cid| {
            let pts: Vec<&Value> = nodes.iter().filter(|n| n["cluster"] == cid).collect();
            let avg = |k: &str| r4(pts.iter().map(|n| n[k].as_f64().unwrap_or(0.0)).sum::<f64>() / pts.len() as f64);
            json!({"id": cid, "label": names.get(&cid).cloned().unwrap_or_else(|| "—".into()), "size": pts.len(),
                   "cx": avg("x"), "cy": avg("y"), "cz": avg("z")})
        })
        .collect();
    clusters.sort_by_key(|c| std::cmp::Reverse(c["size"].as_u64().unwrap_or(0)));
    (nodes, clusters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_cmp)] // the stream must be numpy's bit for bit
    fn the_random_stream_is_numpys() {
        // printed by numpy 2.2: r = default_rng(7); [r.integers(100) x5], r.random(), [r.integers(1600) x3], ...
        let mut r = NumpyRng::new(7);
        let ints: Vec<u64> = (0..5).map(|_| r.integers(100)).collect();
        assert_eq!(ints, vec![94, 62, 68, 89, 57]);
        assert_eq!(r.random(), 0.225_207_189_990_591_86);
        let ints: Vec<u64> = (0..3).map(|_| r.integers(1600)).collect();
        assert_eq!(ints, vec![1241, 88, 480]);
        assert_eq!(r.random(), 0.873_553_445_396_261_9);
        assert_eq!(r.integers(7), 6);
    }

    #[test]
    fn the_seeded_state_is_numpys() {
        let r = NumpyRng::new(7);
        assert_eq!(r.state, 208_745_520_555_909_116_978_795_849_195_383_758_904);
        assert_eq!(r.inc, 261_136_684_632_268_670_825_940_853_076_396_136_793);
    }

    #[test]
    fn k_and_percentiles() {
        assert_eq!(pick_k(5), 3);
        assert_eq!(pick_k(12), 6);
        assert_eq!(pick_k(1576), 14);
        assert!((percentile(&[1.0, 2.0, 3.0, 4.0], 50.0) - 2.5).abs() < 1e-12);
    }
}
