//! How one bit leads into the next (`creative/story.py`): `transition` (proven = done back to back on tape; link =
//! an edge, a shared persona or topic, or the same subject; jump = none), the subject index and `same_subject`,
//! the joke vectors, and the bridge halves with no model call (`bridge_links`, `bridge_user`, the finish).
//!
//! The joke vectors are the mean of its unit vectors, normalised, in float32 as numpy computes them; numpy's dot
//! products go through BLAS, whose summation order is the kernel's, so a cosine here equals the Python's to float32
//! precision, not to the bit (the goldens compare it that way).

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock, Mutex, OnceLock, PoisonError};

use pk_core::py;
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::corpus::{Corpus, num, prefix_words, round_to, s, suffix_words, venue_short};

/// `HUB_DF`: a topic on more jokes than this is a hub, not a link.
pub const HUB_DF: usize = 150;
/// `BRIDGE_SYSTEM`.
pub const BRIDGE_SYSTEM: &str =
    "You draft transitions for one comedian's own set. You never invent facts about his life beyond the links given.";
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z][a-z']{2,}").expect("a valid regex"));
static STOP: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    "the and that this with you your for are was but not what when they them then there their have has had \
     just like its it's i'm don't can't about from all one get got know out who how why would could should will into \
     been were did does doing very really because also more some any him her his she he our we us my me too than \
     only even ever every said says say going gonna want make made thing things people time good bad"
        .split_whitespace()
        .collect()
});

/// `BRIDGE_SCHEMA`.
pub static BRIDGE_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({"type": "object", "properties": {"bridge": {"type": "string"},
                                            "uses": {"type": "array", "items": {"type": "string"}}},
           "required": ["bridge", "uses"], "additionalProperties": false})
});

const LINK_KINDS: [&str; 6] = [
    "segue",
    "same_premise",
    "callback_to",
    "tag_of",
    "same_shape",
    "variant_of",
];

fn link_rank(k: &str) -> i64 {
    match k {
        "segue" => 0,
        "callback_to" => 1,
        "same_premise" => 2,
        "tag_of" => 3,
        "same_shape" => 4,
        "variant_of" => 5,
        _ => 9,
    }
}

/// Per joke, its content words; per word shared by enough jokes, its idf (`_subject_index`).
type SubjectIndex = (HashMap<String, HashSet<String>>, HashMap<String, f64>);

/// The caches the Python keeps on the corpus object (`_subject_idx`, `_cos_band`, `_joke_vec`), built on first use.
/// Each is a function of the corpus alone, so one cache may serve many [`Story`] values over the same corpus; it can
/// only be reached through a [`Story::new`] (its own) or a [`StoryCorpus`] (kept with its corpus), never paired with
/// another corpus.
#[derive(Debug, Default)]
struct StoryCache {
    subject: OnceLock<SubjectIndex>,
    vecs: OnceLock<HashMap<String, Option<Vec<f32>>>>,
    bands: Mutex<HashMap<String, (f64, f64)>>,
}

/// A corpus that owns its story caches: built and dropped together, so a [`Story`] over it keeps what the earlier
/// ones computed (the API keeps one per generation; the Python keeps the caches on its corpus object).
#[derive(Debug)]
pub struct StoryCorpus {
    corpus: Arc<Corpus>,
    cache: StoryCache,
    notebook: OnceLock<crate::notebook::Index>,
    ask: OnceLock<crate::ask::Notebook>,
}

impl StoryCorpus {
    /// Over a freshly built corpus, with empty caches.
    pub fn new(corpus: Corpus) -> Self {
        Self {
            corpus: Arc::new(corpus),
            cache: StoryCache::default(),
            notebook: OnceLock::new(),
            ask: OnceLock::new(),
        }
    }

    /// The corpus itself, shared: what an index that outlives a borrow (`merge::Index`) is built over.
    pub fn shared(&self) -> Arc<Corpus> {
        Arc::clone(&self.corpus)
    }

