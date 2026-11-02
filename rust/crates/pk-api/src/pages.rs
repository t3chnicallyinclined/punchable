//! The pages built over one generation's index: a set's card, its analysis and its review steps, one bit's page,
//! and the pieces they share (`api-dev/server.py` `set_card`, `set_info`, `analysis_rows`, `_summary`,
//! `analysis_brief`, `set_analysis`, `review_steps`, `punch_of`, `brief_joke`, `joke_text`, `Handler.joke`, and the
//! `AppIndex` methods `edge_state`, `deco`, `stage_ai`, `perf_rating`, `joke_rooms`). Every rule is the Python's.
//!
//! Hosted shape (settled from the code): the laughs are the database's (`laugh_run` + `laugh`), the transcripts are
//! `srcfile` rows, and there is no recording on the pod (`AppKB.audio_path` and `video_description` read `tapes/`,
//! which the hosted API never writes), so `audio` is always `null`, `has_audio` `false` and `description` `null`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use pk_core::py::truthy;
use pk_kb::{Bucket, Rec, View, bucket, public};
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::display::{
    at_label, last_sentence_split, r2, short_name_of, suggest_rating, venue_id_of, venue_label, when_label,
};
use crate::index::{AppIndex, NOT_COUNTED, set_dir};
use crate::snapshot::Data;
use crate::tape::{Seg, Transcripts, doc_laughs, laughs_doc, laughs_for, max_by_num, or0};

/// `RANK`.
fn rank(r: &str) -> i64 {
    match r {
        "killed" => 3,
        "worked" => 2,
        "meh" => 1,
        "died" => 0,
        _ => -1,
    }
}

const SUMMARY_PASS: [&str; 13] = [
    "material_s",
    "laugh_s_total",
    "laugh_s_per_min",
    "pct_laughing",
    "laughs_merged",
    "laughs_per_min_merged",
    "applause_s",
    "longest_laugh_s",
    "longest_laugh_joke",
    "dead_air_max_s",
    "dead_air_max_at_ms",
    "room_heat",
    "room_heat_basis",
];
const ROW_PASS: [&str; 5] = [
    "laugh_s_per_min",
    "pct_laughing",
    "vs_room",
    "time_to_first_laugh_s",
    "dead_air_max_s",
];

/// `x or 0` as a number.
fn n0(v: Option<&Value>) -> f64 {
    v.filter(|x| truthy(Some(x))).and_then(Value::as_f64).unwrap_or(0.0)
}

/// A number as JSON: an integer when it is one (the Python's ints stay ints).
#[expect(clippy::cast_possible_truncation, reason = "checked integral and in range")]
pub fn num(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 9.0e15 {
        json!(x as i64)
    } else {
        json!(x)
    }
}

fn get(m: &Map<String, Value>, k: &str) -> Value {
    m.get(k).cloned().unwrap_or(Value::Null)
}

fn sget<'a>(m: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    m.get(k).and_then(Value::as_str)
}

/// `x or default`.
fn or(m: &Map<String, Value>, k: &str, default: Value) -> Value {
    match m.get(k) {
        Some(x) if truthy(Some(x)) => x.clone(),
        _ => default,
    }
}

fn split_words(s: &str) -> usize {
    s.split(pk_core::py::isspace).filter(|w| !w.is_empty()).count()
}

static NOT_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\w' ]+").unwrap_or_else(|e| unreachable!("constant regex: {e}")));

/// `_title_from`: the first six words, or `New line`.
fn title_from(text: &str) -> String {
    let t = NOT_WORD.replace_all(text, " ");
    let w: Vec<&str> = t
        .split(pk_core::py::isspace)
        .filter(|w| !w.is_empty())
        .take(6)
        .collect();
    let out = w.join(" ");
    if out.is_empty() { "New line".into() } else { out }
}

static YT_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A[A-Za-z0-9_-]{11}\z").unwrap_or_else(|e| unreachable!("constant regex: {e}")));

/// `set_source`: `youtube`, `voice_memo`, `app_upload` or `tape`.
fn set_source(meta: &Map<String, Value>, d: Option<&str>) -> &'static str {
    match sget(meta, "source") {
        Some("voice_memo") => return "voice_memo",
        Some("app_upload") => return "app_upload",
        _ => {}
    }
    if let Some(vid) = meta.get("id").filter(|v| truthy(Some(v))) {
        let vid = pk_core::py::str(vid);
        if d.is_some_and(|d| d.ends_with(&format!("_{vid}"))) && YT_ID.is_match(&vid) && truthy(meta.get("upload_date"))
        {
            return "youtube";
        }
    }
    "tape"
}

/// `set_reference`: where the recording lives (a video, his Drive file, the file on his phone), or `None`.
fn set_reference(meta: &Map<String, Value>, d: Option<&str>) -> Value {
    let src = set_source(meta, d);
    if src == "youtube" {
        let id = pk_core::py::str(&get(meta, "id"));
        return json!({"kind": "youtube", "video_id": get(meta, "id"),
                      "url": format!("https://www.youtube.com/watch?v={id}"), "title": get(meta, "title")});
    }
    if truthy(meta.get("drive_id")) {
        let id = pk_core::py::str(&get(meta, "drive_id"));
        return json!({"kind": "drive", "url": format!("https://drive.google.com/file/d/{id}/view"),
                      "name": get(meta, "original_name")});
    }
    if matches!(src, "app_upload" | "voice_memo") {
        return json!({"kind": "file", "name": get(meta, "original_name"), "recorded_at": get(meta, "recorded_at")});
    }
    Value::Null
}

/// `perf_rating`: `(effective rating, by)`; his rating wins over the AI's.
pub fn perf_rating(p: &Rec) -> (Value, Value) {
    if truthy(p.get("rating")) {
        (get(p, "rating"), json!("you"))
    } else if truthy(p.get("ai_rating")) {
        (get(p, "ai_rating"), json!("ai"))
    } else {
        (Value::Null, Value::Null)
    }
}

/// The set ids whose transcripts a bit's page reads (every performance on its `performed_as` edges).
pub fn joke_set_dirs(v: &View, jid: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (_, e) in v.joke_edges(jid) {
        if sget(e, "kind") != Some("performed_as") {
            continue;
        }
        for end in [sget(e, "src"), sget(e, "dst")] {
            if v.ntype(end) == Some("performance")
                && let Some(d) = end
                    .and_then(|p| v.nodes.get(p))
                    .and_then(|p| sget(p, "set"))
                    .and_then(set_dir)
                && !out.contains(&d)
            {
                out.push(d);
            }
        }
    }
    out
}

