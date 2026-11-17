//! The `redline` kind end to end (a bit's redline round, Rust only: there is no Python worker to differ from, so
//! these scenarios state what must happen). A `wiremock` API hands out one job and takes its result; a `wiremock`
//! Anthropic streams canned answers in turn. Proved: the claim asks for `redline`; the request carries his voice
//! profile, the learned block and the method, in that order, with the marks in Redline's shape and the whole bit in
//! the user turn, word for word as the golden (`tests/goldens/redline_prompt.txt`, written by hand from the spec and
//! read against the code, never generated from it); a draft that alters a Keep by one character, or loses an idea he
//! did not mark, is asked for again with the reasons and, refused twice, is never shown; a provider failure is a
//! `provider_error` result. Each check has a red arm (the `refusals` it rests on, made lenient, lets the bad draft
//! through: `kinds::redline` unit tests and `pk_creative::redline`).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use pk_core::Env;
use pk_llm::{Budget, BudgetConfig, LlmConfig, Router};
use pk_obs::WorkerMetrics;
use pk_worker::kinds::Pages;
use pk_worker::legal::Legal;
use pk_worker::{Parts, RunArgs, Worker, WorkerConfig};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const VOICE: &str = "VOICE PROFILE (synthetic). Deadpan; short sentences; the turn lands on the last word.";

const BIT: &str = "My fridge has an app now.\nIt texts me when the milk is sad.\n\n\
The doorbell has a camera too.\nNow my front door judges my outfits.\n\n\
My parrot only says legal disclaimers.\nHe will not confirm or deny the crackers.\n\n\
The vacuum maps my house.\nIt knows where I cry.";
const KEEP: &str = "My parrot only says legal disclaimers.";

fn job() -> Value {
    let at = |t: &str| BIT.find(t).unwrap();
    json!({"id": "job:redline-test000001", "kind": "redline", "status": "claimed", "attempts": 1,
           "input": {"joke": "joke:fridge", "title": "Smart home", "unit": "unit:v3", "on": null, "n": 1,
                     "base": BIT, "bit": null, "marks": [
               {"type": "change", "text": "the milk is sad", "start": at("the milk is sad"), "note": "sharper"},
               {"type": "ask", "text": "judges my outfits", "start": at("judges my outfits"),
                "note": "Too close to the mirror bit?"},
               {"type": "keep", "text": KEEP, "start": at(KEEP), "note": "", "carried": false}]}})
}

fn good_draft() -> String {
    BIT.replace("when the milk is sad.", "when the milk is sad. We're all sad.")
}

fn answer(draft: &str) -> String {
    json!({
        "answers": [{"item": 2, "text": "Not really: \"judges my outfits\" is about the door, not you."}],
        "notes": [{"item": 1, "was": "when the milk is sad", "why": "a second beat"}],
        "draft": draft,
    })
    .to_string()
}

fn sse(text: &str) -> String {
    let ev = [
        json!({"type": "message_start", "message": {"id": "msg_t", "type": "message", "role": "assistant",
               "model": "claude-opus-5-5", "content": [], "stop_reason": null, "stop_sequence": null,
               "usage": {"input_tokens": 1500, "output_tokens": 1, "cache_creation_input_tokens": 0,
                         "cache_read_input_tokens": 0}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null},
               "usage": {"output_tokens": 300}}),
        json!({"type": "message_stop"}),
    ];
    let mut out = String::new();
    for e in &ev {
        let _ = write!(out, "event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap_or(""));
    }
    out
}

/// The model's answers, one per call in turn (the last one repeats); an empty list is a 400.
struct Anthropic {
    answers: Vec<String>,
    n: AtomicUsize,
}

impl Respond for Anthropic {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        if self.answers.is_empty() {
            return ResponseTemplate::new(400)
                .set_body_json(json!({"type": "error", "error": {"type": "invalid_request_error", "message": "bad"}}));
        }
        let k = self.n.fetch_add(1, Ordering::SeqCst).min(self.answers.len() - 1);
        ResponseTemplate::new(200).set_body_raw(sse(&self.answers[k]), "text/event-stream")
    }
}

struct Api {
    handed: AtomicBool,
}