    /// The his-words index over this corpus (`notebook._index`, kept on the corpus object), built on first use.
    pub fn notebook_index(&self) -> &crate::notebook::Index {
        self.notebook.get_or_init(|| crate::notebook::Index::new(&self.corpus))
    }

    /// His notebook flattened for Ask (`askchat.sources`, kept on the corpus object), built on first use.
    pub fn ask_notebook(&self) -> &crate::ask::Notebook {
        self.ask.get_or_init(|| crate::ask::Notebook::from_corpus(&self.corpus))
    }

    /// The story layer over this corpus, with its caches.
    pub fn story(&self) -> Story<'_> {
        Story {
            c: &self.corpus,
            cache: Cache::Shared(&self.cache),
        }
    }
}

impl std::ops::Deref for StoryCorpus {
    type Target = Corpus;

    fn deref(&self) -> &Corpus {
        &self.corpus
    }
}

/// The cache a [`Story`] reads: its own, or one kept beside the corpus.
#[derive(Debug)]
enum Cache<'c> {
    Own(Box<StoryCache>),
    Shared(&'c StoryCache),
}

impl std::ops::Deref for Cache<'_> {
    type Target = StoryCache;

    fn deref(&self) -> &StoryCache {
        match self {
            Cache::Own(c) => c,
            Cache::Shared(c) => c,
        }
    }
}

/// The story layer over one corpus, with the caches the Python keeps on the corpus object ([`StoryCache`]).
#[derive(Debug)]
pub struct Story<'c> {
    c: &'c Corpus,
    cache: Cache<'c>,
}

impl<'c> Story<'c> {
    /// Over a corpus, with caches of its own (dropped with it).
    pub fn new(c: &'c Corpus) -> Self {
        Self {
            c,
            cache: Cache::Own(Box::default()),
        }
    }

