//! Keycloak access tokens, checked exactly as `api-dev/auth.py` checks them, in the same order and with the same
//! refusal texts (the app shows them): three base64url parts; header `alg` is exactly `RS256`; `kid` names a key in
//! the JWKS (an unknown kid refetches the JWKS, at most every 30 s; the set is also refetched after an hour); the
//! RSASSA-PKCS1-v1_5 SHA-256 signature (`jsonwebtoken` over aws-lc-rs); `iss` exact; `aud` contains the audience;
//! `exp`, `nbf`, `iat` with 60 s of clock skew; `typ`, when present, is `Bearer`.
//!
//! The structural checks are written out rather than left to `jsonwebtoken::decode`, because the texts and their
//! order are the contract: the library's own validation answers different errors in a different order.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig, URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, DecodingKey};
use serde_json::{Map, Value};

/// Clock skew allowed on `exp`, `nbf` and `iat`.
pub const LEEWAY_S: f64 = 60.0;
/// An unknown `kid` refetches the JWKS at most this often (a flood of junk kids cannot hammer Keycloak).
pub const MIN_REFETCH_S: f64 = 30.0;
/// The cached key set is refetched once it is older than this.
pub const JWKS_TTL_S: f64 = 3600.0;
/// A longer bearer value is refused before any parsing.
pub const MAX_TOKEN_CHARS: usize = 16384;
/// The JWKS fetch gives up after this long and keeps the cached keys.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// A JWKS document larger than this is not read.
pub const MAX_JWKS_BYTES: usize = 1 << 20;

const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Python's `urlsafe_b64decode` after padding: any padding, non-zero trailing bits accepted.
const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::URL_SAFE,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

/// Why a token was refused; the `Display` text is what the 401 body says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// Not three base64url parts of JSON objects, or longer than 16 KiB.
    #[error("malformed token")]
    Malformed,
    /// `alg` is not `RS256`.
    #[error("unsupported token algorithm")]
    Algorithm,
    /// No `kid` string in the header.
    #[error("token has no key id")]
    NoKid,
    /// The `kid` is not in the JWKS, even after a refetch.
    #[error("unknown signing key")]
    UnknownKey,
    /// The signature does not verify.
    #[error("bad signature")]
    Signature,
    /// `iss` is not the configured issuer.
    #[error("wrong issuer")]
    Issuer,
    /// The audience is not in `aud`.
    #[error("wrong audience")]
    Audience,
    /// `exp` missing, not a number, or passed (with the skew).
    #[error("token expired")]
    Expired,
    /// `nbf` is still ahead.
    #[error("token not yet valid")]
    NotYetValid,
    /// `iat` is ahead.
    #[error("token issued in the future")]
    IssuedInFuture,
    /// `typ` is present and not `Bearer` (a refresh or ID token).
    #[error("not an access token")]
    NotAccessToken,
}

/// Seconds since the epoch, as the verifier sees them (tests pin it).
pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

/// The system clock.
pub fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64())
    })
}

/// `b64d`: a non-empty string of the base64url alphabet only, decoded. `Err(true)` = a character outside the
/// alphabet (Python's `AuthError`), `Err(false)` = a length no decoder accepts (Python's `binascii.Error`).
fn b64d(s: &str) -> Result<Vec<u8>, bool> {
    if s.is_empty() || s.chars().any(|c| !ALPHABET.contains(c)) {
        return Err(true);
    }
    LENIENT.decode(s).map_err(|_| false)
}

/// One RSA signing key.
#[derive(Clone)]
pub struct RsaKey {
    key: DecodingKey,
}

impl std::fmt::Debug for RsaKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RsaKey")
    }
}

/// The bits of a big-endian unsigned integer.
fn bit_length(be: &[u8]) -> usize {
    let s = be.iter().position(|b| *b != 0).map_or(&[][..], |i| &be[i..]);
    s.first()
        .map_or(0, |b| (s.len() - 1) * 8 + (8 - b.leading_zeros() as usize))
}

