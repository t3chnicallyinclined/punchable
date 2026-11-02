//! Punch-up drafts (`creative/workingset.py` `punch_messages` / `punch_up`), the halves with no model call: the
//! lines of his latest taped night after his locks (a locked line is never shown to the model, not even as
//! context), what the room did after each line ("laugh 2.1 s", "laugh · 3/4 bars", "silence"), the user turn, and
//! what is kept of an answer (a numbered, unlocked line with a real alternative).

use std::fmt::Write as _;
use std::sync::LazyLock;

use pk_core::py;
use serde_json::{Map, Value, json};

use crate::corpus::{Corpus, Perf, num, venue_short};
use crate::textdiff::{phrases, present, toks};
use crate::write::DRAFT_LABEL;

/// `PUNCH_SYSTEM`: the task's own system line.
pub const PUNCH_SYSTEM: &str = "You punch up one comedian's own lines in his voice. Drafts only; he decides.";
/// `PUNCH_LINES`: lines offered at most.
pub const PUNCH_LINES: usize = 40;
/// `PUNCH_SETUP_CHARS`: of his written version.
pub const PUNCH_SETUP_CHARS: usize = 400;
/// Drafts asked for.
pub const PUNCH_N: usize = 3;

/// `PUNCH_SCHEMA`.
pub static PUNCH_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({"type": "object", "properties": {"drafts": {"type": "array", "items": {
        "type": "object", "properties": {"line_no": {"type": "integer"}, "alt": {"type": "string"},
                                         "move": {"type": "string"}},
        "required": ["line_no", "alt", "move"], "additionalProperties": false}}},
        "required": ["drafts"], "additionalProperties": false})
});

/// A line of the latest night: (phrase, segment id).
pub type NightLine = (String, String);

fn locks<'a>(ws: &'a Value, jid: &str) -> Option<&'a Map<String, Value>> {
    ws.get("locks")?.get(jid)?.as_object()
}

/// `is_locked_line(ws, jid, line)`: the line is (or contains) one he locked.
pub fn is_locked_line(ws: &Value, jid: &str, line: &str) -> bool {
    locks(ws, jid)
        .and_then(|l| l.get("lines"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|x| {
            let t = x.get("text").and_then(Value::as_str).unwrap_or("");
            present(t, line, 0.5) || present(line, t, 0.5)
        })
}

/// `bit_locked(ws, jid)`: the whole bit is locked (perfected).
pub fn bit_locked(ws: &Value, jid: &str) -> bool {
    py::truthy(locks(ws, jid).and_then(|l| l.get("bit")))
}

/// `latest_lines(c, jid)`: his latest night of the bit and its lines, locks not applied.
pub fn latest_lines<'c>(c: &'c Corpus, jid: &str) -> Option<(&'c Perf, Vec<NightLine>)> {
    let p = c.perfs.get(jid)?.last()?;
    let lines = p
        .seg_ids
        .iter()
        .flat_map(|sid| {
            phrases(&c.text_of(sid, None).unwrap_or_default())
                .into_iter()
                .map(|ph| (ph, sid.clone()))
        })
        .collect();
    Some((p, lines))
}

/// `_same_line(a, b)`: one is a run of the other, or they share three quarters of their words.
pub fn same_line(a: &str, b: &str) -> bool {
    let (ta, tb) = (toks(a), toks(b));
    if ta.is_empty() || tb.is_empty() {
        return false;
    }
    let (ja, jb) = (ta.join(" "), tb.join(" "));
    if jb.contains(&ja) || ja.contains(&jb) {
        return true;
    }
    let sa: std::collections::HashSet<&String> = ta.iter().collect();
    let shared = tb
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .intersection(&sa)
        .count();
    let need = ((ta.len().min(tb.len()) as f64) * 0.75).ceil() as usize;
    shared >= need.max(3)
}

