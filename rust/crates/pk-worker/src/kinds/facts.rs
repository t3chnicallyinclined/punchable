//! `facts` (`creative/write.py` `check_facts`, Write's "Check this"): a few plain facts about an idea, each with a
//! source page that was fetched and actually contains the fact's key words. A fact with no `https` source, a page
//! that does not load, or a page that does not say it is dropped, never shown bare. The pages are fetched a few at
//! a time in the facts' order (the Python fetched one by one); the result is the same because each page decides only
//! its own fact, and the fetching stops once three facts are kept.

use std::sync::LazyLock;
use std::time::Duration;

use futures::StreamExt as _;
use pk_core::py;
use pk_creative::ask::words::tokens;
use pk_llm::ChatMessage;
use pk_wire::worker::FactsInput;
use regex::Regex;
use serde_json::{Map, Value, json};

use super::{Ctx, Done, Runner};
use crate::error::{JobError, provider_why};

/// The task in `engine/config.hosted.toml`.
pub const TASK: &str = "creative_facts";
/// Facts kept at most.
pub const CAP: usize = 3;
/// Pages fetched at once.
const FETCH_AHEAD: usize = 4;
/// The most of a page that is read.
const PAGE_BYTES: usize = 2_000_000;
const PAGE_TIMEOUT: Duration = Duration::from_secs(8);
const PAGE_AGENT: &str = "Mozilla/5.0 (Punchable fact check)";
const LABEL: &str = "Checked by the AI against its sources · read the source";
const SYSTEM: &str = "You are a careful fact checker. Every fact must be stated on the page you cite.";

/// `FACTS_SCHEMA`.
pub static SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    json!({"type": "object", "properties": {"facts": {"type": "array", "items": {
        "type": "object", "properties": {"fact": {"type": "string"}, "source_title": {"type": "string"},
                                         "source_url": {"type": "string"}},
        "required": ["fact", "source_title", "source_url"], "additionalProperties": false}}},
        "required": ["facts"], "additionalProperties": false})
});

/// `URL_RE`: an `https` URL on a public-looking host.
pub static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https://[A-Za-z0-9.-]+\.[A-Za-z]{2,}(/[^\s]*)?$").expect("a valid regex"));
static OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<(script|style)[^>]*>").expect("a valid regex"));
static CLOSE_SCRIPT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)</script>").expect("a valid regex"));
static CLOSE_STYLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)</style>").expect("a valid regex"));
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("a valid regex"));

/// Fetches a fact's source page as lower-cased visible text.
#[derive(Debug, Clone)]
pub struct Pages {
    http: reqwest::Client,
    via: Option<String>,
}

impl Pages {
    /// The real web, with the Python's agent and timeouts.
    pub fn new() -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .user_agent(PAGE_AGENT)
            .connect_timeout(PAGE_TIMEOUT)
            .read_timeout(PAGE_TIMEOUT)
            .build()?;
        Ok(Self { http, via: None })
    }

    /// Tests only: `https://host/path` is fetched as `<base>/host/path` (the differential's page server).
    #[must_use]
    pub fn via(mut self, base: impl Into<String>) -> Self {
        self.via = Some(base.into().trim_end_matches('/').to_string());
        self
    }

    /// `fetch_text(url)`: the page's visible text, lower-cased, or `None` when it does not load with a 200.
    pub async fn text(&self, url: &str) -> Option<String> {
        let target = match &self.via {
            Some(base) => url.replacen("https://", &format!("{base}/"), 1),
            None => url.to_string(),
        };
        let mut resp = self.http.get(target).send().await.ok()?;
        if resp.status() != reqwest::StatusCode::OK {
            return None;
        }
        let mut raw: Vec<u8> = Vec::new();
        while raw.len() < PAGE_BYTES {
            match resp.chunk().await {
                Ok(Some(c)) => raw.extend_from_slice(&c[..c.len().min(PAGE_BYTES - raw.len())]),
                Ok(None) => break,
                Err(_) => return None,
            }
        }
        // up to 2 MB through three regex passes, the entities and the case: off the async threads
        tokio::task::spawn_blocking(move || page_text(&String::from_utf8_lossy(&raw)))
            .await
            .ok()
    }
}

/// The text of a page: `<script>` / `<style>` blocks out (`(?is)<(script|style)[^>]*>.*?</\1>`, written out
/// because the regex crate has no back-references), every tag a space, entities decoded, lower case.
pub fn page_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let (mut kept, mut from) = (0, 0);
    while let Some(c) = OPEN.captures_at(raw, from) {
        let (Some(whole), Some(tag)) = (c.get(0), c.get(1)) else {
            break;
        };
        let close = if tag.as_str().eq_ignore_ascii_case("script") {
            &CLOSE_SCRIPT
        } else {
            &CLOSE_STYLE
        };
        if let Some(end) = close.find_at(raw, whole.end()) {
            out.push_str(&raw[kept..whole.start()]);
            out.push(' ');
            kept = end.end();
            from = end.end();
        } else {
            from = whole.start() + 1; // `<` is one byte: the next position is a char boundary
        }
    }
    out.push_str(&raw[kept..]);
    let untagged = TAG.replace_all(&out, " ");
    html_escape::decode_html_entities(&untagged).to_lowercase()
}

