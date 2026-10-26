//! Replays the pi-ai 1.4.0 oracle fixtures (`scripts/pi-oracle/providers-v1/payload.mjs` and
//! `scripts/pi-oracle/providers-more/payload.mjs`): request URL, headers and body must equal
//! pi's captured request (`SigV4` signatures are recomputed from pi's signed headers), and the
//! recorded SSE / AWS event stream (or error response) must produce pi's events, final message,
//! usage and cost.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use oino_ai::json::json_semantic_eq;
use oino_provider_kit::sigv4;
use oino_provider_kit::{PrepareOptions, ProviderRequest, Wire, finalize, prepare};
use serde_json::{Value, json};

/// Headers the AWS SDK signs or adds as telemetry; `SigV4` is checked separately.
const AWS_HEADERS: [&str; 9] = [
    "authorization",
    "x-amz-date",
    "x-amz-content-sha256",
    "x-amz-security-token",
    "x-amz-user-agent",
    "amz-sdk-invocation-id",
    "amz-sdk-request",
    "content-length",
    ":authority",
];

fn fixtures(api: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(api);
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    files
}

/// Headers compared with pi: everything except SDK telemetry and the product user agent.
fn comparable_headers(headers: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for (key, value) in headers.as_object().into_iter().flatten() {
        let key = key.to_ascii_lowercase();
        if key.starts_with("x-stainless")
            || key == "x-goog-api-client"
            || key == "content-encoding"
            || (key == "user-agent"
                && !value.as_str().unwrap_or_default().starts_with("claude-cli"))
        {
            continue;
        }
        out.insert(key, value.clone());
    }
    Value::Object(out)
}

/// Drops what typed `oino-ai` values imply: `role` on assistant messages and `type` on
/// tool calls.
fn strip_implied(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        if object.get("role") == Some(&json!("assistant")) {
            object.remove("role");
        }
        if object.get("type") == Some(&json!("toolCall")) {
            object.remove("type");
        }
        for child in object.values_mut() {
            strip_implied(child);
        }
    } else if let Some(list) = value.as_array_mut() {
        list.iter_mut().for_each(strip_implied);
    }
}

fn normalize_event(mut event: Value) -> Value {
    for key in ["toolCall", "message", "error"] {
        if let Some(inner) = event.get_mut(key) {
            strip_implied(inner);
        }
    }
    if event["type"] == "toolcall_start"
        && let Some(object) = event.as_object_mut()
    {
        object.remove("id");
        object.remove("toolName");
    }
    for key in ["message", "error"] {
        if let Some(message) = event.get_mut(key).and_then(Value::as_object_mut) {
            message.insert("timestamp".into(), json!(0));
            message.remove("durationMs");
            if let Some(diagnostics) = message.get_mut("diagnostics").and_then(Value::as_array_mut)
            {
                for d in diagnostics {
                    d["timestamp"] = json!(0);
                }
            }
        }
    }
    event
}

/// pi's `SigV4` `authorization` header recomputed with our signer over pi's own signed headers.
fn check_sigv4(pi_request: &Value, signing: &sigv4::Signing) -> Result<(), String> {
    let headers = &pi_request["headers"];
    let authorization = headers["authorization"].as_str().unwrap_or_default();
    let signed = authorization
        .split("SignedHeaders=")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .ok_or_else(|| format!("pi request is not SigV4 signed: {authorization}"))?;
    let pairs: Vec<(String, String)> = signed
        .split(';')
        .map(|name| {
            (
                name.to_owned(),
                headers[name].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    let url = pi_request["url"].as_str().unwrap_or_default();
    let path_query = url
        .split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|i| &rest[i..]))
        .unwrap_or("/");
    let (path, query) = path_query.split_once('?').unwrap_or((path_query, ""));
    let amz_date = headers["x-amz-date"].as_str().unwrap_or_default();
    let (ours_signed, signature) = sigv4::signature(
        "POST",
        path,
        query,
        &pairs,
        headers["x-amz-content-sha256"].as_str().unwrap_or_default(),
        signing,
        amz_date,
    );
    let expected = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}/{}/{}/aws4_request, SignedHeaders={ours_signed}, Signature={signature}",
        signing.access_key_id,
        &amz_date[..8],
        signing.region,
        signing.service
    );
    if expected != authorization {
        return Err(format!("sigv4:\n  pi   {authorization}\n  ours {expected}"));
    }
    if headers["x-amz-security-token"].as_str() != signing.session_token.as_deref() {
        return Err("sigv4: session token differs".into());
    }
    Ok(())
}

fn response_headers(fixture: &Value, wire: Wire) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    if wire == Wire::Bedrock {
        headers.push(("x-amzn-requestid".to_owned(), "req-1".to_owned()));
    }
    let status = fixture["status"].as_u64().unwrap_or(200);
    let content_type = if status == 200 {
        if wire == Wire::Bedrock {
            "application/vnd.amazon.eventstream"
        } else {
            "text/event-stream"
        }
    } else {
        "application/json"
    };
    headers.push(("content-type".to_owned(), content_type.to_owned()));
    for (k, v) in fixture["responseHeaders"].as_object().into_iter().flatten() {
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case(k));
        headers.push((k.clone(), v.as_str().unwrap_or_default().to_owned()));
    }
    headers
}

