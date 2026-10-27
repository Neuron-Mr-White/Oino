//! `PageHeader` (design.md A2.6): one header for Sessions, Board, Extensions and Settings.
//! A 44 px lead / tools row, then the title (`Display` 30/36/600) with an optional title
//! slot, then a 13 px meta line. Padding 10 20 16. Tools end with the header crab (A2.13).

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Div, IntoElement, ParentElement as _, SharedString, Styled as _, div, px,
};

use crate::components::chip::count_badge;
use crate::components::unicrab::unicrab;
use crate::design::brand::CrabState;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, size, space};

/// The parts of a page header.
#[derive(Default)]
pub struct PageHeader {
    /// Title (ellipsised).
    pub title: SharedString,
    /// Meta line under the title (`4 sessions · 1 working`).
    pub meta: Option<SharedString>,
    /// Lead slot of the bar (back, project tile).
    pub lead: Vec<AnyElement>,
    /// Tools at the right of the bar (ghost 40 buttons, then the crab).
    pub tools: Vec<AnyElement>,
    /// After the title (count badge, picker chevron).
    pub title_slot: Option<AnyElement>,
}

impl PageHeader {
    /// A header titled `title`.
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    /// Meta line.
    #[must_use]
    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    /// Adds a lead element.
    #[must_use]
    pub fn lead(mut self, el: impl IntoElement) -> Self {
        self.lead.push(el.into_any_element());
        self
    }

    /// Adds a tool (right side, in order).
    #[must_use]
    pub fn tool(mut self, el: impl IntoElement) -> Self {
        self.tools.push(el.into_any_element());
        self
    }

    /// Ends the tools with the window controls when the window draws its own chrome
    /// (Linux / Windows client decorations).
    #[must_use]
    pub fn window_controls(mut self, window: &gpui_kit::Window, t: &OinoTheme) -> Self {
        if crate::components::window_chrome::client_decorated(window) {
            self.tools.push(
                div()
                    .pl(px(space::S2))
                    .child(crate::components::window_chrome::window_controls(
                        t,
                        window.is_maximized(),
                    ))
                    .into_any_element(),
            );
        }
        self
    }

    /// Element after the title.
    #[must_use]
    pub fn title_slot(mut self, el: impl IntoElement) -> Self {
        self.title_slot = Some(el.into_any_element());
        self
    }

    /// Renders the header.
    #[must_use]
    pub fn render(self, t: &OinoTheme) -> Div {
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w_full()
            .px(px(size::PAGE_GUTTER))
            .pt(px(size::PAGE_HEAD_PAD.0))
            .pb(px(size::PAGE_HEAD_PAD.1))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S1))
                    .h(px(size::PAGE_HEAD_BAR))
                    .children(self.lead)
                    .child(div().flex_1())
                    .children(self.tools),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .mt(px(space::S1))
                    .min_h(px(size::PAGE_HEAD_BAR - space::S2))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_role(t, TextRole::Display)
                            .weight_semibold()
                            .text_color(t.fg())
                            .child(self.title),
                    )
                    .children(self.title_slot),
            )
            .when_some(self.meta, |el, m| {
                el.child(
                    div()
                        .min_h(px(size::BADGE))
                        .truncate()
                        .text_role(t, TextRole::Caption)
                        .text_color(t.fg_subtle())
                        .child(m),
                )
            })
    }
}

/// The header crab (28) with an amber needs-you count badge (A2.13).
pub fn header_crab(state: CrabState, needs: usize, cx: &mut App) -> Div {
    let t = OinoTheme::get(cx);
    div()
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .size(px(size::ICON_BUTTON_LG))
        .flex_shrink_0()
        .child(unicrab(state, size::CRAB_HEADER, cx))
        .when(needs > 0, |el| {
            el.child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .child(count_badge(&t, needs, true)),
            )
        })
}
