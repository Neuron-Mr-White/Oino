//! `QueryRail` (design.md §4.10) and `FindBar` (§4.31).

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, ElementId, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};

use crate::components::button::{Btn, BtnSize};
use crate::components::tooltip::tip;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::timeline::rail_excerpt;
use crate::model::transcript::RowKind;
use crate::views::session_pane::{Find, SessionPane};

/// Bars beyond this count are not mounted (the rail virtualises by sampling).
const MAX_BARS: usize = 120;

impl SessionPane {
    /// The query rail (hidden with < 2 queries or a frame below 600 px).
    pub(crate) fn render_rail(&self, t: &OinoTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let prompts = self.model.transcript.prompt_indices();
        // The rail lives in the free margin left of the reading column; without room for it
        // the bars would sit on the transcript (GA-8).
        let margin = (f32::from(self.width) - size::READING) / 2.0 + size::READING_PAD;
        if prompts.len() < 2 || margin < size::QUERY_RAIL + space::S4 {
            return None;
        }
        let current = self.list.logical_scroll_top().item_ix;
        let step = prompts.len().div_ceil(MAX_BARS).max(1);
        let rows = self.model.transcript.rows();
        let bars: Vec<AnyElement> = prompts
            .iter()
            .enumerate()
            .filter(|(i, _)| i % step == 0)
            .map(|(i, &ix)| {
                let next = prompts.get(i + 1).copied().unwrap_or(usize::MAX);
                let active = current >= ix && current < next;
                let (q, a) = rail_excerpt(rows, ix);
                let text = if a.is_empty() {
                    q
                } else {
                    format!("{q}\n— {a}")
                };
                let (w, h) = size::RAIL_BAR;
                div()
                    .id(ElementId::NamedInteger(
                        "rail".into(),
                        u64::try_from(ix).unwrap_or(0),
                    ))
                    .py(px(space::HALF))
                    .cursor_pointer()
                    .tooltip(tip(text, None))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        // A jump is a locating scroll: it leaves tail following first, so the
                        // next streamed patch cannot pull the view back (ZCode
                        // `chat-scroll-bottom-pinning`).
                        this.list.pause_following_tail();
                        this.list.scroll_to_reveal_item(ix);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w(px(w))
                            .h(px(h))
                            .rounded(px(radius::R_FULL))
                            .bg(if active { t.primary() } else { t.fg_subtle() }),
                    )
                    .into_any_element()
            })
            .collect();
        Some(
            div()
                .absolute()
                .left(px(space::S2))
                .top_0()
                .bottom_0()
                .w(px(size::QUERY_RAIL))
                .flex()
                .flex_col()
                .justify_center()
                .items_center()
                .gap(px(space::S1))
                .children(bars)
                .into_any_element(),
        )
    }

    /// Opens the find bar (⌘F).
    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_some() {
            if let Some(f) = &self.find {
                f.input.update(cx, |s, cx| s.focus(window, cx));
            }
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in conversation"));
        input.update(cx, |s, cx| s.focus(window, cx));
        cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => {
                    this.refind(cx);
                    cx.notify();
                }
                InputEvent::PressEnter { shift, .. } => {
                    this.find_step(if *shift { -1 } else { 1 }, cx);
                }
                _ => {}
            },
        )
        .detach();
        self.find = Some(Find {
            input,
            matches: Vec::new(),
            current: 0,
            case: false,
        });
        cx.notify();
    }

    fn refind(&mut self, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else {
            return;
        };
        let query = find.input.read(cx).value().to_string();
        let case = find.case;
        let needle = if case {
            query.clone()
        } else {
            query.to_lowercase()
        };
        let was: Vec<usize> = std::mem::take(&mut find.matches);
        find.current = 0;
        if needle.is_empty() {
            // The old match rows repaint without their highlight (#21).
            for ix in was {
                self.list.remeasure_items(ix..ix + 1);
            }
            return;
        }
        for (i, row) in self.model.transcript.rows().iter().enumerate() {
            let text = match &row.kind {
                RowKind::User(u) => u.text.as_str(),
                RowKind::Assistant(a) => a.text.as_str(),
                RowKind::Tool(t) => t.output.as_str(),
                RowKind::Custom(c) => c.text.as_str(),
                RowKind::Bash(b) => b.output.as_str(),
                _ => "",
            };
            let hit = if case {
                text.contains(&needle)
            } else {
                text.to_lowercase().contains(&needle)
            };
            if hit {
                find.matches.push(i);
            }
        }
        if let Some(&first) = find.matches.first() {
            self.list.pause_following_tail();
            self.list.scroll_to_reveal_item(first);
        }
        // Only the rows that had or gained a match highlight change (#21: no full
        // `remeasure()` while a turn streams).
        let touched: std::collections::BTreeSet<usize> = was
            .into_iter()
            .chain(find.matches.iter().copied())
            .collect();
        for ix in touched {
            self.list.remeasure_items(ix..ix + 1);
        }
    }

    fn find_step(&mut self, dir: isize, cx: &mut Context<Self>) {
        if let Some(find) = &mut self.find
            && !find.matches.is_empty()
        {
            let n = find.matches.len();
            let was = find.matches[find.current];
            find.current = find
                .current
                .checked_add_signed(dir)
                .map_or(n - 1, |c| c % n);
            let ix = find.matches[find.current];
            self.list.pause_following_tail();
            self.list.scroll_to_reveal_item(ix);
            // Only the rows the active highlight moved between change (#21).
            self.list.remeasure_items(was..was + 1);
            self.list.remeasure_items(ix..ix + 1);
            cx.notify();
        }
    }

    /// Closes the find bar.
    pub fn close_find(&mut self, cx: &mut Context<Self>) {
        if let Some(find) = self.find.take() {
            // The match-highlighted rows repaint without their mark (#21).
            for ix in find.matches {
                self.list.remeasure_items(ix..ix + 1);
            }
        }
        cx.notify();
    }

    /// The find bar.
    pub(crate) fn render_find(&self, t: &OinoTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let find = self.find.as_ref()?;
        let count = if find.matches.is_empty() {
            "0/0".to_owned()
        } else {
            format!("{}/{}", find.current + 1, find.matches.len())
        };
        let case = find.case;
        Some(
            div()
                .absolute()
                .top(px(space::S3))
                .right(px(space::S4))
                .w(px(size::FIND_BAR))
                .flex()
                .items_center()
                .gap(px(space::S1))
                .h(px(size::FIELD))
                .px(px(space::S2))
                .rounded(px(radius::R_FLOAT))
                .bg(t.sheet())
                .border_1()
                .border_color(t.border())
                .shadow(t.e_menu())
                .child(
                    div()
                        .flex_1()
                        .child(Input::new(&find.input).appearance(false)),
                )
                .child(
                    div()
                        .mono_text(t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(SharedString::from(count)),
                )
                .child(
                    Btn::icon_only("find-prev", IconName::ChevronUp, "Previous")
                        .size(BtnSize::Sm)
                        .on_click(cx.listener(|this, _, _, cx| this.find_step(-1, cx))),
                )
                .child(
                    Btn::icon_only("find-next", IconName::ChevronDown, "Next")
                        .size(BtnSize::Sm)
                        .on_click(cx.listener(|this, _, _, cx| this.find_step(1, cx))),
                )
                .child(
                    Btn::icon_only("find-case", IconName::CaseSensitive, "Match case")
                        .size(BtnSize::Sm)
                        .selected(case)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(f) = &mut this.find {
                                f.case = !f.case;
                            }
                            this.refind(cx);
                            cx.notify();
                        })),
                )
                .child(
                    Btn::icon_only("find-close", IconName::X, "Close")
                        .size(BtnSize::Sm)
                        .on_click(cx.listener(|this, _, _, cx| this.close_find(cx))),
                )
                .into_any_element(),
        )
    }
}
