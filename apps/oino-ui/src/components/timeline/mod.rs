//! Timeline rows (design.md §4.9): every row sits in the 896 px reading column and renders
//! from the borrowed transcript row (no per-frame clones of row data).

mod cards;
mod tool;
mod user;

use gpui_kit::assets::IconName;
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};

use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::ticker::Ticker;
use crate::design::{TextRole, size, space};
use crate::host::ThinkingBlocks;
use crate::model::timeline;
use crate::model::transcript::{AssistantRow, Row, RowKind};
use crate::views::session_pane::SessionPane;

pub(crate) use cards::error_banner;

fn row_id(name: &'static str, ix: usize) -> ElementId {
    ElementId::NamedInteger(
        SharedString::new_static(name),
        u64::try_from(ix).unwrap_or(0),
    )
}

impl SessionPane {
    /// Renders transcript row `ix` (the `list` processor).
    pub(crate) fn render_row(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Rows are built during the list's prepaint: tick this pane only.
        Ticker::scoped(cx.entity_id(), || self.render_row_inner(ix, window, cx))
    }

    fn render_row_inner(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let rows = self.model.transcript.rows();
        let Some(row) = rows.get(ix) else {
            return div().into_any_element();
        };
        // "Explored N files" grouping of read-only runs.
        if let Some((start, end, _)) = timeline::explore_run(rows, ix) {
            let open = self.explore_open.contains(&rows[start].id);
            if ix != start && !open {
                return div().into_any_element();
            }
            if ix == start {
                let header = self.render_explore(start, end, open, &t, cx);
                let line = open.then(|| self.render_kind(row, ix, window, cx));
                return self.frame_row(
                    ix,
                    &t,
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(space::S1))
                        .child(header)
                        .children(line)
                        .into_any_element(),
                    cx,
                );
            }
        }
        let inner = self.render_kind(row, ix, window, cx);
        let rows = self.model.transcript.rows();
        let footer = timeline::is_turn_end(rows, ix, self.model.is_busy())
            .then(|| {
                let (start, end) = timeline::turn_bounds(rows, ix);
                let changes = timeline::file_changes(&rows[start..end]);
                (!changes.is_empty()).then(|| self.render_turn_footer(ix, &changes, &t, cx))
            })
            .flatten();
        let body = match footer {
            Some(f) => div()
                .flex()
                .flex_col()
                .gap(px(space::S3))
                .child(inner)
                .child(f)
                .into_any_element(),
            None => inner,
        };
        self.frame_row(ix, &t, body, cx)
    }

    /// Wraps a row in the reading column with turn rhythm and find highlighting.
    fn frame_row(
        &self,
        ix: usize,
        t: &OinoTheme,
        body: AnyElement,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = self.model.transcript.rows();
        let blank = match rows.get(ix).map(|r| &r.kind) {
            Some(RowKind::Assistant(a)) => {
                a.text.is_empty() && a.thinking.is_empty() && a.error.is_none()
            }
            _ => false,
        };
        if blank {
            return div().into_any_element();
        }
        let top = match rows.get(ix).map(|r| &r.kind) {
            Some(RowKind::User(_)) if ix > 0 => space::S6,
            _ => space::S1_5,
        };
        let find = self.find.as_ref().and_then(|f| {
            f.matches
                .iter()
                .position(|m| *m == ix)
                .map(|p| p == f.current)
        });
        // The expanded status panel floats top-right: shift the column left when they would
        // overlap (design.md §3.2).
        let shift = if !self.status.is_empty() && self.status_expanded_now() {
            let free = (f32::from(self.width) - size::READING) / 2.0;
            if free < size::STATUS_PANEL + space::S8 {
                size::STATUS_PANEL + space::S4
            } else {
                0.0
            }
        } else {
            0.0
        };
        let cursor = self.timeline_kb && self.cursor == Some(ix);
        div()
            .w_full()
            .pl(px(space::S4))
            .pr(px(space::S4 + shift))
            .pt(px(top))
            .pb(px(space::S1_5))
            .child(
                div()
                    .w_full()
                    .max_w(px(size::READING))
                    .mx_auto()
                    // Keyboard row cursor: 2 px shell rail (A1.2 focus colour).
                    .when(cursor, |el| {
                        el.border_l_2()
                            .border_color(t.focus())
                            .pl(px(space::S2))
                            .rounded(px(crate::design::radius::R_SM))
                    })
                    .when_some(find, |el, current| {
                        el.rounded(px(crate::design::radius::R_LG)).bg(if current {
                            t.find_active().opacity(0.35)
                        } else {
                            t.find()
                        })
                    })
                    .child(body),
            )
            .into_any_element()
    }

    fn render_kind(
        &self,
        row: &Row,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        match &row.kind {
            RowKind::User(user) => self.render_user(ix, user, &t, cx),
            RowKind::Assistant(a) => self.render_assistant(&row.id, ix, a, &t, cx),
            RowKind::Tool(tool) => self.render_tool(&row.id, ix, tool, &t, window, cx),
            RowKind::Bash(bash) => self.render_bash_row(ix, bash, &t, cx),
            RowKind::Custom(custom) => self.render_custom(ix, custom, &t, cx),
            RowKind::Summary(summary) => self.render_summary(&row.id, ix, summary, &t, cx),
            RowKind::Notice(tone, text) => self.render_notice(ix, *tone, text, &t, cx),
        }
    }

    fn render_explore(
        &self,
        start: usize,
        end: usize,
        open: bool,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = self.model.transcript.rows();
        let label = timeline::explore_label(rows, start, end);
        let key = rows[start].id.clone();
        let running = label.starts_with("Exploring");
        div()
            .id(row_id("explore", start))
            .flex()
            .items_center()
            .gap(px(space::S2))
            .h(px(size::TOOL_ROW))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if !this.explore_open.remove(&key) {
                    this.explore_open.insert(key.clone());
                }
                this.list.remeasure();
                cx.notify();
            }))
            .child(if running {
                crate::design::icons::spinner(Ticker::frame(cx), size::ICON_MD, t.fg_subtle())
                    .into_any_element()
            } else {
                icon(IconName::Search, size::ICON_MD, t.fg_subtlest()).into_any_element()
            })
            .child(
                div()
                    .text_role(t, TextRole::Base)
                    .text_color(t.fg_subtle())
                    .child(SharedString::from(label)),
            )
            .child(icon(
                if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                },
                size::ICON_SM,
                t.fg_subtlest(),
            ))
            .into_any_element()
    }

    fn render_assistant(
        &self,
        id: &str,
        ix: usize,
        a: &AssistantRow,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mode = self.settings.thinking_blocks;
        let open = match mode {
            ThinkingBlocks::Expanded => !self.thinking_open.contains(id),
            ThinkingBlocks::Collapsed => self.thinking_open.contains(id),
            ThinkingBlocks::Hidden => false,
        };
        let thinking = (!a.thinking.is_empty() && mode != ThinkingBlocks::Hidden).then(|| {
            let key = id.to_owned();
            let live = a.streaming && a.text.is_empty();
            let label = if live { "Thinking…" } else { "Thought" };
            div()
                .flex()
                .flex_col()
                .gap(px(space::S1))
                .child(
                    div()
                        .id(row_id("think", ix))
                        .flex()
                        .items_center()
                        .gap(px(space::S1_5))
                        .h(px(size::TOOL_ROW))
                        .cursor_pointer()
                        .text_role(t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if !this.thinking_open.remove(&key) {
                                this.thinking_open.insert(key.clone());
                            }
                            if let Some(ix) = this.model.transcript.position(&key) {
                                this.list.remeasure_items(ix..ix + 1);
                            }
                            cx.notify();
                        }))
                        .child(icon(IconName::Brain, size::ICON_MD, t.fg_subtle()))
                        .child(
                            div()
                                .when(live && !t.reduce_motion, |el| {
                                    let f = Ticker::frame(cx);
                                    el.opacity(
                                        0.55 + 0.45
                                            * (f32::from(u8::try_from(f % 12).unwrap_or(0)) / 12.0),
                                    )
                                })
                                .child(label),
                        )
                        .child(icon(
                            if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            },
                            size::ICON_SM,
                            t.fg_subtlest(),
                        )),
                )
                .when(open, |el| {
                    el.child(
                        div()
                            .pl(px(space::S3))
                            .border_l_2()
                            .border_color(t.border())
                            .text_role(t, TextRole::Caption)
                            .text_color(t.thinking())
                            .child(SharedString::from(a.thinking.clone())),
                    )
                })
        });
        let body = self
            .markdown
            .get(id)
            .filter(|_| !a.text.is_empty())
            .map(|(state, _)| {
                div()
                    .text_role(t, TextRole::Base)
                    .text_color(t.fg())
                    .child(TextView::new(state).selectable(true))
                    .into_any_element()
            });
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(space::S2))
            .children(thinking)
            .children(body)
            .when_some(a.error.clone(), |el, e| {
                el.child(error_banner(t, ix, &e, cx))
            })
            .into_any_element()
    }
}
