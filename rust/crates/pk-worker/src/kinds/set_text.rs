//! `set_text` (`worker/execute.py` `analyze_transcript` + `worker/online.py` `set_text_job`): a set transcribed on
//! the phone. The engine's split step (`engine/inbox.py` `Inbox.split`, task `split` on Opus, 220-line windows,
//! spans clipped, intro and outro dropped) over the phone's segments; `corpus/sets/<set>/{transcript,meta,analysis}
//! .json` written with every audio-derived field absent and `audio.status` "unavailable"; every funny unit posted as
//! an idea (`ai:set_text`) so a bit from a phone-transcribed night is never lost. Court text stops it all.
//!
//! The windows run one after another, as in the Python: the budget is checked before each call, and running them
//! together would let one long set overshoot the day's cap by a window.

use std::sync::LazyLock;

use chrono::{Local, NaiveDateTime};
use pk_core::py;
use pk_creative::ask::words::py_word_count;
use pk_llm::ChatMessage;
use pk_wire::worker::SetTextInput;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::{Ctx, Done, Kind, OutFile, Runner};
use crate::error::JobError;
use crate::pyjson::dumps_indent1;

/// The task in `engine/config.hosted.toml`.
pub const TASK: &str = "split";
/// `SPLIT_WINDOW`: transcript lines per model call.
pub const SPLIT_WINDOW: usize = 220;
/// `TRANSCRIPT_MAX_CHARS`.
pub const TRANSCRIPT_MAX_CHARS: usize = 400_000;
/// `WORDS_PER_S`: synthetic timestamps when the phone sent none.
pub const WORDS_PER_S: f64 = 2.5;
/// `SET_TEXT_IDEA_MAX`: a unit longer than this is not posted as an idea.
pub const IDEA_MAX: usize = 4000;

static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").expect("a valid regex"));

/// `S_SPLIT`.
pub static SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({"type": "object", "properties": {"units": {"type": "array", "items": {
        "type": "object", "properties": {
            "first": {"type": "integer"}, "last": {"type": "integer"},
            "kind": {"type": "string", "enum": ["bit", "idea", "crowd_work", "banter", "intro", "outro", "note"]},
            "name": {"type": "string"}, "funny": {"type": "boolean"}},
        "required": ["first", "last", "kind", "name", "funny"], "additionalProperties": false}}},
        "required": ["units"], "additionalProperties": false})
});

/// `SYS_VOICE`: the split step's system line. `name` is `PK_COMIC_NAME` as set (unstripped), "the comic" when unset.
pub fn sys_voice(name: &str) -> String {
    let owner = if name == "the comic" {
        "a stand-up comic's".to_string()
    } else {
        format!("stand-up comic {name}'s")
    };
    format!(
        "You work on {owner} own material. Be literal and exact. Every quote you give must be copied character for \
         character from the text it names; never paraphrase a quote. Reasons are one plain sentence a comic would \
         understand."
    )
}

/// One transcript line.
#[derive(Debug, Clone, PartialEq)]
pub struct Seg {
    /// Seconds.
    pub start: f64,
    /// Seconds.
    pub end: f64,
    /// The words.
    pub text: String,
}

/// `phone_transcript(inp)`: the segments as the engine's transcript (`device` timestamps when every segment has a
/// start and an end, else `synthetic`, word-count based and monotone) and the `model` line.
pub fn phone_transcript(inp: &SetTextInput) -> Result<(Vec<Seg>, &'static str, String), JobError> {
    if inp.segments.is_empty() {
        return Err(JobError::Value(
            "set_text needs segments: [{text, start?, end?}]".into(),
        ));
    }
    let timed = inp
        .segments
        .iter()
        .all(|s| matches!((s.start, s.end), (Some(a), Some(b)) if b >= a));
    let (mut out, mut t, mut chars) = (Vec::new(), 0.0_f64, 0usize);
    for s in &inp.segments {
        let text = py::strip(&s.text);
        if text.is_empty() {
            continue;
        }
        chars += text.chars().count();
        if chars > TRANSCRIPT_MAX_CHARS {
            return Err(JobError::Value("transcript too long".into()));
        }
        let (a, b) = if let (true, Some(a), Some(b)) = (timed, s.start, s.end) {
            (a, b)
        } else {
            let a = t;
            let b = t + (py_word_count(text) as f64 / WORDS_PER_S).max(0.5);
            t = b;
            (a, b)
        };
        out.push(Seg {
            start: a,
            end: b,
            text: text.to_string(),
        });
    }
    if out.is_empty() {
        return Err(JobError::Value("set_text: every segment is empty".into()));
    }
    let engine = if inp.engine.is_empty() {
        "speech"
    } else {
        inp.engine.as_str()
    };
    let model = format!("phone:{}", engine.chars().take(40).collect::<String>());
    Ok((out, if timed { "device" } else { "synthetic" }, model))
}

