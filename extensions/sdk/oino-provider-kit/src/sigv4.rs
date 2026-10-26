//! AWS Signature Version 4 (header signing) as the Smithy `SignatureV4` signer applies it to
//! Bedrock Runtime requests: double-escaped canonical path, sorted lower-case headers with
//! collapsed whitespace, `x-amz-content-sha256`, optional session token.

use std::fmt::Write as _;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::request::Headers;

type HmacSha256 = Hmac<Sha256>;

const ALGORITHM: &str = "AWS4-HMAC-SHA256";
/// Headers the SDK never signs.
const UNSIGNABLE: [&str; 8] = [
    "authorization",
    "cache-control",
    "connection",
    "expect",
    "from",
    "keep-alive",
    "max-forwards",
    "user-agent",
];

/// IAM credentials and scope for one signed request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signing {
    /// `AWS_ACCESS_KEY_ID`.
    pub access_key_id: String,
    /// `AWS_SECRET_ACCESS_KEY`.
    pub secret_access_key: String,
    /// `AWS_SESSION_TOKEN`.
    pub session_token: Option<String>,
    /// Signing region.
    pub region: String,
    /// Signing service (`bedrock`).
    pub service: String,
}

/// Lower-case hex.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    // HMAC accepts keys of any length; `new_from_slice` cannot fail for it.
    HmacSha256::new_from_slice(key).map_or_else(
        |_| Vec::new(),
        |mut mac| {
            mac.update(data);
            mac.finalize().into_bytes().to_vec()
        },
    )
}

/// RFC 3986 escaping as the SDK's `escapeUri` (unreserved characters kept).
#[must_use]
pub fn escape_uri(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Canonical path: dot segments removed, every segment escaped again (non-S3 services).
fn canonical_path(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    let leading = if path.starts_with('/') { "/" } else { "" };
    let trailing = if path.ends_with('/') && !segments.is_empty() {
        "/"
    } else {
        ""
    };
    let joined: Vec<String> = segments.iter().map(|s| escape_uri(s)).collect();
    let body = format!("{leading}{}{trailing}", joined.join("/"));
    if body.is_empty() {
        "/".to_owned()
    } else {
        body
    }
}

fn canonical_query(query: &str) -> String {
    let mut pairs: Vec<(String, String)> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (escape_uri(&decode(k)), escape_uri(&decode(v)))
        })
        .collect();
    pairs.sort();
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(value) = text
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(value);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn collapse(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Signature over a request whose signing headers are already present (`x-amz-date`,
/// `x-amz-content-sha256`, optional `x-amz-security-token`). Returns
/// `(signed_headers, signature)`.
#[must_use]
pub fn signature(
    method: &str,
    path: &str,
    query: &str,
    headers: &[(String, String)],
    payload_hash: &str,
    signing: &Signing,
    amz_date: &str,
) -> (String, String) {
    let mut canonical: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), collapse(v)))
        .filter(|(k, _)| !UNSIGNABLE.contains(&k.as_str()) && !k.starts_with("x-amzn-trace-id"))
        .collect();
    canonical.sort();
    let signed_headers = canonical
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let mut header_block = String::new();
    for (k, v) in &canonical {
        let _ = writeln!(header_block, "{k}:{v}");
    }
    let request = format!(
        "{method}\n{}\n{}\n{header_block}\n{signed_headers}\n{payload_hash}",
        canonical_path(path),
        canonical_query(query)
    );
    let date = amz_date.get(..8).unwrap_or(amz_date);
    let scope = format!("{date}/{}/{}/aws4_request", signing.region, signing.service);
    let string_to_sign = format!(
        "{ALGORITHM}\n{amz_date}\n{scope}\n{}",
        sha256_hex(request.as_bytes())
    );
    let mut key = hmac(
        format!("AWS4{}", signing.secret_access_key).as_bytes(),
        date.as_bytes(),
    );
    for part in [
        signing.region.as_str(),
        signing.service.as_str(),
        "aws4_request",
    ] {
        key = hmac(&key, part.as_bytes());
    }
    (signed_headers, hex(&hmac(&key, string_to_sign.as_bytes())))
}

/// Signs `headers` for `POST url` with `body` at `amz_date` (`YYYYMMDDTHHMMSSZ`): adds `host`,
/// `x-amz-date`, `x-amz-security-token`, `x-amz-content-sha256` and `authorization`.
pub fn sign(url: &str, headers: &mut Headers, body: &[u8], signing: &Signing, amz_date: &str) {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let (host, path_query) = rest
        .find('/')
        .map_or((rest, "/"), |i| (&rest[..i], &rest[i..]));
    let (path, query) = path_query.split_once('?').unwrap_or((path_query, ""));
    headers.remove("authorization");
    headers.set("host", host);
    headers.set("x-amz-date", amz_date);
    if let Some(token) = &signing.session_token {
        headers.set("x-amz-security-token", token);
    }
    let payload_hash = sha256_hex(body);
    headers.set("x-amz-content-sha256", &payload_hash);
    let (signed, signature) = signature(
        "POST",
        path,
        query,
        headers.pairs(),
        &payload_hash,
        signing,
        amz_date,
    );
    let date = amz_date.get(..8).unwrap_or(amz_date);
    headers.set(
        "authorization",
        format!(
            "{ALGORITHM} Credential={}/{date}/{}/{}/aws4_request, SignedHeaders={signed}, Signature={signature}",
            signing.access_key_id, signing.region, signing.service
        ),
    );
}

/// `YYYYMMDDTHHMMSSZ` for a Unix time in milliseconds.
#[must_use]
pub fn amz_date(now_ms: i64) -> String {
    let secs = now_ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Signing {
        Signing {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
            region: "us-east-1".into(),
            service: "service".into(),
        }
    }

    /// AWS `SigV4` test suite `get-vanilla-query-order-key-case`.
    #[test]
    fn aws_test_suite_vector() {
        let headers = vec![
            ("Host".to_owned(), "example.amazonaws.com".to_owned()),
            ("X-Amz-Date".to_owned(), "20150830T123600Z".to_owned()),
        ];
        let (signed, sig) = signature(
            "GET",
            "/",
            "Param2=value2&Param1=value1",
            &headers,
            &sha256_hex(b""),
            &example(),
            "20150830T123600Z",
        );
        assert_eq!(signed, "host;x-amz-date");
        assert_eq!(
            sig,
            "b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500"
        );
    }

    #[test]
    fn dates() {
        assert_eq!(amz_date(1_793_016_000_000), "20261026T120000Z");
        assert_eq!(amz_date(0), "19700101T000000Z");
    }

    #[test]
    fn canonical_path_double_escapes() {
        assert_eq!(
            canonical_path("/model/amazon.nova-2-lite-v1%3A0/converse-stream"),
            "/model/amazon.nova-2-lite-v1%253A0/converse-stream"
        );
    }
}
