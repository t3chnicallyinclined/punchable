//! The differential against the Python it replaces: `rust/tools/gen_embed_goldens.py` runs a synthetic corpus
//! through `api-dev/search_api.py HashEmbedder` and `engine/embed.py` (the way `db/embedder.py` calls it) and writes
//! what it saw; these tests run the same corpus through pk-embed. The hash vectors must be equal; the model's token
//! ids must be equal; its vectors must agree at cosine >= 0.99999 (a different onnxruntime build rounds differently)
//! and be unit length; the `fetch_bge.py` fingerprint must hold. The model tests skip, saying so, when `PK_BGE_DIR`
//! (or `PK_CACHE_DIR`) does not point at the model, so a CI without it still runs the pure tests.

use std::path::PathBuf;

use pk_embed::{Bge, DIM, Embedder, FINGERPRINT, Hash, MAX_TOKENS, MODEL_NAME, bge_dir, prepare};
use serde::Deserialize;

#[derive(Deserialize)]
struct HashGolden {
    items: Vec<HashItem>,
}

#[derive(Deserialize)]
struct HashItem {
    text: String,
    vec: Vec<f64>,
}

#[derive(Deserialize)]
struct BgeGolden {
    model: String,
    items: Vec<BgeItem>,
}

#[derive(Deserialize)]
struct BgeItem {
    text: String,
    query: bool,
    ids: Vec<u32>,
    vec: Vec<f32>,
}

fn golden<T: for<'a> Deserialize<'a>>(name: &str) -> T {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(name);
    let text = std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("{}: {e} (run python rust/tools/gen_embed_goldens.py)", p.display()));
    serde_json::from_str(&text).expect("golden is JSON")
}

/// The model, or `None` with a line on stderr saying why the test did nothing. A skipped test is a failed test where
/// the model is promised: with `PK_REQUIRE_EXTERNAL` set (CI), a missing model panics instead (docs/ARCHITECTURE.md
/// section 5).
fn model() -> Option<Bge> {
    let Some(dir) = bge_dir() else {
        let why =
            "PK_BGE_DIR unset or without the model (python tools/kb/fetch_bge.py --dest DIR, then PK_BGE_DIR=DIR)";
        assert!(
            std::env::var_os("PK_REQUIRE_EXTERNAL").is_none_or(|v| v.is_empty()),
            "PK_REQUIRE_EXTERNAL is set and the model tests cannot run: {why}"
        );
        eprintln!("pk-embed: {why}; skipping");
        return None;
    };
    Some(Bge::load(&dir).expect("the model loads"))
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| f64::from(*x) * f64::from(*y)).sum();
    let na: f64 = a.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    dot / (na * nb)
}

/// Splits the corpus the way the API calls the embedder: passages in one call, queries in another.
fn embed_corpus(bge: &Bge, items: &[BgeItem]) -> Vec<Vec<f32>> {
    let passages: Vec<&str> = items.iter().filter(|i| !i.query).map(|i| i.text.as_str()).collect();
    let queries: Vec<&str> = items.iter().filter(|i| i.query).map(|i| i.text.as_str()).collect();
    let mut pv = bge.embed(&passages, false).expect("passages embed").into_iter();
    let mut qv = bge.embed(&queries, true).expect("queries embed").into_iter();
    items
        .iter()
        .map(|i| if i.query { qv.next() } else { pv.next() }.expect("one vector per text"))
        .collect()
}

#[test]
fn hash_vectors_equal_python() {
    let g: HashGolden = golden("hash.json");
    let texts: Vec<&str> = g.items.iter().map(|i| i.text.as_str()).collect();
    let got = Hash.embed(&texts, false).expect("hash never fails");
    assert_eq!(got.len(), g.items.len());
    for (item, v) in g.items.iter().zip(&got) {
        assert_eq!(v.len(), DIM);
        let want: Vec<f32> = item.vec.iter().map(|x| *x as f32).collect();
        assert_eq!(v, &want, "hash vector differs from Python for {:?}", item.text);
    }
    assert_eq!(Hash.name(), "hash");
}

#[test]
fn bge_token_ids_equal_python() {
    let Some(bge) = model() else { return };
    let g: BgeGolden = golden("bge.json");
    assert_eq!(g.model, MODEL_NAME);
    assert_eq!(bge.name(), MODEL_NAME);
    for item in &g.items {
        let ids = bge.token_ids(&prepare(&item.text, item.query)).expect("tokenizes");
        assert!(ids.len() <= MAX_TOKENS);
        assert_eq!(
            ids, item.ids,
            "token ids differ from Python for {:?} (query={})",
            item.text, item.query
        );
    }
}

#[test]
fn bge_vectors_match_python_at_the_cosine_floor() {
    let Some(bge) = model() else { return };
    let g: BgeGolden = golden("bge.json");
    let got = embed_corpus(&bge, &g.items);
    let mut floor = 1.0f64;
    for (item, v) in g.items.iter().zip(&got) {
        assert_eq!(v.len(), DIM);
        let c = cosine(v, &item.vec);
        assert!(
            c >= 0.99999,
            "cosine {c} < 0.99999 to Python for {:?} (query={})",
            item.text,
            item.query
        );
        floor = floor.min(c);
    }
    eprintln!(
        "pk-embed: cosine floor over {} texts = {floor:.10} (1 - floor = {:.3e})",
        g.items.len(),
        1.0 - floor
    );
}

#[test]
fn bge_vectors_are_unit_length_and_queries_differ_from_passages() {
    let Some(bge) = model() else { return };
    let g: BgeGolden = golden("bge.json");
    for v in embed_corpus(&bge, &g.items) {
        let n = v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
        assert!((n - 1.0).abs() < 1e-5, "norm {n}");
    }
    let p = bge.embed(&["parking meter pronouns"], false).expect("passage");
    let q = bge.embed(&["parking meter pronouns"], true).expect("query");
    assert!(cosine(&p[0], &q[0]) < 0.9999, "the prefix changes the vector");
}

#[test]
fn bge_fingerprint_matches_fetch_bge() {
    let Some(bge) = model() else { return };
    let (ok, got) = bge.fingerprint().expect("embeds");
    assert!(
        ok,
        "fingerprint {got:?}, expected {FINGERPRINT:?}: not the model the cached vectors were made with"
    );
}

#[test]
fn bge_edge_cases() {
    let Some(bge) = model() else { return };
    assert!(
        bge.embed(&[], false).expect("empty in").is_empty(),
        "no texts, no vectors, no model run"
    );
    let v = bge.embed(&["", " "], false).expect("empties");
    assert_eq!(v[0], v[1], "an empty text embeds as a lone space");
    // 2,000 characters of "a " is 1,000 word pieces: past the tokenizer's 512 but inside MAX_CHARS
    let long = "a ".repeat(1500);
    let ids = bge.token_ids(&prepare(&long, false)).expect("tokenizes");
    assert_eq!(ids.len(), MAX_TOKENS, "a long text truncates to 512 tokens");
    let one = bge.embed(&[&long], false).expect("one long");
    let cut: String = long.chars().take(pk_embed::MAX_CHARS).collect();
    let same = bge.embed(&[&cut], false).expect("cut");
    assert!(
        cosine(&one[0], &same[0]) > 0.999_999,
        "past MAX_CHARS the text is cut before tokenising"
    );
    assert!(
        matches!(
            Bge::load(std::path::Path::new("nowhere")),
            Err(pk_embed::EmbedError::NoModel(_))
        ),
        "a folder without the model is refused in plain words"
    );
}
