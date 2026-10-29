//! Timeline rows (design.md A2.10, the deepseek-harness look): a right user bubble, assistant
//! prose without a bubble, **one fold line per run of thinking and tool calls**, harness cards
//! collapsed to one line, agent rows, inline error rows and the turn end ("Done in 3s · $0.002"
//! plus "Changes this turn"). Every row sits in the 940 px reading column and renders from the
//! borrowed transcript row (no per-frame clones of row data).

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

use crate::automation::Automatable as _;
use crate::design::icons::{icon, spinner};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::ticker::Ticker;
use crate::design::{TextRole, radius, size, space};
use crate::host::{ThinkingBlocks, ToolDensity};
use crate::model::timeline::{self, FoldSummary};
use crate::model::tools::ToolKind;
use crate::model::transcript::{AssistantRow, Row, RowKind};
use crate::views::session_pane::SessionPane;

/// The hover-bar time label of an answer ([`ZCode`] `messageTimeLabel`): `14:05` today,
/// `Yesterday 14:05`, `3/13 14:05` this year, `2025/3/13 14:05` older; local time.
/// `None` for history rows, which carry no timestamp.
///
/// [`ZCode`]: https://github.com/zai-org/ZCode
fn message_time_label(created_ms: Option<i64>, now_ms: i64) -> Option<String> {
    use chrono::{Datelike, Local, TimeZone};
    let created = Local.timestamp_millis_opt(created_ms?).single()?;
    let now = Local.timestamp_millis_opt(now_ms).single()?;
    let time = created.format("%H:%M").to_string();
    if created.date_naive() == now.date_naive() {
        return Some(time);
    }
    if now
        .date_naive()
        .signed_duration_since(created.date_naive())
        .num_days()
        == 1
    {
        return Some(format!("Yesterday {time}"));
    }
    if created.year() == now.year() {
        Some(format!("{}/{} {time}", created.month(), created.day()))
    } else {
        Some(format!(
            "{}/{}/{} {time}",
            created.year(),
            created.month(),
            created.day()
        ))
    }
}

pub(crate) use cards::error_banner;

/// Tool / fold line icon (A2.10: 18).
pub(crate) const ICON_TOOL: f32 = 18.0;
/// Open fold body max height (A2.4 `DETAIL_MAX` 420).
pub(crate) const FOLD_BODY_MAX: f32 = 420.0;
/// Open harness card body max height (A2.10: 320).
pub(crate) const HARNESS_BODY_MAX: f32 = 320.0;

fn row_id(name: &'static str, ix: usize) -> ElementId {
    ElementId::NamedInteger(
        SharedString::new_static(name),
        u64::try_from(ix).unwrap_or(0),
    )
}

/// Shimmer opacity of a live line (1.6 s sweep; frame 0 under reduce motion).
pub(crate) fn shimmer(t: &OinoTheme, frame: u32) -> f32 {
    if t.reduce_motion {
        return 1.0;
    }
    #[allow(clippy::cast_precision_loss, reason = "small frame counter")]
    let phase = (frame % 19) as f32 / 19.0;
    0.55 + 0.45 * (phase * std::f32::consts::TAU).sin().abs()
}

fn fold_icon(kind: Option<ToolKind>) -> IconName {
    match kind {
        Some(ToolKind::Read) => IconName::FileText,
        Some(ToolKind::Write | ToolKind::Edit) => IconName::FilePen,
        Some(ToolKind::Shell) => IconName::SquareTerminal,
        Some(ToolKind::Search) => IconName::Search,
        Some(_) => IconName::Zap,
        None => IconName::Brain,
    }
}

/// `text` without ANSI SGR escapes (tool output is coloured for the terminal view; the
/// automation tree should read as plain text).
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // Skip `ESC [ … m` (SGR) and any other `ESC x` pair.
        if chars.next() == Some('[') {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
    }
    out
}

