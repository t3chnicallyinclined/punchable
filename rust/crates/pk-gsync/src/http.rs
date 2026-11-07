//! One blocking HTTP client for everything gsync talks to (Google, Keycloak, Punchable): a request in, the status,
//! headers and body out. No retries here; each caller keeps the Python's own retry rules.

use std::collections::HashMap;
use std::time::Duration;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

/// A finished exchange.
#[derive(Debug, Clone)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// The reason phrase.
    pub phrase: String,
    /// Header names lower-cased.
    pub headers: HashMap<String, String>,
    /// The body.
    pub body: Vec<u8>,
}

impl Response {
    /// The body as text (lossy), the way `read().decode("utf-8", "replace")` reads it.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// A transport failure: no answer (refused, reset, timed out, DNS).
#[derive(Debug, Clone)]
pub struct Transport {
    /// Whether it was a timeout.
    pub timeout: bool,
    /// The words.
    pub message: String,
}

fn causes(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        let words = c.to_string();
        if !out.contains(&words) {
            out.push_str(": ");
            out.push_str(&words);
        }
        cause = c.source();
    }
    out
}

/// The client.
#[derive(Debug, Clone)]
pub struct Http {
    client: reqwest::blocking::Client,
}

impl Http {
    /// A client whose every request is cut off after `timeout`.
    pub fn new(timeout: Duration) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .user_agent("pk-gsync/1")
            .build()
            .expect("a client with a timeout builds");
        Self { client }
    }

    /// Send one request.
    pub fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, String)],
        body: Option<Vec<u8>>,
    ) -> Result<Response, Transport> {
        let m = reqwest::Method::from_bytes(method.as_bytes()).expect("a valid method");
        let mut req = self.client.request(m, url);
        for (k, v) in headers {
            req = req.header(*k, v);
        }
        if let Some(b) = body {
            req = req.body(b);
        }
        let r = req.send().map_err(|e| Transport {
            timeout: e.is_timeout(),
            message: causes(&e),
        })?;
        let status = r.status();
        let headers = r
            .headers()
            .iter()
            .filter_map(|(k, v)| {
                v.to_str()
                    .ok()
                    .map(|v| (k.as_str().to_ascii_lowercase(), v.to_string()))
            })
            .collect();
        let body = r.bytes().map_err(|e| Transport {
            timeout: e.is_timeout(),
            message: causes(&e),
        })?;
        Ok(Response {
            status: status.as_u16(),
            phrase: status.canonical_reason().unwrap_or_default().to_string(),
            headers,
            body: body.to_vec(),
        })
    }
}

/// `urllib.parse.quote_plus`'s unreserved set: letters, digits and `_.-~`.
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC.remove(b'_').remove(b'.').remove(b'-').remove(b'~');

/// `urllib.parse.quote_plus(s)`.
pub fn quote_plus(s: &str) -> String {
    utf8_percent_encode(s, UNRESERVED).to_string().replace("%20", "+")
}

/// `urllib.parse.quote(s, safe=safe)`: only the characters of `safe` (besides the unreserved ones) stay as they are.
pub fn quote_safe(s: &str, safe: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if safe.contains(ch) {
            out.push(ch);
        } else {
            out.push_str(&utf8_percent_encode(&ch.to_string(), UNRESERVED).to_string());
        }
    }
    out
}

/// `urllib.parse.quote(s)`.
pub fn quote(s: &str) -> String {
    quote_safe(s, "/")
}

/// `urllib.parse.urlencode(pairs)`.
pub fn urlencode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", quote_plus(k), quote_plus(v)))
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_matches_urllib() {
        assert_eq!(quote_plus("text/plain"), "text%2Fplain");
        assert_eq!(quote_plus("a b&c=d'é"), "a+b%26c%3Dd%27%C3%A9");
        assert_eq!(quote_plus("files(id,name)"), "files%28id%2Cname%29");
        assert_eq!(quote_safe("joke:a b/c", ":"), "joke:a%20b%2Fc");
        assert_eq!(quote("kb/gsync/map.json"), "kb/gsync/map.json");
        assert_eq!(urlencode(&[("q", "x y"), ("alt", "json")]), "q=x+y&alt=json");
    }
}
