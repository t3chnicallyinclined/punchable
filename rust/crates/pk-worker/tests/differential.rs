//! The T8 differential: every scenario `rust/tools/gen_worker_goldens.py` ran through the Python worker
//! (`python -m worker --hosted --once`), run through this worker against the same fixtures (a `wiremock` API with
//! the generator's answers, a `wiremock` Anthropic streaming the same canned answers, a page server), and compared
//! as parsed JSON (ADR 0007): the exit code, every claim body, every request the API received in order (voice
//! profile, ideas, files with their sha256, the result), every Anthropic request body (the voice and learned system
//! blocks with their `cache_control`, the schema, the model, the effort).
//!
//! The allow-list, each with its reason (`docs/RUST_WORKSTREAM.md` T8):
//! - `claim.kinds`: this worker claims the ported kinds only, in the Python's order.
//! - the workspace-mirror reads (`GET files`, `files/content`, `decisions`, `creative`): for a kind that reads his
//!   notebook they are compared request for request, query strings included; for any other kind the Python mirrors
//!   anyway and this worker does not (asserted absent), and `result.online.refresh` is then not compared.
//! - `result.online.media_deleted`: the Python's count of recordings deleted from its folder (this worker has none).
//! - `call_id` on a writing result: a random id on both sides (checked to be 12 hex).
//! - `run_id` and the `analysis.json` text it is in: a timestamp; the sha256 is compared over the normalised text.
//!   The one-shot Ask's `run_id` (`ask-<time>`) and its `timings` values (seconds; the keys are compared).
//! - the validator's words after `invalid JSON:` (Python `jsonschema` vs the `jsonschema` crate; `pk-llm`'s own
//!   allow-list), in the retry turn and in the failed result.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use pk_core::Env;
use pk_llm::{Budget, BudgetConfig, LlmConfig, Router};
use pk_obs::WorkerMetrics;
use pk_worker::kinds::{Kind, Pages};
use pk_worker::legal::Legal;
use pk_worker::{Parts, RunArgs, Worker, WorkerConfig};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const KEY: &str = "sk-golden-not-a-key";
const MIRROR: [&str; 3] = ["/api/worker/files", "/api/worker/decisions", "/api/worker/creative"];

