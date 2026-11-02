//! Same-idea groups and auto-attach (`creative/merge.py`), the halves with no model and no IO: the pair index
//! (each bit's texts, distinctive phrases, rare and title words, unit vectors with each unit's own "usually this
//! close" level), the pair features and score, the judge's and the second look's prompts and how their answers
//! land in the cache, the verdict, the groups, and `match_text` (where a new jot belongs).
//!
//! The unit vectors are normalised with numpy's pairwise float32 sum (exactly); their similarity matrix is a BLAS
//! product in the Python, so a cosine here equals it to float32 precision (the goldens compare it that way).

#![allow(clippy::implicit_hasher)] // the sets and maps passed in are this crate's own (std's hasher)
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::LazyLock;

use pk_core::py;
use regex::Regex;
use serde_json::{Map, Value, json};
use sha1::{Digest as _, Sha1};

use crate::corpus::{Corpus, num, round_to, s};
use crate::memolink::{squash, units_of};

/// `UNIT_CAND`: a unit-unit cosine that makes a pair worth scoring.
pub const UNIT_CAND: f32 = 0.72;
/// `JUDGE_FLOOR`.
pub const JUDGE_FLOOR: f64 = 0.25;
/// `SAME_MIN_CONF`.
pub const SAME_MIN_CONF: f64 = 0.80;
/// `JUDGE_BATCH`.
pub const JUDGE_BATCH: usize = 15;
/// `SECOND_LO`.
pub const SECOND_LO: f64 = 0.55;
/// `SECOND_BATCH`.
pub const SECOND_BATCH: usize = 5;
/// `ATTACH_BAND`.
pub const ATTACH_BAND: f64 = 0.10;
/// `ATTACH_MARGIN`.
pub const ATTACH_MARGIN: f64 = 0.04;
/// `ATTACH_MIN_WORDS`.
pub const ATTACH_MIN_WORDS: usize = 4;

/// `JUDGE_SYSTEM`.
pub const JUDGE_SYSTEM: &str = "You sort one stand-up comedian's own notes. For each pair, decide if A and B are THE \
SAME JOKE: the same premise taken the same way, so a comic would keep them as one bit with versions. Same joke: a \
rewrite or later version, a one-line jot of the bit, a longer draft of it, the same story told twice, the same \
argument with the same angle. NOT the same joke: the same broad topic (dating, racism, my niece, Facebook, Uber) with \
a different premise or angle; two different jokes that happen to share a line; roasts of two different people; a set \
list or idea dump that mentions it; one being the other side of an argument. When unsure, answer same=false. \
confidence is how sure you are of your answer (0-1). why is one plain sentence a comic would understand, naming the \
shared premise or the difference.";

/// `JUDGE_SCHEMA`.
pub static JUDGE_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({"type": "object", "properties": {"pairs": {"type": "array", "items": {
        "type": "object",
        "properties": {"id": {"type": "string"}, "same": {"type": "boolean"}, "confidence": {"type": "number"},
                       "why": {"type": "string"}},
        "required": ["id", "same", "confidence", "why"], "additionalProperties": false}}},
        "required": ["pairs"], "additionalProperties": false})
});

static TOK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z0-9]+(?:'[a-z]+)?").expect("a valid regex"));
static STOP: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    "a an the and or but so to of in on at for with is it's its i i'm im you your he she they we me my \
     this that was were be been are am do did does have has had just like if then there what when how who why not no \
     yeah um uh oh ok okay all one get got go going gonna know about from as by up out can can't don't would could \
     should will his her him them their our us it them than too very really also more some any into over about \
     said says say thing things people time good bad want make made see now well even ever every back still \
     dont cant im its thats whats"
        .split_whitespace()
        .collect()
});
static TITLE_STOP: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    let mut t = STOP.clone();
    t.extend("bit joke tag tags idea story version part new old note notes set list roast".split_whitespace());
    t
});

fn stage_rank(s: &str) -> i64 {
    match s {
        "working" => 4,
        "testing" => 3,
        "to_try" => 2,
        "idea" => 1,
        _ => 0,
    }
}

/// `_toks(t)`: lower case, curly and broken apostrophes straightened, words.
pub fn toks(t: &str) -> Vec<String> {
    let n = t.to_lowercase().replace(['\u{2019}', '\u{2018}', '\u{fffd}'], "'");
    TOK.find_iter(&n).map(|m| m.as_str().to_string()).collect()
}

fn is_stop(w: &str) -> bool {
    STOP.contains(w)
}

fn grams(ts: &[String], k: usize) -> HashSet<Vec<String>> {
    if ts.len() < k {
        return HashSet::new();
    }
    (0..=ts.len() - k).map(|i| ts[i..i + k].to_vec()).collect()
}

fn content(ts: &[String]) -> HashSet<String> {
    ts.iter()
        .filter(|w| !is_stop(w) && w.chars().count() > 2)
        .cloned()
        .collect()
}

fn g3(ts: &[String]) -> HashSet<Vec<String>> {
    grams(ts, 3)
        .into_iter()
        .filter(|g| g.iter().filter(|w| !is_stop(w)).count() >= 2)
        .collect()
}

fn g2(ts: &[String]) -> HashSet<Vec<String>> {
    grams(ts, 2)
        .into_iter()
        .filter(|g| !is_stop(&g[0]) && !is_stop(&g[1]))
        .collect()
}

fn stem(w: &str) -> String {
    for suf in ["ing", "ers", "er", "es", "ed", "s"] {
        if w.chars().count() > suf.len() + 3
            && let Some(x) = w.strip_suffix(suf)
        {
            return x.to_string();
        }
    }
    w.to_string()
}

