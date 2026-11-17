//! The usage meter (SPEC 2026-11-17 §13, phase 1) against a scratch `SurrealDB` 3.3.4 (`surreal start memory`;
//! skipped with a message when the binary or the root password is missing, a failure with `PK_REQUIRE_EXTERNAL` set,
//! as `worker.rs`), through the real router with signed tokens (a throwaway RSA key, the JWKS served in process).
//!
//! What it proves, each with a red arm that must turn its check red:
//! 1. every call the worker posts is recorded, once (a batch posted twice is one row each).
//!    Red arm: a lost row (deleted after the post) fails [`all_recorded`].
//! 2. every call is priced at the page's numbers (read 2026-11-17), worked out here by hand, not through the table.
//!    Red arm: a row priced with the table as it was (Sonnet 5.5 cache hits at $0.20) fails [`priced_right`].
//! 3. `GET /api/admin/usage` answers the pinned owner only: another account, a worker token, no token, and any token
//!    on a server with no owner pinned are refused. Red arm: the single-tenant rule every other route uses (any app
//!    token is him) fails [`gate_holds`].
//!
//! Also: the activity counters and the rollup, the report's numbers, the CSV export, and that a transcript's usage
//! row holds its length and size and never a word of it.

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeySize;
use aws_lc_rs::signature::{KeyPair as _, RSA_PKCS1_SHA256, RsaKeyPair};
use axum::body::Body;
use axum::http::{Request, header};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pk_api::auth::verifier::{Clock, Fetch, Jwks, Verifier};
use pk_api::{ApiConfig, AppState, app};
use pk_core::Env;
use pk_store::{Config, Store, from_db, to_db, vars};
use serde_json::{Value, json};
use tower::ServiceExt;

const ISS: &str = "https://auth.example/realms/punchable";
const AUD: &str = "pk-api";
const T0: u64 = 1_800_000_000;
const OWNER_SUB: &str = "sub-owner-1";

struct Scratch(Child);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn skip(why: &str) {
    assert!(
        std::env::var_os("PK_REQUIRE_EXTERNAL").is_none_or(|v| v.is_empty()),
        "PK_REQUIRE_EXTERNAL is set and the usage test cannot run: {why}"
    );
    eprintln!("SKIP usage: {why}");
}

async fn scratch() -> Option<(Scratch, Env, u16)> {
    let exe = std::env::var_os("PUNCH_SURREAL").map_or_else(
        || {
            std::env::home_dir()
                .unwrap_or_default()
                .join(".punchable")
                .join("bin")
                .join("surreal.exe")
        },
        PathBuf::from,
    );
    if !exe.is_file() {
        skip("no surreal binary (PUNCH_SURREAL or ~/.punchable/bin/surreal.exe)");
        return None;
    }
    let env = Env::from_process();
    let Some(pass) = env.opt("SURREAL_PASS").map(str::to_string) else {
        skip(&format!("no SURREAL_PASS in {}", env.place()));
        return None;
    };
    let user = env.opt("SURREAL_USER").unwrap_or("root").to_string();
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("a free port")
        .port();
    let child = Command::new(&exe)
        .args([
            "start",
            "--log",
            "warn",
            "--bind",
            &format!("127.0.0.1:{port}"),
            "memory",
        ])
        .env("SURREAL_USER", &user)
        .env("SURREAL_PASS", &pass)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("start {}: {e}", exe.display()));
    let mut srv = Scratch(child);
    let deadline = Instant::now() + Duration::from_secs(60);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        if let Some(code) = srv.0.try_wait().expect("try_wait") {
            panic!("surreal exited {code} before listening");
        }
        assert!(Instant::now() < deadline, "surreal did not listen on {port} within 60s");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    Some((srv, env, port))
}

struct Key {
    pair: RsaKeyPair,
}

static KEY: LazyLock<Key> = LazyLock::new(|| Key {
    pair: RsaKeyPair::generate(KeySize::Rsa2048).expect("keygen"),
});