/// One request's view of a generation: the index, the data and the transcripts it fetched.
pub struct Ctx<'a> {
    /// The index (and its view).
    pub idx: &'a AppIndex,
    /// The view.
    pub v: &'a View,
    /// The generation.
    pub d: &'a Data,
    /// The transcripts this request reads, by set folder.
    pub tx: &'a Transcripts,
}

struct Laughs<'a>(&'a Data);

impl pk_kb::read::LaughsFor for Laughs<'_> {
    fn laughs_for(&self, set_id: Option<&str>, start_ms: Option<&Value>, end_ms: Option<&Value>) -> Value {
        laughs_for(&self.0.tape.laughs, set_id, start_ms, end_ms)
    }
}

/// `{"laughs": [], "max_bars": 0, "hits": 0}`: a stretch with no laughs doc.
fn no_laughs() -> Value {
    json!({"laughs": [], "max_bars": 0, "hits": 0})
}

impl<'a> Ctx<'a> {
    /// Over `idx` and `d`, with the transcripts `tx`.
    pub fn new(idx: &'a AppIndex, d: &'a Data, tx: &'a Transcripts) -> Self {
        Self {
            idx,
            v: &idx.view,
            d,
            tx,
        }
    }

    fn node(&self, id: Option<&str>) -> Option<&'a Rec> {
        id.and_then(|i| self.v.nodes.get(i))
    }

    fn perf_edge(&self, pid: &str) -> Option<&'a Rec> {
        self.idx.perf_edge.get(pid).and_then(|e| self.v.edges.get(e))
    }

    /// `joke_of`: the joke a performance's edge points at (whatever its status).
    pub fn joke_of(&self, pid: &str) -> Option<&'a str> {
        let e = self.perf_edge(pid)?;
        let dst = sget(e, "dst");
        if self.v.ntype(dst) == Some("joke") {
            dst
        } else {
            sget(e, "src")
        }
    }

    /// The joke when the performance's edge is not rejected (`live_joke`).
    fn live_joke(&self, pid: &str) -> Option<&'a str> {
        let e = self.perf_edge(pid)?;
        (bucket(e.get("status")) != Bucket::Rejected)
            .then(|| self.joke_of(pid))
            .flatten()
    }

    fn laughs(&self, p: &Rec) -> Value {
        match laughs_for(&self.d.tape.laughs, sget(p, "set"), p.get("start_ms"), p.get("end_ms")) {
            Value::Null => no_laughs(),
            l => l,
        }
    }

    fn transcript(&self, set_id: Option<&str>) -> &[Seg] {
        set_id
            .and_then(set_dir)
            .and_then(|d| self.tx.get(&d))
            .map_or(&[], |s| s.as_slice())
    }

    /// `edge_state`: ai | suggested | confirmed | edited | yours | rejected | dismissed.
    pub fn edge_state(&self, e: &Rec) -> &'static str {
        let b = bucket(e.get("status"));
        if truthy(e.get("_by_tristan")) {
            return "yours";
        }
        if let Some(t) = sget(e, "id").and_then(|id| self.idx.touch.get(id)) {
            if b == Bucket::Rejected {
                return "rejected";
            }
            if t.has("edit") {
                return "edited";
            }
            if b == Bucket::Confirmed {
                return "confirmed";
            }
        }
        if sget(e, "status") == Some("inferred") {
            return "ai";
        }
        match b {
            Bucket::Rejected => "dismissed",
            Bucket::Confirmed => "confirmed",
            Bucket::Proposed => "suggested",
        }
    }

    /// `deco`: an edge view plus its AI state.
    pub fn deco(&self, ev: &mut Map<String, Value>) {
        if ev.is_empty() {
            return;
        }
        let state = match sget(ev, "id").and_then(|id| self.v.edges.get(id)) {
            Some(e) => self.edge_state(e),
            None => self.edge_state(ev),
        };
        ev.insert("state".into(), json!(state));
        ev.insert("ai".into(), json!(state == "ai"));
    }

    fn deco_value(&self, ev: &mut Value) {
        if let Value::Object(m) = ev {
            self.deco(m);
        }
    }

    /// `stage_ai`: whether the stage is still the AI's, and what it said.
    pub fn stage_ai(&self, jid: &str) -> Value {
        let empty = Map::new();
        let n = self.v.nodes.get(jid).unwrap_or(&empty);
        let ai = n.get("_ai").filter(|a| truthy(Some(a)));
        let aim = ai.and_then(Value::as_object).unwrap_or(&empty);
        let t = self.idx.touch.get(jid);
        let fields_stage = t.is_some_and(|t| t.fields.contains("stage"));
        let confirmed = t.is_some_and(|t| t.has("confirm"));
        let on = ai.is_some()
            && n.get("stage").unwrap_or(&Value::Null) == aim.get("stage").unwrap_or(&Value::Null)
            && !fields_stage
            && !confirmed;
        let when = |k: &str| if on { get(aim, k) } else { Value::Null };
        json!({"stage_ai": on, "stage_confidence": when("stage_confidence"), "stage_reason_ai": when("stage_reason"),
               "from_stage": when("from_stage"), "stage_confirmed": confirmed && !fields_stage && ai.is_some()})
    }

    /// `DBAppIndex.joke_rooms`.
    pub fn joke_rooms(&self, jid: &str) -> Value {
        let row = self.idx.by_id.get(jid).map(|i| &self.idx.summaries[*i].summary);
        let (sets, hit, ai_hit) = row.map_or((0, 0, 0), |r| (r.sets, r.hit_sets, r.ai_hit_sets));
        let ready = self.d.working(jid).map_or(hit >= 3, |w| w.0);
        json!({"sets": sets, "hit_sets": hit, "ready_for_working": ready, "ai_hit_sets": ai_hit})
    }

    /// `brief_joke`.
    pub fn brief_joke(&self, jid: &str) -> Value {
        let empty = Map::new();
        let j = self.v.nodes.get(jid).unwrap_or(&empty);
        let title = match j.get("title") {
            Some(t) if truthy(Some(t)) => t.clone(),
            _ => json!(jid),
        };
        let rooms = self.joke_rooms(jid);
        let title_s = title.as_str().map_or_else(|| pk_core::py::str(&title), str::to_string);
        json!({"id": jid, "title": title, "stage": get(j, "stage"), "summary": or(j, "summary", json!("")),
               "short_name": short_name_of(&title_s, sget(j, "short_name")),
               "sets": rooms["sets"], "hit_sets": rooms["hit_sets"], "ai_hit_sets": rooms["ai_hit_sets"],
               "stage_ai": self.stage_ai(jid)["stage_ai"]})
    }

    /// `joke_text`: the newest version split so the reveal can be highlighted.
    pub fn joke_text(&self, jid: &str) -> Value {
        let empty = Map::new();
        let mut units: Vec<&Rec> = self
            .v
            .joke_units
            .get(jid)
            .into_iter()
            .flatten()
            .map(|(u, _)| self.v.nodes.get(u).unwrap_or(&empty))
            .filter(|u| truthy(u.get("text")))
            .collect();
        let Some(u) = ({
            units.sort_by_key(|u| {
                (
                    sget(u, "date").unwrap_or("").to_string(),
                    sget(u, "text").map_or(0, |t| t.chars().count()),
                )
            });
            units.last()
        }) else {
            let j = self.v.nodes.get(jid).unwrap_or(&empty);
            let src = [j.get("summary"), j.get("title")]
                .into_iter()
                .find(|x| truthy(*x))
                .flatten()
                .and_then(Value::as_str)
                .unwrap_or("");
            let (setup, reveal) = last_sentence_split(src);
            return json!({"setup": setup, "reveal": reveal, "from": null});
        };
        let text = sget(u, "text").unwrap_or("");
        let cs: Vec<char> = text.chars().collect();
        let shown = if cs.len() > 900 {
            let tail: String = cs[cs.len() - 900..].iter().collect();
            let rest = tail.split_once(' ').map_or(tail.as_str(), |(_, r)| r);
            format!("…{rest}")
        } else {
            text.to_string()
        };
        let (setup, reveal) = last_sentence_split(&shown);
        json!({"setup": setup, "reveal": reveal,
               "from": {"unit": get(u, "id"), "date": get(u, "date"), "doc_title": get(u, "doc_title")}})
    }

    /// `punch_of`: the line said just before the strongest laugh in the span.
    pub fn punch_of(&self, perf: &Rec) -> Map<String, Value> {
        let laughs = self.laughs(perf);
        let ls: &[Value] = laughs["laughs"].as_array().map_or(&[], Vec::as_slice);
        let (s0, s1) = (n0(perf.get("start_ms")), n0(perf.get("end_ms")));
        let segs: Vec<&Seg> = self
            .transcript(sget(perf, "set"))
            .iter()
            .filter(|s| s.end_ms as f64 > s0 && (s.start_ms as f64) < s1 + 500.0)
            .collect();
        // max(ls, key=(bars or 0, strength or 0)): the first largest
        let mut best: Option<(&Value, (f64, f64))> = None;
        for l in ls {
            let k = (n0(l.get("bars")), n0(l.get("strength")));
            if best.is_none_or(|(_, b)| k.partial_cmp(&b) == Some(std::cmp::Ordering::Greater)) {
                best = Some((l, k));
            }
        }
        let best = best.map(|(l, _)| l);
        let chosen: Vec<&Seg> = match best {
            Some(b) if !segs.is_empty() && b.as_object().is_some_and(|o| !o.is_empty()) => {
                let at = n0(b.get("start_ms"));
                let before: Vec<&Seg> = segs
                    .iter()
                    .copied()
                    .filter(|s| s.start_ms as f64 <= at + 300.0)
                    .collect();
                let mut chosen: Vec<&Seg> = if before.is_empty() {
                    segs[segs.len() - 1..].to_vec()
                } else {
                    before[before.len() - 1..].to_vec()
                };
                if chosen.first().is_some_and(|c| split_words(&c.text) < 6) && before.len() >= 2 {
                    chosen = before[before.len() - 2..].to_vec();
                }
                chosen
            }
            _ => segs.last().map(|s| vec![*s]).unwrap_or_default(),
        };
        let laugh = best.cloned().unwrap_or(Value::Null);
        let mut out = Map::new();
        let (Some(first), Some(last)) = (chosen.first(), chosen.last()) else {
            let (_, reveal) = last_sentence_split(sget(perf, "text").unwrap_or(""));
            out.insert("text".into(), json!(reveal));
            out.insert("start_ms".into(), num(s0));
            out.insert("end_ms".into(), num(s1));
            out.insert("laugh".into(), laugh);
            out.insert("segments".into(), json!([]));
            return out;
        };
        let text: Vec<&str> = chosen.iter().map(|c| c.text.as_str()).collect();
        out.insert("text".into(), json!(text.join(" ")));
        out.insert("start_ms".into(), json!(first.start_ms));
        out.insert("end_ms".into(), json!(last.end_ms));
        out.insert("laugh".into(), laugh);
        out.insert(
            "segments".into(),
            Value::Array(
                segs.iter()
                    .map(|s| {
                        // `s in chosen` compares the dicts as values
                        json!({"start_ms": s.start_ms, "end_ms": s.end_ms, "text": s.text,
                               "punch": chosen.iter().any(|c| c == s)})
                    })
                    .collect(),
            ),
        );
        out
    }

    /// `play` for a stretch: from 4 s before the punch, to 1.2 s after its laugh (at least 3 s).
    fn play(start: Option<&Value>, pu: &Map<String, Value>) -> Value {
        let from = n0(start).max(n0(pu.get("start_ms")) - 4000.0);
        let lend = pu.get("laugh").and_then(|l| l.get("end_ms"));
        let end = if truthy(lend) { n0(lend) } else { n0(pu.get("end_ms")) };
        json!({"start_ms": num(from), "end_ms": num((from + 3000.0).max(end + 1200.0))})
    }

    /// `set_info(sid)["reference"]` alone: where the set's recording lives.
    pub fn reference(&self, sid: &str) -> Value {
        let empty = Map::new();
        let d = set_dir(sid);
        let meta = d.as_ref().and_then(|d| self.d.metas.get(d)).unwrap_or(&empty);
        set_reference(meta, d.as_deref())
    }

    /// `set_info`: what a set card adds over the set node.
    pub fn set_info(&self, sid: &str) -> Map<String, Value> {
        let empty = Map::new();
        let s = self.v.nodes.get(sid).unwrap_or(&empty);
        let d = set_dir(sid);
        let meta = d.as_ref().and_then(|d| self.d.metas.get(d)).unwrap_or(&empty);
        let summ = self
            .d
            .ai
            .analysis
            .get(sid)
            .and_then(|m| m.get("summary"))
            .filter(|x| truthy(Some(x)))
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let his_date = s.get("show_date").filter(|x| truthy(Some(x)));
        let mut orig_venue = get(s, "venue");
        for (i, dd) in self.v.decisions.iter().enumerate() {
            if dd.id() == Some(sid) && !self.v.is_undone(i) && !dd.has_undo_of() && dd.after().contains_key("venue") {
                if let Some(b) = dd.before().get("venue") {
                    orig_venue = b.clone();
                }
                break;
            }
        }
        let raw = [Some(&orig_venue), meta.get("venue_guess")]
            .into_iter()
            .flatten()
            .find(|x| truthy(Some(x)))
            .map_or_else(
                || "Unknown".to_string(),
                |x| x.as_str().map_or_else(|| pk_core::py::str(x), str::to_string),
            );
        let show_date = his_date
            .or_else(|| meta.get("show_date").filter(|x| truthy(Some(x))))
            .cloned()
            .unwrap_or(Value::Null);
        let source = if his_date.is_some() {
            json!("yours")
        } else if truthy(meta.get("show_date")) {
            get(meta, "show_date_source")
        } else {
            Value::Null
        };
        let mut out = Map::new();
        out.insert("show_date".into(), show_date);
        out.insert("show_date_source".into(), source);
        out.insert("description".into(), Value::Null);
        out.insert("laugh_s_per_min".into(), get(summ, "laugh_s_per_min"));
        out.insert("room_heat".into(), get(summ, "room_heat"));
        out.insert(
            "audio_unreliable".into(),
            match s.get("audio_unreliable") {
                Some(Value::Bool(b)) => json!(b),
                _ => Value::Null,
            },
        );
        out.insert("venue_id".into(), json!(venue_id_of(&raw)));
        out.insert("source".into(), json!(set_source(meta, d.as_deref())));
        out.insert("reference".into(), set_reference(meta, d.as_deref()));
        out
    }

    /// `analysis_rows`: the AI's rows for a set, with his decisions on top; `None` when the set has no analysis.
    #[expect(
        clippy::too_many_lines,
        clippy::many_single_char_names,
        reason = "one row of the Python, field for field, with its short names"
    )]
    pub fn analysis_rows(&self, sid: &str) -> Option<Vec<Map<String, Value>>> {
        let rows = self.d.ai.rows.get(sid)?;
        let v = self.v;
        let empty = Map::new();
        let laughs: Vec<&Value> = doc_laughs(laughs_doc(&self.d.tape.laughs, sid))
            .iter()
            .filter(|l| matches!(l.get("segment"), None | Some(Value::Null)) || sget_v(l, "segment") == Some("set"))
            .collect();
        let mut saved: HashMap<String, &Rec> = HashMap::new();
        for jid in &v.jokes {
            let Some(j) = v.nodes.get(jid) else { continue };
            if let Some(f) = j.get("from").and_then(Value::as_object)
                && sget(f, "set") == Some(sid)
            {
                let key = match f.get("perf") {
                    Some(p) if truthy(Some(p)) => pk_core::py::str(p),
                    _ => format!("@{}", py_str_opt(f.get("start_ms"))),
                };
                saved.insert(key, j);
            }
        }
        let mut out = Vec::with_capacity(rows.len());
        for (row, pid) in rows {
            let row = row.as_object().unwrap_or(&empty);
            let pid = pid.as_deref();
            let p = self.node(pid).unwrap_or(&empty);
            let e = pid.and_then(|x| self.perf_edge(x));
            let live = e.is_some_and(|e| bucket(e.get("status")) != Bucket::Rejected);
            let jid = if live { pid.and_then(|x| self.joke_of(x)) } else { None };
            let t = pid.and_then(|x| self.idx.touch.get(x));
            let kind = sget(p, "kind");
            let status = match kind {
                Some(k) if NOT_COUNTED.contains(&k) => k,
                _ if jid.is_some() => "known",
                _ => "new",
            };
            let (rating, by) = perf_rating(p);
            let at = |k: &str| n0(if p.contains_key(k) { p.get(k) } else { row.get(k) });
            let (s0, s1) = (at("start_ms"), at("end_ms"));
            let pls: Vec<&Value> = row
                .get("punchlines")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|x| x.is_object() && truthy(x.get("text")))
                .collect();
            let ticks: Vec<Value> = laughs
                .iter()
                .filter(|l| {
                    l.get("start_ms")
                        .and_then(Value::as_f64)
                        .is_some_and(|a| s0 <= a && a <= s1 + 3000.0)
                })
                .map(|l| {
                    json!({"start_ms": l["start_ms"], "end_ms": l.get("end_ms").cloned().unwrap_or(Value::Null),
                                "bars": or0(l.get("bars"))})
                })
                .collect();
            let mut best: Option<(&Value, f64)> = None;
            for x in &pls {
                let b = n0(x.get("bars"));
                if best.is_none_or(|(_, bb)| b > bb) {
                    best = Some((x, b));
                }
            }
            let play = if let Some((b, _)) = best {
                let at_ms = if truthy(b.get("at_ms")) { n0(b.get("at_ms")) } else { s0 };
                let lid = b.get("laugh_id").filter(|x| !x.is_null());
                let hit = laughs
                    .iter()
                    .find(|l| l.get("id").filter(|x| !x.is_null()) == lid)
                    .and_then(|l| l.get("end_ms"));
                let lend = if truthy(hit) { n0(hit) } else { at_ms + 4000.0 };
                let pf = s0.max(at_ms - 4000.0);
                json!({"start_ms": num(pf), "end_ms": num((pf + 3000.0).max(lend + 1200.0))})
            } else {
                json!({"start_ms": num(s0), "end_ms": num(s1.max(s0 + 3000.0).min(s0 + 20000.0))})
            };
            let joke = jid.map(|j| self.brief_joke(j));
            let same_joke = jid.is_some() && jid == sget(row, "joke_id");
            let unit = if same_joke {
                let uid = sget(row, "version_unit_id").filter(|s| !s.is_empty()).unwrap_or("");
                v.nodes.get(uid)
            } else {
                None
            };
            let sim: Vec<Value> = row
                .get("similar")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|x| {
                    let o = x
                        .get("joke_id")
                        .and_then(Value::as_str)
                        .and_then(|id| v.nodes.get(id))?;
                    let oid = sget(o, "id");
                    (sget(o, "type") == Some("joke") && oid != jid).then(|| {
                        json!({"joke_id": get(o, "id"), "title": get(o, "title"), "stage": get(o, "stage"),
                               "why": x.get("why").filter(|w| truthy(Some(w))).cloned().unwrap_or(json!("")),
                               "confidence": x.get("confidence").cloned().unwrap_or(Value::Null)})
                    })
                })
                .collect();
            let scope = jid.map(|j| v.joke_scope(j)).unwrap_or_default();
            let mut hints = Vec::new();
            for x in row.get("hint_labels").and_then(Value::as_array).into_iter().flatten() {
                let raw = x.get("label_id").and_then(Value::as_str);
                let lid = raw.map(|l| v.alias.get(l).map_or(l, String::as_str));
                let Some(ln) = lid
                    .and_then(|l| v.nodes.get(l))
                    .filter(|n| sget(n, "type") == Some("label"))
                else {
                    continue;
                };
                let lid = lid.unwrap_or("");
                let he = v
                    .adj
                    .get(lid)
                    .into_iter()
                    .flatten()
                    .filter_map(|k| v.edges.get(k))
                    .find(|e| {
                        sget(e, "kind") == Some("has_label")
                            && sget(e, "src").is_some_and(|s| scope.iter().any(|x| x == s))
                    });
                hints.push(json!({
                    "label_id": lid, "name": get(ln, "name"), "kind": get(ln, "kind"),
                    "definition": or(ln, "definition", json!("")),
                    "why": x.get("why").filter(|w| truthy(Some(w))).cloned().unwrap_or(json!("")),
                    "edge": he.map_or(Value::Null, |he| json!({"id": get(he, "id"), "state": self.edge_state(he)})),
                }));
            }
            let sv = pid.and_then(|x| saved.get(x)).or_else(|| {
                saved.get(&format!(
                    "@{}",
                    py_num_str(if p.contains_key("start_ms") {
                        p.get("start_ms")
                    } else {
                        row.get("start_ms")
                    })
                ))
            });
            let suggested_title = if jid.is_some() || status != "new" {
                Value::Null
            } else if sget(row, "status") == Some("new") && truthy(row.get("title")) {
                get(row, "title")
            } else {
                let src = match pls.last() {
                    Some(l) => sget_v(l, "text").unwrap_or("").to_string(),
                    None => sget(row, "transcript").unwrap_or("").chars().take(80).collect(),
                };
                json!(title_from(&src))
            };
            let title = match &joke {
                Some(j) => j["title"].clone(),
                None if NOT_COUNTED.contains(&status) && truthy(p.get("name")) => get(p, "name"),
                None => json!("New material"),
            };
            let mut m = Map::new();
            m.insert("idx".into(), get(row, "idx"));
            m.insert("perf".into(), pid.map_or(Value::Null, |x| json!(x)));
            m.insert("start_ms".into(), num(s0));
            m.insert("end_ms".into(), num(s1));
            m.insert("at_label".into(), at_label(Some(&num(s0))));
            m.insert("status".into(), json!(status));
            m.insert("ai_status".into(), get(row, "status"));
            m.insert(
                "kind_by".into(),
                json!(if t.is_some_and(|t| t.fields.contains("kind")) {
                    "you"
                } else {
                    "ai"
                }),
            );
            m.insert("title".into(), title);
            m.insert("suggested_title".into(), suggested_title);
            m.insert("joke".into(), joke.unwrap_or(Value::Null));
            m.insert(
                "match".into(),
                match e.filter(|_| live) {
                    Some(e) => json!({"edge": get(e, "id"), "state": self.edge_state(e),
                                      "confidence": get(e, "confidence"), "reason": or(e, "reason", json!(""))}),
                    None => Value::Null,
                },
            );
            m.insert("match_reason".into(), or(row, "match_reason", json!("")));
            m.insert("match_confidence".into(), get(row, "match_confidence"));
            m.insert(
                "version".into(),
                match unit.filter(|u| sget(u, "type") == Some("unit")) {
                    Some(u) => {
                        let text: String = sget(u, "text").unwrap_or("").chars().take(400).collect();
                        json!({"id": get(u, "id"), "date": get(u, "date"), "doc_title": get(u, "doc_title"), "text": text})
                    }
                    None => Value::Null,
                },
            );
            m.insert("rating".into(), rating);
            m.insert("rating_by".into(), by);
            m.insert(
                "ai_rating".into(),
                if truthy(p.get("ai_rating")) {
                    get(p, "ai_rating")
                } else {
                    get(row, "ai_rating")
                },
            );
            m.insert("rating_basis".into(), or(row, "rating_basis", json!("")));
            let laughs_v = if truthy(row.get("laughs")) {
                get(row, "laughs")
            } else {
                json!({"count": ticks.len(), "max_bars": max_by_num(ticks.iter().map(|x| x["bars"].clone())),
                       "total_s": 0})
            };
            m.insert("laughs".into(), laughs_v);
            m.insert("ticks".into(), Value::Array(ticks));
            m.insert("punchlines".into(), Value::Array(pls.into_iter().cloned().collect()));
            m.insert("play".into(), play);
            m.insert("similar".into(), Value::Array(sim));
            m.insert("hints".into(), Value::Array(hints));
            m.insert(
                "script_diff".into(),
                if same_joke {
                    get(row, "script_diff")
                } else {
                    Value::Null
                },
            );
            m.insert(
                "transcript".into(),
                [row.get("transcript"), p.get("text")]
                    .into_iter()
                    .find(|x| truthy(*x))
                    .flatten()
                    .cloned()
                    .unwrap_or(json!("")),
            );
            m.insert(
                "saved_as".into(),
                sv.map_or(Value::Null, |j| json!({"id": get(j, "id"), "title": get(j, "title")})),
            );
            for k in ROW_PASS {
                m.insert(k.into(), get(row, k));
            }
            out.push(m);
        }
        Some(out)
    }

    /// `_summary`.
    fn summary(&self, rows: &[Map<String, Value>], meta: Option<&Value>, sid: &str) -> Map<String, Value> {
        let empty = Map::new();
        let ms = meta
            .and_then(|m| m.get("summary"))
            .filter(|x| truthy(Some(x)))
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let key = |r: &Map<String, Value>| {
            let rt = r.get("rating").and_then(Value::as_str).unwrap_or("");
            let l = r.get("laughs");
            (
                rank(rt) as f64,
                n0(l.and_then(|l| l.get("max_bars"))),
                n0(l.and_then(|l| l.get("total_s"))),
            )
        };
        let gt = |a: (f64, f64, f64), b: (f64, f64, f64)| a.partial_cmp(&b) == Some(std::cmp::Ordering::Greater);
        let known: Vec<usize> = (0..rows.len())
            .filter(|i| sget(&rows[*i], "status") == Some("known"))
            .collect();
        let (mut best, mut worst): (Option<usize>, Option<usize>) = (None, None);
        for &i in &known {
            if best.is_none_or(|b| gt(key(&rows[i]), key(&rows[b]))) {
                best = Some(i);
            }
            if worst.is_none_or(|w| gt(key(&rows[w]), key(&rows[i]))) {
                worst = Some(i);
            }
        }
        let s = self.v.nodes.get(sid).unwrap_or(&empty);
        let dur = [ms.get("duration_s"), s.get("duration_s")]
            .into_iter()
            .find(|x| truthy(*x))
            .flatten()
            .cloned()
            .unwrap_or(json!(0));
        let nl = match ms.get("laughs") {
            Some(x) if !x.is_null() => x.clone(),
            _ => json!(
                doc_laughs(laughs_doc(&self.d.tape.laughs, sid))
                    .iter()
                    .filter(
                        |l| matches!(l.get("segment"), None | Some(Value::Null)) || sget_v(l, "segment") == Some("set")
                    )
                    .count()
            ),
        };
        let count = |st: &[&str]| {
            rows.iter()
                .filter(|r| sget(r, "status").is_some_and(|x| st.contains(&x)))
                .count()
        };
        let lpm = match ms.get("laughs_per_min") {
            Some(x) if !x.is_null() => x.clone(),
            _ => {
                let minutes = dur.as_f64().unwrap_or(0.0) / 60.0;
                json!(r2(nl.as_f64().unwrap_or(0.0) / minutes.max(1.0)))
            }
        };
        let pick = |i: Option<usize>| {
            i.map_or(Value::Null, |i| {
                let r = &rows[i];
                json!({"id": r["joke"]["id"], "title": r["joke"]["title"], "idx": get(r, "idx"), "rating": get(r, "rating")})
            })
        };
        let mut out = Map::new();
        out.insert("duration_s".into(), dur);
        out.insert("jokes_counted".into(), json!(count(&["known", "new"])));
        out.insert("known_jokes".into(), json!(known.len()));
        out.insert("new_material".into(), json!(count(&["new"])));
        out.insert("crowd_work".into(), json!(count(&["crowd_work"])));
        out.insert("banter".into(), json!(count(&["banter"])));
        out.insert("laughs".into(), nl);
        out.insert("laughs_per_min".into(), lpm);
        out.insert("best".into(), pick(best));
        out.insert("weakest".into(), pick(worst.filter(|w| Some(*w) != best)));
        for k in SUMMARY_PASS {
            out.insert(k.into(), get(ms, k));
        }
        out
    }

    /// `analysis_brief`: the card's line about the AI's reading.
    pub fn analysis_brief(&self, sid: &str) -> Value {
        let meta = self.d.ai.analysis.get(sid);
        let Some(rows) = self.analysis_rows(sid) else {
            return Value::Null;
        };
        let sm = self.summary(&rows, meta, sid);
        let m = |k: &str| meta.and_then(|m| m.get(k)).cloned().unwrap_or(Value::Null);
        json!({"source": m("source"), "run_id": m("run_id"), "generated_at": m("generated_at"),
               "jokes_counted": sm["jokes_counted"], "new_material": sm["new_material"], "crowd_work": sm["crowd_work"],
               "best": sm["best"], "laughs_per_min": sm["laughs_per_min"]})
    }

    /// `GET /api/sets/<id>/analysis`.
    pub fn set_analysis(&self, sid: &str) -> Option<Value> {
        let s = self.v.nodes.get(sid)?;
        let rows = self.analysis_rows(sid);
        let pick = json!({"id": get(s, "id"), "title": get(s, "title"), "date": get(s, "date"), "venue": get(s, "venue"),
                          "duration_s": get(s, "duration_s")});
        let Some(rows) = rows else {
            return Some(
                json!({"set": pick, "source": null, "rows": [], "summary": null, "laughs": [], "audio": null}),
            );
        };
        let meta = self.d.ai.analysis.get(sid);
        let m = |k: &str| meta.and_then(|m| m.get(k)).cloned().unwrap_or(Value::Null);
        let laughs: Vec<Value> = doc_laughs(laughs_doc(&self.d.tape.laughs, sid))
            .iter()
            .filter(|l| matches!(l.get("segment"), None | Some(Value::Null)) || sget_v(l, "segment") == Some("set"))
            .map(|l| {
                json!({"start_ms": l.get("start_ms").cloned().unwrap_or(Value::Null),
                            "end_ms": l.get("end_ms").cloned().unwrap_or(Value::Null), "bars": or0(l.get("bars"))})
            })
            .collect();
        let summary = self.summary(&rows, meta, sid);
        Some(
            json!({"set": pick, "source": m("source"), "run_id": m("run_id"), "generated_at": m("generated_at"),
                    "summary": summary, "rows": rows, "laughs": laughs, "audio": null,
                    "reference": self.reference(sid)}),
        )
    }

    /// `set_card`.
    pub fn set_card(&self, sid: &str) -> Value {
        let empty = Map::new();
        let s = self.v.nodes.get(sid).unwrap_or(&empty);
        let perfs: Vec<&Rec> = self
            .idx
            .set_perfs
            .get(sid)
            .into_iter()
            .flatten()
            .filter_map(|p| self.v.nodes.get(p))
            .collect();
        let pid = |p: &Rec| sget(p, "id").unwrap_or("").to_string();
        let counted = |p: &&&Rec| !sget(p, "kind").is_some_and(|k| NOT_COUNTED.contains(&k));
        let rateable: Vec<&&Rec> = perfs
            .iter()
            .filter(counted)
            .filter(|p| self.joke_of(&pid(p)).is_some())
            .collect();
        let rated = rateable.iter().filter(|p| truthy(p.get("rating"))).count();
        let laughs = doc_laughs(laughs_doc(&self.d.tape.laughs, sid)).len();
        let info = self.set_info(sid);
        let when = when_label(
            [info.get("show_date"), s.get("date")]
                .into_iter()
                .find(|x| truthy(*x))
                .flatten(),
        );
        let (venue_short, venue_guessed) = venue_label(sget(s, "venue"));
        let jokes: Vec<Value> = perfs
            .iter()
            .map(|p| {
                let id = pid(p);
                let live = self.live_joke(&id);
                let j = live.and_then(|j| self.v.nodes.get(j));
                let title = match live {
                    Some(_) => j.map_or(Value::Null, |j| get(j, "title")),
                    None => or(p, "name", json!("New material")),
                };
                let short = match j {
                    Some(j) => {
                        let t = [j.get("title"), j.get("id")]
                            .into_iter()
                            .find(|x| truthy(*x))
                            .flatten()
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        short_name_of(t, sget(j, "short_name"))
                    }
                    None => short_name_of(title.as_str().unwrap_or(""), None),
                };
                let (rating, by) = perf_rating(p);
                json!({"perf": id, "title": title, "short_name": short, "joke": live, "kind": get(p, "kind"),
                       "rating": rating, "rating_by": by, "start_ms": get(p, "start_ms"),
                       "at_label": at_label(p.get("start_ms"))})
            })
            .collect();
        let mut out = Map::new();
        out.insert("id".into(), json!(sid));
        out.insert("title".into(), get(s, "title"));
        out.insert("date".into(), get(s, "date"));
        out.insert("venue".into(), get(s, "venue"));
        out.extend(info);
        out.insert("venue_short".into(), json!(venue_short));
        out.insert("venue_guessed".into(), json!(venue_guessed));
        out.insert("when_label".into(), json!(when));
        out.insert("duration_s".into(), get(s, "duration_s"));
        out.insert("performances".into(), json!(perfs.len()));
        out.insert("rateable".into(), json!(rateable.len()));
        out.insert("rated".into(), json!(rated));
        out.insert(
            "reviewed".into(),
            json!(!rateable.is_empty() && rated == rateable.len()),
        );
        out.insert("laughs".into(), json!(laughs));
        out.insert("has_audio".into(), json!(false));
        out.insert("analysis".into(), self.analysis_brief(sid));
        out.insert("jokes".into(), Value::Array(jokes));
        Value::Object(out)
    }

    /// `review_steps`: one step per performance, then the laughs nobody's joke explains, in tape order.
    pub fn review_steps(&self, sid: &str) -> Vec<Value> {
        let perfs: Vec<&Rec> = self
            .idx
            .set_perfs
            .get(sid)
            .into_iter()
            .flatten()
            .filter_map(|p| self.v.nodes.get(p))
            .collect();
        let mut covered: Vec<(f64, f64)> = perfs
            .iter()
            .map(|p| (n0(p.get("start_ms")), n0(p.get("end_ms")) + 3000.0))
            .collect();
        // (sort key, step)
        let mut steps: Vec<(f64, Value)> = Vec::new();
        for p in &perfs {
            let id = sget(p, "id").unwrap_or("");
            let e = self.perf_edge(id);
            let jid = self.joke_of(id);
            let laughs = self.laughs(p);
            let pu = self.punch_of(p);
            let kind = if sget(p, "kind").is_some_and(|k| NOT_COUNTED.contains(&k)) {
                "crowd_work"
            } else if jid.is_some() && e.is_some_and(|e| bucket(e.get("status")) != Bucket::Rejected) {
                "joke"
            } else {
                "new_line"
            };
            let play = Self::play(p.get("start_ms"), &pu);
            let m = e.map_or(Value::Null, |e| {
                let mut ev = self.v.edge_view(e, None);
                self.deco(&mut ev);
                Value::Object(ev)
            });
            let max_bars = laughs["max_bars"].clone();
            let key = if kind == "new_line" {
                n0(pu.get("start_ms"))
            } else {
                n0(p.get("start_ms"))
            };
            steps.push((
                key,
                json!({"key": id, "kind": kind, "perf": public(p), "name": get(p, "name"),
                       "joke": jid.map_or(Value::Null, |j| self.brief_joke(j)), "match": m,
                       "text": or(p, "text", json!("")), "punch": pu,
                       "laughs": laughs["laughs"], "max_bars": max_bars, "hits": laughs["hits"],
                       "suggested": suggest_rating(max_bars.as_f64().unwrap_or(0.0)), "rating": get(p, "rating"),
                       "ai_rating": get(p, "ai_rating"), "play": play}),
            ));
        }
        let segs = self.transcript(Some(sid));
        for l in doc_laughs(laughs_doc(&self.d.tape.laughs, sid)) {
            let bars = n0(l.get("bars"));
            let seg_ok = matches!(l.get("segment"), None | Some(Value::Null)) || sget_v(l, "segment") == Some("set");
            if bars < 2.0 || !seg_ok {
                continue;
            }
            let at = n0(l.get("start_ms"));
            if covered.iter().any(|(a0, a1)| *a0 <= at && at <= *a1) {
                continue;
            }
            let before: Vec<&Seg> = segs
                .iter()
                .filter(|s| (s.end_ms as f64) <= at + 500.0 && (s.end_ms as f64) >= at - 8000.0)
                .collect();
            let Some(lastb) = before.last() else { continue };
            let chosen: &[&Seg] = if split_words(&lastb.text) < 6 && before.len() >= 2 {
                &before[before.len() - 2..]
            } else {
                &before[before.len() - 1..]
            };
            let text = chosen.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");
            let (start, end) = (chosen[0].start_ms, chosen[chosen.len() - 1].end_ms);
            covered.push((start as f64, n0(l.get("end_ms"))));
            let lend = l
                .get("end_ms")
                .filter(|x| truthy(Some(x)))
                .and_then(Value::as_f64)
                .unwrap_or(end as f64);
            steps.push((
                start as f64,
                json!({"key": format!("line:{sid}@{start}"), "kind": "new_line", "perf": null, "name": null,
                       "joke": null, "match": null, "text": text,
                       "punch": {"text": text, "start_ms": start, "end_ms": end, "laugh": l,
                                 "segments": chosen.iter().map(|s| json!({"start_ms": s.start_ms, "end_ms": s.end_ms,
                                                                          "text": s.text, "punch": true}))
                                                   .collect::<Vec<_>>()},
                       "laughs": [l], "max_bars": or0(l.get("bars")), "hits": 1,
                       "suggested": suggest_rating(bars), "rating": null,
                       "play": {"start_ms": (start - 2000).max(0), "end_ms": num(lend + 1200.0)}}),
            ));
        }
        steps.sort_by(|a, b| a.0.total_cmp(&b.0));
        steps.into_iter().map(|(_, s)| s).collect()
    }

    /// `Handler.joke`: one bit's page; `None` when there is no such joke.
    pub fn joke_page(&self, jid: &str) -> Option<Value> {
        let v = self.v;
        let d = v.joke_detail(jid, &Laughs(self.d))?;
        let empty = Map::new();
        let mut perfs: Vec<Map<String, Value>> = Vec::new();
        for p in d.get("performances").and_then(Value::as_array).into_iter().flatten() {
            let pid = p["perf"].get("id").and_then(Value::as_str);
            let node = self.node(pid).unwrap_or(&empty);
            let mut edge = p["edge"].clone();
            self.deco_value(&mut edge);
            let laughs = if truthy(p.get("laughs")) {
                p["laughs"].clone()
            } else {
                no_laughs()
            };
            let pu = self.punch_of(node);
            let (eff, by) = perf_rating(node);
            let set_id = p["set"].get("id").filter(|x| truthy(Some(x))).and_then(Value::as_str);
            let max_bars = laughs["max_bars"].clone();
            let mut m = Map::new();
            m.insert("perf".into(), p["perf"].clone());
            m.insert("set".into(), p["set"].clone());
            m.insert("edge".into(), edge);
            m.insert("laughs".into(), laughs["laughs"].clone());
            m.insert("max_bars".into(), max_bars.clone());
            m.insert("hits".into(), laughs["hits"].clone());
            m.insert("rating".into(), get(node, "rating"));
            m.insert("ai_rating".into(), get(node, "ai_rating"));
            m.insert("rating_basis".into(), or(node, "rating_basis", json!("")));
            m.insert("effective_rating".into(), eff);
            m.insert("rating_by".into(), by);
            m.insert("kind".into(), get(node, "kind"));
            m.insert(
                "suggested".into(),
                json!(suggest_rating(max_bars.as_f64().unwrap_or(0.0))),
            );
            m.insert(
                "punch".into(),
                json!({"text": pu["text"], "start_ms": pu["start_ms"], "end_ms": pu["end_ms"]}),
            );
            m.insert("play".into(), Self::play(node.get("start_ms"), &pu));
            m.insert("audio".into(), Value::Null);
            m.insert("reference".into(), set_id.map_or(Value::Null, |s| self.reference(s)));
            perfs.push(m);
        }
        let mut edges: Vec<Value> = d.get("edges").and_then(Value::as_array).cloned().unwrap_or_default();
        for e in &mut edges {
            self.deco_value(e);
        }
        let mut groups = d.get("unit_groups").cloned().unwrap_or(json!([]));
        for g in groups.as_array_mut().into_iter().flatten() {
            for u in g.get_mut("units").and_then(Value::as_array_mut).into_iter().flatten() {
                if let Some(e) = u.get_mut("edge") {
                    self.deco_value(e);
                }
            }
        }
        let order = |s: Option<&str>| match s {
            Some("ai") => 0,
            Some("edited" | "yours" | "confirmed") => 1,
            Some("rejected" | "dismissed") => 3,
            _ => 2,
        };
        let key = |e: &Value| (order(e.get("state").and_then(Value::as_str)), -n0(e.get("confidence")));
        let by_key = |a: &Value, b: &Value| {
            let (ka, kb) = (key(a), key(b));
            ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
        };
        let (mut labels, mut links): (Vec<Value>, Vec<Value>) = edges
            .into_iter()
            .partition(|e| e.get("kind").and_then(Value::as_str) == Some("has_label"));
        labels.sort_by(by_key);
        links.sort_by(by_key);
        let ai_ratings: Vec<Value> = perfs
            .iter()
            .filter(|p| {
                p["edge"].get("bucket").and_then(Value::as_str) != Some("rejected")
                    && !p["kind"].as_str().is_some_and(|k| NOT_COUNTED.contains(&k))
            })
            .map(|p| {
                let s = &p["set"];
                let (vs, vg) = venue_label(s.get("venue").and_then(Value::as_str));
                json!({"set": s.get("id"), "date": s.get("date"), "title": s.get("title"), "perf": p["perf"].get("id"),
                       "when_label": when_label(s.get("date")), "venue_short": vs, "venue_guessed": vg,
                       "rating": p["effective_rating"], "rating_by": p["rating_by"], "ai_rating": p["ai_rating"],
                       "max_bars": p["max_bars"]})
            })
            .collect();
        let yours = d.get("yours") == Some(&Value::Bool(true));
        Some(json!({
            "joke": d.get("joke"), "yours": yours, "text": self.joke_text(jid), "rooms": self.joke_rooms(jid),
            "ai": self.stage_ai(jid),
            "who": if yours { "edited" } else { self.idx.who.get(jid).copied().unwrap_or("ai") },
            "ai_ratings": ai_ratings, "versions": groups, "performances": perfs, "labels": labels, "links": links,
            "history": d.get("history"),
        }))
    }
}

