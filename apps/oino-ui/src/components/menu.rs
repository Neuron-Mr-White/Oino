//! `Menu` (design.md §4.4): dropdowns, context menus and select popups. The owner keeps the
//! open state; [`popover`] anchors the shell to a trigger and closes on outside clicks.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, ClickEvent, Div, ElementId, InteractiveElement as _, IntoElement,
    MouseDownEvent, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, anchored, deferred, div, point, px,
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

/// The menu shell: `menu` bg, 1 px border, `R_LG`, padding 4, E2.
#[must_use]
pub fn shell(t: &OinoTheme, width: f32) -> Div {
    div()
        .flex()
        .flex_col()
        .w(px(width))
        .p(px(space::S1))
        .gap(px(space::HALF))
        .rounded(px(radius::R_LG))
        .bg(t.menu())
        .border_1()
        .border_color(t.popover_border())
        .shadow(t.e2())
        .text_color(t.fg())
        .font_family(t.ui_font())
}

/// Renders entries into a shell of `width`.
pub fn menu(
    id: impl Into<SharedString>,
    t: &OinoTheme,
    width: f32,
    entries: Vec<MenuEntry>,
) -> Div {
    let id: SharedString = id.into();
    shell(t, width).children(
        entries
            .into_iter()
            .enumerate()
            .map(|(i, e)| row(&id, i, t, e)),
    )
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
            .px(px(space::S2))
            .text_color(t.fg_subtle())
            .text_role(t, TextRole::Sm)
            .child(text)
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
                .min_h(px(size::CONTROL_LG))
                .px(px(space::S2))
                .rounded(px(radius::R_MD))
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
    let body = div()
        .occlude()
        .on_mouse_down_out(move |_: &MouseDownEvent, w, cx| on_close(w, cx))
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
