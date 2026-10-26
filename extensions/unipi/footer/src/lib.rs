//! `unipi.footer` — unipi `packages/footer` (unipi-port §3.6), ported as a status model: the
//! extension computes segments with priorities and frontends lay them out.
//!
//! | unipi | Here |
//! |---|---|
//! | `GlanceEditor` (`setEditorComponent`) top/bottom borders | `setFrame` segments ([`frame`]) |
//! | stats strip widget `footer-secondary` | `setFooter` priority segments ([`strip`]) |
//! | process line widget `footer-top` | `setWidget` aboveEditor from sticky `unipi:bg:tasks` ([`process`]) |
//! | working line (pending work) | host `run_state: waiting`, rendered by frontends (unipi-port §2.1) |
//! | TPS tracker + streaming hooks | [`tps`] fed by `turn_start`, `message_*`, `tool_execution_*`, `agent_settled` |
//! | 1 s incremental branch scan | [`scan`] on an `oino_timer` tick; fires UI requests only on change |
//! | `unipi.footer.shared-tps` | sticky topic `unipi:footer:tps` |
//! | sticky `LH_STATE`, `WORKFLOW_STATUS`, `KANBOARD_STATUS`, `FUSION_STATUS` | bus reads + `oino_bus` repaint |
//! | `/unipi:footer [on\|off]` | command; writes `footer.enabled` to the global settings file (a local override only when the host refuses the write) |
//! | settings `footer` | [`settings`] |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

pub mod frame;
pub mod process;
pub mod scan;
pub mod settings;
pub mod strip;
pub mod time;
pub mod tps;

use std::time::Duration;

use oino_extension::kv::Scope;
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use unipi_kit::bus::Delivery;
use unipi_kit::settings::Store;
use unipi_kit::topics::{self, FusionStatus, KanboardStatus, LhState, ModuleReady, WorkflowStatus};

use crate::frame::{Status, lh_mode_label};
use crate::scan::Scanner;
use crate::settings::{ColorMode, FooterSettings, IconStyle};
use crate::tps::TpsTracker;

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.footer";
/// Command.
pub const COMMAND: &str = "unipi:footer";
const OVERRIDE_KEY: &str = "enabledOverride";
const TICK_MS: u64 = 1_000;

/// Sticky topics that repaint the frame.
const FRAME_TOPICS: &[&str] = &[
    topics::LH_STATE,
    topics::KANBOARD_STATUS,
    topics::FUSION_STATUS,
    topics::WORKFLOW_STATUS,
    topics::BG_TASKS,
];

#[derive(Default)]
struct Sess {
    tracker: TpsTracker,
    scanner: Scanner,
    /// Branch-local index of the streaming assistant message (`None` = -1).
    streaming: Option<usize>,
    last: [Option<Value>; 4],
}

/// The extension.
#[derive(Default)]
pub struct Footer {
    settings: Store,
    sessions: SessionMap<Sess>,
}

fn tick_id(sid: &str) -> String {
    format!("tick:{sid}")
}

fn is_kept_assistant(m: &Value) -> bool {
    m.get("role").and_then(Value::as_str) == Some("assistant")
        && !matches!(
            m.get("stopReason").and_then(Value::as_str),
            Some("error" | "aborted")
        )
}

/// Persisted assistant messages − 1: the next `message_start` maps to the scan's index.
fn cursor_sync(branch: &[Value]) -> Option<usize> {
    let n = branch
        .iter()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("message"))
        .filter_map(|e| e.get("message"))
        .filter(|m| is_kept_assistant(m))
        .count();
    n.checked_sub(1)
}

/// Current git branch: `.git/HEAD` in the project (live), else the host's workspace info.
fn git_branch() -> Option<String> {
    let head = std::fs::read_to_string("/project/.git/HEAD")
        .ok()
        .or_else(|| {
            let gitfile = std::fs::read_to_string("/project/.git").ok()?;
            let dir = gitfile.trim().strip_prefix("gitdir:")?.trim().to_owned();
            let path = if dir.starts_with('/') {
                None
            } else {
                Some(format!("/project/{dir}/HEAD"))
            };
            path.and_then(|p| std::fs::read_to_string(p).ok())
        });
    match head {
        Some(h) => {
            let h = h.trim();
            Some(
                h.strip_prefix("ref: refs/heads/")
                    .map_or_else(|| h.chars().take(7).collect(), ToOwned::to_owned),
            )
        }
        None => host::workspace().git_branch,
    }
}