impl Respond for Api {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let ok = |v: Value| ResponseTemplate::new(200).set_body_json(v);
        match (req.method.as_str(), req.url.path()) {
            ("POST", "/api/worker/claim") => {
                if self.handed.swap(true, Ordering::SeqCst) {
                    ResponseTemplate::new(204)
                } else {
                    ok(json!({"job": job()}))
                }
            }
            ("GET", "/api/worker/profile/voice") => ok(json!({"kind": "voice", "text": VOICE,
                "updated_at": "2026-10-19T08:00:00Z", "by": "owner", "by_kind": "owner", "chars": VOICE.len(),
                "learned": {"kept": [{"text": "My parrot read me my rights.", "bit": null, "date": "2026-10-18"}],
                            "dropped": [], "updated_at": "2026-10-21T10:00:00Z"},
                "laugh_lines": []})),
            ("POST", p) if p.ends_with("/result") => ok(json!({"ok": true})),
            (_, p) => ResponseTemplate::new(404).set_body_json(json!({"error": format!("no route {p}")})),
        }
    }
}

struct Run {
    claims: Vec<Value>,
    result: Value,
    calls: Vec<Value>,
}

async fn run(answers: Vec<String>) -> Run {
    let api = MockServer::start().await;
    let anthropic = MockServer::start().await;
    let pages = MockServer::start().await;
    Mock::given(any())
        .respond_with(Api {
            handed: AtomicBool::new(false),
        })
        .mount(&api)
        .await;
    Mock::given(any())
        .respond_with(Anthropic {
            answers,
            n: AtomicUsize::new(0),
        })
        .mount(&anthropic)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let spend = tmp.path().join("ai_spend.json");
    let vars: BTreeMap<String, String> = BTreeMap::from([
        ("PK_WORKER_NAME".into(), "test-worker".into()),
        (
            "PK_HEARTBEAT_FILE".into(),
            tmp.path().join("heartbeat").display().to_string(),
        ),
        ("PK_AI_SPEND_FILE".into(), spend.display().to_string()),
        ("PK_METRICS_PORT".into(), "0".into()),
        ("PK_COMIC_NAME".into(), "Pat Example".into()),
        ("PK_COMIC_FIRST".into(), "Pat".into()),
    ]);
    let args = RunArgs {
        api: Some(api.uri()),
        no_auth: true,
        once: true,
        idle: 0.0,
        ..RunArgs::default()
    };
    let mut cfg = WorkerConfig::from_env(&Env::from_map(vars), &args).unwrap();
    cfg.stop_grace = std::time::Duration::ZERO;
    let lc = LlmConfig::new(
        BTreeMap::from([("anthropic".to_string(), "sk-test-not-a-key".to_string())]),
        Some(anthropic.uri()),
        BudgetConfig::new(5.0, &spend),
    );
    let router = Router::hosted(pk_llm::Config::hosted(), &lc, None).unwrap();
    let worker = Worker::new(Parts {
        cfg,
        api: pk_worker::api::ApiClient::new(&api.uri(), None).unwrap(),
        router,
        budget: Arc::new(Budget::from_config(&lc.budget)),
        metrics: Arc::new(WorkerMetrics::new("0.1.0", "test").unwrap()),
        legal: Legal::default(),
        pages: Pages::new().unwrap().via(pages.uri()),
        embedder: None,
    });
    worker
        .run(CancellationToken::new())
        .await
        .expect("one job, then a clean end");
    let body = |r: &Request| serde_json::from_slice::<Value>(&r.body).unwrap_or(Value::Null);
    let got = api.received_requests().await.unwrap();
    let claims = got
        .iter()
        .filter(|r| r.url.path() == "/api/worker/claim")
        .map(body)
        .collect();
    let result = got
        .iter()
        .find(|r| r.url.path().ends_with("/result"))
        .map(body)
        .expect("a result was posted");
    let calls = anthropic.received_requests().await.unwrap().iter().map(body).collect();
    Run { claims, result, calls }
}

fn system_texts(call: &Value) -> Vec<String> {
    call["system"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| b["text"].as_str().map(str::to_string))
        .collect()
}

