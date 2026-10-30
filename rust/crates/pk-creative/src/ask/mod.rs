//! Ask: a conversation with his own notebook (`creative/askchat.py`, the halves with no model call).
//!
//! The notebook comes in flattened: a [`Notebook`] is the ordered map of quotable [`Source`]s (his written bits, his
//! typed ideas, his voice notes, the moments on his tapes) that the Python's `sources(c)` builds, plus each set's
//! short room name and each joke's title. On it: [`Notebook::retrieve`], the sources for one turn (his-words
//! search, a room named in the question, a year, "killed / best" by laughs, the sources the last answer cited);
//! [`Notebook::prompt`], the messages for the model with the sources numbered `S1..Sn`; and [`Notebook::finish`],
//! the gate: every quote word for word in the source it cites or dropped, every `[S#]` a retrieved source or cut,
//! "quoted" prose in no source cut, and with nothing left the answer is [`REFUSAL`], never a guess.
//!
//! The model call between `prompt` and `finish` (`run_job`, `ask_local`) is the worker's and the API's business.
//! Floats are summed in the Python's order so the ranking (score down, id up) is the same; the search formula is
//! written out in [`bm25`] on purpose (see its doc). The quote gate is this crate's one [`verbatim`].

mod bm25;
pub mod words;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{CreativeError, CreativeResult};
use crate::verbatim::verbatim;
use pk_core::py;
use pk_core::text::head;
pub use words::{mmss, month_year, short_title, snippet, tokens};

/// What the comic reads when the notebook has no answer.
pub const REFUSAL: &str = "I couldn't find that in your notebook.";
/// The most sources one turn shows the model (`MAX_SOURCES`).
pub const MAX_SOURCES: usize = 24;
/// Sources per joke (written and taped counted apart) unless pinned or carried (`PER_JOKE`).
pub const PER_JOKE: usize = 3;
/// A source's text in the prompt is cut to this many code points around the first hit (`SNIPPET`).
pub const SNIPPET: usize = 1100;
/// Earlier turns the model sees (`HISTORY_TURNS`).
pub const HISTORY_TURNS: usize = 4;

/// Words of the question that say what kind of answer he wants, not what it is about (`QSTOP`).
pub(crate) const QSTOP: &[&str] = &[
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

/// `RATING_WANT`: a question word to the rating it asks for.
const RATING_WANT: &[(&str, &str)] = &[
    ("killed", "killed"),
    ("kill", "killed"),
    ("kills", "killed"),
    ("best", "killed"),
    ("strongest", "killed"),
    ("funniest", "killed"),
    ("hit", "killed"),
    ("hits", "killed"),
    ("worked", "worked"),
    ("work", "worked"),
    ("laugh", "killed"),
    ("laughs", "killed"),
    ("died", "died"),
    ("bombed", "died"),
    ("bomb", "died"),
    ("worst", "died"),
    ("weakest", "died"),
    ("quiet", "died"),
    ("flopped", "died"),
];

/// `RATING_ORDER` (`creative/data.py`).
fn rating_order(r: &str) -> i64 {
    match r {
        "killed" => 3,
        "worked" => 2,
        "meh" => 1,
        _ => 0,
    }
}

static YEAR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(19[89][0-9]|20[0-4][0-9])\b").expect("regex"));
static MARK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\[\s*(S\d+(?:\s*[,;]\s*S\d+)*)\s*\]").expect("regex"));
static MARK_SPLIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*[,;]\s*").expect("regex"));
static BAD_MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\s*S\d+[^\]]*\]").expect("regex"));
static STRAIGHT_QUOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new("(\")([^\"\n]{3,600})(\")").expect("regex"));
static CURLY_QUOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("(\u{201c})([^\u{201c}\u{201d}\n]{3,600})(\u{201d})").expect("regex"));
static SQUEEZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t]{2,}").expect("regex"));
static ROOM_TOK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z0-9']+").expect("regex"));
static WANT_TOK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z']+").expect("regex"));

/// The rules in the system block (`RULES`).
pub const RULES: &str = "Rules:\n\
- Answer in plain, short sentences, the way a friend who read his notebook would. No tables, no ratios. \
Say dates the way a person would (Jun 20, 2015).\n\
- Put the source after each claim like [S3]. Every claim needs one.\n\
- The line under each source header is computed from his tapes (room, date, how it went, laughs). You may \
state it, citing that source.\n\
- Quotes go ONLY in `citations`: copy a short line (under 25 words) character for character from that \
source's TEXT. Never quote the header line, never fix typos or transcription errors, never join two \
passages. Do not put quotation marks in `answer`.\n\
- One citation per source you used; `source` is its id, like S3.\n\
- If the sources do not answer the question, set found=false, leave `answer` empty and `citations` empty.";

