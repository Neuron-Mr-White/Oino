//! `Menu` (design.md §4.4): dropdowns, context menus and select popups. The owner keeps the
//! open state; [`popover`] anchors the shell to a trigger and closes on outside clicks.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, ClickEvent, Div, ElementId, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseDownEvent, ParentElement as _, RenderOnce, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, anchored, deferred, div, point, px,
};

use crate::components::badge::kbd;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};

/// Activation callback.
pub type OnSelect = Rc<dyn Fn(&mut Window, &mut App)>;

/// One menu entry.
pub enum MenuEntry {
    /// An action row.
    Item {
        /// Label.
        label: SharedString,
        /// Leading icon.
        icon: Option<IconName>,
        /// Detail under / after the label.
        detail: Option<SharedString>,
        /// Trailing key hint.
        keys: Option<SharedString>,
        /// Checked (trailing `check`).
        checked: bool,
        /// Disabled.
        disabled: bool,
        /// Opens a submenu (trailing chevron).
        submenu: bool,
        /// Keyboard-active.
        active: bool,
        /// Action.
        on_select: OnSelect,
    },
    /// A group label with an optional badge.
    Label(SharedString, Option<SharedString>),
    /// A separator.
    Separator,
}

impl MenuEntry {
    /// An action row.
    pub fn item(
        label: impl Into<SharedString>,
        on_select: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self::Item {
            label: label.into(),
            icon: None,
            detail: None,
            keys: None,
            checked: false,
            disabled: false,
            submenu: false,
            active: false,
            on_select: Rc::new(on_select),
        }
    }

    /// Sets the icon.
    #[must_use]
    pub fn icon(mut self, name: IconName) -> Self {
        if let Self::Item { icon, .. } = &mut self {
            *icon = Some(name);
        }
        self
    }

    /// Sets the detail.
    #[must_use]
    pub fn detail(mut self, text: impl Into<SharedString>) -> Self {
        if let Self::Item { detail, .. } = &mut self {
            *detail = Some(text.into());
        }
        self
    }

    /// Sets the key hint.
    #[must_use]
    pub fn keys(mut self, text: impl Into<SharedString>) -> Self {
        if let Self::Item { keys, .. } = &mut self {
            *keys = Some(text.into());
        }
        self
    }

    /// Sets the check mark.
    #[must_use]
    pub fn checked(mut self, on: bool) -> Self {
        if let Self::Item { checked, .. } = &mut self {
            *checked = on;
        }
        self
    }

    /// Disables the row.
    #[must_use]
    pub fn disabled(mut self, on: bool) -> Self {
        if let Self::Item { disabled, .. } = &mut self {
            *disabled = on;
        }
        self
    }

    /// Marks a submenu row.
    #[must_use]
    pub fn submenu(mut self) -> Self {
        if let Self::Item { submenu, .. } = &mut self {
            *submenu = true;
        }
        self
    }

    /// Keyboard-active row.
    #[must_use]
    pub fn active(mut self, on: bool) -> Self {
        if let Self::Item { active, .. } = &mut self {
            *active = on;
        }
        self
    }
}

/// The menu shell (A2.12 anchored menu): `menu` (surface-2), radius 16, padding 6, menu
/// elevation.
#[must_use]
pub fn shell(t: &OinoTheme, width: f32) -> Div {
    div()
        .flex()
        .flex_col()
        .w(px(width))
        .p(px(space::S1_5))
        .gap(px(space::HALF))
        .rounded(px(radius::R_FLOAT))
        .bg(t.menu())
        .border_1()
        .border_color(t.border())
        .shadow(t.e_menu())
        .text_color(t.fg())
        .font_family(t.ui_font())
}

/// Renders entries into a shell of `width`: a keyboard menu (design.md §4.4 keys) that takes
/// focus when it opens. ↑/↓ move (wrapping, skipping disabled rows), Enter/Space choose,
/// → opens a submenu row, type-ahead jumps to the next label starting with the typed text,
/// and Esc (handled by [`popover`]) closes and returns focus to where it was.
pub fn menu(
    id: impl Into<SharedString>,
    t: &OinoTheme,
    width: f32,
    entries: Vec<MenuEntry>,
) -> KeyMenu {
    KeyMenu {
        id: id.into(),
        t: *t,
        width,
        entries,
        max_h: None,
    }
}

/// A keyboard-navigable menu (see [`menu`]).
#[derive(IntoElement)]
pub struct KeyMenu {
    id: SharedString,
    t: OinoTheme,
    width: f32,
    entries: Vec<MenuEntry>,
    max_h: Option<f32>,
}

impl KeyMenu {
    /// Caps the height; the rows scroll beyond it.
    #[must_use]
    pub fn max_h(mut self, h: f32) -> Self {
        self.max_h = Some(h);
        self
    }
}

/// Per-menu keyboard state, kept while the menu stays rendered.
struct MenuState {
    focus: FocusHandle,
    previous: Option<FocusHandle>,
    active: Option<usize>,
    focused: bool,
    typed: String,
    typed_at: Option<std::time::Instant>,
}

