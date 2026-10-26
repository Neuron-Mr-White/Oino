//! OUS v1 renderer (design.md §4.27): every node renders natively from tokens; interactive
//! nodes report `(actionId, value)` through an [`ActionFn`]; `responsive` picks the variant
//! for the available columns; `effect: drain` meters animate on the shared ticker.

pub mod kanban;

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Div, ElementId, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px, relative,
};
use oino_ai::JsonValue;

use crate::components::badge::{badge, kbd, tone_dot};
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::controls::{context_ring, segmented, switch};
use crate::components::tabs::underline_tabs;
use crate::design::icons::{icon, spinner};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::ticker::Ticker;
use crate::design::{TextRole, radius, size, space};
use crate::model::ous::{Effect, OusNode, Rich, Tone, TreeItem};
use crate::model::tools::{DiffLineKind, diff_preview};

/// Callback for interactive nodes: `(actionId, value)`.
pub type ActionFn = Rc<dyn Fn(&str, JsonValue, &mut App)>;

/// An action callback that ignores everything (read-only surfaces).
#[must_use]
pub fn no_action() -> ActionFn {
    Rc::new(|_, _, _| {})
}

/// Persistent text inputs of OUS `input` nodes (keyed by panel + node id).
#[derive(Default)]
pub struct InputStore {
    map: HashMap<String, (Entity<InputState>, Subscription)>,
}

impl InputStore {
    fn get(
        &mut self,
        key: &str,
        initial: &str,
        placeholder: Option<&str>,
        on_submit: impl Fn(String, &mut App) + 'static,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<InputState> {
        if let Some((state, _)) = self.map.get(key) {
            return state.clone();
        }
        let placeholder = SharedString::from(placeholder.unwrap_or_default().to_owned());
        let initial = SharedString::from(initial.to_owned());
        let state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(initial)
        });
        let sub = cx.subscribe(&state, move |entity, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let value = entity.read(cx).value().to_string();
                on_submit(value, cx);
            }
        });
        self.map.insert(key.to_owned(), (state.clone(), sub));
        state
    }

    /// Drops inputs of a closed panel.
    pub fn forget(&mut self, prefix: &str) {
        self.map.retain(|k, _| !k.starts_with(prefix));
    }
}

/// Render context.
pub struct Ous<'a> {
    /// Theme.
    pub t: OinoTheme,
    /// Action callback.
    pub on_action: ActionFn,
    /// Input store (panels); `None` renders inputs read-only.
    pub inputs: Option<&'a mut InputStore>,
    /// Available width in columns (≈ px / 8) for `responsive`.
    pub cols: u32,
    /// Inside a side pane / overlay (diffs render as full viewers).
    pub roomy: bool,
}

/// Start times of draining meters (keyed by node hash).
#[derive(Default)]
struct Drains(HashMap<u64, Instant>);

impl gpui_kit::Global for Drains {}

/// Renders rich text as one paragraph.
#[must_use]
pub fn rich_text(t: &OinoTheme, rich: &Rich) -> AnyElement {
    let mut row = div().flex().flex_row().flex_wrap().items_baseline();
    for span in rich {
        let color = if span.dim {
            t.fg_subtle()
        } else {
            t.tone_fg(span.tone)
        };
        let mut el = div()
            .text_color(color)
            .child(SharedString::from(span.text.clone()));
        if span.bold || crate::design::tones::emphasised(span.tone) {
            el = el.font_weight(if span.bold {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::MEDIUM
            });
        }
        if span.italic {
            el = el.italic();
        }
        if span.code {
            el = el
                .mono_text(t, TextRole::Sm)
                .px(px(space::S1))
                .rounded(px(radius::R_SM))
                .bg(t.tag());
        }
        if span.link.is_some() {
            el = el.underline().text_color(t.ansi(12));
        }
        row = row.child(el);
    }
    row.into_any_element()
}