/// What a source is: a written version of a bit, his idea typed in the app, a voice note, a moment on tape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A written version (a unit from his docs).
    Bit,
    /// His own idea typed in the app (the joke's summary, no doc yet).
    Idea,
    /// A unit that came from a voice memo or a jot, not a doc.
    Note,
    /// A transcript segment of a recorded set.
    Moment,
}

/// One quotable source, as the Python's `sources(c)` flattens it. Written sources have `stage_line` and
/// `nights`; moments have `set_id`, `start_ms`, `venue`, `rating`, `status`, `words` and `laughs`. `gate_text` is
/// the text the quote gate reads when it differs from `text` (`Corpus.text_of`: a moment's transcript before the
/// strip, a typed idea whose joke node carries its own `text`); `None` means the gate reads `text`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// `unit:…`, `joke:…` or `seg:<set>#<idx>`.
    pub id: String,
    /// What it is.
    pub kind: Kind,
    /// The words a quote is checked against (and shown to the model).
    pub text: String,
    /// The joke's short title, or the moment's.
    pub title: String,
    /// `YYYY-MM-DD`, when known.
    #[serde(default)]
    pub date: Option<String>,
    /// The joke it belongs to, when known.
    #[serde(default)]
    pub joke_id: Option<String>,
    /// One line of stage history in words (written sources on tape).
    #[serde(default)]
    pub stage_line: Option<String>,
    /// Nights on tape (written sources).
    #[serde(default)]
    pub nights: Option<i64>,
    /// `set:<key>` (moments).
    #[serde(default)]
    pub set_id: Option<String>,
    /// Where in the recording the moment starts (moments).
    #[serde(default)]
    pub start_ms: Option<i64>,
    /// The room's short name (moments).
    #[serde(default)]
    pub venue: Option<String>,
    /// `killed` / `worked` / `meh` / `died` (moments of a known joke).
    #[serde(default)]
    pub rating: Option<String>,
    /// The segment's status (moments).
    #[serde(default)]
    pub status: Option<String>,
    /// How it went, in words (moments).
    #[serde(default)]
    pub words: Option<String>,
    /// Laugh count (moments).
    #[serde(default)]
    pub laughs: Option<i64>,
    /// The gate's text when it is not `text`.
    #[serde(default)]
    pub gate_text: Option<String>,
}

impl Source {
    /// The text the gate checks a quote against.
    pub fn gate_text(&self) -> &str {
        self.gate_text.as_deref().unwrap_or(&self.text)
    }

    fn joke(&self) -> Option<&str> {
        some_str(self.joke_id.as_ref())
    }

    /// `s["set_id"][4:]`: the set key of a moment.
    fn set_key(&self) -> Option<&str> {
        self.set_id.as_deref().map(|s| s.get(4..).unwrap_or(""))
    }
}

/// `x or None` over an optional string: a present, non-empty string.
fn some_str(v: Option<&String>) -> Option<&str> {
    v.map(String::as_str).filter(|s| !s.is_empty())
}

/// Sources by id, in the Python's order (his jokes' written sources, then the moments set by set).
pub type Sources = IndexMap<String, Source>;

/// An earlier turn of the conversation: its question, the answer shown, the source ids the answer cited.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    /// The question.
    #[serde(default)]
    pub q: Option<String>,
    /// The answer, as shown (the refusal when it was one).
    #[serde(default)]
    pub answer: Option<String>,
    /// The ids its chips pointed at.
    #[serde(default)]
    pub cited: Option<Vec<String>>,
}

/// Who says what in the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// The task's rules.
    System,
    /// The comic.
    User,
    /// The model's earlier answer.
    Assistant,
}

/// One message for the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Who.
    pub role: Role,
    /// The text.
    pub content: String,
}

/// A tappable source as the app shows it (`chip`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chip {
    /// Its number in the answer (`[1]`).
    pub n: usize,
    /// What it is.
    pub kind: Kind,
    /// The source id.
    pub source: String,
    /// The first line on the chip.
    pub label: String,
    /// The second line.
    pub sub: String,
    /// Where a tap goes.
    pub href: String,
    /// The joke, when known.
    pub joke_id: Option<String>,
    /// The set (moments).
    pub set_id: Option<String>,
    /// The position in the recording (moments).
    pub start_ms: Option<i64>,
    /// The date.
    pub date: Option<String>,
    /// The room (moments).
    pub venue: Option<String>,
    /// The exact quote, when one survived the gate.
    pub quote: Option<String>,
}