impl Footer {
    fn current(&self) -> FooterSettings {
        let mut s = FooterSettings::from_json(&self.settings.get(settings::NAMESPACE).0);
        if let Some(on) = kv::get_json::<bool>(Scope::Global, OVERRIDE_KEY)
            .ok()
            .flatten()
        {
            s.enabled = on;
        }
        s
    }

    /// unipi `setSettings("footer", {enabled})` in the global file; falls back to the local
    /// override when the write is refused.
    fn persist_enabled(&self, enable: bool) -> Result<()> {
        if self
            .settings
            .write(
                settings::NAMESPACE,
                "enabled",
                &json!(enable),
                unipi_kit::settings::WriteScope::Global,
            )
            .is_ok()
        {
            kv::delete(Scope::Global, OVERRIDE_KEY);
            return Ok(());
        }
        kv::set_json(Scope::Global, OVERRIDE_KEY, &enable)
    }

    /// One-time import of the pre-write-path override into the global settings file.
    fn import_override(&self) {
        if let Some(on) = kv::get_json::<bool>(Scope::Global, OVERRIDE_KEY)
            .ok()
            .flatten()
        {
            let _ = self.persist_enabled(on);
        }
    }

    fn fire_if_changed(&self, sid: &str, slot: usize, request: Value) {
        let changed = self.sessions.with(sid, |s| match s.last.get_mut(slot) {
            Some(c) if c.as_ref() != Some(&request) => {
                *c = Some(request.clone());
                Some(request)
            }
            _ => None,
        });
        if let Some(r) = changed {
            let _ = ui::fire(&r);
        }
    }

    fn clear_all(&self, sid: &str) {
        self.fire_if_changed(sid, 0, frame::clear_request());
        self.fire_if_changed(sid, 1, strip::clear_request());
        self.fire_if_changed(sid, 2, process::clear_request());
    }

    fn status(s: &FooterSettings, cx: &Ctx) -> Status {
        let usage = control::context_usage().unwrap_or(Value::Null);
        let model = control::model().unwrap_or(Value::Null);
        let raw_name = model
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .or_else(|| model.get("id").and_then(Value::as_str))
            .unwrap_or_default();
        let model_name = raw_name
            .strip_prefix("Claude ")
            .unwrap_or(raw_name)
            .to_owned();
        let wf = unipi_kit::bus::get::<WorkflowStatus>();
        let workspace = cx
            .cwd()
            .file_name()
            .map_or_else(|| "~".to_owned(), |n| n.to_string_lossy().into_owned());
        Status {
            workspace,
            lh_mode: lh_mode_label(unipi_kit::bus::get::<LhState>().as_ref()),
            plan_mode: wf.as_ref().is_some_and(|w| w.plan_mode),
            permission_mode: wf.and_then(|w| w.permission_mode).filter(|p| !p.is_empty()),
            branch: git_branch(),
            context_pct: usage.get("percent").and_then(Value::as_f64),
            context_window: usage
                .get("contextWindow")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            model_name,
            thinking: Some(control::thinking_level()).filter(|t| !t.is_empty()),
            fusion: unipi_kit::bus::get_raw(topics::FUSION_STATUS)
                .and_then(|v| serde_json::from_value::<Option<FusionStatus>>(v).ok())
                .flatten(),
            kanboard: unipi_kit::bus::get::<KanboardStatus>(),
            rainbow: s.rainbow,
            badges: s.badges,
            icon_style: s.icon_style,
            color: s.color_mode != ColorMode::None,
        }
    }

