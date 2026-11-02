//! What the hosted API knows about a taped set beyond the graph: its laughs (the `laugh_run` + `laugh` rows, as
//! `surreal_kb._fetch` reads them into `laughs_doc`) and its transcript (`corpus/sets/<dir>/transcript.json`, a
//! `srcfile` row the hosted Python writes to its disk and reads back: `AppKB.transcript`). The recording itself is
//! never here: the hosted pod has no `tapes/` folder (`api/entry.py` writes `srcfile` rows under `comedy/` and
//! `vectors/` only), so `AppKB.audio_path` is always `None` and every `audio` field is `null`.
//!
//! Laughs are read with the rest of a generation (they change only with a load). Transcripts are large and read
//! only by the pages that show words, so a generation carries only each one's `sha256`; the text is fetched on
//! first use and kept per path while its hash stays the same ([`TranscriptCache`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pk_store::{Store, StoreError, from_db, vars};
use serde_json::{Map, Value, json};

use crate::index::set_dir;

/// Set folder -> its laughs doc (`{detector, run_id, ..., laughs: [...]}`).
pub type LaughDocs = HashMap<String, Map<String, Value>>;

/// Set folder -> the `sha256` of its `transcript.json` row.
pub type TranscriptShas = HashMap<String, String>;

/// One transcript segment (`AppKB.transcript`).
#[derive(Debug, Clone, PartialEq)]
pub struct Seg {
    /// `int(round(start * 1000))`.
    pub start_ms: i64,
    /// `int(round(end * 1000))`.
    pub end_ms: i64,
    /// The words, stripped.
    pub text: String,
}

impl Seg {
    /// `{start_ms, end_ms, text}`.
    pub fn to_value(&self) -> Value {
        json!({"start_ms": self.start_ms, "end_ms": self.end_ms, "text": self.text})
    }
}

/// What a generation knows about the tapes: the laughs, and which transcript each set has.
#[derive(Debug, Default)]
pub struct Tape {
    /// Set folder -> its laughs doc.
    pub laughs: LaughDocs,
    /// Set folder -> its transcript's hash.
    pub transcripts: TranscriptShas,
}

/// Folder -> segments, for the sets a request reads.
pub type Transcripts = HashMap<String, Arc<Vec<Seg>>>;

/// The transcript texts fetched so far, by `srcfile` path: `(sha256, segments)`. Shared by every generation.
#[derive(Debug, Default)]
pub struct TranscriptCache(Mutex<HashMap<String, Cached>>);

/// One cached transcript: its `sha256` and its segments.
type Cached = (String, Arc<Vec<Seg>>);

/// The `srcfile` path of a set folder's transcript.
pub fn transcript_path(dir: &str) -> String {
    format!("corpus/sets/{dir}/transcript.json")
}

/// The key of a `srcfile` row: the hex sha1 of its path.
pub fn srcfile_key(path: &str) -> String {
    use sha1::Digest as _;
    hex::encode(sha1::Sha1::digest(path.as_bytes()))
}

/// `transcript.json` -> segments, as `AppKB.transcript` reads it (an unreadable file is no segments).
pub fn parse_transcript(text: &str) -> Vec<Seg> {
    let Ok(Value::Object(doc)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let ms = |s: &Map<String, Value>, k: &str| {
        // `s.get(k, 0) * 1000`, rounded half to even like Python's `round`
        let x = s.get(k).and_then(Value::as_f64).unwrap_or(0.0) * 1000.0;
        #[expect(clippy::cast_possible_truncation, reason = "a tape position in ms fits an i64")]
        let r = x.round_ties_even() as i64;
        r
    };
    doc.get("segments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|s| Seg {
            start_ms: ms(s, "start"),
            end_ms: ms(s, "end"),
            text: pk_core::py::strip(s.get("text").and_then(Value::as_str).unwrap_or("")).to_string(),
        })
        .collect()
}