impl Key {
    fn jwk(&self) -> Value {
        let pk = self.pair.public_key();
        let n = URL_SAFE_NO_PAD.encode(pk.modulus().big_endian_without_leading_zero());
        let e = URL_SAFE_NO_PAD.encode(pk.exponent().big_endian_without_leading_zero());
        json!({"kty": "RSA", "kid": "k", "use": "sig", "alg": "RS256", "n": n, "e": e})
    }

    /// A token for `sub`, with the `worker` realm role when `worker`.
    fn token(&self, sub: &str, worker: bool) -> String {
        self.token_for(Some(sub), worker)
    }

    /// [`Key::token`], or a token with no `sub` claim at all.
    fn token_for(&self, sub: Option<&str>, worker: bool) -> String {
        let mut c = json!({"iss": ISS, "aud": [AUD, "account"], "typ": "Bearer", "iat": T0, "exp": T0 + 300});
        if let Some(s) = sub {
            c["sub"] = json!(s);
        }
        if worker {
            c["realm_access"] = json!({"roles": ["worker"]});
        }
        let header = json!({"alg": "RS256", "typ": "JWT", "kid": "k"});
        let si = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap()),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&c).unwrap())
        );
        let mut sig = vec![0; self.pair.public_modulus_len()];
        self.pair
            .sign(&RSA_PKCS1_SHA256, &SystemRandom::new(), si.as_bytes(), &mut sig)
            .expect("sign");
        format!("{si}.{}", URL_SAFE_NO_PAD.encode(sig))
    }
}

fn graph() -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../pk-kb/tests/goldens/fixture.json");
    let f: Value = serde_json::from_str(&std::fs::read_to_string(&p).expect("fixture")).expect("fixture is JSON");
    let mut g = f["graph"].clone();
    for n in g["nodes"]
        .as_array_mut()
        .expect("nodes")
        .iter_mut()
        .filter_map(Value::as_object_mut)
    {
        if n.get("type").and_then(Value::as_str) == Some("list")
            && let Some(t) = n.remove("title")
        {
            n.insert("name".into(), t);
        }
    }
    g
}

/// A loaded scratch database (the fixture graph, as `worker.rs` loads it).
async fn load(env: &Env, port: u16, db: &str) -> (Config, Store) {
    let cfg = Config::scratch(env, port, "punchable_test", db).expect("config");
    let writer = Store::connect(cfg.clone()).await.expect("connect");
    assert!(writer.apply_schema(false).await.expect("schema"));
    let g = graph();
    writer.load_graph(&g).await.expect("load");
    let text = serde_json::to_string(&g).expect("graph text");
    let key = pk_api::tape::srcfile_key("kb/v0/graph.json");
    writer
        .write(
            "CREATE type::record('srcfile', $k) CONTENT $c RETURN id;",
            vars([
                ("k", to_db(&json!(key))),
                ("c", pk_store::SrcFile::new("kb/v0/graph.json", &text).content()),
            ]),
            Some(1),
        )
        .await
        .expect("srcfile");
    writer
        .write(
            "UPDATE meta:state MERGE { gen: 0, loading: false, files_rev: 1, load_info: { bundle: $b, graph: 'kb/v0/graph.json' } } RETURN id;",
            vars([("b", to_db(&json!({"graph": "kb/v0/graph.json", "sha": "test"})))]),
            None,
        )
        .await
        .expect("bundle");
    (cfg, writer)
}

/// The API over the scratch database with the token check on; `owner_sub` pins the owner (or not).
async fn api(cfg: &Config, owner_sub: Option<&str>) -> AppState {
    let mut pairs = vec![
        ("SURREAL_PASS", "x"),
        ("PUBLIC_URL", "https://punchable.io/"),
        ("HOST", "127.0.0.1"),
        ("PK_OWNER_ID", "owner"),
        ("PK_EMBEDDER", "hash"),
        ("PK_VEC_SWEEP_S", "0"),
        ("PK_USAGE_FLUSH_S", "0"),
        ("OIDC_ISSUER", ISS),
    ];
    if let Some(s) = owner_sub {
        pairs.push(("PK_OWNER_SUB", s));
    }
    let doc = json!({"keys": [KEY.jwk()]});
    let fetch: Fetch = Arc::new(move || {
        let d = doc.clone();
        Box::pin(async move { Ok(d) })
    });
    let clock: Clock = Arc::new(|| T0 as f64);
    let verifier = Verifier::new(ISS, AUD, Jwks::with_fetch(fetch, clock.clone()), clock);
    let st = AppState::new(
        ApiConfig::from_env(&Env::from_map(pairs)).expect("api config"),
        Some(verifier),
        pk_obs::ApiMetrics::new("0.1.0", "").expect("metrics"),
        reqwest::Client::new(),
    );
    st.set_store(Some(Store::connect(cfg.clone()).await.expect("connect")));
    st.data().await.expect("first load");
    st
}

