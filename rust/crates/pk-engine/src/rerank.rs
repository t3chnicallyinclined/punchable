//! Reordering a read's candidates on this side of the store: plain BM25 over the pool (its own document
//! frequencies), with or without Snowball stemming, with the locator's words as a second field, the best window of a
//! long text, reciprocal rank fusion, and the store's two legs (full-text and vector) as orders of their own. Pure
//! functions over text.
//!
//! From upstream `upstream-ops/src/rerank.rs` (Rust branch 9dfedad), unchanged except: the pool carries each
//! candidate's rank in the store's full-text leg and vector leg instead of its project and rank inside the project
//! (Punchable reads one notebook, so upstream's `own_first` order and its two ladder arms are gone), and the
//! ladder gains the `text`, `vector` and `hybrid` arms.

use std::collections::{HashMap, HashSet};

/// BM25's usual constants; fixed, never tuned on a question set.
pub const K1: f64 = 1.2;
/// BM25's length normalisation.
pub const B: f64 = 0.75;
/// Reciprocal rank fusion's constant, the value its paper used (and `search::rrf`'s in `api-dev/search_api.py`).
pub const RRF_K: f64 = 60.0;
/// How many times a locator's words count beside the text (the field boost).
pub const PATH_WEIGHT: usize = 2;
/// Lines of text in a scoring window.
pub const WINDOW_LINES: usize = 12;

/// Words that carry no meaning of their own (upstream's card `STOP`): a card facet must share a word with its
/// quote beyond these, and a question's query words leave them out.
pub const FUNCTION_WORDS: [&str; 48] = [
    "a", "an", "the", "and", "or", "of", "to", "in", "on", "at", "by", "for", "with", "from", "is", "are", "was",
    "were", "be", "been", "it", "its", "this", "that", "these", "those", "as", "not", "no", "but", "if", "then", "so",
    "we", "our", "you", "they", "i", "do", "does", "did", "has", "have", "had", "can", "will", "would", "should",
];

/// Ask's `QSTOP` (`creative/askchat.py`, ported in `pk-creative::ask`, which reads it from here): words of the
/// question that say what kind of answer he wants, not what it is about.
pub const ASK_QSTOP: &[&str] = &[
    "written",
    "write",
    "wrote",
    "writing",
    "bit",
    "bits",
    "joke",
    "jokes",
    "material",
    "notebook",
    "killed",
    "kill",
    "kills",
    "killing",
    "worked",
    "work",
    "works",
    "died",
    "die",
    "bombed",
    "bomb",
    "best",
    "worst",
    "strongest",
    "weakest",
    "funniest",
    "laugh",
    "laughs",
    "laughed",
    "stage",
    "performed",
    "perform",
    "set",
    "sets",
    "show",
    "shows",
    "night",
    "nights",
    "ever",
    "which",
    "what",
    "where",
    "when",
    "who",
    "how",
    "why",
    "did",
    "does",
    "have",
    "has",
    "any",
    "all",
    "tell",
    "list",
    "find",
    "give",
    "those",
    "these",
    "them",
    "that",
    "this",
    "there",
    "their",
    "one",
    "ones",
    "said",
    "say",
    "talk",
    "talked",
    "mention",
    "mentioned",
    "stuff",
];

/// The words of a question that say what it is about: [`words`] (unstemmed) without [`FUNCTION_WORDS`] and
/// [`ASK_QSTOP`], once each, in order. Every word dropping out leaves the question as it was (a question made
/// only of such words still finds something). Added for Punchable: upstream sent every word, and a word in
/// most of his notebook ("the", "my") cost ~60 ms per full-text leg and drowned the content words (FORK.md).
pub fn question_words(question: &str) -> Vec<String> {
    let all = words(question, false);
    let mut seen = HashSet::new();
    let kept: Vec<String> = all
        .iter()
        .filter(|w| !FUNCTION_WORDS.contains(&w.as_str()) && !ASK_QSTOP.contains(&w.as_str()))
        .filter(|w| seen.insert(w.as_str()))
        .cloned()
        .collect();
    if kept.is_empty() { all } else { kept }
}