impl TranscriptCache {
    /// The transcripts of `dirs` (set folders): each from the cache when its hash is the one `shas` names, else
    /// fetched (one query for every miss). A folder with no row has no segments.
    pub async fn get(
        &self,
        store: &Store,
        shas: &TranscriptShas,
        dirs: impl IntoIterator<Item = String>,
    ) -> Result<Transcripts, StoreError> {
        let mut out = Transcripts::new();
        let mut missing: Vec<String> = Vec::new();
        {
            let cache = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            for dir in dirs {
                if out.contains_key(&dir) {
                    continue;
                }
                let Some(sha) = shas.get(&dir) else {
                    out.insert(dir, Arc::new(Vec::new()));
                    continue;
                };
                match cache.get(&transcript_path(&dir)) {
                    Some((h, segs)) if h == sha => {
                        out.insert(dir, Arc::clone(segs));
                    }
                    _ => missing.push(dir),
                }
            }
        }
        if missing.is_empty() {
            return Ok(out);
        }
        // point reads: a srcfile row's key is sha1(path) (db/push_remote.py, api-dev/ingest_api.py), so no scan of
        // the table (whose vector parts are large) is needed
        let keys: Vec<String> = missing.iter().map(|d| srcfile_key(&transcript_path(d))).collect();
        let rows = store
            .last(
                "LET $ids = array::map($ks, |$k| type::record('srcfile', $k));\
                 SELECT path, sha256, text FROM $ids;",
                vars([("ks", pk_store::to_db(&json!(keys)))]),
            )
            .await?;
        let rows: Vec<Value> = pk_store::store::rows_of(rows).into_iter().map(from_db).collect();
        // parsing is CPU (a transcript can be large): off the runtime
        let fetched: HashMap<String, Cached> = tokio::task::spawn_blocking(move || {
            rows.into_iter()
                .filter_map(|r| {
                    let (Some(p), Some(h), Some(t)) = (
                        r.get("path").and_then(Value::as_str),
                        r.get("sha256").and_then(Value::as_str),
                        r.get("text").and_then(Value::as_str),
                    ) else {
                        return None;
                    };
                    Some((p.to_string(), (h.to_string(), Arc::new(parse_transcript(t)))))
                })
                .collect()
        })
        .await
        .map_err(|e| StoreError::mapping(format!("parsing transcripts failed: {e}")))?;
        let mut fetched = fetched;
        let mut cache = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for dir in missing {
            let p = transcript_path(&dir);
            match fetched.remove(&p) {
                Some((h, segs)) => {
                    out.insert(dir, Arc::clone(&segs));
                    cache.insert(p, (h, segs));
                }
                // gone since the generation read its hash: what the file says now (nothing)
                None => {
                    out.insert(dir, Arc::new(Vec::new()));
                }
            }
        }
        Ok(out)
    }
}

/// The laughs docs as `surreal_kb._fetch` builds them: each `laugh_run` row's `doc` with its set's laughs (in `ord`
/// order; `laughs` are `(set, laugh)` as `pk_store::kbmap::row_to_set_laugh` gives them), keyed by its `set_dir`.
pub fn build(runs: Vec<Value>, laughs: Vec<(Option<String>, Map<String, Value>)>, transcripts: TranscriptShas) -> Tape {
    let mut by_set: HashMap<String, Vec<Value>> = HashMap::new();
    for (set, l) in laughs {
        if let Some(set) = set {
            by_set.entry(set).or_default().push(Value::Object(l));
        }
    }
    let mut docs = LaughDocs::new();
    for run in runs {
        let Value::Object(mut run) = run else { continue };
        let (Some(Value::String(set)), Some(Value::String(dir))) = (run.remove("set"), run.remove("set_dir")) else {
            continue;
        };
        let mut doc = match run.remove("doc") {
            Some(Value::Object(d)) => d,
            _ => Map::new(),
        };
        // a set read by two runs (never written so, but possible) keeps its laughs for both
        let ls = by_set.get(&set).cloned().unwrap_or_default();
        doc.insert("laughs".into(), Value::Array(ls));
        docs.insert(dir, doc);
    }
    Tape {
        laughs: docs,
        transcripts,
    }
}

/// `AppKB.laughs_for`: the laughs of `set_id` between `start_ms` and `end_ms` + 3 s, or `None` (no doc, no start).
pub fn laughs_for(docs: &LaughDocs, set_id: Option<&str>, start_ms: Option<&Value>, end_ms: Option<&Value>) -> Value {
    let Some(doc) = set_id.and_then(set_dir).and_then(|d| docs.get(&d)) else {
        return Value::Null;
    };
    let Some(s0) = start_ms.filter(|v| !v.is_null()).and_then(Value::as_f64) else {
        return Value::Null;
    };
    let end = end_ms.filter(|v| !v.is_null()).and_then(Value::as_f64).unwrap_or(s0) + 3000.0;
    let ls: Vec<Value> = doc
        .get("laughs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|l| {
            l.as_object()
                .and_then(|o| o.get("start_ms"))
                .and_then(Value::as_f64)
                .is_some_and(|at| s0 <= at && at <= end)
        })
        .cloned()
        .collect();
    let max_bars = max_by_num(ls.iter().map(|l| or0(l.get("bars"))));
    json!({
        "detector": doc.get("detector").cloned().unwrap_or(Value::Null),
        "run_id": doc.get("run_id").cloned().unwrap_or(Value::Null),
        "hits": ls.len(),
        "max_bars": max_bars,
        "laughs": ls,
    })
}

/// `x or 0` for a number field.
pub fn or0(v: Option<&Value>) -> Value {
    match v {
        Some(x) if pk_core::py::truthy(Some(x)) => x.clone(),
        _ => json!(0),
    }
}

/// `max(xs, default=0)` over numbers: the first largest, as it was written.
pub fn max_by_num(xs: impl Iterator<Item = Value>) -> Value {
    let mut best: Option<(f64, Value)> = None;
    for x in xs {
        let f = x.as_f64().unwrap_or(0.0);
        if best.as_ref().is_none_or(|(b, _)| f > *b) {
            best = Some((f, x));
        }
    }
    best.map_or_else(|| json!(0), |(_, v)| v)
}

