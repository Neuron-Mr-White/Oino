//! `Dialog` (design.md §4.24): centred modal over a scrim, `popover` bg, `R_2XL`, E2; header
//! title `Lg`/500 + close; footer actions primary first, then Cancel.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px, relative,
};

use crate::components::button::Btn;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};

/// Close callback.
pub type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

/// A modal dialog of `width` with `body` and right-aligned `actions`.
pub fn modal_frame(
    t: &OinoTheme,
    id: &'static str,
    title: SharedString,
    width: f32,
    body: AnyElement,
    actions: Vec<AnyElement>,
    on_close: OnClose,
) -> AnyElement {
    modal_with_toolbar(t, id, title, width, Vec::new(), body, actions, on_close)
}

/// A modal with header toolbar buttons (panel keys) next to the close button.
#[allow(clippy::too_many_arguments, reason = "plain layout parameters")]
pub fn modal_with_toolbar(
    t: &OinoTheme,
    id: &'static str,
    title: SharedString,
    width: f32,
    toolbar: Vec<AnyElement>,
    body: AnyElement,
    actions: Vec<AnyElement>,
    on_close: OnClose,
) -> AnyElement {
    let close_key = Rc::clone(&on_close);
    let close_scrim = Rc::clone(&on_close);
    div()
        .id(ElementId::Name(format!("{id}-scrim").into()))
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(t.scrim())
        .occlude()
        .on_click(move |_: &ClickEvent, w, cx| close_scrim(w, cx))
        .on_key_down(move |e: &KeyDownEvent, w, cx| {
            if e.keystroke.key == "escape" {
                close_key(w, cx);
                cx.stop_propagation();
            }
        })
        .child(
            div()
                .id(ElementId::Name(id.into()))
                .flex()
                .flex_col()
                .w(px(width))
                .max_w(relative(0.94))
                .max_h(px(size::DIALOG_MAX_H))
                .rounded(px(radius::R_2XL))
                .bg(t.popover())
                .border_1()
                .border_color(t.popover_border())
                .shadow(t.e2())
                .text_color(t.fg())
                .font_family(t.ui_font())
                .on_click(|_: &ClickEvent, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S2))
                        .px(px(space::S5))
                        .pt(px(space::S4))
                        .pb(px(space::S2))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_role(t, TextRole::Lg)
                                .weight_medium()
                                .child(title),
                        )
                        .children(toolbar)
                        .child(
                            Btn::icon_only(
                                ElementId::Name(format!("{id}-x").into()),
                                IconName::X,
                                "Close",
                            )
                            .on_click(move |_, w, cx| on_close(w, cx)),
                        ),
                )
                .child(
                    div()
                        .id(ElementId::Name(format!("{id}-body").into()))
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px(px(space::S5))
                        .pb(px(space::S5))
                        .child(body),
                )
                .when(!actions.is_empty(), |el| {
                    el.child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap(px(space::S2))
                            .px(px(space::S5))
                            .pb(px(space::S4))
                            .children(actions),
                    )
                }),
        )
        .into_any_element()
}