/// Renders a unified patch (`InlineDiff` styling) with at most `max` lines.
#[must_use]
pub fn diff_lines(t: &OinoTheme, patch: &str, max: usize) -> AnyElement {
    let preview = diff_preview(patch, max);
    let mut col = div()
        .flex()
        .flex_col()
        .w_full()
        .code_text(t)
        .overflow_hidden();
    for line in preview.lines {
        let (bg, bar, fg) = match line.kind {
            DiffLineKind::Added => (Some(t.diff_added_bg()), Some(t.diff_added()), t.fg()),
            DiffLineKind::Removed => (Some(t.diff_removed_bg()), Some(t.diff_removed()), t.fg()),
            DiffLineKind::Hunk => (None, None, t.fg_subtlest()),
            DiffLineKind::Header | DiffLineKind::Context => (None, None, t.fg_subtle()),
        };
        col = col.child(
            div()
                .flex()
                .flex_row()
                .when_some(bg, gpui_kit::Styled::bg)
                .child(
                    div()
                        .w(px(space::HALF))
                        .flex_shrink_0()
                        .when_some(bar, gpui_kit::Styled::bg),
                )
                .child(
                    div()
                        .px(px(space::S2))
                        .whitespace_nowrap()
                        .text_color(fg)
                        .child(SharedString::from(line.text)),
                ),
        );
    }
    if preview.hidden > 0 {
        col = col.child(
            div()
                .px(px(space::S2))
                .py(px(space::S1))
                .text_color(t.fg_subtlest())
                .text_role(t, TextRole::Sm)
                .child(format!("… {} more lines", preview.hidden)),
        );
    }
    col.into_any_element()
}

fn bar(t: &OinoTheme, ratio: f32, tone: Tone) -> Div {
    let fill = if tone == Tone::Default {
        t.fg_subtle()
    } else {
        t.tone_rail(tone)
    };
    div()
        .h(px(size::DOT))
        .w_full()
        .rounded(px(radius::R_FULL))
        .bg(t.border())
        .child(
            div()
                .h_full()
                .w(relative(ratio.clamp(0.0, 1.0)))
                .rounded(px(radius::R_FULL))
                .bg(fill),
        )
}

#[allow(clippy::cast_possible_truncation, reason = "ratio in 0..=1")]
fn ratio(value: f64, max: f64) -> f32 {
    if max > 0.0 {
        (value / max).clamp(0.0, 1.0) as f32
    } else {
        0.0
    }
}

fn tree_rows(
    o: &Ous<'_>,
    items: &[TreeItem],
    depth: usize,
    selectable: Option<&str>,
    key: &str,
    out: &mut Vec<AnyElement>,
) {
    let t = &o.t;
    for item in items {
        let id = item.id.clone();
        let action = selectable.map(str::to_owned);
        let cb = Rc::clone(&o.on_action);
        #[allow(clippy::cast_precision_loss, reason = "small depth")]
        let indent = px(depth as f32 * space::S4);
        let mut row = div()
            .id(ElementId::Name(format!("{key}/t/{}", item.id).into()))
            .flex()
            .items_center()
            .h(px(size::CONTROL_SM))
            .pl(indent)
            .gap(px(space::S1))
            .rounded(px(radius::R_MD))
            .child(if item.children.is_empty() {
                div().w(px(size::ICON_MD)).into_any_element()
            } else {
                icon(
                    if item.expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    },
                    size::ICON_MD,
                    t.fg_subtle(),
                )
                .into_any_element()
            })
            .child(rich_text(t, &item.label));
        if let Some(action) = action {
            row = row.cursor_pointer().hover(|s| s.bg(t.hover())).on_click(
                move |_: &ClickEvent, _, cx| cb(&action, JsonValue::from(id.clone()), cx),
            );
        }
        out.push(row.into_any_element());
        if item.expanded || depth == 0 && item.children.len() < 8 {
            tree_rows(o, &item.children, depth + 1, selectable, key, out);
        }
    }
}

