//! The router without a socket or a database (`tower::ServiceExt::oneshot`): the refusals before any data is read,
//! the preflight, CORS on errors, `/metrics`, the boot answer of `/api/health`, and that a method this service does
//! not answer (a POST on a moved path) goes to the fallback and is never handled here. The answers over real data
//! are the differential's (`rust/tools/diff_api_reads.py`).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use pk_api::auth::verifier::{Clock, Fetch, Jwks, Verifier};
use pk_api::{ApiConfig, AppState, app};
use pk_core::Env;
use serde_json::Value;
use tower::ServiceExt;

const ORIGIN: &str = "https://punchable.io";

fn state(auth: bool) -> AppState {
    let mut env = vec![
        ("SURREAL_PASS", "x"),
        ("PUBLIC_URL", "https://punchable.io/"),
        ("HOST", "127.0.0.1"),
    ];
    if auth {
        env.push(("OIDC_ISSUER", "https://auth.example/realms/punchable"));
    } else {
        env.extend([("PK_AUTH", "0"), ("PK_ALLOW_NO_AUTH", "1")]);
    }
    let cfg = ApiConfig::from_env(&Env::from_map(env)).expect("config");
    let verifier = auth.then(|| {
        let fetch: Fetch = Arc::new(|| Box::pin(async { Err("no Keycloak in this test".to_string()) }));
        let clock: Clock = Arc::new(|| 0.0);
        Verifier::new(
            "https://auth.example/realms/punchable",
            "pk-api",
            Jwks::with_fetch(fetch, clock.clone()),
            clock,
        )
    });
    let metrics = pk_obs::ApiMetrics::new("0.1.0", "").expect("metrics");
    AppState::new(cfg, verifier, metrics, reqwest::Client::new())
}

async fn send(st: &AppState, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let r = app(st.clone()).oneshot(req).await.expect("infallible");
    let (parts, body) = r.into_parts();
    let b = axum::body::to_bytes(body, 1 << 20).await.expect("body").to_vec();
    (parts.status, parts.headers, b)
}

fn get(path: &str) -> axum::http::request::Builder {
    Request::builder().method("GET").uri(path)
}

fn error_of(b: &[u8]) -> String {
    serde_json::from_slice::<Value>(b).expect("json")["error"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn no_token_is_401_with_the_challenge_and_cors() {
    let st = state(true);
    let (s, h, b) = send(
        &st,
        get("/api/labels")
            .header(header::ORIGIN, ORIGIN)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(error_of(&b), "sign in required");
    assert_eq!(
        h[header::WWW_AUTHENTICATE],
        r#"Bearer realm="punchable", error="invalid_token""#
    );
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], ORIGIN);
    assert_eq!(h[header::VARY], "Origin");
    let (s, _, b) = send(
        &st,
        get("/api/meta")
            .header(header::AUTHORIZATION, "Bearer garbage")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        (s, error_of(&b).as_str()),
        (StatusCode::UNAUTHORIZED, "malformed token")
    );
    // Keycloak unreachable and the key unknown: refused, never let through
    let tok = "eyJhbGciOiJSUzI1NiIsImtpZCI6ImsifQ.e30.c2ln";
    let (s, _, b) = send(
        &st,
        get("/api/meta")
            .header(header::AUTHORIZATION, format!("Bearer {tok}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        (s, error_of(&b).as_str()),
        (StatusCode::UNAUTHORIZED, "unknown signing key")
    );
}

#[tokio::test]
async fn the_preflight_is_fixed_and_needs_no_token() {
    let st = state(true);
    let req = Request::builder()
        .method("OPTIONS")
        .uri("/api/labels")
        .header(header::ORIGIN, "http://localhost:8081")
        .body(Body::empty())
        .unwrap();
    let (s, h, b) = send(&st, req).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(b.is_empty());
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:8081");
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_METHODS], pk_api::cors::ALLOW_METHODS);
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_HEADERS], pk_api::cors::ALLOW_HEADERS);
    assert_eq!(h[header::ACCESS_CONTROL_MAX_AGE], "600");
    let req = Request::builder()
        .method("OPTIONS")
        .uri("/api/meta")
        .header(header::ORIGIN, "https://evil.example")
        .body(Body::empty())
        .unwrap();
    let (s, h, _) = send(&st, req).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
}