/// `laugh_marks(c, perf, lines)`: what the room did right after each line.
pub fn laugh_marks(c: &Corpus, perf: &Perf, lines: &[NightLine]) -> Vec<String> {
    let events = c.laughs.get(&perf.set);
    let bars = |p: &Map<String, Value>| num(p.get("bars")).filter(|b| *b != 0.0).unwrap_or(0.0);
    lines
        .iter()
        .map(|(ph, sid)| {
            let mut hit: Option<&Map<String, Value>> = None;
            for pl in &perf.punchlines {
                let seg_ok = match pl.get("seg_id") {
                    None | Some(Value::Null) => true,
                    Some(v) => v.as_str() == Some(sid.as_str()),
                };
                if !seg_ok || !same_line(ph, pl.get("text").and_then(Value::as_str).unwrap_or("")) {
                    continue;
                }
                if hit.is_none_or(|h| bars(pl) > bars(h)) {
                    hit = Some(pl);
                }
            }
            let Some(hit) = hit else {
                return "silence".to_string();
            };
            let ev = hit
                .get("laugh_id")
                .filter(|v| !v.is_null())
                .and_then(|id| events?.get(&py::str(id)));
            let span = ev.and_then(|e| Some((num(e.get("start_ms"))?, num(e.get("end_ms"))?)));
            match span {
                Some((a, b)) if b > a => format!("laugh {:.1} s", (b - a) / 1000.0),
                _ => format!("laugh · {}/4 bars", bars(hit) as i64),
            }
        })
        .collect()
}

/// What a punch-up was built from.
#[derive(Debug, Clone, PartialEq)]
pub struct Basis {
    /// The latest night's set, when there is one.
    pub based_on: Option<String>,
    /// The lines offered (after the locks), numbered from 1.
    pub lines: Vec<NightLine>,
    /// Locked lines left out.
    pub blocked: usize,
}