/// Something the gate cut, and why, in plain words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dropped {
    /// The tag as the model wrote it (`None` for prose quotes).
    pub source: Option<String>,
    /// The quote, cut to 200 characters.
    pub quote: String,
    /// The reason shown.
    pub why: String,
}

/// The gated answer (`finish`), the dict the API stores and the app shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finish {
    /// Always `done`.
    pub status: String,
    /// Whether an answer with at least one chip survived.
    pub found: bool,
    /// What was cut.
    pub dropped: Vec<Dropped>,
    /// The provider that answered.
    pub provider: Option<String>,
    /// The model that answered.
    pub model: Option<String>,
    /// How many sources the turn retrieved.
    pub retrieved: usize,
    /// The answer, or the refusal.
    pub answer: String,
    /// The chips, numbered as the answer cites them.
    pub chips: Vec<Chip>,
}

/// `material_owner()` (`creative/voice.py`): "stand-up comic Pat Example's" when he is named, else "a stand-up
/// comic's". The name is `PK_COMIC_NAME` stripped; empty or the placeholder "the comic" is unnamed.
pub fn material_owner(comic_name: Option<&str>) -> String {
    let name = comic_name.map_or("", py::strip);
    if name.is_empty() || name == "the comic" {
        "a stand-up comic's".to_string()
    } else {
        format!("stand-up comic {name}'s")
    }
}

/// `system_text()`: the task's own system line, with the owner from [`material_owner`].
pub fn system_text(owner: &str) -> String {
    format!(
        "You answer {owner} questions about his OWN material: his written bits, his ideas, his voice notes and the \
         transcripts of his recorded sets (with the room, the date and the laughs). Use ONLY the numbered SOURCES in \
         the latest message. Earlier answers in this conversation only tell you what 'those' or 'it' refers to; they \
         are not evidence. Never use outside knowledge, never guess, never invent a line, a show or a laugh."
    )
}

/// `header(s)`: the one line over a source in the prompt (computed from his tapes, never quotable).
pub fn header(s: &Source) -> String {
    if s.kind == Kind::Moment {
        let at = mmss(s.start_ms.unwrap_or(0) as f64 / 1000.0);
        let bit = if s.joke().is_some() {
            format!(" \u{b7} bit: {}", s.title)
        } else {
            format!(" \u{b7} {}", s.title)
        };
        return format!(
            "set moment \u{b7} {} \u{b7} {} \u{b7} at {at}{bit} \u{b7} {}",
            some_str(s.venue.as_ref()).unwrap_or("a show"),
            some_str(s.date.as_ref()).unwrap_or("undated"),
            s.words.as_deref().unwrap_or("None")
        );
    }
    let kind = match s.kind {
        Kind::Bit => "written bit",
        Kind::Idea => "his idea (typed in the app)",
        Kind::Note | Kind::Moment => "voice note",
    };
    let st = match some_str(s.stage_line.as_ref()) {
        Some(line) => format!(" \u{b7} on stage: {line}"),
        None => " \u{b7} not on tape".to_string(),
    };
    format!(
        "{kind} \u{b7} \"{}\" \u{b7} {}{st}",
        s.title,
        some_str(s.date.as_ref()).unwrap_or("undated")
    )
}

