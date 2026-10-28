//! Frame tab strips (design.md A2.20 P1 / P5): side-pane and bottom-frame tabs as chip pills
//! (surface-2, active `chip_active` + its border, close ✕ on hover or when active), on a flush
//! strip with a `line_soft` bottom rule. The side pane shows the strip only when more than
//! one tab is open (its `DetailHeader`-style header names the tab); the bottom frame always
//! shows it (terminal tabs, not sessions).

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, ElementId, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};

use crate::automation::Automatable as _;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};

/// One tab.
#[derive(Clone, Debug)]
pub struct TabSpec {
    /// Stable key.
    pub key: SharedString,
    /// Icon.
    pub icon: IconName,
    /// Title.
    pub title: SharedString,
    /// Shows a close button.
    pub closable: bool,
}

/// Tab callback with the tab index.
pub type OnTab = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// Strip density.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripSize {
    /// Side pane: 38 px chip pills (A2.15 `CHIP`).
    Chip,
    /// Bottom frame: a 38 px strip with 30 px pills.
    Compact,
}

impl StripSize {
    fn pill(self) -> f32 {
        match self {
            Self::Chip => size::CHIP,
            Self::Compact => size::MODE_CHIP,
        }
    }

    fn strip(self) -> f32 {
        match self {
            Self::Chip => size::CHIP + space::S3,
            Self::Compact => size::CHIP,
        }
    }
}

/// The pill strip; `trailing` holds the launcher / frame buttons.
pub fn frame_tab_bar(
    id: &str,
    t: &OinoTheme,
    tabs: &[TabSpec],
    active: usize,
    on_select: OnTab,
    on_close: OnTab,
    trailing: Vec<AnyElement>,
) -> Div {
    tab_strip(
        id,
        t,
        StripSize::Chip,
        tabs,
        active,
        on_select,
        on_close,
        trailing,
    )
}

/// The pill strip at `density`.
#[allow(clippy::too_many_arguments, reason = "strip parts")]
pub fn tab_strip(
    id: &str,
    t: &OinoTheme,
    density: StripSize,
    tabs: &[TabSpec],
    active: usize,
    on_select: OnTab,
    on_close: OnTab,
    trailing: Vec<AnyElement>,
) -> Div {
    let pill = density.pill();
    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_shrink_0()
        .h(px(density.strip()))
        .px(px(space::S2))
        .gap(px(space::S1))
        .border_b_1()
        .border_color(t.line_soft())
        .child(
            div()
                .id(ElementId::Name(format!("{id}-strip").into()))
                .flex()
                .flex_row()
                .items_center()
                .flex_1()
                .min_w_0()
                .gap(px(space::S1_5))
                .overflow_x_scroll()
                .children(tabs.iter().enumerate().map(|(i, tab)| {
                    pill_tab(id, t, tab, i, i == active, pill, &on_select, &on_close)
                })),
        )
        .children(trailing)
}

#[allow(clippy::too_many_arguments, reason = "pill parts")]
fn pill_tab(
    id: &str,
    t: &OinoTheme,
    tab: &TabSpec,
    i: usize,
    selected: bool,
    pill: f32,
    on_select: &OnTab,
    on_close: &OnTab,
) -> AnyElement {
    let select = Rc::clone(on_select);
    let close = Rc::clone(on_close);
    let fg = if selected { t.fg() } else { t.fg_subtle() };
    let group = SharedString::from(format!("{id}-g-{}", tab.key));
    let meta =
        crate::automation::Meta::labelled(crate::automation::Role::Tab, tab.title.to_string())
            .selected(selected);
    div()
        .id(ElementId::Name(format!("{id}-tab-{}", tab.key).into()))
        .group(group.clone())
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(px(space::S1_5))
        .h(px(pill))
        .pl(px(space::S3))
        .pr(px(if tab.closable { space::S1_5 } else { space::S3 }))
        .rounded(px(radius::R_FULL))
        .cursor_pointer()
        .text_color(fg)
        .text_role(t, TextRole::Sm)
        .weight_medium()
        .bg(if selected { t.chip_active() } else { t.chip() })
        .border_1()
        .border_color(if selected {
            t.chip_active_border()
        } else {
            t.chip()
        })
        .when(!selected, |el| {
            el.hover(|s| s.bg(t.bubble()).text_color(t.fg()))
        })
        .on_click(move |_: &ClickEvent, w, cx| select(i, w, cx))
        .child(icon(tab.icon, size::ICON_MD, fg))
        .child(
            div()
                .truncate()
                .max_w(px(size::SIDEBAR * 0.75))
                .child(tab.title.clone()),
        )
        .when(tab.closable, |el| {
            el.child(
                div()
                    .id(ElementId::Name(format!("{id}-close-{}", tab.key).into()))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(size::CONTROL_XS))
                    .rounded(px(radius::R_FULL))
                    .when(!selected, |el| {
                        el.invisible().group_hover(group, gpui_kit::Styled::visible)
                    })
                    .hover(|s| s.bg(t.bubble()))
                    .on_click(move |_: &ClickEvent, w, cx| {
                        cx.stop_propagation();
                        close(i, w, cx);
                    })
                    .child(icon(IconName::X, size::ICON_SM, t.fg_subtle())),
            )
        })
        .automation(meta)
        .into_any_element()
}

/// An underline tab strip for OUS `tabs` when they do not fit a segmented control.
pub fn underline_tabs(
    id: &str,
    t: &OinoTheme,
    titles: &[String],
    active: usize,
    on_select: OnTab,
) -> Div {
    div()
        .flex()
        .flex_row()
        .gap(px(space::S4))
        .border_b_1()
        .border_color(t.line_soft())
        .children(titles.iter().enumerate().map(|(i, title)| {
            let select = Rc::clone(&on_select);
            let selected = i == active;
            div()
                .id(ElementId::Name(format!("{id}-u-{i}").into()))
                .py(px(space::S1_5))
                .cursor_pointer()
                .text_role(t, TextRole::Sm)
                .weight_medium()
                .text_color(if selected { t.fg() } else { t.fg_subtle() })
                .when(selected, |el| el.border_b_2().border_color(t.fg()))
                .on_click(move |_: &ClickEvent, w, cx| select(i, w, cx))
                .child(SharedString::from(title.clone()))
                .automation(
                    crate::automation::Meta::labelled(crate::automation::Role::Tab, title.clone())
                        .selected(selected),
                )
        }))
}