    /// One tick: incremental scan, then repaint frame/strip/process line on change.
    fn paint(&self, cx: &Ctx) {
        let sid = cx.session_id();
        let s = self.current();
        if !s.enabled || !cx.has_ui() {
            self.clear_all(sid);
            return;
        }
        let now = timers::now_ms();
        let branch = session::branch(None).unwrap_or_default();
        let subscription = control::model().is_some_and(|m| {
            ["subscription", "usingOAuth", "usingSubscription"]
                .iter()
                .any(|k| m.get(*k).and_then(Value::as_bool) == Some(true))
        });
        let (parts, tps) = self.sessions.with(sid, |st| {
            if st.scanner.needs_full_rescan(&branch) {
                st.scanner.full_rescan(&branch, &mut st.tracker, now);
                st.streaming = cursor_sync(&branch);
            } else {
                st.scanner.scan(&branch, &mut st.tracker, now);
            }
            let cx = strip::Context {
                tracker: &st.tracker,
                subscription,
                text_icons: s.icon_style == IconStyle::Text,
                color: s.color_mode != ColorMode::None,
                now,
            };
            let t = &st.tracker;
            let tps = json!({
                "liveTps": t.live_tps(now),
                "sessionAvgTps": t.session_avg_tps(now),
                "avgTtftMs": t.avg_ttft_ms(),
                "streaming": t.is_streaming(),
                "totalOutput": t.total_output(),
                "turns": t.turns(),
                "steps": t.steps(),
                "llmMs": t.llm_ms(now),
                "toolMs": t.tool_ms(),
            });
            (strip::build_parts(&s.strip, &st.scanner.snapshot, &cx), tps)
        });
        let mut frame = frame::frame_request(&Self::status(&s, cx));
        frame["iconStyle"] = json!(match s.icon_style {
            IconStyle::Nerd => "nerd",
            IconStyle::Emoji => "emoji",
            IconStyle::Text => "text",
        });
        frame["colorMode"] = json!(s.color_mode.as_str());
        self.fire_if_changed(sid, 0, frame);
        self.fire_if_changed(sid, 1, strip::footer_request(&parts));
        let line = if s.process_line {
            process::widget_request(
                process::count(unipi_kit::bus::get_raw(topics::BG_TASKS).as_ref()),
                s.color_mode != ColorMode::None,
            )
        } else {
            process::clear_request()
        };
        self.fire_if_changed(sid, 2, line);
        let changed = self.sessions.with(sid, |st| {
            let c = st.last[3].as_ref() != Some(&tps);
            if c {
                st.last[3] = Some(tps.clone());
            }
            c
        });
        if changed {
            unipi_kit::bus::emit_raw(topics::FOOTER_TPS, &tps);
        }
    }

    fn start(&self, cx: &Ctx) {
        let sid = cx.session_id();
        if self.current().enabled && cx.has_ui() {
            timers::set(&tick_id(sid), Duration::from_millis(TICK_MS), true);
        } else {
            timers::cancel(&tick_id(sid));
        }
        self.paint(cx);
    }

    fn on_stream_event(&self, ev: &Event, cx: &Ctx) {
        let now = timers::now_ms();
        let sid = cx.session_id();
        let m = ev.get("message").cloned().unwrap_or(Value::Null);
        self.sessions.with(sid, |st| match ev.kind {
            EventKind::TurnStart => {
                st.tracker
                    .on_turn_start(ev.get("timestamp").and_then(Value::as_i64), now);
            }
            EventKind::AgentSettled => st.tracker.on_turn_end(now),
            EventKind::ToolExecutionStart => {
                if let Some(id) = ev.get("toolCallId").and_then(Value::as_str) {
                    st.tracker.on_tool_call_start(id, now);
                }
            }
            EventKind::ToolExecutionEnd => {
                if let Some(id) = ev.get("toolCallId").and_then(Value::as_str) {
                    st.tracker.on_tool_call_end(id, now);
                }
            }
            EventKind::MessageStart if is_kept_assistant(&m) => {
                let next = st.streaming.map_or(0, |i| i + 1);
                st.streaming = Some(next);
                st.tracker.on_message_start(next);
            }
            EventKind::MessageUpdate
                if m.get("role").and_then(Value::as_str) == Some("assistant") =>
            {
                let Some(i) = st.streaming else { return };
                let e = ev.get("assistantMessageEvent");
                let kind = e.and_then(|e| e.get("type")).and_then(Value::as_str);
                if matches!(
                    kind,
                    Some("text_delta" | "thinking_delta" | "toolcall_delta")
                ) && let Some(delta) = e.and_then(|e| e.get("delta")).and_then(Value::as_str)
                {
                    st.tracker.on_streaming_delta(i, delta, now);
                }
            }
            EventKind::MessageEnd if is_kept_assistant(&m) => {
                if let Some(i) = st.streaming {
                    st.tracker.on_message_end(i, Some(&m), now);
                }
            }
            _ => {}
        });
    }