fn segments_json(segs: &[Seg]) -> Value {
    Value::Array(
        segs.iter()
            .map(|s| json!({"start": s.start, "end": s.end, "text": s.text, "words": []}))
            .collect(),
    )
}

/// The `[i] (mm:ss) text` line of a window's listing (`int(start // 60):02d`, `int(start % 60):02d`).
pub fn listing_line(i: usize, s: &Seg) -> String {
    let m = (s.start / 60.0).floor() as i64;
    let sec = s.start.rem_euclid(60.0) as i64;
    format!("[{i}] ({m:02}:{sec:02}) {}", py::strip(&s.text))
}

#[derive(Debug, Deserialize)]
struct Span {
    first: Value,
    last: Value,
    kind: String,
    name: String,
    funny: bool,
}

fn int_of(v: &Value) -> i64 {
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).unwrap_or(0)
}

/// One unit the split step found.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    /// `perf:<set>@<start_ms>`.
    pub id: String,
    /// The joined text.
    pub text: String,
    /// The model's kind.
    pub kind: String,
    /// The name, stripped, 80 characters.
    pub name: String,
    /// Comedic material.
    pub funny: bool,
    /// Start.
    pub start_ms: i64,
    /// End.
    pub end_ms: i64,
}

/// The split step's assembly of one window's answer (`Inbox.split` after the model call).
pub fn assemble(segs: &[Seg], w0: usize, win_len: usize, data: &Value, set_dir: &str, out: &mut Vec<Unit>) {
    let mut spans: Vec<Span> = data
        .get("units")
        .and_then(|u| serde_json::from_value(u.clone()).ok())
        .unwrap_or_default();
    spans.sort_by_key(|u| int_of(&u.first));
    let w0i = i64::try_from(w0).unwrap_or(i64::MAX);
    let mut last_end = w0i - 1;
    for u in spans {
        let a = int_of(&u.first).max(last_end + 1).max(w0i);
        let b = int_of(&u.last).min(w0i + i64::try_from(win_len).unwrap_or(0) - 1);
        if b < a {
            continue;
        }
        last_end = b;
        if u.kind == "intro" || u.kind == "outro" {
            continue;
        }
        let piece = &segs[a as usize..=b as usize];
        let text: Vec<&str> = piece.iter().map(|s| py::strip(&s.text)).collect();
        let start_ms = (piece[0].start * 1000.0) as i64;
        let end_ms = (piece[piece.len() - 1].end * 1000.0) as i64;
        out.push(Unit {
            id: format!("perf:{set_dir}@{start_ms}"),
            text: text.join(" "),
            kind: u.kind,
            name: py::strip(&u.name).chars().take(80).collect(),
            funny: u.funny,
            start_ms,
            end_ms,
        });
    }
}

/// The `set_text` kind.
#[derive(Debug)]
pub struct SetText;

impl Runner for SetText {
    type Input = SetTextInput;

    fn bad_input(e: &serde_json::Error) -> JobError {
        JobError::Value(format!("bad set_text input: {e}"))
    }

    async fn run(ctx: Ctx<'_>, inp: SetTextInput) -> Result<Done, JobError> {
        run_at(ctx, &inp, Local::now().naive_local()).await
    }
}

