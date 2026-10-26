//! Provider components end to end (providers-more): loads the built `extensions/dist/oino.*`
//! packages into the wasm host and replays pi oracle fixtures of the new wires (Codex, Azure,
//! Google, Vertex, Bedrock) and provider adaptations (Copilot, Cloudflare, Azure completions)
//! from a loopback HTTP server: request body and event sequence must match pi.
//!
//! Copy to `crates/oino-wasm-host/tests/providers_more.rs` (owner wit-v1) and run
//! `cargo test -p oino-wasm-host --test providers_more` after `cargo xtask wasm`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "test assertions")]

use std::sync::Arc;

use base64::Engine as _;
use futures::StreamExt;
use oino_extension_api::{CallKind, ExtensionRuntime as _, HostServices, InitInput, RawJson};
use oino_wasm_host::packages;
use oino_wasm_host::testing::{Harness, repo_root};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn serve_once(
    status: u16,
    body: Vec<u8>,
    content_type: &'static str,
    extra_headers: String,
) -> (u16, tokio::task::JoinHandle<(String, Vec<u8>)>) {
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
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while buf.len() < pos + 4 + len {
                    let n = sock.read(&mut tmp).await.unwrap();
                    buf.extend_from_slice(&tmp[..n]);
                }
                let req_body = buf[pos + 4..pos + 4 + len].to_vec();
                let resp = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\nx-amzn-requestid: req-1\r\n{extra_headers}content-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
                for chunk in body.chunks(11) {
                    sock.write_all(chunk).await.unwrap();
                }
                sock.flush().await.unwrap();
                return (head, req_body);
            }
        }
    });
    (port, handle)
}

/// How a case points the request at the loopback server.
enum Target {
    /// `model.baseUrl = http://127.0.0.1:<port><suffix>`.
    BaseUrl(&'static str),
    /// `options.env[name] = http://127.0.0.1:<port><suffix>` (Azure endpoints).
    Env(&'static str, &'static str),
}

#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines, reason = "one table-driven replay")]
async fn providers_more_replay_fixtures() {
    let h = Harness::new().unwrap();
    let root = repo_root();
    let cases = [
        ("oino.openai-codex", "openai-codex-responses", "i_tool_stream", Target::BaseUrl("/backend-api")),
        ("oino.openai-codex", "openai-codex-responses", "h_error_429_usage", Target::BaseUrl("")),
        ("oino.azure", "azure-openai-responses", "c_thinking", Target::Env("AZURE_OPENAI_BASE_URL", "/openai/v1")),
        ("oino.google", "google-generative-ai", "i_tool_stream", Target::BaseUrl("/v1beta")),
        ("oino.google", "google-generative-ai", "c_thinking_budget", Target::BaseUrl("/v1beta")),
        ("oino.google", "google-vertex", "a_custom_base_url", Target::BaseUrl("/v1")),
        ("oino.bedrock", "bedrock-converse-stream", "c_thinking_adaptive", Target::BaseUrl("")),
        ("oino.bedrock", "bedrock-converse-stream", "i_tool_stream", Target::BaseUrl("")),
        ("oino.bedrock", "bedrock-converse-stream", "h_error_400", Target::BaseUrl("")),
        ("oino.anthropic", "anthropic-messages", "p_copilot_vision", Target::BaseUrl("")),
        ("oino.openai-compat", "openai-completions", "p_cloudflare_workers", Target::BaseUrl("/v1")),
        ("oino.openai-compat", "openai-completions", "p_azure_completions", Target::Env("AZURE_OPENAI_BASE_URL", "/openai/v1")),
    ];
    for (pkg, api, name, target) in cases {
        let package = packages::load_dir(&root.join("extensions/dist").join(pkg), true).unwrap();
        let host: Arc<dyn HostServices> = Arc::clone(&h.host) as Arc<dyn HostServices>;
        let ext = h.runtime.load(package, host).await.unwrap();
        ext.init(InitInput::default(), h.cx("s1", CallKind::Init)).await.unwrap();
        let provider = ext.provider().unwrap();
        let fixture: Value = serde_json::from_str(
            &std::fs::read_to_string(
                root.join("extensions/sdk/oino-provider-kit/tests/fixtures")
                    .join(api)
                    .join(format!("{name}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        let status = u16::try_from(fixture["status"].as_u64().unwrap()).unwrap();
        let (body, content_type) = match fixture["sseBase64"].as_str() {
            Some(b64) => (
                base64::engine::general_purpose::STANDARD.decode(b64).unwrap(),
                if status == 200 { "application/vnd.amazon.eventstream" } else { "application/json" },
            ),
            None => (
                fixture["sse"].as_str().unwrap().as_bytes().to_vec(),
                if status == 200 { "text/event-stream" } else { "application/json" },
            ),
        };
        let extra: String = fixture["responseHeaders"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, v)| format!("{k}: {}\r\n", v.as_str().unwrap_or_default()))
            .collect();
        let (port, server) = serve_once(status, body, content_type, extra).await;
        let mut model = fixture["model"].clone();
        let mut options = fixture["options"].clone();
        let mut api_key = fixture["apiKey"].clone();
        match target {
            Target::BaseUrl(suffix) => model["baseUrl"] = json!(format!("http://127.0.0.1:{port}{suffix}")),
            Target::Env(var, suffix) => {
                options["env"][var] = json!(format!("http://127.0.0.1:{port}{suffix}"));
                options["env"].as_object_mut().unwrap().remove("AZURE_OPENAI_RESOURCE_NAME");
            }
        }
        // Copilot tokens carry their endpoint; use a token without `proxy-ep` for loopback.
        if model["provider"] == "github-copilot" {
            api_key = json!("ghu_loopback");
        }
        let request = json!({"requestId": "r1", "model": model, "context": fixture["context"], "options": options, "apiKey": api_key});
        let events: Vec<Value> = provider
            .stream(RawJson::from(request))
            .await
            .unwrap()
            .map(|e| serde_json::to_value(e.unwrap()).unwrap())
            .collect()
            .await;
        let (head, body) = server.await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(
            oino_ai::json::json_semantic_eq(&body, &fixture["request"]["body"]),
            "{pkg}/{name} body\n{body}\n{}",
            fixture["request"]["body"]
        );
        let ours: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
        let pi: Vec<&str> = fixture["events"].as_array().unwrap().iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert_eq!(ours, pi, "{pkg}/{name}");
        let last = events.last().unwrap();
        let pl = fixture["events"].as_array().unwrap().last().unwrap();
        assert_eq!(last["message"]["usage"], pl["message"]["usage"], "{pkg}/{name} usage");
        assert_eq!(last["error"]["errorMessage"], pl["error"]["errorMessage"], "{pkg}/{name}");
        println!("OK {pkg}/{name}: {ours:?}\n  head: {}", head.lines().next().unwrap_or_default());
    }
    let registered: Vec<String> = h
        .host
        .calls_named("register_provider")
        .iter()
        .map(|c| c.detail.to_string())
        .collect();
    for id in ["github-copilot", "cloudflare-ai-gateway", "openai-codex", "azure", "amazon-bedrock", "google", "google-vertex"] {
        assert!(
            registered.iter().any(|r| r.contains(&format!("\\\"id\\\":\\\"{id}\\\"")) || r.contains(&format!("\"id\":\"{id}\""))),
            "{id} not registered: {registered:?}"
        );
    }
}