async fn send(
    st: &AppState,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, Value, String) {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(v) => {
            let bytes = serde_json::to_vec(&v).unwrap();
            b.header(header::CONTENT_TYPE, "application/json")
                .header(header::CONTENT_LENGTH, bytes.len())
                .body(Body::from(bytes))
                .unwrap()
        }
        None => b.body(Body::empty()).unwrap(),
    };
    let r = app(st.clone()).oneshot(req).await.expect("infallible");
    let status = r.status().as_u16();
    let ctype = r
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap_or("").to_string())
        .unwrap_or_default();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    let v = if ctype.starts_with("application/json") {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    } else {
        Value::String(String::from_utf8_lossy(&bytes).into_owned())
    };
    (status, v, ctype)
}

/// A call as the worker posts it.
fn call(id: &str, kind: &str, model: &str, t: [u64; 4]) -> Value {
    json!({"call_id": id, "at": pk_core::time::now_iso(), "job": format!("job-{id}"), "kind": kind,
           "task": "creative_riff", "model": model,
           "tokens_in": t[0], "tokens_out": t[1], "cache_read": t[2], "cache_write": t[3]})
}

/// The page's prices (<https://platform.claude.com/docs/en/about-claude/pricing>, read 2026-11-17), per million
/// tokens, typed in here: input, output, 5-minute cache write, cache read. Independent of `pk_core::prices`.
fn page_usd(model: &str, t: [u64; 4]) -> f64 {
    let prompt = t[0] + t[2] + t[3];
    let p: [f64; 4] = match model {
        "claude-opus-5-5" => [4.0, 20.0, 5.0, 0.20],
        "claude-sonnet-5-5" => [2.0, 10.0, 2.50, 0.10],
        "claude-haiku-5-5" if prompt > 100_000 => [0.50, 2.50, 0.625, 0.05],
        "claude-haiku-5-5" => [0.10, 0.50, 0.125, 0.01],
        other => panic!("no page price typed in for {other}"),
    };
    (t[0] as f64 * p[0] + t[1] as f64 * p[1] + t[3] as f64 * p[2] + t[2] as f64 * p[3]) / 1e6
}

