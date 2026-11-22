//! Session caps and manual shell detach. Decision-model watchdog judging was removed.
#![allow(clippy::unused_async_trait_impl, reason = "SDK handlers are async")]

pub mod caps;
pub mod settings;

use crate::caps::{CAPS_ENTRY, Caps, RunState, Verdict};
use crate::settings::NAMESPACE;
use oino_extension::command::{Flag, get_flag};
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use unipi_kit::settings::Store;

#[derive(Default)]
struct State {
    run: RunState,
    caps_pins: Value,
}

/// Session caps extension; no model-based execution judging.
#[derive(Default)]
pub struct Watchdog {
    settings: Store,
    sessions: SessionMap<State>,
}

impl Watchdog {
    fn bg_detach(args: &str) {
        let reason = if args.trim().is_empty() {
            "manual /bg-detach"
        } else {
            args.trim()
        };
        let latest = serde_json::from_str::<Value>(&control::running_bash())
            .ok()
            .and_then(|v| v.as_array().cloned())
            .and_then(|list| {
                list.into_iter()
                    .max_by_key(|r| r.get("startedAt").and_then(Value::as_i64).unwrap_or(0))
            })
            .and_then(|r| {
                r.get("toolCallId")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            });
        let detached = latest.and_then(|id| control::bash_detach(&id, reason).ok());
        let _ = match detached {
            Some(task) => {
                let output = tasks::get(&task)
                    .ok()
                    .flatten()
                    .and_then(|t| {
                        t.get("outputPath")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    })
                    .unwrap_or_default();
                ui::notify(
                    &format!(
                        "Moved the running bash call to background task {task} (output: {output})"
                    ),
                    "info",
                )
            }
            None => ui::notify(
                "Could not detach: no bash call is currently running, or background detachment is unavailable.",
                "warning",
            ),
        };
    }

    // ── session caps ────────────────────────────────────────────────────────

    fn caps(&self, cx: &Ctx) -> Caps {
        let pins = self.sessions.with(cx.session_id(), |s| s.caps_pins.clone());
        Caps::resolve_with_pins(&self.settings.get(NAMESPACE).0, &pins, get_flag)
    }

    /// Reloads this session's pinned caps from the last [`CAPS_ENTRY`] entry
    /// (`SessionStart`; the pins survive restarts because they live in the session file).
    fn load_caps_pins(&self, cx: &Ctx) {
        // perf-fix #1: only this extension's caps entries cross into the guest.
        let pins = session::query(&session::EntryQuery::custom(&[CAPS_ENTRY]))
            .ok()
            .and_then(|entries| {
                entries
                    .iter()
                    .rev()
                    .filter(|e| e.get("type").and_then(Value::as_str) == Some("custom"))
                    .filter(|e| e.get("customType").and_then(Value::as_str) == Some(CAPS_ENTRY))
                    .find_map(|e| e.get("data").filter(|d| d.is_object()).cloned())
            })
            .unwrap_or(Value::Null);
        self.sessions.with(cx.session_id(), |s| s.caps_pins = pins);
    }

    async fn on_turn_end(&self, ev: &Event, cx: &Ctx) {
        let caps = self.caps(cx);
        let now = timers::now_ms();
        let (verdict, run) = self.sessions.with(cx.session_id(), |st| {
            (st.run.turn_end(&caps, &ev.payload, now), st.run.clone())
        });
        let Verdict::Abort { reason, summary } = verdict else {
            return;
        };
        control::abort();
        let _ = ui::notify(&caps::notice(&reason), "warning");
        let _ = session::append_entry(
            "watchdog.session-cap",
            Some(
                &json!({"reason": reason, "turns": run.turns, "costUsd": run.cost_usd, "consecutiveToolErrors": run.consecutive_tool_errors}),
            ),
        );
        if summary {
            let _ = session::send_user_message(
                &json!(caps::summary_message(&reason)),
                Some(session::Delivery::FollowUp),
            )
            .await;
        }
    }
}

impl Extension for Watchdog {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = settings::namespace();
        self.settings.declare(NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&ns.to_json());
        for (name, description) in [
            ("max-turns", "Session cap: abort a run after N turns"),
            ("max-cost", "Session cap: abort a run after spending N USD"),
            ("max-wall", "Session cap: abort a run after N minutes"),
            (
                "max-tool-errors",
                "Session cap: abort after N consecutive tool errors",
            ),
        ] {
            cx.flag(Flag::string(name, description));
        }
        cx.command(
            Command::<Self>::new("bg-detach")
                .description("Move the currently running bash call to a background task")
                .argument_hint("[reason]")
                .run(|_ext, args, _cx| async move {
                    Self::bg_detach(&args);
                    Ok(())
                }),
        );
        // Session caps for the app's Session modal › Info › Limits (WP-A17, G-16):
        // `caps.get` → `{caps, pinned}`; `caps.set` pins for this session and persists
        // the pins as a `watchdog.session-caps` custom entry.
        cx.service("caps.get", |ext, _payload, _caller, cx| async move {
            let pinned = ext
                .sessions
                .with(cx.session_id(), |s| s.caps_pins.is_object());
            Ok(json!({"caps": ext.caps(&cx).to_json(), "pinned": pinned}))
        });
        cx.service("caps.set", |ext, payload, _caller, cx| async move {
            let pins = caps::sanitize_pins(&payload);
            session::append_entry(CAPS_ENTRY, Some(&pins))?;
            ext.sessions.with(cx.session_id(), |s| s.caps_pins = pins);
            let caps = ext.caps(&cx);
            Ok(
                json!({"caps": caps.to_json(), "pinned": caps.max_turns.is_some()
                || caps.max_cost_usd.is_some()
                || caps.max_wall_minutes.is_some()
                || caps.max_consecutive_tool_errors.is_some()}),
            )
        });
        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.load_caps_pins(&cx);
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.sessions.remove(cx.session_id());
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            ext.settings.on_changed(&ev.payload);
            Ok(())
        });
        cx.on(EventKind::AgentStart, |ext, _ev, cx| async move {
            let now = timers::now_ms();
            ext.sessions.with(cx.session_id(), |st| st.run.start(now));
            Ok(())
        });
        cx.on(EventKind::TurnEnd, |ext, ev, cx| async move {
            ext.on_turn_end(&ev, &cx).await;
            Ok(())
        });
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Watchdog);
