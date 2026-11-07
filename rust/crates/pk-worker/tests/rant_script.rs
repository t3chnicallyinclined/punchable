//! The Rant tab's script kinds end to end (`docs/design/SPEC-2026-11-06-rant-tab.md` §12.5; Rust only, so these
//! scenarios state what must happen instead of differing from a Python worker). A `wiremock` API hands out one job
//! and takes its result; a `wiremock` Anthropic streams a canned answer.
//!
//! - `rant_line`: the claim asks for the kind; the call carries his voice profile, the learned block and the method,
//!   with the line, its neighbours and the ask in the user turn; three options come back with the AI marks, his own
//!   line back and repeats dropped; a provider failure is a result that says so.
//! - `rant_draft` with `keep` (Draft the rest): his fixed lines come back as his own text, in order, with the new lines
//!   around them; an answer that changed one of them is refused whole, and no line reaches him.
//!
//! Each has a red arm.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

fn line_job() -> Value {
    json!({"id": "job:rant_line-test000001", "kind": "rant_line", "status": "claimed", "attempts": 1,
           "input": {"joke": "joke:rant", "line": "The machine asked me to wait for help.",
                     "before": ["Nobody asked me."], "after": ["I quit."], "ask": "sharper", "say": ""}})
}

fn line_answer() -> Value {
    json!({"options": ["It called a manager. On me.", "The machine asked me to wait for help.",
                       "It put me on hold in person.", "it called a manager, on me", "I got reported by a scale."]})
}

const KEEP: [&str; 2] = ["Nobody asked me.", "I scan, I bag, I check my own ID."];

fn draft_job() -> Value {
    json!({"id": "job:rant_draft-test000002", "kind": "rant_draft", "status": "claimed", "attempts": 1,
           "input": {"topic": "self-checkout machines", "topic_id": "topic:abc123", "keep": KEEP, "sources": [
               {"id": "joke:self-checkout", "kind": "bit", "label": "Self-checkout v8",
                "text": "Self-checkout is the store hiring you for free."}]}})
}

fn draft_answer(changed: bool) -> Value {
    let second = if changed {
        "I scan, I bag, I check my ID."
    } else {
        KEEP[1]
    };
    json!({"lines": [
        {"role": "hook", "text": "Stop scanning your own groceries.", "his": false, "source": ""},
        {"role": "setup", "text": KEEP[0], "his": false, "source": ""},
        {"role": "beat", "text": "Self-checkout is the store hiring you for free.", "his": true, "source": "S1"},
        {"role": "beat", "text": second, "his": false, "source": ""},
        {"role": "button", "text": "I quit. I never worked there.", "his": false, "source": ""}
    ]})
}

fn sse(text: &str) -> String {
    let ev = [
        json!({"type": "message_start", "message": {"id": "msg_t", "type": "message", "role": "assistant",
               "model": "claude-opus-5-5", "content": [], "stop_reason": null, "stop_sequence": null,
               "usage": {"input_tokens": 900, "output_tokens": 1, "cache_creation_input_tokens": 0,
                         "cache_read_input_tokens": 0}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null},
               "usage": {"output_tokens": 60}}),
        json!({"type": "message_stop"}),
    ];
    let mut out = String::new();
    for e in &ev {
        let _ = write!(out, "event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap_or(""));
    }
    out
}

struct Anthropic {
    status: u16,
    answer: Value,
}

impl Respond for Anthropic {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        if self.status == 200 {
            ResponseTemplate::new(200).set_body_raw(sse(&self.answer.to_string()), "text/event-stream")
        } else {
            ResponseTemplate::new(self.status)
                .set_body_json(json!({"type": "error", "error": {"type": "invalid_request_error", "message": "bad"}}))
        }
    }
}

struct Api {
    handed: AtomicBool,
    job: Value,
}