/// numpy's pairwise float32 sum (`pairwise_sum` for a contiguous float32 run: 8 accumulators up to 128 values,
/// halves above, at a multiple of 8), which is what `np.linalg.norm(m, axis=1)` reduces with.
pub fn pairwise_sum(xs: &[f32]) -> f32 {
    let n = xs.len();
    if n < 8 {
        return xs.iter().fold(-0.0_f32, |a, x| a + x);
    }
    if n <= 128 {
        let mut r = [xs[0], xs[1], xs[2], xs[3], xs[4], xs[5], xs[6], xs[7]];
        let mut i = 8;
        while i < n - (n % 8) {
            for (k, rk) in r.iter_mut().enumerate() {
                *rk += xs[i + k];
            }
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        for x in &xs[i..] {
            res += x;
        }
        return res;
    }
    let mut n2 = n / 2;
    n2 -= n2 % 8;
    pairwise_sum(&xs[..n2]) + pairwise_sum(&xs[n2..])
}

/// `_ref(rows)`: each row's k-th highest value, k = 1% of the columns (at least 10); 0.99 when there are no more
/// columns than that.
fn reference(row: &[f32]) -> f32 {
    let n = row.len();
    let k = 10.max(n / 100);
    if n <= k {
        return 0.99;
    }
    let mut v = row.to_vec();
    let (_, kth, _) = v.select_nth_unstable_by(k - 1, |a, b| b.total_cmp(a));
    *kth
}

/// A float32 dot product in eight lanes (what a BLAS kernel does; the goldens compare cosines at float32 precision).
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0_f32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let tail: f32 = ca.remainder().iter().zip(cb.remainder()).map(|(x, y)| x * y).sum();
    for (x, y) in ca.zip(cb) {
        for l in 0..8 {
            acc[l] += x[l] * y[l];
        }
    }
    ((acc[0] + acc[1]) + (acc[2] + acc[3])) + ((acc[4] + acc[5]) + (acc[6] + acc[7])) + tail
}

/// Everything pair scoring needs, built once per corpus (`merge.Index`).
#[derive(Debug)]
pub struct Index {
    c: std::sync::Arc<Corpus>,
    /// The jokes, sorted.
    pub jokes: Vec<String>,
    /// Each joke's units (non-hidden, sorted).
    pub units: HashMap<String, Vec<String>>,
    parts: HashMap<String, Vec<String>>,
    ucont: HashMap<String, HashSet<String>>,
    ug3: HashMap<String, HashSet<Vec<String>>>,
    ug2: HashMap<String, HashSet<Vec<String>>>,
    df_w: HashMap<String, usize>,
    df_3: HashMap<Vec<String>, usize>,
    df_2: HashMap<Vec<String>, usize>,
    ttoks: BTreeMap<String, BTreeSet<String>>,
    df_t: HashMap<String, usize>,
    vec_units: Vec<String>,
    urow: HashMap<String, usize>,
    ujoke: HashMap<String, String>,
    m: Vec<f32>,
    s: Vec<f32>,
    p99: Vec<f32>,
    premise: BTreeMap<(String, String), usize>,
}

fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

impl Index {
    /// Build it (`Index(c)`).
    #[allow(clippy::too_many_lines, clippy::many_single_char_names)]
    pub fn new(c: std::sync::Arc<Corpus>) -> Self {
        let mut jokes: Vec<String> = c.jokes.clone();
        jokes.sort();
        let units: HashMap<String, Vec<String>> = jokes.iter().map(|j| (j.clone(), units_of(&c, j))).collect();
        let mut utoks: Vec<(String, Vec<String>)> = Vec::new();
        for j in &jokes {
            for u in &units[j] {
                utoks.push((u.clone(), toks(c.node_text(u))));
            }
            if units[j].is_empty() {
                let summary = c.joke(j).and_then(|n| s(n, "summary")).unwrap_or("");
                utoks.push((format!("summary:{j}"), toks(summary)));
            }
        }
        let parts: HashMap<String, Vec<String>> = jokes
            .iter()
            .map(|j| {
                let us = &units[j];
                (
                    j.clone(),
                    if us.is_empty() {
                        vec![format!("summary:{j}")]
                    } else {
                        us.clone()
                    },
                )
            })
            .collect();
        let ucont: HashMap<String, HashSet<String>> = utoks.iter().map(|(u, ts)| (u.clone(), content(ts))).collect();
        let ug3: HashMap<String, HashSet<Vec<String>>> = utoks.iter().map(|(u, ts)| (u.clone(), g3(ts))).collect();
        let ug2: HashMap<String, HashSet<Vec<String>>> = utoks.iter().map(|(u, ts)| (u.clone(), g2(ts))).collect();
        let mut df_w: HashMap<String, usize> = HashMap::new();
        for w in ucont.values().flatten() {
            *df_w.entry(w.clone()).or_default() += 1;
        }
        let mut df_3: HashMap<Vec<String>, usize> = HashMap::new();
        for g in ug3.values().flatten() {
            *df_3.entry(g.clone()).or_default() += 1;
        }
        let mut df_2: HashMap<Vec<String>, usize> = HashMap::new();
        for g in ug2.values().flatten() {
            *df_2.entry(g.clone()).or_default() += 1;
        }
        let ttoks: BTreeMap<String, BTreeSet<String>> = jokes
            .iter()
            .map(|j| {
                let title = c.joke(j).and_then(|n| s(n, "title")).unwrap_or("");
                let ws = toks(title)
                    .into_iter()
                    .filter(|w| !TITLE_STOP.contains(w.as_str()) && w.chars().count() > 2)
                    .map(|w| stem(&w))
                    .collect();
                (j.clone(), ws)
            })
            .collect();
        let mut df_t: HashMap<String, usize> = HashMap::new();
        for w in ttoks.values().flatten() {
            *df_t.entry(w.clone()).or_default() += 1;
        }
        let vec_units: Vec<String> = jokes
            .iter()
            .flat_map(|j| parts[j].iter())
            .filter(|u| c.vectors.get(u).is_some())
            .cloned()
            .collect();
        let urow = vec_units.iter().enumerate().map(|(i, u)| (u.clone(), i)).collect();
        let ujoke: HashMap<String, String> = jokes
            .iter()
            .flat_map(|j| parts[j].iter().map(move |u| (u.clone(), j.clone())))
            .collect();
        let dim = crate::corpus::DIM;
        let mut m: Vec<f32> = Vec::with_capacity(vec_units.len() * dim);
        for u in &vec_units {
            let v = c.vectors.get(u).unwrap_or(&[]);
            let sq: Vec<f32> = v.iter().map(|x| x * x).collect();
            let norm = pairwise_sum(&sq).sqrt().max(1e-9);
            m.extend(v.iter().map(|x| x / norm));
        }
        // `S = M @ M.T` with each joke's own block at -1: the upper triangle, mirrored (a dot product is the same
        // both ways round), joke identity as a row index rather than a string compare per cell
        let n = vec_units.len();
        let jix: HashMap<&str, usize> = jokes.iter().enumerate().map(|(i, j)| (j.as_str(), i)).collect();
        let rj: Vec<usize> = vec_units
            .iter()
            .map(|u| ujoke.get(u).and_then(|j| jix.get(j.as_str())).copied().unwrap_or(usize::MAX))
            .collect();
        let mut sm = vec![0.0_f32; n * n];
        for i in 0..n {
            let a = &m[i * dim..(i + 1) * dim];
            for k in i..n {
                let v = if rj[i] == rj[k] {
                    -1.0
                } else {
                    dot(a, &m[k * dim..(k + 1) * dim])
                };
                sm[i * n + k] = v;
                sm[k * n + i] = v;
            }
        }
        let p99: Vec<f32> = (0..n).map(|i| reference(&sm[i * n..(i + 1) * n])).collect();
        let mut premise = BTreeMap::new();
        let joke_set: HashSet<&String> = jokes.iter().collect();
        for (i, e) in c.edges.iter().enumerate() {
            if s(e, "kind") != Some("same_premise") {
                continue;
            }
            let side = |k: &str| {
                let x = s(e, k).unwrap_or("");
                c.unit_joke.get(x).cloned().unwrap_or_else(|| x.to_string())
            };
            let (a, b) = (side("src"), side("dst"));
            if a != b && joke_set.contains(&a) && joke_set.contains(&b) {
                premise.insert(pair(&a, &b), i);
            }
        }
        Self {
            c,
            jokes,
            units,
            parts,
            ucont,
            ug3,
            ug2,
            df_w,
            df_3,
            df_2,
            ttoks,
            df_t,
            vec_units,
            urow,
            ujoke,
            m,
            s: sm,
            p99,
            premise,
        }
    }