/// The text of a user turn (a string, or the first text block).
fn turn_text(m: &Value) -> String {
    m["content"].as_str().map_or_else(
        || m["content"][0]["text"].as_str().unwrap_or("").to_string(),
        str::to_string,
    )
}

/// What the scenario's own check says of a result: a draft shown whose Keep is not byte for byte, or that lost the
/// vacuum idea (marked nothing at all).
fn shown_but_bad(result: &Value) -> Vec<&'static str> {
    let mut bad = Vec::new();
    if let Some(d) = result["result"]["draft"].as_str() {
        if !d.contains(KEEP) {
            bad.push("a Keep changed");
        }
        if !d.contains("The vacuum maps my house.") {
            bad.push("an unmarked line gone");
        }
    }
    bad
}

#[tokio::test]
async fn a_round_is_one_call_in_redlines_shape_and_a_whole_new_draft_comes_back() {
    let r = run(vec![answer(&good_draft())]).await;
    assert!(
        r.claims[0]["kinds"].as_array().unwrap().contains(&json!("redline")),
        "{}",
        r.claims[0]
    );
    assert_eq!(r.calls.len(), 1, "one model call");
    let call = &r.calls[0];
    assert_eq!(call["model"], "claude-opus-5-5");
    assert_eq!(
        call["max_tokens"], 32000,
        "room for a whole draft (the task's own max_tokens)"
    );
    let sys = system_texts(call);
    assert_eq!(sys.len(), 3, "voice, learned, method: {sys:?}");
    assert!(sys[0].contains("VOICE PROFILE (synthetic)"));
    assert!(sys[1].contains("My parrot read me my rights."));
    assert!(sys[2].starts_with(pk_worker::kinds::redline::SYSTEM) && sys[2].contains("KEEP: those words come back"));
    // the golden prompt: the user turn word for word
    let golden = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens/redline_prompt.txt"),
    )
    .unwrap()
    .replace("\r\n", "\n");
    let user = turn_text(&call["messages"][0]);
    assert_eq!(user, golden, "the user turn differs from the golden");

    let res = &r.result["result"];
    assert_eq!(res["status"], "ok", "{}", r.result);
    assert_eq!(res["draft"], json!(good_draft()));
    assert_eq!(res["tries"], 1);
    assert_eq!(res["mode"], "whole");
    assert_eq!(res["answers"][0]["item"], 2);
    assert_eq!(res["answers"][0]["question"], "Too close to the mirror bit?");
    assert_eq!(res["notes"][0]["was"], "when the milk is sad");
    assert_eq!(res["marks"].as_array().unwrap().len(), 3);
    assert_eq!(res["joke"], "joke:fridge");
    assert_eq!(shown_but_bad(&r.result), Vec::<&str>::new());
}

#[tokio::test]
async fn a_draft_that_changes_a_keep_is_asked_for_again_with_the_reason() {
    let broken = good_draft().replace(KEEP, "My parrot only says legal disclaimers!");
    let r = run(vec![answer(&broken), answer(&good_draft())]).await;
    assert_eq!(r.calls.len(), 2, "asked again once");
    let again = r.calls[1]["messages"].as_array().unwrap();
    assert_eq!(again.len(), 3, "the first turn, the refused answer, the reason");
    let why = turn_text(&again[2]);
    assert!(why.contains("item [3] is KEEP"), "{why}");
    let res = &r.result["result"];
    assert_eq!(
        (res["status"].as_str(), res["tries"].as_i64()),
        (Some("ok"), Some(2)),
        "{}",
        r.result
    );
    assert!(res["draft"].as_str().unwrap().contains(KEEP));
    assert_eq!(shown_but_bad(&r.result), Vec::<&str>::new());
}

#[tokio::test]
async fn a_draft_refused_twice_is_never_shown() {
    let broken = good_draft().replace(KEEP, "My parrot says legal disclaimers.");
    let dropped = good_draft().replace("\n\nThe vacuum maps my house.\nIt knows where I cry.", "");
    for (label, bad) in [("a Keep changed", broken), ("an idea dropped", dropped)] {
        let r = run(vec![answer(&bad)]).await;
        let res = &r.result["result"];
        assert_eq!(res["status"], "refused", "{label}: {}", r.result);
        assert_eq!(res["draft"], Value::Null, "{label}: never shown");
        assert_eq!(res["answers"], json!([]), "{label}");
        assert_eq!(r.calls.len(), 2, "{label}: two tries, no more");
        assert!(res["refused"].as_array().is_some_and(|w| !w.is_empty()), "{label}");
    }
}