fn goldens() -> Vec<(String, Value)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens");
    let mut out: Vec<(String, Value)> = std::fs::read_dir(&dir)
        .expect("the goldens folder")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("scenario_"))
        })
        .map(|p| {
            let text = std::fs::read_to_string(&p).expect("a golden");
            (
                p.display().to_string(),
                serde_json::from_str(&text).expect("golden JSON"),
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(
        out.len() >= 14,
        "the scenario goldens are missing: run rust/tools/gen_worker_goldens.py"
    );
    out
}

// ------------------------------------------------------------------ the fixtures (the generator's, in Rust)
fn sse(a: &Value) -> String {
    let text = a["text"].as_str().unwrap_or("");
    let model = a["model"].as_str().unwrap_or("claude-sonnet-5-5");
    let n = |k: &str, d: u64| a.get(k).and_then(Value::as_u64).unwrap_or(d);
    let mut ev = vec![
        json!({"type": "message_start", "message": {"id": "msg_golden", "type": "message",
        "role": "assistant", "model": model, "content": [], "stop_reason": null, "stop_sequence": null,
        "usage": {"input_tokens": n("input_tokens", 1200), "output_tokens": 1,
                  "cache_creation_input_tokens": n("cache_write", 0), "cache_read_input_tokens": n("cache_read", 0)}}}),
    ];
    if !text.is_empty() {
        let chars: Vec<char> = text.chars().collect();
        let (x, y) = chars.split_at(chars.len() / 2);
        let (x, y): (String, String) = (x.iter().collect(), y.iter().collect());
        ev.push(json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}));
        ev.push(json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": x}}));
        ev.push(json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": y}}));
        ev.push(json!({"type": "content_block_stop", "index": 0}));
    }
    let mut delta = json!({"stop_reason": a.get("stop_reason").and_then(Value::as_str).unwrap_or("end_turn"),
                           "stop_sequence": null});
    if let Some(d) = a.get("stop_details") {
        delta["stop_details"] = d.clone();
    }
    ev.push(json!({"type": "message_delta", "delta": delta, "usage": {"output_tokens": n("output_tokens", 90)}}));
    ev.push(json!({"type": "message_stop"}));
    let mut out = String::new();
    for e in &ev {
        let _ = write!(out, "event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap_or(""));
    }
    out
}

struct Anthropic {
    answers: Vec<Value>,
    next: AtomicUsize,
    /// The stop test: the worker is told to stop while this answer is still on its way.
    stop_mid_call: Option<CancellationToken>,
}

impl Respond for Anthropic {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let i = self.next.fetch_add(1, Ordering::SeqCst);
        let fallback = json!({"text": "{}", "model": "claude-sonnet-5-5"});
        let a = self.answers.get(i).unwrap_or(&fallback);
        let r = ResponseTemplate::new(200).set_body_raw(sse(a), "text/event-stream");
        match &self.stop_mid_call {
            Some(stop) => {
                stop.cancel();
                r.set_delay(std::time::Duration::from_secs(2))
            }
            None => r,
        }
    }
}

struct Api {
    sc: Value,
    handed: AtomicBool,
    ideas: AtomicUsize,
}

impl Respond for Api {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let p = req.url.path();
        let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        let ok = |v: Value| ResponseTemplate::new(200).set_body_json(v);
        match (req.method.as_str(), p) {
            ("POST", "/api/worker/claim") => {
                if body.get("paused").is_none() && !self.handed.swap(true, Ordering::SeqCst) {
                    ok(json!({"job": self.sc["job"]}))
                } else {
                    ResponseTemplate::new(204)
                }
            }
            ("GET", "/api/worker/profile/voice") => ok(self.sc["voice"].clone()),
            ("GET", "/api/worker/files") => {
                let files: Vec<Value> = self
                    .notebook()
                    .iter()
                    .map(|(k, v)| {
                        let text = v.as_str().unwrap_or_default();
                        json!({"path": k, "sha256": hex::encode(Sha256::digest(text.as_bytes())), "bytes": text.len()})
                    })
                    .collect();
                let rev = if files.is_empty() { Value::Null } else { json!(3) };
                ok(json!({"files": files, "files_rev": rev, "mode": "db"}))
            }
            ("GET", "/api/worker/files/content") => {
                let rel = query_param(req, "path");
                match self.notebook().get(&rel).and_then(Value::as_str) {
                    Some(text) => ok(
                        json!({"path": rel, "sha256": hex::encode(Sha256::digest(text.as_bytes())),
                                            "text": text}),
                    ),
                    None => ResponseTemplate::new(404).set_body_json(json!({"error": "no such file"})),
                }
            }
            ("GET", "/api/worker/decisions") => {
                let ds = self
                    .sc
                    .get("notebook_decisions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let after: usize = query_param(req, "after").parse().unwrap_or(0);
                let lines: Vec<Value> = ds
                    .iter()
                    .enumerate()
                    .skip(after)
                    .map(|(i, d)| json!({"decision": d, "index": i}))
                    .collect();
                ok(json!({"lines": lines, "total": ds.len()}))
            }
            ("GET", "/api/worker/creative") => {
                let store = query_param(req, "store");
                ok(json!({"lines": self.sc["creative"].get(&store).cloned().unwrap_or_else(|| json!([]))}))
            }
            ("POST", "/api/worker/ideas") => {
                let n = self.ideas.fetch_add(1, Ordering::SeqCst) + 1;
                ok(
                    json!({"ok": true, "indexes": [100 + n], "id": format!("joke:golden-idea-{n}"), "decisions": [],
                          "attach": {"decision": if n % 2 == 1 { "new" } else { "attached" }, "into": null}}),
                )
            }
            ("POST", "/api/worker/files") => {
                let written: Vec<Value> = body["files"]
                    .as_array()
                    .map(|fs| fs.iter().map(|f| f["path"].clone()).collect())
                    .unwrap_or_default();
                ok(
                    json!({"ok": true, "written": written, "removed": [], "unchanged": 0, "synced": null,
                          "files_rev": 7, "actor": body["actor"]}),
                )
            }
            ("POST", p) if p.starts_with("/api/worker/jobs/") && p.ends_with("/result") => {
                if self.sc.get("refuse_ok_result") == Some(&json!(true)) && body["ok"] == json!(true) {
                    return ResponseTemplate::new(413)
                        .set_body_raw(r#"{"error": "body too large"}"#, "application/json");
                }
                ok(
                    json!({"ok": true, "job": {"id": self.sc["job"]["id"], "kind": self.sc["job"]["kind"],
                                              "status": "done", "created_at": null, "done_at": null, "attempts": 1,
                                              "error": null}}),
                )
            }
            _ => ResponseTemplate::new(404).set_body_json(json!({"error": format!("no route {p}")})),
        }
    }
}

impl Api {
    fn notebook(&self) -> Map<String, Value> {
        self.sc
            .get("notebook_files")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }
}

fn query_param(req: &Request, k: &str) -> String {
    req.url
        .query_pairs()
        .find(|(q, _)| q == k)
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

struct PageServer {
    pages: Map<String, Value>,
}

impl Respond for PageServer {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let url = format!("https://{}", req.url.path().trim_start_matches('/'));
        match self.pages.get(&url).and_then(Value::as_str) {
            Some(html) => ResponseTemplate::new(200).set_body_raw(html.to_string(), "text/html; charset=utf-8"),
            None => ResponseTemplate::new(404),
        }
    }
}

// ------------------------------------------------------------------ the normalisation (the allow-list above)
fn run_id() -> Regex {
    Regex::new(r"settext-\d{8}-\d{6}-").unwrap()
}

fn ask_run_id() -> Regex {
    Regex::new(r"ask-\d{8}-\d{6}").unwrap()
}

fn validator_words() -> [(Regex, &'static str); 2] {
    [
        (
            Regex::new(r"(?s)That was not valid: invalid JSON: .*?\. Reply with ONE JSON").unwrap(),
            "That was not valid: invalid JSON: <words>. Reply with ONE JSON",
        ),
        (Regex::new(r"(?s)(; last: invalid JSON: ).*$").unwrap(), "${1}<words>"),
    ]
}

fn normalise(x: &Value) -> Value {
    match x {
        Value::Object(m) => {
            let mut out: Map<String, Value> = m.iter().map(|(k, v)| (k.clone(), normalise(v))).collect();
            if let (Some(Value::String(raw)), Some(Value::String(_))) = (m.get("text"), m.get("sha256"))
                && run_id().is_match(raw)
            {
                let text = out["text"].as_str().unwrap_or_default().to_string();
                out.insert("sha256".into(), json!(hex::encode(Sha256::digest(text.as_bytes()))));
            }
            if let Some(Value::Object(online)) = out.get_mut("online") {
                online.remove("media_deleted");
            }
            // a model call's id is random on both sides (uuid4, 12 hex): present, not compared
            if let Some(id) = out.get_mut("call_id")
                && id
                    .as_str()
                    .is_some_and(|s| s.len() == 12 && s.chars().all(|c| c.is_ascii_hexdigit()))
            {
                *id = json!("<call_id>");
            }
            // the one-shot Ask's stage timings: the keys are compared, the values are the clock
            if let Some(Value::Object(t)) = out.get_mut("timings") {
                for v in t.values_mut() {
                    *v = json!("<s>");
                }
            }
            Value::Object(out)
        }
        Value::Array(xs) => Value::Array(xs.iter().map(normalise).collect()),
        Value::String(s) => {
            let s = ask_run_id().replace_all(s, "ask-<time>").into_owned();
            let mut s = run_id().replace_all(&s, "settext-<time>-").into_owned();
            for (re, with) in validator_words() {
                s = re.replace_all(&s, with).into_owned();
            }
            Value::String(s)
        }
        other => other.clone(),
    }
}

#[allow(clippy::many_single_char_names)]
fn first_difference(a: &Value, b: &Value, at: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys()) {
                match (x.get(k), y.get(k)) {
                    (Some(p), Some(q)) => {
                        if let Some(d) = first_difference(p, q, &format!("{at}.{k}")) {
                            return Some(d);
                        }
                    }
                    (p, q) => return Some(format!("{at}.{k}: rust {p:?} vs python {q:?}")),
                }
            }
            None
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
            .iter()
            .zip(y)
            .enumerate()
            .find_map(|(i, (p, q))| first_difference(p, q, &format!("{at}[{i}]"))),
        _ if a == b => None,
        _ => Some(format!("{at}: rust {a} vs python {b}")),
    }
}

fn same(rust: &Value, python: &Value, what: &str, name: &str) {
    let (r, p) = (normalise(rust), normalise(python));
    if let Some(d) = first_difference(&r, &p, what) {
        panic!("{name}: {d}");
    }
}

// ------------------------------------------------------------------ the run
struct Rig {
    api: MockServer,
    anthropic: MockServer,
    _pages: MockServer,
    _tmp: tempfile::TempDir,
    worker: Arc<Worker>,
}

async fn rig(sc: &Value, stop_mid_call: Option<CancellationToken>) -> Rig {
    let api = MockServer::start().await;
    let anthropic = MockServer::start().await;
    let pages = MockServer::start().await;
    Mock::given(any())
        .respond_with(Api {
            sc: sc.clone(),
            handed: AtomicBool::new(false),
            ideas: AtomicUsize::new(0),
        })
        .mount(&api)
        .await;
    Mock::given(any())
        .respond_with(Anthropic {
            answers: sc["model"].as_array().cloned().unwrap_or_default(),
            next: AtomicUsize::new(0),
            stop_mid_call,
        })
        .mount(&anthropic)
        .await;
    Mock::given(any())
        .respond_with(PageServer {
            pages: sc.get("pages").and_then(Value::as_object).cloned().unwrap_or_default(),
        })
        .mount(&pages)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let spend = tmp.path().join("ai_spend.json");
    if let Some(usd) = sc.get("spent_usd").and_then(Value::as_f64) {
        let ledger = json!({pk_core::time::today(): {"usd": usd, "calls": 1, "tasks": {"creative_riff": usd}}});
        std::fs::write(&spend, ledger.to_string()).unwrap();
    }
    let mut vars: BTreeMap<String, String> = BTreeMap::from([
        ("PK_WORKER_NAME".into(), "golden-worker".into()),
        (
            "PK_HEARTBEAT_FILE".into(),
            tmp.path().join("heartbeat").display().to_string(),
        ),
        ("PK_AI_SPEND_FILE".into(), spend.display().to_string()),
        ("PK_METRICS_PORT".into(), "0".into()),
    ]);
    let mut embedder = bge(sc);
    if sc.get("bge") == Some(&json!(true))
        && let Some(dir) = std::env::var_os("PK_BGE_DIR")
    {
        if sc.get("bad_bge") == Some(&json!(true)) {
            // a folder that passes the presence check but will not load (gen_worker_goldens.py `bad_bge`)
            let bad = tmp.path().join("bad-bge");
            std::fs::create_dir_all(bad.join("onnx")).unwrap();
            std::fs::write(bad.join("onnx").join("model.onnx"), b"not an onnx model\n").unwrap();
            std::fs::copy(
                std::path::Path::new(&dir).join("tokenizer.json"),
                bad.join("tokenizer.json"),
            )
            .unwrap();
            vars.insert("PK_BGE_DIR".into(), bad.display().to_string());
            embedder = Some(pk_worker::kinds::Model::lazy(bad));
        } else {
            vars.insert("PK_BGE_DIR".into(), dir.to_string_lossy().into_owned());
        }
    }
    for (k, v) in sc["env"].as_object().into_iter().flatten() {
        vars.insert(k.clone(), v.as_str().unwrap_or_default().to_string());
    }
    let args = RunArgs {
        api: Some(api.uri()),
        no_auth: true,
        once: true,
        idle: 0.0,
        ..RunArgs::default()
    };
    let mut cfg = WorkerConfig::from_env(&Env::from_map(vars), &args).unwrap();
    cfg.stop_grace = std::time::Duration::ZERO;
    let daily = sc.get("daily_usd").and_then(Value::as_f64).unwrap_or(5.0);
    let lc = LlmConfig::new(
        BTreeMap::from([("anthropic".to_string(), KEY.to_string())]),
        Some(anthropic.uri()),
        BudgetConfig::new(daily, &spend),
    );
    let router = Router::hosted(pk_llm::Config::hosted(), &lc, None).unwrap();
    let worker = Worker::new(Parts {
        cfg,
        api: pk_worker::api::ApiClient::new(&api.uri(), None).unwrap(),
        router,
        budget: Arc::new(Budget::from_config(&lc.budget)),
        metrics: Arc::new(WorkerMetrics::new("0.1.0", "golden").unwrap()),
        legal: Legal::default(),
        pages: Pages::new().unwrap().via(pages.uri()),
        embedder,
    });
    Rig {
        api,
        anthropic,
        _pages: pages,
        _tmp: tmp,
        worker,
    }
}

/// The bge-small model for a scenario that needs it (`"bge": true`), loaded once per test run.
fn bge(sc: &Value) -> Option<pk_worker::kinds::Model> {
    static MODEL: std::sync::OnceLock<Option<pk_worker::kinds::Model>> = std::sync::OnceLock::new();
    if sc.get("bge") != Some(&json!(true)) {
        return None;
    }
    MODEL
        .get_or_init(|| {
            let dir = std::env::var_os("PK_BGE_DIR").map(std::path::PathBuf::from)?;
            Some(pk_worker::kinds::Model::loaded(Arc::new(
                pk_embed::Bge::load_with_threads(&dir, 1).ok()?,
            )))
        })
        .clone()
}

fn body_of(r: &Request) -> Value {
    serde_json::from_slice::<Value>(&r.body).unwrap_or(Value::Null)
}

/// The posted vector file, compared as the Python's within float tolerance: pk-embed and the Python's ONNX Runtime
/// agree to about 1e-6, not to the seventh printed digit, so the rows are checked here and replaced by a marker.
fn vector_file(v: &mut Value) -> Option<&mut Value> {
    v.pointer_mut("/body/files")?
        .as_array_mut()?
        .iter_mut()
        .find(|f| f["path"] == "vectors/extra_summary.json")
}

fn vectors_checked(rust: &mut [Value], python: &mut [Value], name: &str) {
    for (r, p) in rust.iter_mut().zip(python.iter_mut()) {
        let (Some(rf), Some(pf)) = (vector_file(r), vector_file(p)) else {
            continue;
        };
        let parse = |f: &Value| serde_json::from_str::<Value>(f["text"].as_str().unwrap_or("null")).unwrap();
        let (mut rd, mut pd) = (parse(rf), parse(pf));
        let (ro, po) = (
            rd["rows"].as_object().cloned().unwrap(),
            pd["rows"].as_object().cloned().unwrap(),
        );
        assert_eq!(
            ro.keys().collect::<Vec<_>>(),
            po.keys().collect::<Vec<_>>(),
            "{name}: vector rows"
        );
        for (k, rv) in &ro {
            let (a, b) = (rv.as_array().unwrap(), po[k].as_array().unwrap());
            assert_eq!(a.len(), b.len(), "{name}: {k} width");
            let worst = a
                .iter()
                .zip(b)
                .map(|(x, y)| (x.as_f64().unwrap() - y.as_f64().unwrap()).abs())
                .fold(0.0, f64::max);
            assert!(worst <= 1e-5, "{name}: {k} differs by {worst}");
        }
        for (f, d) in [(rf, &mut rd), (pf, &mut pd)] {
            d["rows"] = json!("<rows compared within 1e-5>");
            f["text"] = d.take();
            f["sha256"] = json!("<over the rows above>");
        }
    }
}

async fn run_scenario(name: &str, sc: &Value) {
    // bind the whole rig: its temp dir holds the spend ledger and must live as long as the run
    let rig = rig(sc, None).await;
    let (api, anthropic, worker) = (&rig.api, &rig.anthropic, &rig.worker);
    let exit = i64::from(worker.run(CancellationToken::new()).await.is_err());

    let expect = &sc["expect"];
    assert_eq!(json!(exit), expect["exit"], "{name}: exit code");

    let received = api.received_requests().await.unwrap();
    let claims: Vec<Value> = received
        .iter()
        .filter(|r| r.url.path() == "/api/worker/claim")
        .map(body_of)
        .collect();
    let mut want_claims = expect["claims"].as_array().cloned().unwrap();
    for c in &mut want_claims {
        let ported: Vec<Value> = c["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|k| k.as_str().and_then(Kind::parse).is_some_and(Kind::ported))
            .cloned()
            .collect();
        c["kinds"] = Value::Array(ported);
    }
    same(&Value::Array(claims), &Value::Array(want_claims), "claims", name);

    let reads_notebook = Kind::parse(sc["job"]["kind"].as_str().unwrap_or("")).is_some_and(Kind::reads_notebook);
    let is_mirror = |method: &str, path: &str| method == "GET" && MIRROR.iter().any(|m| path.starts_with(m));
    if !reads_notebook {
        for r in &received {
            assert!(
                !is_mirror(r.method.as_str(), r.url.path()),
                "{name}: this worker read {}",
                r.url.path()
            );
        }
    }
    let mut effect: Vec<Value> = received
        .iter()
        .filter(|r| r.url.path() != "/api/worker/claim")
        .map(|r| {
            json!({"method": r.method.as_str(), "path": r.url.path(), "query": r.url.query().unwrap_or(""),
                   "body": body_of(r)})
        })
        .collect();
    let mut want: Vec<Value> = expect["api"].as_array().cloned().unwrap();
    if !reads_notebook {
        want.retain(|r| !is_mirror(r["method"].as_str().unwrap_or(""), r["path"].as_str().unwrap_or("")));
        for v in effect.iter_mut().chain(want.iter_mut()) {
            if let Some(online) = v.pointer_mut("/body/result/online").and_then(Value::as_object_mut) {
                online.remove("refresh");
            }
        }
    }
    vectors_checked(&mut effect, &mut want, name);
    if sc.get("bad_bge") == Some(&json!(true)) {
        // the load error's words are the runtime's own (onnxruntime's vs ort's): the generator redacts the Python's
        for v in &mut effect {
            if v["body"]["ok"] == json!(false) && v["body"]["error"].is_string() {
                v["body"]["error"] = json!("<the model would not load>");
            }
        }
    }
    same(&Value::Array(effect), &Value::Array(want), "api", name);

    let calls: Vec<Value> = anthropic
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(body_of)
        .collect();
    same(&Value::Array(calls), &expect["anthropic"], "anthropic", name);
}

/// On the runtime the binary ships with (current-thread).
#[tokio::test]
async fn the_rust_worker_does_what_the_python_worker_did() {
    for (name, sc) in goldens() {
        if sc.get("bge") == Some(&json!(true)) && std::env::var_os("PK_BGE_DIR").is_none() {
            // a skipped test is a failed test where the externals must be there (CI, the gate)
            assert!(
                std::env::var_os("PK_REQUIRE_EXTERNAL").is_none(),
                "{name} needs the bge model: set PK_BGE_DIR"
            );
            eprintln!("{name}: skipped, no PK_BGE_DIR");
            continue;
        }
        run_scenario(&name, &sc).await;
    }
}

/// The red arm: a changed system block (his profile block dropped from an Ask request) must fail the comparison.
#[test]
fn a_missing_voice_block_is_caught() {
    let (_, sc) = goldens()
        .into_iter()
        .find(|(n, _)| n.contains("ask_chat_named"))
        .unwrap();
    let mut body = sc["expect"]["anthropic"][0].clone();
    let original = body.clone();
    body["system"].as_array_mut().unwrap().remove(0);
    assert!(first_difference(&normalise(&body), &normalise(&original), "x").is_some());
}

/// SIGTERM mid-job: the model call is still streaming when the stop comes; the job goes back to the queue with
/// `{ok: false, retry: true}`, nothing else is claimed, and the loop ends cleanly. (New behaviour: the Python died
/// and left the claim to the API's stale sweep.)
#[tokio::test]
async fn a_stop_mid_job_hands_the_job_back() {
    let (_, sc) = goldens()
        .into_iter()
        .find(|(n, _)| n.contains("ask_chat_named"))
        .unwrap();
    let stop = CancellationToken::new();
    let r = rig(&sc, Some(stop.clone())).await;
    r.worker.run(stop).await.expect("a stop is a clean end");
    let received = r.api.received_requests().await.unwrap();
    let claims = received.iter().filter(|q| q.url.path() == "/api/worker/claim").count();
    assert_eq!(claims, 1, "no claim after the stop");
    let last = received.last().unwrap();
    assert!(last.url.path().ends_with("/result"));
    assert_eq!(
        body_of(last),
        json!({"ok": false, "error": pk_worker::runner::STOPPED, "retry": true})
    );
    assert_eq!(r.anthropic.received_requests().await.unwrap().len(), 1);
}