/// Next selectable index from `from` in direction `dir` (wrapping).
#[must_use]
pub fn step_enabled(enabled: &[bool], from: Option<usize>, dir: isize) -> Option<usize> {
    let n = enabled.len();
    if n == 0 || !enabled.iter().any(|e| *e) {
        return None;
    }
    #[allow(
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        reason = "menu sizes are small"
    )]
    let mut i = from.map_or(if dir > 0 { -1 } else { n as isize }, |f| f as isize);
    loop {
        #[allow(
            clippy::cast_possible_wrap,
            clippy::cast_sign_loss,
            reason = "menu sizes are small"
        )]
        {
            i = (i + dir).rem_euclid(n as isize);
            if enabled[i as usize] {
                return Some(i as usize);
            }
        }
    }
}

impl RenderOnce for KeyMenu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(
            ElementId::Name(format!("{}-keys", self.id).into()),
            cx,
            |window, cx| MenuState {
                focus: cx.focus_handle(),
                previous: window.focused(cx),
                active: None,
                focused: false,
                typed: String::new(),
                typed_at: None,
            },
        );
        let (focus, active, first) = {
            let s = state.read(cx);
            (s.focus.clone(), s.active, !s.focused)
        };
        if first {
            window.focus(&focus, cx);
            state.update(cx, |s, _| s.focused = true);
        }
        // What the key handler needs from each row.
        let rows: Vec<(bool, String, bool, Option<OnSelect>)> = self
            .entries
            .iter()
            .map(|e| match e {
                MenuEntry::Item {
                    label,
                    disabled,
                    submenu,
                    on_select,
                    ..
                } => (
                    !*disabled,
                    label.to_lowercase(),
                    *submenu,
                    Some(Rc::clone(on_select)),
                ),
                _ => (false, String::new(), false, None),
            })
            .collect();
        let enabled: Vec<bool> = rows.iter().map(|r| r.0).collect();
        let t = self.t;
        let id = self.id.clone();
        let key_state = state.clone();
        let enter_id = ElementId::Name(format!("{}-enter", self.id).into());
        let el = shell(&t, self.width)
            .id(ElementId::Name(format!("{}-menu", self.id).into()))
            .when_some(self.max_h, |el, h| el.max_h(px(h)).overflow_y_scroll())
            .track_focus(&focus)
            .on_key_down(move |e: &KeyDownEvent, window, cx| {
                let key = e.keystroke.key.as_str();
                let current = key_state.read(cx).active;
                let choose = |i: usize, window: &mut Window, cx: &mut App| {
                    let prev = key_state.read(cx).previous.clone();
                    if let Some(prev) = prev {
                        window.focus(&prev, cx);
                    }
                    if let Some(f) = rows.get(i).and_then(|r| r.3.clone()) {
                        f(window, cx);
                    }
                };
                let next = match key {
                    "down" => step_enabled(&enabled, current, 1),
                    "up" => step_enabled(&enabled, current, -1),
                    "home" => step_enabled(&enabled, None, 1),
                    "end" => step_enabled(&enabled, None, -1),
                    "enter" | "space" => {
                        if let Some(i) = current {
                            choose(i, window, cx);
                        }
                        cx.stop_propagation();
                        return;
                    }
                    "right" => {
                        if let Some(i) = current.filter(|i| rows.get(*i).is_some_and(|r| r.2)) {
                            choose(i, window, cx);
                        }
                        cx.stop_propagation();
                        return;
                    }
                    _ => {
                        let ch = e.keystroke.key_char.clone().unwrap_or_default();
                        if ch.chars().count() != 1
                            || e.keystroke.modifiers.control
                            || e.keystroke.modifiers.platform
                            || e.keystroke.modifiers.alt
                        {
                            return;
                        }
                        let typed = key_state.update(cx, |s, _| {
                            let fresh = s.typed_at.is_none_or(|at| {
                                at.elapsed() > std::time::Duration::from_millis(800)
                            });
                            if fresh {
                                s.typed.clear();
                            }
                            s.typed.push_str(&ch.to_lowercase());
                            s.typed_at = Some(std::time::Instant::now());
                            s.typed.clone()
                        });
                        let n = rows.len();
                        let start = current
                            .map_or(0, |c| if typed.chars().count() == 1 { c + 1 } else { c });
                        (0..n)
                            .map(|k| (start + k) % n.max(1))
                            .find(|i| rows[*i].0 && rows[*i].1.starts_with(&typed))
                    }
                };
                if let Some(i) = next {
                    key_state.update(cx, |s, cx| {
                        s.active = Some(i);
                        cx.notify();
                    });
                }
                cx.stop_propagation();
            })
            .children(self.entries.into_iter().enumerate().map(|(i, mut e)| {
                if let (MenuEntry::Item { active: a, .. }, Some(k)) = (&mut e, active) {
                    *a = i == k;
                }
                row(&id, i, &t, e)
            }));
        crate::design::motion::enter(&t, enter_id, el)
    }
}

