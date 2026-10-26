//! Pure OAuth helpers shared by the provider components' login flows (pi-ai `auth/oauth/pkce.js`,
//! `device-code.js` and the `URLSearchParams` handling of the individual flows). The flows
//! themselves (HTTP, UI, loopback callbacks) live in [`crate::guest::oauth`] and the components.

use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// A PKCE verifier/challenge pair (S256).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pkce {
    /// `code_verifier`.
    pub verifier: String,
    /// `code_challenge`.
    pub challenge: String,
}

/// pi `generatePKCE` from 32 random bytes.
#[must_use]
pub fn pkce(random: &[u8]) -> Pkce {
    let verifier = URL_SAFE_NO_PAD.encode(random);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Pkce {
        verifier,
        challenge,
    }
}

/// Lower-case hex (pi `randomBytes(16).toString("hex")`).
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// `application/x-www-form-urlencoded` component encoding (`URLSearchParams`).
#[must_use]
pub fn form_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'*') {
            out.push(char::from(byte));
        } else if byte == b' ' {
            out.push('+');
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// `URLSearchParams(pairs).toString()`.
#[must_use]
pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Decodes a form/query component (`+` is a space).
#[must_use]
pub fn form_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                if let Some(value) = text
                    .get(i + 1..i + 3)
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                {
                    out.push(value);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            other => out.push(other),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `URLSearchParams(query).get(name)`.
#[must_use]
pub fn query_param(query: &str, name: &str) -> Option<String> {
    let query = query.strip_prefix('?').unwrap_or(query);
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (form_decode(key) == name).then(|| form_decode(value))
    })
}

/// The query string of a URL (`None` when `text` is not an absolute URL).
#[must_use]
pub fn url_query(text: &str) -> Option<&str> {
    let (scheme, rest) = text.split_once("://")?;
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        return None;
    }
    let rest = rest.split('#').next().unwrap_or(rest);
    Some(rest.split_once('?').map_or("", |(_, q)| q))
}

/// pi `parseAuthorizationInput` (Codex / Anthropic): a redirect URL, `code#state`, a query
/// string, or a bare code → `(code, state)`.
#[must_use]
pub fn parse_authorization_input(input: &str) -> (Option<String>, Option<String>) {
    let value = input.trim();
    if value.is_empty() {
        return (None, None);
    }
    if let Some(query) = url_query(value) {
        return (query_param(query, "code"), query_param(query, "state"));
    }
    if let Some((code, state)) = value.split_once('#') {
        return (Some(code.to_owned()), Some(state.to_owned()));
    }
    if value.contains("code=") {
        return (query_param(value, "code"), query_param(value, "state"));
    }
    (Some(value.to_owned()), None)
}

/// One device-code poll result (pi `pollOAuthDeviceCodeFlow` `poll()` outcomes).
#[derive(Clone, Debug, PartialEq)]
pub enum PollStatus<T> {
    /// Authorized.
    Complete(T),
    /// Keep polling.
    Pending,
    /// Poll slower (`interval` seconds from the server, if any).
    SlowDown(Option<f64>),
    /// Give up with a message.
    Failed(String),
}

/// pi's device-code polling schedule (RFC 8628).
#[derive(Clone, Debug, PartialEq)]
pub struct DevicePoll {
    /// Current interval.
    pub interval_ms: u64,
    /// Deadline (epoch ms), if any.
    pub deadline_ms: Option<i64>,
    /// `slow_down` responses seen.
    pub slow_downs: u32,
}

const MINIMUM_INTERVAL_MS: u64 = 1000;
const DEFAULT_POLL_INTERVAL_SECONDS: f64 = 5.0;
const SLOW_DOWN_INCREMENT_MS: u64 = 5000;
/// pi's timeout message.
pub const DEVICE_TIMEOUT: &str = "Device flow timed out";
/// pi's timeout message after `slow_down` responses.
pub const DEVICE_SLOW_DOWN_TIMEOUT: &str = "Device flow timed out after one or more slow_down responses. This is often caused by clock drift in WSL or VM environments. Please sync or restart the VM clock and try again.";

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "positive, finite second counts converted to milliseconds"
)]
fn seconds_to_ms(seconds: f64) -> u64 {
    (seconds * 1000.0).floor().max(0.0) as u64
}

