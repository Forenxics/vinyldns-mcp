//! AWS Signature Version 4 request signing, as verified by the VinylDNS API.
//!
//! VinylDNS authenticates API calls with an AWS SigV4-style `Authorization`
//! header (see `Aws4Authenticator.scala` in the VinylDNS repository). The
//! server re-derives the signature from the request, so the canonical form
//! produced here must match the server's exactly:
//!
//! * the path is the *decoded* request path;
//! * query parameters are decoded, re-encoded with RFC 3986 unreserved
//!   characters left as-is, and sorted by `(key, value)`;
//! * only the headers listed in `SignedHeaders` are canonicalised. We sign
//!   `host` and `x-amz-date`, which the server reads verbatim from the request.
//!
//! The region and service name are part of the credential scope but are not
//! checked by VinylDNS, so any stable values work.

use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use sha2::{Digest, Sha256};
use url::Url;

const ALGORITHM: &str = "AWS4-HMAC-SHA256";
const SIGNED_HEADERS: &str = "host;x-amz-date";

/// Characters that are percent-encoded in query strings: everything except
/// the RFC 3986 unreserved set (`A-Z a-z 0-9 - _ . ~`).
pub const QUERY_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

/// Encodes a single query-string component the way the VinylDNS server does.
pub fn encode_component(s: &str) -> String {
    utf8_percent_encode(s, QUERY_ENCODE_SET).to_string()
}

/// Headers to attach to a signed request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedHeaders {
    /// Value for the `Authorization` header.
    pub authorization: String,
    /// Value for the `X-Amz-Date` header.
    pub x_amz_date: String,
}

/// Signs requests with a VinylDNS access key and secret key.
#[derive(Clone)]
pub struct Signer {
    access_key: String,
    secret_key: String,
    region: String,
    service: String,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer")
            .field("access_key", &self.access_key)
            .field("secret_key", &"<redacted>")
            .field("region", &self.region)
            .field("service", &self.service)
            .finish()
    }
}

impl Signer {
    pub fn new(
        access_key: impl Into<String>,
        secret_key: impl Into<String>,
        region: impl Into<String>,
        service: impl Into<String>,
    ) -> Self {
        Self {
            access_key: access_key.into(),
            secret_key: secret_key.into(),
            region: region.into(),
            service: service.into(),
        }
    }

    /// Produces the `Authorization` and `X-Amz-Date` headers for a request.
    pub fn sign(&self, method: &str, url: &Url, body: &[u8], now: DateTime<Utc>) -> SignedHeaders {
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date = now.format("%Y%m%d").to_string();
        let scope = format!("{date}/{}/{}/aws4_request", self.region, self.service);

        let canonical = canonical_request(method, url, &amz_date, body);
        let string_to_sign = format!("{ALGORITHM}\n{amz_date}\n{scope}\n{}", sha256_hex(canonical.as_bytes()));

        let mut key = format!("AWS4{}", self.secret_key).into_bytes();
        for part in scope.split('/') {
            key = hmac_sha256(&key, part.as_bytes());
        }
        let signature = hex::encode(hmac_sha256(&key, string_to_sign.as_bytes()));

        SignedHeaders {
            authorization: format!(
                "{ALGORITHM} Credential={}/{scope}, SignedHeaders={SIGNED_HEADERS}, Signature={signature}",
                self.access_key
            ),
            x_amz_date: amz_date,
        }
    }
}

/// Value of the `Host` header an HTTP client sends for `url`
/// (the port is included only when it is not the scheme default).
pub fn host_header(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

/// Builds the SigV4 canonical request string.
pub fn canonical_request(method: &str, url: &Url, amz_date: &str, body: &[u8]) -> String {
    let path = percent_decode_str(url.path()).decode_utf8_lossy();
    let path = if path.is_empty() { "/".into() } else { path };

    let mut params: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (encode_component(&k), encode_component(&v)))
        .collect();
    params.sort();
    let query = params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");

    format!(
        "{method}\n{path}\n{query}\nhost:{}\nx-amz-date:{amz_date}\n\n{SIGNED_HEADERS}\n{}",
        host_header(url),
        sha256_hex(body)
    )
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2015, 8, 30, 12, 36, 0).unwrap()
    }

    /// The `get-vanilla` case from the AWS SigV4 test suite.
    #[test]
    fn matches_aws_get_vanilla_test_vector() {
        let signer = Signer::new(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "service",
        );
        let url = Url::parse("https://example.amazonaws.com/").unwrap();
        let signed = signer.sign("GET", &url, b"", fixed_time());

        assert_eq!(signed.x_amz_date, "20150830T123600Z");
        assert_eq!(
            signed.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
             SignedHeaders=host;x-amz-date, \
             Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    #[test]
    fn query_parameters_are_decoded_reencoded_and_sorted() {
        let url = Url::parse("http://localhost:9000/zones?nameFilter=a%20b*&maxItems=10&b=~x").unwrap();
        let canonical = canonical_request("GET", &url, "20150830T123600Z", b"");
        let lines: Vec<&str> = canonical.lines().collect();
        assert_eq!(lines[0], "GET");
        assert_eq!(lines[1], "/zones");
        assert_eq!(lines[2], "b=~x&maxItems=10&nameFilter=a%20b%2A");
        assert_eq!(lines[3], "host:localhost:9000");
    }

    #[test]
    fn host_header_omits_default_port() {
        assert_eq!(
            host_header(&Url::parse("https://dns.example.com:443/x").unwrap()),
            "dns.example.com"
        );
        assert_eq!(
            host_header(&Url::parse("http://localhost:9000/x").unwrap()),
            "localhost:9000"
        );
    }

    /// Cross-checked against an independent Python port of the VinylDNS
    /// server's `Aws4Authenticator` canonicalisation (see `docs/DEVELOPMENT.md`).
    #[test]
    fn matches_vinyldns_server_algorithm_for_post_with_query() {
        let signer = Signer::new("testUserAccessKey", "testUserSecretKey", "us-east-1", "VinylDNS");
        let url = Url::parse("http://localhost:9000/zones/batchrecordchanges?allowManualReview=false").unwrap();
        let body = br#"{"changes":[]}"#;
        let signed = signer.sign("POST", &url, body, fixed_time());
        assert_eq!(
            signed.authorization,
            format!(
                "AWS4-HMAC-SHA256 Credential=testUserAccessKey/20150830/us-east-1/VinylDNS/aws4_request, \
                 SignedHeaders=host;x-amz-date, Signature={}",
                include_str!("../tests/fixtures/post_with_query.sig").trim()
            )
        );
    }

    #[test]
    fn debug_output_redacts_secret() {
        let signer = Signer::new("ak", "super-secret", "r", "s");
        let dbg = format!("{signer:?}");
        assert!(!dbg.contains("super-secret"));
        assert!(dbg.contains("<redacted>"));
    }
}
