//! `UserBubble` (design.md §4.9).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClipboardItem, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, div, px,
};

use super::row_id;
use crate::components::button::{Btn, BtnSize};
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::transcript::{RowKind, UserRow};
use crate::views::session_pane::{PaneEvent, SessionPane};

impl SessionPane {
    pub(super) fn render_user(
        &self,
        ix: usize,
        user: &UserRow,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let nth = self.model.transcript.rows()[..ix]
            .iter()
            .filter(|r| matches!(r.kind, RowKind::User(_)))
            .count();
        let group = SharedString::from(format!("ub{ix}"));
        let copy = user.text.clone();
        let edit = user.text.clone();
        div()
            .group(group.clone())
            .flex()
            .flex_col()
            .items_end()
            .w_full()
            .gap(px(space::S1))
            .when(user.images > 0, |el| {
                el.child(
                    div()
                        .flex()
                        .gap(px(space::S1))
                        .children((0..user.images.min(6)).map(|_| {
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(size::THUMB))
                                .rounded(px(radius::R_LG))
                                .bg(t.surface())
                                .border_1()
                                .border_color(t.border())
                                .child(icon(IconName::File, size::ICON, t.fg_subtle()))
                        })),
                )
            })
            .child(
                div()
                    .when(!t.touch(), |el| el.max_w(px(size::BUBBLE)))
                    .when(t.touch(), |el| {
                        el.max_w(gpui_kit::relative(crate::design::tokens::touch::BUBBLE))
                    })
                    .py(px(space::S3))
                    .px(px(space::S4))
                    .bg(t.surface())
                    .border_1()
                    .border_color(t.border())
                    .rounded(px(radius::R_CARD))
                    .rounded_tr(px(radius::R_SM))
                    .text_role(t, TextRole::Base)
                    .text_color(t.fg())
                    .child(SharedString::from(user.text.clone())),
            )
            .child(
                div()
                    .flex()
                    .gap(px(space::HALF))
                    .invisible()
                    .group_hover(group, gpui_kit::Styled::visible)
                    .child(
                        Btn::icon_only(row_id("ub-copy", ix), IconName::Copy, "Copy")
                            .size(BtnSize::Sm)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                                cx.emit(PaneEvent::Toast("Copied".into()));
                            })),
                    )
                    .child(
                        Btn::icon_only(
                            row_id("ub-edit", ix),
                            IconName::Pencil,
                            "Edit & resend (fork)",
                        )
                        .size(BtnSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.fork_at_user(nth, edit.clone(), cx);
                        })),
                    ),
            )
            .into_any_element()
    }
}