#[tokio::test]
async fn a_write_on_a_moved_path_is_never_answered_here() {
    // no PK_PYTHON_URL in this test: the fallback answers the Python's 404, which proves the POST did not reach a
    // GET handler (with the Python configured, it is forwarded: the differential writes through it)
    let st = state(false);
    for m in ["POST", "PATCH", "PUT", "DELETE"] {
        let req = Request::builder()
            .method(m)
            .uri("/api/labels")
            .body(Body::empty())
            .unwrap();
        let (s, _, b) = send(&st, req).await;
        assert_eq!((s, error_of(&b).as_str()), (StatusCode::NOT_FOUND, "not found"), "{m}");
    }
    let (s, _, _) = send(&st, get("/api/inbox").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn search_similar_and_the_hybrid_list_are_answered_here() {
    // no PK_PYTHON_URL: forwarded = the fallback's 404; answered here = this service's own auth layer's 401
    let st = state(true);
    for p in [
        "/api/jokes?q=dog&mode=hybrid",
        "/api/jokes?q=dog",
        "/api/jokes?q=%20&mode=hybrid",
        "/api/search?q=dog",
        "/api/search",
        "/api/jokes/joke:a/similar",
        "/api/jokes/joke%3Aa/similar?limit=3",
    ] {
        let (s, _, _) = send(&st, get(p).body(Body::empty()).unwrap()).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{p}");
    }
    let (s, _, _) = send(&st, get("/api/jokes/a/b/similar").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "an id with a slash is forwarded");
}

#[tokio::test]
async fn health_before_any_data_is_the_boot_status() {
    let st = state(true);
    let (s, _, b) = send(&st, get("/api/health?ready=1").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["ready"], false);
    assert_eq!(v["surreal"], false);
    let (s, _, b) = send(
        &st,
        get("/api/labels")
            .header(header::AUTHORIZATION, "Bearer x")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        (s, error_of(&b).as_str()),
        (StatusCode::UNAUTHORIZED, "malformed token"),
        "auth comes first"
    );
    st.start_draining();
    let (s, _, _) = send(&st, get("/api/health?ready=1").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
}

#[test]
fn the_prepared_ingress_moves_exactly_the_paths_answered_here() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../infra/k8s/prepared/t10-api-rs.yaml");
    let text = std::fs::read_to_string(p).expect("the prepared manifest");
    let mut exact: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("pathType: Exact"))
        .filter_map(|l| l.split("path: ").nth(1)?.split(',').next())
        .collect();
    exact.sort_unstable();
    // a path read and written here (`/api/boards`, `/api/settings`, ...) is one Ingress path
    let mut own = pk_api::OWN_PATHS.to_vec();
    own.extend(pk_api::OWN_WRITE_PATHS);
    own.sort_unstable();
    own.dedup();
    assert_eq!(exact, own);
    assert!(
        !text.contains("pathType: Prefix"),
        "no prefix moves until the Python is retired"
    );
    let mut regex: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("pathType: ImplementationSpecific"))
        .filter_map(|l| l.split("path: \"").nth(1)?.split('"').next())
        .collect();
    regex.sort_unstable();
    let mut own: Vec<String> = pk_api::OWN_PATTERNS
        .iter()
        .chain(pk_api::OWN_WRITE_PATTERNS.iter())
        .map(|p| pk_api::ingress_regex(p))
        .collect();
    own.sort_unstable();
    own.dedup();
    assert_eq!(regex, own);
}

/// The write methods a moved write path answers here (the Python's own on that path); every other write method
/// on it is forwarded.
fn write_methods(path: &str) -> &'static [&'static str] {
    match path {
        "/api/settings" | "/api/boards/{id}" | "/api/workingsets/{id}/try/{id2}" => &["PATCH"],
        "/api/profile/voice" => &["PUT"],
        _ => &["POST"],
    }
}

/// Whether a moved write path is read here too (it is in [`pk_api::OWN_PATHS`] or [`pk_api::OWN_PATTERNS`] as well).
fn read_here(path: &str) -> bool {
    pk_api::OWN_PATHS.contains(&path) || pk_api::OWN_PATTERNS.contains(&path)
}

#[tokio::test]
async fn every_moved_path_is_answered_here() {
    let st = state(true);
    let ids = pk_api::OWN_PATTERNS.map(|p| p.replace("{id}", "joke:a%2Fb").replace("{id2}", "joke:c"));
    for p in pk_api::OWN_PATHS.iter().copied().chain(ids.iter().map(String::as_str)) {
        let (s, _, b) = send(&st, get(p).body(Body::empty()).unwrap()).await;
        let answered_here =
            s == StatusCode::UNAUTHORIZED || (p == "/api/health" && s == StatusCode::SERVICE_UNAVAILABLE);
        assert!(answered_here, "{p}: {s} {}", String::from_utf8_lossy(&b));
    }
}