/// The usable RS256 keys of a JWKS document by `kid` (`auth.py` `JWKS.refresh`). `None` when the document cannot
/// be read as a key list at all (Python raised, so the cached keys stay).
pub fn parse_jwks(doc: &Value) -> Option<HashMap<String, Arc<RsaKey>>> {
    let items: Vec<&Value> = match doc {
        Value::Object(o) => match o.get("keys") {
            Some(Value::Array(a)) => a.iter().collect(),
            // no list: an empty set; iterating a dict or a str yields keys / characters, none of them a key object
            None | Some(Value::Object(_) | Value::String(_)) => Vec::new(),
            Some(_) => return None,
        },
        _ => Vec::new(),
    };
    let mut keys = HashMap::new();
    for k in items {
        let Value::Object(k) = k else { continue };
        if k.get("kty").and_then(Value::as_str) != Some("RSA") {
            continue;
        }
        let Some(kid) = k.get("kid").and_then(Value::as_str) else {
            continue;
        };
        if !matches!(k.get("use"), None | Some(Value::Null)) && k.get("use").and_then(Value::as_str) != Some("sig") {
            continue;
        }
        if !matches!(k.get("alg"), None | Some(Value::Null)) && k.get("alg").and_then(Value::as_str) != Some("RS256") {
            continue;
        }
        let (Some(Value::String(n)), Some(Value::String(e))) = (k.get("n"), k.get("e")) else {
            continue;
        };
        let (Ok(n), Ok(e)) = (b64d(n), b64d(e)) else {
            continue;
        };
        let e_small = {
            let s: Vec<u8> = e.iter().copied().skip_while(|b| *b == 0).collect();
            s.len() <= 1 && s.first().copied().unwrap_or(0) < 3
        };
        if bit_length(&n) < 2048 || e_small {
            continue;
        }
        keys.insert(
            kid.to_string(),
            Arc::new(RsaKey {
                key: DecodingKey::from_rsa_raw_components(&n, &e),
            }),
        );
    }
    Some(keys)
}

#[derive(Debug)]
struct KeySet {
    keys: Arc<HashMap<String, Arc<RsaKey>>>,
    fetched_at: f64,
    tried_at: f64,
    fetches: u64,
}

/// Where the key set comes from: HTTP in production, a closure in tests.
pub type Fetch = Arc<dyn Fn() -> futures::future::BoxFuture<'static, Result<Value, String>> + Send + Sync>;

/// The signing keys by `kid`, cached; an unknown `kid` refetches, rate-limited.
pub struct Jwks {
    fetch: Fetch,
    clock: Clock,
    state: Arc<RwLock<KeySet>>,
    gate: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for Jwks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jwks")
            .field("fetches", &self.fetches())
            .finish_non_exhaustive()
    }
}

impl Jwks {
    /// Keys fetched from `url` over `client` (one bounded GET, 10 s).
    pub fn http(url: String, client: reqwest::Client, clock: Clock) -> Self {
        let fetch: Fetch = Arc::new(move || {
            let (url, client) = (url.clone(), client.clone());
            Box::pin(async move { fetch_json(&client, &url).await })
        });
        Self::with_fetch(fetch, clock)
    }

    /// Keys from any source.
    pub fn with_fetch(fetch: Fetch, clock: Clock) -> Self {
        Self {
            fetch,
            clock,
            state: Arc::new(RwLock::new(KeySet {
                keys: Arc::new(HashMap::new()),
                fetched_at: 0.0,
                tried_at: f64::NEG_INFINITY,
                fetches: 0,
            })),
            gate: tokio::sync::Mutex::new(()),
        }
    }

    /// How many fetches succeeded.
    pub fn fetches(&self) -> u64 {
        self.read(|s| s.fetches)
    }

