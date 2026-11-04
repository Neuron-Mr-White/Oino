//! A response body the host cut (idle timeout, connection failure) ends the exchange with a
//! transient connection error, never by parsing the cut-off tail (owner lerp graph
//! 2026-11-03: `malformed server-sent event JSON` / `Could not parse Anthropic SSE event`).
//! Replays the 200-status pi fixtures through [`BodyTail`] as the guest driver does.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use std::fs;
use std::path::Path;

use base64::Engine as _;
use oino_provider_kit::body::{BodyCut, BodyTail, CUT_MAGIC};
use oino_provider_kit::{PrepareOptions, ProviderRequest, Wire, prepare};
use serde_json::{Value, json};

struct Fixture {
    name: String,
    wire: Wire,
    request: ProviderRequest,
    body: Vec<u8>,
}

fn fixtures() -> Vec<Fixture> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut out = Vec::new();
    let mut dirs: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    dirs.sort();
    for dir in dirs {
        let mut files: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        for path in files
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
        {
            let fixture: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            if fixture["status"].as_u64() != Some(200) {
                continue;
            }
            let Some(wire) = Wire::from_api(fixture["model"]["api"].as_str().unwrap_or_default())
            else {
                continue;
            };
            let body = match fixture["sseBase64"].as_str() {
                Some(b64) => base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .unwrap(),
                None => fixture["sse"]
                    .as_str()
                    .unwrap_or_default()
                    .as_bytes()
                    .to_vec(),
            };
            if body.is_empty() {
                continue;
            }
            let mut request = ProviderRequest::parse(&json!({
                "model": fixture["model"],
                "context": fixture["context"],
                "options": fixture["options"],
                "apiKey": fixture["apiKey"],
            }))
            .unwrap();
            if wire == Wire::Vertex {
                request.extra.vertex_access_token = Some("ya29.adc-token".into());
            }
            out.push(Fixture {
                name: path.display().to_string(),
                wire,
                request,
                body,
            });
        }
    }
    out
}

fn marker(cut: BodyCut) -> Vec<u8> {
    let (kind, ms) = match cut {
        BodyCut::Idle(ms) => (b'i', ms),
        BodyCut::Reset => (b'r', 0),
    };
    let mut m = CUT_MAGIC.to_vec();
    m.push(kind);
    m.extend_from_slice(&ms.to_le_bytes());
    m
}

/// Replays `body` in 7-byte chunks like the guest driver: tail hold-back, then `body_cut` or
/// `finish`. Returns the events as JSON.
fn replay(f: &Fixture, body: &[u8]) -> Vec<Value> {
    let Ok(mut prepared) = prepare(f.wire, &f.request, PrepareOptions::default(), 0) else {
        return Vec::new();
    };
    let exchange = &mut prepared.exchange;
    let mut events = exchange.start();
    let mut tail = BodyTail::new();
    for chunk in body.chunks(7) {
        let data = tail.push(chunk);
        events.extend(exchange.feed(&data));
    }
    let (rest, cut) = tail.end();
    events.extend(exchange.feed(&rest));
    events.extend(match cut {
        Some(cut) => exchange.body_cut(cut),
        None => exchange.finish(),
    });
    events
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect()
}

fn replay_plain(f: &Fixture, body: &[u8]) -> Vec<Value> {
    let Ok(mut prepared) = prepare(f.wire, &f.request, PrepareOptions::default(), 0) else {
        return Vec::new();
    };
    let exchange = &mut prepared.exchange;
    let mut events = exchange.start();
    for chunk in body.chunks(7) {
        events.extend(exchange.feed(chunk));
    }
    events.extend(exchange.finish());
    events
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect()
}

fn error_message(events: &[Value]) -> Option<String> {
    let last = events.last()?;
    (last["type"] == "error").then(|| {
        last["error"]["errorMessage"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    })
}

/// A cut inside the first event after the middle of the body (mid-JSON for SSE wires).
fn cut_point(f: &Fixture) -> usize {
    let half = f.body.len() / 2;
    if f.wire == Wire::Bedrock {
        return half;
    }
    let text = &f.body[half..];
    text.windows(7)
        .position(|w| w == b"data: {")
        .map_or(half, |i| half + i + 12)
        .min(f.body.len() - 1)
}

#[test]
fn an_unmarked_body_streams_exactly_as_before() {
    let all = fixtures();
    assert!(all.len() > 20, "fixtures: {}", all.len());
    for f in &all {
        assert_eq!(replay(f, &f.body), replay_plain(f, &f.body), "{}", f.name);
    }
}

#[test]
fn a_stalled_stream_fails_with_the_idle_error() {
    for f in fixtures() {
        let mut body = f.body[..cut_point(&f)].to_vec();
        body.extend(marker(BodyCut::Idle(300_000)));
        let events = replay(&f, &body);
        if events.is_empty() {
            continue;
        }
        assert_eq!(
            error_message(&events).as_deref(),
            Some("Connection error: stream idle for 300 s."),
            "{}",
            f.name
        );
    }
}

#[test]
fn a_cut_body_is_a_connection_error_not_a_parse_error() {
    let mut parse_errors_before = 0;
    for f in fixtures() {
        let cut = &f.body[..cut_point(&f)];
        let before = error_message(&replay_plain(&f, cut)).unwrap_or_default();
        if before.contains("JSON") || before.contains("parse") || before.contains("EOF") {
            parse_errors_before += 1;
        }
        let mut body = cut.to_vec();
        body.extend(marker(BodyCut::Reset));
        let events = replay(&f, &body);
        if events.is_empty() {
            continue;
        }
        let message = error_message(&events).unwrap_or_default();
        assert_eq!(
            message, "Connection error: connection reset while streaming.",
            "{}",
            f.name
        );
    }
    // The old path parsed the cut-off tail (the owner's S1/S2 errors).
    assert!(parse_errors_before > 0);
}
