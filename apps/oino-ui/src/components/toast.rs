//! `Toast` (design.md A2.12): bottom-centre stack of at most 3, max 420 wide, `sheet` fill,
//! radius 16, toast elevation, padding 10 14; icon in tone, message up to 3 lines with
//! "More" (opens the full text as a result card, GA-24), at most one action pill.
//! Auto-dismiss 4 s info / 6 s warning, sticky errors; hover pauses (the owner skips expiry
//! while hovered). Messages longer than [`is_long`] skip the toast and open the result card
//! directly. A dismissed toast fades out (`design::motion::exit`, `EXIT_MS`) instead of
//! vanishing: [`Toasts::dismiss`] only marks it, [`Toasts::reap`] drops it once the exit is
//! over (the owner's timer does both).

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, Div, ElementId, InteractiveElement as _, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::automation::Automatable as _;
use crate::components::button::{Btn, BtnSize, Variant};
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::tokens::motion::EXIT_MS as EXIT;
use crate::design::{TextRole, radius, size, space};

/// Toast severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    /// Info (4 s).
    Info,
    /// Success (4 s).
    Success,
    /// Warning (6 s).
    Warning,
    /// Error (sticky).
    Error,
}

impl ToastKind {
    /// Auto-dismiss delay (`None` = sticky).
    #[must_use]
    pub const fn ttl(self) -> Option<Duration> {
        match self {
            Self::Info | Self::Success => Some(Duration::from_secs(4)),
            Self::Warning => Some(Duration::from_secs(6)),
            Self::Error => None,
        }
    }
}

/// One toast.
#[derive(Clone)]
pub struct Toast {
    /// Id.
    pub id: u64,
    /// Severity.
    pub kind: ToastKind,
    /// Bold title (OS notifications shown as toasts).
    pub title: Option<SharedString>,
    /// Message.
    pub message: SharedString,
    /// Created.
    pub at: Instant,
    /// Optional action `(label, callback)`.
    pub action: Option<(SharedString, Rc<dyn Fn(&mut Window, &mut App)>)>,
    /// Set when the toast is on its way out: it fades for [`EXIT`](crate::design::tokens::motion::EXIT_MS)
    /// and is dropped by [`Toasts::reap`].
    pub closing: Option<Instant>,
}

/// The toast stack state.
#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next: u64,
    /// Hovered toast (pauses expiry).
    pub hovered: Option<u64>,
}

impl Toasts {
    /// Pushes a toast; returns its id.
    pub fn push(
        &mut self,
        kind: ToastKind,
        title: Option<SharedString>,
        message: impl Into<SharedString>,
    ) -> u64 {
        self.next += 1;
        self.items.push(Toast {
            id: self.next,
            kind,
            title,
            message: message.into(),
            at: Instant::now(),
            action: None,
            closing: None,
        });
        self.next
    }

    /// Starts the exit of a toast (it stays rendered, fading, until [`Self::reap`]).
    pub fn dismiss(&mut self, id: u64) {
        let now = Instant::now();
        for t in &mut self.items {
            if t.id == id && t.closing.is_none() {
                t.closing = Some(now);
            }
        }
    }

    /// Removes the toasts whose exit finished; returns whether anything changed.
    pub fn reap(&mut self, now: Instant) -> bool {
        let before = self.items.len();
        self.items.retain(|t| {
            t.closing
                .is_none_or(|at| now.duration_since(at) < Duration::from_millis(EXIT))
        });
        before != self.items.len()
    }

    /// Starts the exit of every expired toast; returns whether anything changed.
    pub fn expire(&mut self, now: Instant) -> bool {
        let mut changed = false;
        let hovered = self.hovered;
        for t in &mut self.items {
            if t.closing.is_some() || Some(t.id) == hovered {
                continue;
            }
            if t.kind
                .ttl()
                .is_some_and(|ttl| now.duration_since(t.at) >= ttl)
            {
                t.closing = Some(now);
                changed = true;
            }
        }
        changed
    }

