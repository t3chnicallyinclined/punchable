//! The `rant_draft` kind end to end (the Rant tab's "Write a rant", Rust only: there is no Python worker to differ
//! from, so these scenarios state what must happen instead). A `wiremock` API hands out one job and takes its
//! result; a `wiremock` Anthropic streams a canned answer. Proved: the claim asks for `rant_draft`; the request
//! carries his voice profile, the learned block and the method block, in that order, with the topic and the sources
//! in the user turn; a line claiming to be his that is not in its source word for word never reaches the result;
//! the source's own words are what is kept; a provider failure is a `provider_error` result, not a crash. Each has a
//! red arm.

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

const VOICE: &str = "VOICE PROFILE (synthetic). Deadpan; short sentences; the turn lands on the last word. \
Recurring premises: airports, a parrot that only says legal disclaimers. Moves he names: the flat repeat, the list of \
three where the third is a confession.";

fn job() -> Value {
    json!({"id": "job:rant_draft-test000001", "kind": "rant_draft", "status": "claimed", "attempts": 1,
           "input": {"topic": "self-checkout machines", "topic_id": "topic:abc123", "sources": [
               {"id": "joke:self-checkout", "kind": "bit", "label": "Self-checkout v8",
                "text": "Self-checkout is the store hiring you for free.\nI scanned forty things and nobody clapped."},
               {"id": "performance:p1", "kind": "moment", "label": "On tape",
                "text": "unexpected item in the bagging area, my dignity"}]}})
}

fn answer() -> String {
    json!({"lines": [
        {"role": "hook", "text": "Stop scanning your own groceries.", "his": false, "source": ""},
        {"role": "setup", "text": "Self-checkout is the store hiring you for free.", "his": true, "source": "S1"},
        {"role": "beat", "text": "I scanned 40 things and no one clapped.", "his": true, "source": "S1"},
        {"role": "beat", "text": "You bag. You weigh. You apologise to a machine.", "his": false, "source": ""},
        {"role": "turn", "text": "my dignity", "his": true, "source": "S2"},
        {"role": "button", "text": "I quit. I never worked there.", "his": false, "source": ""}
    ]})
    .to_string()
}

fn sse(text: &str) -> String {
    let ev = [
        json!({"type": "message_start", "message": {"id": "msg_t", "type": "message", "role": "assistant",
               "model": "claude-opus-5-5", "content": [], "stop_reason": null, "stop_sequence": null,
               "usage": {"input_tokens": 1200, "output_tokens": 1, "cache_creation_input_tokens": 0,
                         "cache_read_input_tokens": 0}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null},
               "usage": {"output_tokens": 90}}),
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
}

impl Respond for Anthropic {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        if self.status == 200 {
            ResponseTemplate::new(200).set_body_raw(sse(&answer()), "text/event-stream")
        } else {
            ResponseTemplate::new(self.status)
                .set_body_json(json!({"type": "error", "error": {"type": "invalid_request_error", "message": "bad"}}))
        }
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
                            "dropped": [{"text": "Airports, am I right?", "bit": null, "date": "2026-10-21"}],
                            "updated_at": "2026-10-21T10:00:00Z"},
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

async fn run(anthropic_status: u16) -> Run {
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
            status: anthropic_status,
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

/// The lines a result keeps that claim to be his but are not word for word in the source they name.
fn unverified_his_lines(result: &Value) -> Vec<String> {
    let srcs: BTreeMap<String, String> = job()["input"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["id"].as_str().unwrap().to_string(),
                s["text"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    result["result"]["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| l["his"] == json!(true))
        .filter(|l| {
            let t = l["text"].as_str().unwrap_or("");
            !srcs
                .get(l["source"].as_str().unwrap_or(""))
                .is_some_and(|s| !t.is_empty() && s.contains(t))
        })
        .map(|l| l["text"].as_str().unwrap_or("").to_string())
        .collect()
}

#[tokio::test]
async fn a_draft_is_his_lines_word_for_word_and_new_lines_marked() {
    let r = run(200).await;
    assert!(
        r.claims[0]["kinds"].as_array().unwrap().contains(&json!("rant_draft")),
        "the claim asks for the kind: {}",
        r.claims[0]
    );
    assert_eq!(r.calls.len(), 1, "one model call");
    let call = &r.calls[0];
    assert_eq!(call["model"], "claude-opus-5-5", "Opus, the rant task's routing");
    let sys = system_texts(call);
    assert_eq!(sys.len(), 3, "voice, learned, method: {sys:?}");
    assert!(sys[0].contains("VOICE PROFILE (synthetic)"), "his profile first");
    assert!(
        sys[1].contains("My parrot read me my rights."),
        "the learned block second"
    );
    assert!(
        sys[2].starts_with(pk_worker::kinds::rant_draft::SYSTEM) && sys[2].contains("Beats that escalate"),
        "the method third"
    );
    let user = call["messages"][0]["content"].to_string();
    assert!(user.contains("self-checkout machines") && user.contains("[S1] Self-checkout v8 (bit)"));
    assert!(
        call.get("output_config").is_some() || call.to_string().contains("\"lines\""),
        "the schema is sent"
    );

    assert_eq!(r.result["ok"], true, "{}", r.result);
    let res = &r.result["result"];
    assert_eq!(res["status"], "ok");
    assert_eq!(res["topic_id"], "topic:abc123");
    let lines = res["lines"].as_array().unwrap();
    let texts: Vec<&str> = lines.iter().filter_map(|l| l["text"].as_str()).collect();
    assert_eq!(
        texts,
        [
            "Stop scanning your own groceries.",
            "Self-checkout is the store hiring you for free.",
            "You bag. You weigh. You apologise to a machine.",
            "my dignity",
            "I quit. I never worked there.",
        ],
        "the paraphrase of line 2 never reaches him"
    );
    assert_eq!(res["dropped"], 1);
    assert_eq!(lines[1]["source"], "joke:self-checkout");
    assert_eq!(lines[1]["source_label"], "Self-checkout v8");
    assert_eq!(lines[3]["source"], "performance:p1");
    assert!(
        lines
            .iter()
            .filter(|l| l["his"] == json!(false))
            .all(|l| l["draft"] == json!(true))
    );
    assert_eq!(unverified_his_lines(&r.result), Vec::<String>::new());
}

#[tokio::test]
async fn the_red_arm_the_check_catches_a_line_passed_off_as_his() {
    // the scenario's own check, over the result with the paraphrase put back as his: it must name it
    let r = run(200).await;
    let mut bad = r.result.clone();
    bad["result"]["lines"].as_array_mut().unwrap().push(
        json!({"role": "beat", "text": "I scanned 40 things and no one clapped.", "his": true,
                     "source": "joke:self-checkout"}),
    );
    assert_eq!(unverified_his_lines(&bad), ["I scanned 40 things and no one clapped."]);
}

#[tokio::test]
async fn a_provider_failure_is_a_result_that_says_so() {
    let r = run(400).await;
    let res = &r.result["result"];
    assert_eq!(res["status"], "provider_error", "{}", r.result);
    assert!(res["why"].as_str().is_some_and(|w| !w.is_empty()));
    assert_eq!(res["lines"], json!([]));
}