async fn rows(store: &Store, sql: &str) -> Vec<Value> {
    from_db(store.last(sql, vars([])).await.expect("read"))
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Check 1: every posted call id has exactly one row.
fn all_recorded(posted: &[Value], stored: &[Value]) -> Result<(), String> {
    for c in posted {
        let id = c["call_id"].as_str().unwrap();
        let n = stored
            .iter()
            .filter(|r| r["id"].as_str() == Some(&format!("usage_call:{id}")) || r["id"].as_str() == Some(id))
            .count();
        if n != 1 {
            return Err(format!("call {id}: {n} rows"));
        }
    }
    if stored.len() != posted.len() {
        return Err(format!("{} rows for {} calls", stored.len(), posted.len()));
    }
    Ok(())
}

/// Check 2: each row's USD is the page's.
fn priced_right(stored: &[Value]) -> Result<(), String> {
    for r in stored {
        let t = [
            r["tokens_in"].as_u64().unwrap(),
            r["tokens_out"].as_u64().unwrap(),
            r["cache_read"].as_u64().unwrap(),
            r["cache_write"].as_u64().unwrap(),
        ];
        let want = page_usd(r["model"].as_str().unwrap(), t);
        let got = r["usd"].as_f64().unwrap();
        if (got - want).abs() > 1e-12 {
            return Err(format!("{}: {got} vs the page's {want}", r["id"]));
        }
    }
    Ok(())
}

/// Check 3: the owner reads; every other caller is refused (`answers` = who, status).
fn gate_holds(answers: &[(&str, u16)]) -> Result<(), String> {
    for (who, status) in answers {
        let ok = match *who {
            "owner" => *status == 200,
            "no token" => *status == 401,
            _ => *status == 403,
        };
        if !ok {
            return Err(format!("{who}: {status}"));
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_meter_records_prices_and_reports_to_the_owner_only() {
    let Some((_srv, env, port)) = scratch().await else {
        return;
    };
    let (cfg, store) = load(&env, port, "t_usage").await;
    let st = api(&cfg, Some(OWNER_SUB)).await;
    let owner = KEY.token(OWNER_SUB, false);
    let worker = KEY.token("svc-worker", true);

    // ---- 1. the worker's calls, posted twice
    let calls = vec![
        call("a1b2c3d4e5f6", "riff", "claude-opus-5-5", [2000, 500, 30_000, 3000]),
        call("0a0b0c0d0e0f", "ask_chat", "claude-sonnet-5-5", [800, 200, 12_000, 0]),
        call("abcdef012345", "redline", "claude-haiku-5-5", [60_000, 1000, 50_000, 0]),
    ];
    for _ in 0..2 {
        let (s, v, _) = send(
            &st,
            "POST",
            "/api/worker/usage",
            Some(&worker),
            Some(json!({"calls": calls})),
        )
        .await;
        assert_eq!((s, &v), (200, &json!({"stored": 3})));
    }
    // an app token may not post the worker's calls
    let (s, _, _) = send(
        &st,
        "POST",
        "/api/worker/usage",
        Some(&owner),
        Some(json!({"calls": calls})),
    )
    .await;
    assert_eq!(s, 403);
    // a call id that is not hex is refused, and nothing of the batch is written
    let bad = vec![
        call("a1b2c3d4e5f7", "riff", "claude-opus-5-5", [1, 1, 0, 0]),
        call("x' OR 1", "riff", "m", [1, 1, 0, 0]),
    ];
    let (s, _, _) = send(
        &st,
        "POST",
        "/api/worker/usage",
        Some(&worker),
        Some(json!({"calls": bad})),
    )
    .await;
    assert_eq!(s, 400);

    let stored = rows(&store, "SELECT * FROM usage_call ORDER BY id;").await;
    all_recorded(&calls, &stored).expect("every call recorded once");
    priced_right(&stored).expect("priced at the page's numbers");
    for r in &stored {
        assert_eq!(r["priced"], "2026-11-17");
        assert_eq!(r["account"], "owner");
        assert!(r["job"].as_str().is_some_and(|j| j.starts_with("job-")));
    }

    // red arm 1: a lost row must fail the check
    let mut lost = stored.clone();
    lost.pop();
    assert!(
        all_recorded(&calls, &lost).is_err(),
        "RED ARM: a call not recorded must fail"
    );
    // red arm 2: the Sonnet call priced with the table as it was (cache hits at $0.20) must fail the check
    let mut stale = stored.clone();
    for r in &mut stale {
        if r["model"] == "claude-sonnet-5-5" {
            let t = [800.0, 200.0, 12_000.0];
            r["usd"] = json!((t[0] * 2.0 + t[1] * 10.0 + t[2] * 0.20) / 1e6);
        }
    }
    assert!(priced_right(&stale).is_err(), "RED ARM: a cost priced wrong must fail");

    // ---- activity: two app requests, the health probe (never counted), then the flush and the rollup
    for p in ["/api/labels", "/api/meta", "/api/health"] {
        let (s, _, _) = send(&st, "GET", p, Some(&owner), None).await;
        assert_eq!(s, 200, "{p}");
    }
    let today = pk_core::time::today();
    assert_eq!(st.usage().pending().get(&today).map(|c| c.requests), Some(2));
    st.usage().tick(&store, "owner").await.expect("tick");
    assert!(st.usage().pending().is_empty());
    let day = rows(&store, "SELECT * FROM usage_day;").await;
    assert_eq!(day.len(), 1);
    assert_eq!(day[0]["requests"], 2);
    let want_usd: f64 = calls
        .iter()
        .map(|c| {
            page_usd(
                c["model"].as_str().unwrap(),
                [
                    c["tokens_in"].as_u64().unwrap(),
                    c["tokens_out"].as_u64().unwrap(),
                    c["cache_read"].as_u64().unwrap(),
                    c["cache_write"].as_u64().unwrap(),
                ],
            )
        })
        .sum();
    assert!((day[0]["ai_usd"].as_f64().unwrap() - want_usd).abs() < 1e-9);
    assert_eq!(day[0]["ai_calls"], 3);
    assert!(
        day[0]["storage"]["jokes"].as_i64().unwrap() > 0,
        "the storage sample: {}",
        day[0]["storage"]
    );
    // a second tick adds nothing (the AI totals are recomputed, not added)
    st.usage().tick(&store, "owner").await.expect("tick");
    let again = rows(&store, "SELECT * FROM usage_day;").await;
    assert_eq!(
        (again[0]["requests"].clone(), again[0]["ai_calls"].clone()),
        (json!(2), json!(3))
    );
    let gauges = st.usage().gauges();
    assert!(
        (gauges.usd_month - want_usd).abs() < 1e-9 && gauges.active_days_month >= 1.0 && gauges.requests_today >= 2.0,
        "{gauges:?}"
    );
    let scrape = st.metrics().render().expect("render");
    assert!(scrape.contains("pk_usage_ai_usd_month"), "the gauge is registered");
    for line in scrape.lines().filter(|l| l.starts_with("pk_usage_")) {
        assert!(
            !line.contains("owner") && !line.contains(OWNER_SUB),
            "no account in a series: {line}"
        );
    }

    // ---- 3. the owner's page
    let (s, r, _) = send(&st, "GET", "/api/admin/usage", Some(&owner), None).await;
    assert_eq!(s, 200, "{r}");
    assert!((r["totals"]["ai_usd"].as_f64().unwrap() - want_usd).abs() < 1e-9);
    assert_eq!(r["totals"]["ai_calls"], 3);
    assert!((r["month"]["ai_usd"].as_f64().unwrap() - want_usd).abs() < 1e-9);
    assert_eq!(r["prices"]["read"], "2026-11-17");
    let kinds: Vec<&str> = r["totals"]["by_kind"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|k| k["kind"].as_str())
        .collect();
    assert_eq!(kinds, ["riff", "redline", "ask_chat"], "most expensive first");
    // the page itself is not counted as use
    assert_eq!(st.usage().pending().get(&today).map_or(0, |c| c.requests), 0);
    let (s, csv, ctype) = send(&st, "GET", "/api/admin/usage?format=csv", Some(&owner), None).await;
    assert_eq!((s, ctype.as_str()), (200, "text/csv; charset=utf-8"));
    assert!(
        csv.as_str().unwrap().starts_with("day,ai_usd,ai_calls,requests,"),
        "{csv}"
    );
    let (s, _, _) = send(&st, "GET", "/api/admin/usage?from=2026-13-01", Some(&owner), None).await;
    assert_eq!(s, 400);

    let other = KEY.token("someone-else", false);
    let mut answers = vec![("owner", s_of(&st, Some(&owner)).await)];
    answers.push(("another account", s_of(&st, Some(&other)).await));
    answers.push(("the worker", s_of(&st, Some(&worker)).await));
    answers.push(("no token", s_of(&st, None).await));
    answers.push((
        "a token with no sub",
        s_of(&st, Some(&KEY.token_for(None, false))).await,
    ));
    answers.push((
        "a worker token with the owner's sub",
        s_of(&st, Some(&KEY.token(OWNER_SUB, true))).await,
    ));
    let unpinned = api(&cfg, None).await;
    answers.push(("any token, no owner pinned", s_of(&unpinned, Some(&owner)).await));
    gate_holds(&answers).expect("the owner only");
    let (_, r, _) = send(&st, "GET", "/api/admin/usage", Some(&other), None).await;
    assert_eq!(r["error"], pk_api::auth::NOT_OWNER);
    let (_, r, _) = send(&unpinned, "GET", "/api/admin/usage", Some(&owner), None).await;
    assert_eq!(r["error"], pk_api::auth::OWNER_NOT_PINNED);

    // red arm 3: the single-tenant rule of every other route (any app token is him) must fail the gate's check
    let single_tenant = |who: &str| -> u16 {
        match who {
            "no token" => 401,
            "the worker" | "a worker token with the owner's sub" => 403,
            _ => 200,
        }
    };

    // may this account see the page (the Settings row): the owner yes, another account no, and an unpinned server
    // says so (the app then shows the row, which says the page is not switched on)
    let (s, acc, _) = send(&st, "GET", "/api/usage/access", Some(&owner), None).await;
    assert_eq!((s, acc), (200, json!({"owner": true, "pinned": true})));
    let (_, acc, _) = send(&st, "GET", "/api/usage/access", Some(&other), None).await;
    assert_eq!(acc, json!({"owner": false, "pinned": true}));
    let (_, acc, _) = send(&unpinned, "GET", "/api/usage/access", Some(&owner), None).await;
    assert_eq!(acc, json!({"owner": false, "pinned": false}));
    let leaked: Vec<(&str, u16)> = answers.iter().map(|(w, _)| (*w, single_tenant(w))).collect();
    assert!(
        gate_holds(&leaked).is_err(),
        "RED ARM: another account reading the page must fail"
    );

    // ---- a transcript: its length and size, never a word
    let words = "SENTINEL-WORDS-7f3a the bit about the raccoon";
    // posted twice (the phone's retry): one job, one usage row
    for _ in 0..2 {
        let (s, _, _) = send(
            &st,
            "POST",
            "/api/sets/transcript",
            Some(&owner),
            Some(json!({"segments": [{"text": words, "start": 0, "end": 12.5}], "duration_s": 12.5})),
        )
        .await;
        assert!(s == 200 || s == 202, "transcript: {s}");
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    let media = loop {
        let m = rows(&store, "SELECT * FROM usage_media;").await;
        if !m.is_empty() || Instant::now() > deadline {
            break m;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(media.len(), 1, "one media row");
    let row = &media[0];
    assert_eq!(
        (row["kind"].clone(), row["seconds"].as_f64()),
        (json!("transcript"), Some(12.5))
    );
    assert!(row["bytes"].as_i64().unwrap() > 0);
    let keys: std::collections::BTreeSet<&str> = row.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["account", "at", "bytes", "day", "id", "kind", "seconds"]
            .into_iter()
            .collect()
    );
    assert!(
        !row.to_string().contains("SENTINEL"),
        "no word of the transcript in its usage row"
    );
    st.usage().tick(&store, "owner").await.expect("tick");
    for t in ["usage_day", "usage_call", "usage_media"] {
        let all = rows(&store, &format!("SELECT * FROM {t};")).await;
        assert!(!all.is_empty(), "{t} has rows to check");
        assert!(
            !serde_json::to_string(&all).unwrap().contains("SENTINEL"),
            "{t} holds a word of the transcript"
        );
    }
}

async fn s_of(st: &AppState, token: Option<&str>) -> u16 {
    send(st, "GET", "/api/admin/usage", token, None).await.0
}

/// The scrape has no per-account label even with traffic (a unit check of the registry, no database).
#[test]
fn the_usage_gauges_carry_no_account() {
    let m = pk_obs::ApiMetrics::new("0.1.0", "").expect("metrics");
    m.set_usage_source(|| pk_obs::UsageGauges {
        usd_today: vec![("riff".into(), 0.5)],
        calls_today: vec![("riff".into(), 2.0)],
        usd_month: 3.0,
        active_days_month: 4.0,
        requests_today: 9.0,
    });
    let text = m.render().expect("render");
    assert!(text.contains("pk_usage_ai_usd_today{kind=\"riff\"} 0.5"), "{text}");
    for line in text.lines().filter(|l| l.starts_with("pk_usage_")) {
        assert!(!line.contains("account") && !line.contains("sub="), "{line}");
    }
}