    fn read<T>(&self, f: impl FnOnce(&KeySet) -> T) -> T {
        f(&self.state.read().unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    /// The key for `kid`: from the cache, or after one refetch when it is unknown or the set is stale and the last
    /// attempt was at least 30 s ago. A failed fetch keeps the cached keys.
    pub async fn get(&self, kid: &str) -> Option<Arc<RsaKey>> {
        let fresh = |s: &KeySet, now: f64| s.keys.contains_key(kid) && now - s.fetched_at <= JWKS_TTL_S;
        if let Some(k) = self.read(|s| fresh(s, (self.clock)()).then(|| s.keys.get(kid).cloned()).flatten()) {
            return Some(k);
        }
        // one fetch at a time, like the Python's lock; the others wait and read what it got
        let _g = self.gate.lock().await;
        let now = (self.clock)();
        let due = self.read(|s| !fresh(s, now) && now - s.tried_at >= MIN_REFETCH_S);
        if due {
            self.state
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .tried_at = now;
            // its own task, which stores what it got: a client that hangs up mid-fetch cannot waste the 30 s window
            let (fetch, state, clock) = ((self.fetch)(), Arc::clone(&self.state), Arc::clone(&self.clock));
            let task = tokio::spawn(async move {
                if let Ok(doc) = fetch.await
                    && let Some(keys) = parse_jwks(&doc)
                {
                    let mut s = state.write().unwrap_or_else(std::sync::PoisonError::into_inner);
                    s.keys = Arc::new(keys);
                    s.fetched_at = clock();
                    s.fetches += 1;
                }
            });
            let _ = task.await;
        }
        self.read(|s| s.keys.get(kid).cloned())
    }
}

async fn fetch_json(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let resp = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?;
    if resp.content_length().is_some_and(|n| n > MAX_JWKS_BYTES as u64) {
        return Err("JWKS too large".into());
    }
    let body = resp.bytes().await.map_err(|e| e.to_string())?;
    if body.len() > MAX_JWKS_BYTES {
        return Err("JWKS too large".into());
    }
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

/// Checks access tokens against one issuer, one audience and one key set.
pub struct Verifier {
    issuer: String,
    audience: String,
    jwks: Jwks,
    clock: Clock,
}

impl std::fmt::Debug for Verifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Verifier")
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .finish_non_exhaustive()
    }
}

/// A JSON number, or a bool as Python's `isinstance(x, (int, float))` sees it (for `nbf` / `iat`).
fn number_or_bool(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::Bool(b)) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

impl Verifier {
    /// A verifier over `jwks`.
    pub fn new(issuer: impl Into<String>, audience: impl Into<String>, jwks: Jwks, clock: Clock) -> Self {
        Self {
            issuer: issuer.into(),
            audience: audience.into(),
            jwks,
            clock,
        }
    }

    /// The key cache (for its fetch count).
    pub fn jwks(&self) -> &Jwks {
        &self.jwks
    }

