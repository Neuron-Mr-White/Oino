//! `unipi.input-shortcuts` — unipi `packages/input-shortcuts` (unipi-port §3.4).
//!
//! | unipi | Here |
//! |---|---|
//! | `Alt+S` chord overlay (`ctx.ui.custom`) | shortcut `unipi.chord` (`alt+s`) → `custom` dialog with letter keys ([`actions::chord_dialog`]) |
//! | `S` stash/restore, `A` append stash | [`stash`] persisted in workspace state |
//! | `U`/`R` undo/redo, typing bursts via `onTerminalInput` | [`history`] + [`burst`] fed by `changed` editor intents and `oino_timer` |
//! | `Y` copy last reply (`copyToClipboard`) | [`actions::last_response_text`] → `clipboard-write-text` |
//! | `K` kanboard `captureToBacklog` (`globalThis`) | service `unipi.kanboard/kanboard.capture` |
//! | `Alt+I` insert tab | shortcut `unipi.tab-insert` (`alt+i`) |
//! | Ctrl+C keeps cleared text (`unipi-cleared-input` entry + renderer) | `cleared` editor intent → [`cleared`] entry + OUS renderer |
//! | status 2 s / 3 s | `setStatus` + timer |
//! | info-screen group `input-shortcuts` | service `info-screen.groups` |
//! | settings `chordKey`, `tabInsertKey` | [`settings`] (stored, shown; keys stay on their defaults like unipi) |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

pub mod actions;
pub mod burst;
pub mod cleared;
pub mod history;
pub mod settings;
pub mod stash;

use std::cell::RefCell;
use std::time::Duration;

use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use oino_extension::wit::registry::RendererTarget;
use unipi_kit::settings::Store;
use unipi_kit::topics::ModuleReady;

use crate::actions::{Action, capture_outcome, capture_status, chord_dialog, last_response_text};
use crate::burst::{Burst, TimerOutcome};
use crate::history::EditHistory;
use crate::stash::Stash;

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.input-shortcuts";
/// Status key.
pub const STATUS_KEY: &str = "input-shortcuts";
/// Success status lifetime.
pub const STATUS_SUCCESS_MS: u64 = 2_000;
/// Error status lifetime.
pub const STATUS_ERROR_MS: u64 = 3_000;
/// Kanboard extension and its capture service.
pub const KANBOARD: (&str, &str) = ("unipi.kanboard", "kanboard.capture");

/// Per-session editor tracking.
#[derive(Default)]
struct Editor {
    history: EditHistory,
    burst: Burst,
    /// Last text reported by the frontend (or set by us).
    text: String,
    /// Text we just set; its `changed` echo is not typing.
    echo: Option<String>,
    /// A submit happened since the last non-empty change.
    after_submit: bool,
}

/// The extension.
#[derive(Default)]
pub struct InputShortcuts {
    settings: Store,
    stash: RefCell<Stash>,
    editors: SessionMap<Editor>,
}

fn timer_id(kind: &str, sid: &str) -> String {
    format!("{kind}:{sid}")
}