#[allow(clippy::too_many_lines, reason = "one linear replay")]
fn run(path: &Path) -> Result<(), String> {
    let fixture: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let wire = Wire::from_api(fixture["model"]["api"].as_str().unwrap_or_default())
        .ok_or("unknown api")?;
    let request_json = json!({
        "model": fixture["model"],
        "context": fixture["context"],
        "options": fixture["options"],
        "apiKey": fixture["apiKey"],
    });
    let mut request = ProviderRequest::parse(&request_json)?;
    if wire == Wire::Vertex {
        request.extra.vertex_access_token = Some("ya29.adc-token".into());
    }
    let pi_request = &fixture["request"];
    let mut problems = Vec::new();
    let mut prepared = match prepare(wire, &request, PrepareOptions::default(), 0) {
        Ok(prepared) => prepared,
        Err(message) => {
            // pi failed before any request (setup error): a lone error event with the message.
            let events = fixture["events"].as_array().unwrap();
            let pi_message = events
                .last()
                .and_then(|e| e["error"]["errorMessage"].as_str())
                .unwrap_or_default();
            if pi_request["url"].is_string() || events.len() != 1 || pi_message != message {
                return Err(format!("prepare failed: {message}\n  pi: {pi_message}"));
            }
            return Ok(());
        }
    };
    if pi_request["url"].is_string() {
        if pi_request["url"] != json!(prepared.http.url) {
            problems.push(format!(
                "url: pi {} ours {}",
                pi_request["url"], prepared.http.url
            ));
        }
        let mut headers = prepared.http.headers.clone();
        let body = finalize(wire, prepared.http.payload.clone(), &mut headers);
        if !json_semantic_eq(&pi_request["body"], &body) {
            problems.push(format!(
                "body:\n  pi   {}\n  ours {}",
                pi_request["body"], body
            ));
        }
        let ours: serde_json::Map<String, Value> = headers
            .pairs()
            .iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect();
        let (mut pi_h, mut ours_h) = (
            comparable_headers(&pi_request["headers"]),
            comparable_headers(&Value::Object(ours)),
        );
        if wire == Wire::Bedrock {
            for headers in [&mut pi_h, &mut ours_h] {
                if let Some(map) = headers.as_object_mut() {
                    let bearer = map
                        .get("authorization")
                        .and_then(Value::as_str)
                        .is_some_and(|a| a.starts_with("Bearer "));
                    map.retain(|k, _| {
                        (bearer && k == "authorization") || !AWS_HEADERS.contains(&k.as_str())
                    });
                }
            }
            if let Some(signing) = &prepared.http.signing
                && let Err(problem) = check_sigv4(pi_request, signing)
            {
                problems.push(problem);
            }
        }
        if pi_h != ours_h {
            problems.push(format!("headers:\n  pi   {pi_h}\n  ours {ours_h}"));
        }
    } else {
        problems.push(format!(
            "pi sent no request but ours did: {}",
            prepared.http.url
        ));
    }
    let exchange = &mut prepared.exchange;
    let status = u16::try_from(fixture["status"].as_u64().unwrap()).unwrap();
    let body: Vec<u8> = match fixture["sseBase64"].as_str() {
        Some(b64) => base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap(),
        None => fixture["sse"]
            .as_str()
            .unwrap_or_default()
            .as_bytes()
            .to_vec(),
    };
    let headers = response_headers(&fixture, wire);
    let mut events = Vec::new();
    if status == 200 {
        events.extend(exchange.start_with(&headers));
        for chunk in body.chunks(7) {
            events.extend(exchange.feed(chunk));
        }
        events.extend(exchange.finish());
    } else {
        events.extend(exchange.http_error_with(status, &headers, &body));
    }
    let ours: Vec<Value> = events
        .into_iter()
        .map(|e| normalize_event(serde_json::to_value(e).unwrap()))
        .collect();
    let pi: Vec<Value> = fixture["events"]
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .map(normalize_event)
        .collect();
    if ours.len() == pi.len() {
        for (index, (a, b)) in pi.iter().zip(&ours).enumerate() {
            if !json_semantic_eq(a, b) {
                problems.push(format!("event {index}:\n  pi   {a}\n  ours {b}"));
            }
        }
    } else {
        problems.push(format!(
            "event count {} vs pi {}:\n  pi   {:?}\n  ours {:?}",
            ours.len(),
            pi.len(),
            pi.iter().map(|e| e["type"].clone()).collect::<Vec<_>>(),
            ours.iter().map(|e| e["type"].clone()).collect::<Vec<_>>()
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

fn check(api: &str) {
    let mut failures = Vec::new();
    let files = fixtures(api);
    assert!(files.len() >= 10, "{api}: expected at least 10 fixtures");
    for path in &files {
        if let Err(problem) = run(path) {
            failures.push(format!("== {}\n{problem}", path.display()));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} fixtures differ from pi:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

#[test]
fn anthropic_messages_matches_pi() {
    check("anthropic-messages");
}

#[test]
fn openai_completions_matches_pi() {
    check("openai-completions");
}

#[test]
fn openai_responses_matches_pi() {
    check("openai-responses");
}

#[test]
fn azure_openai_responses_matches_pi() {
    check("azure-openai-responses");
}

#[test]
fn openai_codex_responses_matches_pi() {
    check("openai-codex-responses");
}

#[test]
fn google_generative_ai_matches_pi() {
    check("google-generative-ai");
}

#[test]
fn google_vertex_matches_pi() {
    check("google-vertex");
}

#[test]
fn bedrock_converse_stream_matches_pi() {
    check("bedrock-converse-stream");
}