    /// Whether the expiry timer has work left: a timed toast is up or one is fading out.
    #[must_use]
    pub fn has_timed(&self) -> bool {
        self.items
            .iter()
            .any(|t| t.closing.is_some() || t.kind.ttl().is_some())
    }

    /// The toast `id`.
    #[must_use]
    pub fn get(&self, id: u64) -> Option<&Toast> {
        self.items.iter().find(|t| t.id == id)
    }

    /// Visible toasts (newest last, at most 3).
    #[must_use]
    pub fn visible(&self) -> &[Toast] {
        let n = self.items.len();
        &self.items[n.saturating_sub(3)..]
    }
}

/// Lines a toast shows before "More".
pub const TOAST_LINES: usize = 3;

/// A notify text that does not fit a toast (multi-line status reports, GA-24): more than
/// [`TOAST_LINES`] lines or about three lines of characters.
#[must_use]
pub fn is_long(text: &str) -> bool {
    text.lines().count() > TOAST_LINES || text.chars().count() > 180
}

/// Renders the stack; `on_dismiss(id)` closes one, `on_more(id)` opens its full text.
pub fn stack(
    t: &OinoTheme,
    toasts: &Toasts,
    on_dismiss: Rc<dyn Fn(u64, &mut Window, &mut App)>,
    on_hover: Rc<dyn Fn(Option<u64>, &mut Window, &mut App)>,
    on_more: Rc<dyn Fn(u64, &mut Window, &mut App)>,
) -> Div {
    div()
        .absolute()
        .bottom(px(space::S6))
        .left_0()
        .right_0()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(space::S2))
        .children(toasts.visible().iter().map(|toast| {
            let (glyph, c) = match toast.kind {
                ToastKind::Info => (IconName::Info, t.fg_subtle()),
                ToastKind::Success => (IconName::Check, t.success()),
                ToastKind::Warning => (IconName::TriangleAlert, t.warning()),
                ToastKind::Error => (IconName::CircleX, t.danger()),
            };
            let id = toast.id;
            let dismiss = Rc::clone(&on_dismiss);
            let hover = Rc::clone(&on_hover);
            let more = Rc::clone(&on_more);
            let clipped =
                toast.message.lines().count() > TOAST_LINES || toast.message.chars().count() > 120;
            // A toast is how extension feedback becomes visible, so the driver can assert on
            // it (docs/dev/v1/gui-driver.md).
            let meta = crate::automation::Meta::labelled(
                crate::automation::Role::Text,
                toast.message.clone(),
            )
            .value(toast.title.clone().unwrap_or_default());
            let card = div()
                        .id(ElementId::Name(format!("toast-{id}").into()))
                        .max_w(px(size::TOAST_MAX))
                        .min_w(px(size::TOAST * 0.6))
                        .flex()
                        .items_start()
                        .gap(px(space::S2_5))
                        .py(px(space::S2_5))
                        .px(px(space::S3_5))
                        .rounded(px(radius::R_FLOAT))
                        .bg(t.sheet())
                        .border_1()
                        .border_color(t.border())
                        .shadow(t.e_toast())
                        .on_hover(move |h: &bool, w, cx| hover(h.then_some(id), w, cx))
                        .child(div().pt(px(space::HALF)).child(icon(glyph, size::ICON, c)))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .gap(px(space::S1))
                                .text_color(t.fg())
                                .text_role(t, TextRole::Base)
                                .when_some(toast.title.clone(), |el, title| {
                                    el.child(div().weight_medium().child(title))
                                })
                                .child(
                                    div()
                                        .line_clamp(TOAST_LINES)
                                        .text_color(t.fg_subtle())
                                        .when(toast.title.is_none(), |el| el.text_color(t.fg()))
                                        .child(toast.message.clone()),
                                )
                                .when(clipped || toast.action.is_some(), |el| {
                                    el.child(
                                        div()
                                            .flex()
                                            .gap(px(space::S1_5))
                                            .when(clipped, |el| {
                                                el.child(
                                                    Btn::new(ElementId::Name(
                                                        format!("toast-more-{id}").into(),
                                                    ))
                                                    .label("More")
                                                    .variant(Variant::Secondary)
                                                    .size(BtnSize::Sm)
                                                    .on_click(move |_, w, cx| more(id, w, cx)),
                                                )
                                            })
                                            .when_some(toast.action.clone(), |el, (label, f)| {
                                                el.child(
                                                    Btn::new(ElementId::Name(
                                                        format!("toast-act-{id}").into(),
                                                    ))
                                                    .label(label)
                                                    .variant(Variant::Secondary)
                                                    .size(BtnSize::Sm)
                                                    .on_click(move |_, w, cx| f(w, cx)),
                                                )
                                            }),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .id(ElementId::Name(format!("toast-x-{id}").into()))
                                .cursor_pointer()
                                .rounded(px(radius::R_FULL))
                                .hover(|s| s.bg(t.hover()))
                                .on_click(move |_: &ClickEvent, w, cx| dismiss(id, w, cx))
                                .child(icon(IconName::X, size::ICON_MD, t.fg_subtle()))
                                .automation(crate::automation::Meta::icon_button("Dismiss toast")),
                        );
            // Leaving toasts fade + drop over `EXIT_MS`; the owner reaps them afterwards.
            let animated = if toast.closing.is_some() {
                crate::design::motion::exit(
                    t,
                    ElementId::Name(format!("toast-out-{id}").into()),
                    card,
                )
            } else {
                crate::design::motion::enter(
                    t,
                    ElementId::Name(format!("toast-in-{id}").into()),
                    card,
                )
            };
            crate::automation::registry::probe_named(animated, meta, format!("toast-{id}"))
        }))
}

#[cfg(test)]
mod tests {
    use super::{ToastKind, Toasts};
    use std::time::{Duration, Instant};

    #[test]
    fn an_expired_toast_fades_before_it_is_dropped() {
        let mut toasts = Toasts::default();
        let id = toasts.push(ToastKind::Info, None, "saved");
        assert!(!toasts.expire(Instant::now()));
        // Past the 4 s TTL the toast is marked closing but stays for the exit animation.
        let late = Instant::now() + Duration::from_secs(5);
        assert!(toasts.expire(late));
        assert_eq!(toasts.visible().len(), 1);
        assert!(toasts.get(id).is_some_and(|t| t.closing.is_some()));
        assert!(toasts.has_timed(), "the sweep must keep running while it fades");
        assert!(!toasts.reap(late));
        // …and is gone once the exit window is over.
        assert!(toasts.reap(late + Duration::from_millis(200)));
        assert!(toasts.visible().is_empty());
        assert!(!toasts.has_timed());
    }

    #[test]
    fn a_manual_dismiss_also_fades_and_a_hovered_toast_does_not_expire() {
        let mut toasts = Toasts::default();
        let id = toasts.push(ToastKind::Error, None, "failed");
        let late = Instant::now() + Duration::from_secs(60);
        // Errors are sticky.
        assert!(!toasts.expire(late));
        // Hover pauses a timed toast.
        let hovered = toasts.push(ToastKind::Info, None, "saved");
        toasts.hovered = Some(hovered);
        let _ = toasts.expire(late);
        assert!(toasts.get(hovered).is_some_and(|t| t.closing.is_none()));
        toasts.hovered = None;
        assert!(toasts.expire(late));
        assert!(toasts.get(hovered).is_some_and(|t| t.closing.is_some()));
        toasts.dismiss(id);
        assert!(toasts.get(id).is_some_and(|t| t.closing.is_some()));
        assert!(toasts.reap(late + Duration::from_millis(200)));
        assert!(toasts.visible().is_empty());
    }
}