/// The text of a transcript row, for the automation registry (capped at 400 chars).
pub(crate) fn row_text(row: Option<&Row>) -> String {
    const MAX: usize = 400;
    let raw = match row.map(|r| &r.kind) {
        Some(RowKind::User(u)) => u.text.clone(),
        Some(RowKind::Assistant(a)) => a.text.clone(),
        Some(RowKind::Tool(t)) => format!("{} {}", t.name, t.output),
        Some(RowKind::Bash(b)) => format!("{} {}", b.command, b.output),
        Some(RowKind::Custom(c)) => c.text.clone(),
        Some(RowKind::Summary(s)) => s.summary.clone(),
        Some(RowKind::Notice(_, text)) => text.clone(),
        None => return String::new(),
    };
    let plain = strip_ansi(&raw);
    let trimmed = plain.trim();
    if trimmed.chars().count() <= MAX {
        return trimmed.to_owned();
    }
    let cut: String = trimmed.chars().take(MAX).collect();
    format!("{cut}…")
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

    /// Whether the fold starting at row `start` is open (density settings invert it).
    pub(crate) fn fold_open(&self, start_id: &str) -> bool {
        let toggled = self.explore_open.contains(start_id);
        // Round-5 #22: `expand_tools` is a dead settings key (no setter writes it); the
        // fold respects the row-density setting and the user's toggle only.
        let open_default = self.settings.tool_density == ToolDensity::Advanced;
        toggled ^ open_default
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
            // The last list item while a run is active (`sync_run_row` splices it in):
            // the generating strip is a row, not a fixed footer (#21).
            return self.render_run_row(&t, cx);
        };
        // One fold line per run of thinking + tool calls (A2.10).
        if let Some((start, end)) = timeline::fold_run(rows, ix) {
            let open = self.fold_open(&rows[start].id);
            let blank = matches!(&row.kind, RowKind::Assistant(a) if a.thinking.trim().is_empty());
            let body = (open && !blank).then(|| self.render_fold_body(row, ix, window, cx));
            let el = if ix == start {
                let summary = timeline::fold_summary(rows, start, end);
                let head = self.render_fold_line(start, &summary, open, &t, cx);
                div()
                    .flex()
                    .flex_col()
                    .child(head)
                    .children(body)
                    .into_any_element()
            } else {
                match body {
                    Some(b) => b,
                    None => return div().into_any_element(),
                }
            };
            let last = ix == end;
            let el = self.with_turn_end(ix, el, &t, cx);
            return self.frame_row(ix, &t, el, (ix == start, last), cx);
        }
        let inner = self.render_kind(row, ix, window, cx);
        let body = self.with_turn_end(ix, inner, &t, cx);
        self.frame_row(ix, &t, body, (true, true), cx)
    }

    /// Appends the turn end ("Done in …", "Changes this turn") after the last row of a turn.
    fn with_turn_end(
        &self,
        ix: usize,
        inner: AnyElement,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = self.model.transcript.rows();
        if !timeline::is_turn_end(rows, ix, self.model.is_busy()) {
            return inner;
        }
        let (start, end) = timeline::turn_bounds(rows, ix);
        if !rows
            .get(start)
            .is_some_and(|r| matches!(r.kind, RowKind::User(_)))
        {
            return inner;
        }
        let changes = timeline::file_changes(&rows[start..end]);
        let card =
            (!changes.is_empty()).then(|| self.render_turn_footer(ix, start, &changes, t, cx));
        let done = self.render_turn_end(start, end, t);
        div()
            .flex()
            .flex_col()
            .gap(px(space::S2_5))
            .child(inner)
            .children(card)
            .children(done)
            .into_any_element()
    }

    /// Wraps a row in the reading column (A2.10: max 940, padding 28, rows 10 apart) with
    /// the keyboard cursor and find highlighting. `edges`: (first, last) row of a fold run,
    /// which sit flush so the fold body reads as one block.
    fn frame_row(
        &self,
        ix: usize,
        t: &OinoTheme,
        body: AnyElement,
        edges: (bool, bool),
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
        let half = space::S2_5 / 2.0;
        let top = match rows.get(ix).map(|r| &r.kind) {
            Some(RowKind::User(_)) if ix > 0 => space::S4,
            _ if !edges.0 => space::S0,
            _ => half,
        };
        let bottom = if edges.1 { half } else { space::S0 };
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
        let pad = if t.touch() {
            t.gutter()
        } else {
            px(size::READING_PAD)
        };
        let cursor = self.timeline_kb && self.cursor == Some(ix);
        let text = row_text(rows.get(ix));
        div()
            .w_full()
            .pl(pad)
            .pr(pad + px(shift))
            .pt(px(top))
            .pb(px(bottom))
            .child(
                div()
                    .id(ElementId::NamedInteger("row".into(), ix as u64))
                    .w_full()
                    .max_w(px(size::READING))
                    .mx_auto()
                    .rounded(px(radius::R_ROW))
                    // Keyboard row cursor: 2 px shell outline (A2 L11 focus).
                    .when(cursor, |el| el.border_2().border_color(t.focus()))
                    .when_some(find, |el, current| {
                        el.bg(if current {
                            t.find_active().opacity(0.35)
                        } else {
                            t.find()
                        })
                    })
                    .child(body)
                    // Read-back for the automation driver (STATUS.md §1b.7 gap 2: screen
                    // reading was OCR over screenshots).
                    .automation(
                        crate::automation::Meta::labelled(crate::automation::Role::Text, text)
                            .value(rows[ix].id.clone()),
                    ),
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
            RowKind::Custom(custom) => self.render_custom(&row.id, ix, custom, &t, cx),
            RowKind::Summary(summary) => self.render_summary(&row.id, ix, summary, &t, cx),
            RowKind::Notice(tone, text) => self.render_notice(ix, *tone, text, &t, cx),
        }
    }

    /// The fold line (A2.10): icon, title, `· detail`, chevron; the live action shimmers.
    fn render_fold_line(
        &self,
        start: usize,
        s: &FoldSummary,
        open: bool,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = self.model.transcript.rows()[start].id.clone();
        let frame = if s.live.is_some() {
            Ticker::frame(cx)
        } else {
            0
        };
        let glyph: AnyElement = if s.live.is_some() {
            spinner(frame, ICON_TOOL, t.working()).into_any_element()
        } else {
            icon(fold_icon(s.kind), ICON_TOOL, t.fg_subtlest()).into_any_element()
        };
        let (title, detail) = match &s.live {
            Some(live) => (live.clone(), String::new()),
            None => (s.title.clone(), s.detail.clone()),
        };
        div()
            .id(row_id("fold", start))
            .flex()
            .items_center()
            .gap(px(space::S1_5 + 1.0))
            .min_h(px(size::TOOL_ROW))
            .px(px(space::S1))
            .rounded(px(radius::R_ROW))
            .cursor_pointer()
            .hover(|st| st.bg(t.hover()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if !this.explore_open.remove(&key) {
                    this.explore_open.insert(key.clone());
                }
                // Only this fold's run changed height (#21: no full `remeasure()` while
                // a turn streams).
                if let Some((first, last)) = timeline::fold_run(this.model.transcript.rows(), start)
                {
                    this.list.remeasure_items(first..last + 1);
                }
                cx.notify();
            }))
            .child(glyph)
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(gpui_kit::relative(0.7))
                    .truncate()
                    .text_role(t, TextRole::Caption)
                    .weight_medium()
                    .text_color(t.fg_subtle())
                    .when(s.live.is_some(), |el| el.opacity(shimmer(t, frame)))
                    .child(SharedString::from(title.clone())),
            )
            .when(!detail.is_empty(), |el| {
                el.child(
                    div()
                        .truncate()
                        .text_role(t, TextRole::Caption)
                        .text_color(t.fg_subtle())
                        .child(SharedString::from(format!("· {detail}"))),
                )
            })
            .when(s.failed > 0, |el| {
                el.child(
                    div()
                        .flex_shrink_0()
                        .text_role(t, TextRole::Caption)
                        .text_color(t.danger_surface_fg())
                        .child(SharedString::from(format!("· {} failed", s.failed))),
                )
            })
            .child(div().flex_1())
            .child(icon(
                if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                },
                size::ICON_MD,
                t.fg_subtlest(),
            ))
            .automation(
                crate::automation::Meta::labelled(
                    crate::automation::Role::Row,
                    if detail.is_empty() {
                        title.clone()
                    } else {
                        format!("{title} · {detail}")
                    },
                )
                .selected(open),
            )
            .into_any_element()
    }

    /// One row inside an open fold: indent 12 + 10 with a 1 px left rule (A2.10).
    fn render_fold_body(
        &self,
        row: &Row,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let inner = match &row.kind {
            RowKind::Assistant(a) => self.render_thought_line(&row.id, ix, a, &t, cx),
            _ => self.render_kind(row, ix, window, cx),
        };
        div()
            .id(row_id("fold-row", ix))
            .ml(px(size::FOLD_INDENT))
            .pl(px(space::S2_5))
            .border_l_1()
            .border_color(t.border())
            .child(inner)
            .automation(
                crate::automation::Meta::labelled(
                    crate::automation::Role::Text,
                    row_text(Some(row)),
                )
                .value(row.id.clone()),
            )
            .into_any_element()
    }

    /// A thinking block as one line inside a fold: the latest paragraph; click opens it. The
    /// label is `Thinking` while the phase runs (with the live shimmer), `Thought for 12.3s`
    /// once it is over (design.md §4.9, `ZCode` `chat-thought-level-control`), `Thought` for
    /// history rows that carry no timing.
    fn render_thought_line(
        &self,
        id: &str,
        ix: usize,
        a: &AssistantRow,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.thinking_open.contains(id);
        let key = id.to_owned();
        let frame = Ticker::frame(cx);
        let latest = a
            .thinking
            .split("\n\n")
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .last()
            .unwrap_or_default();
        let line = timeline::excerpt(latest, 1, 140);
        let thinking_live = a.streaming && a.text.is_empty();
        let label = if thinking_live {
            "Thinking".to_owned()
        } else if let Some(ms) = a.thinking_ms {
            format!("Thought for {}", timeline::duration(ms))
        } else {
            "Thought".to_owned()
        };
        let shown = if open { label.clone() } else { line.clone() };
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(row_id("think", ix))
                    .flex()
                    .items_center()
                    .gap(px(space::S1_5 + 1.0))
                    .min_h(px(size::TOOL_ROW))
                    .px(px(space::S1))
                    .rounded(px(radius::R_ROW))
                    .cursor_pointer()
                    .hover(|st| st.bg(t.hover()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if !this.thinking_open.remove(&key) {
                            this.thinking_open.insert(key.clone());
                        }
                        if let Some(ix) = this.model.transcript.position(&key) {
                            this.list.remeasure_items(ix..ix + 1);
                        }
                        cx.notify();
                    }))
                    .child(icon(IconName::Brain, ICON_TOOL, t.fg_subtlest()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_role(t, TextRole::Caption)
                            .text_color(t.fg_subtle())
                            .when(thinking_live, |el| el.opacity(shimmer(t, frame)))
                            .child(SharedString::from(shown.clone()))
                            .automation(
                                crate::automation::Meta::labelled(
                                    crate::automation::Role::Text,
                                    shown,
                                )
                                .value(label),
                            ),
                    ),
            )
            .when(open, |el| {
                el.child(
                    div()
                        .id(row_id("think-body", ix))
                        .max_h(px(FOLD_BODY_MAX))
                        .overflow_y_scroll()
                        .px(px(space::S1))
                        .pb(px(space::S1_5))
                        .text_role(t, TextRole::Caption)
                        .text_color(t.thinking())
                        .child(SharedString::from(a.thinking.clone())),
                )
            })
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
        let thinking = (!a.thinking.trim().is_empty() && mode != ThinkingBlocks::Hidden)
            .then(|| self.render_thought_line(id, ix, a, t, cx));
        let body = self
            .markdown
            .get(id)
            .filter(|_| !a.text.is_empty())
            .map(|(state, _)| {
                div()
                    .max_w(px(size::PROSE_MAX))
                    .text_role(t, TextRole::Prose)
                    .text_color(t.fg())
                    .child(
                        crate::design::theme::markdown_view(TextView::new(state), t)
                            .selectable(true),
                    )
                    .into_any_element()
            });
        // Hover actions of one answer (ZCode `CopyRowAction` + `messageTimeLabel`): copy the
        // answer text (the glyph turning into a check for `COPIED_MS`) and, for live rows,
        // when it arrived. Hidden while the answer still streams.
        let copy_key = format!("row-{ix}");
        let copied = self.copied_check(&copy_key);
        let copy = a.text.clone();
        let time_label =
            message_time_label(a.created_ms, crate::views::now_ms()).map(SharedString::from);
        let actions = (!a.streaming && !a.text.is_empty()).then(|| {
            let group = SharedString::from(format!("as{ix}"));
            div()
                .absolute()
                .top_0()
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
                    crate::components::button::Btn::icon_only(
                        row_id("as-copy", ix),
                        if copied {
                            IconName::Check
                        } else {
                            IconName::Copy
                        },
                        if copied { "Copied" } else { "Copy answer" },
                    )
                    .size(crate::components::button::BtnSize::Sm)
                    .when(copied, |b| b.color(t.success()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(copy.clone()));
                        this.mark_copied(&copy_key.clone(), cx);
                    })),
                )
                .children(time_label.map(|label| {
                    div()
                        .id(row_id("as-time", ix))
                        .pl(px(space::S1))
                        .pr(px(space::S2))
                        .text_role(t, TextRole::Caption)
                        .text_color(t.fg_subtle())
                        .child(label)
                        .automation(crate::automation::Meta::labelled(
                            crate::automation::Role::Text,
                            "Answer time",
                        ))
                }))
                .into_any_element()
        });
        div()
            .relative()
            .group(SharedString::from(format!("as{ix}")))
            .flex()
            .flex_col()
            .w_full()
            .gap(px(space::S2))
            .children(thinking)
            .children(body)
            .children(actions)
            .when_some(a.error.clone(), |el, e| {
                el.child(error_banner(t, ix, &e, cx))
            })
            .into_any_element()
    }

    /// The trailing "generating" list item: the run-state strip drawn inside the
    /// reading column so it scrolls with the transcript instead of pinning a fixed
    /// footer (round-5 #21; Zed `thread_view.rs` `sync_generating_indicator`, mechanism
    /// reimplemented). `sync_run_row` keeps it spliced while a run is busy or its exit
    /// line is fading; anything else out of range renders nothing.
    fn render_run_row(&mut self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let Some(strip) = self.render_run_state(t, cx) else {
            return div().into_any_element();
        };
        let pad = if t.touch() {
            t.gutter()
        } else {
            px(size::READING_PAD)
        };
        div()
            .w_full()
            .pl(pad)
            .pr(pad)
            .child(
                div()
                    .id("run-state")
                    .w_full()
                    .max_w(px(size::READING))
                    .mx_auto()
                    .child(strip)
                    .automation(crate::automation::Meta::labelled(
                        crate::automation::Role::Text,
                        "Generating",
                    )),
            )
            .into_any_element()
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::message_time_label;
    use chrono::{Datelike, Days, Local, TimeZone};

    /// Local time `h:m` on `date`, as Unix ms (0 if that local time does not exist).
    fn on(date: chrono::NaiveDate, h: u32, m: u32) -> i64 {
        Local
            .with_ymd_and_hms(date.year(), date.month(), date.day(), h, m, 0)
            .single()
            .map_or(0, |t| t.timestamp_millis())
    }

    #[test]
    fn label_graduates_with_distance() {
        let today = Local::now().date_naive();
        let now = on(today, 15, 0);
        assert_eq!(
            message_time_label(Some(on(today, 14, 5)), now).as_deref(),
            Some("14:05")
        );
        // History rows carry no timestamp.
        assert_eq!(message_time_label(None, now), None);
        let yesterday = today.pred_opt().expect("a previous day");
        assert_eq!(
            message_time_label(Some(on(yesterday, 14, 0)), now).as_deref(),
            Some("Yesterday 14:00")
        );
        // 90 days back: `M/D HH:MM` while the year matches.
        let old = today.checked_sub_days(Days::new(90)).unwrap_or(today);
        if old.year() == today.year() {
            let label = message_time_label(Some(on(old, 14, 0)), now).expect("same year");
            assert_eq!(label.matches('/').count(), 1, "{label}");
        }
        // A year back adds `Y/`.
        let stale = today
            .with_year(today.year() - 1)
            .or_else(|| today.with_day(1).and_then(|d| d.with_year(d.year() - 1)))
            .unwrap_or(today);
        let label = message_time_label(Some(on(stale, 14, 0)), now).expect("a year back");
        assert_eq!(label.matches('/').count(), 2, "{label}");
    }
}
