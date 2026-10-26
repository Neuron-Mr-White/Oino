//! `FrameTabBar` (design.md §4.5): side and bottom frame tabs (height 36, icon 14 + title,
//! close on hover/active, active `tab_active`).

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, ElementId, InteractiveElement as _, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

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

/// The tab bar; `trailing` holds the launcher / frame close buttons.
pub fn frame_tab_bar(
    id: &str,
    t: &OinoTheme,
    tabs: &[TabSpec],
    active: usize,
    on_select: OnTab,
    on_close: OnTab,
    trailing: Vec<AnyElement>,
) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_shrink_0()
        .h(px(size::TAB_BAR))
        .px(px(space::S1))
        .gap(px(space::HALF))
        .border_b_1()
        .border_color(t.border())
        .child(
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_w_0()
                .gap(px(space::HALF))
                .overflow_hidden()
                .children(tabs.iter().enumerate().map(|(i, tab)| {
                    let selected = i == active;
                    let select = Rc::clone(&on_select);
                    let close = Rc::clone(&on_close);
                    let fg = if selected { t.fg() } else { t.fg_subtle() };
                    div()
                        .id(ElementId::Name(format!("{id}-tab-{}", tab.key).into()))
                        .group(SharedString::from(format!("{id}-g-{}", tab.key)))
                        .flex()
                        .items_center()
                        .flex_shrink_0()
                        .min_w(px(size::CONTROL_LG * 2.0))
                        .gap(px(space::S1_5))
                        .h(px(size::CONTROL))
                        .px(px(space::S2))
                        .rounded(px(radius::R_MD))
                        .cursor_pointer()
                        .text_color(fg)
                        .text_role(t, TextRole::Sm)
                        .weight_medium()
                        .when(selected, |el| {
                            el.bg(t.tab_active()).border_1().border_color(t.border())
                        })
                        .when(!selected, |el| el.hover(|s| s.bg(t.hover())))
                        .on_click(move |_: &ClickEvent, w, cx| select(i, w, cx))
                        .child(icon(tab.icon, size::ICON_MD, fg))
                        .child(
                            div()
                                .truncate()
                                .max_w(px(size::SIDEBAR / 1.5))
                                .child(tab.title.clone()),
                        )
                        .when(tab.closable, |el| {
                            el.child(
                                div()
                                    .id(ElementId::Name(format!("{id}-close-{}", tab.key).into()))
                                    .rounded(px(radius::R_SM))
                                    .hover(|s| s.bg(t.hover()))
                                    .on_click(move |_: &ClickEvent, w, cx| {
                                        cx.stop_propagation();
                                        close(i, w, cx);
                                    })
                                    .child(icon(IconName::X, size::ICON_SM, t.fg_subtle())),
                            )
                        })
                })),
        )
        .children(trailing)
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
        .border_color(t.border())
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
        }))
}
