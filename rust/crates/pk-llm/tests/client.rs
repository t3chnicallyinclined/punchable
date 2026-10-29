//! The client against a `wiremock` Anthropic: the canned streams the generator recorded (a normal answer, a cache
//! hit charged into the budget, a refusal, a cut answer), the retry on invalid JSON and the fallback model after a
//! refusal with the request bodies compared to the Python's, a 429, a 5xx retried, the budget refusing before the
//! call, the non-streaming shape. Never the real API, never a key.

mod common;

use common::{KEY, backend, golden, messages_of, same};
use pk_llm::api::Response;
use pk_llm::{Budget, ChatMessage, Config, LlmError, Router};
use std::fmt::Write as _;

use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const DAY: &str = "2026-10-29";

fn sse(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.to_string(), "text/event-stream")
}

/// A Messages stream answering with one text block, the way the API sends one.
fn answer_sse(model: &str, text: &str) -> String {
    let events = [
        json!({"type": "message_start", "message": {"id": "msg_t", "type": "message", "role": "assistant", "model": model,
            "content": [], "stop_reason": null, "stop_sequence": null, "usage": {"input_tokens": 11, "output_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null}, "usage": {"output_tokens": 7}}),
        json!({"type": "message_stop"}),
    ];
    let mut out = String::new();
    for e in &events {
        let _ = write!(out, "event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap());
    }
    out
}

fn post() -> wiremock::MockBuilder {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(header("x-api-key", KEY))
        .and(header("content-type", "application/json"))
        .and(header("accept", "application/json"))
}

/// Serve the responses in order, the last one for every call after.
async fn serve(responses: &[ResponseTemplate]) -> MockServer {
    let server = MockServer::start().await;
    let n = responses.len();
    for (i, r) in responses.iter().enumerate() {
        let m = post().respond_with(r.clone());
        if i + 1 < n {
            m.up_to_n_times(1).mount(&server).await;
        } else {
            m.mount(&server).await;
        }
    }
    server
}

async fn bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.body_json::<Value>().unwrap())
        .collect()
}

fn make_router(server: &MockServer, budget: Budget) -> Router {
    Router::new(Config::hosted(), vec![backend(Some(&server.uri()), Some(budget))], None)
}

fn budget(dir: &tempfile::TempDir, name: &str) -> Budget {
    Budget::new(5.0, dir.path().join(name)).with_day(DAY)
}

fn two_system_messages() -> Vec<ChatMessage> {
    let g = golden("requests.json");
    let case = g["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "ask_chat/2sys/text")
        .unwrap();
    messages_of(&case["messages"])
}

#[tokio::test]
async fn the_recorded_streams_read_the_same_here() {
    let g = golden("streams.json");
    let cfg = Config::hosted();
    let tcfg = cfg.task("ask_chat").unwrap();
    let dir = tempfile::tempdir().unwrap();
    for s in g.as_array().unwrap() {
        let name = s["name"].as_str().unwrap();
        let server = serve(&[sse(s["sse"].as_str().unwrap())]).await;
        let b = backend(Some(&server.uri()), Some(budget(&dir, &format!("{name}.json"))));
        let got = b
            .call(
                s["model"].as_str().unwrap(),
                &two_system_messages(),
                None,
                &tcfg,
                "ask_chat",
            )
            .await;
        match got {
            Ok(c) => {
                assert!(s["ok"].as_bool().unwrap(), "{name}: python failed, rust answered {c:?}");
                same(
                    name,
                    &json!({"text": c.text, "tokens_in": c.tokens_in, "tokens_out": c.tokens_out,
                    "cache_read": c.cache_read, "cache_write": c.cache_write, "usd": c.usd}),
                    &json!({
                    "text": s["text"], "tokens_in": s["tokens_in"], "tokens_out": s["tokens_out"],
                    "cache_read": s["cache_read"], "cache_write": s["cache_write"], "usd": s["usd"]}),
                );
            }
            Err(e) => {
                assert!(!s["ok"].as_bool().unwrap(), "{name}: python answered, rust failed: {e}");
                assert_eq!(e.kind(), s["error"].as_str().unwrap(), "{name}");
                assert_eq!(e.to_string(), s["plain"].as_str().unwrap(), "{name}");
            }
        }
        // the tokens were spent either way: charged into the ledger before the stop reason is read
        let ledger: Value =
            serde_json::from_str(&std::fs::read_to_string(b.budget().unwrap().path()).unwrap()).unwrap();
        same(&format!("{name}.charged"), &ledger[DAY]["usd"], &s["charged_usd"]);
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "{name}");
    }
}

/// The correction's text holds the parser's words, which differ between Python and Rust: check the frame and
/// mask the middle.
fn mask_correction(body: &mut Value, schema: &Value) {
    let last = body["messages"].as_array_mut().unwrap().last_mut().unwrap();
    let text = last["content"].as_str().unwrap().to_string();
    assert!(text.starts_with("That was not valid: invalid JSON: "), "{text}");
    let tail = format!(". {}", pk_llm::schema::schema_instructions(schema));
    assert!(text.ends_with(&tail), "{text}");
    last["content"] = Value::String("<masked>".into());
}

#[tokio::test]
async fn invalid_json_is_retried_once_with_the_pythons_correction() {
    let g = golden("requests.json");
    let dir = tempfile::tempdir().unwrap();
    for (i, r) in g["retries"].as_array().unwrap().iter().enumerate() {
        let bad = r["bad_answer"].as_str().unwrap();
        let model = "claude-sonnet-5-5";
        let server = serve(&[
            sse(&answer_sse(model, bad)),
            sse(&answer_sse(model, "{\"search_queries\": [\"q\"]}")),
        ])
        .await;
        let router = make_router(&server, budget(&dir, &format!("retry{i}.json")));
        let schema = r["requests"][0]["body"]["output_config"]["format"]["schema"].clone();
        let plan_schema = json!({"type": "object", "properties": {"search_queries": {"type": "array", "items": {"type": "string"}}},
            "required": ["search_queries"], "additionalProperties": true});
        assert_eq!(pk_llm::schema::transform_schema(&plan_schema).unwrap(), schema);
        let c = router
            .complete("ask_plan", &two_system_messages(), Some(&plan_schema), None, None)
            .await
            .unwrap();
        same(&format!("retry{i}.data"), &c.data.clone().unwrap(), &r["data"]);
        same(&format!("retry{i}.attempts"), &json!(c.attempts), &r["attempts"]);
        let mut got = bodies(&server).await;
        let mut want: Vec<Value> = r["requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|q| q["body"].clone())
            .collect();
        assert_eq!(got.len(), want.len(), "retry{i}: calls");
        if got.len() == 2 {
            mask_correction(&mut got[1], &plan_schema);
            mask_correction(&mut want[1], &plan_schema);
        }
        same(&format!("retry{i}.requests"), &json!(got), &json!(want));
        let calls = router.calls();
        assert_eq!(
            calls.iter().map(|c| c.ok).collect::<Vec<_>>(),
            vec![false; got.len() - 1].into_iter().chain([true]).collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn a_refusal_on_opus_is_retried_once_on_the_default_model() {
    let g = golden("requests.json");
    let streams = golden("streams.json");
    let f = &g["fallback"];
    let refusal_sse = streams
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "refusal")
        .unwrap()["sse"]
        .as_str()
        .unwrap();
    let second = answer_sse("claude-sonnet-5-5", "second try");
    let dir = tempfile::tempdir().unwrap();
    let server = serve(&[sse(refusal_sse), sse(&second)]).await;
    let router = make_router(&server, budget(&dir, "fallback.json"));
    let case = g["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "creative_riff/3sys/rich")
        .unwrap();
    let c = router
        .complete("creative_riff", &messages_of(&case["messages"]), None, None, None)
        .await
        .unwrap();
    assert_eq!(c.text, f["text"].as_str().unwrap());
    assert_eq!(c.model, f["model"].as_str().unwrap());
    same("fallback.attempts", &json!(c.attempts), &f["attempts"]);
    let got = bodies(&server).await;
    same(
        "fallback.models",
        &json!(got.iter().map(|b| b["model"].clone()).collect::<Vec<_>>()),
        &f["models"],
    );
    let want: Vec<Value> = f["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["body"].clone())
        .collect();
    same("fallback.requests", &json!(got), &json!(want));

    // both refuse: the plain words reach the caller, never "failed on all providers"
    let server = serve(&[sse(refusal_sse)]).await;
    let router = make_router(&server, budget(&dir, "fallback2.json"));
    let e = router
        .complete("creative_riff", &messages_of(&case["messages"]), None, None, None)
        .await
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "Claude declined this one: the request names a real person. Try rewording."
    );
    assert!(matches!(e, LlmError::Refusal { ref category, .. } if category.as_deref() == Some("general_harms")));
    assert_eq!(bodies(&server).await.len(), 2);

    // a Sonnet task's fallback is its own model: one call, the refusal is immediate
    let server = serve(&[sse(refusal_sse)]).await;
    let router = make_router(&server, budget(&dir, "fallback3.json"));
    let e = router
        .complete("ask_chat", &two_system_messages(), None, None, None)
        .await
        .unwrap_err();
    assert!(e.is_plain());
    assert_eq!(bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn a_429_is_rate_limited_and_retried_when_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let limited = ResponseTemplate::new(429)
        .insert_header("retry-after-ms", "10")
        .set_body_raw(
            r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#,
            "application/json",
        );
    let server = serve(std::slice::from_ref(&limited)).await;
    let router = make_router(&server, budget(&dir, "429.json"));
    let e = router
        .complete("ask_chat", &two_system_messages(), None, None, None)
        .await
        .unwrap_err();
    // the router slides to the next provider; there is none, so the chain fails with the 429 as the last error
    match &e {
        LlmError::Failed { task, attempts, last } => {
            assert_eq!(task, "ask_chat");
            assert_eq!(attempts, &vec![("anthropic".to_string(), "RateLimited".to_string())]);
            assert!(last.starts_with("Error code: 429 - "), "{last}");
        }
        other => panic!("{other}"),
    }
    assert_eq!(bodies(&server).await.len(), 1); // max_retries 0 in the test client

    // with the SDK's two retries, a 429 then a 200 is an answer, two requests on the wire
    let streams = golden("streams.json");
    let server = serve(&[limited, sse(streams[0]["sse"].as_str().unwrap())]).await;
    let client = pk_llm::Client::new(KEY)
        .unwrap()
        .with_base_url(server.uri())
        .with_max_retries(2);
    let pcfg = Config::hosted().providers()["anthropic"].clone();
    let b = pk_llm::AnthropicBackend::new("anthropic", pcfg, client, None);
    let tcfg = Config::hosted().task("ask_chat").unwrap();
    let c = b
        .call("claude-sonnet-5-5", &two_system_messages(), None, &tcfg, "ask_chat")
        .await
        .unwrap();
    assert_eq!(c.text, "Three turns: one, two, three.");
    assert_eq!(c.usd, None); // no budget on this backend
    assert_eq!(bodies(&server).await.len(), 2);
}

#[tokio::test]
async fn a_500_is_an_http_error_and_the_budget_refuses_before_the_wire() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve(&[ResponseTemplate::new(500).set_body_string("boom")]).await;
    let router = make_router(&server, budget(&dir, "500.json"));
    let e = router
        .complete("ask_chat", &two_system_messages(), None, None, None)
        .await
        .unwrap_err();
    assert!(
        matches!(&e, LlmError::Failed { attempts, .. } if attempts[0].1 == "APIStatusError"),
        "{e}"
    );
    assert!(e.to_string().contains("Error code: 500 - boom"), "{e}");

    let server = serve(&[sse("")]).await;
    let router = make_router(&server, Budget::new(0.0, dir.path().join("zero.json")).with_day(DAY));
    let e = router
        .complete("ask_chat", &two_system_messages(), None, None, None)
        .await
        .unwrap_err();
    assert!(matches!(e, LlmError::Budget(_)));
    assert!(
        e.to_string()
            .starts_with("over today's AI budget ($0.00 of $0.00); queued jobs run again after midnight UTC")
    );
    assert_eq!(bodies(&server).await.len(), 0);
    assert_eq!(router.calls().len(), 1);
    assert!(
        router.calls()[0]
            .error
            .as_deref()
            .unwrap()
            .starts_with("BudgetExceeded: over today's AI budget")
    );
}

#[tokio::test]
async fn a_non_streaming_provider_posts_without_stream_and_reads_the_json() {
    let answer: Response = serde_json::from_value(json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-5-5",
        "content": [{"type": "text", "text": "plain"}], "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 3, "output_tokens": 2, "cache_read_input_tokens": 1}
    }))
    .unwrap();
    let server = MockServer::start().await;
    post()
        .respond_with(ResponseTemplate::new(200).set_body_json(&answer))
        .mount(&server)
        .await;
    let mut pcfg = Config::hosted().providers()["anthropic"].clone();
    pcfg.stream = false;
    let client = pk_llm::Client::new(KEY).unwrap().with_base_url(server.uri());
    let b = pk_llm::AnthropicBackend::new("anthropic", pcfg, client, None);
    let tcfg = Config::hosted().task("ask_chat").unwrap();
    let c = b
        .call("claude-sonnet-5-5", &two_system_messages(), None, &tcfg, "ask_chat")
        .await
        .unwrap();
    assert_eq!(
        (c.text.as_str(), c.tokens_in, c.tokens_out, c.cache_read, c.cache_write),
        ("plain", 3, 2, 1, 0)
    );
    let body = &bodies(&server).await[0];
    assert!(body.get("stream").is_none());
    assert_eq!(body["system"].as_array().unwrap().len(), 2);
}
