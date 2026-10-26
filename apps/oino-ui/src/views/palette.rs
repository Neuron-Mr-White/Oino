//! Command palette / model picker (gpui-app §5.5): a filter input over a virtualised list.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
    uniform_list,
};

use crate::model::palette::{PaletteItem, PaletteMode, filter};
use crate::views::theme;

/// Palette outcome.
#[derive(Clone, Debug)]
pub enum PaletteEvent {
    /// The user picked an item.
    Pick(PaletteItem),
    /// Closed without a pick.
    Dismiss,
}

/// The palette.
pub struct PaletteView {
    mode: PaletteMode,
    items: Vec<PaletteItem>,
    filtered: Vec<PaletteItem>,
    selected: usize,
    input: Entity<InputState>,
    _subscription: Subscription,
}

impl EventEmitter<PaletteEvent> for PaletteView {}

impl Focusable for PaletteView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl PaletteView {
    /// A palette over `items`, focused, with `query` prefilled.
    pub fn new(
        mode: PaletteMode,
        items: Vec<PaletteItem>,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = match mode {
            PaletteMode::Command => "Search sessions and commands  (> commands · # sessions)",
            PaletteMode::Models => "Select a model…",
            PaletteMode::Thinking => "Thinking level…",
        };
        let query = query.to_owned();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(SharedString::from(query.clone()))
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |this: &mut Self, _, event, _, cx| match event {
                    InputEvent::Change => this.refilter(cx),
                    InputEvent::PressEnter { .. } => this.pick(cx),
                    _ => {}
                },
            );
        let filtered = filter(&items, &query);
        let selected = filtered.iter().position(|i| i.current).unwrap_or(0);
        Self {
            mode,
            items,
            filtered,
            selected,
            input,
            _subscription: subscription,
        }
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().to_string();
        self.filtered = filter(&self.items, &query);
        self.selected = 0;
        cx.notify();
    }

    fn pick(&mut self, cx: &mut Context<Self>) {
        match self.filtered.get(self.selected) {
            Some(item) => cx.emit(PaletteEvent::Pick(item.clone())),
            None => cx.emit(PaletteEvent::Dismiss),
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.filtered.len();
        match event.keystroke.key.as_str() {
            "escape" => cx.emit(PaletteEvent::Dismiss),
            "down" if n > 0 => self.selected = (self.selected + 1) % n,
            "up" if n > 0 => self.selected = (self.selected + n - 1) % n,
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for PaletteView {
    #[allow(
        clippy::too_many_lines,
        reason = "declarative element tree; splitting scatters the layout"
    )]
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let count = self.filtered.len();
        let title = match self.mode {
            PaletteMode::Command => None,
            PaletteMode::Models => Some("Model"),
            PaletteMode::Thinking => Some("Thinking"),
        };
        let rows = uniform_list(
            "palette-items",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let theme = cx.theme().clone();
                range
                    .filter_map(|ix| {
                        let item = this.filtered.get(ix)?.clone();
                        let selected = ix == this.selected;
                        Some(
                            h_flex()
                                .id(ix)
                                .w_full()
                                .h(px(32.))
                                .px_3()
                                .gap_3()
                                .rounded_md()
                                .cursor_pointer()
                                .when(selected, |el| el.bg(theme.list_active))
                                .hover(|s| s.bg(theme.list_hover))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.selected = ix;
                                    this.pick(cx);
                                }))
                                .child(
                                    div()
                                        .w(px(72.))
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(item.group),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .when(item.current, |el| {
                                            el.font_weight(FontWeight::SEMIBOLD)
                                        })
                                        .child(SharedString::from(item.label.clone())),
                                )
                                .when_some(item.detail.clone(), |el, d| {
                                    el.child(
                                        div()
                                            .max_w(px(260.))
                                            .truncate()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child(SharedString::from(d)),
                                    )
                                })
                                .when_some(item.keys.clone(), |el, k| {
                                    el.child(
                                        div()
                                            .px_1()
                                            .rounded_sm()
                                            .border_1()
                                            .border_color(theme.border)
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child(SharedString::from(k)),
                                    )
                                })
                                .when(item.current, |el| {
                                    el.child(div().text_xs().text_color(theme.success).child("●"))
                                }),
                        )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .h(px(380.));
        div()
            .id("palette-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .pt(px(96.))
            .bg(theme::scrim())
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PaletteEvent::Dismiss)))
            .child(
                v_flex()
                    .id("palette")
                    .w(px(640.))
                    .h(px(460.))
                    .p_2()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .shadow_lg()
                    .on_click(|_: &ClickEvent, _, cx| cx.stop_propagation())
                    .capture_key_down(cx.listener(Self::on_key))
                    .when_some(title, |el, t| {
                        el.child(
                            div()
                                .px_1()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(t),
                        )
                    })
                    .child(Input::new(&self.input))
                    .child(if count == 0 {
                        div()
                            .p_4()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("No matches")
                            .into_any_element()
                    } else {
                        rows.into_any_element()
                    }),
            )
    }
}