    /// The claims of a valid access token, or why it is refused.
    pub async fn verify(&self, token: &str) -> Result<Map<String, Value>, AuthError> {
        if token.matches('.').count() != 2 || token.chars().count() > MAX_TOKEN_CHARS {
            return Err(AuthError::Malformed);
        }
        let mut parts = token.split('.');
        let (Some(h64), Some(p64), Some(s64)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(AuthError::Malformed);
        };
        let parse = |s: &str| -> Result<Value, AuthError> {
            let b = b64d(s).map_err(|_| AuthError::Malformed)?;
            serde_json::from_slice(&b).map_err(|_| AuthError::Malformed)
        };
        let header = parse(h64)?;
        let claims = parse(p64)?;
        let (Value::Object(header), Value::Object(claims)) = (header, claims) else {
            return Err(AuthError::Malformed);
        };
        if header.get("alg").and_then(Value::as_str) != Some("RS256") {
            return Err(AuthError::Algorithm);
        }
        let Some(kid) = header.get("kid").and_then(Value::as_str) else {
            return Err(AuthError::NoKid);
        };
        let key = self.jwks.get(kid).await.ok_or(AuthError::UnknownKey)?;
        // The Python decodes the signature only here; a length no decoder takes raised past its handler and dropped
        // the connection. Here it is the same refusal as a bad character.
        let sig = b64d(s64).map_err(|_| AuthError::Malformed)?;
        let msg = format!("{h64}.{p64}");
        let ok = jsonwebtoken::crypto::verify(
            &URL_SAFE_NO_PAD.encode(&sig),
            msg.as_bytes(),
            &key.key,
            Algorithm::RS256,
        )
        .unwrap_or(false);
        if !ok {
            return Err(AuthError::Signature);
        }
        if claims.get("iss").and_then(Value::as_str) != Some(self.issuer.as_str()) {
            return Err(AuthError::Issuer);
        }
        let has_aud = match claims.get("aud") {
            Some(Value::Array(a)) => a.iter().any(|x| x.as_str() == Some(self.audience.as_str())),
            other => other.and_then(Value::as_str) == Some(self.audience.as_str()),
        };
        if !has_aud {
            return Err(AuthError::Audience);
        }
        let now = (self.clock)();
        match claims.get("exp") {
            Some(Value::Number(n)) if n.as_f64().is_some_and(|exp| now <= exp + LEEWAY_S) => {}
            _ => return Err(AuthError::Expired),
        }
        if number_or_bool(claims.get("nbf")).is_some_and(|nbf| now + LEEWAY_S < nbf) {
            return Err(AuthError::NotYetValid);
        }
        if number_or_bool(claims.get("iat")).is_some_and(|iat| iat > now + LEEWAY_S) {
            return Err(AuthError::IssuedInFuture);
        }
        if claims.get("typ").is_some_and(|t| t.as_str() != Some("Bearer")) {
            return Err(AuthError::NotAccessToken);
        }
        Ok(claims)
    }
}

/// `auth.py` `bearer`: the token of an `Authorization: Bearer <token>` value, or None.
pub fn bearer(header_value: Option<&str>) -> Option<&str> {
    let v = pk_core::py::strip(header_value?);
    let (scheme, rest) = v.split_once(' ')?;
    let tok = pk_core::py::strip(rest);
    (scheme.eq_ignore_ascii_case("bearer") && !tok.is_empty()).then_some(tok)
}

/// The realm roles of a token (`realm_access.roles`, strings only).
pub fn roles(claims: &Map<String, Value>) -> Vec<&str> {
    claims
        .get("realm_access")
        .and_then(|r| r.get("roles"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// The role that marks the hosted worker's client.
pub const WORKER_ROLE: &str = "worker";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bearer_follows_the_python() {
        assert_eq!(bearer(Some("Bearer abc")), Some("abc"));
        assert_eq!(bearer(Some("  bearer   abc  ")), Some("abc"));
        assert_eq!(bearer(Some("BEARER x.y.z")), Some("x.y.z"));
        assert_eq!(bearer(Some("Bearer")), None);
        assert_eq!(bearer(Some("Bearer    ")), None);
        assert_eq!(bearer(Some("Basic abc")), None);
        assert_eq!(bearer(Some("")), None);
        assert_eq!(bearer(None), None);
        assert_eq!(bearer(Some("Bearer\tabc")), None);
    }

    #[test]
    fn b64d_refuses_outside_the_alphabet_and_takes_any_trailing_bits() {
        assert_eq!(b64d(""), Err(true));
        assert_eq!(b64d("ab=c"), Err(true));
        assert_eq!(b64d("a+b"), Err(true));
        assert_eq!(b64d("abcde"), Err(false));
        assert_eq!(b64d("QR").unwrap(), vec![0x41]);
        assert_eq!(b64d("QQ").unwrap(), vec![0x41]);
    }

    #[test]
    fn the_key_list_is_read_as_the_python_reads_it() {
        assert!(parse_jwks(&json!({"keys": 5})).is_none());
        assert!(parse_jwks(&json!({"keys": null})).is_none());
        assert_eq!(parse_jwks(&json!({"keys": {}})).map(|k| k.len()), Some(0));
        assert_eq!(parse_jwks(&json!([1, 2])).map(|k| k.len()), Some(0));
        let small = json!({"keys": [{"kty": "RSA", "kid": "a", "n": "AQAB", "e": "AQAB"}]});
        assert_eq!(parse_jwks(&small).map(|k| k.len()), Some(0));
    }

    #[test]
    fn bit_lengths() {
        assert_eq!(bit_length(&[0, 0, 1]), 1);
        assert_eq!(bit_length(&[0x80, 0]), 16);
        assert_eq!(bit_length(&[]), 0);
    }
}
