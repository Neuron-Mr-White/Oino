//! User bubble (design.md A2.10): right-aligned, `bubble` (surface-3), radius 20, `Prose`,
//! max 80 %. A leading `/command` shows as a chip. Hover action bar (pill, surface-2, 28 px
//! round buttons): Copy, Edit & resend, Rewind (files + conversation, file checkpoints).

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

/// Splits a leading `/command` off a prompt.
fn command_head(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix('/')?;
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    (end > 0).then(|| (&text[..=end], text[end + 1..].trim_start()))
}

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
        let copy_key = format!("row-{ix}");
        let copied = self.copied_check(&copy_key);
        let busy = self.model.is_busy();
        let rewind = !busy && self.has_checkpoint(nth);
        let (chip, text) = match command_head(&user.text) {
            Some((cmd, rest)) => (Some(cmd.to_owned()), rest.to_owned()),
            None => (None, user.text.clone()),
        };
        let (prose, _) = t.text(TextRole::Prose);
        // A2.10: the hover bar sits 14 px above the bubble's top edge (a negative offset
        // without a `px(-…)` literal, which the design guard bans).
        let hover_raise = 0.0 - space::S3_5;
        let actions = div()
            .absolute()
            .top(px(hover_raise))
            .right(px(space::S2))
            .flex()
            .items_center()
            .gap(px(space::HALF))
            .p(px(space::HALF))
            .rounded(px(radius::R_FULL))
            .bg(t.chip())
            .border_1()
            .border_color(t.border())
            .invisible()
            .group_hover(group.clone(), gpui_kit::Styled::visible)
            .child(
                // ZCode `CopyRowAction`: the glyph turns into a check for 1200 ms.
                Btn::icon_only(
                    row_id("ub-copy", ix),
                    if copied { IconName::Check } else { IconName::Copy },
                    if copied { "Copied" } else { "Copy" },
                )
                .size(BtnSize::Sm)
                .when(copied, |b| b.color(t.success()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                    this.mark_copied(&copy_key.clone(), cx);
                    cx.emit(PaneEvent::Toast("Copied".into()));
                })),
            )
            .when(!busy, |el| {
                el.child(
                    Btn::icon_only(row_id("ub-edit", ix), IconName::Pencil, "Edit & resend")
                        .size(BtnSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.fork_at_user(nth, edit.clone(), cx);
                        })),
                )
            })
            .when(rewind, |el| {
                el.child(
                    Btn::icon_only(
                        row_id("ub-rewind", ix),
                        IconName::RotateCcw,
                        "Rewind to here (restore files and conversation)",
                    )
                    .size(BtnSize::Sm)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.request_rewind(nth, true, cx);
                    })),
                )
            });
        div()
            .flex()
            .flex_col()
            .items_end()
            .w_full()
            .gap(px(space::S1))
            // Hovering anywhere on the row (not only the bubble) reveals the action bar.
            .group(group.clone())
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
                                .rounded(px(radius::R_INPUT))
                                .bg(t.bubble())
                                .child(icon(IconName::File, size::ICON, t.fg_subtle()))
                        })),
                )
            })
            .child(
                div()
                    .group(group)
                    .relative()
                    .max_w(gpui_kit::relative(if t.touch() {
                        crate::design::tokens::touch::BUBBLE
                    } else {
                        size::BUBBLE_FRACTION
                    }))
                    .py(px(space::S2 + 1.0))
                    .px(px(space::S3_5 + 1.0))
                    .bg(t.bubble())
                    .rounded(px(radius::R_BUBBLE))
                    .text_size(prose)
                    .line_height(px(f32::from(prose) + space::S2 - 1.0))
                    .text_color(t.fg())
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(space::S1_5))
                            .when_some(chip, |el, c| {
                                el.child(
                                    div()
                                        .flex_none()
                                        .px(px(space::S1_5))
                                        .rounded(px(radius::R_CODE))
                                        .bg(t.command_node())
                                        .text_color(t.command_node_fg())
                                        .mono_text(t, TextRole::Sm)
                                        .child(SharedString::from(c)),
                                )
                            })
                            .when(!text.is_empty(), |el| {
                                el.child(div().min_w_0().child(SharedString::from(text)))
                            }),
                    )
                    .child(actions),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_heads() {
        assert_eq!(
            super::command_head("/plan on now"),
            Some(("/plan", "on now"))
        );
        assert_eq!(super::command_head("/new"), Some(("/new", "")));
        assert_eq!(super::command_head("hi /x"), None);
        assert_eq!(super::command_head("/ x"), None);
    }
}