/// `punch_messages` without the voice blocks: the user turn (`None` when there is nothing to punch up) and its
/// basis.
pub fn punch_user(c: &Corpus, ws: &Value, jid: &str) -> (Option<String>, Basis) {
    let Some((p, raw)) = latest_lines(c, jid) else {
        return (
            None,
            Basis {
                based_on: None,
                lines: Vec::new(),
                blocked: 0,
            },
        );
    };
    let mut blocked = 0;
    let mut lines: Vec<NightLine> = Vec::new();
    for (ph, sid) in raw {
        if is_locked_line(ws, jid, &ph) {
            blocked += 1;
        } else {
            lines.push((ph, sid));
        }
    }
    lines.truncate(PUNCH_LINES);
    let basis = Basis {
        based_on: Some(p.set.clone()),
        lines,
        blocked,
    };
    if basis.lines.is_empty() {
        return (None, basis);
    }
    let ps = &c.perfs[jid];
    let key = |x: &Perf| (x.laugh_s_per_min.unwrap_or(0.0), x.date.clone());
    let mut best = &ps[0];
    for x in &ps[1..] {
        let (a, b) = (key(x), key(best));
        #[allow(clippy::float_cmp)] // Python's tuple comparison: exact, 0.0 == -0.0
        let later_wins = a.0 > b.0 || (a.0 == b.0 && a.1 > b.1);
        if later_wins {
            best = x;
        }
    }
    let hits: Vec<String> = best
        .punchlines
        .iter()
        .filter(|x| num(x.get("bars")).unwrap_or(0.0) >= 2.0)
        .map(|x| x.get("text").map(py::str).unwrap_or_default())
        .take(4)
        .collect();
    let devs: Vec<String> = c
        .devices(jid, 3, 0.55)
        .iter()
        .map(|l| py::str(&c.label_field(l, "name")))
        .collect();
    let marks = laugh_marks(c, p, &basis.lines);
    let lead = c.lead_unit(jid);
    let mut written = py::strip(lead.as_deref().map_or("", |u| c.node_text(u))).to_string();
    if phrases(&written).iter().any(|ph| is_locked_line(ws, jid, ph)) {
        written.clear(); // a locked line is never shown to the model, not even as context
    }
    let chars: Vec<char> = written.chars().collect();
    if chars.len() > PUNCH_SETUP_CHARS {
        let cut = chars[..PUNCH_SETUP_CHARS].iter().rposition(|ch| *ch == ' ');
        let at = cut.filter(|c| *c > PUNCH_SETUP_CHARS / 2).unwrap_or(PUNCH_SETUP_CHARS);
        let head: String = chars[..at].iter().collect();
        written = format!("{} [...]", head.trim_end_matches(py::isspace));
    }
    let dead = marks.iter().filter(|m| *m == "silence").count();
    let n_lines = basis.lines.len();
    let mut msg = format!(
        "Bit: '{}'. His moves in this bit: {}.\nLines that got real laughs on his best night ({}):\n",
        c.title(jid),
        if devs.is_empty() {
            "n/a".to_string()
        } else {
            devs.join(", ")
        },
        venue_short(best.venue.as_str(), Some(&best.date)),
    );
    if hits.is_empty() {
        msg.push_str("- (none on tape)");
    } else {
        msg.push_str(&hits.iter().map(|h| format!("- {h}")).collect::<Vec<_>>().join("\n"));
    }
    if !written.is_empty() {
        let _ = write!(msg, "\n\nHis written version of the setup (verbatim):\n\"{written}\"");
    }
    let _ = write!(
        msg,
        "\n\nLines from his latest night ({}) you MAY punch up (only these, by number), each with what the room did \
         right after it:\n",
        venue_short(p.venue.as_str(), Some(&p.date))
    );
    let numbered: Vec<String> = basis
        .lines
        .iter()
        .zip(&marks)
        .enumerate()
        .map(|(i, ((ph, _), m))| format!("{}. {ph}   [{m}]", i + 1))
        .collect();
    msg.push_str(&numbered.join("\n"));
    let _ = write!(
        msg,
        "\n\nGive up to {PUNCH_N} punch-up drafts: a sharper alternative for one numbered line each, in his voice, \
         using his own moves. Keep the premise. "
    );
    if dead > 0 {
        let _ = write!(
            msg,
            "Target the lines that got nothing ([silence]: {dead} of {n_lines}); leave a line that already lands. "
        );
    } else {
        msg.push_str("Every line got a laugh: sharpen the weakest, leave the biggest alone. ");
    }
    msg.push_str("`move` names the move you used, by the name his profile gives it.");
    (Some(msg), basis)
}

/// The drafts kept from an answer, and the blocked count after the belt-and-braces lock check.
pub fn punch_drafts(data: Option<&Value>, ws: &Value, jid: &str, basis: &Basis) -> (Vec<Value>, usize) {
    let mut blocked = basis.blocked;
    let mut out = Vec::new();
    let drafts = data
        .and_then(|d| d.get("drafts"))
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for d in drafts.iter().take(PUNCH_N) {
        // `isinstance(k, int)`: a JSON integer, never 2.0
        let k = d
            .get("line_no")
            .and_then(|v| if v.is_i64() || v.is_u64() { v.as_i64() } else { None });
        let Some(k) = k
            .and_then(|k| usize::try_from(k).ok())
            .filter(|k| (1..=basis.lines.len()).contains(k))
        else {
            continue;
        };
        let (ph, sid) = &basis.lines[k - 1];
        if is_locked_line(ws, jid, ph) {
            blocked += 1;
            continue;
        }
        let alt = py::strip(d.get("alt").and_then(Value::as_str).unwrap_or(""));
        if alt.is_empty() || toks(alt).join(" ") == toks(ph).join(" ") {
            continue;
        }
        out.push(
            json!({"target_line": ph, "target_evidence": {"node": sid, "quote": ph}, "alt": alt,
                        "move": d.get("move").cloned().unwrap_or(Value::Null), "draft": true, "label": DRAFT_LABEL}),
        );
    }
    (out, blocked)
}