impl InputShortcuts {
    fn stash(&self) -> std::cell::RefMut<'_, Stash> {
        let mut s = self.stash.borrow_mut();
        s.set_path(unipi_kit::paths::workspace(stash::FILE));
        s
    }

    fn show(sid: &str, text: &str, ok: bool) {
        let _ = ui::status(STATUS_KEY, Some(text));
        let ms = if ok {
            STATUS_SUCCESS_MS
        } else {
            STATUS_ERROR_MS
        };
        timers::set(&timer_id("status", sid), Duration::from_millis(ms), false);
    }

    fn success(sid: &str, text: &str) {
        Self::show(sid, text, true);
    }

    fn error(sid: &str, text: &str) {
        Self::show(sid, text, false);
    }

    fn editor_text(&self, sid: &str) -> String {
        let live = ui::editor_text();
        self.editors.with(sid, |e| e.text.clone_from(&live));
        live
    }

    fn set_text(&self, sid: &str, text: &str) {
        ui::editor_set(text);
        self.editors.with(sid, |e| {
            text.clone_into(&mut e.text);
            e.echo = Some(text.to_owned());
        });
    }

    fn record(&self, sid: &str, text: &str) {
        self.editors.with(sid, |e| e.history.record(text));
    }

    /// Flushes an open burst (undo/redo capture un-flushed typing first).
    fn close_burst(&self, sid: &str) {
        let was_open = self.editors.with(sid, |e| e.burst.close());
        timers::cancel(&timer_id("burst", sid));
        if was_open {
            let text = self.editor_text(sid);
            self.record(sid, &text);
        }
    }

    // ── actions ──────────────────────────────────────────────────────────────

    fn do_stash(&self, sid: &str) {
        let text = self.editor_text(sid);
        if text.is_empty() {
            let stash = self.stash().get();
            if stash.is_empty() {
                return Self::error(sid, "stash empty");
            }
            self.record(sid, "");
            self.set_text(sid, &stash);
            self.record(sid, &stash);
            Self::success(sid, "✓ stash restored");
        } else {
            self.record(sid, &text);
            self.stash().set(&text);
            self.set_text(sid, "");
            self.record(sid, "");
            Self::success(sid, "✓ stash saved");
        }
    }

    fn do_step(&self, sid: &str, undo: bool) {
        self.close_burst(sid);
        let current = self.editor_text(sid);
        let step = self.editors.with(sid, |e| {
            if undo {
                e.history.undo(&current)
            } else {
                e.history.redo(&current)
            }
        });
        if step.ok {
            self.set_text(sid, &step.text);
            Self::success(sid, if undo { "✓ undo" } else { "✓ redo" });
        } else {
            Self::error(sid, step.reason.unwrap_or("nothing to undo"));
        }
    }

    fn do_append_stash(&self, sid: &str) {
        let stash = self.stash().get();
        if stash.is_empty() {
            return Self::error(sid, "stash empty");
        }
        let before = self.editor_text(sid);
        self.record(sid, &before);
        let after = format!("{before}{stash}");
        self.set_text(sid, &after);
        self.record(sid, &after);
        Self::success(sid, "✓ stash appended");
    }

    fn do_copy(sid: &str) {
        let branch = session::branch(None).unwrap_or_default();
        let Some(text) = last_response_text(&branch) else {
            return Self::error(sid, "no response to copy");
        };
        match ui::clipboard_write_text(&text) {
            Ok(()) => Self::success(sid, "✓ copied last response"),
            Err(e) => Self::error(sid, &format!("copy failed: {e}")),
        }
    }

    async fn do_kanboard(&self, cx: &CommandCtx) {
        let sid = cx.session_id().to_owned();
        let text = self.editor_text(&sid);
        if text.trim().is_empty() {
            return Self::error(&sid, "nothing to add");
        }
        let loaded = services::list()
            .iter()
            .any(|(ext, name)| ext == KANBOARD.0 && name == KANBOARD.1);
        if !loaded {
            return Self::error(&sid, "kanboard not loaded");
        }
        let payload = json!({"cwd": cx.cx.cwd().to_string_lossy(), "text": text, "body": text});
        let reply = services::call(Some(KANBOARD.0), KANBOARD.1, &payload, None).await;
        let outcome = match reply {
            Ok(v) => capture_outcome(&v),
            Err(e) => Err(format!("kanboard: {e}")),
        };
        match outcome {
            Err(reason) => Self::error(&sid, &reason),
            Ok((id, attachments)) => {
                // Only clear when the user has not typed since the capture started.
                if ui::editor_text() == text {
                    self.record(&sid, &text);
                    self.set_text(&sid, "");
                    self.record(&sid, "");
                }
                Self::success(&sid, &capture_status(&id, attachments));
            }
        }
    }

    async fn chord(&self, cx: CommandCtx) -> Result<()> {
        if !cx.has_ui() {
            return Ok(());
        }
        let answer = ui::dialog(&chord_dialog()).await?;
        let Some(action) = answer
            .get("value")
            .and_then(|v| v.get("actionId"))
            .and_then(Value::as_str)
            .and_then(Action::parse)
        else {
            return Ok(());
        };
        let sid = cx.session_id().to_owned();
        match action {
            Action::Stash => self.do_stash(&sid),
            Action::Undo => self.do_step(&sid, true),
            Action::Redo => self.do_step(&sid, false),
            Action::AppendStash => self.do_append_stash(&sid),
            Action::CopyLastResponse => Self::do_copy(&sid),
            Action::Kanboard => self.do_kanboard(&cx).await,
        }
        Ok(())
    }

    fn tab_insert(&self, sid: &str) {
        let text = self.editor_text(sid);
        self.set_text(sid, &format!("{text}\t"));
    }

    // ── editor intents and timers ────────────────────────────────────────────

    fn on_editor(&self, ev: &Event, cx: &Ctx) {
        let sid = cx.session_id();
        let intent = ev.get("intent").and_then(Value::as_str).unwrap_or_default();
        let text = ev
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        match intent {
            "changed" => {
                let now = timers::now_ms();
                let opened = self.editors.with(sid, |e| {
                    if e.echo.as_deref() == Some(text.as_str()) {
                        e.echo = None;
                        return None;
                    }
                    e.echo = None;
                    if !text.is_empty() {
                        e.after_submit = false;
                    }
                    let previous = std::mem::replace(&mut e.text, text.clone());
                    e.burst
                        .on_input(now)
                        .inspect(|_| e.history.record(&previous))
                });
                if let Some(delay) = opened {
                    timers::set(
                        &timer_id("burst", sid),
                        Duration::from_millis(u64::try_from(delay).unwrap_or(0)),
                        false,
                    );
                }
            }
            "submit" => self.editors.with(sid, |e| e.after_submit = true),
            "cleared" => {
                let reason = ev
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let previous = self.editors.with(sid, |e| {
                    let prev = std::mem::take(&mut e.text);
                    let keep =
                        cleared::is_interrupt_clear(&prev, reason.as_deref(), e.after_submit);
                    e.after_submit = false;
                    keep.then_some(prev)
                });
                if let Some(prev) = previous {
                    let _ =
                        session::append_entry(cleared::CLEARED_ENTRY, Some(&json!({"text": prev})));
                }
            }
            _ => {}
        }
    }

    fn on_timer(&self, ev: &Event, cx: &Ctx) {
        let Some(id) = ev.get("timerId").and_then(Value::as_str) else {
            return;
        };
        let sid = cx.session_id();
        if id == timer_id("status", sid) {
            let _ = ui::status(STATUS_KEY, None);
        } else if id == timer_id("burst", sid) {
            let now = timers::now_ms();
            match self.editors.with(sid, |e| e.burst.on_timer(now)) {
                TimerOutcome::Rearm(delay) => timers::set(
                    id,
                    Duration::from_millis(u64::try_from(delay).unwrap_or(0)),
                    false,
                ),
                TimerOutcome::Close => {
                    let text = self.editors.with(sid, |e| e.text.clone());
                    self.record(sid, &text);
                }
                TimerOutcome::Idle => {}
            }
        }
    }

    fn info_group(&self) -> Value {
        let s = self.settings.get(settings::NAMESPACE);
        let chord = settings::key_or_default(s.get("chordKey"), settings::CHORD_KEY);
        let tab = settings::key_or_default(s.get("tabInsertKey"), settings::TAB_INSERT_KEY);
        let stash = self.stash().get().encode_utf16().count();
        json!({
            "id": "input-shortcuts",
            "name": "Input Shortcuts",
            "icon": "⌨️",
            "priority": 115,
            "config": {"showByDefault": true, "stats": [
                {"id": "chordKey", "label": "Chord key", "show": true},
                {"id": "tabInsertKey", "label": "Tab insert key", "show": true},
                {"id": "stashStatus", "label": "Stash", "show": true},
            ]},
            "data": {
                "raw": {"value": "", "raw": {"chordKey": chord, "tabInsertKey": tab, "stash": stash}},
                "chordKey": {"value": chord, "detail": "Key to open shortcuts overlay"},
                "tabInsertKey": {"value": tab, "detail": "Key to insert tab"},
                "stashStatus": {"value": if stash > 0 { "set" } else { "empty" }, "detail": "Stash register"},
            },
        })
    }
}