/// `chip(s, n, quote)`: the source as the app shows it.
pub fn chip(s: &Source, n: usize, quote: Option<&str>) -> Chip {
    let (label, sub, href) = if s.kind == Kind::Moment {
        let at = mmss(s.start_ms.unwrap_or(0) as f64 / 1000.0);
        (
            format!(
                "{} \u{b7} {} \u{b7} {at}",
                some_str(s.venue.as_ref()).unwrap_or("On stage"),
                month_year(s.date.as_deref())
            ),
            format!("{} \u{b7} {}", s.title, s.words.as_deref().unwrap_or("None")),
            format!(
                "/set/{}?at={}",
                s.set_id.as_deref().unwrap_or("None"),
                s.start_ms.unwrap_or(0)
            ),
        )
    } else {
        let when = match some_str(s.date.as_ref()) {
            Some(d) => format!(" \u{b7} {}", month_year(Some(d))),
            None => String::new(),
        };
        let sub = match some_str(s.stage_line.as_ref()) {
            Some(line) => line.to_string(),
            None => match s.kind {
                Kind::Bit => "Written",
                Kind::Idea => "Your idea",
                Kind::Note | Kind::Moment => "Voice note",
            }
            .to_string(),
        };
        (
            format!("{}{when}", s.title),
            sub,
            format!("/joke/{}", s.joke_id.as_deref().unwrap_or("None")),
        )
    };
    Chip {
        n,
        kind: s.kind,
        source: s.id.clone(),
        label,
        sub,
        href,
        joke_id: s.joke_id.clone(),
        set_id: s.set_id.clone(),
        start_ms: s.start_ms,
        date: s.date.clone(),
        venue: s.venue.clone(),
        quote: quote.map(str::to_string),
    }
}

/// `refusal(ids)`: nothing retrieved, answered without a model call.
pub fn refusal(ids: &[String]) -> Finish {
    Finish {
        status: "done".to_string(),
        found: false,
        dropped: Vec::new(),
        provider: None,
        model: None,
        retrieved: ids.len(),
        answer: REFUSAL.to_string(),
        chips: Vec::new(),
    }
}

/// His notebook, flattened for Ask: the sources, the rooms, the titles, and the search index over them.
pub struct Notebook {
    sources: Sources,
    /// Set key to the room's short name (`venue_short`), every set, moments or not.
    rooms: IndexMap<String, String>,
    /// Joke id to its title (`Corpus.title`), every joke he has.
    joke_titles: IndexMap<String, String>,
    index: bm25::Index,
}

impl std::fmt::Debug for Notebook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notebook")
            .field("sources", &self.sources.len())
            .field("rooms", &self.rooms.len())
            .field("jokes", &self.joke_titles.len())
            .finish_non_exhaustive()
    }
}

impl Notebook {
    /// A notebook from its flattened parts; the index is built here.
    pub fn new(sources: Sources, rooms: IndexMap<String, String>, joke_titles: IndexMap<String, String>) -> Self {
        let index = bm25::Index::build(&sources);
        Notebook {
            sources,
            rooms,
            joke_titles,
            index,
        }
    }

    /// The sources, in order.
    pub fn sources(&self) -> &Sources {
        &self.sources
    }

    /// `rooms_named(c, question)`: the set keys whose room is named in the question ("The Corner Room",
    /// "Mabel's", "Riverside", "AC" for the seaside room).
    pub fn rooms_named(&self, question: &str) -> BTreeSet<String> {
        let low = question.to_lowercase();
        let qwords: Vec<&str> = ROOM_TOK.find_iter(&low).map(|m| m.as_str()).collect();
        let qw: HashSet<&str> = qwords.iter().copied().collect();
        let ql = format!(" {} ", qwords.join(" "));
        let mut out = BTreeSet::new();
        for (key, room) in &self.rooms {
            let name = room.trim_end_matches('?').to_lowercase();
            if name.is_empty() || name.starts_with("unknown") {
                continue;
            }
            let all: Vec<&str> = ROOM_TOK.find_iter(&name).map(|m| m.as_str()).collect();
            let words: Vec<&str> = all
                .iter()
                .copied()
                .filter(|w| !matches!(*w, "the" | "comedy" | "club" | "on"))
                .collect();
            let named = !words.is_empty()
                && (ql.contains(&format!(" {} ", all.join(" "))) || words.iter().all(|w| qw.contains(w)));
            // the `elif`: the seaside room's alias counts only when its name did not already match
            let alias = !named && name == "seaside" && (qw.contains("ac") || ql.contains("seaside"));
            if named || alias {
                out.insert(key.clone());
            }
        }
        out
    }

    /// `pinned(c, jid)`: every source of one bit (its versions, its typed idea, its taped moments), in source
    /// order, for a conversation about that bit.
    pub fn pinned(&self, jid: Option<&str>) -> Vec<String> {
        let Some(jid) = jid.filter(|j| !j.is_empty()) else {
            return Vec::new();
        };
        self.sources
            .values()
            .filter(|s| s.joke() == Some(jid))
            .map(|s| s.id.clone())
            .collect()
    }

