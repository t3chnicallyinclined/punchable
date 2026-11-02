//! The token check against a JWKS served by wiremock, with a throwaway RSA key made here (aws-lc-rs) and a pinned
//! clock: every refusal text of `api-dev/auth.py`, the unknown-kid refetch and its 30 s rate limit, the hourly
//! refresh, a failed fetch keeping the cached keys. The same cases run against the Python itself in
//! `rust/tools/diff_api_reads.py`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeySize;
use aws_lc_rs::signature::{KeyPair as _, RSA_PKCS1_SHA256, RsaKeyPair};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pk_api::auth::verifier::{AuthError, Clock, Jwks, Verifier};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ISS: &str = "https://auth.example/realms/punchable";
const AUD: &str = "pk-api";
const T0: u64 = 1_800_000_000;

struct Key {
    kid: &'static str,
    pair: RsaKeyPair,
}

impl Key {
    fn new(kid: &'static str) -> Self {
        Self {
            kid,
            pair: RsaKeyPair::generate(KeySize::Rsa2048).expect("keygen"),
        }
    }

    fn jwk(&self) -> Value {
        let pk = self.pair.public_key();
        let n = URL_SAFE_NO_PAD.encode(pk.modulus().big_endian_without_leading_zero());
        let e = URL_SAFE_NO_PAD.encode(pk.exponent().big_endian_without_leading_zero());
        json!({"kty": "RSA", "kid": self.kid, "use": "sig", "alg": "RS256", "n": n, "e": e})
    }

    #[allow(clippy::needless_pass_by_value)] // the call sites read as json!(...) literals
    fn token_with(&self, header: Value, claims: Value) -> String {
        let si = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap()),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
        );
        let mut sig = vec![0; self.pair.public_modulus_len()];
        self.pair
            .sign(&RSA_PKCS1_SHA256, &SystemRandom::new(), si.as_bytes(), &mut sig)
            .expect("sign");
        format!("{si}.{}", URL_SAFE_NO_PAD.encode(sig))
    }

    fn token(&self, set: &[(&str, Value)]) -> String {
        let mut c = json!({"iss": ISS, "aud": [AUD, "account"], "sub": "u-owner", "typ": "Bearer",
                            "iat": T0, "exp": T0 + 300});
        for (k, v) in set {
            if v.is_null() {
                c.as_object_mut().unwrap().remove(*k);
            } else {
                c[*k] = v.clone();
            }
        }
        self.token_with(json!({"alg": "RS256", "typ": "JWT", "kid": self.kid}), c)
    }
}

static A: LazyLock<Key> = LazyLock::new(|| Key::new("a"));
static B: LazyLock<Key> = LazyLock::new(|| Key::new("b"));

struct Rig {
    server: MockServer,
    now: Arc<AtomicU64>,
    v: Verifier,
}

async fn rig(keys: &[&Key]) -> Rig {
    let server = MockServer::start().await;
    serve(&server, keys).await;
    let now = Arc::new(AtomicU64::new(T0));
    let n = Arc::clone(&now);
    let clock: Clock = Arc::new(move || n.load(Ordering::SeqCst) as f64);
    let jwks = Jwks::http(format!("{}/certs", server.uri()), reqwest::Client::new(), clock.clone());
    Rig {
        v: Verifier::new(ISS, AUD, jwks, clock),
        server,
        now,
    }
}

