//! The hash embedder: deterministic 384-d vectors from the words, so texts that share words are close. For tests
//! and a PC without the model, never the server. Equal to `api-dev/search_api.py HashEmbedder` value for value.

use std::sync::LazyLock;

use regex::Regex;
use sha1::{Digest, Sha1};

use crate::{DIM, EmbedError, Embedder};

/// The word-hash embedder (`PK_EMBEDDER=hash` in the Python API).
#[derive(Debug, Default, Clone, Copy)]
pub struct Hash;

/// Python's `re.findall(r"[a-z0-9']+", text.lower())`.
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new("[a-z0-9']+").expect("a literal pattern"));

impl Hash {
    /// The vector of one text. Computed in `f64` exactly as the Python does (its floats are doubles) and rounded to
    /// `f32` at the end; the sum of squares is a sum of quarter-integers, exact in either language's `sum`.
    pub fn vector(text: &str) -> Vec<f32> {
        let mut v = [0f64; DIM];
        for w in WORD.find_iter(&text.to_lowercase()) {
            let d = Sha1::digest(w.as_str().as_bytes());
            // `int(hexdigest()[:8], 16)`: the first four bytes, big-endian
            let h = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) as usize;
            v[h % DIM] += 1.0;
            v[(h >> 9) % DIM] += 0.5;
        }
        let n = v.iter().map(|x| x * x).sum::<f64>().sqrt();
        let n = if n == 0.0 { 1.0 } else { n };
        v.iter().map(|x| (x / n) as f32).collect()
    }
}

impl Embedder for Hash {
    fn embed(&self, texts: &[&str], _query: bool) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts.iter().map(|t| Self::vector(t)).collect())
    }

    fn name(&self) -> &'static str {
        "hash"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_words_are_close_and_nothing_is_zero_length() {
        let a = Hash::vector("the parking meter asked for my pronouns");
        let b = Hash::vector("my pronouns, asked by a parking meter");
        let c = Hash::vector("a cat swiped left");
        let dot = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(p, q)| p * q).sum::<f32>();
        assert!(dot(&a, &b) > dot(&a, &c));
        assert!((dot(&a, &a) - 1.0).abs() < 1e-5);
        assert_eq!(Hash::vector(""), vec![0.0; DIM], "no words: the zero vector, not NaN");
        assert_eq!(Hash::vector("x"), Hash::vector("X"), "lower-cased first");
    }
}