/// The words of `text`: runs of letters, digits and `_`, lowercased, Snowball-stemmed (English) when `stem` is set,
/// which is what the `punch_en` analyzer does (`db/schema.surql`: class tokenizer, lowercase, ascii,
/// snowball(english); the `ascii` folding is not repeated here).
pub fn words(text: &str, stem: bool) -> Vec<String> {
    let stemmer = stem.then(|| rust_stemmers::Stemmer::create(rust_stemmers::Algorithm::English));
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(|w| {
            let lower = w.to_lowercase();
            match &stemmer {
                // A word the stemmer leaves alone comes back borrowed: keep `lower` instead of copying it.
                Some(s) => match s.stem(&lower) {
                    std::borrow::Cow::Borrowed(_) => lower,
                    std::borrow::Cow::Owned(root) => root,
                },
                None => lower,
            }
        })
        .collect()
}

/// The words of a locator (`unit:abc` gives `unit`, `abc`; `src/ask.rs#L10-20` gives `src`, `ask`, `rs`).
pub fn path_words(locator: &str, stem: bool) -> Vec<String> {
    let path = locator.rsplit_once('#').map_or(locator, |(p, _)| p);
    words(&path.replace(['/', '\\', '.', '-', ':'], " "), stem)
}

/// BM25 of each document against the query, with document frequencies taken over these documents alone.
pub fn bm25(query: &[String], docs: &[Vec<String>]) -> Vec<f64> {
    bm25_with(query, docs, B)
}

/// The distinct query terms, in order.
fn terms(query: &[String]) -> Vec<&str> {
    let mut seen: HashSet<&str> = HashSet::with_capacity(query.len());
    query.iter().map(String::as_str).filter(|t| seen.insert(t)).collect()
}

/// Document frequency of every word over `docs`, and the inverse document frequency of a term. Linear in the words
/// (upstream checked each word against a `Vec` of the document's words so far: quadratic on a long set
/// transcript).
fn idf_over(docs: &[Vec<String>]) -> impl Fn(&str) -> f64 + '_ {
    let n = docs.len() as f64;
    let mut df: HashMap<&str, usize> = HashMap::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for d in docs {
        seen.clear();
        for w in d {
            if seen.insert(w.as_str()) {
                *df.entry(w).or_default() += 1;
            }
        }
    }
    move |t: &str| {
        let n_t = *df.get(t).unwrap_or(&0) as f64;
        ((n - n_t + 0.5) / (n_t + 0.5) + 1.0).ln()
    }
}

/// One bag's BM25 against the terms.
fn score_bag(
    terms: &[&str],
    idf: &dyn Fn(&str) -> f64,
    bag: &HashMap<&str, usize>,
    len: f64,
    avg: f64,
    k: (f64, f64),
) -> f64 {
    let (k1, b) = k;
    terms
        .iter()
        .filter_map(|t| {
            let tf = *bag.get(t)? as f64;
            let norm = tf + k1 * (1.0 - b + b * len / avg.max(1.0));
            Some(idf(t) * tf * (k1 + 1.0) / norm)
        })
        .sum()
}

fn bag_of(doc: &[String]) -> HashMap<&str, usize> {
    let mut bag: HashMap<&str, usize> = HashMap::new();
    for w in doc {
        *bag.entry(w.as_str()).or_default() += 1;
    }
    bag
}

/// BM25 with the length-normalisation constant `b` given (0: none; 1: full).
pub fn bm25_with(query: &[String], docs: &[Vec<String>], b: f64) -> Vec<f64> {
    bm25_params(query, docs, K1, b)
}

/// BM25 with both constants given.
pub fn bm25_params(query: &[String], docs: &[Vec<String>], k1: f64, b: f64) -> Vec<f64> {
    let n = docs.len();
    if n == 0 {
        return Vec::new();
    }
    let avg = docs.iter().map(Vec::len).sum::<usize>() as f64 / n as f64;
    let idf = idf_over(docs);
    let terms = terms(query);
    docs.iter()
        .map(|doc| score_bag(&terms, &idf, &bag_of(doc), doc.len() as f64, avg, (k1, b)))
        .collect()
}

