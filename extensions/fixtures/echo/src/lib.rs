//! Echo fixture: registers one of each contribution, records every event via `append-entry`,
//! and exposes `probe <name> <json>` to exercise each host import (host tests run it with
//! different manifests to check grants and denials).

#![allow(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    reason = "SDK handler signatures are async even when a body does not await"
)]

use std::time::Duration;

use oino_extension::command::{Command, Flag, Shortcut, item};
use oino_extension::prelude::*;
use oino_extension::{kv, wit};

/// Fixture state.
#[derive(Default)]
struct Echo;

#[derive(Deserialize, JsonSchema)]
struct EchoArgs {
    /// Text to echo.
    text: String,
}

const PI_EVENTS: &[EventKind] = &[
    EventKind::ProjectTrust,
    EventKind::ResourcesDiscover,
    EventKind::McpServersChange,
    EventKind::SessionStart,
    EventKind::SessionInfoChanged,
    EventKind::SessionBeforeSwitch,
    EventKind::SessionBeforeFork,
    EventKind::SessionBeforeCompact,
    EventKind::SessionCompact,
    EventKind::SessionCompactFailed,
    EventKind::SessionShutdown,
    EventKind::SessionBeforeTree,
    EventKind::SessionTree,
    EventKind::Context,
    EventKind::ContextWithSystem,
    EventKind::CacheWarmingDecision,
    EventKind::BeforeProviderHeaders,
    EventKind::BeforeProviderRequest,
    EventKind::AfterProviderResponse,
    EventKind::ProviderStreamEvent,
    EventKind::BeforeAgentStart,
    EventKind::AgentStart,
    EventKind::AgentEnd,
    EventKind::AgentBeforeSettle,
    EventKind::AgentSettled,
    EventKind::UiPromptStart,
    EventKind::UiPromptEnd,
    EventKind::TurnStart,
    EventKind::TurnEnd,
    EventKind::MessageStart,
    EventKind::MessageUpdate,
    EventKind::MessageEnd,
    EventKind::ToolExecutionStart,
    EventKind::ToolExecutionUpdate,
    EventKind::ToolExecutionEnd,
    EventKind::ModelSelect,
    EventKind::ThinkingLevelSelect,
    EventKind::ToolCall,
    EventKind::ToolResult,
    EventKind::UserBash,
    EventKind::Input,
    EventKind::OinoTimer,
    EventKind::OinoBus,
    EventKind::OinoTask,
];

impl Echo {
    /// Echoes `text` back.
    #[oino_extension::tool(label = "Echo", kind = Other)]
    async fn echo(&'static self, args: EchoArgs, cx: ToolCtx) -> Result<ToolResult> {
        cx.progress(&ToolResult::text("echoing"));
        Ok(ToolResult::text(args.text).with_details(json!({"session": cx.session_id()})))
    }
}

