//! `SummaryCard`, `ProvenanceCard`, `CustomRow`, `ErrorBanner`, `TurnFooter` (design.md §4.9).

use gpui_kit::assets::IconName;
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, ClipboardItem, Context, ElementId, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    div, px,
};

use super::row_id;
use crate::components::button::{Btn, BtnSize, Variant};
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::timeline::{FileChange, split_path};
use crate::model::transcript::{CustomRow, NoticeTone, SummaryKind, SummaryRow};
use crate::views::session_pane::{PaneEvent, SessionPane, SideTab};

/// `ErrorBanner`: `danger_surface` card with actions (Retry, Copy details).
pub(crate) fn error_banner(
    t: &OinoTheme,
    ix: usize,
    message: &str,
    cx: &mut Context<SessionPane>,
) -> AnyElement {
    let copy = message.to_owned();
    div()
        .flex()
        .items_start()
        .gap(px(space::S3))
        .p(px(space::S3))
        .rounded(px(radius::R_CARD))
        .bg(t.danger_surface())
        .text_color(t.danger_surface_fg())
        .child(
            div()
                .pt(px(space::HALF))
                .child(icon(IconName::CircleAlert, size::ICON, t.danger())),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap(px(space::S2))
                .child(
                    div()
                        .text_role(t, TextRole::Base)
                        .child(SharedString::from(message.to_owned())),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(space::S2))
                        .child(
                            Btn::new(row_id("err-retry", ix))
                                .label("Retry")
                                .icon(IconName::RefreshCw)
                                .variant(Variant::Outline)
                                .size(BtnSize::Sm)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.run_prompt("/unipi:retry", cx);
                                })),
                        )
                        .child(
                            Btn::new(row_id("err-copy", ix))
                                .label("Copy details")
                                .icon(IconName::Copy)
                                .variant(Variant::Ghost)
                                .size(BtnSize::Sm)
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                                    cx.emit(PaneEvent::Toast("Copied".into()));
                                })),
                        )
                        .child(
                            Btn::new(row_id("err-model", ix))
                                .label("Switch model")
                                .icon(IconName::Box)
                                .variant(Variant::Ghost)
                                .size(BtnSize::Sm)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_menu(
                                        crate::views::session_pane::PaneMenu::Model,
                                        cx,
                                    );
                                })),
                        ),
                ),
        )
        .into_any_element()
}

