//! `MetaRow` (design.md §4.18): 24 px row under the composer. Left: frame-bottom segments then
//! `setStatus` chips; right: `setFooter` segments (stats strip). Lowest priorities drop first
//! across both sides; `⋯` reveals the dropped ones.

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
const CHAR_PX: f32 = 6.5;

fn seg_el(t: &OinoTheme, s: &Segment, i: usize, cx: &mut Context<SessionPane>) -> AnyElement {
    let command = s.command.clone();
    let body = match &s.spans {
        Some(spans) => rich_text(t, spans),
        None => div()
            .text_color(t.tone_fg(s.tone))
            .child(SharedString::from(s.text.clone()))
            .into_any_element(),
    };
    div()
        .id(ElementId::NamedInteger(
            "seg".into(),
            u64::try_from(i).unwrap_or(0),
        ))
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(px(space::S1))
        .when_some(s.icon.as_deref().and_then(ous_icon), |el, i| {
            el.child(icon(i, size::ICON_SM, t.tone_fg(s.tone)))
        })
        .child(body)
        .when_some(command, |el, c| {
            el.cursor_pointer()
                .hover(|st| st.text_color(t.fg()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.run_prompt(&format!("/{c}"), cx);
                }))
        })
        .into_any_element()
}

impl SessionPane {
    /// All meta-row segments in display order (left group, then right group).
    fn meta_segments(&self) -> Vec<Segment> {
        let mut out: Vec<Segment> = self.model.ui.frame().bottom;
        for status in self.model.ui.statuses() {
            out.push(Segment {
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
        let footer: Vec<Segment> = self.model.ui.footer_segments().cloned().collect();
        if footer.is_empty() {
            for spec in self.model.ui.footers() {
                out.push(Segment {
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
        out.extend(footer.into_iter().map(|mut s| {
            s.align = Align::Right;
            s
        }));
        if out.iter().all(|s| s.align == Align::Left) {
            if let Some(pct) = self.model.context_percent() {
                out.push(Segment {
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
                out.push(Segment {
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
                out.push(Segment {
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
        out
    }

    /// The meta row.
    pub(crate) fn render_meta_row(
        &self,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let all = self.meta_segments();
        if all.is_empty() {
            return None;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "width estimate"
        )]
        let width = ((f32::from(self.width) - space::S8) / CHAR_PX).max(10.0) as usize;
        let kept: Vec<&Segment> = fit_segments(&all, width, 3);
        let dropped: Vec<Segment> = all
            .iter()
            .filter(|s| !kept.iter().any(|k| std::ptr::eq(*k, *s)))
            .cloned()
            .collect();
        let mut left = Vec::new();
        let mut right = Vec::new();
        for (i, s) in kept.iter().enumerate() {
            let el = seg_el(t, s, i, cx);
            if s.align == Align::Left {
                left.push(el);
            } else {
                right.push(el);
            }
        }
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
                        .children(dropped.iter().map(|s| {
                            div()
                                .text_color(t.tone_fg(s.tone))
                                .child(SharedString::from(s.text.clone()))
                        }))
                        .into_any_element()
                },
                close,
            )
        });
        Some(
            div()
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .h(px(size::META_ROW))
                .px(px(space::S2))
                .gap(px(space::S3))
                .text_role(t, TextRole::Sm)
                .text_color(t.fg_subtle())
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(space::S3))
                        .min_w_0()
                        .overflow_hidden()
                        .children(left),
                )
                // Local dev-server preview chip (design.md A1.4): opens the Browser tab.
                .children(self.local_previews().into_iter().next().map(|url| {
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
                }))
                .child(div().flex_1())
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(space::S3))
                        .children(right),
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
