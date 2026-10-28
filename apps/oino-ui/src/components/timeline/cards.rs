//! Harness cards, summary cards, inline error rows, notices and the turn end (design.md A2.10).

use std::fmt::Write as _;

use gpui_kit::assets::IconName;
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, ClipboardItem, Context, ElementId, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    div, px,
};

use super::{HARNESS_BODY_MAX, row_id};
use crate::components::button::{Btn, BtnSize, Variant};
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::timeline::{self, FileChange, split_path};
use crate::model::transcript::{CustomRow, NoticeTone, RowKind, SummaryKind, SummaryRow};
use crate::views::session_pane::{PaneEvent, PaneMenu, SessionPane, SideTab};

/// Rows of "Changes this turn" shown before "Show all".
const CHANGES_VISIBLE: usize = 3;

fn stopped(message: &str) -> bool {
    let m = message.trim().to_lowercase();
    m == "aborted" || m.starts_with("operation aborted") || m == "request was aborted"
}

/// Inline error row (A2.10): `danger` x + message + Retry pill. "Switch model" only for
/// model / provider errors (GA-9). An aborted run reads "Stopped" (amber chip).
pub(crate) fn error_banner(
    t: &OinoTheme,
    ix: usize,
    message: &str,
    cx: &mut Context<SessionPane>,
) -> AnyElement {
    if stopped(message) {
        return div()
            .flex()
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(size::BADGE))
                    .px(px(space::S2))
                    .rounded(px(radius::R_FULL))
                    .bg(t.chip())
                    .text_role(t, TextRole::Xs)
                    .weight_semibold()
                    .text_color(t.amber())
                    .child("Stopped"),
            )
            .into_any_element();
    }
    let copy = message.to_owned();
    let model = timeline::is_model_error(message);
    div()
        .flex()
        .items_start()
        .gap(px(space::S2))
        .w_full()
        .py(px(space::S1))
        .child(
            div()
                .pt(px(space::HALF))
                .child(icon(IconName::CircleX, size::ICON, t.danger())),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_role(t, TextRole::Caption)
                .text_color(t.danger_surface_fg())
                .child(SharedString::from(message.to_owned())),
        )
        .child(
            div()
                .flex()
                .flex_shrink_0()
                .gap(px(space::S1))
                .child(
                    Btn::new(row_id("err-retry", ix))
                        .label("Retry")
                        .icon(IconName::RefreshCw)
                        .variant(Variant::Secondary)
                        .size(BtnSize::Sm)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.run_prompt("/retry", cx);
                        })),
                )
                .when(model, |el| {
                    el.child(
                        Btn::new(row_id("err-model", ix))
                            .label("Switch model")
                            .variant(Variant::Ghost)
                            .size(BtnSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_menu(PaneMenu::Model, cx);
                            })),
                    )
                })
                .child(
                    Btn::icon_only(row_id("err-copy", ix), IconName::Copy, "Copy details")
                        .size(BtnSize::Sm)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                            cx.emit(PaneEvent::Toast("Copied".into()));
                        })),
                ),
        )
        .into_any_element()
}

/// A harness card (A2.10): radius 14, transparent, inset line, head = claw icon + source +
/// title + chevron; collapsed by default (GA-6).
#[allow(clippy::too_many_arguments, reason = "card parts")]
fn harness(
    t: &OinoTheme,
    id: ElementId,
    glyph: IconName,
    source: String,
    title: String,
    open: bool,
    body: Option<AnyElement>,
    on_toggle: impl Fn(&ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .w_full()
        .rounded(px(radius::R_HARNESS))
        .border_1()
        .border_color(t.border().opacity(0.75))
        .child(
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(space::S2))
                .min_h(px(size::TOOL_ROW))
                .px(px(space::S3))
                .py(px(space::S1))
                .rounded(px(radius::R_HARNESS))
                .cursor_pointer()
                .hover(|s| s.bg(t.card()))
                .on_click(on_toggle)
                .child(icon(glyph, size::ICON_MD, t.working()))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_role(t, TextRole::Caption)
                        .weight_semibold()
                        .text_color(t.fg())
                        .child(SharedString::from(source)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_role(t, TextRole::Caption)
                        .text_color(t.fg_subtle())
                        .child(SharedString::from(title)),
                )
                .child(icon(
                    if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    },
                    size::ICON_MD,
                    t.fg_subtlest(),
                )),
        )
        .when_some(body.filter(|_| open), |el, b| {
            el.child(
                div()
                    .px(px(space::S3))
                    .pb(px(space::S2))
                    .text_role(t, TextRole::Caption)
                    .text_color(t.fg_subtle())
                    .child(b),
            )
        })
        .into_any_element()
}

