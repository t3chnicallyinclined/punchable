//! Voice memos and app ideas linked to his bits (`creative/memolink.py`), the halves with no model and no IO:
//! which ideas have no units yet (`summary_only`, the text the matcher embeds), the text hash the vector file keys
//! on, and the vector file's update (`update_summary_vectors`): new or edited ideas embedded, gone ones dropped.

#![allow(clippy::implicit_hasher)] // the sets and maps passed in are this crate's own (std's hasher)
use std::collections::HashSet;

use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::corpus::{Corpus, s};

/// The vector file's path in the workspace (`vectors/extra_summary.json`).
pub const SUMMARY_FILE: &str = "vectors/extra_summary.json";

/// `sha(text)`: the first 16 hex of sha256.
pub fn sha16(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))[..16].to_string()
}

/// `" ".join(text.split())`: words joined by one space (Python's whitespace).
pub fn squash(text: &str) -> String {
    text.split(pk_core::py::isspace)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A joke's units as the same-idea index holds them (`merge.Index.units`): its non-hidden units, sorted.
pub fn units_of(c: &Corpus, j: &str) -> Vec<String> {
    let mut us: Vec<String> = c
        .joke_units
        .get(j)
        .map(|us| {
            us.iter()
                .filter(|u| !c.nodes.get(*u).is_some_and(|n| pk_core::py::truthy(n.get("hidden"))))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    us.sort();
    us
}

/// `summary_only(c)`: jokes with no units yet (an idea from a memo, the app, a Google Doc) → the text the matcher
/// sees, in joke-id order.
pub fn summary_only(c: &Corpus) -> IndexMap<String, String> {
    let mut jokes: Vec<&String> = c.jokes.iter().collect();
    jokes.sort();
    let mut out = IndexMap::new();
    for j in jokes {
        if !units_of(c, j).is_empty() {
            continue;
        }
        let summary = c.nodes.get(j).and_then(|n| s(n, "summary")).unwrap_or("");
        if !pk_core::py::strip(summary).is_empty() {
            out.insert(j.clone(), squash(summary));
        }
    }
    out
}

/// The vector file as `embed_summaries` keeps it: `{dim, model, rows, text_sha}` (an unreadable file starts over).
pub fn summary_doc(text: Option<&str>) -> Map<String, Value> {
    let mut doc = text
        .and_then(|t| serde_json::from_str::<Value>(t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_else(|| {
            json!({"dim": 384, "model": "bge-small-en-v1.5", "rows": {}, "text_sha": {}})
                .as_object()
                .cloned()
                .unwrap_or_default()
        });
    for k in ["rows", "text_sha"] {
        if !doc.get(k).is_some_and(Value::is_object) {
            doc.insert(k.to_string(), json!({}));
        }
    }
    doc
}

/// What to embed: the summary-only ideas whose text hash moved and that are not court text, by joke id.
pub fn to_embed(
    doc: &Map<String, Value>,
    want: &IndexMap<String, String>,
    is_legal: impl Fn(&str) -> bool,
) -> Vec<(String, String)> {
    let tsha = doc.get("text_sha").and_then(Value::as_object);
    let mut todo: Vec<(String, String)> = want
        .iter()
        .filter(|(j, t)| {
            tsha.and_then(|m| m.get(&format!("summary:{j}")))
                .and_then(Value::as_str)
                != Some(sha16(t).as_str())
                && !is_legal(t)
        })
        .map(|(j, t)| (j.clone(), t.clone()))
        .collect();
    todo.sort_by(|a, b| a.0.cmp(&b.0));
    todo
}

/// `ATTACH_JUDGE_MIN`: the judge's confidence a clear attach needs.
pub const ATTACH_JUDGE_MIN: f64 = 0.90;
/// `LONG_CHARS`: longer memos were kept as notes, not filed as ideas.
pub const LONG_CHARS: usize = 4000;
/// `TRIAGE_KINDS`.
pub const TRIAGE_KINDS: [&str; 6] = ["joke", "performed_set", "rehearsal", "conversation", "note", "unclear"];
/// `TRIAGE_BATCH` / `TRIAGE_BATCH_LONG`.
pub const TRIAGE_BATCH: usize = 8;
/// Long memos per triage call.
pub const TRIAGE_BATCH_LONG: usize = 3;
/// The merge judge's cache file.
pub const JUDGE_FILE: &str = "kb/creative/merge_judge.json";
/// The memo triage's cache file.
pub const TRIAGE_FILE: &str = "kb/creative/suggest_judge.json";
/// `TRIAGE_SYSTEM`.
pub const TRIAGE_SYSTEM: &str = "You sort one stand-up comedian's own voice memos (speech-to-text transcripts, so expect \
errors). For each memo pick ONE kind: joke = a joke, bit, premise, tag, story or riff he could use on stage, even rough \
or half-formed; performed_set = a recording of him performing to a live audience (audience laughter or reactions, crowd \
work, 'give it up', 'thank you guys', a host bringing him up); rehearsal = him running material alone, as if on stage, \
with no audience; conversation = him talking with other people that is not him performing or working out a bit; note = \
a work, tech, business, shopping or personal note, reminder or message to someone, not comedy; unclear = too short or \
garbled to tell. A memo that is mostly comedy material is joke even if it wanders. audience = true only if the \
transcript itself shows a live audience. confidence is how sure you are (0-1). why is one plain sentence a comic would \
understand, quoting a few words when they decide it.";

/// `TRIAGE_SCHEMA`.
pub static TRIAGE_SCHEMA: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    json!({"type": "object", "properties": {"items": {"type": "array", "items": {
        "type": "object",
        "properties": {"id": {"type": "string"}, "kind": {"type": "string", "enum": TRIAGE_KINDS},
                       "audience": {"type": "boolean"}, "confidence": {"type": "number"}, "why": {"type": "string"}},
        "required": ["id", "kind", "audience", "confidence", "why"], "additionalProperties": false}}},
        "required": ["items"], "additionalProperties": false})
});

/// `memos(comedy)`: transcribed memos (`kb/gsync/memos.json` items with status done) with their transcript's text,
/// by key, in the file's order. `files` is the workspace (path → text).
pub fn memos<T: AsRef<str>>(files: &std::collections::BTreeMap<String, T>) -> IndexMap<String, Map<String, Value>> {
    let mut out = IndexMap::new();
    let Some(doc) = files
        .get("kb/gsync/memos.json")
        .and_then(|t| serde_json::from_str::<Value>(t.as_ref()).ok())
    else {
        return out;
    };
    for (k, it) in doc.get("items").and_then(Value::as_object).into_iter().flatten() {
        let Some(it) = it.as_object() else { continue };
        if it.get("status").and_then(Value::as_str) != Some("done") || !pk_core::py::truthy(it.get("transcript")) {
            continue;
        }
        let path = it.get("transcript").map(pk_core::py::str).unwrap_or_default();
        let Some(tx) = files
            .get(&path)
            .and_then(|t| serde_json::from_str::<Value>(t.as_ref()).ok())
        else {
            continue;
        };
        let segs = tx
            .get("segments")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let joined: Vec<&str> = segs
            .iter()
            .map(|s| pk_core::py::strip(s.get("text").and_then(Value::as_str).unwrap_or("")))
            .collect();
        let text = squash(&joined.join(" "));
        if !text.is_empty() {
            let mut m = it.clone();
            m.insert("text".into(), json!(text));
            out.insert(k.clone(), m);
        }
    }
    out
}

/// `memo_jokes(c, ms)`: joke id → memo key, for memo ideas still in the corpus.
pub fn memo_jokes(c: &Corpus, ms: &IndexMap<String, Map<String, Value>>) -> IndexMap<String, String> {
    ms.iter()
        .filter_map(|(k, it)| {
            let j = it.get("joke_id").and_then(Value::as_str)?;
            c.is_joke(j).then(|| (j.to_string(), k.clone()))
        })
        .collect()
}

fn excerpt_long(t: &str) -> String {
    let ch: Vec<char> = t.chars().collect();
    if ch.len() <= 3200 {
        return t.to_string();
    }
    let m = ch.len() / 2;
    let s = |a: usize, b: usize| ch[a..b].iter().collect::<String>();
    format!(
        "{} [...] {} [...] {}",
        s(0, 1600),
        s(m - 400, m + 400),
        s(ch.len() - 800, ch.len())
    )
}

/// `triage`'s batches: memos whose text moved since their cached verdict, court text left out (and counted),
/// short ones eight at a time, long ones three.
pub fn triage_batches(
    ms: &IndexMap<String, Map<String, Value>>,
    cache: &Map<String, Value>,
    is_legal: impl Fn(&str) -> bool,
) -> (Vec<Vec<String>>, usize, usize) {
    let mut keys: Vec<&String> = ms.keys().collect();
    keys.sort();
    let text = |k: &str| ms[k].get("text").and_then(Value::as_str).unwrap_or("").to_string();
    let todo: Vec<String> = keys
        .into_iter()
        .filter(|k| {
            cache.get(*k).and_then(|v| v.get("text_sha")).and_then(Value::as_str) != Some(sha16(&text(k)).as_str())
        })
        .cloned()
        .collect();
    let (skipped, todo): (Vec<String>, Vec<String>) = todo.into_iter().partition(|k| is_legal(&text(k)));
    let (short, long): (Vec<String>, Vec<String>) = todo
        .iter()
        .cloned()
        .partition(|k| text(k).chars().count() <= LONG_CHARS);
    let mut batches: Vec<Vec<String>> = short.chunks(TRIAGE_BATCH).map(<[String]>::to_vec).collect();
    batches.extend(long.chunks(TRIAGE_BATCH_LONG).map(<[String]>::to_vec));
    (batches, todo.len(), skipped.len())
}

/// The triage's user turn for one batch.
pub fn triage_user(ms: &IndexMap<String, Map<String, Value>>, batch: &[String]) -> String {
    let lines: Vec<String> = batch
        .iter()
        .enumerate()
        .map(|(n, k)| {
            let it = &ms[k];
            let t = it.get("text").and_then(Value::as_str).unwrap_or("");
            let len = t.chars().count();
            let title = it
                .get("title")
                .filter(|v| pk_core::py::truthy(Some(v)))
                .map(pk_core::py::str)
                .unwrap_or_default();
            let secs = crate::corpus::num(it.get("duration_s")).unwrap_or(0.0) as i64;
            let body = if len > LONG_CHARS {
                excerpt_long(t)
            } else {
                t.chars().take(3200).collect()
            };
            format!(
                "MEMO m{} (title '{title}', {secs} s, {len} characters{}):\n{body}",
                n + 1,
                if len > 3200 { ", excerpts" } else { "" }
            )
        })
        .collect();
    format!("Sort each memo.\n\n{}", lines.join("\n\n"))
}

/// The triage's answers into the cache; how many landed.
pub fn triage_finish(
    ms: &IndexMap<String, Map<String, Value>>,
    batch: &[String],
    data: Option<&Value>,
    model: &str,
    cache: &mut Map<String, Value>,
) -> usize {
    let got: std::collections::HashMap<String, &Value> = data
        .and_then(|d| d.get("items"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|x| (x.get("id").map_or_else(|| "None".to_string(), pk_core::py::str), x))
        .collect();
    let mut done = 0;
    for (n, k) in batch.iter().enumerate() {
        let Some(x) = got.get(&format!("m{}", n + 1)) else {
            continue;
        };
        let Some(kind) = x
            .get("kind")
            .and_then(Value::as_str)
            .filter(|k| TRIAGE_KINDS.contains(k))
        else {
            continue;
        };
        let t = ms[k].get("text").and_then(Value::as_str).unwrap_or("");
        let why: String = x
            .get("why")
            .filter(|v| pk_core::py::truthy(Some(v)))
            .map(pk_core::py::str)
            .unwrap_or_default();
        cache.insert(
            k.clone(),
            json!({"kind": kind, "audience": pk_core::py::truthy(x.get("audience")),
                   "confidence": crate::corpus::num(x.get("confidence")).unwrap_or(0.0),
                   "why": why.chars().take(300).collect::<String>(), "text_sha": sha16(t), "chars": t.chars().count(),
                   "joke_id": ms[k].get("joke_id").cloned().unwrap_or(Value::Null), "model": model}),
        );
        done += 1;
    }
    done
}

/// `set_recordings(creative_dir)`: memo ideas the triage heard as a recording of a performed set.
pub fn set_recordings(triage_doc: Option<&Value>) -> HashSet<String> {
    triage_doc
        .and_then(|d| d.get("items"))
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, v)| v.get("kind").and_then(Value::as_str) == Some("performed_set"))
        .filter_map(|(_, v)| {
            v.get("joke_id")
                .and_then(Value::as_str)
                .filter(|j| !j.is_empty())
                .map(str::to_string)
        })
        .collect()
}

fn groups_for(
    ix: &crate::merge::Index,
    judgements: &Map<String, Value>,
    st: &crate::merge::MergeState,
    recordings: &HashSet<String>,
) -> Vec<Value> {
    let exclude: HashSet<String> = st.attached.keys().cloned().chain(recordings.iter().cloned()).collect();
    crate::merge::build_groups(ix, judgements, st, &exclude)
}

/// `suggestions(c, creative_dir, mj, items)`: the "Same idea · Merge" groups that include a memo idea.
pub fn suggestions(
    ix: &crate::merge::Index,
    judgements: &Map<String, Value>,
    st: &crate::merge::MergeState,
    recordings: &HashSet<String>,
    mj: &IndexMap<String, String>,
) -> Vec<Value> {
    groups_for(ix, judgements, st, recordings)
        .into_iter()
        .filter(|g| {
            g["members"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|m| m["joke_id"].as_str().is_some_and(|j| mj.contains_key(j)))
        })
        .collect()
}

/// `clear_attaches(c, creative_dir, mj, items)`: memo ideas that clearly ARE one existing bit: the judge says same
/// at >= `ATTACH_JUDGE_MIN` with exactly one bit (or its group), and `match_text` places the memo there on its own.
pub fn clear_attaches(
    ix: &crate::merge::Index,
    c: &Corpus,
    judgements: &Map<String, Value>,
    st: &crate::merge::MergeState,
    recordings: &HashSet<String>,
    mj: &IndexMap<String, String>,
) -> Vec<Value> {
    use crate::corpus::num;
    let groups = groups_for(ix, judgements, st, recordings);
    let group_of: std::collections::HashMap<String, String> = groups
        .iter()
        .flat_map(|g| {
            let main = g["main"].as_str().unwrap_or("").to_string();
            g["members"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(move |m| Some((m["joke_id"].as_str()?.to_string(), main.clone())))
        })
        .collect();
    let mut sure: std::collections::BTreeMap<String, Vec<(String, f64, Value)>> = std::collections::BTreeMap::new();
    for (a, b) in ix.candidates() {
        let (am, bm) = (mj.contains_key(&a), mj.contains_key(&b));
        if am == bm {
            continue;
        }
        let (memo, bit) = if am {
            (a.clone(), b.clone())
        } else {
            (b.clone(), a.clone())
        };
        if ix.units.get(&bit).is_none_or(Vec::is_empty) || st.dismissed.contains(&(a.clone(), b.clone())) {
            continue;
        }
        let f = ix.features(&a, &b);
        let Some(v) = judgements
            .get(&ix.judge_key(&f))
            .filter(|v| pk_core::py::truthy(Some(v)))
        else {
            continue;
        };
        if !pk_core::py::truthy(v.get("same")) {
            continue;
        }
        let mut conf = num(v.get("confidence")).unwrap_or(0.0);
        if let Some(sec) = v.get("second").filter(|s| pk_core::py::truthy(s.get("same"))) {
            conf = conf.max(num(sec.get("confidence")).unwrap_or(0.0));
        }
        if conf >= ATTACH_JUDGE_MIN {
            sure.entry(memo)
                .or_default()
                .push((bit, conf, v.get("why").cloned().unwrap_or(Value::Null)));
        }
    }
    let mut out = Vec::new();
    for (memo, hits) in sure {
        if st.attached.contains_key(&memo) || st.merged.contains(&memo) || recordings.contains(&memo) {
            continue;
        }
        let homes: HashSet<String> = hits
            .iter()
            .map(|(b, _, _)| group_of.get(b).cloned().unwrap_or_else(|| b.clone()))
            .collect();
        if homes.len() != 1 {
            continue;
        }
        let Some(home) = homes.into_iter().next() else { continue };
        let text = squash(c.joke(&memo).and_then(|n| s(n, "summary")).unwrap_or(""));
        let vec = c.vectors.get(&format!("summary:{memo}"));
        let exclude: HashSet<String> = HashSet::from([memo.clone()]);
        let r = crate::merge::match_text(ix, &text, vec, &exclude, &group_of);
        let rj = r["joke_id"].as_str().unwrap_or("").to_string();
        if r["decision"] != "attach" || group_of.get(&rj).cloned().unwrap_or(rj) != home {
            continue;
        }
        let mut best = &hits[0];
        for h in &hits[1..] {
            if h.1 > best.1 {
                best = h;
            }
        }
        out.push(json!({"memo_joke": memo, "into": home, "into_title": c.title(&home), "memo_title": c.title(&memo),
                        "judge_confidence": crate::corpus::round_to(best.1, 2), "judge_why": best.2,
                        "matcher": r["reasons"], "text": text, "score": r.get("score").cloned().unwrap_or(Value::Null)}));
    }
    out
}

/// `attach_event(x, memo_key)`: the event id (`at:<sha8 of [memo, into]>`) and its data; `today` is the date.
pub fn attach_event(x: &Value, today: &str) -> (String, Value) {
    let ids = [
        x["memo_joke"].as_str().unwrap_or("").to_string(),
        x["into"].as_str().unwrap_or("").to_string(),
    ];
    let aid = format!("at:{}", crate::merge::sha8(&ids));
    let conf = crate::corpus::num(x.get("judge_confidence")).unwrap_or(0.0);
    let mut reasons = vec![json!(format!(
        "Claude: the same bit ({conf:.2}): {}",
        pk_core::py::str(&x["judge_why"])
    ))];
    reasons.extend(x["matcher"].as_array().into_iter().flatten().take(2).cloned());
    (
        aid,
        json!({"joke": x["memo_joke"], "into": x["into"], "text": x["text"], "date": today,
                 "score": x.get("score").cloned().unwrap_or(Value::Null), "source": "memo", "reasons": reasons}),
    )
}

/// `float(f"{x:.7g}")`: seven significant digits, as the file stores a vector.
pub fn g7(x: f32) -> f64 {
    format!("{:.6e}", f64::from(x)).parse().unwrap_or(f64::from(x))
}

/// The file after embedding: new rows and hashes in, rows for ideas no longer summary-only out. Returns how many
/// were removed.
pub fn update_summary_vectors(
    doc: &mut Map<String, Value>,
    want: &IndexMap<String, String>,
    embedded: &[(String, String, Vec<f32>)],
) -> usize {
    let wanted: HashSet<&str> = want.keys().map(String::as_str).collect();
    // in place, as the Python's dicts: the file keeps its key order
    let mut gone: Vec<String> = Vec::new();
    if let Some(rows) = doc.get_mut("rows").and_then(Value::as_object_mut) {
        for (j, _, v) in embedded {
            rows.insert(
                format!("summary:{j}"),
                Value::Array(v.iter().map(|x| json!(g7(*x))).collect()),
            );
        }
        gone = rows
            .keys()
            .filter(|k| !wanted.contains(k.strip_prefix("summary:").unwrap_or(k)))
            .cloned()
            .collect();
        for k in &gone {
            rows.shift_remove(k);
        }
    }
    if let Some(tsha) = doc.get_mut("text_sha").and_then(Value::as_object_mut) {
        for (j, text, _) in embedded {
            tsha.insert(format!("summary:{j}"), json!(sha16(text)));
        }
        for k in &gone {
            tsha.shift_remove(k);
        }
    }
    gone.len()
}