impl Extension for InputShortcuts {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = settings::namespace();
        self.settings
            .declare(settings::NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&ns.to_json());

        cx.shortcut(
            Shortcut::<Self>::new("unipi.chord", &[settings::CHORD_KEY])
                .description(
                    "Input shortcuts — stash, undo, redo, append stash, copy last response, kanboard",
                )
                .run(InputShortcuts::chord),
        );
        cx.shortcut(
            Shortcut::<Self>::new("unipi.tab-insert", &[settings::TAB_INSERT_KEY])
                .description("Insert tab character into input")
                .run(|ext, cx| async move {
                    ext.tab_insert(cx.session_id());
                    Ok(())
                }),
        );
        cx.renderer(
            RendererTarget::CustomEntry,
            cleared::CLEARED_ENTRY,
            |_ext, req, _cx| {
                let text = req
                    .pointer("/data/data/text")
                    .or_else(|| req.pointer("/data/text"))?;
                cleared::render(text.as_str()?)
            },
        );
        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.settings.refresh();
            ext.editors.remove(cx.session_id());
            unipi_kit::bus::emit(&ModuleReady {
                name: unipi_kit::suite::pkg::INPUT_SHORTCUTS.npm.to_owned(),
                version: unipi_kit::suite::UNIPI_VERSION.to_owned(),
                commands: Vec::new(),
                tools: Vec::new(),
                load_time_ms: None,
            });
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            let sid = cx.session_id();
            timers::cancel(&timer_id("burst", sid));
            timers::cancel(&timer_id("status", sid));
            ext.editors.remove(sid);
            Ok(())
        });
        cx.on(EventKind::Input, |ext, _ev, cx| async move {
            ext.editors.with(cx.session_id(), |e| e.after_submit = true);
            Ok(())
        });
        cx.on(EventKind::OinoEditor, |ext, ev, cx| async move {
            ext.on_editor(&ev, &cx);
            Ok(())
        });
        cx.on(EventKind::OinoTimer, |ext, ev, cx| async move {
            ext.on_timer(&ev, &cx);
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            ext.settings.on_changed(&ev.payload);
            Ok(())
        });
        cx.service(
            unipi_kit::services::fanout::INFO_SCREEN_GROUPS,
            |ext, _payload, _caller, _cx| async move { Ok(ext.info_group()) },
        );
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(InputShortcuts);