async fn serve(server: &MockServer, keys: &[&Key]) {
    server.reset().await;
    let doc = json!({"keys": keys.iter().map(|k| k.jwk()).collect::<Vec<_>>()});
    Mock::given(method("GET"))
        .and(path("/certs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(doc))
        .mount(server)
        .await;
}

#[tokio::test]
async fn every_refusal_says_what_the_python_says() {
    let r = rig(&[&A]).await;
    let ok = r.v.verify(&A.token(&[])).await.expect("valid");
    assert_eq!(ok["sub"], "u-owner");
    let cases: Vec<(String, AuthError)> = vec![
        ("a.b".into(), AuthError::Malformed),
        ("a.b.c.d".into(), AuthError::Malformed),
        // the signature part is read only at the signature step, as in the Python: `{}` fails on `alg` first
        ("e30.e30.".into(), AuthError::Algorithm),
        ("$$.e30.x".into(), AuthError::Malformed),
        ("W10.e30.x".into(), AuthError::Malformed),
        (format!("{}.e30.x", "a".repeat(16_400)), AuthError::Malformed),
        (
            A.token_with(json!({"alg": "HS256", "kid": "a"}), json!({})),
            AuthError::Algorithm,
        ),
        (
            A.token_with(json!({"alg": "none", "kid": "a"}), json!({})),
            AuthError::Algorithm,
        ),
        (A.token_with(json!({"alg": "RS256"}), json!({})), AuthError::NoKid),
        (
            A.token_with(json!({"alg": "RS256", "kid": 7}), json!({})),
            AuthError::NoKid,
        ),
        (
            A.token_with(json!({"alg": "RS256", "kid": "zz"}), json!({})),
            AuthError::UnknownKey,
        ),
        (
            B.token_with(
                json!({"alg": "RS256", "kid": "a"}),
                json!({"iss": ISS, "aud": AUD, "exp": T0 + 300}),
            ),
            AuthError::Signature,
        ),
        (
            A.token(&[("iss", json!("https://elsewhere.example/realms/x"))]),
            AuthError::Issuer,
        ),
        (A.token(&[("iss", Value::Null)]), AuthError::Issuer),
        (A.token(&[("aud", json!("account"))]), AuthError::Audience),
        (A.token(&[("aud", json!(["account"]))]), AuthError::Audience),
        (A.token(&[("exp", json!(T0 - 61))]), AuthError::Expired),
        (A.token(&[("exp", Value::Null)]), AuthError::Expired),
        (A.token(&[("exp", json!(true))]), AuthError::Expired),
        (A.token(&[("exp", json!("9999999999"))]), AuthError::Expired),
        (A.token(&[("nbf", json!(T0 + 61))]), AuthError::NotYetValid),
        (A.token(&[("iat", json!(T0 + 61))]), AuthError::IssuedInFuture),
        (A.token(&[("typ", json!("Refresh"))]), AuthError::NotAccessToken),
        (A.token(&[("typ", json!("ID"))]), AuthError::NotAccessToken),
    ];
    for (tok, want) in cases {
        assert_eq!(
            r.v.verify(&tok).await.err(),
            Some(want),
            "{}",
            &tok[..tok.len().min(60)]
        );
    }
    // the edges that pass: a string audience, the 60 s skew, no typ, a bool nbf (Python: an int)
    for tok in [
        A.token(&[("aud", json!(AUD))]),
        A.token(&[("exp", json!(T0 - 60))]),
        A.token(&[("typ", Value::Null)]),
        A.token(&[("nbf", json!(true))]),
        A.token(&[("iat", json!(T0 + 60))]),
    ] {
        assert!(r.v.verify(&tok).await.is_ok());
    }
    assert_eq!(AuthError::Malformed.to_string(), "malformed token");
    assert_eq!(AuthError::UnknownKey.to_string(), "unknown signing key");
}

#[tokio::test]
async fn a_signature_is_bound_to_the_whole_token() {
    let r = rig(&[&A]).await;
    let tok = A.token(&[]);
    let mut parts: Vec<&str> = tok.split('.').collect();
    let other = A.token(&[("sub", json!("someone-else"))]);
    let payload = other.split('.').nth(1).unwrap().to_string();
    parts[1] = &payload;
    assert_eq!(r.v.verify(&parts.join(".")).await.err(), Some(AuthError::Signature));
}

#[tokio::test]
async fn an_unknown_kid_refetches_at_most_every_30_s() {
    let r = rig(&[&A]).await;
    assert!(r.v.verify(&A.token(&[])).await.is_ok());
    assert_eq!(r.v.jwks().fetches(), 1);
    // Keycloak rotates: B appears. The first B token refetches once and works.
    serve(&r.server, &[&A, &B]).await;
    r.now.fetch_add(31, Ordering::SeqCst);
    let b = B.token(&[("iat", json!(T0 + 31)), ("exp", json!(T0 + 331))]);
    assert!(r.v.verify(&b).await.is_ok());
    assert_eq!(r.v.jwks().fetches(), 2);
    // a flood of junk kids inside 30 s: no fetch
    let junk = A.token_with(json!({"alg": "RS256", "kid": "junk"}), json!({}));
    for _ in 0..20 {
        assert_eq!(r.v.verify(&junk).await.err(), Some(AuthError::UnknownKey));
    }
    assert_eq!(r.v.jwks().fetches(), 2);
    assert_eq!(
        r.server.received_requests().await.unwrap().len(),
        1,
        "the reset server saw one fetch"
    );
}

#[tokio::test]
async fn a_failed_fetch_keeps_the_cached_keys_and_the_set_refreshes_hourly() {
    let r = rig(&[&A]).await;
    assert!(r.v.verify(&A.token(&[])).await.is_ok());
    r.server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&r.server)
        .await;
    r.now.fetch_add(3601, Ordering::SeqCst);
    let late = A.token(&[("iat", json!(T0 + 3601)), ("exp", json!(T0 + 3900))]);
    assert!(
        r.v.verify(&late).await.is_ok(),
        "Keycloak down: the cached key still verifies"
    );
    assert_eq!(
        r.server.received_requests().await.unwrap().len(),
        1,
        "the stale set was refetched"
    );
    assert_eq!(r.v.jwks().fetches(), 1);
}

#[tokio::test]
async fn keys_the_python_would_skip_are_skipped() {
    let server = MockServer::start().await;
    let mut weak = A.jwk();
    weak["kid"] = json!("enc");
    weak["use"] = json!("enc");
    let mut es = A.jwk();
    es["kid"] = json!("es");
    es["alg"] = json!("ES256");
    let mut nokty = A.jwk();
    nokty["kid"] = json!("oct");
    nokty["kty"] = json!("oct");
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"keys": [weak, es, nokty]})))
        .mount(&server)
        .await;
    let clock: Clock = Arc::new(|| T0 as f64);
    let jwks = Jwks::http(format!("{}/certs", server.uri()), reqwest::Client::new(), clock);
    for kid in ["enc", "es", "oct"] {
        assert!(jwks.get(kid).await.is_none(), "{kid}");
    }
}
