//! The opt-in desktop **mascot window** (design.md A1.2, unipi-app DESIGN §1.5.1): a
//! transparent, frameless, always-on-top window at the bottom-right of the primary display
//! with only the animated Unicrab (aggregate state of every session) and a fade bubble for
//! the newest needs-you / done message. Click the crab → Oino in front; drag moves it.
//! Settings → Appearance → "Mascot window" turns it on; crab size 72 / 96 / 128.

use std::time::{Duration, Instant};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind,
    WindowOptions, div, point, px, size as gsize,
};

use crate::components::unicrab::unicrab;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::views::shell::Workspace;

/// How long a bubble stays up.
pub const BUBBLE: Duration = Duration::from_secs(6);
/// Width reserved for the bubble next to the crab.
const BUBBLE_W: f32 = size::TOAST * 0.75;

/// The mascot window's root view.
pub struct MascotView {
    workspace: WeakEntity<Workspace>,
}

impl MascotView {
    /// Opens the mascot window for `workspace`.
    pub fn open(
        workspace: WeakEntity<Workspace>,
        crab_px: f32,
        cx: &mut App,
    ) -> Option<WindowHandle<Self>> {
        let w = crab_px + BUBBLE_W + space::S4;
        let h = crab_px * 1.3 + space::S8;
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
        cx.open_window(options, |_, cx| cx.new(|_| Self { workspace }))
            .map_err(|error| tracing::warn!(%error, "opening the mascot window"))
            .ok()
    }
}

impl Render for MascotView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = OinoTheme::get(cx);
        let (state, bubble, crab_px) = self
            .workspace
            .upgrade()
            .map(|ws| {
                let ws = ws.read(cx);
                let bubble = ws
                    .mascot_bubble
                    .as_ref()
                    .filter(|(_, at)| at.elapsed() < BUBBLE)
                    .map(|(text, _)| text.clone());
                (ws.crab(cx), bubble, f32::from(ws.settings.mascot_size))
            })
            .unwrap_or_default();
        if bubble.is_some() {
            // Fade-out check while a bubble is up.
            window.request_animation_frame();
        }
        let ws = self.workspace.clone();
        div()
            .id("mascot")
            .size_full()
            .flex()
            .flex_row()
            .items_end()
            .justify_end()
            .gap(px(space::S2))
            .p(px(space::S2))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .when_some(bubble, |el, text| {
                el.child(
                    div()
                        .max_w(px(BUBBLE_W))
                        .mb(px(crab_px * 0.6))
                        .px(px(space::S3))
                        .py(px(space::S2))
                        .rounded(px(radius::R_CARD))
                        .bg(t.popover())
                        .border_1()
                        .border_color(t.popover_border())
                        .shadow(t.e2())
                        .text_role(&t, TextRole::Sm)
                        .text_color(t.fg())
                        .font_family(t.ui_font())
                        .child(SharedString::from(text)),
                )
            })
            .child(
                div()
                    .id("mascot-crab")
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        cx.activate(true);
                        let _ = ws.update(cx, |ws, cx| {
                            ws.mascot_bubble = None;
                            cx.notify();
                        });
                    })
                    .child(crate::design::ticker::Ticker::scoped(
                        cx.entity_id(),
                        || unicrab(state, crab_px.max(size::CRAB.1), cx),
                    )),
            )
    }
}

impl Workspace {
    /// Opens or closes the mascot window to match the setting.
    pub(crate) fn sync_mascot(&mut self, cx: &mut Context<Self>) {
        let want = self.settings.mascot_window;
        match (&self.mascot, want) {
            (None, true) => {
                let px_size = f32::from(self.settings.mascot_size).clamp(72.0, 128.0);
                self.mascot = MascotView::open(cx.entity().downgrade(), px_size, cx);
            }
            (Some(handle), false) => {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
                self.mascot = None;
            }
            _ => {
                if let Some(handle) = &self.mascot {
                    let _ = handle.update(cx, |_, _, cx| cx.notify());
                }
            }
        }
    }

    /// Shows `text` in the mascot bubble (newest needs-you / done message).
    pub(crate) fn mascot_say(&mut self, text: String, cx: &mut Context<Self>) {
        self.mascot_bubble = Some((text, Instant::now()));
        self.sync_mascot(cx);
    }
}
