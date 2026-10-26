//! OUS v1 renderer (ui-protocol §6.1 "GPUI rendering" column). Every node renders natively;
//! interactive nodes report `(actionId, value)` through an [`ActionFn`].

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px, relative,
};
use oino_ai::JsonValue;

use crate::model::ous::{OusNode, Rich, Tone, TreeItem};
use crate::model::tools::{DiffLineKind, diff_preview};
use crate::views::theme::{self, tone_color};

/// Callback for interactive nodes: `(actionId, value)`.
pub type ActionFn = Rc<dyn Fn(&str, JsonValue, &mut Window, &mut App)>;

/// An action callback that ignores everything (read-only surfaces).
#[must_use]
pub fn no_action() -> ActionFn {
    Rc::new(|_, _, _, _| {})
}

/// Renders rich text as one paragraph.
pub fn rich_text(rich: &Rich, cx: &App) -> AnyElement {
    let mut row = div().flex().flex_wrap().items_baseline();
    for span in rich {
        let mut el = div().child(SharedString::from(span.text.clone()));
        let color = if span.dim {
            cx.theme().muted_foreground
        } else {
            tone_color(span.tone, cx)
        };
        el = el.text_color(color);
        if span.bold {
            el = el.font_weight(FontWeight::SEMIBOLD);
        }
        if span.italic {
            el = el.italic();
        }
        if span.code {
            el = el
                .font_family(cx.theme().mono_font_family.clone())
                .px_1()
                .rounded_sm()
                .bg(cx.theme().muted);
        }
        if span.link.is_some() {
            el = el.underline().text_color(cx.theme().link);
        }
        row = row.child(el);
    }
    row.into_any_element()
}

/// Renders a unified patch with colored lines (at most `max` lines).
pub fn diff_lines(patch: &str, max: usize, cx: &App) -> AnyElement {
    let preview = diff_preview(patch, max);
    let mono = cx.theme().mono_font_family.clone();
    let mut col = v_flex()
        .w_full()
        .font_family(mono)
        .text_xs()
        .overflow_hidden();
    for line in preview.lines {
        let (bg, fg) = match line.kind {
            DiffLineKind::Added => (Some(theme::diff_added_bg(cx)), cx.theme().success),
            DiffLineKind::Removed => (Some(theme::diff_removed_bg(cx)), cx.theme().danger),
            DiffLineKind::Hunk => (None, cx.theme().info),
            DiffLineKind::Header | DiffLineKind::Context => (None, cx.theme().muted_foreground),
        };
        col = col.child(
            div()
                .px_2()
                .whitespace_nowrap()
                .text_color(fg)
                .when_some(bg, gpui_kit::Styled::bg)
                .child(SharedString::from(line.text)),
        );
    }
    if preview.hidden > 0 {
        col = col.child(
            div()
                .px_2()
                .text_color(cx.theme().muted_foreground)
                .child(format!("… {} more lines", preview.hidden)),
        );
    }
    col.into_any_element()
}

fn bar(value: f64, max: f64, tone: Tone, cx: &App) -> AnyElement {
    let ratio = if max > 0.0 {
        (value / max).clamp(0.0, 1.0)
    } else {
        0.0
    };
    #[allow(clippy::cast_possible_truncation, reason = "ratio in 0..=1")]
    let ratio = ratio as f32;
    let color = if tone == Tone::Default {
        cx.theme().primary
    } else {
        tone_color(tone, cx)
    };
    div()
        .h(px(6.))
        .w_full()
        .rounded_full()
        .bg(cx.theme().muted)
        .child(div().h_full().w(relative(ratio)).rounded_full().bg(color))
        .into_any_element()
}