    /// `scope_line(c, jid)`: the bit's short title when it is one of his jokes.
    pub fn scope_line(&self, jid: Option<&str>) -> Option<String> {
        let jid = jid.filter(|j| !j.is_empty())?;
        let title = self.joke_titles.get(jid)?;
        let short = short_title(Some(title));
        Some(if short.is_empty() {
            "Untitled".to_string()
        } else {
            short
        })
    }

    /// The scores behind [`Notebook::retrieve`], after the year and room filters, by source id: what the Python
    /// sorts. Exposed so the differential can compare the sums, not only the order.
    pub fn scores(&self, question: &str, history: &[Turn], pin: &[String]) -> IndexMap<String, f64> {
        self.scored(question, history, pin).0
    }

    /// `retrieve(c, question, history, cap, pin)`: the source ids for one turn, best first. `history` is the
    /// earlier turns, oldest first (the last `HISTORY_TURNS` count); `pin` leads and never counts against the
    /// per-joke cap.
    pub fn retrieve(&self, question: &str, history: &[Turn], cap: usize, pin: &[String]) -> Vec<String> {
        let (score, carried, pinned) = self.scored(question, history, pin);
        let mut order: Vec<&String> = score.keys().collect();
        order.sort_by(|a, b| score[*b].total_cmp(&score[*a]).then_with(|| a.cmp(b)));
        let mut picked = Vec::new();
        let mut per: HashMap<(&str, bool), usize> = HashMap::new();
        for sid in order {
            let s = &self.sources[sid];
            let is_moment = s.kind == Kind::Moment;
            let key = (s.joke().unwrap_or(sid.as_str()), is_moment);
            let n = per.entry(key).or_insert(0);
            if *n >= PER_JOKE && !carried.contains(&sid) && !pinned.contains(sid.as_str()) {
                continue;
            }
            *n += 1;
            picked.push(sid.clone());
            if picked.len() >= cap {
                break;
            }
        }
        picked
    }