fn chart_bars(o: &Ous<'_>, values: &[f64], color_ix: usize, height: f32) -> Div {
    let t = &o.t;
    let max = values.iter().copied().fold(0.0_f64, f64::max);
    div()
        .flex()
        .flex_row()
        .items_end()
        .gap(px(space::HALF / 2.0))
        .h(px(height))
        .children(values.iter().map(|v| {
            div()
                .flex_1()
                .min_w(px(space::HALF))
                .h(relative(ratio(*v, max).max(0.02)))
                .rounded(px(radius::R_SM / 2.0))
                .bg(t.categorical(color_ix))
        }))
}

/// Renders `node`; `key` must be unique among siblings rendered in the same frame.
#[allow(clippy::too_many_lines, reason = "one arm per OUS node kind")]
pub fn render(
    node: &OusNode,
    key: &str,
    o: &mut Ous<'_>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = o.t;
    match node {
        OusNode::Responsive { .. } => {
            let resolved = node.resolve(o.cols);
            render(resolved, key, o, window, cx)
        }
        OusNode::Text { value, wrap } => {
            let el = rich_text(&t, value);
            if *wrap {
                el
            } else {
                div()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(el)
                    .into_any_element()
            }
        }
        OusNode::Markdown { value } => {
            TextView::markdown(ElementId::Name(format!("{key}/md").into()), value.clone())
                .selectable(true)
                .into_any_element()
        }
        OusNode::Code { value, language } => code_block(&t, value, language.as_deref(), key),
        OusNode::Diff { patch, path } => div()
            .flex()
            .flex_col()
            .w_full()
            .rounded(px(radius::R_LG))
            .border_1()
            .border_color(t.border())
            .overflow_hidden()
            .when_some(path.clone(), |el, path| {
                el.child(
                    div()
                        .px(px(space::S2))
                        .py(px(space::S1))
                        .bg(t.surface())
                        .mono_text(&t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(SharedString::from(path)),
                )
            })
            .child(diff_lines(
                &t,
                patch,
                if o.roomy {
                    2000
                } else {
                    crate::model::tools::INLINE_DIFF_LINES
                },
            ))
            .into_any_element(),
        OusNode::Image { alt } => div()
            .flex()
            .items_center()
            .gap(px(space::S2))
            .p(px(space::S2))
            .rounded(px(radius::R_LG))
            .bg(t.surface())
            .text_color(t.fg_subtle())
            .text_role(&t, TextRole::Sm)
            .child(icon(IconName::File, size::ICON, t.fg_subtle()))
            .child(SharedString::from(alt.clone()))
            .into_any_element(),
        OusNode::Log {
            lines, max_lines, ..
        } => {
            let start = max_lines.map_or(0, |m| lines.len().saturating_sub(m));
            crate::components::log_view::log_lines(&t, &lines[start..], key)
        }
        OusNode::Kv { items } => div()
            .flex()
            .flex_col()
            .gap(px(space::S1))
            .children(items.iter().map(|(k, v)| {
                div()
                    .flex()
                    .flex_row()
                    .gap(px(space::S3))
                    .child(
                        div()
                            .w(px(size::SIDEBAR / 2.0))
                            .flex_shrink_0()
                            .text_color(t.fg_subtle())
                            .text_role(&t, TextRole::Sm)
                            .child(rich_text(&t, k)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_role(&t, TextRole::Base)
                            .child(rich_text(&t, v)),
                    )
            }))
            .into_any_element(),
        OusNode::Table {
            columns,
            rows,
            selectable,
        } => {
            let header = div()
                .flex()
                .flex_row()
                .w_full()
                .gap(px(space::S2))
                .px(px(space::S2))
                .h(px(size::CONTROL))
                .items_center()
                .border_b_1()
                .border_color(t.border())
                .text_role(&t, TextRole::Sm)
                .weight_medium()
                .text_color(t.fg_subtle())
                .children(columns.iter().map(|c| {
                    div()
                        .flex_1()
                        .truncate()
                        .child(SharedString::from(c.clone()))
                }));
            let body = rows.iter().enumerate().map(|(i, row)| {
                let cb = Rc::clone(&o.on_action);
                let action = selectable.clone();
                div()
                    .id(ElementId::NamedInteger(
                        SharedString::from(format!("{key}/r")),
                        u64::try_from(i).unwrap_or(0),
                    ))
                    .flex()
                    .flex_row()
                    .w_full()
                    .gap(px(space::S2))
                    .px(px(space::S2))
                    .min_h(px(size::CONTROL_LG))
                    .items_center()
                    .border_b_1()
                    .border_color(t.border())
                    .text_role(&t, TextRole::Base)
                    .children(
                        row.iter()
                            .map(|cell| div().flex_1().min_w_0().child(rich_text(&t, cell))),
                    )
                    .when_some(action, |el, action| {
                        el.cursor_pointer().hover(|s| s.bg(t.hover())).on_click(
                            move |_: &ClickEvent, _, cx| cb(&action, JsonValue::from(i), cx),
                        )
                    })
            });
            div()
                .flex()
                .flex_col()
                .w_full()
                .rounded(px(radius::R_LG))
                .border_1()
                .border_color(t.border())
                .overflow_hidden()
                .child(header)
                .children(body)
                .into_any_element()
        }
        OusNode::List {
            items,
            selectable,
            selected,
        } => div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(space::HALF))
            .children(items.iter().map(|item| {
                let cb = Rc::clone(&o.on_action);
                let action = selectable.clone();
                let id = item.id.clone();
                let is_selected = selected.as_deref() == Some(item.id.as_str());
                div()
                    .id(ElementId::Name(format!("{key}/l/{}", item.id).into()))
                    .flex()
                    .flex_row()
                    .items_center()
                    .w_full()
                    .gap(px(space::S2))
                    .px(px(space::S2))
                    .min_h(px(size::CONTROL_LG))
                    .rounded(px(radius::R_MD))
                    .when(is_selected, |el| el.bg(t.selected()))
                    .when(item.tone != Tone::Default, |el| {
                        el.child(tone_dot(&t, item.tone))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .py(px(space::S1))
                            .child(
                                div()
                                    .text_role(&t, TextRole::Base)
                                    .text_color(t.fg())
                                    .child(rich_text(&t, &item.label)),
                            )
                            .when_some(item.detail.as_ref(), |el, d| {
                                el.child(
                                    div()
                                        .text_role(&t, TextRole::Sm)
                                        .text_color(t.fg_subtle())
                                        .child(rich_text(&t, d)),
                                )
                            }),
                    )
                    .when_some(item.badge.as_ref(), |el, b| {
                        el.child(badge(&t, item.tone, crate::model::ous::plain(b)))
                    })
                    .when_some(action, |el, action| {
                        el.cursor_pointer().hover(|s| s.bg(t.hover())).on_click(
                            move |_: &ClickEvent, _, cx| {
                                cb(&action, JsonValue::from(id.clone()), cx);
                            },
                        )
                    })
            }))
            .into_any_element(),
        OusNode::Tree { nodes, selectable } => {
            let mut rows = Vec::new();
            tree_rows(o, nodes, 0, selectable.as_deref(), key, &mut rows);
            div()
                .flex()
                .flex_col()
                .w_full()
                .text_role(&t, TextRole::Base)
                .children(rows)
                .into_any_element()
        }
        OusNode::Tabs {
            tabs,
            active,
            on_change,
        } => {
            let active_ix = active
                .as_ref()
                .and_then(|a| tabs.iter().position(|(id, _, _)| id == a))
                .unwrap_or(0);
            let ids: Vec<String> = tabs.iter().map(|(id, _, _)| id.clone()).collect();
            let cb = Rc::clone(&o.on_action);
            let action = on_change.clone();
            let pick: Rc<dyn Fn(usize, &mut Window, &mut App)> = Rc::new(move |i, _, cx| {
                if let (Some(action), Some(id)) = (&action, ids.get(i)) {
                    cb(action, JsonValue::from(id.clone()), cx);
                }
            });
            let titles: Vec<String> = tabs.iter().map(|(_, title, _)| title.clone()).collect();
            let bar = if titles.len() <= 5 && o.cols >= 60 {
                let labels: Vec<&str> = titles.iter().map(String::as_str).collect();
                div().child(segmented(
                    format!("{key}/tabs"),
                    &t,
                    &labels,
                    active_ix,
                    pick,
                ))
            } else {
                underline_tabs(&format!("{key}/tabs"), &t, &titles, active_ix, pick)
            };
            let body = tabs
                .get(active_ix)
                .map(|(id, _, body)| render(body, &format!("{key}/{id}"), o, window, cx));
            div()
                .flex()
                .flex_col()
                .w_full()
                .gap(px(space::S3))
                .child(bar)
                .children(body)
                .into_any_element()
        }
        OusNode::Split {
            horizontal,
            children,
        }
        | OusNode::Stack {
            horizontal,
            children,
            ..
        } => {
            let gap = match node {
                OusNode::Stack { gap: Some(g), .. } => px(*g * space::S1),
                OusNode::Stack { gap: None, .. } => px(space::S2),
                _ => px(space::S3),
            };
            let split = matches!(node, OusNode::Split { .. });
            let n = u32::try_from(children.len().max(1)).unwrap_or(1);
            let saved = o.cols;
            if *horizontal && split {
                o.cols = (o.cols / n).max(8);
            }
            let mut base = div().flex().w_full().gap(gap);
            base = if *horizontal {
                base.flex_row().items_start()
            } else {
                base.flex_col()
            };
            let mut kids = Vec::with_capacity(children.len());
            for (i, c) in children.iter().enumerate() {
                let el = render(c, &format!("{key}/{i}"), o, window, cx);
                kids.push(if *horizontal && split {
                    div().flex_1().min_w_0().child(el).into_any_element()
                } else {
                    el
                });
            }
            o.cols = saved;
            base.children(kids).into_any_element()
        }
        OusNode::Section {
            title,
            body,
            collapsed,
            ..
        } => {
            let inner = (!collapsed).then(|| render(body, &format!("{key}/b"), o, window, cx));
            div()
                .flex()
                .flex_col()
                .w_full()
                .gap(px(space::S2))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S1))
                        .text_role(&t, TextRole::Sm)
                        .weight_medium()
                        .text_color(t.fg_subtle())
                        .when(*collapsed, |el| {
                            el.child(icon(IconName::ChevronRight, size::ICON_SM, t.fg_subtle()))
                        })
                        .child(rich_text(&t, title)),
                )
                .children(inner)
                .into_any_element()
        }
        OusNode::Card {
            title,
            rail,
            source,
            body,
            effect,
        } => {
            let inner = render(body, &format!("{key}/c"), o, window, cx);
            let rail_color = rail.map(|r| t.tone_rail(r));
            let shimmer = *effect == Some(Effect::Shimmer) && !t.reduce_motion;
            div()
                .flex()
                .flex_row()
                .w_full()
                .rounded(px(radius::R_CARD))
                .bg(t.card())
                .border_1()
                .border_color(if shimmer {
                    t.border_hover()
                } else {
                    t.card_border()
                })
                .overflow_hidden()
                .when_some(rail_color, |el, c| {
                    el.child(div().w(px(size::RAIL)).flex_shrink_0().bg(c))
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .p(px(space::S3))
                        .gap(px(space::S2))
                        .when(title.is_some() || source.is_some(), |el| {
                            el.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::S2))
                                    .when_some(source.clone(), |el, s| {
                                        el.child(
                                            div()
                                                .text_role(&t, TextRole::Xs)
                                                .weight_medium()
                                                .text_color(rail_color.unwrap_or(t.harness()))
                                                .child(SharedString::from(s)),
                                        )
                                    })
                                    .when_some(title.as_ref(), |el, ti| {
                                        el.child(
                                            div()
                                                .text_role(&t, TextRole::Base)
                                                .weight_medium()
                                                .child(rich_text(&t, ti)),
                                        )
                                    }),
                            )
                        })
                        .child(inner),
                )
                .into_any_element()
        }
        OusNode::Badge { value, tone } => {
            badge(&t, *tone, crate::model::ous::plain(value)).into_any_element()
        }
        OusNode::Meter {
            value,
            max,
            label,
            tone,
            effect,
            duration_ms,
        } => {
            let mut r = ratio(*value, *max);
            if *effect == Some(Effect::Drain)
                && let Some(ms) = duration_ms.filter(|ms| *ms > 0)
                && !t.reduce_motion
            {
                let hash = node.content_hash();
                let start = *cx
                    .default_global::<Drains>()
                    .0
                    .entry(hash)
                    .or_insert_with(Instant::now);
                #[allow(clippy::cast_precision_loss, reason = "durations in ms")]
                let left = 1.0 - (start.elapsed().as_millis() as f32 / ms as f32);
                r *= left.clamp(0.0, 1.0);
                if left > 0.0 {
                    Ticker::frame(cx);
                }
            }
            div()
                .flex()
                .flex_col()
                .w_full()
                .gap(px(space::S1))
                .when(label.is_some() || effect.is_none(), |el| {
                    el.child(
                        div()
                            .flex()
                            .justify_between()
                            .text_role(&t, TextRole::Sm)
                            .text_color(t.fg_subtle())
                            .child(SharedString::from(label.clone().unwrap_or_default()))
                            .when(effect.is_none(), |el| {
                                el.child(
                                    div()
                                        .mono_text(&t, TextRole::Sm)
                                        .child(format!("{value} / {max}")),
                                )
                            }),
                    )
                })
                .child(bar(&t, r, *tone))
                .into_any_element()
        }
        OusNode::Gauge { value, label } => div()
            .flex()
            .items_center()
            .gap(px(space::S2))
            .child(context_ring(&t, ratio(*value, 1.0)))
            .child(
                div()
                    .text_role(&t, TextRole::Sm)
                    .text_color(t.fg_subtle())
                    .child(format!(
                        "{}{:.0}%",
                        label.as_ref().map(|l| format!("{l} ")).unwrap_or_default(),
                        value * 100.0
                    )),
            )
            .into_any_element(),
        OusNode::Sparkline { values, label } => div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(space::S1))
            .when_some(label.clone(), |el, l| {
                el.child(
                    div()
                        .text_role(&t, TextRole::Xs)
                        .text_color(t.fg_subtlest())
                        .child(SharedString::from(l)),
                )
            })
            .child(chart_bars(o, values, 0, size::CONTROL))
            .into_any_element(),
        OusNode::BarChart { bars } => {
            let max = bars.iter().map(|b| b.value).fold(0.0_f64, f64::max);
            div()
                .flex()
                .flex_col()
                .w_full()
                .gap(px(space::S1_5))
                .children(bars.iter().enumerate().map(|(i, b)| {
                    let tone = if b.tone == Tone::Default {
                        Tone::Palette(u8::try_from(i % 12).unwrap_or(0))
                    } else {
                        b.tone
                    };
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(space::S2))
                        .text_role(&t, TextRole::Sm)
                        .child(
                            div()
                                .w(px(size::SIDEBAR / 2.0))
                                .truncate()
                                .text_color(t.fg_subtle())
                                .child(SharedString::from(b.label.clone())),
                        )
                        .child(div().flex_1().child(bar(&t, ratio(b.value, max), tone)))
                        .child(
                            div()
                                .w(px(size::CONTROL_LG * 2.0))
                                .flex()
                                .justify_end()
                                .mono_text(&t, TextRole::Sm)
                                .child(format!("{}", b.value)),
                        )
                }))
                .into_any_element()
        }
        OusNode::Area { series } => div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(space::S2))
            .children(series.iter().enumerate().map(|(i, (label, values))| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::S1))
                    .child(
                        div()
                            .text_role(&t, TextRole::Xs)
                            .text_color(t.fg_subtlest())
                            .child(SharedString::from(label.clone())),
                    )
                    .child(chart_bars(o, values, i, size::CONTROL_LG))
            }))
            .into_any_element(),
        OusNode::Spinner { label } => div()
            .flex()
            .items_center()
            .gap(px(space::S2))
            .text_color(t.fg_subtle())
            .text_role(&t, TextRole::Sm)
            .child(spinner(Ticker::frame(cx), size::ICON_MD, t.fg_subtle()))
            .when_some(label.clone(), |el, l| el.child(SharedString::from(l)))
            .into_any_element(),
        OusNode::Divider { label } => div()
            .flex()
            .items_center()
            .w_full()
            .gap(px(space::S2))
            .child(div().flex_1().h(px(size::HAIRLINE)).bg(t.border()))
            .when_some(label.clone(), |el, l| {
                el.child(
                    div()
                        .text_role(&t, TextRole::Xs)
                        .text_color(t.fg_subtle())
                        .child(SharedString::from(l)),
                )
                .child(div().flex_1().h(px(size::HAIRLINE)).bg(t.border()))
            })
            .into_any_element(),
        OusNode::KeyHints { hints } => div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(space::S3))
            .children(hints.iter().map(|h| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S1))
                    .text_role(&t, TextRole::Sm)
                    .text_color(t.fg_subtle())
                    .child(kbd(&t, h.keys.clone()))
                    .child(SharedString::from(h.label.clone()))
            }))
            .into_any_element(),
        OusNode::Input {
            id,
            value,
            placeholder,
            on_submit,
            multiline: _,
        } => {
            let action = on_submit.clone();
            let cb = Rc::clone(&o.on_action);
            match o.inputs.as_deref_mut() {
                Some(store) => {
                    let cb2 = Rc::clone(&cb);
                    let action2 = action.clone();
                    let state = store.get(
                        &format!("{key}/{id}"),
                        value,
                        placeholder.as_deref(),
                        move |v, cx| cb2(&action2, JsonValue::from(v), cx),
                        window,
                        cx,
                    );
                    let read = state.clone();
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .w_full()
                        .gap(px(space::S2))
                        .child(div().flex_1().child(Input::new(&state)))
                        .child(
                            Btn::new(ElementId::Name(format!("{key}/{id}/submit").into()))
                                .icon(IconName::ArrowUp)
                                .variant(Variant::Primary)
                                .size(BtnSize::Default)
                                .tooltip("Submit", Some("Enter"))
                                .on_click(move |_, _, cx| {
                                    let v = read.read(cx).value().to_string();
                                    cb(&action, JsonValue::from(v), cx);
                                }),
                        )
                        .into_any_element()
                }
                None => div()
                    .px(px(space::S2))
                    .h(px(size::CONTROL))
                    .flex()
                    .items_center()
                    .rounded(px(radius::R_LG))
                    .border_1()
                    .border_color(t.input_border())
                    .text_color(if value.is_empty() {
                        t.fg_subtlest()
                    } else {
                        t.fg()
                    })
                    .child(SharedString::from(if value.is_empty() {
                        placeholder.clone().unwrap_or_default()
                    } else {
                        value.clone()
                    }))
                    .into_any_element(),
            }
        }
        OusNode::Toggle {
            id,
            label,
            value,
            on_change,
        } => {
            let cb = Rc::clone(&o.on_action);
            let action = on_change.clone();
            div()
                .flex()
                .items_center()
                .gap(px(space::S2))
                .text_role(&t, TextRole::Base)
                .child(switch(
                    ElementId::Name(format!("{key}/{id}").into()),
                    &t,
                    *value,
                    move |on, _, cx| {
                        cb(&action, JsonValue::from(on), cx);
                    },
                ))
                .child(SharedString::from(label.clone()))
                .into_any_element()
        }
        OusNode::Select {
            id,
            options,
            value,
            on_change,
        } => {
            let current = value
                .as_ref()
                .and_then(|v| options.iter().position(|(ov, _)| ov == v))
                .unwrap_or(usize::MAX);
            let values: Vec<String> = options.iter().map(|(v, _)| v.clone()).collect();
            let labels: Vec<&str> = options.iter().map(|(_, l)| l.as_str()).collect();
            let cb = Rc::clone(&o.on_action);
            let action = on_change.clone();
            let pick: Rc<dyn Fn(usize, &mut Window, &mut App)> = Rc::new(move |i, _, cx| {
                if let Some(v) = values.get(i) {
                    cb(&action, JsonValue::from(v.clone()), cx);
                }
            });
            if labels.len() <= 5 {
                div()
                    .child(segmented(format!("{key}/{id}"), &t, &labels, current, pick))
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(space::S1))
                    .children(labels.iter().enumerate().map(|(i, l)| {
                        let pick = Rc::clone(&pick);
                        Btn::new(ElementId::Name(format!("{key}/{id}/{i}").into()))
                            .label(SharedString::from((*l).to_owned()))
                            .size(BtnSize::Sm)
                            .variant(Variant::Outline)
                            .selected(i == current)
                            .on_click(move |_, w, cx| pick(i, w, cx))
                    }))
                    .into_any_element()
            }
        }
        OusNode::Button {
            label,
            action_id,
            tone,
            primary,
        } => {
            let cb = Rc::clone(&o.on_action);
            let action = action_id.clone();
            let variant = match (primary, tone) {
                (true, _) => Variant::Primary,
                (_, Tone::Error) => Variant::OutlineDanger,
                _ => Variant::Outline,
            };
            Btn::new(ElementId::Name(format!("{key}/btn/{action_id}").into()))
                .label(SharedString::from(label.clone()))
                .variant(variant)
                .size(BtnSize::Default)
                .on_click(move |_, _, cx| cb(&action, JsonValue::Null, cx))
                .into_any_element()
        }
        OusNode::Empty { message } => match message {
            Some(m) => crate::components::empty::empty_compact(
                &t,
                IconName::CircleDashed,
                SharedString::from(m.clone()),
            ),
            None => div().into_any_element(),
        },
    }
}