fn tree_rows(
    items: &[TreeItem],
    depth: usize,
    selectable: Option<&str>,
    key: &str,
    on_action: &ActionFn,
    cx: &App,
    out: &mut Vec<AnyElement>,
) {
    for item in items {
        let id = item.id.clone();
        let action = selectable.map(str::to_owned);
        let cb = Rc::clone(on_action);
        #[allow(clippy::cast_precision_loss, reason = "small depth")]
        let indent = px(depth as f32 * 14.0);
        let mut row = h_flex()
            .id(SharedString::from(format!("{key}/t/{}", item.id)))
            .pl(indent)
            .gap_1()
            .py(px(2.))
            .child(div().text_color(cx.theme().muted_foreground).child(
                if item.children.is_empty() {
                    "·"
                } else if item.expanded {
                    "▾"
                } else {
                    "▸"
                },
            ))
            .child(rich_text(&item.label, cx));
        if let Some(action) = action {
            row = row
                .cursor_pointer()
                .hover(|s| s.bg(cx.theme().list_hover))
                .on_click(move |_: &ClickEvent, window, cx| {
                    cb(&action, JsonValue::from(id.clone()), window, cx);
                });
        }
        out.push(row.into_any_element());
        if item.expanded || depth == 0 && item.children.len() < 8 {
            tree_rows(
                &item.children,
                depth + 1,
                selectable,
                key,
                on_action,
                cx,
                out,
            );
        }
    }
}