/// The best window of each text: BM25 of every `WINDOW_LINES`-line window (stepping half a window), document
/// frequencies over the whole texts, the average length over the windows, a text scoring its best window's.
pub fn bm25_window(query: &[String], texts: &[&str]) -> Vec<f64> {
    bm25_window_with(query, texts, false, (K1, B), WINDOW_LINES)
}

/// [`bm25_window`] with the stemming, constants `(k1, b)` and window size given.
pub fn bm25_window_with(query: &[String], texts: &[&str], stem: bool, k: (f64, f64), lines_per: usize) -> Vec<f64> {
    let docs: Vec<Vec<String>> = texts.iter().map(|t| words(t, stem)).collect();
    if docs.is_empty() {
        return Vec::new();
    }
    let idf = idf_over(&docs);
    let terms = terms(query);
    let windows: Vec<Vec<Vec<String>>> = texts
        .iter()
        .map(|t| {
            let lines: Vec<&str> = t.lines().collect();
            let stride = (lines_per / 2).max(1);
            let starts: Vec<usize> = if lines.len() <= lines_per {
                vec![0]
            } else {
                (0..=lines.len() - lines_per).step_by(stride).collect()
            };
            starts
                .into_iter()
                .map(|s| words(&lines[s..(s + lines_per).min(lines.len())].join("\n"), stem))
                .collect()
        })
        .collect();
    let (sum, count) = windows
        .iter()
        .flatten()
        .fold((0usize, 0usize), |(s, c), w| (s + w.len(), c + 1));
    let avg = sum as f64 / count.max(1) as f64;
    windows
        .iter()
        .map(|ws| {
            ws.iter()
                .map(|w| score_bag(&terms, &idf, &bag_of(w), w.len() as f64, avg, k))
                .fold(0.0f64, f64::max)
        })
        .collect()
}

/// Indices in descending score order; equal scores keep their original order.
pub fn order_desc(scores: &[f64]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..scores.len()).collect();
    idx.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then(a.cmp(&b)));
    idx
}

/// Reciprocal rank fusion of several orders of the same indices: each index scores the sum over the orders of
/// 1 / (k + its rank), best first.
pub fn rrf(orders: &[Vec<usize>], k: f64) -> Vec<usize> {
    let n = orders.first().map_or(0, Vec::len);
    let mut score = vec![0.0f64; n];
    for order in orders {
        for (rank, &i) in order.iter().enumerate() {
            score[i] += 1.0 / (k + rank as f64 + 1.0);
        }
    }
    order_desc(&score)
}

/// Reciprocal rank fusion of score lists over the same indices, where an index counts only in the lists in which
/// it scored above zero, and an index that scored nowhere is left out (a link must not be proposed on nothing).
pub fn rrf_positive(scores: &[Vec<f64>], k: f64) -> Vec<usize> {
    let n = scores.first().map_or(0, Vec::len);
    let mut fused = vec![0.0f64; n];
    let mut in_any = vec![false; n];
    for list in scores {
        let ranked = order_desc(list).into_iter().filter(|&i| list[i] > 0.0);
        for (rank, i) in ranked.enumerate() {
            fused[i] += 1.0 / (k + rank as f64 + 1.0);
            in_any[i] = true;
        }
    }
    order_desc(&fused).into_iter().filter(|&i| in_any[i]).collect()
}

/// A read's candidates as the orders see them: text and locator, and each one's rank (1 = best) in the store's
/// full-text leg and its vector leg, `None` where that leg did not return it.
#[derive(Debug, Clone, Copy)]
pub struct Pool<'a> {
    /// What each candidate says (the text the local BM25 scores).
    pub texts: &'a [&'a str],
    /// Where each candidate is.
    pub locators: &'a [&'a str],
    /// Rank in the store's full-text read.
    pub text_rank: &'a [Option<usize>],
    /// Rank in the store's vector read.
    pub vector_rank: &'a [Option<usize>],
    /// Rank in the store's own fusion of its legs (`SurrealDB`'s `search::rrf`, as Search ranks today).
    pub fused_rank: &'a [Option<usize>],
}

