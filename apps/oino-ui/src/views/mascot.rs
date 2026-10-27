//! The opt-in desktop **mascot window** (design.md A2.13, unipi-app DESIGN §1.5.1): a
//! transparent, frameless, always-on-top window at the bottom-right of the primary display
//! with only the animated Unicrab (aggregate state of every session) and a fade bubble for
//! the newest needs-you / done message. Click the crab → Oino in front with the needs-you
//! drawer open; drag moves it. Settings → Unicrab → "Mascot window"; crab 72 / 96 / 128.
//!
//! The window renders a [`MascotSnapshot`] pushed by the shell. It never reads or leases the
//! shell entity while drawing (GA-32: opening it from inside the shell's update panicked with
//! a double lease and crash-looped every later launch).

use std::time::{Duration, Instant};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Bounds, Context, FontWeight, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, div, point, px, size as gsize,
};

use crate::components::unicrab::unicrab;
use crate::design::brand::CrabState;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::views::shell::Workspace;

/// How long a bubble stays up (fade in, hold, fade out).
pub const BUBBLE: Duration = Duration::from_secs(6);
/// Fade duration at each end of the bubble.
const FADE: Duration = Duration::from_millis(400);
/// Width reserved for the bubble next to the crab (A2.13: max 260).
const BUBBLE_W: f32 = 260.0;

/// What the mascot window shows; built by the shell, never read back from it.
#[derive(Clone, Debug, PartialEq)]
pub struct MascotSnapshot {
    /// Aggregate crab state.
    pub state: CrabState,
    /// Crab size in px (72 / 96 / 128).
    pub crab_px: f32,
    /// Bubble: (head = session title, text, shown at).
    pub bubble: Option<(String, String, Instant)>,
}

impl Default for MascotSnapshot {
    fn default() -> Self {
        Self {
            state: CrabState::Idle,
            crab_px: 96.0,
            bubble: None,
        }
    }
}

/// Splits a shell bubble line (`"<title> needs you: <what>"`, `"<title> finished"`) into
/// the bubble head (session title) and its text.
#[must_use]
pub fn split_bubble(text: &str) -> (String, String) {
    if let Some((title, what)) = text.split_once(" needs you: ") {
        let what = what.trim();
        return (
            title.to_owned(),
            if what.is_empty() {
                "Needs you".to_owned()
            } else {
                format!("Needs you: {what}")
            },
        );
    }
    if let Some(title) = text.strip_suffix(" finished") {
        return (title.to_owned(), "Finished. Have a look.".to_owned());
    }
    ("Oino".to_owned(), text.to_owned())
}

/// Bubble opacity at `elapsed` (fade in, hold, fade out; `None` once gone).
#[must_use]
pub fn bubble_opacity(elapsed: Duration) -> Option<f32> {
    if elapsed >= BUBBLE {
        return None;
    }
    let left = BUBBLE.saturating_sub(elapsed);
    let f = |d: Duration| d.as_secs_f32() / FADE.as_secs_f32();
    Some(if elapsed < FADE {
        f(elapsed)
    } else if left < FADE {
        f(left)
    } else {
        1.0
    })
}

/// The mascot window's root view.
pub struct MascotView {
    snap: MascotSnapshot,
    /// Click targets only (never read while rendering).
    workspace: WeakEntity<Workspace>,
}

impl MascotView {
    /// Opens the mascot window showing `snap`.
    pub fn open(
        workspace: WeakEntity<Workspace>,
        snap: MascotSnapshot,
        cx: &mut App,
    ) -> Option<WindowHandle<Self>> {
        let crab_px = snap.crab_px;
        let w = crab_px + BUBBLE_W + space::S4;
        let h = (crab_px * 1.3 + space::S8).max(size::CRAB.2 + space::S12);
        let display = cx.primary_display();
        let origin = display.map_or_else(
            || point(px(space::S8), px(space::S8)),
            |d| {
                let b = d.bounds();
                point(
                    b.origin.x + b.size.width - px(w + space::S6),
                    b.origin.y + b.size.height - px(h + space::S12),
                )
            },
        );
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                origin,
                gsize(px(w), px(h)),
            ))),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: true,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some("dev.oino.desktop.mascot".into()),
            ..WindowOptions::default()
        };
        cx.open_window(options, |_, cx| cx.new(|_| Self { snap, workspace }))
            .map_err(|error| tracing::warn!(%error, "opening the mascot window"))
            .ok()
    }

    /// Replaces the snapshot (redraws only on change).
    pub fn set(&mut self, snap: MascotSnapshot, cx: &mut Context<Self>) {
        if self.snap != snap {
            self.snap = snap;
            cx.notify();
        }
    }

    fn front(&self, cx: &mut App) {
        cx.activate(true);
        let ws = self.workspace.clone();
        // Outside any draw: the shell is free to update here.
        cx.defer(move |cx| {
            let _ = ws.update(cx, |ws, cx| {
                ws.mascot_bubble = None;
                ws.needs_drawer = true;
                cx.notify();
            });
        });
    }
}

