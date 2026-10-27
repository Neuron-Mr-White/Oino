//! The **glance frame** under the composer (design.md A2.11, unipi `glance-footer`): the
//! `setFrame` bottom segments (left / right groups, 1 px `line` dividers, toned) plus
//! `setStatus` chips, then the `setFooter` stats strip centred on its own line. Both lines are
//! one line each: they never wrap and drop the lowest priorities first (GA-26); `⋯` reveals
//! what was dropped. [`segment_body`] renders one segment (spans, tones, the wordmark brand)
//! for the top glance row drawn inside the composer shell.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};

use crate::components::menu::{Placement, popover, shell};
use crate::components::ous::rich_text;
use crate::design::icons::{icon, ous_icon};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, size, space};
use crate::model::ous::Tone;
use crate::model::ui_state::{Align, Segment, fit_segments};
use crate::views::session_pane::{PaneMenu, SessionPane};

/// Approximate px per character of `Sm` text, for priority dropping.
const CHAR_PX: f32 = 7.0;
/// Characters reserved per divider / gap between kept segments.
const GAP_CHARS: usize = 3;

/// One segment's body: its spans when present (the footer's toned numbers, the rainbow
/// wordmark), else the text in its tone, with the segment icon in front.
pub fn segment_body(t: &OinoTheme, s: &Segment) -> AnyElement {
    let body = match &s.spans {
        Some(spans) => rich_text(t, spans),
        None => div()
            .text_color(match s.tone {
                Tone::Default => t.fg_subtle(),
                tone => t.tone_fg(tone),
            })
            .child(SharedString::from(s.text.clone()))
            .into_any_element(),
    };
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .whitespace_nowrap()
        .gap(px(space::S1))
        .when_some(s.icon.as_deref().and_then(ous_icon), |el, i| {
            el.child(icon(i, size::ICON_SM, t.fg_subtlest()))
        })
        .child(body)
        .into_any_element()
}

fn divider(t: &OinoTheme) -> AnyElement {
    div()
        .flex_shrink_0()
        .w(px(size::HAIRLINE))
        .h(px(space::S3))
        .bg(t.border())
        .into_any_element()
}

fn seg_el(t: &OinoTheme, s: &Segment, i: usize, cx: &mut Context<SessionPane>) -> AnyElement {
    let command = s.command.clone();
    div()
        .id(ElementId::NamedInteger(
            "seg".into(),
            u64::try_from(i).unwrap_or(0),
        ))
        .flex()
        .flex_shrink_0()
        .child(segment_body(t, s))
        .when_some(command, |el, c| {
            el.cursor_pointer()
                .hover(|st| st.opacity(0.8))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.run_prompt(&format!("/{c}"), cx);
                }))
        })
        .into_any_element()
}

/// Joins elements with 1 px dividers.
fn with_dividers(t: &OinoTheme, items: Vec<AnyElement>) -> Vec<AnyElement> {
    let mut out = Vec::with_capacity(items.len() * 2);
    for (i, el) in items.into_iter().enumerate() {
        if i > 0 {
            out.push(divider(t));
        }
        out.push(el);
    }
    out
}

impl SessionPane {
    /// Frame-line segments (`setFrame` bottom + statuses), and the stats strip
    /// (`setFooter` segments, or the spec's text / the session's own numbers).
    fn glance_segments(&self) -> (Vec<Segment>, Vec<Segment>) {
        let mut line: Vec<Segment> = self.model.ui.frame().bottom;
        for status in self.model.ui.statuses() {
            line.push(Segment {
                id: None,
                text: status.text.clone(),
                spans: None,
                command: None,
                tone: if status.tone == Tone::Default {
                    Tone::Muted
                } else {
                    status.tone
                },
                icon: None,
                priority: status.priority,
                align: Align::Left,
            });
        }
        let mut strip: Vec<Segment> = self.model.ui.footer_segments().cloned().collect();
        if strip.is_empty() {
            for spec in self.model.ui.footers() {
                strip.push(Segment {
                    id: None,
                    text: spec.to_lines().join(" · "),
                    spans: None,
                    command: None,
                    tone: Tone::Muted,
                    icon: None,
                    priority: 0,
                    align: Align::Right,
                });
            }
        }
        // Without a footer extension the session's own numbers keep the glance useful.
        if line.iter().all(|s| s.align == Align::Left) && strip.is_empty() {
            if let Some(pct) = self.model.context_percent() {
                line.push(Segment {
                    id: Some("ctx".into()),
                    text: format!("ctx {pct}%"),
                    spans: None,
                    command: Some("compact".into()),
                    tone: if pct >= 80 {
                        Tone::Warning
                    } else {
                        Tone::Muted
                    },
                    icon: None,
                    priority: 5,
                    align: Align::Right,
                });
            }
            let u = self.model.usage;
            if u.input + u.output > 0 {
                line.push(Segment {
                    id: None,
                    text: format!("↑{} ↓{}", compact(u.input), compact(u.output)),
                    spans: None,
                    command: None,
                    tone: Tone::Muted,
                    icon: None,
                    priority: 3,
                    align: Align::Right,
                });
            }
            if u.cost > 0.0 {
                line.push(Segment {
                    id: None,
                    text: format!("${:.2}", u.cost),
                    spans: None,
                    command: None,
                    tone: Tone::Muted,
                    icon: None,
                    priority: 2,
                    align: Align::Right,
                });
            }
        }
        (line, strip)
    }