/// Renders `node`; `key` must be unique among siblings rendered in the same frame.
#[allow(clippy::too_many_lines, reason = "one arm per OUS node kind")]
pub fn render(node: &OusNode, key: &str, on_action: &ActionFn, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let mono = cx.theme().mono_font_family.clone();
    match node {
        OusNode::Text { value, .. } => rich_text(value, cx),
        OusNode::Markdown { value } => {
            TextView::markdown(SharedString::from(format!("{key}/md")), value.clone())
                .selectable(true)
                .into_any_element()
        }
        OusNode::Code { value, .. } => div()
            .w_full()
            .p_2()
            .rounded_md()
            .bg(cx.theme().muted)
            .font_family(mono)
            .text_xs()
            .child(SharedString::from(value.clone()))
            .into_any_element(),
        OusNode::Diff { patch, path } => v_flex()
            .w_full()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .overflow_hidden()
            .when_some(path.clone(), |el, path| {
                el.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .bg(cx.theme().muted)
                        .child(SharedString::from(path)),
                )
            })
            .child(diff_lines(patch, 400, cx))
            .into_any_element(),
        OusNode::Image { alt } => div()
            .text_color(muted)
            .child(format!("[image: {alt}]"))
            .into_any_element(),
        OusNode::Log {
            lines, max_lines, ..
        } => {
            let start = max_lines.map_or(0, |m| lines.len().saturating_sub(m));
            v_flex()
                .w_full()
                .p_2()
                .rounded_md()
                .bg(cx.theme().muted)
                .font_family(mono)
                .text_xs()
                .children(lines[start..].iter().map(|l| {
                    div()
                        .whitespace_nowrap()
                        .child(SharedString::from(l.clone()))
                }))
                .into_any_element()
        }
        OusNode::Kv { items } => v_flex()
            .gap_1()
            .children(items.iter().map(|(k, v)| {
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .min_w(px(120.))
                            .text_color(muted)
                            .child(rich_text(k, cx)),
                    )
                    .child(div().flex_1().child(rich_text(v, cx)))
            }))
            .into_any_element(),
        OusNode::Table {
            columns,
            rows,
            selectable,
        } => {
            let header = h_flex()
                .w_full()
                .gap_2()
                .px_2()
                .py_1()
                .bg(cx.theme().table_head)
                .text_xs()
                .text_color(cx.theme().table_head_foreground)
                .children(
                    columns
                        .iter()
                        .map(|c| div().flex_1().child(SharedString::from(c.clone()))),
                );
            let body = rows.iter().enumerate().map(|(i, row)| {
                let cb = Rc::clone(on_action);
                let action = selectable.clone();
                h_flex()
                    .id(SharedString::from(format!("{key}/r{i}")))
                    .w_full()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().table_row_border)
                    .children(
                        row.iter()
                            .map(|cell| div().flex_1().child(rich_text(cell, cx))),
                    )
                    .when_some(action, |el, action| {
                        el.cursor_pointer()
                            .hover(|s| s.bg(cx.theme().table_hover))
                            .on_click(move |_: &ClickEvent, window, cx| {
                                cb(&action, JsonValue::from(i), window, cx);
                            })
                    })
            });
            v_flex()
                .w_full()
                .text_sm()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .overflow_hidden()
                .child(header)
                .children(body)
                .into_any_element()
        }
        OusNode::List {
            items,
            selectable,
            selected,
        } => v_flex()
            .w_full()
            .children(items.iter().map(|item| {
                let cb = Rc::clone(on_action);
                let action = selectable.clone();
                let id = item.id.clone();
                let is_selected = selected.as_deref() == Some(item.id.as_str());
                h_flex()
                    .id(SharedString::from(format!("{key}/l/{}", item.id)))
                    .w_full()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .when(is_selected, |el| el.bg(cx.theme().list_active))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_color(tone_color(item.tone, cx))
                                    .child(rich_text(&item.label, cx)),
                            )
                            .when_some(item.detail.as_ref(), |el, d| {
                                el.child(div().text_xs().text_color(muted).child(rich_text(d, cx)))
                            }),
                    )
                    .when_some(item.badge.as_ref(), |el, b| {
                        el.child(
                            div()
                                .px_1()
                                .rounded_sm()
                                .text_xs()
                                .bg(cx.theme().secondary)
                                .child(rich_text(b, cx)),
                        )
                    })
                    .when_some(action, |el, action| {
                        el.cursor_pointer()
                            .hover(|s| s.bg(cx.theme().list_hover))
                            .on_click(move |_: &ClickEvent, window, cx| {
                                cb(&action, JsonValue::from(id.clone()), window, cx);
                            })
                    })
            }))
            .into_any_element(),
        OusNode::Tree { nodes, selectable } => {
            let mut rows = Vec::new();
            tree_rows(
                nodes,
                0,
                selectable.as_deref(),
                key,
                on_action,
                cx,
                &mut rows,
            );
            v_flex()
                .w_full()
                .text_sm()
                .children(rows)
                .into_any_element()
        }
        OusNode::Tabs {
            tabs,
            active,
            on_change,
        } => {
            let active_id = active
                .clone()
                .or_else(|| tabs.first().map(|t| t.0.clone()))
                .unwrap_or_default();
            let bar = h_flex()
                .gap_1()
                .border_b_1()
                .border_color(cx.theme().border)
                .children(tabs.iter().map(|(id, title, _)| {
                    let cb = Rc::clone(on_action);
                    let action = on_change.clone();
                    let value = id.clone();
                    let is_active = *id == active_id;
                    div()
                        .id(SharedString::from(format!("{key}/tab/{id}")))
                        .px_3()
                        .py_1()
                        .text_sm()
                        .cursor_pointer()
                        .when(is_active, |el| {
                            el.border_b_2()
                                .border_color(cx.theme().primary)
                                .font_weight(FontWeight::MEDIUM)
                        })
                        .when(!is_active, |el| el.text_color(muted))
                        .child(SharedString::from(title.clone()))
                        .when_some(action, |el, action| {
                            el.on_click(move |_: &ClickEvent, window, cx| {
                                cb(&action, JsonValue::from(value.clone()), window, cx);
                            })
                        })
                }));
            let body = tabs
                .iter()
                .find(|t| t.0 == active_id)
                .map(|(id, _, body)| render(body, &format!("{key}/{id}"), on_action, cx));
            v_flex()
                .w_full()
                .gap_2()
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
                OusNode::Stack { gap: Some(g), .. } => px(*g),
                _ => px(8.),
            };
            let base = if *horizontal {
                h_flex().items_start()
            } else {
                v_flex()
            };
            base.w_full()
                .gap(gap)
                .children(children.iter().enumerate().map(|(i, c)| {
                    let el = render(c, &format!("{key}/{i}"), on_action, cx);
                    if *horizontal && matches!(node, OusNode::Split { .. }) {
                        div().flex_1().min_w_0().child(el).into_any_element()
                    } else {
                        el
                    }
                }))
                .into_any_element()
        }
        OusNode::Section {
            title,
            body,
            collapsed,
            ..
        } => v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child(rich_text(title, cx)),
            )
            .when(!collapsed, |el| {
                el.child(render(body, &format!("{key}/b"), on_action, cx))
            })
            .into_any_element(),
        OusNode::Card {
            title,
            rail,
            source,
            body,
        } => {
            let rail_color = rail.map(|r| tone_color(r, cx));
            h_flex()
                .w_full()
                .items_start()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .overflow_hidden()
                .when_some(rail_color, |el, color| {
                    el.child(div().w(px(3.)).h_full().min_h(px(24.)).bg(color))
                })
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .p_2()
                        .gap_1()
                        .when(title.is_some() || source.is_some(), |el| {
                            el.child(
                                h_flex()
                                    .gap_2()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .when_some(title.as_ref(), |el, t| el.child(rich_text(t, cx)))
                                    .when_some(source.clone(), |el, s| {
                                        el.child(
                                            div()
                                                .text_xs()
                                                .text_color(theme::harness())
                                                .child(SharedString::from(s)),
                                        )
                                    }),
                            )
                        })
                        .child(render(body, &format!("{key}/c"), on_action, cx)),
                )
                .into_any_element()
        }
        OusNode::Badge { value, tone } => div()
            .px_1p5()
            .rounded_sm()
            .text_xs()
            .border_1()
            .border_color(tone_color(*tone, cx).opacity(0.5))
            .text_color(tone_color(*tone, cx))
            .child(rich_text(value, cx))
            .into_any_element(),
        OusNode::Meter {
            value,
            max,
            label,
            tone,
        } => v_flex()
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .justify_between()
                    .text_xs()
                    .text_color(muted)
                    .child(SharedString::from(label.clone().unwrap_or_default()))
                    .child(format!("{value} / {max}")),
            )
            .child(bar(*value, *max, *tone, cx))
            .into_any_element(),
        OusNode::Gauge { value, label } => v_flex()
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .justify_between()
                    .text_xs()
                    .text_color(muted)
                    .child(SharedString::from(label.clone().unwrap_or_default()))
                    .child(format!("{:.0}%", value * 100.0)),
            )
            .child(bar(*value, 1.0, Tone::Default, cx))
            .into_any_element(),
        OusNode::Sparkline { values, label } => {
            let max = values.iter().copied().fold(0.0_f64, f64::max);
            h_flex()
                .gap_2()
                .items_end()
                .when_some(label.clone(), |el, l| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(SharedString::from(l)),
                    )
                })
                .child(
                    h_flex()
                        .items_end()
                        .gap(px(1.))
                        .h(px(18.))
                        .children(values.iter().map(|v| {
                            #[allow(clippy::cast_possible_truncation, reason = "pixel height")]
                            let h = if max > 0.0 {
                                (v / max * 18.0) as f32
                            } else {
                                0.0
                            };
                            div().w(px(3.)).h(px(h.max(1.0))).bg(cx.theme().primary)
                        })),
                )
                .into_any_element()
        }
        OusNode::BarChart { bars } => {
            let max = bars.iter().map(|b| b.value).fold(0.0_f64, f64::max);
            v_flex()
                .w_full()
                .gap_1()
                .children(bars.iter().map(|b| {
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .child(
                            div()
                                .w(px(96.))
                                .truncate()
                                .child(SharedString::from(b.label.clone())),
                        )
                        .child(div().flex_1().child(bar(b.value, max, b.tone, cx)))
                        .child(div().w(px(48.)).text_right().child(format!("{}", b.value)))
                }))
                .into_any_element()
        }
        OusNode::Area { series } => v_flex()
            .gap_1()
            .children(series.iter().enumerate().map(|(i, (label, values))| {
                render(
                    &OusNode::Sparkline {
                        values: values.clone(),
                        label: Some(label.clone()),
                    },
                    &format!("{key}/a{i}"),
                    on_action,
                    cx,
                )
            }))
            .into_any_element(),
        OusNode::Spinner { label } => h_flex()
            .gap_2()
            .text_color(muted)
            .child("◌")
            .when_some(label.clone(), |el, l| el.child(SharedString::from(l)))
            .into_any_element(),
        OusNode::Divider { label } => h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(div().flex_1().h(px(1.)).bg(cx.theme().border))
            .when_some(label.clone(), |el, l| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(SharedString::from(l)),
                )
                .child(div().flex_1().h(px(1.)).bg(cx.theme().border))
            })
            .into_any_element(),
        OusNode::KeyHints { hints } => h_flex()
            .gap_3()
            .text_xs()
            .text_color(muted)
            .children(hints.iter().map(|h| {
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .px_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(cx.theme().border)
                            .font_family(mono.clone())
                            .child(SharedString::from(h.keys.clone())),
                    )
                    .child(SharedString::from(h.label.clone()))
            }))
            .into_any_element(),
        OusNode::Input {
            value,
            placeholder,
            on_submit,
            ..
        } => {
            let cb = Rc::clone(on_action);
            let action = on_submit.clone();
            let current = value.clone();
            h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .border_1()
                        .border_color(cx.theme().input)
                        .text_color(if value.is_empty() {
                            muted
                        } else {
                            cx.theme().foreground
                        })
                        .child(SharedString::from(if value.is_empty() {
                            placeholder.clone().unwrap_or_default()
                        } else {
                            value.clone()
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("{key}/submit")))
                        .label("Submit")
                        .small()
                        .on_click(move |_, window, cx| {
                            cb(&action, JsonValue::from(current.clone()), window, cx);
                        }),
                )
                .into_any_element()
        }
        OusNode::Toggle {
            label,
            value,
            on_change,
            ..
        } => {
            let cb = Rc::clone(on_action);
            let action = on_change.clone();
            Switch::new(SharedString::from(format!("{key}/toggle")))
                .checked(*value)
                .label(SharedString::from(label.clone()))
                .on_click(move |checked: &bool, window, cx| {
                    cb(&action, JsonValue::from(*checked), window, cx);
                })
                .into_any_element()
        }
        OusNode::Select {
            options,
            value,
            on_change,
            ..
        } => h_flex()
            .gap_1()
            .flex_wrap()
            .children(options.iter().map(|(v, label)| {
                let cb = Rc::clone(on_action);
                let action = on_change.clone();
                let chosen = v.clone();
                let mut b = Button::new(SharedString::from(format!("{key}/opt/{v}")))
                    .label(SharedString::from(label.clone()))
                    .small()
                    .on_click(move |_, window, cx| {
                        cb(&action, JsonValue::from(chosen.clone()), window, cx);
                    });
                b = if value.as_deref() == Some(v.as_str()) {
                    b.primary()
                } else {
                    b.outline()
                };
                b
            }))
            .into_any_element(),
        OusNode::Button {
            label,
            action_id,
            tone,
            primary,
        } => {
            let cb = Rc::clone(on_action);
            let action = action_id.clone();
            let mut b = Button::new(SharedString::from(format!("{key}/btn/{action_id}")))
                .label(SharedString::from(label.clone()))
                .small()
                .on_click(move |_, window, cx| cb(&action, JsonValue::Null, window, cx));
            b = match (primary, tone) {
                (true, _) => b.primary(),
                (_, Tone::Error) => b.danger(),
                (_, Tone::Warning) => b.warning(),
                (_, Tone::Success) => b.success(),
                _ => b.outline(),
            };
            b.into_any_element()
        }
        OusNode::Empty { message } => div()
            .text_sm()
            .text_color(muted)
            .when_some(message.clone(), |el, m| el.child(SharedString::from(m)))
            .into_any_element(),
    }
}