impl SessionPane {
    pub(super) fn render_custom(
        &self,
        ix: usize,
        custom: &CustomRow,
        t: &OinoTheme,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let provenance = custom.provenance().map(str::to_owned);
        let harness = provenance.is_some();
        let source = provenance.unwrap_or_else(|| custom.custom_type.clone());
        let rail = if harness {
            t.harness()
        } else {
            t.border_hover()
        };
        div()
            .flex()
            .flex_row()
            .w_full()
            .rounded(px(radius::R_CARD))
            .bg(t.card())
            .border_1()
            .border_color(t.card_border())
            .overflow_hidden()
            .child(div().w(px(size::RAIL)).flex_shrink_0().bg(rail))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .p(px(space::S3))
                    .gap(px(space::S1))
                    .child(
                        div()
                            .text_role(t, TextRole::Xs)
                            .weight_medium()
                            .text_color(if harness { t.harness() } else { t.fg_subtle() })
                            .child(SharedString::from(source)),
                    )
                    .child(
                        div().text_role(t, TextRole::Base).child(
                            TextView::markdown(row_id("custom-md", ix), custom.text.clone())
                                .selectable(true),
                        ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_summary(
        &self,
        id: &str,
        ix: usize,
        s: &SummaryRow,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = id.to_owned();
        let title = match s.kind {
            SummaryKind::Compaction => format!(
                "Context compacted · {}k tokens before",
                s.tokens_before / 1000
            ),
            SummaryKind::Branch => "Branch summary".to_owned(),
        };
        div()
            .id(row_id("summary", ix))
            .flex()
            .flex_col()
            .w_full()
            .gap(px(space::S2))
            .p(px(space::S3))
            .rounded(px(radius::R_CARD))
            .bg(t.card())
            .border_1()
            .border_color(t.card_border())
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_row(&key, cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .text_role(t, TextRole::Sm)
                    .text_color(t.fg_subtle())
                    .child(icon(IconName::Archive, size::ICON_MD, t.fg_subtle()))
                    .child(SharedString::from(title))
                    .child(icon(
                        if s.expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        },
                        size::ICON_SM,
                        t.fg_subtlest(),
                    )),
            )
            .when(s.expanded, |el| {
                el.child(div().text_role(t, TextRole::Base).child(TextView::markdown(
                    row_id("summary-md", ix),
                    s.summary.clone(),
                )))
            })
            .into_any_element()
    }

    pub(super) fn render_notice(
        &self,
        ix: usize,
        tone: NoticeTone,
        text: &str,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match tone {
            NoticeTone::Error => error_banner(t, ix, text, cx),
            NoticeTone::Warning => div()
                .flex()
                .items_center()
                .gap(px(space::S2))
                .text_role(t, TextRole::Sm)
                .text_color(t.warning_surface_fg())
                .child(icon(IconName::TriangleAlert, size::ICON_MD, t.warning()))
                .child(SharedString::from(text.to_owned()))
                .into_any_element(),
            NoticeTone::Info => div()
                .text_role(t, TextRole::Sm)
                .text_color(t.fg_subtle())
                .child(SharedString::from(text.to_owned()))
                .into_any_element(),
        }
    }

    pub(super) fn render_turn_footer(
        &self,
        ix: usize,
        changes: &[FileChange],
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let added: usize = changes.iter().map(|c| c.added).sum();
        let removed: usize = changes.iter().map(|c| c.removed).sum();
        let n = changes.len();
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(space::S2))
            .p(px(space::S2))
            .rounded(px(radius::R_LG))
            .border_1()
            .border_color(t.border())
            .child(
                div()
                    .text_role(t, TextRole::Sm)
                    .text_color(t.fg_subtle())
                    .child(format!("{n} file{} changed", if n == 1 { "" } else { "s" })),
            )
            .child(
                div()
                    .mono_text(t, TextRole::Sm)
                    .text_color(t.diff_added())
                    .child(format!("+{added}")),
            )
            .child(
                div()
                    .mono_text(t, TextRole::Sm)
                    .text_color(t.diff_removed())
                    .child(format!("−{removed}")),
            )
            .children(changes.iter().take(4).enumerate().map(|(i, c)| {
                let (name, _) = split_path(&c.path);
                let title = c.path.clone();
                let patch = c.patches.join("\n");
                div()
                    .id(ElementId::NamedInteger(
                        SharedString::new_static("turn-file"),
                        u64::try_from(ix * 16 + i).unwrap_or(0),
                    ))
                    .flex()
                    .items_center()
                    .gap(px(space::S1))
                    .h(px(size::TOKEN))
                    .px(px(space::S1_5))
                    .rounded(px(radius::R_MD))
                    .bg(t.file_node())
                    .text_color(t.file_node_fg())
                    .text_role(t, TextRole::Sm)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.open_side(
                            SideTab::Diff {
                                title: title.clone(),
                                patch: patch.clone(),
                            },
                            cx,
                        );
                    }))
                    .child(icon(IconName::FileText, size::ICON_SM, t.file_node_fg()))
                    .child(SharedString::from(name))
            }))
            .child(div().flex_1())
            .child(
                Btn::new(row_id("turn-review", ix))
                    .label("Review")
                    .variant(Variant::Outline)
                    .size(BtnSize::Sm)
                    .on_click(cx.listener(|this, _, _, cx| this.open_side(SideTab::Review, cx))),
            )
            // Undo is offered only once core file checkpoints exist (P1-6); no dead button.
            .into_any_element()
    }
}