fn row(id: &SharedString, i: usize, t: &OinoTheme, entry: MenuEntry) -> AnyElement {
    match entry {
        MenuEntry::Separator => div()
            .my(px(space::S1))
            .h(px(size::HAIRLINE))
            .bg(t.border())
            .into_any_element(),
        MenuEntry::Label(text, badge) => div()
            .flex()
            .items_center()
            .gap(px(space::S2))
            .h(px(size::CONTROL))
            .px(px(space::S2_5))
            .child(crate::components::section_label::label(t, text))
            .when_some(badge, |el, b| {
                el.child(super::badge::pill(t, t.tag(), t.fg_subtle(), b))
            })
            .into_any_element(),
        MenuEntry::Item {
            label,
            icon: lead,
            detail,
            keys,
            checked,
            disabled,
            submenu,
            active,
            on_select,
        } => {
            let fg = if disabled { t.fg_subtlest() } else { t.fg() };
            div()
                .id(ElementId::Name(format!("{id}-{i}").into()))
                .flex()
                .items_center()
                .gap(px(space::S2))
                .min_h(px(size::MENU_ROW))
                .px(px(space::S2_5))
                .rounded(px(radius::R_ROW))
                .text_color(fg)
                .text_role(t, TextRole::Base)
                .when(active, |el| el.bg(t.menu_hover()))
                .when(!disabled, |el| {
                    el.cursor_pointer()
                        .hover(|s| s.bg(t.menu_hover()))
                        .on_click(move |_: &ClickEvent, w, cx| on_select(w, cx))
                })
                .when_some(lead, |el, i| el.child(icon(i, size::ICON, t.fg_subtle())))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .py(px(space::S1))
                        .child(div().truncate().child(label))
                        .when_some(detail, |el, d| {
                            el.child(
                                div()
                                    .truncate()
                                    .text_color(t.fg_subtle())
                                    .text_role(t, TextRole::Sm)
                                    .child(d),
                            )
                        }),
                )
                .when_some(keys, |el, k| el.child(kbd(t, k)))
                .when(checked, |el| {
                    el.child(icon(IconName::Check, size::ICON, t.fg()))
                })
                .when(submenu, |el| {
                    el.child(icon(IconName::ChevronRight, size::ICON_MD, t.fg_subtle()))
                })
                .into_any_element()
        }
    }
}

/// Where a popover opens relative to its trigger corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Below the trigger, leading edges aligned.
    BelowStart,
    /// Below the trigger, trailing edges aligned.
    BelowEnd,
    /// Above the trigger, leading edges aligned.
    AboveStart,
    /// Above the trigger, trailing edges aligned.
    AboveEnd,
}

/// Wraps `trigger` and, when `open`, a deferred anchored `content` that closes on an outside
/// mouse-down (`on_close`).
pub fn popover(
    trigger: impl IntoElement,
    open: bool,
    placement: Placement,
    content: impl FnOnce() -> AnyElement,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    let wrapper = div().relative().flex_shrink_0().child(trigger);
    if !open {
        return wrapper;
    }
    let (anchor, corner) = match placement {
        Placement::BelowStart => (Anchor::TopLeft, div().absolute().left_0().bottom_0()),
        Placement::BelowEnd => (Anchor::TopRight, div().absolute().right_0().bottom_0()),
        Placement::AboveStart => (Anchor::BottomLeft, div().absolute().left_0().top_0()),
        Placement::AboveEnd => (Anchor::BottomRight, div().absolute().right_0().top_0()),
    };
    let gap = match placement {
        Placement::BelowStart | Placement::BelowEnd => space::S1,
        Placement::AboveStart | Placement::AboveEnd => -space::S1,
    };
    let on_close = Rc::new(on_close);
    let on_escape = Rc::clone(&on_close);
    let body = div()
        .occlude()
        .on_mouse_down_out(move |_: &MouseDownEvent, w, cx| on_close(w, cx))
        .on_key_down(move |e: &KeyDownEvent, w, cx| {
            if e.keystroke.key == "escape" || e.keystroke.key == "left" {
                cx.stop_propagation();
                on_escape(w, cx);
            }
        })
        .child(content());
    wrapper.child(
        corner.child(deferred(
            anchored()
                .anchor(anchor)
                .offset(point(px(space::S0), px(gap)))
                .snap_to_window_with_margin(px(space::S2))
                .child(body),
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::step_enabled;

    #[test]
    fn steps_wrap_and_skip_disabled() {
        let e = [true, false, true];
        assert_eq!(step_enabled(&e, None, 1), Some(0));
        assert_eq!(step_enabled(&e, Some(0), 1), Some(2));
        assert_eq!(step_enabled(&e, Some(2), 1), Some(0));
        assert_eq!(step_enabled(&e, Some(0), -1), Some(2));
        assert_eq!(step_enabled(&[false], None, 1), None);
    }
}
