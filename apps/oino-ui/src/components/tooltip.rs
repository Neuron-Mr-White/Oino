//! `Tooltip` (design.md §4.2): `tooltip` bg, `Sm` text, `R_LG`, shortcut as `Kbd`.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyView, App, AppContext as _, Context, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Window, div, px,
};

use crate::components::badge::kbd;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};

/// Tooltip content view.
pub struct TipView {
    text: SharedString,
    keys: Option<SharedString>,
}

impl Render for TipView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = OinoTheme::get(cx);
        div()
            .flex()
            .items_center()
            .gap(px(space::S2))
            .max_w(px(size::TOAST))
            .py(px(space::S1_5))
            .px(px(space::S2))
            .rounded(px(radius::R_LG))
            .bg(t.tooltip())
            .border_1()
            .border_color(t.popover_border())
            .shadow(t.e2())
            .text_color(t.tooltip_fg())
            .text_role(&t, TextRole::Sm)
            .font_family(t.ui_font())
            .child(self.text.clone())
            .when_some(self.keys.clone(), |el, k| el.child(kbd(&t, k)))
    }
}

/// A tooltip builder for `.tooltip(..)`.
pub fn tip(
    text: impl Into<SharedString>,
    keys: Option<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| {
        cx.new(|_| TipView {
            text: text.clone(),
            keys: keys.clone(),
        })
        .into()
    }
}