impl DevicePoll {
    /// Schedule for an `interval` / `expires_in` (seconds) answer at `now`.
    #[must_use]
    pub fn new(interval_seconds: Option<f64>, expires_in_seconds: Option<f64>, now: i64) -> Self {
        Self {
            interval_ms: MINIMUM_INTERVAL_MS.max(seconds_to_ms(
                interval_seconds.unwrap_or(DEFAULT_POLL_INTERVAL_SECONDS),
            )),
            deadline_ms: expires_in_seconds
                .map(|s| now.saturating_add(i64::try_from(seconds_to_ms(s)).unwrap_or(i64::MAX))),
            slow_downs: 0,
        }
    }

    /// Milliseconds left before the deadline (`None` without one).
    #[must_use]
    pub fn remaining(&self, now: i64) -> Option<i64> {
        self.deadline_ms.map(|d| d - now)
    }

    /// Whether polling may continue at `now`.
    #[must_use]
    pub fn active(&self, now: i64) -> bool {
        self.remaining(now).is_none_or(|r| r > 0)
    }

    /// Registers a `slow_down`.
    pub fn slow_down(&mut self, interval_seconds: Option<f64>) {
        self.slow_downs += 1;
        self.interval_ms = match interval_seconds.filter(|s| s.is_finite() && *s > 0.0) {
            Some(seconds) => MINIMUM_INTERVAL_MS.max(seconds_to_ms(seconds)),
            None => MINIMUM_INTERVAL_MS.max(self.interval_ms + SLOW_DOWN_INCREMENT_MS),
        };
    }

    /// Sleep before the next poll at `now`.
    #[must_use]
    pub fn next_sleep(&self, now: i64) -> u64 {
        match self.remaining(now) {
            Some(remaining) => self
                .interval_ms
                .min(u64::try_from(remaining.max(0)).unwrap_or(0)),
            None => self.interval_ms,
        }
    }

    /// The timeout error.
    #[must_use]
    pub fn timeout_message(&self) -> &'static str {
        if self.slow_downs > 0 {
            DEVICE_SLOW_DOWN_TIMEOUT
        } else {
            DEVICE_TIMEOUT
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7636_pkce_vector() {
        let bytes = [
            116, 24, 223, 180, 151, 153, 224, 37, 79, 250, 96, 125, 216, 173, 187, 186, 22, 212,
            37, 77, 105, 214, 191, 240, 91, 88, 5, 88, 83, 132, 141, 121,
        ];
        let p = pkce(&bytes);
        assert_eq!(p.verifier, "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(p.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn authorization_input() {
        assert_eq!(
            parse_authorization_input("http://localhost:1455/auth/callback?code=a%2Bb&state=s"),
            (Some("a+b".into()), Some("s".into()))
        );
        assert_eq!(
            parse_authorization_input("abc#xyz"),
            (Some("abc".into()), Some("xyz".into()))
        );
        assert_eq!(
            parse_authorization_input("code=1&state=2"),
            (Some("1".into()), Some("2".into()))
        );
        assert_eq!(parse_authorization_input("abc"), (Some("abc".into()), None));
    }

    #[test]
    fn device_schedule() {
        let mut poll = DevicePoll::new(None, Some(10.0), 0);
        assert_eq!(poll.interval_ms, 5000);
        poll.slow_down(None);
        assert_eq!(poll.interval_ms, 10_000);
        poll.slow_down(Some(2.0));
        assert_eq!(poll.interval_ms, 2000);
        assert_eq!(poll.next_sleep(9_500), 500);
        assert!(!poll.active(10_000));
        assert_eq!(poll.timeout_message(), DEVICE_SLOW_DOWN_TIMEOUT);
    }

    #[test]
    fn forms() {
        assert_eq!(
            form(&[("scope", "read:user"), ("a", "b c")]),
            "scope=read%3Auser&a=b+c"
        );
    }
}
