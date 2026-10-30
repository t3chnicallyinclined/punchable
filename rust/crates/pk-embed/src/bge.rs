//! The model: bge-small-en-v1.5's ONNX export over `ort`, its `tokenizer.json` over `tokenizers`, run the way
//! `engine/embed.py` runs it so the vectors agree with the stored ones.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ndarray::{Array2, Axis};
use ort::environment::Environment;
use ort::ep;
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::{Encoding, PaddingParams, Tokenizer, TruncationParams};

use crate::{
    BATCH, DIM, EmbedConfig, EmbedError, Embedder, FINGERPRINT, FINGERPRINT_SENTENCE, MAX_TOKENS, MODEL_NAME, prepare,
};

/// bge-small-en-v1.5 from a folder with `onnx/model.onnx` and `tokenizer.json`.
///
/// CLS pooling (the first token of the last hidden state), L2-normalised, 384-d. No CPU memory arena: with it,
/// onnxruntime keeps the peak of the largest batch for the life of the process (measured on the hosted API,
/// 2026-10-28: 824 MiB against a 1 GiB limit); without it each batch's buffers go back to the OS. The session sits
/// behind a mutex because `ort` runs inference on `&mut Session`; that also makes `Bge` shareable across threads.
pub struct Bge {
    dir: PathBuf,
    tok: Tokenizer,
    session: Mutex<Session>,
}

impl std::fmt::Debug for Bge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bge").field("dir", &self.dir).finish_non_exhaustive()
    }
}

/// One ONNX Runtime environment per process, created on first use.
fn environment() -> Result<Environment, EmbedError> {
    static ENV: OnceLock<Environment> = OnceLock::new();
    if let Some(env) = ENV.get() {
        return Ok(env.clone());
    }
    let built = ort::init().with_name("pk-embed").build()?;
    Ok(ENV.get_or_init(|| built).clone())
}

impl Bge {
    /// Load the model from `dir` with the Python's thread count: one intra-op thread per CPU, at most four.
    pub fn load(dir: &Path) -> Result<Self, EmbedError> {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().clamp(1, 4));
        Self::load_with_threads(dir, threads)
    }

    /// Load the model from `dir` with a given number of intra-op threads (the API uses one: its pod has one CPU).
    pub fn load_with_threads(dir: &Path, intra_op_threads: usize) -> Result<Self, EmbedError> {
        if !crate::has_model(dir) {
            return Err(EmbedError::NoModel(dir.to_path_buf()));
        }
        let mut tok = Tokenizer::from_file(dir.join("tokenizer.json"))?;
        // tokenizers' defaults are Python's `enable_truncation(512)` and `enable_padding()`: longest-first, right,
        // stride 0; pad to the batch's longest with id 0 / type 0 / "[PAD]"
        tok.with_truncation(Some(TruncationParams {
            max_length: MAX_TOKENS,
            ..TruncationParams::default()
        }))?;
        tok.with_padding(Some(PaddingParams::default()));
        let session = Session::builder(&environment()?)?
            .with_execution_providers([ep::CPU::default().with_arena_allocator(false).build()])
            .map_err(ort::Error::from)?
            .with_intra_threads(intra_op_threads.max(1))
            .map_err(ort::Error::from)?
            .commit_from_file(dir.join("onnx").join("model.onnx"))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            tok,
            session: Mutex::new(session),
        })
    }

    /// Load from where [`EmbedConfig::model_dir`] says the model is.
    pub fn open(cfg: &EmbedConfig) -> Result<Self, EmbedError> {
        let dir = cfg
            .model_dir()
            .ok_or_else(|| EmbedError::NoModel(cfg.bge_dir.clone().unwrap_or_else(|| PathBuf::from("$PK_BGE_DIR"))))?;
        Self::load(&dir)
    }

    /// The folder the model was loaded from.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The token ids the model sees for an already [`prepare`]d text: `[CLS]`, the word pieces, `[SEP]`, truncated
    /// to [`MAX_TOKENS`], not padded.
    pub fn token_ids(&self, prepared: &str) -> Result<Vec<u32>, EmbedError> {
        Ok(self.tok.encode(prepared, true)?.get_ids().to_vec())
    }

    /// The `fetch_bge.py` check: the first six values of the fingerprint sentence's passage vector, rounded to five
    /// places as the script rounds them, and whether each is within `1e-4` of [`FINGERPRINT`].
    pub fn fingerprint(&self) -> Result<(bool, [f32; 6]), EmbedError> {
        let v = self.embed(&[FINGERPRINT_SENTENCE], false)?;
        let mut got = [0f32; 6];
        for (g, x) in got.iter_mut().zip(&v[0]) {
            *g = (x * 1e5).round() / 1e5;
        }
        let ok = got.iter().zip(FINGERPRINT).all(|(a, b)| (a - b).abs() < 1e-4);
        Ok((ok, got))
    }

    /// One batch (already sorted, at most [`BATCH`] texts) through the tokenizer and the model; the CLS rows, unit
    /// length.
    fn run_batch(&self, session: &mut Session, texts: Vec<&str>) -> Result<Vec<Vec<f32>>, EmbedError> {
        let enc = self.tok.encode_batch(texts, true)?;
        let (b, n) = (enc.len(), enc.first().map_or(0, Encoding::len));
        if enc.iter().any(|e| e.len() != n) {
            return Err(EmbedError::Shape("the batch did not pad to one length".into()));
        }
        let column = |f: fn(&Encoding) -> &[u32]| Array2::from_shape_fn((b, n), |(i, j)| i64::from(f(&enc[i])[j]));
        let outputs = session.run(ort::inputs![
            "input_ids" => Tensor::from_array(column(Encoding::get_ids))?,
            "attention_mask" => Tensor::from_array(column(Encoding::get_attention_mask))?,
            "token_type_ids" => Tensor::from_array(column(Encoding::get_type_ids))?,
        ])?;
        let hidden = outputs[0].try_extract_array::<f32>()?;
        let shape = hidden.shape();
        if shape.len() != 3 || shape[0] != b || shape[2] != DIM {
            return Err(EmbedError::Shape(format!("{shape:?}")));
        }
        Ok((0..b)
            .map(|row| {
                let cls = hidden.index_axis(Axis(0), row);
                let cls = cls.index_axis(Axis(0), 0);
                let norm = cls.iter().map(|x| x * x).sum::<f32>().sqrt();
                cls.iter().map(|x| x / norm).collect()
            })
            .collect())
    }
}

impl Embedder for Bge {
    fn embed(&self, texts: &[&str], query: bool) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let prepared: Vec<String> = texts.iter().map(|t| prepare(t, query)).collect();
        // shortest first (a stable sort on the character count, Python's `len`), then batches of eight: each batch
        // pads only to its own longest text
        let mut order: Vec<usize> = (0..prepared.len()).collect();
        order.sort_by_key(|&i| prepared[i].chars().count());
        let mut out = vec![Vec::new(); prepared.len()];
        let mut session = self.session.lock().map_err(|_| EmbedError::Poisoned)?;
        for chunk in order.chunks(BATCH) {
            let rows = self.run_batch(&mut session, chunk.iter().map(|&i| prepared[i].as_str()).collect())?;
            for (&i, row) in chunk.iter().zip(rows) {
                out[i] = row;
            }
        }
        Ok(out)
    }

    fn name(&self) -> &'static str {
        MODEL_NAME
    }
}