    fn s_at(&self, i: usize, k: usize) -> f32 {
        self.s[i * self.vec_units.len() + k]
    }

    /// `phrases(a, b)`: distinctive word runs both use, a run inside a longer shared run counted once, sorted.
    pub fn phrases(&self, a: &str, b: &str) -> Vec<String> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        for u in &self.parts[a] {
            for v in &self.parts[b] {
                for g in self.ug3[u].intersection(&self.ug3[v]) {
                    if self.df_3[g] <= 3 {
                        out.insert(g.join(" "));
                    }
                }
                for g in self.ug2[u].intersection(&self.ug2[v]) {
                    if self.df_2[g] <= 3 {
                        out.insert(g.join(" "));
                    }
                }
            }
        }
        out.iter()
            .filter(|p| !out.iter().any(|q| q != *p && q.contains(p.as_str())))
            .cloned()
            .collect()
    }

    fn words_of(&self, j: &str) -> HashSet<&String> {
        self.parts[j].iter().flat_map(|u| self.ucont[u].iter()).collect()
    }

    /// `rare_words(a, b)`: content words both use that few texts have, rarest first.
    pub fn rare_words(&self, a: &str, b: &str) -> Vec<String> {
        let (wa, wb) = (self.words_of(a), self.words_of(b));
        let mut out: Vec<&String> = wa.intersection(&wb).copied().filter(|w| self.df_w[*w] <= 4).collect();
        out.sort_by(|x, y| (self.df_w[*x], *x).cmp(&(self.df_w[*y], *y)));
        out.into_iter().cloned().collect()
    }

    /// `title_words(a, b)`.
    pub fn title_words(&self, a: &str, b: &str) -> Vec<String> {
        self.ttoks[a]
            .intersection(&self.ttoks[b])
            .filter(|w| self.df_t[*w] <= 12)
            .cloned()
            .collect()
    }

    /// `best_units(a, b)`: (cosine, band, unit a, unit b) of the closest pair of units.
    pub fn best_units(&self, a: &str, b: &str) -> Option<(f64, f64, String, String)> {
        if self.vec_units.is_empty() {
            return None;
        }
        let ra: Vec<usize> = self.parts[a].iter().filter_map(|u| self.urow.get(u).copied()).collect();
        let rb: Vec<usize> = self.parts[b].iter().filter_map(|u| self.urow.get(u).copied()).collect();
        if ra.is_empty() || rb.is_empty() {
            return None;
        }
        let (mut bi, mut bk, mut best) = (0, 0, f32::NEG_INFINITY);
        for (i, &r) in ra.iter().enumerate() {
            for (k, &q) in rb.iter().enumerate() {
                let v = self.s_at(r, q);
                if v > best {
                    (bi, bk, best) = (i, k, v);
                }
            }
        }
        let cs = f64::from(best);
        let band = (cs - f64::from(self.p99[ra[bi]])).min(cs - f64::from(self.p99[rb[bk]]));
        Some((cs, band, self.vec_units[ra[bi]].clone(), self.vec_units[rb[bk]].clone()))
    }

    fn containment(&self, u: &str, v: &str) -> f64 {
        let empty = HashSet::new();
        let (a, b) = (self.ucont.get(u).unwrap_or(&empty), self.ucont.get(v).unwrap_or(&empty));
        let small = a.len().min(b.len());
        if small == 0 {
            0.0
        } else {
            a.intersection(b).count() as f64 / small as f64
        }
    }

    /// `candidates()`: close units, same-premise edges, and bits sharing a rare title word; sorted.
    pub fn candidates(&self) -> BTreeSet<(String, String)> {
        let mut pairs = BTreeSet::new();
        let n = self.vec_units.len();
        for i in 0..n {
            for k in i + 1..n {
                if self.s_at(i, k) >= UNIT_CAND {
                    let (a, b) = (&self.ujoke[&self.vec_units[i]], &self.ujoke[&self.vec_units[k]]);
                    if a != b {
                        pairs.insert(pair(a, b));
                    }
                }
            }
        }
        pairs.extend(self.premise.keys().cloned());
        let mut by_t: BTreeMap<&String, Vec<&String>> = BTreeMap::new();
        for (j, ws) in &self.ttoks {
            for w in ws {
                if self.df_t[w] <= 12 {
                    by_t.entry(w).or_default().push(j);
                }
            }
        }
        for js in by_t.values() {
            for x in 0..js.len() {
                for y in x + 1..js.len() {
                    pairs.insert(pair(js[x], js[y]));
                }
            }
        }
        pairs
    }

    /// `features(a, b)`.
    pub fn features(&self, a: &str, b: &str) -> Value {
        let mut f = json!({"a": a, "b": b, "cos": null, "band": null, "units": null, "contain": 0.0,
                           "phrases": self.phrases(a, b), "rare": self.rare_words(a, b), "title": self.title_words(a, b),
                           "premise": null, "n_a": self.units[a].len(), "n_b": self.units[b].len()});
        if let Some((cs, band, ua, ub)) = self.best_units(a, b) {
            f["cos"] = json!(round_to(cs, 3));
            f["band"] = json!(round_to(band, 3));
            f["contain"] = json!(round_to(self.containment(&ua, &ub), 2));
            f["units"] = json!([ua, ub]);
        }
        if let Some(&i) = self.premise.get(&pair(a, b)) {
            let e = &self.c.edges[i];
            f["premise"] = json!({"edge": e.get("id"), "confidence": e.get("confidence").cloned().unwrap_or(Value::Null),
                                  "source": self.c.source_of(e), "reason": e.get("reason").cloned().unwrap_or(Value::Null)});
        }
        f
    }

    /// `_excerpt(c, ix, j, unit)`: what the judge reads of one side.
    pub fn excerpt(&self, j: &str, unit: Option<&str>, n: usize) -> String {
        let c = &*self.c;
        let from_unit = unit
            .and_then(|u| c.nodes.get(u))
            .and_then(|x| s(x, "text"))
            .filter(|t| !t.is_empty());
        let joke = c.joke(j);
        let t = from_unit
            .or_else(|| joke.and_then(|x| s(x, "summary")).filter(|t| !t.is_empty()))
            .or_else(|| joke.and_then(|x| s(x, "title")).filter(|t| !t.is_empty()))
            .unwrap_or("");
        let t = squash(t);
        let head: String = t.chars().take(n).collect();
        if t.chars().count() > n {
            format!("{head}...")
        } else {
            head
        }
    }

    /// `judge_key(c, ix, f)`: the two excerpts as shown (order-free), hashed: an edit to either asks again.
    pub fn judge_key(&self, f: &Value) -> String {
        let (ua, ub) = units_pair(f);
        let (a, b) = (f["a"].as_str().unwrap_or(""), f["b"].as_str().unwrap_or(""));
        let mut xs = [
            format!("{}\n{}", self.c.title(a), self.excerpt(a, ua.as_deref(), 900)),
            format!("{}\n{}", self.c.title(b), self.excerpt(b, ub.as_deref(), 900)),
        ];
        xs.sort();
        hex::encode(Sha1::digest(xs.join("\n--\n").as_bytes()))[..16].to_string()
    }

    /// `_full(c, ix, j)`: every written version, oldest first, for the second look.
    pub fn full(&self, j: &str, n: usize) -> String {
        let c = &*self.c;
        let date = |u: &str| c.nodes.get(u).and_then(|x| s(x, "date")).unwrap_or("").to_string();
        let mut us: Vec<&String> = self.units.get(j).map(|u| u.iter().collect()).unwrap_or_default();
        us.sort_by(|x, y| (date(x), *x).cmp(&(date(y), *y)));
        let parts: Vec<String> = us
            .iter()
            .map(|u| {
                let d = date(u);
                format!(
                    "[{}] {}",
                    if d.is_empty() { "undated" } else { d.as_str() },
                    squash(c.node_text(u))
                )
            })
            .collect();
        let t = if parts.is_empty() {
            let joke = c.joke(j);
            squash(
                joke.and_then(|x| s(x, "summary"))
                    .filter(|t| !t.is_empty())
                    .or_else(|| joke.and_then(|x| s(x, "title")))
                    .unwrap_or(""),
            )
        } else {
            parts.join(" / ")
        };
        let head: String = t.chars().take(n).collect();
        if t.chars().count() > n {
            format!("{head}...")
        } else {
            head
        }
    }

    /// `_dates(c, ix, j)`.
    fn dates(&self, j: &str) -> Vec<String> {
        let c = &*self.c;
        let mut ds: BTreeSet<String> = self.units[j]
            .iter()
            .filter_map(|u| {
                c.nodes
                    .get(u)
                    .and_then(|x| s(x, "date"))
                    .filter(|d| !d.is_empty())
                    .map(str::to_string)
            })
            .collect();
        ds.extend(c.perfs.get(j).into_iter().flatten().map(|p| p.date.clone()));
        if ds.is_empty()
            && let Some(n) = c.joke(j)
        {
            ds.extend(
                ["first_seen", "last_seen"]
                    .iter()
                    .filter_map(|k| s(n, k).filter(|d| !d.is_empty()).map(str::to_string)),
            );
        }
        let mut out: Vec<String> = ds.iter().map(|d| d.chars().take(10).collect()).collect();
        out.sort();
        out
    }

    fn docs(&self, j: &str) -> Vec<Value> {
        let c = &*self.c;
        let mut out: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        for u in &self.units[j] {
            let Some(n) = c.nodes.get(u) else { continue };
            let Some(doc) = n.get("doc_id").filter(|v| py::truthy(Some(v))).map(py::str) else {
                continue;
            };
            let date = n.get("date").cloned().unwrap_or(Value::Null);
            let d = out.entry(doc.clone()).or_insert_with(|| {
                order.push(doc.clone());
                json!({"doc_id": doc, "title": n.get("doc_title").cloned().unwrap_or(Value::Null), "date": date.clone()})
                    .as_object()
                    .cloned()
                    .unwrap_or_default()
            });
            if let Some(nd) = date.as_str().filter(|x| !x.is_empty()) {
                let cur = d.get("date").and_then(Value::as_str).unwrap_or("");
                if cur.is_empty() || nd < cur {
                    d.insert("date".into(), json!(nd));
                }
            }
        }
        let mut v: Vec<Map<String, Value>> = order.into_iter().filter_map(|k| out.remove(&k)).collect();
        v.sort_by(|a, b| {
            let k = |m: &Map<String, Value>| {
                (
                    m.get("date").and_then(Value::as_str).unwrap_or("").to_string(),
                    m["doc_id"].as_str().unwrap_or("").to_string(),
                )
            };
            k(a).cmp(&k(b))
        });
        v.into_iter().map(Value::Object).collect()
    }

    /// `member(c, ix, j)`.
    pub fn member(&self, j: &str) -> Value {
        let c = &*self.c;
        let ds = self.dates(j);
        let (st, src) = c.stage(j);
        json!({"joke_id": j, "title": c.title(j), "stage": st, "stage_source": src,
               "first_date": ds.first(), "last_date": ds.last(), "dates": ds,
               "nights": c.perfs.get(j).map_or(0, Vec::len), "units": self.units[j].len(), "docs": self.docs(j),
               "yours": c.his_nodes.contains(j)})
    }
}

