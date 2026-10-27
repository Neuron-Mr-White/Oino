//! `Dialog` (design.md A2.12): centred over a 45 % scrim, `sheet` (surface-1) fill, radius 16,
//! dialog elevation; header title 20/26/600 (padding 14 20 10) + ✕ 34; the body scrolls
//! inside min(80 %, 760) (GA-7); sticky footer with right-aligned actions, primary last.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px, relative,
};

use crate::components::button::{Btn, BtnSize};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};

/// Dialog enter duration.
const MOTION_MS: u64 = crate::design::tokens::motion::DIALOG_MS;

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
                .flex()
                .flex_col()
                .justify_center()
                .h(relative(size::DIALOG_MAX_FRACTION))
                .max_h(px(size::DIALOG_MAX_H))
                .max_w(relative(0.94))
                .child(crate::design::motion::enter_ms(
                    t,
                    ElementId::Name(format!("{id}-enter").into()),
                    div()
                        .id(ElementId::Name(id.into()))
                        .flex()
                        .flex_col()
                        .w(px(width))
                        .max_w_full()
                        .max_h_full()
                        .rounded(px(radius::R_FLOAT))
                        .bg(t.sheet())
                        .border_1()
                        .border_color(t.border())
                        .shadow(t.e_dialog())
                        .text_color(t.fg())
                        .font_family(t.ui_font())
                        .on_click(|_: &ClickEvent, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .flex()
                                .flex_shrink_0()
                                .items_center()
                                .gap(px(space::S2))
                                .pl(px(space::S5))
                                .pr(px(space::S3))
                                .pt(px(space::S3_5))
                                .pb(px(space::S2_5))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_role(t, TextRole::Title)
                                        .weight_semibold()
                                        .child(title),
                                )
                                .children(toolbar)
                                .child(
                                    Btn::icon_only(
                                        ElementId::Name(format!("{id}-x").into()),
                                        IconName::X,
                                        "Close",
                                    )
                                    .size(BtnSize::Md)
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
                                    .flex_shrink_0()
                                    .justify_end()
                                    .gap(px(space::S2))
                                    .px(px(space::S5))
                                    .pt(px(space::S3))
                                    .pb(px(space::S5))
                                    .border_t_1()
                                    .border_color(t.line_soft())
                                    .children(actions.into_iter().map(|a| {
                                        div().min_w(px(size::DIALOG_ACTION_MIN)).flex().child(a)
                                    })),
                            )
                        }),
                    MOTION_MS,
                )),
        )
        .into_any_element()
}