    fn command(&self, args: &str, cx: &CommandCtx) -> Result<()> {
        let enable = match args.trim().to_lowercase().as_str() {
            "on" => true,
            "off" => false,
            _ => !self.current().enabled,
        };
        self.persist_enabled(enable)?;
        if enable {
            self.sessions.with(cx.session_id(), |s| {
                s.scanner.reset();
                s.last = Default::default();
            });
            self.start(&cx.cx);
            let _ = ui::notify("Footer enabled", "info");
        } else {
            timers::cancel(&tick_id(cx.session_id()));
            self.clear_all(cx.session_id());
            let _ = ui::notify("Footer disabled", "info");
        }
        Ok(())
    }
}

const STREAM_EVENTS: &[EventKind] = &[
    EventKind::TurnStart,
    EventKind::MessageStart,
    EventKind::MessageUpdate,
    EventKind::MessageEnd,
    EventKind::ToolExecutionStart,
    EventKind::ToolExecutionEnd,
    EventKind::AgentSettled,
];

impl Extension for Footer {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = settings::namespace();
        self.settings
            .declare(settings::NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&ns.to_json());

        cx.command(
            Command::<Self>::new(COMMAND)
                .description("Toggle footer on/off")
                .argument_hint("[on|off]")
                .run(|ext, args, cx| async move { ext.command(&args, &cx) }),
        );
        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.settings.refresh();
            if !cx.is_child() {
                ext.import_override();
            }
            ext.sessions.remove(cx.session_id());
            unipi_kit::bus::emit(&ModuleReady {
                name: unipi_kit::suite::pkg::FOOTER.npm.to_owned(),
                version: unipi_kit::suite::UNIPI_VERSION.to_owned(),
                commands: vec![COMMAND.to_owned()],
                tools: Vec::new(),
                load_time_ms: None,
            });
            ext.start(&cx);
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            timers::cancel(&tick_id(cx.session_id()));
            ext.sessions.remove(cx.session_id());
            Ok(())
        });
        for kind in STREAM_EVENTS {
            cx.on(*kind, |ext, ev, cx| async move {
                ext.on_stream_event(&ev, &cx);
                Ok(())
            });
        }
        cx.on(EventKind::OinoTimer, |ext, ev, cx| async move {
            if ev.get("timerId").and_then(Value::as_str) == Some(tick_id(cx.session_id()).as_str())
            {
                ext.paint(&cx);
            }
            Ok(())
        });
        cx.on(EventKind::OinoBus, |ext, ev, cx| async move {
            if Delivery::from_event(&ev).is_some_and(|d| FRAME_TOPICS.contains(&d.topic.as_str())) {
                ext.paint(&cx);
            }
            Ok(())
        });
        cx.on(EventKind::ModelSelect, |ext, _ev, cx| async move {
            ext.paint(&cx);
            Ok(())
        });
        cx.on(EventKind::ThinkingLevelSelect, |ext, _ev, cx| async move {
            ext.paint(&cx);
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, cx| async move {
            if ext.settings.on_changed(&ev.payload).is_some() {
                // A real settings write supersedes the command's override.
                kv::delete(Scope::Global, OVERRIDE_KEY);
                ext.start(&cx);
            }
            Ok(())
        });
        unipi_kit::bus::subscribe(FRAME_TOPICS);
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Footer);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_sync_counts_kept_assistant_messages() {
        let b = vec![
            json!({"type": "message", "message": {"role": "user"}}),
            json!({"type": "message", "message": {"role": "assistant", "stopReason": "stop"}}),
            json!({"type": "message", "message": {"role": "assistant", "stopReason": "aborted"}}),
            json!({"type": "message", "message": {"role": "assistant", "stopReason": "toolUse"}}),
        ];
        assert_eq!(cursor_sync(&b), Some(1));
        assert_eq!(cursor_sync(&[]), None);
    }
}