fn sget_v<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

/// `f"{x}"` of an optional value (`None` for a missing key).
fn py_str_opt(v: Option<&Value>) -> String {
    v.map_or_else(|| "None".to_string(), pk_core::py::str)
}

/// `f"@{s0}"` where `s0 = value or 0`: the value as written when it is truthy, else `0`.
fn py_num_str(v: Option<&Value>) -> String {
    match v {
        Some(x) if truthy(Some(x)) => pk_core::py::str(x),
        _ => "0".into(),
    }
}

/// Every set folder whose transcript `review_steps` / `set_card` of `sid` reads.
pub fn set_dirs_of(sid: &str) -> Vec<String> {
    set_dir(sid).into_iter().collect()
}

/// Shared: compute in a blocking task over owned handles.
pub async fn compute<T: Send + 'static>(
    idx: Arc<AppIndex>,
    d: Arc<Data>,
    tx: Transcripts,
    f: impl FnOnce(&Ctx<'_>) -> T + Send + 'static,
) -> Result<T, crate::error::ApiError> {
    tokio::task::spawn_blocking(move || f(&Ctx::new(&idx, &d, &tx)))
        .await
        .map_err(|e| crate::error::ApiError::internal("JoinError", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_from_words() {
        assert_eq!(
            title_from("Well, it's the dog -- again! and more words here"),
            "Well it's the dog again and"
        );
        assert_eq!(title_from("!!"), "New line");
    }

    #[test]
    fn sources_and_references() {
        let m = |v: Value| v.as_object().cloned().unwrap();
        let yt = m(json!({"id": "abcdefghijk", "upload_date": "20240101", "title": "T"}));
        assert_eq!(set_source(&yt, Some("20240101_abcdefghijk")), "youtube");
        assert_eq!(set_source(&yt, Some("20240101_other")), "tape");
        assert_eq!(set_reference(&yt, Some("20240101_abcdefghijk"))["kind"], "youtube");
        let up = m(json!({"source": "app_upload", "original_name": "a.m4a"}));
        assert_eq!(
            set_reference(&up, None),
            json!({"kind": "file", "name": "a.m4a", "recorded_at": null})
        );
        assert_eq!(set_reference(&m(json!({})), None), Value::Null);
    }
}
