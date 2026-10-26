//! Async probe fixture: concurrency, cancellation, deadlines, services and traps.

#![allow(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    reason = "SDK handler signatures are async even when a body does not await"
)]

use std::cell::Cell;
use std::time::Duration;

use oino_extension::command::Command;
use oino_extension::prelude::*;

/// Fixture state: counts events seen while tools are in flight.
#[derive(Default)]
struct AsyncProbe {
    events: Cell<u32>,
}

impl Extension for AsyncProbe {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        cx.on(
            EventKind::TurnStart,
            |ext: &'static Self, _ev, _cx| async move {
                ext.events.set(ext.events.get() + 1);
                Ok(())
            },
        );
        cx.on(EventKind::AgentStart, |_ext, _ev, _cx| async move {
            // Exceeds the manifest hookTimeoutMs (200 ms).
            timers::sleep(Duration::from_millis(2000)).await;
            Ok(())
        });
        cx.on(EventKind::OinoTask, |_ext, ev: Event, _cx| async move {
            session::append_entry("async.task", Some(&ev.payload))?;
            Ok(())
        });
        cx.on(EventKind::OinoBus, |_ext, ev: Event, _cx| async move {
            session::append_entry("async.bus", Some(&ev.payload))?;
            Ok(())
        });
        cx.tool(
            Tool::new("wait_dialog")
                .description("Waits for a dialog answer")
                .run(|ext: &'static Self, _a: Value, _cx| async move {
                    let before = ext.events.get();
                    let answer =
                        ui::dialog(&json!({"method": "confirm", "title": "continue?"})).await?;
                    Ok(ToolResult::text("answered").with_details(
                        json!({"answer": answer, "eventsDuring": ext.events.get() - before}),
                    ))
                }),
        );
        cx.tool(
            Tool::new("wait_cancel")
                .description("Sleeps until cancelled")
                .run(|_ext, _a: Value, cx: ToolCtx| async move {
                    for _ in 0..200 {
                        if cx.is_cancelled() {
                            return Ok(ToolResult::text("cancelled"));
                        }
                        timers::sleep(Duration::from_millis(10)).await;
                    }
                    Ok(ToolResult::text("finished"))
                }),
        );
        cx.tool(
            Tool::new("crash")
                .description("Traps")
                .run(|_ext, _a: Value, _cx| async move {
                    let v: Vec<u8> = Vec::new();
                    #[allow(clippy::indexing_slicing, reason = "deliberate trap")]
                    let byte = v[usize::from(v.is_empty())];
                    Ok(ToolResult::text(byte.to_string()))
                }),
        );
        cx.tool(Tool::new("count").description("Events seen").run(
            |ext: &'static Self, _a: Value, _cx| async move {
                Ok(ToolResult::text(ext.events.get().to_string()))
            },
        ));
        cx.service("add", |_ext, payload, _caller, _cx| async move {
            let a = payload["a"].as_i64().unwrap_or(0);
            let b = payload["b"].as_i64().unwrap_or(0);
            Ok(json!(a + b))
        });
        cx.command(
            Command::new("call")
                .description("Call a service")
                .run_value(|_ext, args, _cx| async move {
                    let (target, name) = args.split_once(' ').unwrap_or(("", args.as_str()));
                    let target = (!target.is_empty()).then_some(target);
                    services::call(target, name, &json!({"a": 2, "b": 3}), None)
                        .await
                        .map(Some)
                }),
        );
        cx.command(
            Command::new("bus")
                .description("Subscribe/publish")
                .run_value(|_ext, args, _cx| async move {
                    match args.split_once(' ') {
                        Some(("sub", pattern)) => bus::subscribe(pattern),
                        Some(("pub", topic)) => {
                            bus::publish(topic, &json!({"from": "async"}), false)?;
                        }
                        _ => return Err(Error::msg("usage: bus sub|pub <topic>")),
                    }
                    Ok(None)
                }),
        );
        cx.command(Command::new("task").description("Start a task").run_value(|_ext, args, _cx| async move {
            Ok(Some(json!(tasks::start(&json!({"name": "probe", "command": args, "notifyOnCompletion": true, "triggerOnCompletion": false}))?)))
        }));
        Ok(())
    }
}

oino_extension::export_extension!(AsyncProbe);