    /// `_direct_edges(c, a, b)`: the link edges between two bits (a unit stands for its joke), best first.
    #[allow(clippy::many_single_char_names)] // the Python's names
    pub fn direct_edges(&self, a: &str, b: &str) -> Vec<&'c Map<String, Value>> {
        let c = self.c;
        let mut out: Vec<&Map<String, Value>> = Vec::new();
        for &i in c.edges_by_node.get(a).map_or(&[][..], Vec::as_slice) {
            let e = &c.edges[i];
            if !s(e, "kind").is_some_and(|k| LINK_KINDS.contains(&k)) {
                continue;
            }
            let side = |k: &str| -> String {
                let x = s(e, k).unwrap_or("");
                if x.starts_with("unit:") {
                    c.unit_joke.get(x).cloned().unwrap_or_else(|| x.to_string())
                } else {
                    x.to_string()
                }
            };
            let (sx, dx) = (side("src"), side("dst"));
            let ends: HashSet<&str> = [sx.as_str(), dx.as_str()].into_iter().collect();
            let want: HashSet<&str> = [a, b].into_iter().collect();
            if ends == want {
                out.push(e);
            }
        }
        out.sort_by(|x, y| {
            let key = |e: &Map<String, Value>| {
                (
                    link_rank(s(e, "kind").unwrap_or("")),
                    -num(e.get("confidence")).unwrap_or(0.0),
                )
            };
            let (kx, ky) = (key(x), key(y));
            kx.0.cmp(&ky.0)
                .then(kx.1.total_cmp(&ky.1))
                .then_with(|| s(x, "id").unwrap_or("").cmp(s(y, "id").unwrap_or("")))
        });
        out
    }

    /// `shared_labels(c, a, b, kinds, min_w)`: labels both hold, rarest first.
    pub fn shared_labels(&self, a: &str, b: &str, kinds: &[&str], min_w: f64) -> Vec<String> {
        let la = self.c.labels_of(a, Some(kinds), min_w);
        let lb = self.c.labels_of(b, Some(kinds), min_w);
        let mut out: Vec<String> = la.keys().filter(|l| lb.contains_key(*l)).cloned().collect();
        let idf = |l: &str| self.c.idf.get(l).copied().unwrap_or(1.0);
        out.sort_by(|x, y| idf(y).total_cmp(&idf(x)).then_with(|| x.cmp(y)));
        out
    }

    fn subject_index(&self) -> &SubjectIndex {
        self.cache.subject.get_or_init(|| {
            let c = self.c;
            let mut words: HashMap<String, HashSet<String>> = HashMap::new();
            for j in &c.jokes {
                let n = &c.nodes[j];
                let lead = c.lead_unit(j);
                let title = s(n, "title").unwrap_or("");
                let txt = [
                    title,
                    title,
                    s(n, "summary").unwrap_or(""),
                    lead.as_deref().map_or("", |u| c.node_text(u)),
                ]
                .join(" ")
                .to_lowercase();
                let ws = WORD
                    .find_iter(&txt)
                    .map(|m| m.as_str())
                    .filter(|w| !STOP.contains(w))
                    .map(str::to_string);
                words.insert(j.clone(), ws.collect());
            }
            let mut df: HashMap<&str, usize> = HashMap::new();
            for ws in words.values() {
                for w in ws {
                    *df.entry(w.as_str()).or_default() += 1;
                }
            }
            let nj = words.len().max(1);
            let hi = 3.max(nj / 50);
            let idf: HashMap<String, f64> = df
                .into_iter()
                .filter(|(_, d)| (2..=hi).contains(d))
                .map(|(w, d)| (w.to_string(), (nj as f64 / (1.0 + d as f64)).ln()))
                .collect();
            (words, idf)
        })
    }

    /// `joke_vec(jid)`: the mean of its unit vectors, normalised (float32); `None` without vectors.
    pub fn joke_vec(&self, jid: &str) -> Option<&[f32]> {
        let all = self.cache.vecs.get_or_init(|| {
            let c = self.c;
            c.jokes
                .iter()
                .map(|j| {
                    let vs: Vec<&[f32]> = c
                        .joke_units
                        .get(j)
                        .map(|us| us.iter().filter_map(|u| c.vectors.get(u)).collect())
                        .unwrap_or_default();
                    (j.clone(), mean_unit(&vs))
                })
                .collect()
        });
        all.get(jid).and_then(Option::as_deref)
    }

    /// `cos(a, b)`: the joke vectors' dot product.
    pub fn cos(&self, a: &str, b: &str) -> Option<f64> {
        let (va, vb) = (self.joke_vec(a)?, self.joke_vec(b)?);
        Some(f64::from(va.iter().zip(vb).fold(0.0_f32, |acc, (x, y)| acc + x * y)))
    }

    /// `_cos_band(j)`: the 97th and 99.7th percentile of the joke's cosines to every other (1, 1 under 50).
    pub fn cos_band(&self, j: &str) -> (f64, f64) {
        if let Some(b) = self.cache.bands.lock().unwrap_or_else(PoisonError::into_inner).get(j) {
            return *b;
        }
        // n dot products for the one bit asked about (the Python caches per bit, too)
        let mut cs: Vec<f64> = self
            .c
            .jokes
            .iter()
            .filter(|o| *o != j)
            .filter_map(|o| self.cos(j, o))
            .collect();
        let band = if cs.len() < 50 {
            (1.0, 1.0)
        } else {
            cs.sort_by(f64::total_cmp);
            let at = |q: f64| cs[(q * (cs.len() - 1) as f64) as usize];
            (at(0.97), at(0.997))
        };
        self.cache
            .bands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(j.to_string(), band);
        band
    }

    /// `same_subject(c, ids, o)`: how strongly `o` shares the subject of `ids` (rare shared words, or a cosine above
    /// the bit's own band), the shared words, the closest of `ids`.
    pub fn same_subject(&self, ids: &[&str], o: &str) -> (f64, Vec<String>, Option<String>) {
        let (words, idf) = self.subject_index();
        let empty = HashSet::new();
        let ow = words.get(o).unwrap_or(&empty);
        let (mut best, mut best_words, mut best_j) = (0.0_f64, Vec::new(), None);
        for &j in ids {
            let jw = words.get(j).unwrap_or(&empty);
            let mut shared: Vec<&String> = ow.iter().filter(|w| jw.contains(*w) && idf.contains_key(*w)).collect();
            shared.sort_by(|x, y| idf[*y].total_cmp(&idf[*x]).then_with(|| x.cmp(y)));
            let lex = if shared.len() >= 2 {
                (shared.iter().take(6).fold(0.0, |acc, w| acc + idf[*w]) / 12.0).min(1.0)
            } else {
                0.0
            };
            let (lo, hi) = self.cos_band(j);
            let sem = match self.cos(j, o) {
                Some(cs) if hi > lo && cs > lo => (cs - lo) / (hi - lo),
                _ => 0.0,
            };
            let sc = lex.max(sem);
            if sc > best {
                best = sc.min(1.0);
                best_words = shared.iter().take(3).map(|w| (*w).clone()).collect();
                best_j = Some(j.to_string());
            }
        }
        (best, best_words, best_j)
    }

    fn label_name(&self, l: &str) -> String {
        py::str(&self.c.label_field(l, "name"))
    }

    fn label_evidence(&self, l: &str, joke: &str) -> Vec<Value> {
        self.c
            .joke_labels
            .get(joke)
            .and_then(|ls| ls.get(l))
            .map(|(_, e)| self.c.edge_evidence(&self.c.edges[*e], 1))
            .unwrap_or_default()
    }

    /// `transition(c, a, b)`.
    #[allow(clippy::too_many_lines)]
    pub fn transition(&self, a: &str, b: &str) -> Value {
        let c = self.c;
        let key = (a.to_string(), b.to_string());
        let rows = c.back_to_back.get(&key).map_or(&[][..], Vec::as_slice);
        let edges = self.direct_edges(a, b);
        let personas = self.shared_labels(a, b, &["persona"], 0.5);
        let topics = self.shared_labels(a, b, &["topic"], 0.4);
        let strong: Vec<&String> = topics
            .iter()
            .filter(|l| c.label_df.get(*l).copied().unwrap_or(0) <= HUB_DF)
            .collect();
        let mut via: Vec<Value> = Vec::new();
        let mut evidence: Vec<Value> = Vec::new();
        for e in edges.iter().take(3) {
            let kind = s(e, "kind").unwrap_or("");
            via.push(json!({"kind": kind, "edge": e.get("id"), "source": c.source_of(e),
                            "why": e.get("reason").cloned().unwrap_or(Value::Null),
                            "reverse": kind == "segue" && s(e, "src") != Some(a)}));
            evidence.extend(c.edge_evidence(e, 1));
        }
        for l in &personas {
            via.push(json!({"kind": "persona", "label_id": l, "name": c.label_field(l, "name"), "source": "proposed"}));
        }
        let (label, why);
        if !rows.is_empty() {
            label = "proven";
            let rooms: Vec<String> = rows
                .iter()
                .map(|r| venue_short(r.venue.as_str(), Some(&r.date)))
                .collect();
            why = format!("Done back-to-back {}x ({})", rows.len(), rooms.join(", "));
            let mut ev: Vec<Value> = rows[rows.len().saturating_sub(2)..]
                .iter()
                .map(|r| c.seg_evidence(&r.b_seg).unwrap_or(Value::Null))
                .collect();
            ev.append(&mut evidence);
            evidence = ev;
        } else if let Some(e) = edges.first() {
            label = "link";
            let mut kind = s(e, "kind").unwrap_or("").replace('_', " ");
            if s(e, "kind") == Some("segue") && s(e, "src") != Some(a) {
                kind = "segue (you usually do it the other way round)".to_string();
            }
            let reason = e
                .get("reason")
                .filter(|v| py::truthy(Some(v)))
                .map(py::str)
                .unwrap_or_default();
            why = py::strip(&format!("{kind}: {reason}")).to_string();
        } else if !personas.is_empty() || !strong.is_empty() {
            label = "link";
            if personas.is_empty() {
                let names: Vec<String> = strong.iter().take(2).map(|l| self.label_name(l)).collect();
                why = format!("topic link: {} (needs a bridge line)", names.join(", "));
                for l in strong.iter().take(2) {
                    via.push(json!({"kind": "topic", "label_id": l, "name": c.label_field(l, "name"),
                                    "source": "proposed"}));
                }
            } else {
                let names: Vec<String> = personas.iter().map(|l| self.label_name(l)).collect();
                why = format!("persona link: {} (needs a bridge line)", names.join(", "));
            }
            let first = personas.first().or(strong.first().copied());
            if let Some(l) = first {
                for j in [a, b] {
                    evidence.extend(self.label_evidence(l, j));
                }
            }
        } else {
            let (sc, ws, _) = self.same_subject(&[a], b);
            if sc >= 0.35 {
                label = "link";
                why = format!(
                    "same subject{} (needs a bridge line)",
                    if ws.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", ws.join(", "))
                    }
                );
                via.push(json!({"kind": "subject", "score": round_to(sc, 2), "words": ws, "source": "inferred"}));
            } else {
                label = "jump";
                why = if topics.is_empty() {
                    "topic jump · no link yet".to_string()
                } else {
                    let names: Vec<String> = topics.iter().take(2).map(|l| self.label_name(l)).collect();
                    format!("topic jump (only a weak shared topic: {})", names.join(", "))
                };
            }
        }
        let rev = c.back_to_back.get(&(b.to_string(), a.to_string())).map_or(0, Vec::len);
        let shared_topics: Vec<Value> = topics.iter().take(3).map(|l| c.label_field(l, "name")).collect();
        let nights: Vec<Value> = rows
            .iter()
            .map(|r| json!({"set": r.set, "date": r.date, "venue": r.venue}))
            .collect();
        let evidence: Vec<Value> = evidence.into_iter().filter(|x| py::truthy(Some(x))).take(3).collect();
        json!({"from": a, "to": b, "label": label, "why": why.chars().take(240).collect::<String>(), "via": via,
               "needs_bridge": label == "jump" || (label == "link" && edges.is_empty()),
               "shared_topics": shared_topics, "nights": nights, "reverse_order_nights": rev, "evidence": evidence})
    }

    /// `_edge_text(c, jid, end)`: the last (or first) thirty words of the bit, from his latest night or its lead.
    pub fn edge_text(&self, jid: &str, end: bool) -> String {
        let c = self.c;
        let t = match c.perfs.get(jid).and_then(|ps| ps.last()) {
            Some(p) => {
                let sid = if end { p.seg_ids.last() } else { p.seg_ids.first() };
                sid.and_then(|s| c.text_of(s, None)).unwrap_or_default()
            }
            None => c
                .lead_unit(jid)
                .map(|u| c.node_text(&u).to_string())
                .unwrap_or_default(),
        };
        if end {
            suffix_words(&t, 30)
        } else {
            prefix_words(&t, 30)
        }
    }

    /// The links a bridge may stand on (`bridge_draft`): shared personas then topics (each with its evidence),
    /// then up to two direct edges.
    pub fn bridge_links(&self, a: &str, b: &str) -> Vec<Value> {
        let c = self.c;
        let mut links: Vec<Value> = Vec::new();
        let labels = [
            self.shared_labels(a, b, &["persona"], 0.4),
            self.shared_labels(a, b, &["topic"], 0.4),
        ]
        .concat();
        for l in labels {
            let mut ev: Vec<Value> = Vec::new();
            for j in [a, b] {
                ev.extend(self.label_evidence(&l, j));
            }
            ev.truncate(2);
            links.push(
                json!({"id": format!("L{}", links.len() + 1), "kind": c.label_field(&l, "kind"),
                              "label_id": l, "name": c.label_field(&l, "name"), "evidence": ev}),
            );
        }
        for e in self.direct_edges(a, b).into_iter().take(2) {
            links.push(
                json!({"id": format!("L{}", links.len() + 1), "kind": e.get("kind"), "edge_id": e.get("id"),
                              "name": e.get("reason").cloned().unwrap_or(Value::Null),
                              "evidence": c.edge_evidence(e, 2)}),
            );
        }
        links
    }

    /// The bridge's user turn. `he_could_say` is "Pat could say" when he is named, else "he could say".
    #[allow(clippy::similar_names)]
    pub fn bridge_user(&self, he_could_say: &str, a: &str, b: &str, links: &[Value]) -> String {
        let c = self.c;
        let mut examples: Vec<String> = Vec::new();
        for e in &c.edges {
            if s(e, "kind") == Some("segue") && c.source_of(e) != "proposed" {
                let ev = c.edge_evidence(e, 2);
                if ev.len() == 2 {
                    let q = |i: usize| ev[i].get("quote").map(py::str).unwrap_or_default();
                    examples.push(format!("- '{}' -> '{}'", q(0), q(1)));
                }
            }
            if examples.len() >= 3 {
                break;
            }
        }
        let mut msg = format!(
            "Write ONE short spoken bridge line (max 30 words) that {he_could_say} between two of his bits.\n\
             End of '{}': \"{}\"\nStart of '{}': \"{}\"\n\
             Ground it ONLY in these links between the two bits (cite their ids in `uses`):\n",
            c.title(a),
            self.edge_text(a, true),
            c.title(b),
            self.edge_text(b, false),
        );
        let lines: Vec<String> = links
            .iter()
            .map(|x| {
                let f = |k: &str| x.get(k).map_or_else(|| "None".to_string(), py::str);
                format!("{}: {} = {}", f("id"), f("kind"), f("name"))
            })
            .collect();
        msg.push_str(&lines.join("\n"));
        if !examples.is_empty() {
            let _ = write!(
                msg,
                "\nHow his real transitions on tape sound:\n{}",
                examples.join("\n")
            );
        }
        msg.push_str(
            "\nFirst person, his conversational voice (the way his bridges sound in the profile, when it says), no \
             new premise, no punchline of its own.",
        );
        msg
    }
}

