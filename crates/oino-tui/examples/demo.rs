//! Manual smoke test: the interactive client over a scripted [`FakeBackend`].
//!
//! `cargo run -p oino-tui --example demo` — type a prompt and press Enter to see a scripted
//! streaming reply with a tool call; `ctrl+d` exits.

#![allow(clippy::print_stderr, reason = "example reports errors on stderr")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use oino_client::SessionBackend;
use oino_client::fake::FakeBackend;
use oino_core::events::ClientEvent;
use oino_tui::{TuiHost, TuiOptions};
use serde_json::json;

struct DemoHost;

impl TuiHost for DemoHost {
    fn cwd(&self) -> PathBuf {
        std::env::current_dir().unwrap_or_default()
    }
    fn home(&self) -> Option<PathBuf> {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

fn event(value: serde_json::Value) -> Option<ClientEvent> {
    serde_json::from_value(value).ok()
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let fake = FakeBackend::new("demo");
    fake.reply(
        "get_state",
        Ok(Some(json!({"model": {"id": "demo-1", "name": "Demo", "api": "faux", "provider": "demo", "reasoning": true, "contextWindow": 128_000},
            "thinkingLevel": "medium", "isStreaming": false, "isCompacting": false, "steeringMode": "one-at-a-time",
            "followUpMode": "one-at-a-time", "sessionId": "demo", "autoCompactionEnabled": true, "messageCount": 0, "pendingMessageCount": 0}))),
    );
    fake.reply(
        "get_entries",
        Ok(Some(json!({"entries": [], "leafId": null}))),
    );
    let script = fake.clone();
    tokio::spawn(async move {
        let mut seen = 0;
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let prompts: Vec<String> = script
                .commands()
                .iter()
                .filter_map(|c| match serde_json::to_value(c) {
                    Ok(v) if v["type"] == "prompt" => v["message"].as_str().map(str::to_owned),
                    _ => None,
                })
                .collect();
            for prompt in prompts.iter().skip(seen) {
                let user = json!({"role": "user", "content": prompt, "timestamp": 0});
                let usage = json!({"input": 10, "output": 5, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 15,
                    "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}});
                let reply = |text: &str, stop: &str| {
                    json!({"role": "assistant", "content": [
                    {"type": "text", "text": text},
                    {"type": "toolCall", "id": "t1", "name": "ls", "arguments": {"path": "."}}],
                    "api": "faux", "provider": "demo", "model": "demo-1", "usage": usage, "stopReason": stop, "timestamp": 0})
                };
                let steps = [
                    json!({"type": "agent_start"}),
                    json!({"type": "message_start", "message": user}),
                    json!({"type": "message_start", "message": reply("", "pending")}),
                    json!({"type": "message_update", "message": reply("Listing the **directory**…", "pending"),
                        "assistantMessageEvent": {"type": "start", "partial": reply("", "pending")}}),
                    json!({"type": "message_end", "message": reply("Listing the **directory**…", "toolUse")}),
                    json!({"type": "tool_execution_start", "toolCallId": "t1", "toolName": "ls", "args": {"path": "."}}),
                    json!({"type": "tool_execution_end", "toolCallId": "t1", "toolName": "ls",
                        "result": {"content": [{"type": "text", "text": "Cargo.toml\nsrc/"}]}, "isError": false}),
                    json!({"type": "agent_end", "messages": [], "willRetry": false}),
                    json!({"type": "agent_settled", "aborted": false}),
                ];
                for step in steps {
                    if let Some(e) = event(step) {
                        script.emit(e);
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
            seen = prompts.len();
        }
    });
    let backend: Arc<dyn SessionBackend> = Arc::new(fake);
    let options = TuiOptions {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        ..TuiOptions::default()
    };
    if let Err(error) = oino_tui::run(backend, Arc::new(DemoHost), options).await {
        eprintln!("{error}");
    }
}