    /// The score dict, the carried ids (the last turn's, repeats kept) and the pins that exist.
    fn scored<'a>(
        &'a self,
        question: &str,
        history: &'a [Turn],
        pin: &'a [String],
    ) -> (IndexMap<String, f64>, Vec<&'a String>, HashSet<&'a str>) {
        let history = &history[history.len().saturating_sub(HISTORY_TURNS)..];
        let mut score: IndexMap<String, f64> = IndexMap::new();
        let pinned: HashSet<&str> = pin
            .iter()
            .map(String::as_str)
            .filter(|p| self.sources.contains_key(*p))
            .collect();
        let add = |score: &mut IndexMap<String, f64>, sid: &str, v: f64| {
            if self.sources.contains_key(sid) {
                let e = score.entry(sid.to_string()).or_insert(0.0);
                *e += v;
            }
        };
        // the pins are a set in Python: each gets its 12.0 once, in an order that cannot change a per-id sum
        for sid in pin.iter().filter(|p| pinned.contains(p.as_str())) {
            if !score.contains_key(sid.as_str()) {
                add(&mut score, sid, 12.0);
            }
        }
        for (sid, v) in self.index.score(question) {
            add(&mut score, &sid, v);
        }
        if !history.is_empty() {
            let qs: Vec<&str> = history.iter().map(|h| h.q.as_deref().unwrap_or("")).collect();
            for (sid, v) in self.index.score(&qs.join(" ")) {
                add(&mut score, &sid, 0.4 * v);
            }
        }
        let carried: Vec<&String> = history
            .last()
            .and_then(|h| h.cited.as_ref())
            .map(|c| c.iter().collect())
            .unwrap_or_default();
        let carried_jokes: HashSet<&str> = carried
            .iter()
            .filter_map(|x| self.sources.get(x.as_str()))
            .filter_map(Source::joke)
            .collect();
        for sid in &carried {
            add(&mut score, sid, 6.0);
        }
        let low = question.to_lowercase();
        let want: Option<&str> = WANT_TOK
            .find_iter(&low)
            .map(|m| m.as_str().trim_matches('\''))
            .find_map(|w| RATING_WANT.iter().find(|(k, _)| *k == w).map(|(_, v)| *v));
        let rooms = self.rooms_named(question);
        for (sid, s) in &self.sources {
            if s.kind != Kind::Moment {
                continue;
            }
            let in_room = !rooms.is_empty() && s.set_key().is_some_and(|k| rooms.contains(k));
            if in_room {
                add(&mut score, sid, 5.0 + if s.joke().is_some() { 0.5 } else { 0.0 });
            }
            if !carried_jokes.is_empty()
                && s.joke().is_some_and(|j| carried_jokes.contains(j))
                && (want.is_some() || !rooms.is_empty())
            {
                add(&mut score, sid, 4.0); // "which of those killed?": their nights on tape
            }
            if let Some(want) = want
                && score.contains_key(sid.as_str())
                && let Some(rating) = some_str(s.rating.as_ref())
            {
                let r = rating_order(rating);
                let r = if want == "died" { 3 - r } else { r };
                let laughs = s.laughs.unwrap_or(0).min(10);
                add(&mut score, sid, r as f64 * 0.8 + laughs as f64 * 0.05);
            }
        }
        let years: HashSet<u32> = YEAR_RE
            .find_iter(question)
            .filter_map(|m| m.as_str().parse().ok())
            .collect();
        if !years.is_empty() {
            let kept: IndexMap<String, f64> = score
                .iter()
                .filter(|(k, _)| {
                    let year: String = self.sources[k.as_str()]
                        .date
                        .as_deref()
                        .unwrap_or("")
                        .chars()
                        .take(4)
                        .collect();
                    !year.is_empty()
                        && year.chars().all(|c| c.is_ascii_digit())
                        && year.parse::<u32>().is_ok_and(|y| years.contains(&y))
                })
                .map(|(k, v)| (k.clone(), *v))
                .collect();
            if !kept.is_empty() {
                score = kept;
            }
        }
        if !rooms.is_empty() {
            // a question about a room is answered from that room's sets (and what was already on screen)
            let kept: IndexMap<String, f64> = score
                .iter()
                .filter(|(k, _)| {
                    let s = &self.sources[k.as_str()];
                    (s.kind == Kind::Moment && s.set_key().is_some_and(|key| rooms.contains(key)))
                        || carried.contains(k)
                })
                .map(|(k, v)| (k.clone(), *v))
                .collect();
            if !kept.is_empty() {
                score = kept;
            }
        }
        (score, carried, pinned)
    }

    /// `prompt(c, question, history, ids, scope)`: the messages for the model. `ids` are the retrieved sources
    /// (`S1` = `ids[0]`), `scope` the short title of the bit this conversation is about, `owner` the string from
    /// [`material_owner`]. An id that is not a source is the Python's `KeyError`.
    pub fn prompt(
        &self,
        question: &str,
        history: &[Turn],
        ids: &[String],
        scope: Option<&str>,
        owner: &str,
    ) -> CreativeResult<Vec<Message>> {
        let terms: Vec<String> = tokens(question)
            .into_iter()
            .filter(|w| !QSTOP.contains(&w.as_str()))
            .collect();
        let mut msgs = vec![Message {
            role: Role::System,
            content: format!("{}\n\n{RULES}", system_text(owner)),
        }];
        for h in &history[history.len().saturating_sub(HISTORY_TURNS)..] {
            if let Some(q) = some_str(h.q.as_ref()) {
                msgs.push(Message {
                    role: Role::User,
                    content: q.to_string(),
                });
                msgs.push(Message {
                    role: Role::Assistant,
                    content: some_str(h.answer.as_ref()).unwrap_or(REFUSAL).to_string(),
                });
            }
        }
        let mut blocks = Vec::with_capacity(ids.len());
        for (i, sid) in ids.iter().enumerate() {
            let s = self.sources.get(sid).ok_or_else(|| CreativeError::missing(sid))?;
            blocks.push(format!(
                "[S{}] {}\nTEXT: {}",
                i + 1,
                header(s),
                snippet(&s.text, &terms, SNIPPET)
            ));
        }
        let about = scope.map_or_else(String::new, |scope| {
            format!("THIS CONVERSATION IS ABOUT HIS BIT: {scope}. 'This bit', 'it' and 'the joke' mean that one.\n\n")
        });
        msgs.push(Message {
            role: Role::User,
            content: format!(
                "{about}QUESTION: {question}\n\nSOURCES:\n\n{}\n\nAnswer by the rules above.",
                blocks.join("\n\n")
            ),
        });
        Ok(msgs)
    }

    /// `finish(c, ids, data, provider, model)`: the model's answer through the gate. `ids` are the retrieved
    /// sources (`S1` = `ids[0]`); `data` is the model's JSON (anything but an object counts as empty). A `citations`
    /// value Python could not iterate (a number, `true`) is the Python's `TypeError`; a truthy non-string `quote`
    /// is a fault here too (the Python faults on it as well, with a message that depends on the path).
    pub fn finish(
        &self,
        ids: &[String],
        data: &Value,
        provider: Option<&str>,
        model: Option<&str>,
    ) -> CreativeResult<Finish> {
        let empty = serde_json::Map::new();
        let data = data.as_object().unwrap_or(&empty);
        let sid_of: IndexMap<String, &str> = ids
            .iter()
            .enumerate()
            .filter(|(_, x)| self.sources.contains_key(x.as_str()))
            .map(|(i, x)| (format!("S{}", i + 1), x.as_str()))
            .collect();
        let mut dropped: Vec<Dropped> = Vec::new();
        let mut quotes: HashMap<&str, String> = HashMap::new();
        let mut cited_order: Vec<&str> = Vec::new();
        for cit in citations(data)? {
            let Some(cit) = cit.as_object() else {
                continue;
            };
            let tag = cit
                .get("source")
                .filter(|v| py::truthy(Some(v)))
                .map(py::str)
                .unwrap_or_default();
            let tag = py::strip(&tag).trim_matches(['[', ']']).to_uppercase();
            let quote = quote_of(cit.get("quote"))?;
            let Some(&sid) = sid_of.get(&tag) else {
                dropped.push(Dropped {
                    source: Some(tag),
                    quote: head(&quote, 200),
                    why: "not a retrieved source".to_string(),
                });
                continue;
            };
            let q = py::strip(&quote);
            let exact = if q.is_empty() {
                None
            } else {
                verbatim(q, self.sources[sid].gate_text())
            };
            match exact {
                None if !q.is_empty() => dropped.push(Dropped {
                    source: Some(tag),
                    quote: head(q, 200),
                    why: "not word for word in the source".to_string(),
                }),
                Some(found) if !quotes.contains_key(sid) => {
                    quotes.insert(sid, found.to_string());
                }
                _ => {}
            }
            if !cited_order.contains(&sid) {
                cited_order.push(sid);
            }
        }
        let answer = data
            .get("answer")
            .filter(|v| py::truthy(Some(v)))
            .map(py::str)
            .unwrap_or_default();
        let answer = py::strip(&answer).to_string();
        // [S#] markers: retrieved ones become [1], [2] … (chip numbers); anything else is cut
        let mut num: IndexMap<&str, usize> = IndexMap::new();
        let answer = MARK_RE.replace_all(&answer, |m: &Captures| {
            let mut out = String::new();
            for tag in MARK_SPLIT.split(&m[1]) {
                let Some(&sid) = sid_of.get(&tag.to_uppercase()) else {
                    dropped.push(Dropped {
                        source: Some(tag.to_string()),
                        quote: String::new(),
                        why: "cited in the answer but not a retrieved source".to_string(),
                    });
                    continue;
                };
                let next = num.len() + 1;
                let n = *num.entry(sid).or_insert(next);
                out.push('[');
                out.push_str(&n.to_string());
                out.push(']');
            }
            out
        });
        let answer = BAD_MARK.replace_all(&answer, ""); // a malformed marker ("[S3a]", "[S3, S4b]") is cut, not shown
        // "quoted" passages in the prose must be in a retrieved source, word for word
        let texts: Vec<&str> = sid_of.values().map(|sid| self.sources[*sid].gate_text()).collect();
        let mut q_ok = |m: &Captures| -> String {
            let q = &m[2];
            if words::py_word_count(q) < 4 || texts.iter().any(|t| verbatim(q, t).is_some()) {
                return m[0].to_string();
            }
            dropped.push(Dropped {
                source: None,
                quote: head(q, 200),
                why: "quoted in the answer, not word for word in any source".to_string(),
            });
            "[quote removed]".to_string()
        };
        let answer = STRAIGHT_QUOTE.replace_all(&answer, &mut q_ok);
        let answer = CURLY_QUOTE.replace_all(&answer, &mut q_ok);
        for sid in cited_order {
            let next = num.len() + 1;
            num.entry(sid).or_insert(next);
        }
        let mut numbered: Vec<(&str, usize)> = num.iter().map(|(k, v)| (*k, *v)).collect();
        numbered.sort_by_key(|(_, n)| *n);
        let chips: Vec<Chip> = numbered
            .into_iter()
            .map(|(sid, n)| chip(&self.sources[sid], n, quotes.get(sid).map(String::as_str)))
            .collect();
        let found = py::truthy(data.get("found")) && !chips.is_empty() && !py::strip(&answer).is_empty();
        let (answer, chips) = if found {
            (py::strip(&SQUEEZE.replace_all(&answer, " ")).to_string(), chips)
        } else {
            (REFUSAL.to_string(), Vec::new())
        };
        Ok(Finish {
            status: "done".to_string(),
            found,
            dropped,
            provider: provider.map(str::to_string),
            model: model.map(str::to_string),
            retrieved: ids.len(),
            answer,
            chips,
        })
    }
}

