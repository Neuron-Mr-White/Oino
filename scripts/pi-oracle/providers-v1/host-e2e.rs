//! Provider components end to end (providers-v1): loads the built `extensions/dist/oino.*`
//! packages into the wasm host and replays pi oracle fixtures from a loopback HTTP server
//! (request body and event sequence must match pi). Copy to `crates/oino-wasm-host/tests/`
//! (owner wit-v1) and run `cargo test -p oino-wasm-host --test providers` after `cargo xtask wasm`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "test assertions")]

use std::sync::Arc;

use futures::StreamExt;
use oino_extension_api::{CallKind, ExtensionRuntime as _, HostServices, InitInput, RawJson};
use oino_wasm_host::packages;
use oino_wasm_host::testing::{Harness, repo_root};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn serve_once(status: u16, body: String) -> (u16, tokio::task::JoinHandle<(String, Vec<u8>)>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            let n = sock.read(&mut tmp).await.unwrap();
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                let len = head
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                    .unwrap_or(0);
                while buf.len() < pos + 4 + len {
                    let n = sock.read(&mut tmp).await.unwrap();
                    buf.extend_from_slice(&tmp[..n]);
                }
                let req_body = buf[pos + 4..pos + 4 + len].to_vec();
                let ctype = if status == 200 { "text/event-stream" } else { "application/json" };
                let resp = format!("HTTP/1.1 {status} X\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                sock.write_all(resp.as_bytes()).await.unwrap();
                // dribble the body
                for chunk in body.as_bytes().chunks(11) {
                    sock.write_all(chunk).await.unwrap();
                }
                sock.flush().await.unwrap();
                return (head, req_body);
            }
        }
    });
    (port, handle)
}

#[tokio::test(flavor = "multi_thread")]
async fn providers_replay_fixtures() {
    let h = Harness::new().unwrap();
    let root = repo_root();
    let cases = [
        ("oino.anthropic", "anthropic-messages", "i_tool_stream", ""),
        ("oino.anthropic", "anthropic-messages", "c_thinking_adaptive", ""),
        ("oino.anthropic", "anthropic-messages", "h_error_429", ""),
        ("oino.openai-responses", "openai-responses", "c_thinking_replay", "/v1"),
        ("oino.openai-responses", "openai-responses", "i_tool_stream", "/v1"),
        ("oino.openai-compat", "openai-completions", "d_openrouter_cache_long", "/api/v1"),
        ("oino.openai-compat", "openai-completions", "i_tool_stream", "/v1"),
        ("oino.openai-compat", "openai-completions", "k_content_filter", "/v1"),
    ];
    for (pkg, api, name, suffix) in cases {
        let package = packages::load_dir(&root.join("extensions/dist").join(pkg), true).unwrap();
        let host: Arc<dyn HostServices> = Arc::clone(&h.host) as Arc<dyn HostServices>;
        let ext = h.runtime.load(package, host).await.unwrap();
        ext.init(InitInput::default(), h.cx("s1", CallKind::Init)).await.unwrap();
        let provider = ext.provider().unwrap();
        let fixture: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("extensions/sdk/oino-provider-kit/tests/fixtures").join(api).join(format!("{name}.json"))).unwrap(),
        )
        .unwrap();
        let status = u16::try_from(fixture["status"].as_u64().unwrap()).unwrap();
        let (port, server) = serve_once(status, fixture["sse"].as_str().unwrap().to_owned()).await;
        let mut model = fixture["model"].clone();
        model["baseUrl"] = json!(format!("http://127.0.0.1:{port}{suffix}"));
        let request = json!({"requestId": "r1", "model": model, "context": fixture["context"], "options": fixture["options"], "apiKey": fixture["apiKey"]});
        let events: Vec<Value> = provider
            .stream(RawJson::from(request))
            .await
            .unwrap()
            .map(|e| serde_json::to_value(e.unwrap()).unwrap())
            .collect()
            .await;
        let (head, body) = server.await.unwrap();
        let mut body: Value = serde_json::from_slice(&body).unwrap();
        // ChatGPT sign-in detection depends on the real api.openai.com base URL.
        if fixture["request"]["body"].get("max_output_tokens").is_none() {
            body.as_object_mut().unwrap().remove("max_output_tokens");
        }
        assert!(oino_ai::json::json_semantic_eq(&body, &fixture["request"]["body"]), "{pkg}/{name} body\n{body}\n{}", fixture["request"]["body"]);
        let ours: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
        let pi: Vec<&str> = fixture["events"].as_array().unwrap().iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert_eq!(ours, pi, "{pkg}/{name}");
        let last = events.last().unwrap();
        let pl = fixture["events"].as_array().unwrap().last().unwrap();
        assert_eq!(last["message"]["usage"], pl["message"]["usage"], "{pkg}/{name} usage");
        assert_eq!(last["error"]["errorMessage"], pl["error"]["errorMessage"], "{pkg}/{name}");
        println!("OK {pkg}/{name}: {ours:?}\n  head: {}", head.lines().next().unwrap_or_default());
    }
    assert!(h.host.calls_named("register_provider").len() > 30);
}
