# oino-extension — guest SDK

Write Oino extensions in Rust, compiled to WASM components (`wasm32-wasip2`) for the WIT
package `oino:extension@1.0.0` (`wit/oino-extension`). Component-model async: tool calls,
commands and event handlers of one extension run concurrently.

## Crate setup

```toml
[package]
name = "my-extension"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
oino-extension = { path = "../../sdk/oino-extension" }          # add features = ["provider"] for providers
serde = { version = "1", features = ["derive"] }
schemars = "1.2"
```

Next to `Cargo.toml`, an `extension.json` manifest (extension-api §7):

```json
{
  "apiVersion": "oino.extension/1",
  "id": "unipi.notify",
  "name": "Notify",
  "version": "3.0.0",
  "permissions": { "network": { "hosts": ["ntfy.sh"] }, "osNotify": true }
}
```

Every permission defaults to denied (except `ui`); a missing grant makes the import fail
with `permission denied: <permission>`.

## Example

```rust
use std::time::Duration;
use oino_extension::command::Command;
use oino_extension::prelude::*;

#[derive(Default)]
struct Notify;

#[derive(Deserialize, JsonSchema)]
struct NotifyArgs {
    /// Message body.
    message: String,
    title: Option<String>,
}

impl Notify {
    /// Send a notification to the user's configured platforms.
    #[oino_extension::tool(label = "Notify user", kind = Other)]
    async fn notify_user(&'static self, args: NotifyArgs, cx: ToolCtx) -> Result<ToolResult> {
        cx.progress(&ToolResult::text("sending…"));
        let resp = http::post("https://ntfy.sh/oino")
            .header("Title", args.title.as_deref().unwrap_or("Oino"))
            .body(args.message)
            .send()
            .await?;
        Ok(ToolResult::text(format!("Sent ({})", resp.status())))
    }
}

impl Extension for Notify {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        cx.tool(Self::notify_user_tool());
        cx.on(EventKind::AgentSettled, |_ext, _ev, cx| async move {
            if !host::frontend_focused() {
                ui::os_notify("Oino", "Agent finished", "normal", None)?;
            }
            session::append_entry("notify.settled", Some(&json!({"session": cx.session_id()})))?;
            Ok(())
        });
        cx.on(EventKind::ToolCall, |_ext, ev, _cx| async move {
            // Returning Some(json) is the event result (pi composition rules apply).
            Ok(ev.get("toolName").and_then(Value::as_str).filter(|n| *n == "rm-rf")
                .map(|_| json!({"block": true, "reason": "nope"})))
        });
        cx.command(Command::new("unipi:notify-test").description("Send a test notification")
            .run(|_ext, _args, _cx| async { ui::notify("test", "info") }));
        cx.service("notify.send", |_ext, payload, _caller, _cx| async move { Ok(payload) });
        cx.timer("renotify", Duration::from_secs(120), true);
        Ok(())
    }
}

oino_extension::export_extension!(Notify);
```

## Modules

| Module | Capability |
|---|---|
| `session` | entries, branch, tree, `append_entry`, `send_message`, `send_user_message`, sessions index |
| `control` | model/thinking, tools, commands, `compact`, `execute_tool`, command-context ops (`new_session`, `fork`, …) |
| `models` | `complete`, `stream`, `summarize_default`, `credential_for`, `classify`, `generate_image` |
| `ui` | `dialog` (`select`/`confirm`/`input` helpers), `notify`, `status`, `widget`, panels, editor, OS notify, clipboard, open URL, external editor |
| `agents` | nested agent sessions: `Agent::spawn(spec)`, `prompt`, `steer`, `wait_idle`, `events()` |
| `process` | `exec`, `Child::spawn` with streaming stdio, signals, process trees/stats |
| `tasks` | host background tasks (`start`, `list`, `logs`, `kill`) with `oino_task` events |
| `http` | `get/post/request(..).send()` with streaming bodies, provider auth, loopback `Listener` |
| `fs` | preopens (`/ext-global`, `fs::ext_workspace()`, `fs::ext_session(sid)`, `/tmp`, `/cache`, `/logs`, `/project`), `atomic_write`, JSON helpers, `Lock`, `Watcher`; plain `std::fs` works on preopen paths |
| `timers` | `set`/`cancel` (`oino_timer` events), `sleep`, `now_ms` |
| `kv` | scoped key-value (`Scope::Global/Workspace/Session`), JSON helpers |
| `secrets`, `bus`, `services` | secrets, pub/sub (`oino_bus` events), inter-extension service calls |
| `host` | platform/version/mode, workspace id, env (granted vars), logging |
| `provider` (feature) | `Provider` trait, `ProviderStream::spawn`, `before_request`/`after_response`/`stream_event` hooks |
| `sync` | `Mutex` for state held across `.await`, `SessionMap<T>` |
| `wit` | raw generated WIT types (escape hatch) |

## Build and test

```sh
cargo xtask wasm                  # every component → extensions/dist/<id>/
cargo xtask wasm -p my-extension  # one component
```

Integration tests load components with `oino_wasm_host::testing::Harness` against
`oino_extension_api::testing::RecordingHostServices` (see `crates/oino-wasm-host/tests/`);
the fixtures in `extensions/fixtures/` are worked examples (`echo`: every contribution and
import; `async-probe`: concurrency, cancellation, services, tasks; `ui-probe`: UI;
`provider-faux`: a provider).