/// `supported(fact, page)`: at least two thirds of the fact's content words (and at least 2) are on the page.
pub fn supported(fact: &str, page: Option<&str>) -> bool {
    let Some(page) = page.filter(|p| !p.is_empty()) else {
        return false;
    };
    let mut ws: Vec<String> = Vec::new();
    for w in tokens(fact) {
        if !ws.contains(&w) {
            ws.push(w);
        }
    }
    ws.retain(|w| w.chars().count() >= 4);
    if ws.len() < 2 {
        return false;
    }
    let hit = ws.iter().filter(|w| page.contains(w.as_str())).count();
    hit >= 2 && hit * 3 >= ws.len() * 2
}

fn field<'a>(f: &'a Value, k: &str) -> &'a str {
    f.get(k).and_then(Value::as_str).unwrap_or("")
}

/// The `facts` kind.
#[derive(Debug)]
pub struct Facts;

impl Runner for Facts {
    type Input = FactsInput;

    fn bad_input(e: &serde_json::Error) -> JobError {
        JobError::Value(format!("bad facts input: {e}"))
    }

    async fn run(ctx: Ctx<'_>, input: FactsInput) -> Result<Done, JobError> {
        let text = py::strip(&input.text).to_string();
        let mut out = Map::new();
        out.insert("idea".into(), json!(text));
        out.insert("draft".into(), json!(true));
        out.insert("label".into(), json!(LABEL));
        if text.is_empty() {
            out.insert("status".into(), json!("empty"));
            out.insert("facts".into(), json!([]));
            out.insert("dropped".into(), json!(0));
            return Ok(Done::result(Value::Object(out)));
        }
        let msg = format!(
            "A comedian is writing about this idea:\n\"{text}\"\n\nGive up to {} short, plain, checkable facts that \
             would help him understand the subject (how it works, real numbers, history). One sentence each. For each, \
             the https URL of a public page (Wikipedia, an encyclopedia, a news or reference site) that states it. \
             No jokes. If you are not sure of a fact or its source, leave it out.",
            CAP + 2
        );
        let msgs = [ChatMessage::system(SYSTEM), ChatMessage::user(msg)];
        let c = match ctx
            .router
            .complete(TASK, &msgs, Some(&SCHEMA), Some("creative"), None)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                out.insert("status".into(), json!("provider_error"));
                out.insert("facts".into(), json!([]));
                out.insert("dropped".into(), json!(0));
                out.insert("why".into(), json!(provider_why(&e)));
                return Ok(Done::result(Value::Object(out)));
            }
        };
        let proposed: Vec<Value> = c
            .data
            .as_ref()
            .and_then(|d| d.get("facts"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // each fact with its page (fetched only for a well-formed https URL), in order, a few in flight
        let mut checked = futures::stream::iter(proposed)
            .map(|f| async move {
                let fact = py::strip(field(&f, "fact")).to_string();
                let url = py::strip(field(&f, "source_url")).to_string();
                let usable = !fact.is_empty() && URL_RE.is_match(&url);
                let page = if usable { ctx.pages.text(&url).await } else { None };
                (f, fact, url, usable, page)
            })
            .buffered(FETCH_AHEAD);
        let (mut kept, mut dropped) = (Vec::new(), 0u64);
        while let Some((f, fact, url, usable, page)) = checked.next().await {
            if !usable || !supported(&fact, page.as_deref()) {
                dropped += 1;
                continue;
            }
            let title = field(&f, "source_title");
            let title = if title.is_empty() { url.as_str() } else { title };
            let title: String = py::strip(title).chars().take(120).collect();
            kept.push(json!({"text": fact, "source_title": title, "source_url": url}));
            if kept.len() >= CAP {
                break;
            }
        }
        out.insert("status".into(), json!(if kept.is_empty() { "none" } else { "ok" }));
        out.insert("facts".into(), Value::Array(kept));
        out.insert("dropped".into(), json!(dropped));
        out.insert("provider".into(), json!(c.provider));
        out.insert("model".into(), json!(c.model));
        Ok(Done::result(Value::Object(out)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_and_style_blocks_go_like_the_backreference() {
        assert_eq!(page_text("a<script>x</script>b"), "a b");
        // the block runs to the first closing tag of its own name, past a `</style>`
        assert_eq!(page_text("<script>a</style>b</script>tail"), " tail");
        assert_eq!(page_text("<style>unterminated <p>text</p>"), " unterminated  text ");
    }
}