/// The pool in one leg's order: the candidates that leg returned, by their rank, then the rest in the given order.
pub fn by_leg(ranks: &[Option<usize>]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..ranks.len()).collect();
    idx.sort_by_key(|&i| (ranks[i].is_none(), ranks[i].unwrap_or(usize::MAX), i));
    idx
}

/// The arms of the retrieval ladder, in the order they are reported. The first is the order given.
pub const ARMS: [&str; 12] = [
    "store",
    "bm25",
    "bm25-b1",
    "bm25-window",
    "bm25-stem",
    "bm25-fields",
    "rrf",
    "rrf-window",
    "text",
    "vector",
    "hybrid",
    "fused",
];

/// Every arm's order of the pool against `question`, as indices into it, keyed by arm name.
pub fn ladder_orders(question: &str, pool: &Pool<'_>) -> Vec<(&'static str, Vec<usize>)> {
    let (texts, locators) = (pool.texts, pool.locators);
    let plain_q = words(question, false);
    let stem_q = words(question, true);
    let plain_docs: Vec<Vec<String>> = texts.iter().map(|t| words(t, false)).collect();
    let stem_docs: Vec<Vec<String>> = texts.iter().map(|t| words(t, true)).collect();
    let field_docs: Vec<Vec<String>> = stem_docs
        .iter()
        .zip(locators)
        .map(|(d, l)| {
            let mut doc = d.clone();
            let path = path_words(l, true);
            for _ in 0..PATH_WEIGHT {
                doc.extend(path.iter().cloned());
            }
            doc
        })
        .collect();
    let store: Vec<usize> = (0..texts.len()).collect();
    let bm25_plain = order_desc(&bm25(&plain_q, &plain_docs));
    let bm25_b1 = order_desc(&bm25_with(&plain_q, &plain_docs, 1.0));
    let window = order_desc(&bm25_window(&plain_q, texts));
    let bm25_stem = order_desc(&bm25(&stem_q, &stem_docs));
    let bm25_fields = order_desc(&bm25(&stem_q, &field_docs));
    let fused = rrf(&[store.clone(), bm25_stem.clone()], RRF_K);
    let fused_window = rrf(&[bm25_plain.clone(), window.clone()], RRF_K);
    let text = by_leg(pool.text_rank);
    let vector = by_leg(pool.vector_rank);
    let hybrid = rrf(&[text.clone(), vector.clone(), bm25_stem.clone()], RRF_K);
    let fused_store = by_leg(pool.fused_rank);
    vec![
        ("store", store),
        ("bm25", bm25_plain),
        ("bm25-b1", bm25_b1),
        ("bm25-window", window),
        ("bm25-stem", bm25_stem),
        ("bm25-fields", bm25_fields),
        ("rrf", fused),
        ("rrf-window", fused_window),
        ("text", text),
        ("vector", vector),
        ("hybrid", hybrid),
        ("fused", fused_store),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_positive_leaves_out_what_scored_nowhere() {
        assert_eq!(rrf_positive(&[vec![2.0, 0.0, 0.0], vec![0.0, 0.0, 1.0]], RRF_K), [0, 2]);
        assert_eq!(rrf_positive(&[vec![1.0, 2.0, 0.0], vec![0.0, 3.0, 0.0]], RRF_K), [1, 0]);
        assert!(rrf_positive(&[vec![0.0, 0.0]], RRF_K).is_empty());
    }

    #[test]
    fn words_split_lowercase_and_stem_like_the_store() {
        assert_eq!(
            words("Rollback re-simulates Frames!", false),
            ["rollback", "re", "simulates", "frames"]
        );
        assert_eq!(
            words("Rollback re-simulates Frames!", true),
            ["rollback", "re", "simul", "frame"]
        );
        assert_eq!(path_words("src/net_sim.rs#L10-20", false), ["src", "net_sim", "rs"]);
        assert_eq!(path_words("unit:abc", false), ["unit", "abc"]);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn bm25_prefers_the_rarer_matching_word_and_order_is_stable() {
        let docs: Vec<Vec<String>> = ["the cat sat", "the dog sat", "the cat and the dog"]
            .iter()
            .map(|d| words(d, false))
            .collect();
        let q = words("dog", false);
        let scores = bm25(&q, &docs);
        assert_eq!(scores[0], 0.0);
        assert!(scores[1] > 0.0 && scores[2] > 0.0);
        assert!(scores[1] > scores[2], "the shorter document ranks first: {scores:?}");
        assert_eq!(order_desc(&[0.0, 0.0, 0.0]), [0, 1, 2]);
        assert_eq!(order_desc(&scores), [1, 2, 0]);
    }

    #[test]
    fn a_question_keeps_what_it_is_about() {
        assert_eq!(question_words("which night did the Parrot bit kill"), ["parrot"]);
        assert_eq!(question_words("my dog, my DOG and the lawyer"), ["my", "dog", "lawyer"]);
        assert_eq!(
            question_words("what is it"),
            ["what", "is", "it"],
            "all stop words: the question as it was"
        );
    }

    #[test]
    fn rrf_fuses_two_orders() {
        assert_eq!(rrf(&[vec![0, 2, 1], vec![2, 1, 0]], 60.0), [2, 0, 1]);
        assert_eq!(rrf(&[vec![0, 2, 1], vec![1, 2, 0]], 60.0)[0], 0);
    }

    #[test]
    fn a_leg_orders_what_it_returned_then_the_rest_as_given() {
        assert_eq!(by_leg(&[None, Some(2), None, Some(1)]), [3, 1, 0, 2]);
        assert_eq!(by_leg(&[None, None]), [0, 1]);
    }

    #[test]
    fn the_ladder_reports_every_arm_over_the_given_pool() {
        let texts = ["nothing here", "the parser rejects empty input", "parsers and inputs"];
        let locators = ["unit:a", "unit:b", "unit:c"];
        let pool = Pool {
            texts: &texts,
            locators: &locators,
            text_rank: &[None, Some(1), Some(2)],
            vector_rank: &[Some(1), Some(2), None],
            fused_rank: &[Some(2), Some(1), None],
        };
        let orders = ladder_orders("why does the parser reject empty input", &pool);
        assert_eq!(orders.len(), ARMS.len());
        for ((arm, order), name) in orders.iter().zip(ARMS) {
            assert_eq!(order.len(), 3, "{arm}");
            assert_eq!(*arm, name);
        }
        assert_eq!(orders[0].1, [0, 1, 2], "store keeps the given order");
        assert_eq!(orders[1].1[0], 1, "plain bm25 finds the exact words");
        assert_eq!(orders[4].1[0], 1, "stemmed too");
        assert_eq!(orders[8].1, [1, 2, 0], "the full-text leg's order");
        assert_eq!(orders[9].1, [0, 1, 2], "the vector leg's order");
        assert_eq!(orders[10].1[0], 1, "hybrid: the one every signal likes leads");
        assert_eq!(orders[11].1, [1, 0, 2], "the store's own fusion, as given");
    }

    #[test]
    fn the_best_window_beats_breadth() {
        let scattered = format!(
            "the save set\n{}\nrollback\n{}\nbytes restored\n{}\n277,288",
            "filler line\n".repeat(20),
            "filler line\n".repeat(20),
            "filler line\n".repeat(20)
        );
        let texts = [
            scattered.as_str(),
            "the save set is 277,288 bytes, every rollback restores it",
        ];
        let q = words("how many bytes is the save set a rollback restores", false);
        let window = bm25_window(&q, &texts);
        assert!(window[1] > window[0], "{window:?}");
        assert_eq!(bm25_window(&q, &[]), Vec::<f64>::new());
    }
}
