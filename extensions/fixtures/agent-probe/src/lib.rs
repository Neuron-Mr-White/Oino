//! Agent probe fixture: nested agents (`agents.spawn`), the turn arbiter (`propose-nudge`,
//! `wait-reason`, `nudge-delivered`) and control operations (`run-state`, `shutdown`).
//!
//! - tool `delegate {task}`: spawns a child (no extensions, no tools), prompts it, streams its
//!   events until it settles, closes it and returns its answer with the event types seen.
//! - command `spawn <AgentSpec JSON>`: spawns, prompts `"probe task"`, waits idle and reports
//!   `{id, sessionFile, text, usage, listed}` (the agent stays open); `close` closes it.
//! - commands `nudge on|off`, `wait <reason>|off`, `run-state`, `shutdown`, `child`.

#![allow(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    reason = "SDK handler signatures are async even when a body does not await"
)]

use std::cell::{Cell, RefCell};

use oino_extension::agents::{self, Agent};
use oino_extension::command::Command;
use oino_extension::futures::StreamExt;
use oino_extension::prelude::*;

/// Fixture state.
#[derive(Default)]
struct AgentProbe {
    nudge: Cell<bool>,
    wait: RefCell<Option<String>>,
    agent: RefCell<Option<Agent>>,
}

impl AgentProbe {
    async fn delegate(task: &str) -> Result<ToolResult> {
        let spec = json!({"label": "delegate", "childRole": "probe", "tools": {"builtin": []}});
        let agent = Agent::spawn(&spec).await?;
        let mut events = agent.events();
        agent.prompt(task).await?;
        let mut kinds = Vec::new();
        while let Some(event) = events.next().await {
            let kind = event["type"].as_str().unwrap_or_default().to_owned();
            let settled = kind == "agent_settled";
            kinds.push(kind);
            if settled {
                break;
            }
        }
        let text = agent.last_assistant_text().unwrap_or_default();
        let usage = agent.usage()?;
        agent.close(0).await;
        Ok(ToolResult::text(text).with_details(json!({"events": kinds, "usage": usage})))
    }

    async fn spawn_command(&self, args: &str) -> Result<Option<Value>> {
        let spec: Value = oino_extension::serde_json::from_str(args)?;
        let agent = Agent::spawn(&spec).await?;
        let disposition = agent.prompt("probe task").await?;
        agent.wait_idle().await?;
        let report = json!({
            "id": agent.id(),
            "disposition": disposition,
            "sessionFile": agent.session_file(),
            "text": agent.last_assistant_text(),
            "usage": agent.usage()?,
            "listed": agents::list()?,
        });
        *self.agent.borrow_mut() = Some(agent);
        Ok(Some(report))
    }
}

impl Extension for AgentProbe {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        cx.tool(
            Tool::new("delegate")
                .description("Delegates a task to a nested agent")
                .parameters(&json!({"type": "object", "properties": {"task": {"type": "string"}}, "required": ["task"]}))
                .run(|_ext, args: Value, _cx| async move {
                    Self::delegate(args["task"].as_str().unwrap_or_default()).await
                }),
        );
        cx.command(
            Command::new("spawn")
                .description("Spawn a nested agent from an AgentSpec")
                .run_value(|ext: &'static Self, args, _cx| async move {
                    ext.spawn_command(&args).await
                }),
        );
        cx.command(
            Command::new("close")
                .description("Close the spawned agent")
                .run_value(|ext: &'static Self, _args, _cx| async move {
                    let agent = ext.agent.borrow_mut().take();
                    if let Some(agent) = agent {
                        agent.close(0).await;
                    }
                    Ok(Some(json!(agents::list()?)))
                }),
        );
        cx.command(
            Command::new("nudge")
                .description("Arm the nudge")
                .run_value(|ext: &'static Self, args, _cx| async move {
                    ext.nudge.set(args.trim() == "on");
                    Ok(None)
                }),
        );
        cx.command(
            Command::new("wait")
                .description("Set the wait reason")
                .run_value(|ext: &'static Self, args, _cx| async move {
                    let reason = args.trim();
                    *ext.wait.borrow_mut() = (reason != "off").then(|| reason.to_owned());
                    Ok(None)
                }),
        );
        cx.command(
            Command::new("run-state")
                .description("control.run-state")
                .run_value(|_ext, _args, _cx| async move { Ok(Some(control::run_state()?)) }),
        );
        cx.command(
            Command::new("shutdown")
                .description("control.shutdown")
                .run_value(|_ext, _args, _cx| async move {
                    oino_extension::wit::control::shutdown();
                    Ok(None)
                }),
        );
        cx.nudge_provider(|ext: &'static Self, cx| async move {
            if cx.is_child() || !ext.nudge.get() {
                return None;
            }
            Some(json!({"id": "probe-nudge", "priority": 5, "customType": "probe:nudge", "content": "nudged: keep going", "display": true}))
        });
        cx.on_nudge_delivered(|ext: &'static Self, id, _cx| {
            ext.nudge.set(false);
            let _ = session::append_entry("probe.delivered", Some(&json!({"id": id})));
        });
        cx.wait_source(|ext: &'static Self, _cx| async move { ext.wait.borrow().clone() });
        Ok(())
    }
}

oino_extension::export_extension!(AgentProbe);
