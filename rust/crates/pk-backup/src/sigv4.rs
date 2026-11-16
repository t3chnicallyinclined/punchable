//! AWS Signature Version 4, the S3 flavour (the path is encoded once, `/` kept), as `backup.py` signs: every header
//! handed in is signed. Checked against AWS's published vectors in the tests below.

use std::collections::BTreeMap;
use std::fmt::Write;

use sha2::{Digest, Sha256};

/// SHA-256 of nothing, the payload hash of a request without a body.
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// `urllib.parse.quote(s, safe=...)`: UTF-8, `A-Za-z0-9_.-~` and `safe` kept, the rest `%XX` (upper case).
pub fn quote(s: &str, safe: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'~') || safe.as_bytes().contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// The canonical query string and the URL's query: sorted by the raw pairs, each side quoted.
pub fn query_string(query: &[(String, String)]) -> String {
    let mut q: Vec<&(String, String)> = query.iter().collect();
    q.sort();
    q.iter()
        .map(|(k, v)| format!("{}={}", quote(k, ""), quote(v, "")))
        .collect::<Vec<_>>()
        .join("&")
}

/// HMAC-SHA256 (RFC 2104).
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// `sha256(data)` as lower-case hex.
pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// What a signature needs besides the request line and headers.
#[derive(Debug, Clone, Copy)]
pub struct Creds<'a> {
    /// The access key id.
    pub access_key: &'a str,
    /// The secret access key.
    pub secret_key: &'a str,
    /// `auto` for R2.
    pub region: &'a str,
    /// `s3`.
    pub service: &'a str,
}

/// The `Authorization` header value. `headers` must already hold host and x-amz-date (and x-amz-content-sha256 for S3);
/// every header given is signed. `path` is the raw (unencoded) path.
pub fn sigv4(
    method: &str,
    path: &str,
    query: &[(String, String)],
    headers: &[(String, String)],
    payload_hash: &str,
    creds: &Creds<'_>,
    amz_date: &str,
) -> String {
    let canon_path = quote(path, "/");
    let canon_query = query_string(query);
    let hs: BTreeMap<String, String> = headers
        .iter()
        .map(|(k, v)| {
            (
                k.trim().to_lowercase(),
                v.split_whitespace().collect::<Vec<_>>().join(" "),
            )
        })
        .collect();
    let signed = hs.keys().cloned().collect::<Vec<_>>().join(";");
    let canon_headers = hs.iter().fold(String::new(), |mut acc, (k, v)| {
        let _ = writeln!(acc, "{k}:{v}");
        acc
    });
    let creq = [method, &canon_path, &canon_query, &canon_headers, &signed, payload_hash].join("\n");
    let day = &amz_date[..8];
    let scope = format!("{day}/{}/{}/aws4_request", creds.region, creds.service);
    let sts = ["AWS4-HMAC-SHA256", amz_date, &scope, &sha256_hex(creq.as_bytes())].join("\n");
    let k = hmac_sha256(format!("AWS4{}", creds.secret_key).as_bytes(), day.as_bytes());
    let k = hmac_sha256(&k, creds.region.as_bytes());
    let k = hmac_sha256(&k, creds.service.as_bytes());
    let k = hmac_sha256(&k, b"aws4_request");
    let sig = hex::encode(hmac_sha256(&k, sts.as_bytes()));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={sig}",
        creds.access_key
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // AWS's published expected signatures and RFC 4231 test case 2 (public vectors, not secrets)
    const SIG_SHA256_GET_VANILLA: &str = "5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31";
    const SIG_SHA256_QUERY_ORDER: &str = "b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500";
    const SIG_SHA256_S3_GET_RANGE: &str = "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41";
    const SIG_SHA256_S3_LIST: &str = "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7";
    const HMAC_SHA256_RFC4231_CASE_2: &str = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";
    // AWS's published example keys (the aws-sig-v4-test-suite and the S3 documentation); not secrets
    const SUITE: Creds<'static> = Creds {
        access_key: "AKIDEXAMPLE",
        secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        region: "us-east-1",
        service: "service",
    };
    const S3: Creds<'static> = Creds {
        access_key: "AKIAIOSFODNN7EXAMPLE",
        secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        region: "us-east-1",
        service: "s3",
    };

    fn h(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn empty_hash_constant() {
        assert_eq!(sha256_hex(b""), EMPTY_SHA256);
    }

    #[test]
    fn suite_get_vanilla() {
        let hd = h(&[("Host", "example.amazonaws.com"), ("X-Amz-Date", "20150830T123600Z")]);
        let got = sigv4("GET", "/", &[], &hd, EMPTY_SHA256, &SUITE, "20150830T123600Z");
        assert_eq!(
            got,
            format!(
                "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
                 SignedHeaders=host;x-amz-date, Signature={SIG_SHA256_GET_VANILLA}"
            )
        );
    }

    #[test]
    fn suite_query_order() {
        let hd = h(&[("Host", "example.amazonaws.com"), ("X-Amz-Date", "20150830T123600Z")]);
        let q = h(&[("Param2", "value2"), ("Param1", "value1")]);
        let got = sigv4("GET", "/", &q, &hd, EMPTY_SHA256, &SUITE, "20150830T123600Z");
        assert!(got.ends_with(&format!("Signature={SIG_SHA256_QUERY_ORDER}")));
    }

    #[test]
    fn s3_get_object_range() {
        let hd = h(&[
            ("Host", "examplebucket.s3.amazonaws.com"),
            ("Range", "bytes=0-9"),
            ("x-amz-content-sha256", EMPTY_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ]);
        let got = sigv4("GET", "/test.txt", &[], &hd, EMPTY_SHA256, &S3, "20130524T000000Z");
        assert_eq!(
            got,
            format!(
                "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
                 SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature={SIG_SHA256_S3_GET_RANGE}"
            )
        );
    }

    #[test]
    fn s3_list_objects() {
        let hd = h(&[
            ("Host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-content-sha256", EMPTY_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ]);
        let q = h(&[("max-keys", "2"), ("prefix", "J")]);
        let got = sigv4("GET", "/", &q, &hd, EMPTY_SHA256, &S3, "20130524T000000Z");
        assert!(got.ends_with(&format!("Signature={SIG_SHA256_S3_LIST}")));
    }

    #[test]
    fn hmac_matches_rfc4231_case_2() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(hex::encode(mac), HMAC_SHA256_RFC4231_CASE_2);
    }

    #[test]
    fn quote_matches_python() {
        assert_eq!(quote("a b/c~d", "/"), "a%20b/c~d");
        assert_eq!(quote("a b/c", ""), "a%20b%2Fc");
        assert_eq!(quote("\u{e9}", ""), "%C3%A9");
    }
}
