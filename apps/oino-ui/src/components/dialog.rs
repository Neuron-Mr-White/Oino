//! `Dialog` (design.md A2.12): centred over a 45 % scrim, `sheet` (surface-1) fill, radius 16,
//! dialog elevation; header title 20/26/600 (padding 14 20 10) + ✕ 34; the body scrolls
//! inside min(80 %, 760) (GA-7); sticky footer with right-aligned actions, primary last.

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px, relative,
};

use crate::automation::Automatable as _;
use crate::components::button::{Btn, BtnSize};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};

/// Dialog enter duration.
const MOTION_MS: u64 = crate::design::tokens::motion::DIALOG_MS;

/// Close callback.
pub type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

/// Focus handles of app-level modals (result card, confirm, readme; QA-1).
#[derive(Default)]
struct OwnedFocus(HashMap<&'static str, FocusHandle>);

impl gpui_kit::Global for OwnedFocus {}

/// The focus handle of the app-level modal `id`. A newly shown modal focuses itself (a
/// dialog with no autofocus field focuses itself, A2 L11), so Esc and its buttons act at
/// once instead of requiring a click first (QA-1 repro 3).
pub fn owned_focus(id: &'static str, window: &mut Window, cx: &mut App) -> FocusHandle {
    let existing = cx.default_global::<OwnedFocus>().0.get(id).cloned();
    let handle = if let Some(h) = existing {
        h
    } else {
        let h = cx.focus_handle();
        cx.global_mut::<OwnedFocus>().0.insert(id, h.clone());
        h
    };
    if !handle.contains_focused(window, cx) {
        window.focus(&handle, cx);
    }
    handle
}

/// Forgets the focus handle of a closed app-level modal.
pub fn forget_owned_focus(id: &'static str, cx: &mut App) {
    if cx.has_global::<OwnedFocus>() {
        cx.global_mut::<OwnedFocus>().0.remove(id);
    }
}

/// [`modal_frame`] for an app-level modal that owns the keyboard while it is open (QA-1):
/// the frame tracks `focus` (see [`owned_focus`]) and every close path runs `restore` to put
/// focus back on a live surface instead of the removed dialog.
#[allow(clippy::too_many_arguments, reason = "plain layout parameters")]
pub fn modal_owned(
    t: &OinoTheme,
    id: &'static str,
    title: SharedString,
    width: f32,
    body: AnyElement,
    actions: Vec<AnyElement>,
    focus: &FocusHandle,
    restore: Rc<dyn Fn(&mut Window, &mut App)>,
) -> AnyElement {
    let close_id = id;
    let on_close: OnClose = Rc::new(move |w, cx| {
        forget_owned_focus(close_id, cx);
        restore(w, cx);
    });
    // The scrim itself tracks the handle: the focused element must be the scrim (or inside
    // it) for its Esc handler and buttons to receive keys (QA-1).
    modal_with_toolbar_focused(
        t,
        id,
        title,
        width,
        Vec::new(),
        body,
        actions,
        on_close,
        Some(focus.clone()),
    )
}

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

/// A modal with header toolbar buttons (panel keys) next to the close button. `focus`
/// tracks the scrim so the dialog can own the keyboard (QA-1); `None` for modals that
/// focus a field of their own.
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
    modal_with_toolbar_focused(t, id, title, width, toolbar, body, actions, on_close, None)
}

/// [`modal_with_toolbar`] with the scrim focus handle (see [`modal_owned`]).
#[allow(clippy::too_many_arguments, reason = "plain layout parameters")]
pub fn modal_with_toolbar_focused(
    t: &OinoTheme,
    id: &'static str,
    title: SharedString,
    width: f32,
    toolbar: Vec<AnyElement>,
    body: AnyElement,
    actions: Vec<AnyElement>,
    on_close: OnClose,
    focus: Option<FocusHandle>,
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
        .when_some(focus, |el, f| el.track_focus(&f))
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
                                        .child(title.clone())
                                        // Read-back for scenarios: every modal's title.
                                        .automation_named(
                                            crate::automation::Meta::labelled(
                                                crate::automation::Role::Text,
                                                title.to_string(),
                                            ),
                                            format!("{id}-title"),
                                        ),
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