fn custom_icon(custom_type: &str) -> IconName {
    let c = custom_type.to_lowercase();
    if c.contains("task") {
        IconName::CircleCheck
    } else if c.contains("memory") || c.contains("recall") {
        IconName::Brain
    } else if c.contains("plan") {
        IconName::ListChecks
    } else if c.contains("subagent") || c.contains("sidekick") {
        IconName::Bot
    } else if c.contains("kanboard") || c.contains("board") {
        IconName::Kanban
    } else {
        IconName::Sparkles
    }
}

impl SessionPane {
    /// Toggles harness card `id`.
    pub(crate) fn toggle_card(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.cards_open.remove(id) {
            self.cards_open.insert(id.to_owned());
        }
        if let Some(ix) = self.model.transcript.position(id) {
            self.list.remeasure_items(ix..ix + 1);
        }
        cx.notify();
    }

    pub(super) fn render_custom(
        &self,
        id: &str,
        ix: usize,
        custom: &CustomRow,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (source, title, body) =
            timeline::harness_card(&custom.custom_type, custom.provenance(), &custom.text);
        let open = self.cards_open.contains(id);
        let key = id.to_owned();
        let body = open.then(|| {
            div()
                .id(row_id("harness-body", ix))
                .max_h(px(HARNESS_BODY_MAX))
                .overflow_y_scroll()
                .child(TextView::markdown(row_id("custom-md", ix), body).selectable(true))
                .into_any_element()
        });
        harness(
            t,
            row_id("harness", ix),
            custom_icon(&custom.custom_type),
            source,
            title,
            open,
            body,
            cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_card(&key, cx)),
        )
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
        let (source, title) = match s.kind {
            SummaryKind::Compaction => (
                "Context compacted".to_owned(),
                format!("{}k tokens before", s.tokens_before / 1000),
            ),
            SummaryKind::Branch => ("Branch summary".to_owned(), String::new()),
        };
        let body = s.expanded.then(|| {
            div()
                .id(row_id("summary-body", ix))
                .max_h(px(HARNESS_BODY_MAX))
                .overflow_y_scroll()
                .child(TextView::markdown(
                    row_id("summary-md", ix),
                    s.summary.clone(),
                ))
                .into_any_element()
        });
        harness(
            t,
            row_id("summary", ix),
            IconName::Archive,
            source,
            title,
            s.expanded,
            body,
            cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_row(&key, cx)),
        )
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
                .min_h(px(size::TOOL_ROW))
                .text_role(t, TextRole::Caption)
                .text_color(t.amber())
                .child(icon(IconName::TriangleAlert, size::ICON_MD, t.amber()))
                .child(SharedString::from(text.to_owned()))
                .into_any_element(),
            NoticeTone::Info => div()
                .flex()
                .items_center()
                .gap(px(space::S2))
                .min_h(px(size::TOOL_ROW))
                .text_role(t, TextRole::Caption)
                .text_color(t.fg_subtle())
                .child(icon(IconName::Info, size::ICON_MD, t.fg_subtlest()))
                .child(SharedString::from(text.to_owned()))
                .into_any_element(),
        }
    }

    /// "Done in 3s · $0.002" (A2.10), when this run's timing is known.
    pub(super) fn render_turn_end(
        &self,
        start: usize,
        end: usize,
        t: &OinoTheme,
    ) -> Option<AnyElement> {
        let rows = self.model.transcript.rows();
        let (took, cost) = rows[start..end]
            .iter()
            .rev()
            .find_map(|r| self.turn_stats.get(&r.id))?;
        let mut text = format!(
            "Done in {}",
            timeline::duration(u64::try_from(took.as_millis()).unwrap_or(u64::MAX))
        );
        if *cost > 0.0 {
            let _ = write!(text, " · ${cost:.4}");
        }
        Some(
            div()
                .px(px(space::S1))
                .text_role(t, TextRole::Xs)
                .text_color(t.fg_subtlest())
                .child(SharedString::from(text))
                .into_any_element(),
        )
    }

    /// "Changes this turn" card (A2.10): file rows with stats, review and undo.
    pub(super) fn render_turn_footer(
        &self,
        ix: usize,
        start: usize,
        changes: &[FileChange],
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let added: usize = changes.iter().map(|c| c.added).sum();
        let removed: usize = changes.iter().map(|c| c.removed).sum();
        let n = changes.len();
        let rows = self.model.transcript.rows();
        let nth = rows[..start]
            .iter()
            .filter(|r| matches!(r.kind, RowKind::User(_)))
            .count();
        let undo = !self.model.is_busy() && self.has_checkpoint(nth);
        let file_rows = changes
            .iter()
            .take(CHANGES_VISIBLE)
            .enumerate()
            .map(|(i, c)| {
                let (name, dir) = split_path(&c.path);
                let title = c.path.clone();
                let patch = c.patches.join("\n");
                div()
                    .id(ElementId::NamedInteger(
                        SharedString::new_static("turn-file"),
                        u64::try_from(ix * 16 + i).unwrap_or(0),
                    ))
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .h(px(size::CONTROL))
                    .px(px(space::S2))
                    .rounded(px(radius::R_ROW))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.open_side(
                            SideTab::Diff {
                                title: title.clone(),
                                patch: patch.clone(),
                            },
                            cx,
                        );
                    }))
                    .child(icon(IconName::FileText, size::ICON_MD, t.fg_subtlest()))
                    .child(
                        div()
                            .mono_text(t, TextRole::Sm)
                            .text_color(t.fg())
                            .child(SharedString::from(name)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .mono_text(t, TextRole::Sm)
                            .text_color(t.fg_subtlest())
                            .child(SharedString::from(dir)),
                    )
                    .child(
                        div()
                            .mono_text(t, TextRole::Xs)
                            .text_color(t.diff_added())
                            .child(format!("+{}", c.added)),
                    )
                    .child(
                        div()
                            .mono_text(t, TextRole::Xs)
                            .text_color(t.diff_removed())
                            .child(format!("−{}", c.removed)),
                    )
            });
        div()
            .flex()
            .flex_col()
            .w_full()
            .p(px(space::S1_5))
            .gap(px(space::HALF))
            .rounded(px(radius::R_HARNESS))
            .bg(t.card())
            .border_1()
            .border_color(t.border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .px(px(space::S2))
                    .h(px(size::CONTROL))
                    .child(
                        div()
                            .text_role(t, TextRole::Sm)
                            .weight_semibold()
                            .text_color(t.fg())
                            .child(format!(
                                "Changes this turn · {n} file{}",
                                if n == 1 { "" } else { "s" }
                            )),
                    )
                    .child(
                        div()
                            .mono_text(t, TextRole::Xs)
                            .text_color(t.diff_added())
                            .child(format!("+{added}")),
                    )
                    .child(
                        div()
                            .mono_text(t, TextRole::Xs)
                            .text_color(t.diff_removed())
                            .child(format!("−{removed}")),
                    )
                    .child(div().flex_1())
                    .when(undo, |el| {
                        el.child(
                            Btn::new(row_id("turn-undo", ix))
                                .label("Undo")
                                .icon(IconName::RotateCcw)
                                .variant(Variant::Ghost)
                                .size(BtnSize::Sm)
                                .tooltip("Restore the files this turn changed", None)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.request_rewind(nth, false, cx);
                                })),
                        )
                    })
                    .child(
                        Btn::new(row_id("turn-review", ix))
                            .label(if n > CHANGES_VISIBLE {
                                "Show all"
                            } else {
                                "Review"
                            })
                            .variant(Variant::Secondary)
                            .size(BtnSize::Sm)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.open_side(SideTab::Review, cx)),
                            ),
                    ),
            )
            .children(file_rows)
            .into_any_element()
    }
}