impl Respond for Api {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let ok = |v: Value| ResponseTemplate::new(200).set_body_json(v);
        match (req.method.as_str(), req.url.path()) {
            ("POST", "/api/worker/claim") => {
                if self.handed.swap(true, Ordering::SeqCst) {
                    ResponseTemplate::new(204)
                } else {
                    ok(json!({"job": self.job}))
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

async fn run(job: Value, answer: Value, anthropic_status: u16) -> Run {
    let api = MockServer::start().await;
    let anthropic = MockServer::start().await;
    let pages = MockServer::start().await;
    Mock::given(any())
        .respond_with(Api {
            handed: AtomicBool::new(false),
            job,
        })
        .mount(&api)
        .await;
    Mock::given(any())
        .respond_with(Anthropic {
            status: anthropic_status,
            answer,
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

/// The fixed lines a draft result shows as his that are not his exact text, or are out of order or missing.
fn broken_fixed_lines(result: &Value) -> Vec<String> {
    let fixed: Vec<String> = result["result"]["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| l["fixed"] == json!(true))
        .map(|l| l["text"].as_str().unwrap_or("").to_string())
        .collect();
    let lines_out = !result["result"]["lines"].as_array().is_none_or(Vec::is_empty);
    if lines_out && fixed != KEEP {
        let mut bad = fixed;
        bad.retain(|t| !KEEP.contains(&t.as_str()));
        if bad.is_empty() {
            bad.push("a fixed line is missing or out of order".into());
        }
        return bad;
    }
    Vec::new()
}

#[tokio::test]
async fn claude_on_a_line_offers_three_and_changes_nothing() {
    let r = run(line_job(), line_answer(), 200).await;
    assert!(
        r.claims[0]["kinds"].as_array().unwrap().contains(&json!("rant_line")),
        "the claim asks for the kind: {}",
        r.claims[0]
    );
    assert_eq!(r.calls.len(), 1, "one model call");
    let sys = system_texts(&r.calls[0]);
    assert_eq!(sys.len(), 3, "voice, learned, method: {sys:?}");
    assert!(sys[0].contains("VOICE PROFILE (synthetic)"));
    assert!(sys[1].contains("My parrot read me my rights."));
    assert!(sys[2].starts_with(pk_worker::kinds::rant_line::SYSTEM));
    let user = r.calls[0]["messages"][0]["content"].to_string();
    assert!(user.contains("wait for help") && user.contains("Nobody asked me.") && user.contains("I quit."));
    assert!(user.contains("Make it sharper"), "the ask reaches the prompt");
    let res = &r.result["result"];
    assert_eq!(res["status"], "ok", "{}", r.result);
    assert_eq!(res["joke"], "joke:rant");
    let texts: Vec<&str> = res["options"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o["text"].as_str())
        .collect();
    assert_eq!(
        texts,
        [
            "It called a manager. On me.",
            "It put me on hold in person.",
            "I got reported by a scale."
        ],
        "his own line back and a repeat are not options"
    );
    assert!(
        res["options"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["draft"] == json!(true))
    );
    assert_eq!(res["dropped"], 2);
}

#[tokio::test]
async fn the_red_arm_his_own_line_would_be_an_option_without_the_check() {
    // the same answer checked against no line: his own line comes through as an "option", so the exclusion is what
    // kept it out above
    let (opts, _) = pk_worker::kinds::rant_line::checked_options(Some(&line_answer()), "");
    let texts: Vec<&str> = opts.iter().filter_map(|o| o["text"].as_str()).collect();
    assert!(texts.contains(&"The machine asked me to wait for help."), "{texts:?}");
}

#[tokio::test]
async fn a_line_with_a_provider_failure_is_a_result_that_says_so() {
    let r = run(line_job(), line_answer(), 400).await;
    let res = &r.result["result"];
    assert_eq!(res["status"], "provider_error", "{}", r.result);
    assert_eq!(res["options"], json!([]));
}

#[tokio::test]
async fn draft_the_rest_keeps_his_lines_word_for_word_and_adds_around_them() {
    let r = run(draft_job(), draft_answer(false), 200).await;
    let user = r.calls[0]["messages"][0]["content"].to_string();
    assert!(
        user.contains("FIXED") && user.contains(KEEP[1]),
        "his lines go to the model as fixed"
    );
    let res = &r.result["result"];
    assert_eq!(res["status"], "ok", "{}", r.result);
    let lines = res["lines"].as_array().unwrap();
    let texts: Vec<&str> = lines.iter().filter_map(|l| l["text"].as_str()).collect();
    assert_eq!(
        texts,
        [
            "Stop scanning your own groceries.",
            KEEP[0],
            "Self-checkout is the store hiring you for free.",
            KEEP[1],
            "I quit. I never worked there."
        ]
    );
    assert_eq!(lines[2]["source"], "joke:self-checkout");
    assert_eq!(broken_fixed_lines(&r.result), Vec::<String>::new());
}

#[tokio::test]
async fn draft_the_rest_is_refused_whole_when_claude_changed_one_of_his_lines() {
    let r = run(draft_job(), draft_answer(true), 200).await;
    let res = &r.result["result"];
    assert_eq!(res["status"], "refused", "{}", r.result);
    assert_eq!(res["lines"], json!([]), "nothing reaches him");
    assert_eq!(res["why"], pk_worker::kinds::rant_draft::REFUSED_WHY);
}

#[tokio::test]
async fn the_red_arm_the_check_names_a_changed_fixed_line() {
    // the refused answer, as it would read if it had been let through: the scenario's own check must name it
    let mut bad = json!({"result": {"lines": draft_answer(true)["lines"].clone()}});
    for l in bad["result"]["lines"].as_array_mut().unwrap() {
        if l["text"] == KEEP[0] || l["text"] == "I scan, I bag, I check my ID." {
            l["fixed"] = json!(true);
        }
    }
    assert_eq!(broken_fixed_lines(&bad), ["I scan, I bag, I check my ID."]);
}