/// The links an answer used (`uses` filtered to known ids, else the first link) and the bridge text.
pub fn bridge_finish(data: Option<&Value>, links: &[Value]) -> (String, Vec<Value>) {
    let known: HashSet<&str> = links
        .iter()
        .filter_map(|x| x.get("id").and_then(Value::as_str))
        .collect();
    let mut uses: Vec<&str> = data
        .and_then(|d| d.get("uses"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|u| known.contains(u))
        .collect();
    if uses.is_empty() {
        uses = links
            .first()
            .and_then(|x| x.get("id"))
            .and_then(Value::as_str)
            .into_iter()
            .collect();
    }
    let text = py::strip(data.and_then(|d| d.get("bridge")).and_then(Value::as_str).unwrap_or("")).to_string();
    let grounded = links
        .iter()
        .filter(|x| x.get("id").and_then(Value::as_str).is_some_and(|i| uses.contains(&i)))
        .cloned()
        .collect();
    (text, grounded)
}

/// numpy's `mean(stack(vs), axis=0)` then `v / norm(v)` in float32; `None` without vectors or with a zero norm.
fn mean_unit(vs: &[&[f32]]) -> Option<Vec<f32>> {
    let first = vs.first()?;
    let mut sum: Vec<f32> = first.to_vec();
    for v in &vs[1..] {
        for (s, x) in sum.iter_mut().zip(*v) {
            *s += x;
        }
    }
    let k = vs.len() as f32;
    let mean: Vec<f32> = sum.into_iter().map(|x| x / k).collect();
    let norm = mean.iter().fold(0.0_f32, |acc, x| acc + x * x).sqrt();
    if norm == 0.0 {
        return None;
    }
    Some(mean.into_iter().map(|x| x / norm).collect())
}