#[allow(clippy::too_many_lines, reason = "one arm per probed import")]
async fn probe(name: &str, arg: Value) -> Result<Value> {
    let s = |v: &str| {
        arg.get(v)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Ok(match name {
        "host-info" => {
            let info = host::info();
            json!({"api": info.api_version, "platform": info.platform, "ext": info.extension_version, "ws": host::workspace().id})
        }
        "env" => json!(host::env(&s("name"))?),
        "log" => {
            host::info_log("probe log line");
            Value::Null
        }
        "kv" => {
            kv::set(kv::Scope::Global, "k", b"v1")?;
            kv::set_json(kv::Scope::Session, "s", &json!({"n": 1}))?;
            json!({"global": String::from_utf8(kv::get(kv::Scope::Global, "k").unwrap_or_default()).unwrap_or_default(),
                   "session": kv::get_json::<Value>(kv::Scope::Session, "s")?, "keys": kv::keys(kv::Scope::Global, "")})
        }
        "fs" => {
            fs::atomic_write("/ext-global/probe.txt", b"hello")?;
            let text = std::fs::read_to_string("/ext-global/probe.txt")
                .map_err(|e| Error::msg(e.to_string()))?;
            let names: Vec<String> = fs::preopens().into_iter().map(|p| p.name).collect();
            json!({"text": text, "preopens": names})
        }
        "agent-config" => {
            // The pi config file reaches the guest through the /agent preopen; the secrets next
            // to it must not.
            let mcp =
                std::fs::read_to_string("/agent/mcp.json").unwrap_or_else(|e| format!("ERR {e:?}"));
            let unreadable = ["/agent/auth.json", "/agent/secrets.json"]
                .iter()
                .filter(|path| std::fs::read_to_string(path).is_err())
                .count();
            let names: Vec<String> = fs::preopens()
                .into_iter()
                .map(|p| p.name)
                .filter(|name| name == "agent" || name.starts_with("agent/"))
                .collect();
            json!({"mcp": mcp, "unreadable": unreadable, "preopens": names})
        }
        "fs-write-project" => {
            fs::atomic_write("/project/forbidden.txt", b"x")?;
            Value::Null
        }
        "fs-lock" => {
            let _lock = fs::Lock::acquire("/ext-global/lockfile", true).await?;
            json!("locked")
        }
        "exec" => {
            let out = process::exec("sh", &["-c", &s("script")], None, Some(5000)).await?;
            json!({"stdout": out.stdout, "stderr": out.stderr, "code": out.code, "killed": out.killed})
        }
        "spawn" => {
            let child = process::Child::spawn(&process::spec("cat", &[]))?;
            child.write_stdin(s("input").as_bytes()).await?;
            child.close_stdin();
            let out = child.read_stdout().await;
            let status = child.wait().await;
            json!({"stdout": out, "code": status.code, "pid": child.pid() > 0})
        }
        "http" => {
            let resp = http::get(&s("url")).send().await?;
            let status = resp.status();
            json!({"status": status, "body": resp.text().await})
        }
        "listener" => {
            let l = http::Listener::bind(0)?;
            json!({"port": l.port() > 0})
        }
        "timer" => {
            timers::set(
                &s("id"),
                Duration::from_millis(arg.get("ms").and_then(Value::as_u64).unwrap_or(10)),
                false,
            );
            Value::Null
        }
        "sleep" => {
            let start = timers::now_ms();
            timers::sleep(Duration::from_millis(20)).await;
            json!(timers::now_ms() - start >= 15)
        }
        "secrets" => {
            secrets::set("token", "abc")?;
            json!(secrets::get("token")?)
        }
        "bus-publish" => {
            bus::publish(&s("topic"), &arg["data"], true)?;
            json!(bus::get(&s("topic")))
        }
        "bus-subscribe" => {
            bus::subscribe(&s("pattern"));
            Value::Null
        }
        "service" => {
            services::call(
                arg.get("target").and_then(Value::as_str),
                &s("name"),
                &arg["payload"],
                None,
            )
            .await?
        }
        "session" => json!({"id": session::session_id(), "entries": session::entries()?.len()}),
        "append" => json!(session::append_entry(&s("type"), Some(&arg["data"]))?),
        "send-message" => {
            let msg = session::CustomMessage {
                custom_type: "echo.msg".into(),
                content: json!(s("text")),
                display: true,
                details: None,
            };
            session::send_message(&msg, session::SendOptions::default()).await?;
            Value::Null
        }
        "set-name" => {
            session::set_session_name(Some(&s("name")))?;
            Value::Null
        }
        "list-sessions" => json!(
            session::list_sessions(session::SessionsScope::Cwd)
                .await?
                .len()
        ),
        "control" => json!({"idle": control::is_idle(), "thinking": control::thinking_level()}),
        "set-model" => json!(control::set_model("p", "m")?),
        "new-session" => json!(control::new_session(None).await?),
        "complete" => models::complete(&json!({"context": {"messages": []}})).await?,
        "credential" => json!(
            models::credential_for(&s("provider"))
                .await?
                .map(|c| c.provider)
        ),
        "agent" => {
            let agent = agents::Agent::spawn(&json!({"role": "test"})).await?;
            json!(agent.id())
        }
        "task" => json!(tasks::start(
            &json!({"name": "t", "command": s("command"), "notifyOnCompletion": false})
        )?),
        "tasks" => json!(tasks::list()?),
        "ui-notify" => {
            ui::notify("hi", "info")?;
            Value::Null
        }
        "os-notify" => {
            ui::os_notify("t", "b", "normal", None)?;
            Value::Null
        }
        "register-tool" => {
            Tool::<Echo>::new(s("name"))
                .description("dynamic")
                .run(|_e, _a: Value, _cx| async { Ok(ToolResult::text("dyn")) })
                .register(ECHO.with(|e| *e))?;
            Value::Null
        }
        other => return Err(Error::msg(format!("unknown probe {other}"))),
    })
}

thread_local! {
    static ECHO: &'static Echo = Box::leak(Box::new(Echo));
}

impl Extension for Echo {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        for kind in PI_EVENTS {
            cx.on(*kind, |_ext, ev: Event, _cx| async move {
                let kind = oino_extension::events::wire_name(ev.kind);
                let ty = ev.get("type").cloned().unwrap_or(Value::Null);
                session::append_entry("echo.event", Some(&json!({"kind": kind, "type": ty})))?;
                if ev.kind == EventKind::ToolCall
                    && ev.get("toolName").and_then(Value::as_str) == Some("blocked")
                {
                    return Ok(Some(
                        json!({"block": true, "reason": "echo blocks `blocked`"}),
                    ));
                }
                Ok(None)
            });
        }
        cx.tool(Self::echo_tool());
        cx.command(
            Command::new("echo")
                .description("Echo arguments")
                .run_value(|_ext, args, _cx| async move { Ok(Some(json!({"args": args}))) })
                .complete(|_ext, prefix, _cx| async move {
                    vec![item(format!("{prefix}-one")), item(format!("{prefix}-two"))]
                }),
        );
        cx.command(
            Command::new("probe")
                .description("Probe a host import")
                .hidden()
                .run_value(|_ext, args, _cx| async move {
                    let (name, rest) = args.split_once(' ').unwrap_or((args.as_str(), "null"));
                    let arg: Value =
                        oino_extension::serde_json::from_str(rest).unwrap_or(Value::Null);
                    Ok(Some(match probe(name, arg).await {
                        Ok(v) => json!({"ok": v}),
                        Err(e) => json!({"err": e.to_string()}),
                    }))
                }),
        );
        cx.shortcut(
            Shortcut::new("echo.shortcut", &["ctrl+e"])
                .description("Echo shortcut")
                .run(|_ext, _cx| async {
                    session::append_entry("echo.shortcut", None::<&Value>)?;
                    Ok(())
                }),
        );
        cx.flag(Flag::boolean("echo-verbose", "Verbose echo"));
        cx.service("echo.ping", |_ext, payload, caller, cx| async move {
            Ok(json!({"pong": payload, "caller": caller, "session": cx.session_id()}))
        });
        cx.renderer(
            wit::registry::RendererTarget::CustomMessage,
            "echo.msg",
            |_ext, req, _cx| Some(json!({"type": "text", "text": req})),
        );
        Ok(())
    }
}

oino_extension::export_extension!(Echo);