/// T10: the write method the Python has on a moved write path is answered here (the auth check's 401, never the
/// fallback's 404), every other write method on it is forwarded, a GET on it is answered only where the path is a
/// read route too (else forwarded: the Python has no such GET), and the paths beside them stay forwarded.
#[tokio::test]
async fn every_moved_write_path_is_answered_here_for_its_own_methods_only() {
    let st = state(true);
    let patterns: Vec<(&str, String)> = pk_api::OWN_WRITE_PATTERNS
        .iter()
        .map(|p| (*p, p.replace("{id}", "joke:a%2Fb").replace("{id2}", "try:x")))
        .collect();
    for (pattern, p) in pk_api::OWN_WRITE_PATHS
        .iter()
        .map(|p| (*p, (*p).to_string()))
        .chain(patterns)
    {
        let p = p.as_str();
        for m in ["POST", "PATCH", "PUT"] {
            let r = Request::builder().method(m).uri(p).body(Body::empty()).unwrap();
            let (s, _, b) = send(&st, r).await;
            if write_methods(pattern).contains(&m) {
                assert_eq!(s, StatusCode::UNAUTHORIZED, "{m} {p}: {}", String::from_utf8_lossy(&b));
            } else {
                assert_eq!(s, StatusCode::NOT_FOUND, "{m} {p} is forwarded");
            }
        }
        let (s, _, _) = send(&st, get(p).body(Body::empty()).unwrap()).await;
        if read_here(pattern) {
            assert_eq!(s, StatusCode::UNAUTHORIZED, "GET {p} is answered here too");
        } else {
            assert_eq!(s, StatusCode::NOT_FOUND, "GET {p} is forwarded");
        }
        let options = Request::builder()
            .method("OPTIONS")
            .uri(p)
            .header(header::ORIGIN, ORIGIN)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            send(&st, options).await.0,
            StatusCode::NO_CONTENT,
            "OPTIONS {p} is the preflight"
        );
    }
    for p in [
        "/api/merge/candidates",
        "/api/merge/history",
        "/api/jokes/a/b/versions",
        "/api/jokes/x/versions/y",
    ] {
        let post = Request::builder().method("POST").uri(p).body(Body::empty()).unwrap();
        assert_eq!(send(&st, post).await.0, StatusCode::NOT_FOUND, "{p} is forwarded");
    }
}

#[tokio::test]
async fn the_creative_and_model_halves_under_an_id_are_not_answered_here() {
    // no Python configured in these tests: anything forwarded is the fallback's 404, never the auth check's 401
    let st = state(true);
    for p in [
        "/api/sets/set:x/clips/x",
        "/api/jokes/a/b",
        "/api/jokes/",
        "/api/boards/b/items",
        "/api/mysets/match",
        "/api/mysets/s/x",
    ] {
        let (s, _, _) = send(&st, get(p).body(Body::empty()).unwrap()).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{p}");
    }
    let post = Request::builder()
        .method("POST")
        .uri("/api/jokes/joke:a")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        send(&st, post).await.0,
        StatusCode::NOT_FOUND,
        "a POST under a moved id path is forwarded"
    );
    for (m, p) in [("PATCH", "/api/mysets/s"), ("POST", "/api/mysets/match")] {
        let r = Request::builder().method(m).uri(p).body(Body::empty()).unwrap();
        assert_eq!(send(&st, r).await.0, StatusCode::NOT_FOUND, "{m} {p} is forwarded");
    }
    // round 2: a set list's PATCH is answered here (the auth check's 401), beside its GET
    let r = Request::builder()
        .method("PATCH")
        .uri("/api/boards/b")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        send(&st, r).await.0,
        StatusCode::UNAUTHORIZED,
        "PATCH /api/boards/b is answered here"
    );
}

#[tokio::test]
async fn metrics_are_open_and_count_by_route_pattern() {
    let st = state(true);
    let _ = send(&st, get("/api/labels").body(Body::empty()).unwrap()).await;
    let (s, h, b) = send(&st, get("/metrics").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h[header::CONTENT_TYPE], pk_obs::CONTENT_TYPE);
    let text = String::from_utf8(b).unwrap();
    assert!(text.contains(r#"pk_build_info{sha="dev",version="0.1.0"} 1"#), "{text}");
    assert!(text.contains(r#"pk_http_requests_total{method="GET",route="/api/labels",status="401"} 1"#));
    assert!(!text.contains(r#"route="/metrics""#), "the scrape is not app traffic");
}

/// Every accepted connection has Nagle off: a forwarded answer is streamed in several writes, and with Nagle on a
/// small write after the first waits for the client's delayed ACK (T9 round 6). Red arm: a bare
/// `tokio::net::TcpListener::bind` (axum 0.8's default) fails this.
#[tokio::test]
async fn accepted_connections_have_nagle_off() {
    use axum::serve::Listener as _;
    let mut l = pk_api::bind("127.0.0.1:0").await.expect("bind");
    let addr = l.local_addr().expect("addr");
    let _client = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let (accepted, _) = l.accept().await;
    assert!(
        accepted.nodelay().expect("nodelay"),
        "TCP_NODELAY is off on an accepted connection"
    );
}