    /// The glance frame under the composer: the frame line, then the stats strip.
    pub(crate) fn render_meta_row(
        &self,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (line, strip) = self.glance_segments();
        let preview = self.local_previews().into_iter().next();
        if line.is_empty() && strip.is_empty() && preview.is_none() {
            return None;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "width estimate"
        )]
        let cols = ((f32::from(self.width) - space::S8) / CHAR_PX).max(10.0) as usize;
        // Touch keeps fewer segments (design.md §6.2).
        let line_cols = if t.touch() { cols / 2 } else { cols };
        let kept_line: Vec<&Segment> = fit_segments(&line, line_cols, GAP_CHARS);
        let kept_strip: Vec<&Segment> = fit_segments(&strip, cols, GAP_CHARS);
        let dropped: Vec<Segment> = line
            .iter()
            .filter(|s| !kept_line.iter().any(|k| std::ptr::eq(*k, *s)))
            .chain(
                strip
                    .iter()
                    .filter(|s| !kept_strip.iter().any(|k| std::ptr::eq(*k, *s))),
            )
            .cloned()
            .collect();
        let mut left = Vec::new();
        let mut right = Vec::new();
        for (i, s) in kept_line.iter().enumerate() {
            let el = seg_el(t, s, i, cx);
            if s.align == Align::Left {
                left.push(el);
            } else {
                right.push(el);
            }
        }
        let strip_els: Vec<AnyElement> = kept_strip
            .iter()
            .enumerate()
            .map(|(i, s)| seg_el(t, s, 1000 + i, cx))
            .collect();
        let overflow = (!dropped.is_empty()).then(|| {
            let close = {
                let e = cx.entity().downgrade();
                move |_: &mut Window, cx: &mut App| {
                    let _ = e.update(cx, |p, cx| {
                        p.menu = None;
                        cx.notify();
                    });
                }
            };
            popover(
                crate::components::button::Btn::icon_only("meta-more", IconName::Ellipsis, "More")
                    .size(crate::components::button::BtnSize::Xs)
                    .on_click(
                        cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::MetaOverflow, cx)),
                    ),
                self.menu == Some(PaneMenu::MetaOverflow),
                Placement::AboveEnd,
                || {
                    shell(t, size::MENU_MIN * 1.25)
                        .p(px(space::S2))
                        .gap(px(space::S1))
                        .text_role(t, TextRole::Sm)
                        .children(dropped.iter().map(|s| segment_body(t, s)))
                        .into_any_element()
                },
                close,
            )
        });
        // Local dev-server preview chip (design.md A1.4): opens the Browser tab.
        let preview = preview.map(|url| {
            let label = url
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .trim_end_matches('/')
                .to_owned();
            crate::components::button::Btn::new("preview-chip")
                .label(SharedString::from(label))
                .icon(IconName::Globe)
                .size(crate::components::button::BtnSize::Xs)
                .variant(crate::components::button::Variant::Outline)
                .color(t.info_surface_fg())
                .tooltip("Open the preview in the Browser tab", None)
                .on_click(cx.listener(move |this, _, _, cx| this.open_link(&url, cx)))
                .into_any_element()
        });
        let row = || {
            div()
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .min_w_0()
                .h(px(size::META_ROW))
                .overflow_hidden()
                .whitespace_nowrap()
                .gap(px(space::S2))
        };
        let frame_line = (!left.is_empty() || !right.is_empty() || preview.is_some()).then(|| {
            row()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(space::S2))
                        .min_w_0()
                        .overflow_hidden()
                        .children(with_dividers(t, left)),
                )
                .children(preview)
                .child(div().flex_1())
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_shrink_0()
                        .gap(px(space::S2))
                        .children(with_dividers(t, right)),
                )
        });
        let strip_line = (!strip_els.is_empty()).then(|| {
            row().justify_center().child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .min_w_0()
                    .overflow_hidden()
                    .gap(px(space::S2))
                    .children(with_dividers(t, strip_els)),
            )
        });
        Some(
            div()
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .px(px(space::S3))
                .gap(px(space::S1))
                .text_role(t, TextRole::Sm)
                .text_color(t.fg_subtle())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .children(frame_line)
                        .children(strip_line),
                )
                .children(overflow)
                .into_any_element(),
        )
    }
}

/// `1.2k`, `3.4M`.
#[must_use]
pub fn compact(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{}.{}k", n / 1000, (n % 1000) / 100),
        _ => format!("{}.{}M", n / 1_000_000, (n % 1_000_000) / 100_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(text: &str, priority: i64) -> Segment {
        Segment {
            id: None,
            text: text.into(),
            spans: None,
            command: None,
            tone: Tone::Default,
            icon: None,
            priority,
            align: Align::Left,
        }
    }

    #[test]
    fn strip_drops_low_priority_instead_of_wrapping() {
        let strip = [
            seg("1 turn · 2 steps", 5),
            seg("00:02 · tool 00:00", 3),
            seg("477ms avg ttft · 234 tok/s", 1),
            seg("91% cache hit", 4),
        ];
        let kept = fit_segments(&strip, 40, GAP_CHARS);
        let texts: Vec<&str> = kept.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["1 turn · 2 steps", "91% cache hit"]);
        assert_eq!(compact(14_100), "14.1k");
    }
}