#[tokio::test]
async fn the_red_arm_the_scenario_check_names_a_bad_draft_shown() {
    // the scenario's own check over a result that shows the broken draft: it must name it
    let r = run(vec![answer(&good_draft())]).await;
    let mut bad = r.result.clone();
    bad["result"]["draft"] = json!(good_draft().replace(KEEP, "x").replace("vacuum", "robot"));
    assert_eq!(shown_but_bad(&bad), ["a Keep changed", "an unmarked line gone"]);
}

#[tokio::test]
async fn a_provider_failure_is_a_result_that_says_so() {
    let r = run(vec![]).await;
    let res = &r.result["result"];
    assert_eq!(res["status"], "provider_error", "{}", r.result);
    assert!(res["why"].as_str().is_some_and(|w| !w.is_empty()));
    assert_eq!(res["draft"], Value::Null);
}

/// The prompt of a round over a synthetic bit of `chars` characters with three marks, as text files in `PK_DUMP_DIR`
/// (the system's method block, the user turn, and what the model must write back: the whole bit, or the parts), so the
/// cost of a round can be measured with a token counter. Not a check: `cargo test -p pk-worker --test redline --
/// --ignored dump_round_prompts` with `PK_DUMP_DIR` set (the slice's cost numbers were made this way).
#[test]
#[ignore = "a measurement, run by hand"]
fn dump_round_prompts() {
    use pk_creative::redline as rl;
    use pk_worker::kinds::redline::{METHOD, RoundJob, SYSTEM, user_turn};
    let dir = std::path::PathBuf::from(std::env::var("PK_DUMP_DIR").expect("PK_DUMP_DIR"));
    let words = [
        "fridge",
        "parrot",
        "doorbell",
        "landlord",
        "airport",
        "vacuum",
        "wifi",
        "neighbour",
        "dentist",
        "printer",
        "gym",
        "toaster",
        "cousin",
        "elevator",
        "receipt",
        "umbrella",
        "password",
        "voicemail",
        "thermostat",
        "sandwich",
    ];
    for chars in [4_000usize, 40_000] {
        let mut text = String::new();
        let (mut i, mut line) = (0usize, 0usize);
        while text.chars().count() < chars {
            let w = |k: usize| words[(i * 7 + k * 3) % words.len()];
            let _ = write!(
                text,
                "My {} told my {} that the {} was never the problem.",
                w(0),
                w(1),
                w(2)
            );
            line += 1;
            i += 1;
            text.push_str(if line % 5 == 0 { "\n\n" } else { "\n" });
        }
        let base: String = text.trim_end().chars().take(chars).collect();
        let ideas = rl::ideas(&base);
        let first = |k: usize| base[ideas[k].clone()].lines().next().unwrap_or("").to_string();
        let marks = json!([
            {"type": "change", "text": first(0), "note": "sharper"},
            {"type": "ask", "text": first(1), "note": "Is this the same joke as the first one?"},
            {"type": "keep", "text": first(2)}]);
        let job: RoundJob = serde_json::from_value(
            json!({"joke": "joke:x", "title": "Synthetic", "unit": "unit:x", "n": 1, "base": base, "marks": marks}),
        )
        .unwrap();
        let (m, _) = job.marks();
        let whole = rl::is_whole(&job.base);
        let parts = rl::parts(&job.base, &m);
        let out: String = parts
            .iter()
            .map(|r| &job.base[r.clone()])
            .collect::<Vec<_>>()
            .join("\n\n");
        std::fs::write(dir.join(format!("system_{chars}.txt")), format!("{SYSTEM}\n\n{METHOD}")).unwrap();
        std::fs::write(
            dir.join(format!("user_{chars}.txt")),
            user_turn(&job, &m, &parts, whole),
        )
        .unwrap();
        std::fs::write(dir.join(format!("out_{chars}.txt")), out).unwrap();
    }
}