fn units_pair(f: &Value) -> (Option<String>, Option<String>) {
    match f.get("units").and_then(Value::as_array) {
        Some(u) if u.len() == 2 => (u[0].as_str().map(str::to_string), u[1].as_str().map(str::to_string)),
        _ => (None, None),
    }
}

/// `pair_score(f)`: how likely two jokes are one bit before any judge, 0..1.
pub fn pair_score(f: &Value) -> f64 {
    let mut sc = 0.0;
    if let Some(band) = num(f.get("band")) {
        sc += (band * 3.0).clamp(0.0, 0.45).max(0.0);
    }
    let len = |k: &str| f.get(k).and_then(Value::as_array).map_or(0, Vec::len) as f64;
    sc += 0.08 * len("phrases").min(5.0);
    sc += 0.04 * len("rare").min(4.0);
    sc += 0.12 * len("title").min(2.0);
    if let Some(p) = f.get("premise").filter(|p| py::truthy(Some(p))) {
        sc += if p.get("source").and_then(Value::as_str) == Some("his") {
            0.6
        } else {
            0.5 * num(p.get("confidence")).filter(|x| *x != 0.0).unwrap_or(0.5)
        };
    }
    round_to(sc.min(1.0), 3)
}

/// `judge_candidates(c)`: candidate pairs worth asking about (score over the floor, and close in meaning or a
/// same-premise edge), best first.
pub fn judge_candidates(ix: &Index) -> Vec<Value> {
    let mut fs: Vec<Value> = ix
        .candidates()
        .iter()
        .map(|(a, b)| ix.features(a, b))
        .filter(|f| {
            pair_score(f) >= JUDGE_FLOOR
                && (num(f.get("cos")).unwrap_or(0.0) >= f64::from(UNIT_CAND) || py::truthy(f.get("premise")))
        })
        .collect();
    fs.sort_by(|x, y| {
        pair_score(y)
            .total_cmp(&pair_score(x))
            .then_with(|| x["a"].as_str().cmp(&y["a"].as_str()))
            .then_with(|| x["b"].as_str().cmp(&y["b"].as_str()))
    });
    fs
}