impl Render for MascotView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = OinoTheme::get(cx);
        let snap = self.snap.clone();
        let bubble = snap.bubble.as_ref().and_then(|(head, text, at)| {
            bubble_opacity(at.elapsed()).map(|o| (head.clone(), text.clone(), o))
        });
        if bubble.is_some() {
            // Fades and the final hide.
            window.request_animation_frame();
        }
        let crab_px = snap.crab_px;
        div()
            .id("mascot")
            .size_full()
            .flex()
            .flex_row()
            .items_end()
            .justify_end()
            .gap(px(space::S2))
            .p(px(space::S2))
            .font_family(t.ui_font())
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .when_some(bubble, |el, (head, text, opacity)| {
                el.child(
                    div()
                        .id("mascot-bubble")
                        .flex()
                        .flex_col()
                        .gap(px(space::HALF))
                        .max_w(px(BUBBLE_W))
                        .mb(px(crab_px * 0.55))
                        .px(px(space::S3_5))
                        .py(px(space::S2_5))
                        .rounded(px(radius::R_FLOAT))
                        // The corner nearest the crab is tighter (A2.13).
                        .rounded_br(px(radius::R_KBD - 1.0))
                        .bg(t.input())
                        .border_1()
                        .border_color(t.border())
                        .shadow(t.e_toast())
                        .opacity(opacity)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.front(cx)))
                        .child(
                            div()
                                .truncate()
                                .text_role(&t, TextRole::Xs)
                                .font_weight(FontWeight::BOLD)
                                .text_color(t.working())
                                .child(SharedString::from(head)),
                        )
                        .child(
                            div()
                                .text_role(&t, TextRole::Base)
                                .text_color(t.fg())
                                .line_clamp(3)
                                .child(SharedString::from(text)),
                        ),
                )
            })
            .child(
                div()
                    .id("mascot-crab")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.front(cx)))
                    .child(crate::design::ticker::Ticker::scoped(
                        cx.entity_id(),
                        || unicrab(snap.state, crab_px.max(size::CRAB.1), cx),
                    )),
            )
    }
}

impl Workspace {
    /// The mascot's snapshot of this shell.
    fn mascot_snapshot(&self, cx: &App) -> MascotSnapshot {
        MascotSnapshot {
            state: self.crab(cx),
            crab_px: f32::from(self.settings.mascot_size).clamp(72.0, 128.0),
            bubble: self
                .mascot_bubble
                .as_ref()
                .filter(|(_, at)| at.elapsed() < BUBBLE)
                .map(|(text, at)| {
                    let (head, body) = split_bubble(text);
                    (head, body, *at)
                }),
        }
    }

    /// Opens, updates or closes the mascot window to match the setting. The window is opened
    /// in a deferred callback, outside this update (GA-32).
    pub(crate) fn sync_mascot(&mut self, cx: &mut Context<Self>) {
        let want = self.settings.mascot_window;
        let snap = self.mascot_snapshot(cx);
        // A size change reopens the window at the new size.
        let resized = self.mascot.as_ref().is_some_and(|h| {
            h.read_with(cx, |v, _| {
                (v.snap.crab_px - snap.crab_px).abs() > f32::EPSILON
            })
            .unwrap_or(true)
        });
        if resized && let Some(handle) = self.mascot.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        match (&self.mascot, want) {
            (None, true) => {
                let ws = cx.entity().downgrade();
                cx.defer(move |cx| {
                    let handle = MascotView::open(ws.clone(), snap, cx);
                    let _ = ws.update(cx, |ws, cx| {
                        match (handle, ws.settings.mascot_window, ws.mascot.is_some()) {
                            (Some(h), true, false) => ws.mascot = Some(h),
                            (Some(h), _, _) => {
                                let _ = h.update(cx, |_, window, _| window.remove_window());
                            }
                            (None, ..) => {}
                        }
                        cx.notify();
                    });
                });
            }
            (Some(handle), false) => {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
                self.mascot = None;
            }
            (Some(handle), true) => {
                let _ = handle.update(cx, |view, _, cx| view.set(snap, cx));
            }
            (None, false) => {}
        }
    }

    /// Shows `text` in the mascot bubble (newest needs-you / done message).
    pub(crate) fn mascot_say(&mut self, text: String, cx: &mut Context<Self>) {
        self.mascot_bubble = Some((text, Instant::now()));
        self.sync_mascot(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bubble_lines_split_into_head_and_text() {
        assert_eq!(
            split_bubble("Fix tests needs you: Approve bash"),
            ("Fix tests".into(), "Needs you: Approve bash".into())
        );
        assert_eq!(
            split_bubble("Fix tests finished"),
            ("Fix tests".into(), "Finished. Have a look.".into())
        );
        assert_eq!(split_bubble("hello").0, "Oino");
    }

    #[test]
    fn bubble_fades_in_holds_and_out() {
        assert_eq!(bubble_opacity(Duration::ZERO), Some(0.0));
        assert_eq!(bubble_opacity(Duration::from_secs(3)), Some(1.0));
        assert!(bubble_opacity(BUBBLE.saturating_sub(Duration::from_millis(100))).unwrap() < 0.5);
        assert_eq!(bubble_opacity(BUBBLE), None);
    }
}
