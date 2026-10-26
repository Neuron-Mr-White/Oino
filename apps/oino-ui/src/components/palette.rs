//! `CommandPalette` (design.md §4.25): scopes, prefixes, sections, history, virtualised rows.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
    uniform_list,
};

use crate::components::badge::kbd;
use crate::components::controls::segmented;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::palette::{
    self, Kind, PaletteAction, PaletteItem, Row, Scope, matched_positions,
};

/// Palette outcome.
#[derive(Clone, Debug)]
pub enum PaletteEvent {
    /// The user picked an action (and the query that found it).
    Pick(PaletteAction, String),
    /// Closed without a pick.
    Dismiss,
}

/// The palette.
pub struct PaletteView {
    items: Vec<PaletteItem>,
    rows: Vec<Row>,
    selected: usize,
    scope: Scope,
    history: Vec<String>,
    input: Entity<InputState>,
    _subscription: Subscription,
}

impl EventEmitter<PaletteEvent> for PaletteView {}

impl Focusable for PaletteView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

fn icon_of(stem: &str) -> IconName {
    match stem {
        "square-pen" => IconName::SquarePen,
        "folder-open" => IconName::FolderOpen,
        "settings" => IconName::Settings,
        "archive" => IconName::Archive,
        "git-branch" => IconName::GitBranch,
        "list-tree" => IconName::ListTree,
        "square" => IconName::Square,
        "panel-left" => IconName::PanelLeft,
        "panel-right" => IconName::PanelRight,
        "panel-bottom" => IconName::PanelBottom,
        "kanban" => IconName::Kanban,
        "blocks" => IconName::Blocks,
        "zap" => IconName::Zap,
        "keyboard" => IconName::Keyboard,
        "sun" => IconName::Sun,
        "moon" => IconName::Moon,
        "circle-arrow-down" => IconName::CircleArrowDown,
        "file" | "file-text" => IconName::FileText,
        "dot" => IconName::CircleDot,
        "search" => IconName::Search,
        "box" => IconName::Box,
        "brain" => IconName::Brain,
        "refresh-cw" => IconName::RefreshCw,
        "smartphone" => IconName::Smartphone,
        _ => IconName::ChevronRight,
    }
}

impl PaletteView {
    /// A palette over `items`, focused, with `query` prefilled.
    pub fn new(
        items: Vec<PaletteItem>,
        history: Vec<String>,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = query.to_owned();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search commands, sessions, files…")
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
        let rows = palette::rows(&items, &query, Scope::All);
        let selected = palette::first(&rows).unwrap_or(0);
        Self {
            items,
            rows,
            selected,
            scope: Scope::All,
            history,
            input,
            _subscription: subscription,
        }
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().to_string();
        self.rows = palette::rows(&self.items, &query, self.scope);
        self.selected = palette::first(&self.rows).unwrap_or(0);
        cx.notify();
    }

    fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        self.scope = scope;
        self.refilter(cx);
    }

    fn pick(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().to_string();
        match self.rows.get(self.selected) {
            Some(Row::Item(i)) => {
                if let Some(item) = self.items.get(*i) {
                    cx.emit(PaletteEvent::Pick(item.action.clone(), query));
                }
            }
            Some(Row::More(kind, _)) => {
                let scope = match kind {
                    Kind::Action => Scope::Actions,
                    Kind::Session => Scope::Sessions,
                    Kind::File => Scope::Files,
                };
                self.set_scope(scope, cx);
            }
            _ => cx.emit(PaletteEvent::Dismiss),
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = event.keystroke.modifiers;
        match event.keystroke.key.as_str() {
            "escape" => cx.emit(PaletteEvent::Dismiss),
            "down" => self.selected = palette::step(&self.rows, self.selected, 1).unwrap_or(0),
            "up" => self.selected = palette::step(&self.rows, self.selected, -1).unwrap_or(0),
            "tab" => {
                let i = Scope::ALL
                    .iter()
                    .position(|s| *s == self.scope)
                    .unwrap_or(0);
                let next = if m.shift {
                    i + Scope::ALL.len() - 1
                } else {
                    i + 1
                } % Scope::ALL.len();
                self.set_scope(Scope::ALL[next], cx);
            }
            k if crate::components::secondary(&m) && matches!(k, "1" | "2" | "3" | "4") => {
                let i = k.parse::<usize>().unwrap_or(1) - 1;
                self.set_scope(Scope::ALL[i], cx);
            }
            "backspace" if self.input.read(cx).value().is_empty() && self.scope != Scope::All => {
                self.set_scope(Scope::All, cx);
            }
            _ => return,
        }
        let _ = window;
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for PaletteView {
    #[allow(clippy::too_many_lines, reason = "declarative layout")]
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = OinoTheme::get(cx);
        let count = self.rows.len();
        let query = self.input.read(cx).value().to_string();
        let (_, needle) = palette::parse_query(&query, self.scope);
        let scope_ix = Scope::ALL
            .iter()
            .position(|s| *s == self.scope)
            .unwrap_or(0);
        let labels: Vec<&str> = Scope::ALL.iter().map(|s| s.label()).collect();
        let entity = cx.entity().downgrade();
        let tabs = segmented(
            "palette-scope",
            &t,
            &labels,
            scope_ix,
            std::rc::Rc::new(move |i, _, cx| {
                let _ = entity.update(cx, |p, cx| p.set_scope(Scope::ALL[i], cx));
            }),
        );
        let rows = uniform_list(
            "palette-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                let t = OinoTheme::get(cx);
                let (_, needle) = palette::parse_query(&this.input.read(cx).value(), this.scope);
                range
                    .filter_map(|ix| {
                        let row = this.rows.get(ix)?;
                        let selected = ix == this.selected;
                        Some(match row {
                            Row::Header(label) => div()
                                .id(ix)
                                .flex()
                                .items_end()
                                .h(px(size::CONTROL_LG))
                                .px(px(space::S2))
                                .pb(px(space::S1))
                                .text_role(&t, TextRole::Xs)
                                .weight_medium()
                                .text_color(t.fg_subtlest())
                                .child(SharedString::new_static(label).to_uppercase())
                                .into_any_element(),
                            Row::More(_, n) => div()
                                .id(ix)
                                .w_full()
                                .flex()
                                .items_center()
                                .h(px(size::CONTROL_LG))
                                .px(px(space::S2))
                                .rounded(px(radius::R_MD))
                                .cursor_pointer()
                                .when(selected, |el| el.bg(t.menu_hover()))
                                .text_role(&t, TextRole::Sm)
                                .text_color(t.fg_subtle())
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.selected = ix;
                                    this.pick(cx);
                                }))
                                .child(format!("Show {n} more"))
                                .into_any_element(),
                            Row::Item(i) => {
                                let item = this.items.get(*i)?;
                                let hits = matched_positions(&needle, &item.label);
                                let label = div().flex().flex_row().children(
                                    item.label.chars().enumerate().map(|(ci, ch)| {
                                        let hit = hits.contains(&ci);
                                        div()
                                            .text_color(if hit || needle.is_empty() {
                                                t.fg()
                                            } else {
                                                t.fg_subtle()
                                            })
                                            .when(
                                                hit,
                                                super::super::design::theme::TextExt::weight_medium,
                                            )
                                            .child(SharedString::from(ch.to_string()))
                                    }),
                                );
                                div()
                                    .id(ix)
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::S2))
                                    .h(px(size::CONTROL_LG))
                                    .px(px(space::S2))
                                    .rounded(px(radius::R_MD))
                                    .cursor_pointer()
                                    .when(selected, |el| el.bg(t.menu_hover()))
                                    .hover(|s| s.bg(t.menu_hover()))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.selected = ix;
                                        this.pick(cx);
                                    }))
                                    .child(icon(icon_of(item.icon), size::ICON, t.fg_subtle()))
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_role(&t, TextRole::Base)
                                            .child(label),
                                    )
                                    .when_some(item.detail.clone(), |el, d| {
                                        el.child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_role(&t, TextRole::Sm)
                                                .text_color(t.fg_subtle())
                                                .child(SharedString::from(d)),
                                        )
                                    })
                                    .when(item.detail.is_none(), |el| el.child(div().flex_1()))
                                    .when(item.current, |el| {
                                        el.child(crate::components::badge::dot(
                                            &t,
                                            crate::components::badge::DotKind::Unread,
                                        ))
                                    })
                                    .when_some(item.keys.clone(), |el, k| el.child(kbd(&t, k)))
                                    .into_any_element()
                            }
                        })
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .h(px(size::PALETTE_MAX_H - size::INPUT * 3.0));
        let history: Vec<String> = self.history.iter().take(3).cloned().collect();
        let more = self.history.len().saturating_sub(3);
        let _ = needle;
        div()
            .id("palette-scrim")
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .pt(px(size::PALETTE_TOP))
            .bg(t.scrim())
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PaletteEvent::Dismiss)))
            .child(
                div()
                    .id("palette")
                    .flex()
                    .flex_col()
                    .w(px(size::PALETTE))
                    .max_h(px(size::PALETTE_MAX_H))
                    .p(px(space::S2))
                    .gap(px(space::S2))
                    .rounded(px(radius::R_2XL))
                    .bg(t.popover())
                    .border_1()
                    .border_color(t.popover_border())
                    .shadow(t.e2())
                    .text_color(t.fg())
                    .font_family(t.ui_font())
                    .on_click(|_: &ClickEvent, _, cx| cx.stop_propagation())
                    .capture_key_down(cx.listener(Self::on_key))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S2))
                            .px(px(space::S2))
                            .child(icon(IconName::Search, size::ICON, t.fg_subtle()))
                            .child(
                                div()
                                    .flex_1()
                                    .child(Input::new(&self.input).appearance(false)),
                            )
                            .child(kbd(&t, "esc")),
                    )
                    .child(div().px(px(space::S1)).child(tabs))
                    .when(!history.is_empty() && query.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(space::S1))
                                .px(px(space::S2))
                                .text_role(&t, TextRole::Sm)
                                .text_color(t.fg_subtle())
                                .child("History")
                                .children(history.into_iter().enumerate().map(|(i, h)| {
                                    let q = h.clone();
                                    div()
                                        .id(ElementId::NamedInteger(
                                            "palette-hist".into(),
                                            i as u64,
                                        ))
                                        .px(px(space::S2))
                                        .rounded(px(radius::R_FULL))
                                        .bg(t.surface())
                                        .cursor_pointer()
                                        .hover(|s| s.bg(t.hover()))
                                        .on_click(cx.listener(
                                            move |this, _: &ClickEvent, window, cx| {
                                                let q = q.clone();
                                                this.input
                                                    .update(cx, |s, cx| s.set_value(q, window, cx));
                                                this.refilter(cx);
                                            },
                                        ))
                                        .child(SharedString::from(h))
                                }))
                                .when(more > 0, |el| el.child(format!("+{more}"))),
                        )
                    })
                    .child(if count == 0 {
                        div()
                            .p(px(space::S4))
                            .text_role(&t, TextRole::Base)
                            .text_color(t.fg_subtle())
                            .child("No matches")
                            .into_any_element()
                    } else {
                        rows.into_any_element()
                    }),
            )
    }
}