/// The laughs doc of a set (`laughs_doc`), if there is one.
pub fn laughs_doc<'a>(docs: &'a LaughDocs, set_id: &str) -> Option<&'a Map<String, Value>> {
    set_dir(set_id).and_then(|d| docs.get(&d))
}

/// A doc's `laughs` list (`doc.get("laughs", [])`).
pub fn doc_laughs(doc: Option<&Map<String, Value>>) -> &[Value] {
    doc.and_then(|d| d.get("laughs"))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srcfile_keys_are_the_loaders() {
        // hashlib.sha1(path.encode()).hexdigest(): 40 lowercase hex digits, one per path (the differential proves the
        // rows are found by it: its pages read every transcript through this key)
        let k = srcfile_key(&transcript_path("20240101_x"));
        assert_eq!(k.len(), 40);
        assert!(k.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
        assert_ne!(k, srcfile_key(&transcript_path("20240101_y")));
    }

    #[test]
    fn transcripts_round_half_to_even_and_strip() {
        let t = r#"{"segments": [{"start": 0.0025, "end": 1.5, "text": "  hi \n"}, {"end": 2}, "not a segment",
                     {"start": 0.0035, "end": 0.0135, "text": null}]}"#;
        let segs = parse_transcript(t);
        assert_eq!(segs.len(), 3);
        // Python: 0.0025 * 1000 == 2.5 -> round() 2; 0.0035 * 1000 == 3.5 -> 4 (half to even)
        assert_eq!((segs[0].start_ms, segs[2].start_ms), (2, 4));
        assert_eq!((segs[0].end_ms, segs[0].text.as_str()), (1500, "hi"));
        assert_eq!((segs[1].start_ms, segs[1].end_ms, segs[1].text.as_str()), (0, 2000, ""));
        assert_eq!(parse_transcript("[1, 2]"), Vec::new());
        assert_eq!(parse_transcript("not json"), Vec::new());
    }

    #[test]
    fn laughs_in_a_stretch() {
        let mut docs = LaughDocs::new();
        let doc = json!({"detector": "d", "run_id": "r", "laughs": [
            {"start_ms": 900, "bars": 1}, {"start_ms": 1000, "bars": 2}, {"start_ms": 5000, "bars": 2},
            {"start_ms": 5001, "bars": 3}, {"bars": 9}, {"start_ms": null, "bars": 9}]});
        docs.insert("20240101_x".into(), doc.as_object().cloned().unwrap());
        let l = laughs_for(&docs, Some("set:2024-01-01_x"), Some(&json!(1000)), Some(&json!(2000)));
        assert_eq!(l["hits"], 2, "start <= at <= end + 3 s, both ends in");
        assert_eq!(l["max_bars"], 2);
        assert_eq!(l["run_id"], "r");
        let no_end = laughs_for(&docs, Some("set:2024-01-01_x"), Some(&json!(1000)), Some(&Value::Null));
        assert_eq!(no_end["hits"], 1, "no end: start + 3 s");
        assert_eq!(laughs_for(&docs, Some("set:2024-01-01_x"), None, None), Value::Null);
        assert_eq!(
            laughs_for(&docs, Some("set:2024-09-09_y"), Some(&json!(0)), None),
            Value::Null
        );
    }

    #[test]
    fn the_first_largest_as_written() {
        let m = max_by_num([json!(2), json!(3.0), json!(3), json!(1)].into_iter());
        assert!(m.is_f64(), "the first of the equal largest: {m}");
        assert_eq!(max_by_num(std::iter::empty()), json!(0));
        assert_eq!(or0(Some(&Value::Null)), json!(0));
        assert_eq!(or0(Some(&json!(2))), json!(2));
    }

    #[test]
    fn laugh_docs_group_by_set() {
        let runs = vec![
            json!({"set": "set:a", "set_dir": "20240101_a", "doc": {"detector": "d"}}),
            json!({"set": "set:b", "set_dir": "20240101_b"}),
            json!({"set_dir": "no set"}),
        ];
        let l = |s: Option<&str>, at: i64| {
            (
                s.map(str::to_string),
                json!({"start_ms": at}).as_object().cloned().unwrap(),
            )
        };
        let tape = build(
            runs,
            vec![l(Some("set:a"), 1), l(None, 2), l(Some("set:a"), 3)],
            TranscriptShas::new(),
        );
        assert_eq!(tape.laughs.len(), 2);
        assert_eq!(
            tape.laughs["20240101_a"]["laughs"],
            json!([{"start_ms": 1}, {"start_ms": 3}])
        );
        assert_eq!(tape.laughs["20240101_a"]["detector"], "d");
        assert_eq!(tape.laughs["20240101_b"]["laughs"], json!([]));
    }
}