/// A code block (`surface`, `R_LG`, header row with language + copy).
#[must_use]
pub fn code_block(t: &OinoTheme, value: &str, language: Option<&str>, key: &str) -> AnyElement {
    let copy = value.to_owned();
    div()
        .flex()
        .flex_col()
        .w_full()
        .rounded(px(radius::R_LG))
        .bg(t.surface())
        .border_1()
        .border_color(t.border())
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(space::S3))
                .h(px(size::CONTROL))
                .text_role(t, TextRole::Sm)
                .text_color(t.fg_subtle())
                .child(SharedString::from(language.unwrap_or("text").to_owned()))
                .child(
                    Btn::icon_only(
                        ElementId::Name(format!("{key}/copy").into()),
                        IconName::Copy,
                        "Copy",
                    )
                    .size(BtnSize::Xs)
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(copy.clone()));
                    }),
                ),
        )
        .child(
            div()
                .id(ElementId::Name(format!("{key}/code").into()))
                .px(px(space::S3))
                .pb(px(space::S2))
                .code_text(t)
                .text_color(t.fg())
                .overflow_x_scroll()
                .children(value.lines().map(|l| {
                    div()
                        .whitespace_nowrap()
                        .child(SharedString::from(l.to_owned()))
                })),
        )
        .into_any_element()
}