/// [`SetText`] at a given local time (the run id and the date when the phone sent none).
pub async fn run_at(ctx: Ctx<'_>, inp: &SetTextInput, now: NaiveDateTime) -> Result<Done, JobError> {
    let (segs, timestamps, model) = phone_transcript(inp)?;
    let seg_json = segments_json(&segs);
    let sha = hex::encode(Sha256::digest(py::dumps_sorted(&seg_json).as_bytes()));
    let recorded = inp.recorded_at.as_deref().unwrap_or("");
    let mut date: String = recorded.chars().take(10).collect();
    if !DATE.is_match(&date) {
        date = now.format("%Y-%m-%d").to_string();
    }
    let set_dir = format!("{}_ph{}", date.replace('-', ""), &sha[..9]);
    let title: String = match inp.title.as_deref() {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => format!("Set {date} (phone)"),
    }
    .chars()
    .take(200)
    .collect();
    let run_id = format!("settext-{}-{}", now.format("%Y%m%d-%H%M%S"), &sha[..8]);

    // the split step, window by window
    let system = sys_voice(&ctx.names.raw);
    let mut units = Vec::new();
    for w0 in (0..segs.len()).step_by(SPLIT_WINDOW) {
        let win = &segs[w0..segs.len().min(w0 + SPLIT_WINDOW)];
        let listing: Vec<String> = win.iter().enumerate().map(|(i, s)| listing_line(w0 + i, s)).collect();
        let user = format!(
            "This is the transcript of a recorded stand-up SET. Units are: bit (a joke or bit he performs), \
             crowd_work, banter (filler, transitions), intro (host/walk-on), outro, one numbered line each.\n\
             Split it into consecutive units. Each unit covers lines first..last (inclusive) and units must not \
             overlap. A bit is ONE unit including its tags, act-outs and asides; start a new unit only when he moves \
             to a different premise (watch for the turn: 'anyway', 'so', a new subject). intro = only the host \
             introducing him / walk-on applause: his own first lines are material, even quick openers. outro = only \
             his sign-off ('that's my time', socials). name = a short title for the unit (3-7 words). funny = true \
             if it is comedic material.\n\n{}",
            listing.join("\n")
        );
        let msgs = [ChatMessage::system(system.clone()), ChatMessage::user(user)];
        let c = ctx
            .router
            .complete(TASK, &msgs, Some(&SCHEMA), Some(&run_id), None)
            .await?;
        assemble(
            &segs,
            w0,
            win.len(),
            &c.data.unwrap_or(Value::Null),
            &set_dir,
            &mut units,
        );
    }

    let dur = inp.duration_s.filter(|d| *d != 0.0).unwrap_or(segs[segs.len() - 1].end);
    let rows: Vec<Value> = units
        .iter()
        .map(|u| {
            json!({"id": u.id, "name": u.name, "kind": u.kind, "text": u.text, "funny": u.funny,
                   "start_ms": u.start_ms, "end_ms": u.end_ms, "laugh_s": null, "laugh_s_per_min": null,
                   "pct_laughing": null, "vs_room": null, "time_to_first_laugh_s": null, "dead_air_max_s": null})
        })
        .collect();
    let funny_units = units.iter().filter(|u| u.funny).count();
    let audio = json!({"status": "unavailable",
                       "why": "transcribed on the phone; laughs, room heat and timing need the recording",
                       "timestamps": timestamps});
    let tx = json!({"model": model, "segments": seg_json, "timestamps": timestamps});
    let mut meta = Map::new();
    let mut put = |k: &str, v: Value| {
        if !v.is_null() {
            meta.insert(k.to_string(), v);
        }
    };
    put("id", json!(format!("ph{}", &sha[..9])));
    put("title", json!(title));
    put("source", json!("phone_transcript"));
    put("recorded_at", json!(inp.recorded_at));
    put("show_date", json!(date));
    put(
        "show_date_source",
        json!(if inp.recorded_at.as_deref().is_some_and(|r| !r.is_empty()) {
            "the phone (when it was recorded)"
        } else {
            "today"
        }),
    );
    put("show_venue", json!(inp.venue));
    put("duration_s", if dur == 0.0 { Value::Null } else { json!(dur) });
    put("transcript_sha256", json!(sha));
    put("audio", audio.clone());
    put("received_at", json!(inp.received_at));
    let analysis = json!({"set": set_dir, "source": "set_text", "run_id": run_id, "audio": audio, "rows": rows,
        "summary": {"material_s": null, "laugh_s_total": null, "laugh_s_per_min": null, "pct_laughing": null,
                    "laughs_merged": null, "laughs_per_min_merged": null, "applause_s": null,
                    "longest_laugh_s": null, "longest_laugh_joke": null, "dead_air_max_s": null,
                    "dead_air_max_at_ms": null, "room_heat": null, "room_heat_basis": "unavailable: no audio",
                    "units": units.len(), "funny_units": funny_units}});

    let transcript_text = dumps_indent1(&tx);
    if ctx.legal.is_legal(&transcript_text) {
        return Ok(Done::result(json!({"status": "skipped_legal"})));
    }

    // every funny unit as an idea, in order (each post is one decision line)
    let posted_title: String = match inp.title.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => "a set",
    }
    .chars()
    .take(120)
    .collect();
    let posted_date = if recorded.is_empty() {
        "unknown".to_string()
    } else {
        recorded.chars().take(10).collect()
    };
    let mut ideas = Vec::new();
    for u in &units {
        if !u.funny || u.text.is_empty() || u.text.chars().count() > IDEA_MAX {
            continue;
        }
        let body = json!({
            "text": u.text, "title": u.name.chars().take(80).collect::<String>(), "actor": Kind::SetText.actor(),
            "from": {"set": set_dir, "unit": u.id},
            "reason": format!("[set_text] bit from '{posted_title}' ({posted_date}), transcribed on the phone; no audio"),
        });
        let got = ctx
            .api
            .call(reqwest::Method::POST, "/api/worker/ideas", Some(&body))
            .await?
            .unwrap_or(Value::Null);
        let attach = got.get("attach").filter(|a| py::truthy(Some(a)));
        ideas.push(
            json!({"unit": u.id, "joke_id": got.get("id").cloned().unwrap_or(Value::Null),
                          "attach": attach.and_then(|a| a.get("decision")).cloned().unwrap_or(Value::Null)}),
        );
    }

    let dir = format!("corpus/sets/{set_dir}");
    let files = vec![
        OutFile {
            path: format!("{dir}/analysis.json"),
            text: dumps_indent1(&analysis),
        },
        OutFile {
            path: format!("{dir}/meta.json"),
            text: dumps_indent1(&Value::Object(meta)),
        },
        OutFile {
            path: format!("{dir}/transcript.json"),
            text: transcript_text,
        },
    ];
    Ok(Done {
        result: json!({"status": "analyzed_text", "kind": "set", "set_dir": set_dir, "units": units.len(),
                       "funny_units": funny_units, "audio": audio, "run_id": run_id, "ideas": ideas})
        .as_object()
        .cloned()
        .unwrap_or_default(),
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pk_wire::worker::TranscriptSegment;

    fn input(segs: Vec<TranscriptSegment>) -> SetTextInput {
        SetTextInput {
            segments: segs,
            title: None,
            recorded_at: None,
            venue: None,
            duration_s: None,
            received_at: "2026-10-20T21:30:00Z".into(),
            engine: String::new(),
        }
    }

    fn seg(text: &str, start: Option<f64>, end: Option<f64>) -> TranscriptSegment {
        TranscriptSegment {
            text: text.into(),
            start,
            end,
        }
    }

    #[test]
    fn synthetic_unless_every_segment_is_timed() {
        let (s, ts, model) = phone_transcript(&input(vec![
            seg("one two three", Some(0.0), Some(1.0)),
            seg("x", None, None),
        ]))
        .unwrap();
        assert_eq!((ts, model.as_str()), ("synthetic", "phone:speech"));
        assert_eq!((s[0].start, s[0].end, s[1].start, s[1].end), (0.0, 1.2, 1.2, 1.7));
        let (_, ts, _) = phone_transcript(&input(vec![seg("a", Some(2.0), Some(1.0))])).unwrap();
        assert_eq!(ts, "synthetic", "an end before its start is not a timestamp");
        assert!(phone_transcript(&input(vec![])).is_err());
        assert!(phone_transcript(&input(vec![seg("  ", None, None)])).is_err());
    }

    #[test]
    fn listing_floors_minutes_like_python() {
        let s = |start| Seg {
            start,
            end: start,
            text: " hi ".into(),
        };
        assert_eq!(listing_line(3, &s(125.9)), "[3] (02:05) hi");
        assert_eq!(listing_line(0, &s(-5.0)), "[0] (-1:55) hi");
    }

    #[test]
    fn spans_are_clipped_and_intro_outro_dropped() {
        let segs: Vec<Seg> = (0..4)
            .map(|i| Seg {
                start: f64::from(i),
                end: f64::from(i) + 0.5,
                text: format!("l{i}"),
            })
            .collect();
        let data = json!({"units": [
            {"first": 2, "last": 9, "kind": "bit", "name": " B ", "funny": true},
            {"first": 0, "last": 0, "kind": "intro", "name": "I", "funny": false},
            {"first": 0, "last": 2, "kind": "bit", "name": "A", "funny": true}]});
        let mut out = Vec::new();
        assemble(&segs, 0, 4, &data, "s", &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].text.as_str(), out[0].id.as_str()), ("l1 l2", "perf:s@1000"));
        assert_eq!((out[1].text.as_str(), out[1].name.as_str()), ("l3", "B"));
    }

    #[test]
    fn the_date_falls_back_to_today() {
        let now = NaiveDateTime::parse_from_str("2026-11-02 09:08:07", "%Y-%m-%d %H:%M:%S").unwrap();
        assert_eq!(now.format("%Y%m%d-%H%M%S").to_string(), "20261102-090807");
        assert!(DATE.is_match("2026-10-20") && !DATE.is_match("2026-10-2T"));
    }
}