/// `data.get("citations") or []` as the items Python would loop over: a list's items; a string's characters and
/// an object's keys are never dicts, so nothing; a truthy number or `true` is the `TypeError`.
fn citations(data: &serde_json::Map<String, Value>) -> CreativeResult<Vec<Value>> {
    match data.get("citations") {
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(v) if py::truthy(Some(v)) && !matches!(v, Value::String(_) | Value::Object(_)) => Err(
            CreativeError::faulted(format!("'{}' object is not iterable", py::type_name(v))),
        ),
        _ => Ok(Vec::new()),
    }
}

/// `cit.get("quote") or ""`: the string, or empty; a truthy non-string is the Python's `TypeError`.
fn quote_of(v: Option<&Value>) -> CreativeResult<String> {
    match v {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(v) if py::truthy(Some(v)) => Err(CreativeError::faulted(format!(
            "'{}' object is not subscriptable",
            py::type_name(v)
        ))),
        _ => Ok(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bit(id: &str, joke: &str, text: &str) -> Source {
        Source {
            id: id.to_string(),
            kind: Kind::Bit,
            text: text.to_string(),
            title: "A bit".to_string(),
            date: Some("2024-01-10".to_string()),
            joke_id: Some(joke.to_string()),
            stage_line: None,
            nights: Some(0),
            set_id: None,
            start_ms: None,
            venue: None,
            rating: None,
            status: None,
            words: None,
            laughs: None,
            gate_text: None,
        }
    }

    fn notebook() -> Notebook {
        let mut sources = Sources::new();
        for (id, text) in [
            (
                "unit:a1",
                "Alpha opener line one. The alpha punch lands here every time.",
            ),
            (
                "unit:b1",
                "Bravo setup about the dog. The purple monkey dishwasher line here.",
            ),
        ] {
            sources.insert(id.to_string(), bit(id, &format!("joke:{}", &id[5..6]), text));
        }
        Notebook::new(sources, IndexMap::new(), IndexMap::new())
    }

    #[test]
    fn owner_and_system_text() {
        assert_eq!(material_owner(Some(" Pat Example ")), "stand-up comic Pat Example's");
        assert_eq!(material_owner(Some("the comic")), "a stand-up comic's");
        assert_eq!(material_owner(None), "a stand-up comic's");
        assert!(system_text(&material_owner(None)).starts_with("You answer a stand-up comic's questions"));
    }

    #[test]
    fn the_gate_can_fail() {
        let nb = notebook();
        let ids = vec!["unit:b1".to_string()];
        let data = json!({"found": true, "answer": "The dog [S1] and \"a line that is nowhere\".",
            "citations": [{"source": "S1", "quote": "purple monkey washed"}]});
        let out = nb.finish(&ids, &data, Some("p"), Some("m")).unwrap();
        assert!(out.found);
        assert_eq!(out.answer, "The dog [1] and [quote removed].");
        assert_eq!(out.chips[0].quote, None);
        assert_eq!(out.dropped.len(), 2);
        let none = nb.finish(&ids, &json!("nope"), None, None).unwrap();
        assert!(!none.found);
        assert_eq!(none.answer, REFUSAL);
        assert!(nb.finish(&ids, &json!({"citations": 5}), None, None).is_err());
    }

    #[test]
    fn retrieve_and_prompt() {
        let nb = notebook();
        let ids = nb.retrieve("the purple monkey", &[], MAX_SOURCES, &[]);
        assert_eq!(ids, ["unit:b1"]);
        let msgs = nb
            .prompt("the purple monkey", &[], &ids, Some("B bit"), "a stand-up comic's")
            .unwrap();
        assert_eq!(msgs.len(), 2);
        assert!(
            msgs[1]
                .content
                .starts_with("THIS CONVERSATION IS ABOUT HIS BIT: B bit.")
        );
        assert!(msgs[1].content.contains("[S1] written bit"));
        assert!(nb.prompt("x", &[], &["unit:zz".to_string()], None, "o").is_err());
    }
}