/// The judge's user turn for one batch of features.
pub fn judge_user(ix: &Index, batch: &[Value]) -> String {
    let lines: Vec<String> = batch
        .iter()
        .enumerate()
        .map(|(k, f)| {
            let (ua, ub) = units_pair(f);
            let (a, b) = (f["a"].as_str().unwrap_or(""), f["b"].as_str().unwrap_or(""));
            format!(
                "PAIR p{}\nA ({}): {}\nB ({}): {}",
                k + 1,
                ix.c.title(a),
                ix.excerpt(a, ua.as_deref(), 900),
                ix.c.title(b),
                ix.excerpt(b, ub.as_deref(), 900)
            )
        })
        .collect();
    format!("Decide each pair.\n\n{}", lines.join("\n\n"))
}

fn answer_pairs(data: Option<&Value>) -> HashMap<String, &Value> {
    data.and_then(|d| d.get("pairs"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|p| (p.get("id").map_or_else(|| "None".to_string(), py::str), p))
        .collect()
}

fn why_of(p: &Value) -> String {
    let w = p
        .get("why")
        .filter(|v| py::truthy(Some(v)))
        .map(py::str)
        .unwrap_or_default();
    w.chars().take(240).collect()
}

/// `judge` after the call: each answered pair into the cache under its key.
pub fn judge_finish(ix: &Index, batch: &[Value], data: Option<&Value>, model: &str, cache: &mut Map<String, Value>) {
    let got = answer_pairs(data);
    for (k, f) in batch.iter().enumerate() {
        let Some(p) = got.get(&format!("p{}", k + 1)) else {
            continue;
        };
        cache.insert(
            ix.judge_key(f),
            json!({"same": py::truthy(p.get("same")), "confidence": num(p.get("confidence")).unwrap_or(0.0),
                   "why": why_of(p), "model": model, "a": f["a"], "b": f["b"]}),
        );
    }
}

/// `second_opinion`'s queue: borderline "same" answers not looked at twice yet, in key order.
pub fn second_todo(c: &Corpus, cache: &Map<String, Value>) -> Vec<String> {
    let mut keys: Vec<&String> = cache.keys().collect();
    keys.sort();
    keys.into_iter()
        .filter(|k| {
            let v = &cache[*k];
            let conf = num(v.get("confidence")).unwrap_or(0.0);
            py::truthy(v.get("same"))
                && (SECOND_LO..SAME_MIN_CONF).contains(&conf)
                && v.get("second").is_none()
                && v.get("a").and_then(Value::as_str).is_some_and(|a| c.is_joke(a))
                && v.get("b").and_then(Value::as_str).is_some_and(|b| c.is_joke(b))
        })
        .cloned()
        .collect()
}

/// The second look's user turn for a batch of cache keys.
pub fn second_user(ix: &Index, cache: &Map<String, Value>, batch: &[String]) -> String {
    let lines: Vec<String> = batch
        .iter()
        .enumerate()
        .map(|(n, k)| {
            let v = &cache[k];
            let (a, b) = (v["a"].as_str().unwrap_or(""), v["b"].as_str().unwrap_or(""));
            format!(
                "PAIR p{}\nA ({}): {}\nB ({}): {}",
                n + 1,
                ix.c.title(a),
                ix.full(a, 2500),
                ix.c.title(b),
                ix.full(b, 2500)
            )
        })
        .collect();
    format!(
        "Decide each pair. Each side shows every written version of that joke, oldest first, separated by ' / '. Look \
         closely: would he keep these as ONE bit?\n\n{}",
        lines.join("\n\n")
    )
}

/// The second look's answers into the cache.
pub fn second_finish(batch: &[String], data: Option<&Value>, cache: &mut Map<String, Value>) {
    let got = answer_pairs(data);
    for (n, k) in batch.iter().enumerate() {
        if let Some(p) = got.get(&format!("p{}", n + 1))
            && let Some(v) = cache.get_mut(k).and_then(Value::as_object_mut)
        {
            v.insert(
                "second".into(),
                json!({"same": py::truthy(p.get("same")), "confidence": num(p.get("confidence")).unwrap_or(0.0),
                       "why": why_of(p)}),
            );
        }
    }
}

/// `verdict(v)`: the answer that counts: a confident first look, else a confident second look.
pub fn verdict(v: Option<&Value>) -> Option<Value> {
    let v = v.filter(|v| py::truthy(Some(v)))?;
    let conf = |x: &Value| num(x.get("confidence")).unwrap_or(0.0);
    if py::truthy(v.get("same")) && conf(v) >= SAME_MIN_CONF {
        return Some(v.clone());
    }
    let s2 = v.get("second").filter(|x| py::truthy(Some(x)))?;
    if py::truthy(s2.get("same")) && conf(s2) >= SAME_MIN_CONF {
        let mut out = v.as_object().cloned().unwrap_or_default();
        for (k, x) in s2.as_object().into_iter().flatten() {
            out.insert(k.clone(), x.clone());
        }
        return Some(Value::Object(out));
    }
    None
}

/// A merges-store view: dismissed pairs and live attaches (`dismissed_pairs`, `attached`).
#[derive(Debug, Default)]
pub struct MergeState {
    /// Pairs he dismissed, as sorted pairs.
    pub dismissed: HashSet<(String, String)>,
    /// New joke → its live attach.
    pub attached: HashMap<String, Value>,
    /// Jokes merged away into another.
    pub merged: HashSet<String>,
}

impl MergeState {
    /// From the merges store's items.
    pub fn from_items(items: &Map<String, Value>) -> Self {
        let mut st = Self::default();
        for m in items.values() {
            let kind = m.get("kind").and_then(Value::as_str);
            let status = m.get("status").and_then(Value::as_str);
            match (kind, status) {
                (Some("dismiss"), Some("dismissed")) => {
                    let js: Vec<&str> = m
                        .get("jokes")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect();
                    for (x, a) in js.iter().enumerate() {
                        for b in &js[x + 1..] {
                            st.dismissed.insert(pair(a, b));
                        }
                    }
                }
                (Some("attach"), Some("attached")) => {
                    if let Some(j) = m.get("joke").and_then(Value::as_str) {
                        st.attached.insert(j.to_string(), m.clone());
                    }
                }
                (Some("merge"), Some("merged")) => {
                    st.merged.extend(
                        m.get("others")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(str::to_string),
                    );
                }
                _ => {}
            }
        }
        st
    }
}

/// `sha8(obj)` over a list of ids: the first 8 hex of sha1 over its compact JSON.
pub fn sha8(keep: &[String]) -> String {
    let text = serde_json::to_string(keep).unwrap_or_default();
    hex::encode(Sha1::digest(text.as_bytes()))[..8].to_string()
}

fn main_key(m: &Value) -> (i64, i64, bool, Vec<i64>, i64, String) {
    let n = |k: &str| m.get(k).and_then(Value::as_i64).unwrap_or(0);
    let recent: Vec<i64> = m
        .get("last_date")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .map(|ch| -i64::from(u32::from(ch)))
        .collect();
    (
        -n("nights"),
        -stage_rank(m.get("stage").and_then(Value::as_str).unwrap_or("")),
        !py::truthy(m.get("yours")),
        recent,
        -n("units"),
        m["joke_id"].as_str().unwrap_or("").to_string(),
    )
}

fn main_why(m: &Value, others: &[Value]) -> String {
    let n = |x: &Value, k: &str| x.get(k).and_then(Value::as_i64).unwrap_or(0);
    let st = |x: &Value| stage_rank(x.get("stage").and_then(Value::as_str).unwrap_or(""));
    let last = |x: &Value| x.get("last_date").and_then(Value::as_str).unwrap_or("").to_string();
    let mut bits = Vec::new();
    let nights = n(m, "nights");
    if nights != 0 && nights >= others.iter().map(|o| n(o, "nights")).max().unwrap_or(0).max(0) {
        bits.push(format!(
            "performed on {nights} night{} (the most)",
            if nights == 1 { "" } else { "s" }
        ));
    }
    if st(m) > others.iter().map(st).max().unwrap_or(0).max(0) {
        bits.push(format!(
            "furthest along ({})",
            m.get("stage").map(py::str).unwrap_or_default()
        ));
    }
    if py::truthy(m.get("yours")) && !others.iter().any(|o| py::truthy(o.get("yours"))) {
        bits.push("you have edited it".to_string());
    }
    let ml = last(m);
    if !ml.is_empty() && others.iter().all(|o| last(o) <= ml) {
        bits.push(format!("the most recent version ({ml})"));
    }
    if bits.is_empty() {
        "the fullest version".to_string()
    } else {
        bits.join("; ")
    }
}

/// One accepted same-idea pair.
#[derive(Debug, Clone)]
struct Accepted {
    score: f64,
    feat: Value,
    verdict: Option<Value>,
    his: bool,
}

fn accepted_pairs(
    ix: &Index,
    judgements: &Map<String, Value>,
    st: &MergeState,
    exclude: &HashSet<String>,
) -> BTreeMap<(String, String), Accepted> {
    let mut out = BTreeMap::new();
    for (a, b) in ix.candidates() {
        if st.dismissed.contains(&(a.clone(), b.clone())) || exclude.contains(&a) || exclude.contains(&b) {
            continue;
        }
        let f = ix.features(&a, &b);
        let sc = pair_score(&f);
        let his = f.get("premise").and_then(|p| p.get("source")).and_then(Value::as_str) == Some("his");
        let v = if sc >= JUDGE_FLOOR {
            verdict(judgements.get(&ix.judge_key(&f)))
        } else {
            None
        };
        if his || v.is_some() {
            out.insert(
                (a, b),
                Accepted {
                    score: sc,
                    feat: f,
                    verdict: v,
                    his,
                },
            );
        }
    }
    out
}

fn reasons(ix: &Index, prs: &[(String, String, Accepted)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (a, b, p) in prs {
        if p.his {
            out.push(format!(
                "You linked {} and {} as the same premise.",
                ix.c.title(a),
                ix.c.title(b)
            ));
        }
        if let Some(w) = p
            .verdict
            .as_ref()
            .and_then(|v| v.get("why"))
            .filter(|v| py::truthy(Some(v)))
        {
            out.push(py::str(w));
        }
        let phr: Vec<String> = p.feat["phrases"]
            .as_array()
            .into_iter()
            .flatten()
            .take(3)
            .map(|x| format!("\"{}\"", py::str(x)))
            .collect();
        if !phr.is_empty() {
            out.push(format!("Same words in both: {}", phr.join(", ")));
        }
    }
    let mut seen = HashSet::new();
    out.retain(|x| seen.insert(x.clone()));
    out.truncate(5);
    out
}

/// `build_groups(c, judgements, dismissed, exclude=…)`: the same-idea groups, biggest and surest first.
pub fn build_groups(
    ix: &Index,
    judgements: &Map<String, Value>,
    st: &MergeState,
    exclude: &HashSet<String>,
) -> Vec<Value> {
    let max_size = 6;
    let acc = accepted_pairs(ix, judgements, st, exclude);
    let mut nbr: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (a, b) in acc.keys() {
        nbr.entry(a.clone()).or_default().insert(b.clone());
        nbr.entry(b.clone()).or_default().insert(a.clone());
    }
    let mut left: BTreeSet<String> = nbr.keys().cloned().collect();
    let mut groups = Vec::new();
    while let Some(start) = left.iter().next().cloned() {
        let mut comp: BTreeSet<String> = BTreeSet::from([start.clone()]);
        let mut todo = vec![start];
        while let Some(x) = todo.pop() {
            for y in nbr[&x].intersection(&left) {
                if comp.insert(y.clone()) {
                    todo.push(y.clone());
                }
            }
        }
        let ms: HashMap<&String, Value> = comp.iter().map(|j| (j, ix.member(j))).collect();
        let Some(main) = comp
            .iter()
            .min_by(|x, y| main_key(&ms[x]).cmp(&main_key(&ms[y])))
            .cloned()
        else {
            break;
        };
        let mut keep: Vec<String> = std::iter::once(main.clone())
            .chain(nbr[&main].intersection(&comp).cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        keep.sort_by(|x, y| main_key(&ms[x]).cmp(&main_key(&ms[y])));
        keep.truncate(max_size);
        if keep.len() < 2 {
            left.remove(&main);
            continue;
        }
        for k in &keep {
            left.remove(k);
        }
        let keep_set: HashSet<&String> = keep.iter().collect();
        let prs: Vec<(String, String, Accepted)> = acc
            .iter()
            .filter(|((a, b), _)| keep_set.contains(a) && keep_set.contains(b))
            .map(|((a, b), v)| (a.clone(), b.clone(), v.clone()))
            .collect();
        let mem: Vec<Value> = keep.iter().map(|j| ms[j].clone()).collect();
        let mut conf: Vec<f64> = prs
            .iter()
            .filter_map(|(_, _, p)| p.verdict.as_ref().and_then(|v| num(v.get("confidence"))))
            .collect();
        conf.extend(prs.iter().filter(|(_, _, p)| p.his && p.verdict.is_none()).map(|_| 1.0));
        let pairs: Vec<Value> = prs
            .iter()
            .map(|(a, b, p)| {
                let judge = p
                    .verdict
                    .as_ref()
                    .map(|v| json!({"same": v["same"], "confidence": v["confidence"], "why": v["why"]}));
                let phr: Vec<Value> = p.feat["phrases"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(5)
                    .cloned()
                    .collect();
                json!({"a": a, "b": b, "score": p.score, "judge": judge, "his": p.his, "phrases": phr})
            })
            .collect();
        groups.push(json!({
            "id": format!("grp:{}", sha8(&keep)), "main": main, "main_title": ix.c.title(&main),
            "main_why": main_why(&ms[&main], &mem[1..]), "members": mem, "reasons": reasons(ix, &prs),
            "confidence": if conf.is_empty() { 1.0 } else { round_to(conf.iter().copied().fold(f64::INFINITY, f64::min), 2) },
            "pairs": pairs,
            "source": if !prs.is_empty() && prs.iter().all(|(_, _, p)| p.his) { "his" } else { "inferred" }}));
    }
    groups.sort_by(|x, y| {
        let len = |g: &Value| g["members"].as_array().map_or(0, Vec::len);
        len(y)
            .cmp(&len(x))
            .then_with(|| {
                num(y.get("confidence"))
                    .unwrap_or(0.0)
                    .total_cmp(&num(x.get("confidence")).unwrap_or(0.0))
            })
            .then_with(|| x["id"].as_str().cmp(&y["id"].as_str()))
    });
    groups
}

/// `_jot_support(ix, ts, j)`: shared distinctive phrases between a jot and a bit, and how much of the jot's
/// content words its closest unit holds.
fn jot_support(ix: &Index, ts: &[String], j: &str) -> (Vec<String>, f64) {
    let (j3, j2, cont) = (g3(ts), g2(ts), content(ts));
    let mut phr: BTreeSet<String> = BTreeSet::new();
    let mut best = 0.0_f64;
    for u in &ix.parts[j] {
        for g in j3.intersection(&ix.ug3[u]) {
            if ix.df_3[g] <= 3 {
                phr.insert(g.join(" "));
            }
        }
        for g in j2.intersection(&ix.ug2[u]) {
            if ix.df_2[g] <= 3 {
                phr.insert(g.join(" "));
            }
        }
        if !cont.is_empty() {
            best = best.max(cont.intersection(&ix.ucont[u]).count() as f64 / cont.len() as f64);
        }
    }
    let keep = phr
        .iter()
        .filter(|p| !phr.iter().any(|q| q != *p && q.contains(p.as_str())))
        .cloned()
        .collect();
    (keep, round_to(best, 2))
}

/// `match_text(c, text, vec, exclude, group_of)`: where a new jot belongs (`attach`, `ambiguous` or `new`).
#[allow(clippy::too_many_lines)]
pub fn match_text(
    ix: &Index,
    text: &str,
    vec: Option<&[f32]>,
    exclude: &HashSet<String>,
    group_of: &HashMap<String, String>,
) -> Value {
    let c = &*ix.c;
    let ts = toks(text);
    let cont = content(&ts);
    let mut out = json!({"decision": "new", "joke_id": null, "title": null, "reasons": [], "top": []});
    if cont.len() < ATTACH_MIN_WORDS {
        out["reasons"] = json!([format!("too short to place ({} content words)", cont.len())]);
        return out;
    }
    let mut cands: Vec<Value> = Vec::new();
    let with_vec = vec.is_some() && !ix.vec_units.is_empty();
    if let (Some(v), true) = (vec, with_vec) {
        let sq: Vec<f32> = v.iter().map(|x| x * x).collect();
        let norm = pairwise_sum(&sq).sqrt().max(1e-9);
        let v: Vec<f32> = v.iter().map(|x| x / norm).collect();
        let dim = crate::corpus::DIM;
        let sims: Vec<f32> = (0..ix.vec_units.len())
            .map(|i| {
                ix.m[i * dim..(i + 1) * dim]
                    .iter()
                    .zip(&v)
                    .fold(0.0_f32, |a, (x, y)| a + x * y)
            })
            .collect();
        let jot_p99 = f64::from(reference(&sims));
        let mut order: Vec<usize> = (0..sims.len()).collect();
        order.sort_by(|a, b| sims[*b].total_cmp(&sims[*a]));
        let mut best: BTreeMap<String, (f64, f64, String)> = BTreeMap::new();
        let mut seen_order: Vec<String> = Vec::new();
        for i in order.into_iter().take(40) {
            let u = &ix.vec_units[i];
            let j = &ix.ujoke[u];
            if exclude.contains(j) || !c.is_joke(j) {
                continue;
            }
            let cs = f64::from(sims[i]);
            let band = (cs - f64::from(ix.p99[i])).min(cs - jot_p99);
            if best.get(j).is_none_or(|b| cs > b.0) {
                if !best.contains_key(j) {
                    seen_order.push(j.clone());
                }
                best.insert(j.clone(), (cs, band, u.clone()));
            }
        }
        for j in seen_order {
            let (cs, band, u) = best[&j].clone();
            let (phr, ct) = jot_support(ix, &ts, &j);
            cands.push(
                json!({"joke_id": j, "cos": round_to(cs, 3), "band": round_to(band, 3), "unit": u,
                              "phrases": phr, "contain": ct}),
            );
        }
    } else {
        for j in &ix.jokes {
            if exclude.contains(j) {
                continue;
            }
            let (phr, ct) = jot_support(ix, &ts, j);
            if !phr.is_empty() || ct >= 0.6 {
                cands.push(
                    json!({"joke_id": j, "cos": null, "band": null, "unit": null, "phrases": phr, "contain": ct}),
                );
            }
        }
    }
    let f = |x: &Value, k: &str| num(x.get(k)).unwrap_or(0.0);
    let plen = |x: &Value| x["phrases"].as_array().map_or(0, Vec::len);
    cands.sort_by(|x, y| {
        f(y, "cos")
            .total_cmp(&f(x, "cos"))
            .then_with(|| plen(y).cmp(&plen(x)))
            .then_with(|| f(y, "contain").total_cmp(&f(x, "contain")))
            .then_with(|| x["joke_id"].as_str().cmp(&y["joke_id"].as_str()))
    });
    out["top"] = Value::Array(
        cands
            .iter()
            .take(3)
            .map(|x| {
                let mut x = x.clone();
                x["title"] = json!(c.title(x["joke_id"].as_str().unwrap_or("")));
                x
            })
            .collect(),
    );
    let Some(top) = cands.first().cloned() else { return out };
    let words_ok = |x: &Value| plen(x) >= 2 || (f(x, "contain") >= 0.6 && cont.len() >= ATTACH_MIN_WORDS);
    let tj = top["joke_id"].as_str().unwrap_or("").to_string();
    let home = group_of.get(&tj).cloned().unwrap_or_else(|| tj.clone());
    let others: Vec<&Value> = cands[1..]
        .iter()
        .filter(|x| {
            let j = x["joke_id"].as_str().unwrap_or("");
            group_of.get(j).map_or(j, String::as_str) != home
        })
        .collect();
    let (close, rival) = if vec.is_some() {
        let close = num(top.get("band")).is_some_and(|b| b >= ATTACH_BAND);
        let rival = others
            .iter()
            .find(|x| f(x, "cos") > f(&top, "cos") - ATTACH_MARGIN && (plen(x) > 0 || f(x, "contain") >= 0.4))
            .copied();
        (close, rival)
    } else {
        (
            plen(&top) >= 3 && f(&top, "contain") >= 0.6,
            others.iter().find(|x| plen(x) >= 2).copied(),
        )
    };
    if close && words_ok(&top) && rival.is_none() {
        let jid = if c.is_joke(&home) && home != tj {
            home.clone()
        } else {
            tj.clone()
        };
        let title = c.title(&jid);
        let cos = num(top.get("cos"));
        let mut first = format!("reads like your bit \"{title}\"");
        if let Some(cs) = cos.filter(|x| *x != 0.0) {
            let _ = write!(first, " (meaning {cs:.2}, above both texts' usual range)");
        }
        let phr: Vec<String> = top["phrases"]
            .as_array()
            .into_iter()
            .flatten()
            .take(3)
            .map(|x| format!("\"{}\"", py::str(x)))
            .collect();
        let second = if phr.is_empty() {
            format!("{}% of its words are in that bit", (f(&top, "contain") * 100.0) as i64)
        } else {
            format!("same words: {}", phr.join(", "))
        };
        out["decision"] = json!("attach");
        out["joke_id"] = json!(jid);
        out["title"] = json!(title);
        out["score"] = top.get("cos").cloned().unwrap_or(Value::Null);
        out["reasons"] = json!([first, second]);
    } else if close || words_ok(&top) {
        let mut r = format!("close to \"{}\" but not clearly the same bit", c.title(&tj));
        if let Some(x) = rival {
            let _ = write!(
                r,
                "; \"{}\" is about as close",
                c.title(x["joke_id"].as_str().unwrap_or(""))
            );
        }
        r.push_str(", so it stays a new idea");
        out["decision"] = json!("ambiguous");
        out["reasons"] = json!([r]);
    }
    out
}
